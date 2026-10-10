//! Video encoding pipeline via an ffmpeg subprocess (ported from VideoEncoder.cpp /
//! encoder.py). Frames are composited by the application and piped to ffmpeg as
//! rawvideo; the original audio track is mapped in the same command.
//!
//! Key invariants (verified in the previous implementation):
//! * MP4 output uses `-movflags frag_keyframe+empty_moov -flush_packets 1` so the moov
//!   index is written up front and every keyframe fragment lands on disk immediately —
//!   an aborted encode still leaves a playable file.
//! * Never use `-shortest` (deadlocks the final fragment flush on long encodes).
//! * QSV hwupload path must not force `-pix_fmt`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use crate::gyro::Stabilizer;
use crate::process::background_command;
use crate::settings::{EncodeSettings, Settings};

/// Locate the ffmpeg executable: next to our own exe first, then PATH.
pub fn find_ffmpeg() -> String {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let cand = dir.join("ffmpeg.exe");
            if cand.exists() {
                return cand.to_string_lossy().into_owned();
            }
        }
    }
    "ffmpeg.exe".to_string()
}

/// List the encoder names reported by this ffmpeg build.
pub fn available_encoders() -> Vec<String> {
    let out = background_command(find_ffmpeg())
        .arg("-hide_banner")
        .arg("-encoders")
        .output();
    let mut names = Vec::new();
    if let Ok(out) = out {
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 && (parts[0].starts_with('V') || parts[0].starts_with('A')) {
                names.push(parts[1].to_string());
            }
        }
    }
    names
}

/// Map (encoder name) -> (init_hw_device args, vf args) for the GPU fast path.
fn hw_device_args(vendor: &str) -> (Vec<String>, Vec<String>) {
    match vendor {
        "qsv" => (
            vec![
                "-init_hw_device".into(),
                "qsv=hw".into(),
                "-filter_hw_device".into(),
                "hw".into(),
            ],
            vec!["-vf".into(), "format=nv12,hwupload".into()],
        ),
        "nvenc" => (
            vec![
                "-init_hw_device".into(),
                "cuda=hw".into(),
                "-filter_hw_device".into(),
                "hw".into(),
            ],
            vec!["-vf".into(), "format=nv12,hwupload_cuda".into()],
        ),
        "amf" => (
            vec![
                "-init_hw_device".into(),
                "d3d11va=hw".into(),
                "-filter_hw_device".into(),
                "hw".into(),
            ],
            vec!["-vf".into(), "format=nv12,hwupload".into()],
        ),
        _ => (Vec::new(), Vec::new()),
    }
}

/// 软件编码器名（每格式的**首选**）。仅用于展示/默认值；
/// 真正决定用哪个编码器的是 [`resolve_video_encoder`]（它按本机实际可用的
/// 编码器在 [`software_fallback_candidates`] 里挑第一个）。
pub fn software_encoder_for(format: &str) -> &'static str {
    software_fallback_candidates(format)
        .first()
        .copied()
        .unwrap_or("libx264")
}

/// 该格式的软件编码器候选（按偏好排序）。
///
/// AV1 首选 SVT-AV1：实测 640x360 单帧比 libaom 快约 3.5 倍（163ms vs 566ms），
/// 回退路径本来就慢，能快一点是一点。libaom 保底，rav1e 最后。
fn software_fallback_candidates(format: &str) -> &'static [&'static str] {
    match format {
        "av1" => &["libsvtav1", "libaom-av1", "librav1e"],
        "hevc" => &["libx265"],
        "h264" => &["libx264"],
        "mpeg4" => &["mpeg4", "libxvid"],
        _ => &["libx264"],
    }
}

/// 为"自动回退到 CPU"挑一个**本机 ffmpeg 真的支持**的软件编码器。
///
/// 返回 `(format, encoder)`：优先用原格式的软件编码器；如果这个 ffmpeg 构建里
/// 根本没有该格式的软件编码器（典型：gpl-lite 没有 libaom-av1 / libsvtav1），
/// 就退到 H.264（`libx264`，所有构建都有），保证"一定能导出成功"，
/// 而不是把用户从"硬件不支持"直接推到"软件也不支持"。
pub fn pick_software_fallback(format: &str, available: &[String]) -> (String, String) {
    let has = |name: &str| available.iter().any(|e| e == name);
    for c in software_fallback_candidates(format) {
        if has(c) {
            return (format.to_string(), (*c).to_string());
        }
    }
    // 该格式没有可用的软件编码器 → 换成 H.264。
    ("h264".to_string(), "libx264".to_string())
}

/// The effective encode range in source-video milliseconds: start is always
/// `clip_start_ms`; end falls back to the full duration when `clip_end_ms` is
/// zero or beyond it. Shared by the command builder and the frame loop so the
/// total-frame count and the actual frames fed to ffmpeg always agree.
pub fn clip_range(enc: &EncodeSettings, src_duration_ms: f64) -> (f64, f64) {
    let start = enc.clip_start_ms.max(0.0);
    let mut end = if enc.clip_end_ms > start {
        enc.clip_end_ms
    } else {
        src_duration_ms
    };
    if src_duration_ms > 0.0 {
        end = end.min(src_duration_ms);
    }
    (start, end.max(start))
}

/// Total number of frames a clip-range encode will produce. With a speed
/// multiplier N the frame loop samples source time N times faster, so the
/// output holds 1/N of the frames (output fps unchanged → clip is N× shorter).
pub fn clip_total_frames(enc: &EncodeSettings, src_duration_ms: f64, fps: f64) -> usize {
    let (s, e) = clip_range(enc, src_duration_ms);
    let n = encode_speed_ratio(enc);
    (((((e - s) / 1000.0) * fps) / n).round() as usize).max(1)
}

/// Export speed multiplier N parsed from `enc.speed` ("0.8"|"1"|"1.5"|"2"|"3"|"4").
/// Falls back to 1.0 on invalid values; clamped to a safe 0.5..8.0 range.
pub fn encode_speed_ratio(enc: &EncodeSettings) -> f64 {
    enc.speed
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|r| r.is_finite() && *r > 0.0)
        .map(|r| r.clamp(0.5, 8.0))
        .unwrap_or(1.0)
}

/// ffmpeg `atempo` filter chain that changes tempo by `ratio` while keeping the
/// pitch unchanged (each stage must stay within [0.5, 2.0], so larger/smaller
/// ratios are decomposed into multiplicative stages). Returns "" for 1.0.
pub fn atempo_chain(ratio: f64) -> String {
    if (ratio - 1.0).abs() <= 1e-6 {
        return String::new();
    }
    let mut parts: Vec<String> = Vec::new();
    let mut r = ratio;
    while r > 2.0 + 1e-6 {
        parts.push("atempo=2.0".to_string());
        r /= 2.0;
    }
    while r < 0.5 - 1e-6 {
        parts.push("atempo=0.5".to_string());
        r /= 0.5;
    }
    if (r - 1.0).abs() > 1e-6 {
        parts.push(format!("atempo={:.6}", r));
    }
    parts.join(",")
}

/// Resolve the composite output size from the `resolution` token + source dims.
/// Presets are 16:9; "source" keeps the video's own (often 4:3) aspect. A raw
/// "WxH" token is honoured too. Dimensions are forced even (yuv420 requirement).
pub fn encode_output_size(enc: &EncodeSettings, src_w: usize, src_h: usize) -> (usize, usize) {
    let even = |v: usize| (v.max(2)) - (v.max(2) % 2);
    let (w, h) = match enc.resolution.trim().to_ascii_lowercase().as_str() {
        "720p" | "hd" => (1280usize, 720usize),
        "1080p" | "fhd" | "" => (1920usize, 1080usize),
        "4k" | "uhd" | "2160p" => (3840usize, 2160usize),
        "source" => (src_w.max(2), src_h.max(2)),
        other => {
            // "1920x1080" style override.
            if let Some((a, b)) = other.split_once(['x', 'X']) {
                if let (Ok(w), Ok(h)) = (a.trim().parse::<usize>(), b.trim().parse::<usize>()) {
                    if w >= 2 && h >= 2 {
                        (w, h)
                    } else {
                        (1920, 1080)
                    }
                } else {
                    (1920, 1080)
                }
            } else {
                (1920, 1080)
            }
        }
    };
    (even(w), even(h))
}

/// Resolve the output frame rate from the `frame_rate` token + source fps.
pub fn encode_output_fps(enc: &EncodeSettings, src_fps: f64) -> f64 {
    match enc.frame_rate.trim().to_ascii_lowercase().as_str() {
        "source" | "" => src_fps,
        "60" => 60.0,
        "59.94" | "5994" => 59.94,
        "50" => 50.0,
        "30" => 30.0,
        "29.97" => 29.97,
        "25" => 25.0,
        "24" => 24.0,
        "23.976" => 23.976,
        other => other
            .parse::<f64>()
            .ok()
            .filter(|f| *f > 0.0)
            .unwrap_or(src_fps),
    }
}

/// 解析出**实际**要用的编码器：`(编码器名, 是否硬件, 有效格式)`。
///
/// 软件路径下会核对这个 ffmpeg 构建是否真有该编码器；没有就按
/// [`pick_software_fallback`] 换一个（必要时降级格式）。`build_command` 与
/// `FfmpegEncoder::open` 都必须走这里，否则"实际用的编码器"会和"记下来的编码器"
/// 不一致，错误归类与自动回退都会失效。
fn resolve_video_encoder(enc: &EncodeSettings, available: &[String]) -> (String, bool, String) {
    let fmt = enc.format.as_str();
    let backend = enc.backend.as_str();
    if backend != "software" {
        let base = match fmt {
            "hevc" => "hevc",
            "av1" => "av1",
            _ => "h264",
        };
        return (format!("{base}_{backend}"), true, fmt.to_string());
    }
    // 依次尝试该格式的候选编码器（AV1: SVT → libaom → rav1e），
    // 用第一个本机真的存在的。
    for c in software_fallback_candidates(fmt) {
        if available.iter().any(|e| e == c) {
            return ((*c).to_string(), false, fmt.to_string());
        }
    }
    // 该格式一个都没有 → 换格式（如 AV1 → H.264）。
    let (f2, c2) = pick_software_fallback(fmt, available);
    (c2, false, f2)
}

/// 描述这次导出**实际**会用的组合，给界面显示用。
///
/// 形如：`编码：中央处理器（AV1 · libsvtav1）｜解码：显卡硬解`。
/// 用户不用去翻进程命令行就能确认编码器选对没有 —— 尤其 CPU/QSV 都靠
/// 显卡解码，任务管理器里 GPU 一直在动是正常的，这行字能把两者说清楚。
pub fn encoder_status_text(enc: &EncodeSettings) -> String {
    let available = available_encoders();
    let (codec, hw, eff_fmt) = resolve_video_encoder(enc, &available);
    let fmt_name = format_display_name(&eff_fmt);
    let enc_part = if hw {
        // 硬件后端：说明走哪家，以及编码格式。
        let vendor = match enc.backend.as_str() {
            "qsv" => "核显 QSV",
            "nvenc" => "NVIDIA 显卡",
            "amf" => "AMD 显卡",
            other => other,
        };
        format!("编码：{vendor}硬件（{fmt_name} · {codec}）")
    } else {
        format!("编码：中央处理器（{fmt_name} · {codec}）")
    };
    // 解码固定走硬件（软件解码 2688×2016 约 520ms/帧，慢到不可用）。
    format!("{enc_part}｜解码：显卡硬解")
}

/// A running ffmpeg encode. Frames are pushed as raw RGBA (or RGB24).
pub struct FfmpegEncoder {
    proc: Option<Child>,
    pub width: usize,
    pub height: usize,
    pub fps: f64,
    pub frame_number: usize,
    pub total_frames: usize,
    output_path: String,
    temp_files: Vec<PathBuf>,
    /// 实际使用的 ffmpeg 编码器名（如 av1_qsv / libaom-av1），用于错误归类与回退。
    codec: String,
    /// 编码格式 token（h264|hevc|av1|mpeg4），用于面向用户的原因文案。
    format: String,
    /// 是否走硬件编码（软件编码不做"显卡不支持"的归类）。
    hw: bool,
    /// stderr 读取线程。ffmpeg 的 stderr 是管道，写满 64KB 就会阻塞 ffmpeg 主循环；
    /// 必须由独立线程持续抽走，而不是只在出错时读一次。
    stderr_reader: Option<std::thread::JoinHandle<String>>,
}

impl Drop for FfmpegEncoder {
    fn drop(&mut self) {
        for p in &self.temp_files {
            let _ = std::fs::remove_file(p);
        }
        if let Some(dir) = self.temp_files.first().and_then(|p| p.parent()) {
            let _ = std::fs::remove_dir(dir);
        }
    }
}

impl FfmpegEncoder {
    /// The configured output path (for progress/abort reporting).
    pub fn output_path_clone(&self) -> String {
        self.output_path.clone()
    }

    /// Build and spawn the ffmpeg encode. `segs` = stitched multi-video timeline
    /// (empty = single-file export from `src_video`).
    #[allow(clippy::too_many_arguments)]
    pub fn open(
        output_path: &str,
        src_video: &str,
        segs: &[crate::gui::VideoSeg],
        width: usize,
        height: usize,
        fps: f64,
        src_duration_ms: f64,
        enc: &EncodeSettings,
    ) -> Result<Self, String> {
        if (!src_video.is_empty() && crate::export_queue::path_key(src_video) == crate::export_queue::path_key(output_path))
            || segs.iter().any(|s| crate::export_queue::path_key(&s.path) == crate::export_queue::path_key(output_path)) {
            return Err(format!("输出路径与源视频冲突：{output_path}"));
        }
        // Multi-file exports need the pre-merged audio (no ffmpeg filters involved).
        // On extraction failure we degrade to a silent video instead of failing the encode.
        let mut temp_files: Vec<PathBuf> = Vec::new();
        let stitched: Option<String> =
            if segs.len() > 1 && !enc.map_only && enc.audio_codec != "none" {
                match prepare_stitched_audio(segs, src_duration_ms, enc) {
                    Ok(Some(p)) => {
                        let s = p.to_string_lossy().into_owned();
                        temp_files.push(p);
                        Some(s)
                    }
                    Ok(None) => None,
                    Err(e) => {
                        eprintln!("[encode] stitched audio failed, exporting without audio: {e}");
                        None
                    }
                }
            } else {
                None
            };
        let cmd = build_command(
            output_path,
            src_video,
            segs,
            width,
            height,
            fps,
            src_duration_ms,
            stitched.as_deref(),
            enc,
        );
        // 与 build_command 用同一套解析，记下实际选中的编码器 / 是否硬件 / 有效格式，
        // 这样出错时才能判断"是不是显卡不支持"。
        let (codec, use_hw, fmt_eff) = resolve_video_encoder(enc, &available_encoders());
        let fmt = fmt_eff.as_str();
        let mut child = background_command(&cmd[0])
            .args(&cmd[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("spawn ffmpeg: {e}"))?;
        // Drain stderr on its own thread for the whole encode: the pipe holds ~64 KB, and
        // a verbose ffmpeg would block on a full pipe if we only read it after a failure.
        // The thread owns the handle and returns everything it saw when ffmpeg exits.
        let stderr_reader = child.stderr.take().map(|mut se| {
            std::thread::spawn(move || {
                use std::io::Read;
                let mut buf = String::new();
                let _ = se.read_to_string(&mut buf);
                buf
            })
        });
        // Fail fast: an encoder that cannot open dies within milliseconds. Wait briefly,
        // then report — the caller turns this into a one-line reason (and possibly a CPU
        // fallback) instead of making the user read ffmpeg's internal dump.
        std::thread::sleep(std::time::Duration::from_millis(300));
        let mut stderr_reader = stderr_reader;
        let early_exit: Option<String> = match child.try_wait() {
            Ok(Some(status)) => {
                let mut err = stderr_reader
                    .take()
                    .map(|h| h.join().unwrap_or_default())
                    .unwrap_or_default();
                if err.trim().is_empty() {
                    err = format!("{status}");
                }
                Some(err)
            }
            _ => None,
        };
        if let Some(err) = early_exit {
            return Err(Self::startup_error(&err, &codec, fmt, use_hw));
        }
        let total_frames = clip_total_frames(enc, src_duration_ms, fps);
        Ok(Self {
            proc: Some(child),
            width,
            height,
            fps,
            frame_number: 0,
            total_frames,
            output_path: output_path.to_string(),
            temp_files,
            codec,
            format: fmt.to_string(),
            hw: use_hw,
            stderr_reader,
        })
    }

    /// 把"启动即失败"的 stderr 变成用户能看懂的一句话。
    ///
    /// 完整 stderr 永远进日志（见 [`Self::log_stderr`]），返回值只是简短原因：
    /// * 硬件编码器打不开 → 「显卡不支持 …」这类具体结论（并可触发 CPU 回退）；
    /// * 其它情况 → 保留原始错误，但同样记进日志。
    fn startup_error(stderr: &str, codec: &str, fmt: &str, use_hw: bool) -> String {
        Self::log_raw_stderr("编码器启动失败", stderr);
        if use_hw {
            if let Some(f) = hw_failure_reason(stderr, codec, fmt) {
                return mark_hw_failure(&f.reason);
            }
        }
        // 非硬件问题（路径非法、磁盘满、参数错误…）：给一句短说明 + 最后一行，
        // 详细信息在日志里。
        let last = stderr
            .lines()
            .rev()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("无输出");
        format!("编码器启动失败：{last}（详细信息见日志）")
    }

    /// 把 ffmpeg 的完整 stderr 写进日志文件。这样界面上只显示一句话，
    /// 而排查时完整信息仍在输出视频旁边的日志里。
    fn log_raw_stderr(stage: &str, stderr: &str) {
        let t = stderr.trim();
        if t.is_empty() {
            crate::log_warn!("{stage}：ffmpeg 未输出任何信息");
            return;
        }
        let lines = t.lines().count();
        crate::log_error!(
            "{stage}（ffmpeg 原始输出 {lines} 行，完整内容如下）：\n{t}"
        );
    }

    /// 取出 stderr 读取线程收集到的完整内容（线程随 ffmpeg 退出而结束）。
    fn take_stderr(&mut self) -> String {
        match self.stderr_reader.take() {
            Some(h) => h.join().unwrap_or_default(),
            None => String::new(),
        }
    }

    /// Write one RGB24 frame (width*height*3 bytes).
    pub fn write_frame(&mut self, rgb: &[u8]) -> Result<(), String> {
        let Some(proc) = self.proc.as_mut() else {
            return Err("编码器未打开".into());
        };
        let Some(stdin) = proc.stdin.as_mut() else {
            return Err("编码器输入已关闭".into());
        };
        stdin.write_all(rgb).map_err(|e| {
            // 管道断开 = ffmpeg 已经死了。真正的死因在 stderr 里：硬件编码器打不开时
            // 就是这里（用户看到的"写入帧失败: 管道已结束"）。
            let err = self.take_stderr();
            Self::log_raw_stderr("写入帧失败时的 ffmpeg 输出", &err);
            if self.hw {
                if let Some(f) = hw_failure_reason(&err, &self.codec, &self.format) {
                    return mark_hw_failure(&f.reason);
                }
            }
            let last = err
                .lines()
                .rev()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .unwrap_or("");
            if last.is_empty() {
                format!("写入帧失败：编码进程已退出（{e}）")
            } else {
                format!("写入帧失败：编码进程已退出（{last}）")
            }
        })?;
        self.frame_number += 1;
        Ok(())
    }

    /// Close the input and wait for the process, returning the output path.
    pub fn close(&mut self) -> Result<String, String> {
        let Some(mut proc) = self.proc.take() else {
            return Err("编码器未打开".into());
        };
        if let Some(mut stdin) = proc.stdin.take() {
            let _ = stdin.flush();
            drop(stdin);
        }
        // 90s cap to guard against a deadlock on the final fragment flush.
        let start = std::time::Instant::now();
        let mut code = -1;
        loop {
            if let Some(status) = proc.try_wait().map_err(|e| format!("wait: {e}"))? {
                code = status.code().unwrap_or(-1);
                break;
            }
            if start.elapsed().as_secs() > 90 {
                let _ = proc.kill();
                let _ = proc.wait();
                let err = self.take_stderr();
                Self::log_raw_stderr("收尾超时时的 ffmpeg 输出", &err);
                return Err("编码收尾超时（90 秒），已强制结束；输出文件可能不完整".into());
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let err = self.take_stderr();
        if code != 0 {
            Self::log_raw_stderr("编码失败时的 ffmpeg 输出", &err);
            if self.hw {
                if let Some(f) = hw_failure_reason(&err, &self.codec, &self.format) {
                    return Err(mark_hw_failure(&f.reason));
                }
            }
            let last = err
                .lines()
                .rev()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .unwrap_or("");
            return Err(if last.is_empty() {
                format!("编码失败（退出码 {code}；详细信息见日志）")
            } else {
                format!("编码失败：{last}（详细信息见日志）")
            });
        }
        Ok(self.output_path.clone())
    }

    /// Abort the encode, keeping the partially-written file playable.
    ///
    /// Closing stdin signals EOF so ffmpeg's fragmented-muxer finalizes the CURRENT
    /// fragment (a hard kill would leave a trailing fragment with a truncated mdat that
    /// ffprobe reports as `duration=N/A`). We wait a bounded time for that graceful exit
    /// and only hard-kill if ffmpeg is wedged.
    pub fn abort(&mut self) {
        if let Some(mut proc) = self.proc.take() {
            if let Some(mut stdin) = proc.stdin.take() {
                let _ = stdin.flush();
                drop(stdin);
            }
            let start = std::time::Instant::now();
            loop {
                match proc.try_wait() {
                    Ok(Some(_)) => break,
                    Ok(None) => {
                        if start.elapsed().as_secs() > 10 {
                            let _ = proc.kill();
                            let _ = proc.wait();
                        }
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                    Err(_) => {
                        let _ = proc.kill();
                        let _ = proc.wait();
                        break;
                    }
                }
            }
        }
    }
}

/// Pre-extract per-segment audio for a stitched multi-file export and join the pieces
/// into ONE lossless WAV file (48 kHz stereo s16le). Plain PCM byte concatenation is
/// exact — unlike the previous ADTS/AAC approach, where the first frames of a part
/// decoded differently inside the concatenated stream than on their own (measured: the
/// content at the part boundary was warped), and unlike the old filter_complex
/// approach, which dies on "lite" ffmpeg builds that lack the `concat` filter.
/// The final mux encodes the merged WAV to AAC in a single pass (no double encoding,
/// no per-part encoder priming at every seam).
/// Returns the merged file path, or None when no segment overlaps the clip range.
pub fn prepare_stitched_audio(
    segs: &[crate::gui::VideoSeg],
    src_duration_ms: f64,
    enc: &EncodeSettings,
) -> Result<Option<PathBuf>, String> {
    let (clip_start, clip_end) = clip_range(enc, src_duration_ms);
    let dir = std::env::temp_dir().join(format!("tracereel-audio-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("create temp dir: {e}"))?;
    let ffmpeg = find_ffmpeg();
    // Export speed: each per-segment extraction is atempo-compressed (pitch preserved),
    // so the merged WAV ends up clip_dur / N long — matching the N×-shorter video track.
    let chain = atempo_chain(encode_speed_ratio(enc));
    let mut parts: Vec<PathBuf> = Vec::new();
    let mut err: Option<String> = None;
    for (i, seg) in segs.iter().enumerate() {
        let ov_s = clip_start.max(seg.start_ms) - seg.start_ms;
        let ov_e = clip_end.min(seg.end_ms()) - seg.start_ms;
        let dur_s = ((ov_e - ov_s) / 1000.0).max(0.0);
        if dur_s <= 0.005 {
            continue;
        }
        let part = dir.join(format!("part{i:03}.pcm"));
        let mut c = background_command(&ffmpeg);
        c.args(["-y", "-hide_banner", "-loglevel", "error"]);
        if ov_s > 0.0 {
            c.args(["-ss", &format!("{:.3}", ov_s / 1000.0)]);
        }
        c.args(["-t", &format!("{:.3}", dur_s)]);
        c.arg("-i").arg(&seg.path);
        c.args(["-vn", "-f", "s16le", "-ar", "48000", "-ac", "2"]);
        if !chain.is_empty() {
            c.args(["-af", &chain]);
        }
        c.arg(&part);
        let out = match c.output() {
            Ok(o) => o,
            Err(e) => {
                err = Some(format!("spawn ffmpeg for segment {}: {e}", i + 1));
                break;
            }
        };
        if !out.status.success() {
            let m = String::from_utf8_lossy(&out.stderr).trim().to_string();
            let n = m.chars().count();
            let msg: String = if n > 200 {
                m.chars().skip(n - 200).collect()
            } else {
                m
            };
            err = Some(format!("audio extract failed for segment {}: {msg}", i + 1));
            break;
        }
        parts.push(part);
    }
    let mut merged_path: Option<PathBuf> = None;
    if err.is_none() && !parts.is_empty() {
        let merged = dir.join("merged.wav");
        match concat_to_wav(&parts, &merged) {
            Ok(()) => merged_path = Some(merged),
            Err(e) => err = Some(e),
        }
    }
    for p in &parts {
        let _ = std::fs::remove_file(p);
    }
    match err {
        Some(msg) => {
            let _ = std::fs::remove_dir(&dir);
            Err(msg)
        }
        None => Ok(merged_path),
    }
}

/// Crossfade length at stitched audio seams, in 48 kHz stereo frames (1440 = 30 ms).
const XFADE_FRAMES: usize = 1440;

/// Merge s16le stereo PCM chunks (4 bytes/frame) with a shortened crossfade at every
/// seam. The stitched parts are strictly adjacent on the recording timeline (no source
/// overlap to read), so this is the NLE-standard trick: blend the last `xf` output
/// frames with the first `xf` frames of the next chunk using equal-power Hann weights,
/// shortening the timeline by `xf` frames per seam. The ≤30 ms shift is inaudible and
/// absorbs the AAC frame-boundary gap that used to click at each original file seam.
fn merge_pcm_with_xfade(parts: &[Vec<u8>], xf: usize) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    for part in parts {
        let n = xf.min(out.len() / 4).min(part.len() / 4);
        if n == 0 {
            out.extend_from_slice(part);
            continue;
        }
        let a_off = out.len() - n * 4;
        let denom = (n - 1).max(1) as f64;
        let mut mixed = Vec::with_capacity(n * 4);
        for j in 0..n {
            // Hann fade-in weight for the incoming chunk (0 → 1); outgoing gets 1 - w.
            let w = 0.5 - 0.5 * (std::f64::consts::PI * j as f64 / denom).cos();
            for c in 0..2 {
                let ai = a_off + j * 4 + c * 2;
                let bi = j * 4 + c * 2;
                let a = i16::from_le_bytes([out[ai], out[ai + 1]]) as f64;
                let b = i16::from_le_bytes([part[bi], part[bi + 1]]) as f64;
                let m = (a * (1.0 - w) + b * w).round().clamp(-32768.0, 32767.0) as i16;
                mixed.extend_from_slice(&m.to_le_bytes());
            }
        }
        out.truncate(a_off);
        out.extend_from_slice(&mixed);
        out.extend_from_slice(&part[n * 4..]);
    }
    out
}

/// Concatenate s16le PCM parts behind a canonical 44-byte RIFF/WAVE header
/// (48 kHz, stereo, 16 bit). Sample alignment (4 bytes) is enforced per part so a
/// torn tail from one part cannot shift the channel pairing of the next.
fn concat_to_wav(parts: &[PathBuf], out: &Path) -> Result<(), String> {
    let mut chunks: Vec<Vec<u8>> = Vec::new();
    for p in parts {
        let bytes = std::fs::read(p).map_err(|e| format!("read audio part: {e}"))?;
        let n = bytes.len() - (bytes.len() % 4);
        chunks.push(bytes[..n].to_vec());
    }
    let pcm = merge_pcm_with_xfade(&chunks, XFADE_FRAMES);
    let data_len = pcm.len() as u32;
    let hdr = {
        let mut h = [0u8; 44];
        h[..4].copy_from_slice(b"RIFF");
        h[4..8].copy_from_slice(&(36 + data_len).to_le_bytes());
        h[8..12].copy_from_slice(b"WAVE");
        h[12..16].copy_from_slice(b"fmt ");
        h[16..20].copy_from_slice(&16u32.to_le_bytes());
        h[20..22].copy_from_slice(&1u16.to_le_bytes()); // PCM
        h[22..24].copy_from_slice(&2u16.to_le_bytes()); // stereo
        h[24..28].copy_from_slice(&48000u32.to_le_bytes());
        h[28..32].copy_from_slice(&(48000u32 * 4).to_le_bytes()); // byte rate
        h[32..34].copy_from_slice(&4u16.to_le_bytes()); // block align
        h[34..36].copy_from_slice(&16u16.to_le_bytes()); // bits per sample
        h[36..40].copy_from_slice(b"data");
        h[40..44].copy_from_slice(&data_len.to_le_bytes());
        h
    };
    let mut f = std::fs::File::create(out).map_err(|e| format!("create merged audio: {e}"))?;
    f.write_all(&hdr)
        .and_then(|_| f.write_all(&pcm))
        .map_err(|e| format!("write merged audio: {e}"))
}

/// Build the full ffmpeg argument array. `segs` is the stitched multi-video timeline
/// (global ms); empty = legacy single-file export from `src_video`.
/// `stitched_audio` = pre-merged WAV file for multi-segment exports (see
/// [`prepare_stitched_audio`]); ignored otherwise.
#[allow(clippy::too_many_arguments)]
pub fn build_command(
    output_path: &str,
    src_video: &str,
    segs: &[crate::gui::VideoSeg],
    width: usize,
    height: usize,
    fps: f64,
    src_duration_ms: f64,
    stitched_audio: Option<&str>,
    enc: &EncodeSettings,
) -> Vec<String> {
    let ffmpeg = find_ffmpeg();
    let mut cmd = vec![
        ffmpeg,
        "-y".into(),
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
    ];
    // rawvideo stdin (RGB24)
    cmd.push("-f".into());
    cmd.push("rawvideo".into());
    cmd.push("-pix_fmt".into());
    cmd.push("bgra".into());
    cmd.push("-s".into());
    cmd.push(format!("{width}x{height}"));
    cmd.push("-r".into());
    cmd.push(format!("{fps:.6}"));
    cmd.push("-i".into());
    cmd.push("-".into());

    // Resolve audio handling ONCE, up front. Stream copy preserves the original packet
    // timestamps, so a MID-FILE trim (input -ss) leaves the audio offset → A/V desync and a
    // wrong (over-long) container. When we actually trim and the user asked for `copy`,
    // transparently re-encode to AAC (which rebases to 0 and honours -t exactly). A full
    // export (no trim) keeps the lossless copy. Map-only has no audio input at all.
    let mut audio_vec: Vec<String> = Vec::new();
    if enc.map_only {
        cmd.push("-map".into());
        cmd.push("0:v:0".into());
        cmd.push("-an".into());
    } else if segs.len() > 1 {
        // Stitched multi-file export: audio arrives as ONE pre-merged WAV file
        // (see `prepare_stitched_audio`) — no per-segment inputs, no filters, so even
        // "lite" ffmpeg builds (missing `concat`) work. Encoded to AAC exactly once.
        audio_vec = audio_args(enc);
        let want_audio = !audio_vec.iter().any(|a| a == "-an");
        // NB: the `-i` group must come BEFORE `-map`/codec options — ffmpeg parses
        // options positionally, and `-map` before an `-i` is treated as that input's
        // option ("cannot be applied to input url ... Invalid argument").
        match stitched_audio {
            Some(merged) if want_audio => {
                cmd.push("-f".into());
                cmd.push("wav".into());
                cmd.push("-i".into());
                cmd.push(merged.to_string());
                cmd.push("-map".into());
                cmd.push("0:v:0".into());
                cmd.push("-map".into());
                cmd.push("1:a:0".into());
                audio_vec = vec![
                    "-c:a".into(),
                    "aac".into(),
                    "-b:a".into(),
                    enc.audio_bitrate.clone(),
                ];
            }
            _ => {
                cmd.push("-map".into());
                cmd.push("0:v:0".into());
                audio_vec = vec!["-an".into()];
            }
        }
    } else {
        let (clip_start, clip_end) = clip_range(enc, src_duration_ms);
        let clip_dur_s = ((clip_end - clip_start) / 1000.0).max(0.0);
        let src_dur_s = src_duration_ms / 1000.0;
        let trimmed = clip_start > 0.0 || (clip_dur_s > 0.0 && clip_dur_s < src_dur_s - 0.05);
        let ratio = encode_speed_ratio(enc);
        let speeded = (ratio - 1.0).abs() > 1e-6;
        audio_vec = audio_args(enc);
        if (trimmed || speeded) && enc.audio_codec == "copy" {
            // Speed change is a filter too — stream copy can't survive it.
            audio_vec = vec![
                "-c:a".into(),
                "aac".into(),
                "-b:a".into(),
                enc.audio_bitrate.clone(),
            ];
        }
        let has_audio = !audio_vec.iter().any(|a| a == "-an");
        if has_audio {
            // Input-side seek + fixed read length (both apply to the following `-i src_video`,
            // i.e. ONLY the audio — the rawvideo input 0 is untouched).
            if clip_start > 0.0 {
                cmd.push("-ss".into());
                cmd.push(format!("{:.3}", clip_start / 1000.0));
            }
            if clip_dur_s > 0.0 {
                cmd.push("-t".into());
                cmd.push(format!("{:.3}", clip_dur_s));
            }
        }
        cmd.push("-i".into());
        cmd.push(src_video.to_string());
        cmd.push("-map".into());
        cmd.push("0:v:0".into());
        if has_audio {
            cmd.push("-map".into());
            cmd.push("1:a:0?".into());
            if speeded {
                // Read the FULL source clip window, then time-compress with atempo
                // (pitch preserved): the filtered length becomes clip_dur / N which
                // matches the N×-shorter video track exactly.
                let chain = atempo_chain(ratio);
                if !chain.is_empty() {
                    audio_vec.push("-af".into());
                    audio_vec.push(chain);
                }
            }
        }
    }

    // codec
    let backend = enc.backend.as_str();
    let (codec, use_hw, eff_fmt) = resolve_video_encoder(enc, &available_encoders());
    let fmt = enc.format.as_str();
    if eff_fmt != fmt {
        crate::log_warn!(
            "本机 ffmpeg 没有 {} 的软件编码器，软件编码改用 {}（{codec}）",
            format_display_name(fmt),
            format_display_name(&eff_fmt)
        );
    }
    if use_hw {
        let (init, vf) = hw_device_args(backend);
        cmd.extend(init);
        cmd.extend(vf);
    }
    cmd.push("-c:v".into());
    cmd.push(codec.clone());
    cmd.extend(codec_args(&codec, backend, &eff_fmt, enc, use_hw));
    if eff_fmt == "hevc" {
        cmd.push("-tag:v".into());
        cmd.push("hvc1".into());
    }
    if !use_hw {
        cmd.push("-pix_fmt".into());
        cmd.push("yuv420p".into());
    }
    // Audio codec args (map-only already forced `-an`; empty vec = nothing to add).
    if !audio_vec.is_empty() {
        cmd.extend(audio_vec);
    }

    // fragmented mp4 + per-packet flush (see module docs)
    let ext = Path::new(output_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if matches!(ext.as_str(), "mp4" | "mov" | "m4v" | "3gp" | "m4a") {
        cmd.push("-movflags".into());
        cmd.push("frag_keyframe+empty_moov".into());
    }
    cmd.push("-flush_packets".into());
    cmd.push("1".into());
    cmd.push(output_path.to_string());
    cmd
}

fn codec_args(
    codec: &str,
    backend: &str,
    fmt: &str,
    e: &EncodeSettings,
    use_hw: bool,
) -> Vec<String> {
    let rc = e.rate_control.as_str();
    let kbps = e
        .bitrate
        .trim_end_matches('M')
        .trim_end_matches('m')
        .parse::<f64>()
        .map(|v| (v * 1000.0) as i64)
        .unwrap_or(2000)
        .max(100);
    let crf = e.quality.clamp(0.0, 51.0) as i64;
    let cbr = vec!["-b:v".into(), format!("{kbps}k")];
    let vbr = vec![
        "-b:v".into(),
        format!("{kbps}k"),
        "-maxrate".into(),
        format!("{}k", (kbps as f64 * 1.5) as i64),
        "-bufsize".into(),
        format!("{}k", kbps * 2),
    ];

    let mut args = Vec::new();
    if backend == "software" {
        if codec == "libx264" {
            args.push("-preset".into());
            args.push(e.preset.clone());
        } else if codec == "libx265" {
            args.push("-preset".into());
            args.push(e.preset.clone());
            args.push("-tag:v".into());
            args.push("hvc1".into());
        } else if codec == "libaom-av1" {
            args.extend(["-cpu-used".into(), "6".into(), "-row-mt".into(), "1".into()]);
        } else if codec == "libsvtav1" {
            // SVT-AV1 默认 preset 极慢；回退路径要的是"能跑完"。
            // 用户 preset 是 x264/x265 风格（medium/slow…）时不能直接用，给固定快速档。
            args.extend(["-preset".into(), "10".into()]);
        } else if codec == "librav1e" {
            args.extend(["-speed".into(), "8".into()]);
        } else if codec.contains("av1") {
            // 走到了这里说明 fmt=av1 但软件编码器既不是 libaom/libsvt/librav1e：
            // 只可能是 ffmpeg 把 -c:v 解析成了别的编码器（例如该构建没有软件 AV1，
            // ffmpeg 接受了一个不存在的编码器名）。**绝不能**再附加任何编码器私有
            // 选项（-cpu-used/-preset/-speed）—— 那会得到一句毫无线索的
            // "Unrecognized option"，让人以为是参数写错了。
            // 这里什么都不加，让 ffmpeg 自己报出"编码器不存在"这种可读错误。
        }
        if rc == "crf" {
            if fmt == "mpeg4" {
                args.extend(["-qscale:v".into(), crf.clamp(2, 31).to_string()]);
            } else {
                args.extend(["-crf".into(), crf.to_string()]);
            }
        } else if rc == "cbr" {
            args.extend(cbr);
        } else {
            args.extend(vbr);
        }
        return args;
    }

    // hardware
    if codec.contains("nvenc") {
        args.push("-preset".into());
        args.push(if e.preset.contains('p') {
            e.preset.clone()
        } else {
            "p4".into()
        });
        if rc == "crf" {
            args.extend([
                "-rc".into(),
                "vbr".into(),
                "-cq".into(),
                crf.to_string(),
                "-b:v".into(),
                "0".into(),
            ]);
        } else if rc == "cbr" {
            args.extend(["-rc".into(), "cbr".into()]);
            args.extend(cbr);
            args.extend([
                "-maxrate".into(),
                format!("{kbps}k"),
                "-bufsize".into(),
                format!("{}k", kbps * 2),
            ]);
        } else {
            args.extend(["-rc".into(), "vbr".into()]);
            args.extend(vbr);
        }
    } else if codec.contains("qsv") {
        args.push("-preset".into());
        args.push(e.preset.clone());
        if rc == "crf" {
            args.extend(["-global_quality".into(), crf.to_string()]);
        } else if rc == "cbr" {
            args.extend(["-maxrate".into(), format!("{kbps}k")]);
            args.extend(cbr);
        } else {
            args.extend(vbr);
        }
    } else if codec.contains("amf") {
        args.push("-quality".into());
        args.push(e.preset.clone());
        if rc == "crf" {
            args.extend([
                "-rc".into(),
                "cqp".into(),
                "-qp_i".into(),
                crf.to_string(),
                "-qp_p".into(),
                crf.to_string(),
            ]);
        } else if rc == "cbr" {
            args.extend(["-rc".into(), "cbr".into()]);
            args.extend(cbr);
        } else {
            args.extend(["-rc".into(), "vbr_peak".into()]);
            args.extend(vbr);
        }
    } else {
        let _ = use_hw;
        if rc == "crf" {
            args.extend(["-crf".into(), crf.to_string()]);
        } else if rc == "cbr" {
            args.extend(cbr);
        } else {
            args.extend(vbr);
        }
    }
    args
}

/// 硬件编码器不可用的判定结果：给用户看的一句话原因 + 给日志看的完整 stderr。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HwFailure {
    /// 界面/弹窗上显示的一句话原因（中文，不含英文技术名词之外的内容）。
    pub reason: String,
}

/// 错误串前缀：标记"这条失败是硬件编码器不可用"，供上层决定是否回退到 CPU。
/// 用前缀而不是靠字符串匹配中文原因，回退判断才不会被文案改动破坏。
pub const HW_FAILURE_MARK: &str = "\u{1}HWUNSUPPORTED\u{1}";

/// 把一句话原因包装成带硬件标记的错误串。
pub fn mark_hw_failure(reason: &str) -> String {
    format!("{HW_FAILURE_MARK}{reason}")
}

/// 错误串是否被标记为"硬件编码器不可用"，是则返回去掉标记的原因。
///
/// 标记可能被包在更外层的前缀里（例如 "写入帧失败：␁HWUNSUPPORTED␁…"），
/// 所以这里是**查找**而不是只判断开头 —— 否则自动回退会静默失效。
pub fn unmark_hw_failure(err: &str) -> Option<&str> {
    let i = err.find(HW_FAILURE_MARK)?;
    Some(&err[i + HW_FAILURE_MARK.len()..])
}

/// 硬件编码器为何打不开：显卡/驱动不支持该编码格式，还是别的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HwCause {
    /// 显卡或驱动不支持这个编码格式（核显不支持 AV1 是典型例子）。
    UnsupportedFormat,
    /// 驱动/设备初始化失败（CUDA/MFX/AMF 打不开设备，多为驱动过旧或设备被占用）。
    DeviceInit,
    /// stderr 里出现了编码器打不开的特征，但具体原因无法进一步区分。
    GenericOpenFailure,
}

impl HwCause {
    /// 面向用户的一句话原因。`codec` 是 ffmpeg 编码器名（如 av1_qsv），
    /// `fmt` 是格式 token（h264|hevc|av1|mpeg4）。
    pub fn reason(self, codec: &str, fmt: &str) -> String {
        let fmt_name = format_display_name(fmt);
        match self {
            HwCause::UnsupportedFormat => format!(
                "显卡不支持 {fmt_name} 硬件编码（编码器 {codec} 无法启动）；\
                 请把编码后端改为「软件」，或把编码格式改为该显卡支持的格式。"
            ),
            HwCause::DeviceInit => format!(
                "显卡驱动初始化失败（编码器 {codec} 无法打开设备）；\
                 可能是显卡驱动过旧或设备被其他程序占用，请更新驱动后重试。"
            ),
            HwCause::GenericOpenFailure => format!(
                "显卡硬件编码器 {codec} 无法启动（{fmt_name}）；\
                 可能是该显卡或当前驱动不支持此格式，建议改用软件编码。"
            ),
        }
    }
}

/// 格式 token → 界面显示名（产品名/格式名保留原文，符合项目的命名约定）。
pub fn format_display_name(fmt: &str) -> &'static str {
    match fmt {
        "hevc" => "H.265/HEVC",
        "av1" => "AV1",
        "mpeg4" => "MPEG-4",
        "h264" => "H.264",
        _ => "该格式",
    }
}

/// 把 ffmpeg 的 stderr 归类成"硬件编码器不可用"。
///
/// ffmpeg 在编码器打不开时会打印十几行内部信息（"Error while opening encoder…"、
/// "Error sending frames to consumers…"、"Nothing was written into output file…"），
/// 这些对用户没有意义，只应该进日志。返回值 `Some(cause)` 表示可以给出一句
/// 具体原因；`None` 表示这是别的问题（磁盘满、输出路径非法、参数错误等），
/// 调用方应保留原始错误。
pub fn classify_hw_failure(stderr: &str) -> Option<HwCause> {
    let s = stderr.to_ascii_lowercase();

    // 必须是"编码器打不开"这一类，否则不认（避免把磁盘满误判成显卡问题）。
    let opened_encoder = s.contains("error while opening encoder")
        || s.contains("could not open encoder")
        || s.contains("nothing was written into output file")
        || s.contains("error initializing output stream")
        // 这个 ffmpeg 构建里根本没有这个编码器（例如选了某个后端/格式，而该构建
        // 没编进去）。等价于"硬件路径不可用"，同样应该回退到 CPU。
        || s.contains("unknown encoder")
        || s.contains("error selecting an encoder")
        || s.contains("encoder not found");
    if !opened_encoder {
        return None;
    }

    // 编码器压根不存在 —— 与"显卡不支持"是同一类问题：当前硬件路径走不通。
    if s.contains("unknown encoder")
        || s.contains("error selecting an encoder")
        || s.contains("encoder not found")
    {
        return Some(HwCause::UnsupportedFormat);
    }

    // 设备/驱动初始化失败：驱动过旧、设备不存在、设备被占用。
    const DEVICE_MARKERS: &[&str] = &[
        "cuda_error_no_device",
        "cuinit(0) failed",
        "no cuda-capable device",
        "cannot load nvcuda.dll",
        "no nvidia devices",
        "mfx session",
        "mfx_err_not_initialized",
        "mfx_err_unsupported",
        "device creation failed",
        "failed to create a mfx session",
        "amf failed to initialize",
        "d3d11va",
        "no device available",
        "failed to initialize",
        "cannot open device",
    ];
    if DEVICE_MARKERS.iter().any(|m| s.contains(m)) {
        return Some(HwCause::DeviceInit);
    }

    // 编码器存在但就是开不起来 —— 典型就是核显/驱动不支持该格式。
    // ffmpeg 的 "Function not implemented" 是 QSV/部分驱动不支持该编码格式时的
    // 直接信号；"invalid argument" 则是参数/格式组合被拒绝。
    const UNSUPPORTED_MARKERS: &[&str] = &[
        "function not implemented",
        "not implemented",
        "unsupported",
        "invalid argument",
        "invalid or unsupported",
    ];
    if UNSUPPORTED_MARKERS.iter().any(|m| s.contains(m)) {
        return Some(HwCause::UnsupportedFormat);
    }

    Some(HwCause::GenericOpenFailure)
}

/// 从 ffmpeg stderr 里提炼"硬件编码器不可用"的结论，仅用于**硬件后端**。
/// 软件后端的同类报错（例如 libaom 不支持某参数）不走这个判定。
pub fn hw_failure_reason(stderr: &str, codec: &str, fmt: &str) -> Option<HwFailure> {
    classify_hw_failure(stderr).map(|cause| HwFailure {
        reason: cause.reason(codec, fmt),
    })
}

fn audio_args(e: &EncodeSettings) -> Vec<String> {
    let mode = e.audio_codec.as_str();
    if mode == "none" {
        return vec!["-an".into()];
    }
    if mode == "copy" {
        return vec!["-c:a".into(), "copy".into()];
    }
    let codec = match mode {
        "aac" => "aac",
        "mp3" => "libmp3lame",
        "opus" => "libopus",
        _ => "copy",
    };
    vec![
        "-c:a".into(),
        codec.into(),
        "-b:a".into(),
        e.audio_bitrate.clone(),
    ]
}

/// Stabilization inputs for an encode. (The GUI resolves readiness itself and hands
/// the worker a plain `Option<Arc<Stabilizer>>`; this enum is kept for CLI callers.)
#[derive(Clone)]
pub enum StabInput {
    /// Gyroflow stabilization is off for this encode.
    Disabled,
    /// A stabilizer loaded and ready to process frames.
    Ready(std::sync::Arc<crate::gyro::Stabilizer>),
    /// Enabled but not available yet: carries the reason shown to the user.
    Unavailable(String),
}

impl StabInput {
    pub fn as_option(&self) -> Option<std::sync::Arc<crate::gyro::Stabilizer>> {
        match self {
            StabInput::Ready(s) => Some(s.clone()),
            _ => None,
        }
    }
}

/// Snapshot of encode progress polled by the GUI from the worker thread.
#[derive(Default, Clone)]
pub struct EncodeProgress {
    pub frame: usize,
    pub total_frames: usize,
    pub elapsed_s: f64,
    pub remaining_s: f64,
    pub fps: f64,
    pub out_size: i64,
    pub current_t_ms: f64,
    pub duration_ms: f64,
}

/// Terminal outcome of a full export, read once by the GUI after `done`.
#[derive(Default, Clone)]
pub struct EncodeResult {
    pub output: String,
    pub aborted: bool,
    pub error: String,
}

/// Shared control + status for a running encode.
pub struct EncodeHandle {
    pub stop: Arc<std::sync::atomic::AtomicBool>,
    pub pause: Arc<std::sync::atomic::AtomicBool>,
    status: Mutex<EncodeProgress>,
    result: Mutex<Option<EncodeResult>>,
    done: Arc<std::sync::atomic::AtomicBool>,
    /// 逐段导出时给弹窗显示的当前分段说明（如「第 2/5 段 [1-2]」）；空 = 非逐段。
    segment_label: Mutex<String>,
    batch_position: Mutex<(usize, usize)>,
    /// 非致命的提示（例如"显卡不支持该编码格式，已自动改用 CPU"）。界面把它显示成
    /// 警告，而不是失败：导出仍在继续/已成功。空 = 无提示。
    notice: Mutex<String>,
}

impl Default for EncodeHandle {
    fn default() -> Self {
        Self {
            stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            pause: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            status: Mutex::new(EncodeProgress::default()),
            result: Mutex::new(None),
            done: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            segment_label: Mutex::new(String::new()),
            batch_position: Mutex::new((0, 1)),
            notice: Mutex::new(String::new()),
        }
    }
}

impl EncodeHandle {
    pub fn overall_progress(&self) -> f64 {
        let (completed, count) = *self.batch_position.lock().unwrap();
        let p = self.get();
        let fraction = if p.total_frames > 0 { p.frame as f64 / p.total_frames as f64 } else { 0.0 };
        ((completed as f64 + fraction) / count.max(1) as f64).clamp(0.0, 1.0)
    }
    fn set(&self, p: EncodeProgress) {
        *self.status.lock().unwrap() = p;
    }
    /// Public progress reporter (used by the GUI pump and headless harnesses).
    pub fn set_progress(&self, p: EncodeProgress) {
        self.set(p);
    }
    pub fn get(&self) -> EncodeProgress {
        self.status.lock().unwrap().clone()
    }
    /// Record the terminal outcome (before `mark_done`).
    pub fn set_result(&self, r: EncodeResult) {
        *self.result.lock().unwrap() = Some(r);
    }
    /// Take the terminal outcome; None until the worker sets it.
    pub fn take_result(&self) -> Option<EncodeResult> {
        self.result.lock().unwrap().take()
    }
    /// Snapshot of the last recorded result without taking it (used by the segment
    /// sequence to stop early when a segment fails).
    pub fn peek_result(&self) -> Option<EncodeResult> {
        self.result.lock().unwrap().clone()
    }
    /// 清掉上一次尝试留下的终止状态（但**不动** done 标志）。
    /// 自动回退到 CPU 成功后必须调用：否则逐段导出会在 `peek_result()` 里看到
    /// 第一次失败的 error，误判这一段失败并终止整个批次。
    pub fn clear_stale_result(&self) {
        *self.result.lock().unwrap() = None;
    }
    pub fn mark_done(&self) {
        self.done.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    /// 设置/读取当前分段说明（仅逐段导出使用，普通导出始终为空串）。
    pub fn set_segment_label(&self, s: String) {
        *self.segment_label.lock().unwrap() = s;
    }
    pub fn segment_label(&self) -> String {
        self.segment_label.lock().unwrap().clone()
    }
    /// 发布一条非致命提示（会自动换行累积？不 —— 直接覆盖，同一次导出只会有一条）。
    pub fn set_notice(&self, s: String) {
        *self.notice.lock().unwrap() = s;
    }
    /// 读取非致命提示（无提示 = 空串）。
    pub fn notice(&self) -> String {
        self.notice.lock().unwrap().clone()
    }
    pub fn is_done(&self) -> bool {
        self.done.load(std::sync::atomic::Ordering::SeqCst)
    }
    pub fn stop_flag(&self) -> Arc<std::sync::atomic::AtomicBool> {
        Arc::clone(&self.stop)
    }
    pub fn pause_flag(&self) -> Arc<std::sync::atomic::AtomicBool> {
        Arc::clone(&self.pause)
    }
    pub fn done_flag(&self) -> Arc<std::sync::atomic::AtomicBool> {
        Arc::clone(&self.done)
    }
}

/// Qt-free composite inputs (map bitmaps + per-map route managers, gpx track) built
/// once per export. All fields are plain data so this moves onto the worker thread.
/// `maps` is ordered by `start_ms` (main map first) and ALWAYS has at least one entry
/// (a placeholder keeps the video-only export path working when no map is configured).
pub struct EncodePrep {
    pub(crate) maps: Vec<crate::gui::PreparedMap>,
    pub gpx: Option<tracereel_core::gpx::GpxTrack>,
}

impl EncodePrep {
    /// Load + downscale every map (main + 额外地图), initialize one RouteManager per
    /// map, parse the gpx. Reads the QuickRoute JPEGs and map images from disk (no
    /// Qt) — runs on the encode thread.
    pub fn build(settings: &Settings, out_w: usize, out_h: usize) -> Result<Self, String> {
        if settings.encode.include_video
            && !settings.encode.map_only
            && !std::path::Path::new(&settings.video.video_path).exists()
        {
            return Err("请先设置有效的视频文件路径（视频页）".into());
        }
        let view_mode = crate::gui::view_mode_from_settings(settings);
        let route_style = crate::gui::route_style_from_settings(settings);
        let window = (out_w.max(2) as f64, out_h.max(2) as f64);
        let jobs = if settings.encode.include_map || settings.encode.map_only {
            crate::gui::map_jobs(settings)
        } else {
            Vec::new()
        };
        let mut maps = Vec::with_capacity(jobs.len().max(1));
        for (map_path, route_path, start_ms) in jobs {
            let (m, err) = crate::gui::prepare_one_map(
                &map_path,
                &route_path,
                start_ms,
                settings,
                &settings.splits.split_times,
                settings.splits.is_relative,
                route_style.clone(),
                window,
                view_mode,
            );
            if let Some(e) = err {
                return Err(e);
            }
            maps.push(m);
        }
        if maps.is_empty() {
            maps.push(crate::gui::PreparedMap {
                rgba: Vec::new(),
                w: 0,
                h: 0,
                start_ms: 0.0,
                rm: Default::default(),
            });
        }
        let gpx = if settings.encode.include_hud {
            let gpx_path = if !settings.hud.gpx_file_path.is_empty() {
                settings.hud.gpx_file_path.clone()
            } else {
                tracereel_core::gpx::find_gpx_for_video(&settings.video.video_path)
            };
            if !gpx_path.is_empty() {
                tracereel_core::gpx::parse_gpx(&gpx_path).ok()
            } else {
                None
            }
        } else {
            None
        };
        Ok(Self { maps, gpx })
    }
}

/// Everything a full export run needs, moved onto the encode thread. Pure data + the
/// shared handle + an optional already-resolved stabilizer.
#[derive(Clone)]
pub struct EncodeJob {
    pub settings: Settings,
    pub out_path: String,
    /// Source (native video) geometry + frame rate.
    pub src_w: usize,
    pub src_h: usize,
    pub src_fps: f64,
    /// Decode / stabilizer buffer size — the source aspect ratio, capped (matches the
    /// size the stabilizer was initialised at). `draw_scene` letterboxes this AR-preserved
    /// frame into the output panel, so the video is never stretched.
    pub dec_w: usize,
    pub dec_h: usize,
    pub duration_ms: f64,
    pub clip_start: f64,
    pub clip_end: f64,
    pub handle: Arc<EncodeHandle>,
    /// Per-segment stabilizers aligned with `segs` (None = that segment exports raw).
    pub stabs: Vec<Option<Arc<Stabilizer>>>,
    /// The stitched multi-video timeline (end-to-end, global ms). Empty = legacy
    /// single-file export from `settings.video.video_path`.
    pub segs: Vec<crate::gui::VideoSeg>,
}

/// 逐段导出的分段文件名：在扩展名前插入 `[a-b]`。
/// `test.mp4` + (0,1) → `test[0-1].mp4`；无扩展名时直接追加。
pub fn segment_output_path(out: &str, a: i32, b: i32) -> String {
    let p = Path::new(out);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("output");
    let dir = p.parent().unwrap_or_else(|| Path::new(""));
    let name = match p.extension().and_then(|s| s.to_str()) {
        Some(ext) if !ext.is_empty() => format!("{stem}[{a}-{b}].{ext}"),
        _ => format!("{stem}[{a}-{b}]"),
    };
    dir.join(name).to_string_lossy().into_owned()
}

/// 全部逐段导出：在同一后台线程上串行编码多个分段任务（每个分段一个文件）。
/// 所有 job 必须共享同一个 `handle`：进度持续发布到该 handle，用户中止（stop）
/// 会取消整个批次；某一段失败也会停止后续分段并把错误作为整批的结果上报。
/// 只有最后一个 job 完成后才翻转 `done`，GUI 全程轮询同一个 handle 即可。
pub fn encode_export_segments(jobs: Vec<EncodeJob>) {
    let Some(first) = jobs.first() else { return };
    let handle = first.handle.clone();
    let n = jobs.len();
    for (i, job) in jobs.into_iter().enumerate() {
        *handle.batch_position.lock().unwrap() = (i, n);
        handle.set_progress(EncodeProgress::default());
        let last = i + 1 == n;
        handle.set_segment_label(format!("第 {}/{} 段 [{}-{}]", i + 1, n, i, i + 1));
        eprintln!("[encode] 逐段导出 {}/{} -> {}", i + 1, n, job.out_path);
        // 返回值 = 本次是否发生了"硬件编码不可用 → 已回退到 CPU"；回退后这一段已经
        // 成功，批次继续往下走，不应该被当成失败终止。
        let _fell_back = encode_export_run(job, last);
        if last {
            return;
        }
        // 中止或失败：停止批次，把终止状态挂到共享 handle 上让 GUI 收尾。
        if handle.stop.load(Ordering::SeqCst) {
            handle.set_result(EncodeResult {
                output: String::new(),
                aborted: true,
                error: "逐段导出已中止".into(),
            });
            handle.mark_done();
            return;
        }
        if handle
            .peek_result()
            .map(|r| !r.error.is_empty())
            .unwrap_or(false)
        {
            handle.mark_done();
            return;
        }
    }
}

/// Run a complete export on the current (worker) thread: decode → stabilize →
/// composite with QPainter (off-main, incl. HUD text — proven safe here) → feed ffmpeg.
/// Publishes progress to the handle and, at the end, the terminal outcome. Never touches
/// the preview/GUI state. `done` is set only after the encoder is closed/aborted.
pub fn encode_export(job: EncodeJob) {
    encode_export_run(job, true);
}

/// Same as `encode_export`, but `mark_done` controls whether the shared handle's `done`
/// flag is flipped at the end. The 逐段导出 sequence (`encode_export_segments`) runs all
/// but the last segment with `mark_done = false` so the GUI keeps polling one handle
/// across the whole batch.
///
/// 这里还负责**硬件编码器不可用时自动改用 CPU**：显卡（尤其核显）不支持所选编码格式
/// 时，硬件编码器根本打不开。用户不需要读懂 ffmpeg 的报错，也不需要手动改设置 ——
/// 这里给出一条警告说明原因，然后直接用软件编码重新导出。
/// 每次导出都会重新先试硬件（驱动升级后能自动恢复用 GPU），所以只对**本次**回退。
fn encode_export_run(job: EncodeJob, mark_done: bool) -> bool {
    if job.settings.encode.map_only {
        encode_export_map_only_run(job, mark_done);
        return false;
    }
    // 把完整日志放到输出视频所在目录（用户要发给别人排查时日志就在旁边）。
    crate::encode_log::set_log_dir_for_output(&job.out_path);
    let handle = job.handle.clone();
    let hw_backend = job.settings.encode.backend.clone();
    // 记录导出开始的原因，日志里能对上界面上的那次导出。
    crate::log_info!(
        "开始导出 -> {}（编码后端={} 格式={} 分辨率={} 帧率={}）",
        job.out_path,
        hw_backend,
        job.settings.encode.format,
        job.settings.encode.resolution,
        job.settings.encode.frame_rate
    );
    let failed_hw = encode_export_attempt(job.clone(), /*publish_done=*/ false);
    // 只有被标记为"硬件编码器不可用"的失败才回退；磁盘满、路径非法等照常报错。
    let Some(marked) = failed_hw else {
        // 第一次尝试就成功。
        if mark_done {
            handle.mark_done();
        }
        return false;
    };
    // 非硬件问题（或已经是软件后端）：直接发布失败结果，不回退。
    let reason = match unmark_hw_failure(&marked) {
        Some(r) if hw_backend != "software" => r.to_string(),
        _ => {
            crate::log_info!("导出失败（非硬件编码问题或已是软件后端），不回退到 CPU：{marked}");
            if mark_done {
                handle.mark_done();
            }
            return false;
        }
    };
    let fmt = job.settings.encode.format.clone();
    // 挑一个本机真的存在的软件编码器：如果这个 ffmpeg 构建没有该格式的软件编码器
    // （例如 gpl-lite 没有 libaom-av1），就换格式到 H.264，否则"回退"会再次失败。
    let (fb_fmt, sw) = pick_software_fallback(&fmt, &available_encoders());
    let changed_format = fb_fmt != fmt;
    crate::log_warn!(
        "硬件编码器不可用（后端={hw_backend} 格式={fmt}）：{reason}\n\
         已自动改用 CPU 软件编码（{sw}，格式={fb_fmt}）重新导出，并保留本次硬件编码的完整报错日志。"
    );
    // 提示用户：不是静默降级 —— 明确说明原因、以及这次会在 CPU 上慢一些。
    let fmt_note = if changed_format {
        format!(
            "\n本机 ffmpeg 没有 {} 的软件编码器，已改用 H.264 输出。",
            format_display_name(&fmt)
        )
    } else {
        String::new()
    };
    handle.set_notice(format!(
        "{reason}\n已自动改用中央处理器（CPU）软件编码（{sw}）重新导出；\
         本次导出速度会明显变慢，画质设置保持不变。{fmt_note}"
    ));
    let mut retry = job;
    retry.settings.encode.backend = "software".into();
    retry.settings.encode.format = fb_fmt.clone();
    crate::log_info!("回退重试：后端=software 编码器={sw} 格式={fb_fmt}");
    // 第一次尝试刻意没有发布 done，所以这里直接重试即可；重试才是最后一次尝试，
    // 由它发布终止状态。
    // 回退后从第 0 帧重新导出：编码器必须在写入任何一帧之前就确定，而失败的
    // 那次最多只写进了几百毫秒。ffmpeg 用 `-y`，会直接截断这个残留文件。
    let again = encode_export_attempt(retry, mark_done);
    if let Some(second) = again {
        crate::log_error!("改用 CPU 软件编码后仍然失败：{second}");
    } else {
        // 回退成功：第一次失败留下的结果必须先清掉，否则逐段导出会把它当成
        // "这一段失败了"而终止整个批次。
        handle.clear_stale_result();
        crate::log_info!("改用 CPU 软件编码后导出成功。");
    }
    true
}

/// 跑一次导出尝试。成功返回 `None`；失败返回错误串。若失败是"硬件编码器不可用"，
/// 返回的串带 [`HW_FAILURE_MARK`] 前缀，调用方据此决定是否回退到 CPU。
///
/// `publish_done = false` 时**不翻转** handle 的 done 标志：硬件失败后还要用 CPU 重试，
/// 这时若先把 done 置位，GUI（以及 encgui 这类轮询者）会立刻收尾退出，回退就白做了。
/// 终止状态只在最后一次尝试时发布。
fn encode_export_attempt(job: EncodeJob, publish_done: bool) -> Option<String> {
    if job.settings.encode.map_only {
        return None;
    }
    let EncodeJob {
        mut settings,
        out_path,
        src_w,
        src_h,
        src_fps,
        dec_w,
        dec_h,
        duration_ms,
        clip_start,
        clip_end,
        handle,
        stabs,
        segs,
    } = job;
    if !settings.encode.include_map {
        settings.map.relative_width = 0.0;
    }
    if !settings.encode.include_hud {
        settings.hud.layout = Some(crate::hud::Layout {
            version: 1,
            widgets: Vec::new(),
        });
    }
    if !settings.encode.include_video {
        settings.encode.audio_codec = "none".into();
    }
    // Effective stitched timeline: GUI jobs carry the multi-video segments; an empty
    // list means a legacy single-file caller — synthesize one segment so the frame
    // loop below has exactly one code path.
    let segs: Vec<crate::gui::VideoSeg> = if segs.is_empty() {
        vec![crate::gui::VideoSeg {
            path: settings.video.video_path.clone(),
            start_ms: 0.0,
            duration_ms,
            fps: src_fps,
            frame_count: 0,
            has_motion: false,
        }]
    } else {
        segs
    };
    // 结束时把结果交给 handle。返回值 = 本次尝试的失败原因（成功为 None），
    // 让上层决定是否需要回退到 CPU 重试。
    let finish = |aborted: bool, error: String| -> Option<String> {
        handle.set_result(EncodeResult {
            output: out_path.clone(),
            aborted,
            error: error.clone(),
        });
        if publish_done {
            handle.mark_done();
        }
        if error.is_empty() {
            None
        } else {
            Some(error)
        }
    };
    // Target output geometry + frame rate come from the encode page presets, NOT the
    // preview window. The RouteManager is laid out at the same size so the map / route
    // proportions match the preview at this resolution.
    let (out_w, out_h) = encode_output_size(&settings.encode, src_w, src_h);
    let out_fps = encode_output_fps(&settings.encode, src_fps);
    let (dec_w, dec_h) = if dec_w >= 2 && dec_h >= 2 {
        (dec_w, dec_h)
    } else {
        crate::gui::processing_size(src_w, src_h, out_h)
    };
    // Export speed N: sample source time N× faster (frame-skip), output fps unchanged,
    // so the clip holds total/N frames and is N× shorter. Audio matches via atempo.
    let speed_n = encode_speed_ratio(&settings.encode);
    let total =
        (((((clip_end - clip_start) / 1000.0) * out_fps) / speed_n).round() as usize).max(1);
    let dt_out = 1000.0 / out_fps;
    // Optional per-stage profiling (TRACEREEL_TIME=1): decode / stabilize / draw / pack / write.
    let prof = std::env::var("TRACEREEL_TIME").is_ok();
    let mut t_dec = 0.0f64;
    let mut t_stab = 0.0f64;
    let mut t_draw = 0.0f64;
    let mut t_pack = 0.0f64;
    let mut t_wr = 0.0f64;
    // Publish an initial progress BEFORE the slow startup (ffmpeg spawn + seek + gyroflow
    // warm-up) so the dialog shows the correct total / clip range immediately instead of 0.
    handle.set_progress(EncodeProgress {
        frame: 0,
        total_frames: total,
        elapsed_s: 0.0,
        remaining_s: 0.0,
        fps: 0.0,
        out_size: 0,
        current_t_ms: clip_start,
        duration_ms: clip_end,
    });
    let mut prep = match EncodePrep::build(&settings, out_w, out_h) {
        Ok(p) => p,
        Err(e) => {
            return finish(false, e);
        }
    };
    let mut enc = match FfmpegEncoder::open(
        &out_path,
        &settings.video.video_path,
        &segs,
        out_w,
        out_h,
        out_fps,
        duration_ms,
        &settings.encode,
    ) {
        Ok(e) => e,
        Err(e) => {
            return finish(false, e);
        }
    };
    // Warm-up (pre-roll) the RouteManager's runner smoothing. The camera transform is a
    // moving average with a multi-second memory; a clip that starts mid-course would
    // otherwise "settle" over its first seconds (a visible rotate/slide-in) because it has
    // no earlier samples. Feeding the preceding ~10s of updates (never rendered) makes the
    // first exported frame match what a full export would show at that time — i.e. the
    // clip looks exactly like a segment cut out of a normal export.
    if clip_start > 0.0 {
        let preroll_s = 10.0f64;
        let warm_end = clip_start / 1000.0;
        let mut wt = (warm_end - preroll_s).max(0.0);
        let step = dt_out / 1000.0;
        while wt < warm_end {
            // Warm up EVERY map's runner-smoothing, not just the one active at
            // clip_start: a later map switch must not "settle" on its first frame.
            for m in &mut prep.maps {
                m.rm.update(wt, dt_out);
            }
            wt += step;
        }
    }
    let t0 = std::time::Instant::now();
    let mut count = 0usize;
    let stop = handle.stop_flag();
    let pause = handle.pause_flag();
    let mut paused_secs = 0.0f64;

    // ── Frame production loop ────────────────────────────────────────────────────────
    // Decode → (optional stabilize) → RouteManager advance → QPainter composite → BGRA →
    // ffmpeg stdin, one frame at a time, on the export worker thread (off the GUI).
    //
    // This stays SINGLE-THREADED (a frame-sharded compositor was measured: byte-identical
    // but no real speedup — the QPainter composite is memory-bandwidth-bound — and the shared
    // gyroflow Stabilizer is not concurrency-safe, so it blacked-out frames). The real export
    // cost is DECODE, not composite (measured ~520 ms/frame *software* on the 2688×2016 DJI
    // vs ~12 ms draw), so we decode through the same **hardware** d3d11va/QSV ffmpeg-subprocess
    // PipeDecoder the preview uses (lighting the GPU's Video Decode engine), falling back to the
    // in-process software SoftDecoder if the hardware chain is unavailable/dies.
    // ── Segment-aware decoding ──
    // The export walks the stitched timeline; on each file crossing both decoders are
    // rebuilt for that segment and the per-segment stabilizer is swapped in. The
    // hardware chain needs ~0.3-0.5s to deliver its first frame after a (re)spawn, and
    // every miss during that window used to fall through to slow software decode — the
    // visible "stuck at the seam". Two fixes: the next segment's pipe is pre-warmed
    // shortly BEFORE the seam, and export (offline, no realtime constraint) now BLOCKS
    // at the current timestamp instead of dropping past a cold-start miss.
    let mut cur_seg = usize::MAX;
    let mut decoder: Option<crate::video::SoftDecoder> = None;
    let mut pipe: Option<crate::pipe::PipeDecoder> = None;
    let mut pipe_ok = false;
    // Next segment's pipe, spawned ahead of the seam (segment index, decoder). Only
    // stored once its spawn actually succeeded.
    let mut warmed: Option<(usize, crate::pipe::PipeDecoder)> = None;
    let mut stab: Option<Arc<Stabilizer>> = None;
    // One QImage per prepared map + its start times for timeline lookup (aligned).
    let map_imgs: Vec<Option<_>> = prep
        .maps
        .iter()
        .map(|m| {
            if m.w > 0 {
                Some(crate::qtwrap::qimage_from_rgba(m.w, m.h, &m.rgba))
            } else {
                None
            }
        })
        .collect();
    let map_starts: Vec<f64> = prep.maps.iter().map(|m| m.start_ms).collect();
    let mut t_ms = clip_start;
    let mut pause_since: Option<std::time::Instant> = None;
    while count < total && !stop.load(Ordering::SeqCst) {
        if pause.load(Ordering::SeqCst) {
            if pause_since.is_none() {
                pause_since = Some(std::time::Instant::now());
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
            continue;
        }
        if let Some(ps) = pause_since.take() {
            paused_secs += ps.elapsed().as_secs_f64();
        }
        let _t = std::time::Instant::now();
        // Global clock → owning segment + file-local time; the stabilizer is fed LOCAL
        // timestamps (each segment has its own motion data), everything user-facing
        // (route/HUD/progress) is fed GLOBAL time.
        let (si, t_local) = crate::gui::locate_video_seg(&segs, t_ms);
        let dt_i = 1000.0 / segs[si].fps.max(1.0);
        let ts = ((t_local / dt_i).round() * dt_i).max(0.0);
        let fi = (ts / dt_i).round() as usize;
        let t_global = segs[si].start_ms + ts;
        if si != cur_seg {
            cur_seg = si;
            decoder = match crate::video::SoftDecoder::open(&segs[si].path, dec_w, dec_h) {
                Ok(d) => Some(d),
                Err(e) => {
                    return finish(false, format!("解码器启动失败（分段 {}）: {e}", si + 1));
                }
            };
            match warmed.take().filter(|(wi, _)| *wi == si) {
                // Pre-warmed before the seam: chain is already up, first frames in ring.
                Some((_, wp)) => {
                    pipe = Some(wp);
                    pipe_ok = true;
                    eprintln!("[encode] seam adopted pre-warmed pipe seg {}", si + 1);
                }
                None => {
                    let mut p =
                        crate::pipe::PipeDecoder::new(&segs[si].path, dec_w, dec_h, segs[si].fps);
                    pipe_ok = p.spawn_at(ts);
                    pipe = Some(p);
                }
            }
            stab = stabs.get(si).cloned().flatten();
        }
        // Pre-warm the NEXT segment's hardware chain ~4s before the seam: the
        // ~0.3-0.5s cold start then overlaps current-segment decoding instead of
        // stalling the crossing itself (mirrors the preview's pre-warm).
        if si + 1 < segs.len() && warmed.is_none() {
            let cur_end = segs[si].start_ms + segs[si].duration_ms;
            if t_ms > cur_end - 4000.0 {
                let mut wp = crate::pipe::PipeDecoder::new(
                    &segs[si + 1].path,
                    dec_w,
                    dec_h,
                    segs[si + 1].fps,
                );
                if wp.spawn_at(0.0) {
                    eprintln!(
                        "[encode] pre-warm seg {} at t={t_ms:.0} (seam in {:.1}s)",
                        si + 2,
                        (cur_end - t_ms) / 1000.0
                    );
                    warmed = Some((si + 1, wp));
                }
            }
        }
        let mut rgba = vec![0u8; dec_w * dec_h * 4];
        // Prefer the hardware pipe decoder (matches the preview, uses the GPU Video Decode
        // engine). Export is OFFLINE: on a miss BLOCK at the same timestamp (the chain's
        // cold start after a respawn takes a few hundred ms — waiting costs one frame of
        // wall time, while falling back would software-decode the whole cold window at
        // ~10x the cost). Only a dead chain or a long give-up drops to software.
        let mut hw = false;
        if pipe_ok {
            if let Some(pp) = pipe.as_mut() {
                let waited = std::time::Instant::now();
                loop {
                    match pp.get(ts) {
                        Some(fr) if fr.len() == rgba.len() => {
                            rgba.copy_from_slice(&fr);
                            hw = true;
                            break;
                        }
                        Some(_) => break, // unexpected size → software
                        None => {
                            if pp.dead() {
                                pipe_ok = false;
                                break;
                            } // permanent failure → software
                            if stop.load(Ordering::SeqCst) || waited.elapsed().as_millis() > 5000 {
                                eprintln!("[encode] pipe wait give-up at t={ts:.0} seg={} after {}ms -> software", si + 1, waited.elapsed().as_millis());
                                pipe_ok = false;
                                break;
                            }
                        }
                    }
                }
            }
        }
        if !hw {
            let mut soft_ok = false;
            if let Some(d) = decoder.as_mut() {
                soft_ok = d.decode_at(ts, &mut rgba).is_ok();
            }
            if !soft_ok {
                // Past the segment's decodable end (probe duration > real stream): hold
                // the last buffered frame to the seam/end instead of aborting the export.
                match pipe.as_ref().and_then(|pp| pp.eof_frame(ts)) {
                    Some(fr) if fr.len() == rgba.len() => rgba.copy_from_slice(&fr),
                    _ => break,
                }
            }
        }
        if prof {
            t_dec += _t.elapsed().as_secs_f64();
        }
        let _t = std::time::Instant::now();
        if let Some(stab) = &stab {
            let mut sbuf = vec![0u8; dec_w * dec_h * 4];
            if stab
                .process_frame(ts, fi, &mut rgba, dec_w, dec_h, &mut sbuf, dec_w, dec_h)
                .is_ok()
            {
                rgba = sbuf;
            }
        }
        if prof {
            t_stab += _t.elapsed().as_secs_f64();
        }
        let _t = std::time::Instant::now();
        let video_img = crate::qtwrap::qimage_from_rgba(dec_w as i32, dec_h as i32, &rgba);
        let mut canvas = crate::qtwrap::qimage_new_argb32(out_w as i32, out_h as i32);
        {
            let mut p = crate::qtwrap::qpainter_on_image(&mut canvas);
            // Advance EVERY map's runner/camera (state must be live at a switch),
            // render the one active on the global timeline.
            for m in &mut prep.maps {
                m.rm.update(t_global / 1000.0, dt_out);
            }
            let mi = crate::gui::locate_map(&map_starts, t_global);
            let map_layer = if settings.encode.include_map {
                map_imgs[mi].clone()
            } else {
                None
            };
            let video_layer = if settings.encode.include_video {
                Some(video_img)
            } else {
                None
            };
            crate::gui::draw_scene(
                &mut p,
                &prep.maps[mi].rm,
                &settings,
                map_layer,
                video_layer,
                out_w as f64,
                out_h as f64,
                t_global,
                prep.maps[mi].start_ms,
                prep.gpx.as_ref(),
            );
        }
        if prof {
            t_draw += _t.elapsed().as_secs_f64();
        }
        let _t = std::time::Instant::now();
        let frame = crate::qtwrap::qimage_to_bgra(&canvas);
        if prof {
            t_pack += _t.elapsed().as_secs_f64();
        }
        let _t = std::time::Instant::now();
        if let Err(e) = enc.write_frame(&frame) {
            enc.abort();
            // 硬件编码器不可用属于"配置问题"，不是用户中止：用 aborted=false 上报，
            // 上层才能识别标记并自动改用 CPU 重新导出。
            // NB: 不要在这里再加前缀 —— e 可能已经带 HW_FAILURE_MARK，
            // 只保留原始错误串（已经是一句人话）。
            return finish(false, e);
        }
        if prof {
            t_wr += _t.elapsed().as_secs_f64();
        }
        count += 1;
        t_ms += dt_out * speed_n;
        let elapsed = t0.elapsed().as_secs_f64() - paused_secs;
        let per = elapsed / count.max(1) as f64;
        let out_size = std::fs::metadata(&out_path)
            .map(|m| m.len() as i64)
            .unwrap_or(0);
        handle.set_progress(EncodeProgress {
            frame: count,
            total_frames: total,
            elapsed_s: elapsed,
            remaining_s: per * (total.saturating_sub(count)) as f64,
            fps: count as f64 / elapsed.max(0.001),
            out_size,
            current_t_ms: t_global,
            duration_ms: clip_end,
        });
    }
    if prof && count > 0 {
        let n = count as f64;
        eprintln!("[TRACEREEL_TIME] frames={count} wall={:.1}s  per-frame ms: dec={:.1} stab={:.1} draw={:.1} pack={:.1} write={:.1}  (sum={:.1})",
            t0.elapsed().as_secs_f64(), t_dec*1000.0/n, t_stab*1000.0/n, t_draw*1000.0/n, t_pack*1000.0/n, t_wr*1000.0/n,
            (t_dec+t_stab+t_draw+t_pack+t_wr)*1000.0/n);
    }
    let aborted = stop.load(Ordering::SeqCst);
    if aborted {
        enc.abort();
        return finish(true, "编码已中止（部分文件已保留，可播放）".into());
    }
    match enc.close() {
        Ok(_) => {
            crate::log_info!("导出完成 -> {out_path}");
            finish(false, String::new())
        }
        Err(e) => finish(false, format!("收尾失败：{e}")),
    }
}

/// Map-only export: render the map + route + runner as a FULL-FRAME video with no source
/// video, no audio, no stabilization — for users who only have a map/track. The timeline is
/// the route duration. Shares encode_export's progress + close/abort semantics.
pub fn encode_export_map_only(job: EncodeJob) {
    encode_export_map_only_run(job, true);
}

/// 同 `encode_export_map_only`，但由 `mark_done` 控制结束时是否翻转共享 handle 的
/// done 标志（逐段导出批次中只有最后一段翻转）。
fn encode_export_map_only_run(job: EncodeJob, mark_done: bool) {
    let EncodeJob {
        mut settings,
        out_path,
        src_w,
        src_h,
        handle,
        ..
    } = job;
    let finish = |aborted: bool, error: String| {
        handle.set_result(EncodeResult {
            output: out_path.clone(),
            aborted,
            error,
        });
        if mark_done {
            handle.mark_done();
        }
    };
    // A route-driven export has no video panel; map and HUD remain independent layers.
    settings.map.relative_width = 1.0;
    settings.encode.audio_codec = "none".into();
    if !settings.encode.include_hud {
        settings.hud.layout = Some(crate::hud::Layout {
            version: 1,
            widgets: Vec::new(),
        });
    }
    // Map-only frame t is a route timestamp; preserve the corresponding GPX instant.
    settings.hud.gpx_time_offset +=
        settings.video.start_time_offset_s - settings.route.control_time_offset;
    settings.video.start_time_offset_s = settings.route.control_time_offset;
    // Nominal source size so a "source" resolution token still yields a sane frame.
    let (out_w, out_h) = encode_output_size(&settings.encode, src_w.max(1920), src_h.max(1080));
    // Frame rate: "source" is meaningless with no video → default 30.
    let raw = settings.encode.frame_rate.trim().to_ascii_lowercase();
    let out_fps = if raw == "source" || raw.is_empty() {
        30.0
    } else {
        encode_output_fps(&settings.encode, 30.0)
    };
    let mut prep = match EncodePrep::build(&settings, out_w, out_h) {
        Ok(p) => p,
        Err(e) => {
            finish(false, e);
            return;
        }
    };
    // Timeline = route duration (seconds). Clip range from the user's start/end seconds.
    let route_dur_ms = (prep
        .maps
        .iter()
        .map(|m| m.rm.duration_s())
        .fold(0.0f64, f64::max)
        * 1000.0)
        .max(0.0);
    let gpx_dur_ms = prep
        .gpx
        .as_ref()
        .and_then(|g| {
            g.points
                .last()
                .map(|p| (p.t - g.start_time).max(0.0) * 1000.0)
        })
        .unwrap_or(0.0);
    let route_dur_ms = route_dur_ms.max(gpx_dur_ms);
    if route_dur_ms <= 0.0 {
        finish(
            false,
            "没有可用的路线或 GPX 时间数据，无法确定导出时长".into(),
        );
        return;
    }
    let clip_start = settings.encode.clip_start_ms.max(0.0).min(route_dur_ms);
    let clip_end = if settings.encode.clip_end_ms > clip_start {
        settings.encode.clip_end_ms.min(route_dur_ms)
    } else {
        route_dur_ms
    };
    // Export speed applies to map-only too: sample the route timeline N× faster.
    let speed_n = encode_speed_ratio(&settings.encode);
    let dt_out = 1000.0 / out_fps;
    let total =
        (((((clip_end - clip_start) / 1000.0) * out_fps) / speed_n).round() as usize).max(1);
    handle.set_progress(EncodeProgress {
        frame: 0,
        total_frames: total,
        elapsed_s: 0.0,
        remaining_s: 0.0,
        fps: 0.0,
        out_size: 0,
        current_t_ms: clip_start,
        duration_ms: clip_end,
    });
    let mut enc = match FfmpegEncoder::open(
        &out_path,
        "",
        &[],
        out_w,
        out_h,
        out_fps,
        route_dur_ms,
        &settings.encode,
    ) {
        Ok(e) => e,
        Err(e) => {
            finish(false, format!("编码器启动失败: {e}"));
            return;
        }
    };
    let map_imgs: Vec<Option<_>> = prep
        .maps
        .iter()
        .map(|m| {
            if m.w > 0 {
                Some(crate::qtwrap::qimage_from_rgba(m.w, m.h, &m.rgba))
            } else {
                None
            }
        })
        .collect();
    let map_starts: Vec<f64> = prep.maps.iter().map(|m| m.start_ms).collect();
    // Pre-roll smoothing so a mid-course start doesn't "settle" (same reason as encode_export).
    // Warm up every map so a switch never has to catch up.
    if clip_start > 0.0 {
        let warm_end = clip_start / 1000.0;
        let mut wt = (warm_end - 10.0).max(0.0);
        let step = dt_out / 1000.0;
        while wt < warm_end {
            for m in &mut prep.maps {
                m.rm.update(wt, dt_out);
            }
            wt += step;
        }
    }
    let t0 = std::time::Instant::now();
    let mut count = 0usize;
    let mut t_ms = clip_start;
    let mut paused_secs = 0.0f64;
    let mut pause_since: Option<std::time::Instant> = None;
    let stop = handle.stop_flag();
    let pause = handle.pause_flag();
    while count < total && !stop.load(Ordering::SeqCst) {
        if pause.load(Ordering::SeqCst) {
            if pause_since.is_none() {
                pause_since = Some(std::time::Instant::now());
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
            continue;
        }
        if let Some(ps) = pause_since.take() {
            paused_secs += ps.elapsed().as_secs_f64();
        }
        let mut canvas = crate::qtwrap::qimage_new_argb32(out_w as i32, out_h as i32);
        {
            let mut p = crate::qtwrap::qpainter_on_image(&mut canvas);
            let secs = t_ms / 1000.0;
            for m in &mut prep.maps {
                m.rm.update(secs, dt_out);
            }
            let mi = crate::gui::locate_map(&map_starts, t_ms);
            crate::gui::draw_scene(
                &mut p,
                &prep.maps[mi].rm,
                &settings,
                if settings.encode.include_map {
                    map_imgs[mi].clone()
                } else {
                    None
                },
                None,
                out_w as f64,
                out_h as f64,
                t_ms,
                prep.maps[mi].start_ms,
                prep.gpx.as_ref(),
            );
        }
        let frame = crate::qtwrap::qimage_to_bgra(&canvas);
        if let Err(e) = enc.write_frame(&frame) {
            enc.abort();
            finish(true, format!("写入帧失败: {e}"));
            return;
        }
        count += 1;
        t_ms += dt_out * speed_n;
        let elapsed = t0.elapsed().as_secs_f64() - paused_secs;
        let per = elapsed / count.max(1) as f64;
        let out_size = std::fs::metadata(&out_path)
            .map(|m| m.len() as i64)
            .unwrap_or(0);
        handle.set_progress(EncodeProgress {
            frame: count,
            total_frames: total,
            elapsed_s: elapsed,
            remaining_s: per * (total.saturating_sub(count)) as f64,
            fps: count as f64 / elapsed.max(0.001),
            out_size,
            current_t_ms: t_ms,
            duration_ms: clip_end,
        });
    }
    let aborted = stop.load(Ordering::SeqCst);
    if aborted {
        enc.abort();
        finish(true, "编码已中止（部分文件已保留，可播放）".into());
    } else {
        match enc.close() {
            Ok(_) => {
                eprintln!("[encode] map-only export finished -> {out_path}");
                finish(false, String::new());
            }
            Err(e) => {
                finish(false, format!("收尾失败: {e}"));
            }
        }
    }
}

/// Convenience: resolve a project-relative output path to absolute.
pub fn absolute_output(path: &str, cwd: &str) -> PathBuf {
    let pb = PathBuf::from(path);
    if pb.is_absolute() {
        return pb;
    }
    PathBuf::from(cwd).join(pb)
}

#[cfg(test)]
mod tests {
    #[test]
    fn batch_progress_covers_the_whole_queue_entry() {
        let handle = super::EncodeHandle::default();
        *handle.batch_position.lock().unwrap() = (0, 2);
        handle.set_progress(super::EncodeProgress { frame: 10, total_frames: 10, ..Default::default() });
        assert_eq!(handle.overall_progress(), 0.5);
        *handle.batch_position.lock().unwrap() = (1, 2);
        handle.set_progress(super::EncodeProgress { frame: 5, total_frames: 10, ..Default::default() });
        assert_eq!(handle.overall_progress(), 0.75);
        handle.set_progress(super::EncodeProgress { frame: 10, total_frames: 10, ..Default::default() });
        assert_eq!(handle.overall_progress(), 1.0);
    }
    use super::*;

    fn enc_base() -> EncodeSettings {
        crate::settings::Settings::default().encode
    }

    #[test]
    fn speed_ratio_parses_tokens_and_rejects_junk() {
        let mut enc = enc_base();
        enc.speed = "2".into();
        assert_eq!(encode_speed_ratio(&enc), 2.0);
        enc.speed = "0.8".into();
        assert_eq!(encode_speed_ratio(&enc), 0.8);
        enc.speed = "".into();
        assert_eq!(encode_speed_ratio(&enc), 1.0);
        enc.speed = "abc".into();
        assert_eq!(encode_speed_ratio(&enc), 1.0);
        enc.speed = "100".into();
        assert_eq!(encode_speed_ratio(&enc), 8.0); // clamped to a safe max
    }

    #[test]
    fn atempo_chains_stay_within_stage_limits() {
        assert_eq!(atempo_chain(1.0), "");
        assert_eq!(atempo_chain(2.0), "atempo=2.000000");
        assert_eq!(atempo_chain(0.8), "atempo=0.800000");
        assert_eq!(atempo_chain(1.5), "atempo=1.500000");
        assert_eq!(atempo_chain(3.0), "atempo=2.0,atempo=1.500000");
        assert_eq!(atempo_chain(4.0), "atempo=2.0,atempo=2.000000");
    }

    #[test]
    fn clip_total_frames_divides_by_speed() {
        let mut enc = enc_base();
        enc.clip_start_ms = 0.0;
        enc.clip_end_ms = 10_000.0;
        enc.speed = "1".into();
        assert_eq!(clip_total_frames(&enc, 20_000.0, 30.0), 300);
        enc.speed = "4".into();
        assert_eq!(clip_total_frames(&enc, 20_000.0, 30.0), 75);
        enc.speed = "0.8".into();
        assert_eq!(clip_total_frames(&enc, 20_000.0, 30.0), 375);
    }

    #[test]
    fn build_command_speed_applies_atempo_and_forces_reencode() {
        let mut enc = enc_base();
        enc.audio_codec = "copy".into();
        enc.speed = "2".into();
        let cmd = build_command(
            "out.mp4",
            "src.mp4",
            &[],
            1920,
            1080,
            30.0,
            60_000.0,
            None,
            &enc,
        );
        let joined = cmd.join(" ");
        assert!(joined.contains("-af atempo=2.000000"), "{joined}");
        assert!(
            joined.contains("-c:a aac") && !joined.contains("-c:a copy"),
            "{joined}"
        );
        // No speed change → untouched copy path, no filter.
        enc.speed = "1".into();
        let cmd1 = build_command(
            "out.mp4",
            "src.mp4",
            &[],
            1920,
            1080,
            30.0,
            60_000.0,
            None,
            &enc,
        );
        let joined1 = cmd1.join(" ");
        assert!(!joined1.contains("-af"), "{joined1}");
        assert!(joined1.contains("-c:a copy"), "{joined1}");
    }

    /// One stereo s16le frame.
    fn frame(l: i16, r: i16) -> Vec<u8> {
        let mut v = l.to_le_bytes().to_vec();
        v.extend_from_slice(&r.to_le_bytes());
        v
    }

    fn frames(pcm: &[u8]) -> Vec<(i16, i16)> {
        pcm.chunks_exact(4)
            .map(|c| {
                (
                    i16::from_le_bytes([c[0], c[1]]),
                    i16::from_le_bytes([c[2], c[3]]),
                )
            })
            .collect()
    }

    #[test]
    fn xfade_single_part_is_identity() {
        let a: Vec<u8> = frame(100, -100).into_iter().cycle().take(4 * 10).collect();
        let m = merge_pcm_with_xfade(&[a.clone()], XFADE_FRAMES);
        assert_eq!(m, a);
    }

    #[test]
    fn xfade_shortens_per_seam_and_blends() {
        let a: Vec<u8> = frame(1000, 1000).into_iter().cycle().take(4 * 20).collect();
        let b: Vec<u8> = frame(-1000, -1000)
            .into_iter()
            .cycle()
            .take(4 * 20)
            .collect();
        let m = merge_pcm_with_xfade(&[a, b], 9);
        let s = frames(&m);
        // 20 + 20 frames shortened by the 9-frame overlap.
        assert_eq!(s.len(), 31);
        // Pure regions untouched.
        assert_eq!(s[0].0, 1000);
        assert_eq!(s[10].0, 1000);
        assert_eq!(s[30].0, -1000);
        // Overlap is frames 11..=19: first ≈ pure A, exact middle = equal blend,
        // last = pure B (Hann endpoints 0/1).
        assert_eq!(s[11].0, 1000);
        assert_eq!(s[15].0, 0);
        assert_eq!(s[19].0, -1000);
        // Monotonic fade across the overlap.
        for w in s[11..20].windows(2) {
            assert!(w[1].0 <= w[0].0);
        }
    }

    #[test]
    fn xfade_clamps_to_short_parts() {
        let a: Vec<u8> = frame(100, 100).into_iter().cycle().take(4 * 3).collect();
        let b: Vec<u8> = frame(-100, -100).into_iter().cycle().take(4 * 2).collect();
        let m = merge_pcm_with_xfade(&[a, b], XFADE_FRAMES);
        // n clamps to b's 2 frames: total = 3 + 2 - 2.
        let s = frames(&m);
        assert_eq!(s.len(), 3);
        assert_eq!(s[0].0, 100);
        assert_eq!(s[2].0, -100);
    }

    #[test]
    fn xfade_three_parts_two_seams() {
        let p: Vec<Vec<u8>> = [500i16, -500, 500]
            .iter()
            .map(|&v| frame(v, v).into_iter().cycle().take(4 * 30).collect())
            .collect();
        let m = merge_pcm_with_xfade(&p, 6);
        assert_eq!(m.len() / 4, 30 * 3 - 6 * 2);
        // Regions far from the seams keep the original constants: seam1 = frames
        // 24..29, seam2 = frames 48..53, so 0 / 40 / 70 sit in pure regions.
        let s = frames(&m);
        assert_eq!(s[0].0, 500);
        assert_eq!(s[40].0, -500);
        assert_eq!(s[70].0, 500);
    }

    // ── 硬件编码器失败的归类 ─────────────────────────────────────────────────
    // 下面第一段 stderr 是用户实测（Intel 核显不支持 AV1）的原样输出，
    // 第二段是在本机跑 av1_nvenc（无 CUDA 设备）复现出来的。

    /// 核显不支持 AV1（用户截图里的原始报错）。
    const STDERR_AV1_QSV_UNSUPPORTED: &str = "\
[out#0/mp4 @ 0000020D940DE900] Error opening output file C:/media/example.mp4.
[out#0/mp4 @ 0000020D940DE900] Error opening output files: Invalid argument
[vost#0:0/av1_qsv @ 0000020D9106E100] [enc:av1_qsv @ 0000020D9383B300] Could not open encoder before EOF
[vost#0:0/av1_qsv @ 0000020D9106E100] Task finished with error code: -22 (Invalid argument)
[vost#0:0/av1_qsv @ 0000020D9106E100] Terminating thread with return code -22 (Invalid argument)
[out#0/mp4 @ 0000020D940DD400] Nothing was written into output file, because at least one of its streams received no packets.";

    /// 无 CUDA 设备时 av1_nvenc 的原始报错（本机复现）。
    const STDERR_AV1_NVENC_NO_DEVICE: &str = "\
[av1_nvenc @ 000001A9A959E240] dl_fn->cuda_dl->cuInit(0) failed -> CUDA_ERROR_NO_DEVICE: no CUDA-capable device is detected
[vost#0:0/av1_nvenc @ 000001A9A959C040] [enc:av1_nvenc @ 000001A9A959E180] Error while opening encoder - maybe incorrect parameters such as bit_rate, rate, width or height.
[vf#0:0 @ 000001A9A95B9200] Error sending frames to consumers: Unknown error occurred
[vost#0:0/av1_nvenc @ 000001A9A959C040] [enc:av1_nvenc @ 000001A9A959E180] Could not open encoder before EOF
[out#0/mp4 @ 000001A9A953F3C0] Nothing was written into output file, because at least one of its streams received no packets.";

    #[test]
    fn av1_qsv_unsupported_is_classified_as_unsupported_format() {
        let cause = classify_hw_failure(STDERR_AV1_QSV_UNSUPPORTED).expect("应识别为硬件编码失败");
        assert_eq!(cause, HwCause::UnsupportedFormat);
    }

    #[test]
    fn nvenc_without_cuda_device_is_classified_as_device_init() {
        let cause =
            classify_hw_failure(STDERR_AV1_NVENC_NO_DEVICE).expect("应识别为硬件编码失败");
        assert_eq!(cause, HwCause::DeviceInit);
    }

    #[test]
    fn hw_marker_survives_an_outer_prefix() {
        // 回归：write_frame 的错误曾经被包成 "写入帧失败：<标记>原因"，
        // 而当时 unmark 只判断开头，导致自动回退静默失效。
        let marked = mark_hw_failure("显卡不支持 AV1 硬件编码");
        assert_eq!(unmark_hw_failure(&marked), Some("显卡不支持 AV1 硬件编码"));
        let wrapped = format!("写入帧失败：{marked}");
        assert_eq!(
            unmark_hw_failure(&wrapped),
            Some("显卡不支持 AV1 硬件编码"),
            "前缀包裹后必须仍能识别出硬件失败标记"
        );
        assert_eq!(unmark_hw_failure("普通错误"), None);
    }

    #[test]
    fn encoder_status_text_names_the_real_encoder() {
        let mut enc = EncodeSettings {
            backend: "software".into(),
            format: "av1".into(),
            rate_control: "cbr".into(),
            quality: 21.0,
            bitrate: "4".into(),
            preset: "medium".into(),
            speed: "1".into(),
            audio_codec: "none".into(),
            audio_bitrate: "128k".into(),
            clip_start_ms: 0.0,
            clip_end_ms: 0.0,
            output_file_path: String::new(),
            resolution: "source".into(),
            frame_rate: "source".into(),
            map_only: false,
            include_map: false,
            include_video: true,
            include_hud: false,
            seg_enabled: false,
            start_seg: 0,
            end_seg: 0,
            all_seg_enabled: false,
        };
        // 每种后端/格式组合都要能说清"编码用什么、解码用什么"。
        let s = encoder_status_text(&enc);
        assert!(s.contains("中央处理器"), "{s}");
        assert!(s.contains("解码：显卡硬解"), "必须说明解码仍走显卡: {s}");
        assert!(!s.contains("libx264") || enc.format == "h264", "{s}");

        enc.backend = "qsv".into();
        enc.format = "hevc".into();
        let h = encoder_status_text(&enc);
        assert!(h.contains("核显 QSV"), "{h}");
        assert!(h.contains("H.265/HEVC"), "{h}");

        // 界面文字不得出现英文技术词（产品名/格式名除外）。
        for bad in ["hardware", "software", "d3d11va", "hwaccel"] {
            assert!(!h.to_ascii_lowercase().contains(bad), "泄漏英文: {h}");
            assert!(!s.to_ascii_lowercase().contains(bad), "泄漏英文: {s}");
        }
    }

    #[test]
    fn software_av1_never_emits_encoder_private_flags_for_a_missing_encoder() {
        // 回归：用户用「CPU + AV1」导出时报
        //   Unrecognized option 'cpu-used'.
        //   Error splitting the argument list: Option not found
        // 根因是 ffmpeg 构建里没有 libaom-av1（avbuild gpl-lite 就是），
        // 却仍然附加了 libaom 专有的 -cpu-used/-row-mt，于是报出一句毫无线索的
        // "选项找不到"。修复后必须换成真实存在的编码器。
        let mut enc = EncodeSettings {
            backend: "software".into(),
            format: "av1".into(),
            rate_control: "cbr".into(),
            quality: 21.0,
            bitrate: "4".into(),
            preset: "medium".into(),
            speed: "1".into(),
            audio_codec: "none".into(),
            audio_bitrate: "128k".into(),
            clip_start_ms: 0.0,
            clip_end_ms: 0.0,
            output_file_path: "out.mp4".into(),
            resolution: "source".into(),
            frame_rate: "source".into(),
            map_only: false,
            include_map: false,
            include_video: true,
            include_hud: false,
            seg_enabled: false,
            start_seg: 0,
            end_seg: 0,
            all_seg_enabled: false,
        };
        // gpl-lite 的编码器集合：只有 x264/x265，没有任何软件 AV1。
        let lite = vec!["libx264".to_string(), "libx265".to_string()];
        let (codec, hw, eff_fmt) = resolve_video_encoder(&enc, &lite);
        assert!(!hw);
        assert_eq!(codec, "libx264", "没有软件 AV1 时必须换成一个存在的编码器");
        assert_eq!(eff_fmt, "h264", "格式随之降级，避免产出无效的 AV1 标签");
        // 关键：绝不能带上 libaom/SVT 的私有选项。
        let args = codec_args(&codec, "software", &eff_fmt, &enc, false);
        let joined = args.join(" ");
        assert!(!joined.contains("cpu-used"), "不该出现 cpu-used: {joined}");
        assert!(!joined.contains("row-mt"), "不该出现 row-mt: {joined}");
        assert!(!joined.contains("svtav1"), "{joined}");

        // 有 libaom 时仍然走 libaom 并带它的选项（保持原有行为）。
        let full = vec!["libx264".to_string(), "libaom-av1".to_string()];
        let (c2, _, f2) = resolve_video_encoder(&enc, &full);
        assert_eq!((c2.as_str(), f2.as_str()), ("libaom-av1", "av1"));
        let a2 = codec_args(&c2, "software", &f2, &enc, false);
        assert!(a2.join(" ").contains("cpu-used"), "{a2:?}");

        // 有 SVT-AV1 时优先用它，并且同样保持 AV1 格式。
        let svt = vec!["libx264".to_string(), "libsvtav1".to_string(), "libaom-av1".to_string()];
        let (c3, _, f3) = resolve_video_encoder(&enc, &svt);
        assert_eq!((c3.as_str(), f3.as_str()), ("libsvtav1", "av1"));
        let a3 = codec_args(&c3, "software", &f3, &enc, false);
        assert!(!a3.join(" ").contains("cpu-used"), "SVT 不接受 cpu-used: {a3:?}");

        // 硬件路径不受影响。
        enc.backend = "qsv".into();
        let (c4, hw4, f4) = resolve_video_encoder(&enc, &lite);
        assert!(hw4);
        assert_eq!((c4.as_str(), f4.as_str()), ("av1_qsv", "av1"));
    }

    #[test]
    fn unknown_encoder_is_also_a_hardware_path_failure() {
        // 这个 ffmpeg 构建里没有该编码器（端到端验证时实测到的场景）：
        // 与"显卡不支持"同类 —— 当前硬件路径走不通，应该回退到 CPU。
        let s = "Unknown encoder 'av1_qsv_bogus'\n\
                 Error selecting an encoder\n\
                 Error opening output file x.mp4.\n\
                 Error opening output files: Encoder not found";
        assert_eq!(classify_hw_failure(s), Some(HwCause::UnsupportedFormat));
        let r = hw_failure_reason(s, "av1_qsv_bogus", "av1").unwrap();
        assert!(r.reason.contains("av1_qsv_bogus"), "{}", r.reason);
    }

    #[test]
    fn classifier_ignores_non_encoder_errors() {
        // 磁盘满 / 路径非法：不能误判成显卡问题（否则会白白回退到 CPU）。
        for s in [
            "av_interleaved_write_frame(): No space left on device",
            "[out#0/mp4] Could not write header (incorrect codec parameters ?): Permission denied",
            "Error opening output file out.mp4: No such file or directory",
            "",
        ] {
            assert_eq!(classify_hw_failure(s), None, "不该被判定为硬件失败: {s}");
            assert!(hw_failure_reason(s, "av1_qsv", "av1").is_none(), "{s}");
        }
    }

    #[test]
    fn reason_names_the_codec_and_format_for_the_user() {
        let f = hw_failure_reason(STDERR_AV1_QSV_UNSUPPORTED, "av1_qsv", "av1").unwrap();
        assert!(f.reason.contains("av1_qsv"), "原因里应带编码器名: {}", f.reason);
        assert!(f.reason.contains("AV1"), "原因里应带格式名: {}", f.reason);
        assert!(f.reason.contains("显卡不支持"), "原因应说明显卡不支持: {}", f.reason);
        // 面向用户的一句话原因里不该出现 ffmpeg 的内部堆栈文本。
        assert!(!f.reason.contains("Error sending frames"), "{}", f.reason);
        assert!(!f.reason.contains("Task finished"), "{}", f.reason);

        let h = hw_failure_reason(STDERR_AV1_QSV_UNSUPPORTED, "av1_qsv", "hevc").unwrap();
        assert!(h.reason.contains("H.265/HEVC"), "{}", h.reason);

        let n = hw_failure_reason(STDERR_AV1_NVENC_NO_DEVICE, "av1_nvenc", "av1").unwrap();
        assert!(n.reason.contains("驱动"), "{}", n.reason);
    }

    #[test]
    fn software_encoder_is_derived_for_every_format() {
        // 回退到 CPU 时必须拿到一个有效的软件编码器。
        assert_eq!(software_encoder_for("h264"), "libx264");
        assert_eq!(software_encoder_for("hevc"), "libx265");
        assert_eq!(software_encoder_for("av1"), "libsvtav1", "AV1 首选更快的 SVT-AV1");
        assert_eq!(software_encoder_for("mpeg4"), "mpeg4");
        assert_eq!(software_encoder_for("未知"), "libx264");
    }

    #[test]
    fn software_fallback_prefers_a_real_av1_encoder_when_present() {
        let with_av1 = vec![
            "libx264".to_string(),
            "libx265".to_string(),
            "libaom-av1".to_string(),
            "libsvtav1".to_string(),
        ];
        // 两个 AV1 编码器都在时选更快的 SVT-AV1，并且**保持 AV1 格式**。
        assert_eq!(
            pick_software_fallback("av1", &with_av1),
            ("av1".to_string(), "libsvtav1".to_string())
        );
        // 只有 libaom 时仍然保住 AV1。
        let only_aom = vec!["libx264".to_string(), "libaom-av1".to_string()];
        assert_eq!(
            pick_software_fallback("av1", &only_aom),
            ("av1".to_string(), "libaom-av1".to_string())
        );
        // 只有 rav1e 时也保住 AV1。
        let only_rav1e = vec!["libx264".to_string(), "librav1e".to_string()];
        assert_eq!(
            pick_software_fallback("av1", &only_rav1e),
            ("av1".to_string(), "librav1e".to_string())
        );
    }

    #[test]
    fn software_fallback_degrades_to_h264_only_when_the_format_has_no_encoder() {
        // 这正是打包用的 gpl-lite 构建：没有任何软件 AV1 编码器。
        let lite = vec!["libx264".to_string(), "libx265".to_string()];
        assert_eq!(
            pick_software_fallback("av1", &lite),
            ("h264".to_string(), "libx264".to_string()),
            "没有软件 AV1 时必须降级到 H.264，否则回退会二次失败"
        );
        // HEVC / H.264 各自有编码器时保持原格式。
        assert_eq!(
            pick_software_fallback("hevc", &lite),
            ("hevc".to_string(), "libx265".to_string())
        );
        assert_eq!(
            pick_software_fallback("h264", &lite),
            ("h264".to_string(), "libx264".to_string())
        );
    }
}
