//! Video encoding pipeline via an ffmpeg subprocess (ported from VideoEncoder.cpp /
//! encoder.py). Frames are composited by the application and piped to ffmpeg as
//! rawvideo; the original audio track is mapped in the same command.
//!
//! Key invariants (verified in the previous implementation):
//! * The encode ITSELF writes `-movflags frag_keyframe+empty_moov -flush_packets 1` so the
//!   moov index is written up front and every keyframe fragment lands on disk immediately —
//!   an aborted encode still leaves a playable file.
//! * Every mp4 that is KEPT — the finished one (see [`FfmpegEncoder::close`]) and the
//!   partial one left by a user abort (see [`finalize_mobile_mp4`]) — is then re-muxed once
//!   with `-c copy -movflags +faststart`: a fragmented file carries `mvhd/tkhd/mdhd duration
//!   = 0` in its moov, and the phone-side apps people actually deliver to (微信 / 视频号 /
//!   抖音 / 剪映) read only the moov, so they show 3 秒 / <1 秒 / 不识别. The re-mux is a
//!   stream copy (lossless, no re-encode) and restores a real duration with moov at the front.
//!   The partial file is additionally trimmed to the video track's end: at abort time the
//!   copied audio is usually already complete while the picture stops where the user stopped.
//!   A *failed* attempt is the one case that is left alone: its residue is thrown away by the
//!   `-y` retry (or is a few hundred ms of a failed export), so re-muxing it would be wasted IO.
//! * Never use `-shortest` (deadlocks the final fragment flush on long encodes).
//! * QSV hwupload path must not force `-pix_fmt`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use qmetaobject::qttypes::QPainter;
use qmetaobject::QImage;

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

/// 导出解码 + 防抖缓冲尺寸：按**输出画布里面板实际需要的像素**取，上限=源尺寸（不放大）。
///
/// 以前固定限高 720（复用预览那套稳定化器），于是输出选 1080p / 4K 时，视频那一层其实是
/// 从 720p 级放大出来的（地图/路线/HUD 不受影响，它们按输出画布绘制）。现在按面板实际
/// 像素解码：1080p 输出从 1080p 源解码，4K 输出顶到源尺寸为止，既不放大也不多解码。
///
/// 与 `draw_scene` 共用四侧固定/自由摆放的版面计算：
/// 缩放系数 = min(面板宽/源宽, 面板高/源高) × 用户缩放；自由地图叠在完整视频面板上。
/// 注意：gyroflow 的稳定化器与帧尺寸强绑定，尺寸变了必须重新加载（见
/// [`reload_export_stabs_if_needed`]）。
pub fn export_decode_size(settings: &Settings, src_w: usize, src_h: usize) -> (usize, usize) {
    let src = (src_w.max(2), src_h.max(2));
    let (out_w, out_h) = encode_output_size(&settings.encode, src.0, src.1);
    let panel = crate::gui::scene_panels(settings, out_w as f64, out_h as f64).video;
    let vp_w = panel.width.max(1.0);
    let vp_h = panel.height.max(1.0);
    let base = (vp_w / src.0 as f64).min(vp_h / src.1 as f64);
    // 视频被画成的像素尺寸 = 源尺寸 × base × 用户缩放；超过源尺寸没有意义（k 上限 1）。
    let k = (base * settings.video.scale).clamp(0.05, 1.0);
    let even = |v: usize| v.max(2) - (v.max(2) % 2);
    let max_w = even(src.0);
    let max_h = even(src.1);
    (
        even((src.0 as f64 * k).round() as usize).min(max_w),
        even((src.1 as f64 * k).round() as usize).min(max_h),
    )
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
        // 编码本身写的是分片 MP4（中止也能留下可播放文件），成功后重封装一次，
        // 得到手机端能正确识别时长的标准 MP4。失败只记日志：文件本身仍然可播。
        let _ = finalize_mobile_mp4(&self.output_path, None);
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

/// 导出收尾：把分片 MP4 重封装成「手机能认时长」的标准 MP4。
///
/// 为什么需要它：编码阶段刻意用 `frag_keyframe+empty_moov` 写**分片** MP4，好处是
/// 每个关键帧片段立刻落盘、中止也留下可播放的文件；代价是 moov 里的
/// `mvhd/tkhd/mdhd` 时长全是 0（真实时长分散在各 `moof/tfdt` 里，文件尾另有 `mfra`）。
/// 桌面播放器（PotPlayer/VLC/ffmpeg）会把整个文件扫一遍算出来，所以电脑上一切正常；
/// 手机上的 App（微信、视频号、抖音、剪映）只读 moov，于是时长认成 0 —— 微信显示
/// 3 秒、视频号导入时不足 1 秒、抖音直接不识别。这与编码器/画质/码率无关，
/// 换 h264/hevc/av1、换 CPU/QSV 都一样。
///
/// 调用点有两个，覆盖**所有会被留下的 mp4**：
/// * [`FfmpegEncoder::close`] —— 完整导出的收尾；
/// * 用户中止（点「停止」）后 —— 已写入的部分同样要能发到手机上，代价是与已写入
///   大小成正比的一次搬运（不重编码），所以调用方会先把提示写进 notice，
///   让编码弹窗说明"正在重封装"，用户不会以为程序卡住。
/// **导出失败**留下的残片不重封装：它会被 CPU 回退重试的 `-y` 直接截断，搬一遍纯属浪费。
///
/// 动作是一次 `-c copy -movflags +faststart` 重封装：只搬运码流、不重新编码，
/// 画质与「复制原声」的音频完全不变，只是把 moov 补上真实时长并前置。
/// 先写同目录临时文件再原子替换，因此中途失败也不会留下半截文件；
/// 重封装失败则保留原分片文件（电脑上依旧能播）并写进日志。
///
/// `keep_s`：只保留该时刻之前的内容（容器级裁剪，仍不重新编码）。中止导出时用它按
/// **视频轨的实际结尾**裁掉多余的音频（见 [`video_track_end_s`]）；完整导出传 `None`。
/// 返回是否真的做了一次重封装（调用方据此决定要不要对用户说"已封装为标准 MP4"）。
fn finalize_mobile_mp4(output_path: &str, keep_s: Option<f64>) -> bool {
    let p = Path::new(output_path);
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !matches!(ext.as_str(), "mp4" | "mov" | "m4v" | "3gp" | "m4a") {
        return false;
    }
    if !mp4_has_moof(p) {
        // 已经是标准（非分片）MP4，或还没写出任何片段（空文件）：无需处理。
        return false;
    }
    let tmp = tmp_sibling_path(p);
    let tmp_s = tmp.to_string_lossy().into_owned();
    crate::log_info!("正在重封装为标准 MP4（手机端识别时长用）-> {output_path}");
    let cmd = finalize_remux_command(&find_ffmpeg(), output_path, &tmp_s, keep_s);
    let out = background_command(&cmd[0]).args(&cmd[1..]).output();
    let ok = matches!(&out, Ok(o) if o.status.success());
    if !ok {
        let detail = match &out {
            Ok(o) => {
                let e = String::from_utf8_lossy(&o.stderr);
                let last = e
                    .lines()
                    .rev()
                    .map(str::trim)
                    .find(|l| !l.is_empty())
                    .unwrap_or("无输出");
                format!("退出码 {:?}：{last}", o.status.code())
            }
            Err(e) => e.to_string(),
        };
        crate::log_warn!(
            "导出文件已生成，但重封装为标准 MP4 失败（{detail}）；\
             保留原分片文件 {output_path}。该文件在电脑上可正常播放，\
             手机端（微信/视频号/抖音/剪映）可能无法识别时长。"
        );
        let _ = std::fs::remove_file(&tmp);
        return false;
    }
    // 原子替换：同目录 rename，Windows 上会覆盖已存在的目标。
    if let Err(e) = std::fs::rename(&tmp, p) {
        crate::log_warn!(
            "标准 MP4 已写好（临时文件 {tmp_s}），但替换 {output_path} 失败：{e}；\
             请手工用临时文件替换原文件。"
        );
        return false;
    }
    crate::log_info!("已重封装为标准 MP4（手机兼容，时长写入 moov）-> {output_path}");
    true
}

/// 读出（分片）文件里视频轨的实际结尾时刻（秒），供中止导出裁剪用。
///
/// 中止时音频往往已经**全部**混流完（音频是流复制，比逐帧喂入的视频快得多），
/// 若不裁，手机拿到的是一个"时长 6 分钟、画面只有 30 秒"的文件。
/// ffprobe 缺失或读不出时长就返回 `None`（不裁，退回原来的行为）。
fn video_track_end_s(path: &Path) -> Option<f64> {
    let ffmpeg = find_ffmpeg();
    let probe = match Path::new(&ffmpeg).parent() {
        Some(dir) if dir.join("ffprobe.exe").exists() => {
            dir.join("ffprobe.exe").to_string_lossy().into_owned()
        }
        _ => "ffprobe.exe".to_string(),
    };
    let out = background_command(&probe)
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=duration",
            "-of",
            "default=nw=1:nk=1",
        ])
        .arg(path)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let t: f64 = String::from_utf8_lossy(&out.stdout).trim().parse().ok()?;
    (t.is_finite() && t > 0.0).then_some(t)
}

/// 重封装命令：搬运码流 + moov 前置（可选按 `keep_s` 裁掉尾巴）。抽成纯函数便于单测。
fn finalize_remux_command(
    ffmpeg: &str,
    input: &str,
    tmp_out: &str,
    keep_s: Option<f64>,
) -> Vec<String> {
    let mut cmd = vec![
        ffmpeg.to_string(),
        "-y".into(),
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-i".into(),
        input.to_string(),
        "-c".into(),
        "copy".into(),
        "-movflags".into(),
        "+faststart".into(),
    ];
    if let Some(t) = keep_s {
        cmd.push("-t".into());
        cmd.push(format!("{t:.3}"));
    }
    cmd.push(tmp_out.to_string());
    cmd
}

/// 重封装用的同目录临时路径：扩展名必须保留（ffmpeg 靠它选混流器）。
fn tmp_sibling_path(p: &Path) -> PathBuf {
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("mp4")
        .to_string();
    let stem = p
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output")
        .to_string();
    p.with_file_name(format!("{stem}.mobile.tmp.{ext}"))
}

/// 判断一个 MP4/MOV 是否为分片格式（顶层出现 `moof` 即为分片）。
///
/// 只看顶层盒子头并靠 seek 跳过 mdat，所以哪怕文件几十 GB 也只读几百字节。
fn mp4_has_moof(path: &Path) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else {
        return false;
    };
    let Ok(len) = f.metadata().map(|m| m.len()) else {
        return false;
    };
    let mut off: u64 = 0;
    while off + 8 <= len {
        if f.seek(SeekFrom::Start(off)).is_err() {
            return false;
        }
        let mut hdr = [0u8; 16];
        if f.read_exact(&mut hdr[..8]).is_err() {
            return false;
        }
        if &hdr[4..8] == b"moof" {
            return true;
        }
        let size32 = u32::from_be_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]) as u64;
        let (size, hdr_len) = match size32 {
            0 => (len - off, 8u64), // 最后一个盒子：延伸到文件尾
            1 => {
                if f.read_exact(&mut hdr[8..16]).is_err() {
                    return false;
                }
                (u64::from_be_bytes(hdr[8..16].try_into().unwrap()), 16u64)
            }
            n => (n, 8u64),
        };
        if size < hdr_len || size > len - off {
            return false;
        }
        off += size;
    }
    false
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
                rms: Vec::new(),
                rm: Default::default(),
            });
        }
        // Map-only exports also need the GPX clock when the bitmap has no embedded route.
        let gpx = if settings.encode.include_hud || !settings.encode.include_video {
            // 与预览/离屏渲染同一条解析路径：GPX → 视频同名 .gpx → 轨迹自带的时间轴。
            crate::gui::resolve_hud_track(
                &settings.hud.gpx_file_path,
                &settings.video.video_path,
                maps.first().map(|m| &m.rm),
            )
        } else {
            None
        };
        Ok(Self { maps, gpx })
    }
}

/// 画「当前活动地图」的全部地图框（普通导出与 map_only 导出共用）。
///
/// - 图层顺序 = `PreparedMap.rms` 顺序（= `gui::effective_placements` 的框顺序，数组先画的在下层）；
/// - 每个框的画布矩形 / 旋转 / 视频区都由 `gui::draw_scene_layers` 内部按
///   `composition::scene_layout` 统一计算，这里**不自己算几何**；
/// - 没有地图层（没勾地图 / 没配地图 / 这块地图没有位图）时传空图层，
///   画面只剩视频（或黑底）+ HUD，与单框时代行为一致；
/// - `rms` 长度由地图框数决定（现在是 1，多框后是 N），按长度遍历、不假设 1。
#[allow(clippy::too_many_arguments)]
fn draw_active_map_layers(
    p: &mut QPainter,
    settings: &Settings,
    video: Option<&QImage>,
    prep: &EncodePrep,
    map_index: usize,
    map_img: Option<&QImage>,
    out_w: f64,
    out_h: f64,
    t_ms: f64,
) {
    let map = &prep.maps[map_index];
    let mut layers: Vec<crate::gui::SceneLayer<'_>> = Vec::new();
    if let Some(map_img) = map_img.filter(|_| settings.encode.include_map) {
        let placements = crate::gui::effective_placements(settings);
        layers.reserve(map.rms.len().min(placements.len()));
        for (i, rm) in map.rms.iter().enumerate() {
            // 框数比取景器少/多都不会崩：只画两者都有的那些框。
            let Some(placement) = placements.get(i).copied() else {
                break;
            };
            layers.push(crate::gui::SceneLayer {
                rm,
                map: map_img,
                placement,
                route_start_ms: map.start_ms,
            });
        }
    }
    crate::gui::draw_scene_layers(p, settings, video, &layers, out_w, out_h, t_ms, prep.gpx.as_ref());
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

/// 导出用的分段表：GUI 的 job 带多段视频；空表 = 单文件调用方（CLI），合成一段占位。
/// `has_stab`：调用方为这一段准备了稳定化器。合成段必须如实标注 —— 否则按导出尺寸
/// 重新加载防抖时它会被当成「没有运动数据」，把防抖悄悄丢掉。
fn effective_segs(
    settings: &Settings,
    segs: Vec<crate::gui::VideoSeg>,
    src_fps: f64,
    duration_ms: f64,
    has_stab: bool,
) -> Vec<crate::gui::VideoSeg> {
    if !segs.is_empty() {
        return segs;
    }
    vec![crate::gui::VideoSeg {
        path: settings.video.video_path.clone(),
        start_ms: 0.0,
        duration_ms,
        fps: src_fps,
        frame_count: 0,
        has_motion: has_stab,
    }]
}

/// 按导出解码尺寸重新加载整套防抖稳定化器（每个分段一个），并做接缝预热平滑。
/// 与预览那套（限高 720）分开：稳定化器按尺寸初始化，喂不同尺寸的帧会
/// SizeMismatch / InvalidStride，所以尺寸不一致时必须重算。没有运动数据的分段是 None。
fn load_export_stabilizers(
    settings: &Settings,
    segs: &[crate::gui::VideoSeg],
    src_fps: f64,
    w: usize,
    h: usize,
) -> Result<Vec<Option<Arc<Stabilizer>>>, String> {
    let offset_ms = crate::gui::gyro_offset_ms(settings, src_fps);
    let mut slots: Vec<Option<Arc<Stabilizer>>> = vec![None; segs.len()];
    let mut loaded = 0usize;
    for (i, seg) in segs.iter().enumerate() {
        if !seg.has_motion {
            continue;
        }
        let fps = if seg.fps > 0.0 { seg.fps } else { src_fps };
        let t0 = std::time::Instant::now();
        match crate::gyro::load_stabilizer(
            &seg.path,
            &settings.gyro,
            w,
            h,
            fps,
            seg.duration_ms,
            seg.frame_count,
            false,
            offset_ms,
            crate::gyro::no_progress(),
        ) {
            Ok(s) => {
                eprintln!(
                    "[encode] 按导出尺寸 {w}x{h} 重新加载防抖：分段 {}（{:.1}s）",
                    i + 1,
                    t0.elapsed().as_secs_f64()
                );
                slots[i] = Some(Arc::new(s));
                loaded += 1;
            }
            Err(e) => {
                return Err(format!("按导出尺寸重新加载防抖失败（分段 {}）: {e}", i + 1));
            }
        }
    }
    if loaded == 0 {
        return Err("按导出尺寸重新加载防抖失败：没有任何分段带运动数据".into());
    }
    crate::gyro::apply_seam_warmup_smoothing(&slots, crate::gyro::no_progress());
    Ok(slots)
}

/// 现有稳定化器尺寸与导出解码尺寸不一致时按导出尺寸重新加载一套。
/// `Ok(None)` = 尺寸已经一致（不用动）；`Ok(Some(..))` = 新的一套；`Err` = 加载失败，
/// 调用方应把解码尺寸退回旧稳定化器的尺寸（宁可分辨率低，也不能帧与稳定化器对不上）。
fn reload_export_stabs_if_needed(
    settings: &Settings,
    segs: &[crate::gui::VideoSeg],
    src_fps: f64,
    dec_w: usize,
    dec_h: usize,
    stabs: &[Option<Arc<Stabilizer>>],
    handle: &EncodeHandle,
) -> Result<Option<Vec<Option<Arc<Stabilizer>>>>, String> {
    if !stabs
        .iter()
        .flatten()
        .any(|s| s.width != dec_w || s.height != dec_h)
    {
        return Ok(None);
    }
    // 弹窗上给一句话：重读运动数据可能要几秒到几十秒，不能让界面看起来卡住。
    // 已经有一条提示时（例如硬件编码失败、已改用 CPU 的那条）不覆盖它。
    let noticed = handle.notice().is_empty();
    if noticed {
        handle.set_notice(format!(
            "正在按导出尺寸 {dec_w}x{dec_h} 重新准备防抖（要重读运动数据）…"
        ));
    }
    let r = load_export_stabilizers(settings, segs, src_fps, dec_w, dec_h);
    if noticed {
        handle.set_notice(String::new());
    }
    r.map(Some)
}

/// 全部逐段导出：在同一后台线程上串行编码多个分段任务（每个分段一个文件）。
/// 所有 job 必须共享同一个 `handle`：进度持续发布到该 handle，用户中止（stop）
/// 会取消整个批次；某一段失败也会停止后续分段并把错误作为整批的结果上报。
/// 只有最后一个 job 完成后才翻转 `done`，GUI 全程轮询同一个 handle 即可。
pub fn encode_export_segments(jobs: Vec<EncodeJob>) {
    let Some(first) = jobs.first() else { return };
    let handle = first.handle.clone();
    let n = jobs.len();
    // 逐段导出：所有分段同一源视频、同一解码尺寸 —— 按导出尺寸重载防抖只做一次，
    // 否则每一段都会重读一遍运动数据。先把第一段要用的信息抄出来，之后才能改 jobs。
    let plan = (
        first.settings.clone(),
        first.src_fps,
        first.duration_ms,
        first.stabs.clone(),
        first.segs.clone(),
        (first.dec_w, first.dec_h),
    );
    let mut jobs = jobs;
    let (f_settings, f_fps, f_duration, f_stabs, f_segs, (f_dec_w, f_dec_h)) = plan;
    if !f_settings.encode.map_only {
        let segs = effective_segs(
            &f_settings,
            f_segs,
            f_fps,
            f_duration,
            f_stabs.iter().flatten().next().is_some(),
        );
        match reload_export_stabs_if_needed(
            &f_settings, &segs, f_fps, f_dec_w, f_dec_h, &f_stabs, &handle,
        ) {
            Ok(Some(fresh)) => {
                for job in jobs.iter_mut() {
                    job.stabs = fresh.clone();
                }
            }
            Ok(None) => {}
            Err(e) => {
                let (fw, fh) = f_stabs
                    .iter()
                    .flatten()
                    .next()
                    .map(|s| (s.width, s.height))
                    .unwrap_or((f_dec_w, f_dec_h));
                eprintln!("[encode] 逐段导出：{e}；本批改用 {fw}x{fh}（预览尺寸）解码");
                for job in jobs.iter_mut() {
                    job.dec_w = fw;
                    job.dec_h = fh;
                }
            }
        }
    }
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
        mut stabs,
        segs,
    } = job;
    // 版面只由 `include_map` / `include_video` 两个标志决定（`composition::scene_layout`）。
    // 旧单框时代这里用 `map.relative_width = 0.0` 模拟「没有地图层、视频铺满」；新版面已经
    // 按标志算，再改 relative_width 反而会把多框工程里第 1 框的贴边量算错，故删除该补丁。
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
    let segs = effective_segs(&settings, segs, src_fps, duration_ms, stabs.iter().flatten().next().is_some());
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
    let (mut dec_w, mut dec_h) = if dec_w >= 2 && dec_h >= 2 {
        (dec_w, dec_h)
    } else {
        crate::gui::processing_size(src_w, src_h, out_h)
    };
    // 稳定化器按尺寸初始化，喂不同尺寸的帧会 SizeMismatch / InvalidStride：导出尺寸比
    // 预览那套（限高 720）大时（现在常见）必须按导出尺寸重新加载一套。加载失败就退回旧
    // 稳定化器的尺寸 —— 宁可分辨率低一点，也不能让画面和防抖对不上。
    match reload_export_stabs_if_needed(&settings, &segs, src_fps, dec_w, dec_h, &stabs, &handle) {
        Ok(Some(fresh)) => stabs = fresh,
        Ok(None) => {}
        Err(e) => {
            let (fw, fh) = stabs
                .iter()
                .flatten()
                .next()
                .map(|s| (s.width, s.height))
                .unwrap_or((dec_w, dec_h));
            eprintln!("[encode] {e}；回退到 {fw}x{fh}（预览尺寸）");
            dec_w = fw;
            dec_h = fh;
        }
    }
    crate::log_info!(
        "解码尺寸 {dec_w}x{dec_h}（输出 {out_w}x{out_h}，源 {src_w}x{src_h}，防抖={}）",
        if stabs.iter().flatten().next().is_some() { "开" } else { "关" }
    );
    // Export speed N: sample source time N× faster (frame-skip), output fps unchanged,
    // so the clip holds total/N frames and is N× shorter. Audio matches via atempo.
    let speed_n = encode_speed_ratio(&settings.encode);
    let total =
        (((((clip_end - clip_start) / 1000.0) * out_fps) / speed_n).round() as usize).max(1);
    let dt_out = 1000.0 / out_fps;
    // 跟随跑者按内容时间平滑；update_playback 会将固定分段的转图动画
    // 换算成输出帧时间，避免倍速时动画也加速。
    let cam_dt = dt_out * speed_n;
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
        let step = cam_dt / 1000.0;
        while wt < warm_end {
            // Warm up EVERY map's runner-smoothing, not just the one active at
            // clip_start: a later map switch must not "settle" on its first frame.
            // 每个地图 × 每个地图框各有一套取景器（rms），全部预热。
            for m in &mut prep.maps {
                for rm in &mut m.rms {
                    rm.update_playback(wt, cam_dt, speed_n);
                }
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
            // Advance EVERY map's runner/camera (state must be live at a switch), and
            // every map box's camera; render the map active on the global timeline.
            for m in &mut prep.maps {
                for rm in &mut m.rms {
                    rm.update_playback(t_global / 1000.0, cam_dt, speed_n);
                }
            }
            let mi = crate::gui::locate_map(&map_starts, t_global);
            let video_layer = if settings.encode.include_video {
                Some(&video_img)
            } else {
                None
            };
            draw_active_map_layers(
                &mut p,
                &settings,
                video_layer,
                &prep,
                mi,
                map_imgs[mi].as_ref(),
                out_w as f64,
                out_h as f64,
                t_global,
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
        // 中止留下的部分文件同样要能发到手机上：先让弹窗说明"正在重封装"（这一步是
        // 与已写入大小成正比的纯搬运，大文件要等几秒），再重封装，最后才发布结果。
        handle.set_notice(
            "已停止：正在把已写入的部分封装成手机兼容的标准 MP4（只搬运码流，不重新编码）…"
                .into(),
        );
        enc.abort();
        let keep = video_track_end_s(Path::new(&out_path));
        let packed = finalize_mobile_mp4(&out_path, keep);
        let msg = if packed {
            "编码已中止（部分文件已保留，并已封装为标准 MP4，可发手机）"
        } else {
            "编码已中止（部分文件已保留，可播放）"
        };
        return finish(true, msg.into());
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
    settings.encode.include_video = false;
    // 不再用 `relative_width = 1.0` 模拟「地图铺满整幅」：新版面由 scene_layout 决定 ——
    // 无视频时贴边的第 1 框自动铺满整幅，自由框保持自己的矩形（多框就叠在黑底上）。
    settings.encode.audio_codec = "none".into();
    if !settings.encode.include_hud {
        settings.hud.layout = Some(crate::hud::Layout {
            version: 1,
            widgets: Vec::new(),
        });
    }
    // Map-only frame t is a route timestamp, so the HUD has to read the track at exactly that
    // instant. `hud::hud_track_offset` = `gpx_time_offset - start_time_offset_s`; equalising the
    // two calibration values below makes the route clock stand in for the video clock, and the
    // GPX offset (a property of the GPX file itself) then applies as-is. This spot used to patch
    // `gpx_time_offset` by hand against the old "offset from the video start" convention — that
    // arithmetic would double-count under the race-start anchoring.
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
    let route_dur_ms = crate::gui::track_timeline_duration_ms(
        prep.maps.iter().map(|m| m.rm.duration_s()),
        prep.gpx.as_ref(),
    );
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
    // 与普通导出共用双时间轴：跟随跑者用内容时间，固定分段转图用成片时间。
    let cam_dt = dt_out * speed_n;
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
        let step = cam_dt / 1000.0;
        while wt < warm_end {
            // 每个地图 × 每个地图框的取景器都要预热（见普通导出的同一段说明）。
            for m in &mut prep.maps {
                for rm in &mut m.rms {
                    rm.update_playback(wt, cam_dt, speed_n);
                }
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
                for rm in &mut m.rms {
                    rm.update_playback(secs, cam_dt, speed_n);
                }
            }
            let mi = crate::gui::locate_map(&map_starts, t_ms);
            draw_active_map_layers(
                &mut p,
                &settings,
                None,
                &prep,
                mi,
                map_imgs[mi].as_ref(),
                out_w as f64,
                out_h as f64,
                t_ms,
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
        // 同 encode_export_attempt：中止的部分文件也要能发到手机上。
        handle.set_notice(
            "已停止：正在把已写入的部分封装成手机兼容的标准 MP4（只搬运码流，不重新编码）…"
                .into(),
        );
        enc.abort();
        let keep = video_track_end_s(Path::new(&out_path));
        let packed = finalize_mobile_mp4(&out_path, keep);
        finish(
            true,
            if packed {
                "编码已中止（部分文件已保留，并已封装为标准 MP4，可发手机）".into()
            } else {
                "编码已中止（部分文件已保留，可播放）".into()
            },
        );
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
    fn output_size_keeps_source_resolution_when_asked() {
        let mut enc = enc_base();
        // 「与原始视频一致」= 不缩放，直接用源尺寸（只做偶数对齐）。
        enc.resolution = "source".into();
        assert_eq!(encode_output_size(&enc, 2688, 2016), (2688, 2016));
        assert_eq!(encode_output_size(&enc, 1920, 1081), (1920, 1080));
        // 显式尺寸仍然照写（含 4:3），旧档位名仍能解析。
        enc.resolution = "1440x1080".into();
        assert_eq!(encode_output_size(&enc, 2688, 2016), (1440, 1080));
        enc.resolution = "4k".into();
        assert_eq!(encode_output_size(&enc, 2688, 2016), (3840, 2160));
        enc.resolution = "1080p".into();
        assert_eq!(encode_output_size(&enc, 2688, 2016), (1920, 1080));
        // 默认值与无法解析的写法回退 1920x1080。
        enc.resolution = String::new();
        assert_eq!(encode_output_size(&enc, 2688, 2016), (1920, 1080));
        enc.resolution = "原始".into();
        assert_eq!(encode_output_size(&enc, 2688, 2016), (1920, 1080));
    }

    #[test]
    fn export_decode_size_follows_the_output_panel() {
        let mut s = crate::settings::Settings::default();
        // 默认工程没配地图（面板 = 整幅画布）；4:3 源受高度约束 → 1440x1080。
        s.encode.resolution = "1920x1080".into();
        assert_eq!(export_decode_size(&s, 2688, 2016), (1440, 1080));
        // 配了地图（占 1/4 宽）时面板更窄，但 4:3 源本例仍受高度约束 → 同尺寸。
        s.map.image_file_path = "D:/map.qrt".into();
        assert_eq!(export_decode_size(&s, 2688, 2016), (1440, 1080));
        // 4K 输出：顶到源尺寸（旧行为是 1280x720，被放大 3 倍）。
        s.encode.resolution = "3840x2160".into();
        assert_eq!(export_decode_size(&s, 2688, 2016), (2688, 2016));
        // 720p 输出：与旧行为一致（就是 960x720，1:1 不放大）。
        s.encode.resolution = "1280x720".into();
        assert_eq!(export_decode_size(&s, 2688, 2016), (960, 720));
        // 源比输出小：绝不放大。
        s.encode.resolution = "1920x1080".into();
        assert_eq!(export_decode_size(&s, 640, 480), (640, 480));
        // 用户把视频放大 2×：按放大后的像素解码，上限仍是源。
        s.video.scale = 2.0;
        assert_eq!(export_decode_size(&s, 2688, 2016), (2688, 2016));
        // 导出内容不含地图 → 面板按整幅画布算（与编码时的等价改动一致）。
        s.video.scale = 1.0;
        s.encode.include_map = false;
        assert_eq!(export_decode_size(&s, 2688, 2016), (1440, 1080));
    }

    #[test]
    fn export_decode_size_respects_docks_and_overlay() {
        let mut s = Settings::default();
        s.encode.resolution = "1920x1080".into();
        s.map.image_file_path = "map.png".into();
        s.map.relative_width = 0.5;
        for mode in [0, 1] {
            s.map.placement = mode;
            assert_eq!(export_decode_size(&s, 1920, 1080), (960, 540));
        }
        s.map.placement = 4;
        assert_eq!(export_decode_size(&s, 1920, 1080), (1920, 1080));
        s.encode.include_map = false;
        s.map.placement = 2;
        assert_eq!(export_decode_size(&s, 1920, 1080), (1920, 1080));
    }

    /// 多地图框（自由叠加）不改变导出解码尺寸：视频区仍是整幅画布。
    ///
    /// `export_decode_size` 只读 `scene_panels().video`，而 `composition::scene_layout`
    /// 规定「只有第 1 个框可以贴边，第 2 个及以后的框一律自由叠加」——自由框不挤压视频区。
    /// 所以多框工程的导出解码尺寸必须与「没有地图层」完全一致；这里同时用两个自由框的
    /// `scene_layout` 钉住这条真相（多框落地后 `effective_placements` 返回 N 个框时，
    /// `export_decode_size` 无需改动）。
    #[test]
    fn export_decode_size_is_unchanged_by_free_overlay_boxes() {
        let mut s = Settings::default();
        s.encode.resolution = "1920x1080".into();
        let without_map = export_decode_size(&s, 2688, 2016);

        s.map.image_file_path = "map.png".into();
        s.map.placement = 4; // 自由叠加
        assert_eq!(export_decode_size(&s, 2688, 2016), without_map);

        // 两个自由框：视频区仍是整幅（上面「解码尺寸不变」的依据）。
        let first = s.map.placement();
        let mut second = first;
        second.frame.x = 0.6;
        second.frame.y = 0.5;
        let layout = crate::composition::scene_layout(&[first, second], 1920.0, 1080.0, true, true);
        assert_eq!(
            layout.video,
            crate::composition::Rect {
                x: 0.0,
                y: 0.0,
                width: 1920.0,
                height: 1080.0
            }
        );

        // 对照：第 1 框贴边（单框 docked）会挤压视频区 → 解码尺寸变小。
        // 说明多框时只有「不能贴边的多余框」是免费叠加的。
        s.map.placement = 0;
        s.map.relative_width = 0.5;
        assert_eq!(export_decode_size(&s, 1920, 1080), (960, 540));
    }

    /// 占位（无地图位图 / 无取景器）时导出帧仍能正常合成：不 panic、保持黑底。
    /// 覆盖 `draw_active_map_layers` 的空图层与空 `rms` 分支（只导视频、或地图加载失败的工程）。
    #[test]
    fn draw_active_map_layers_tolerates_placeholder_maps() {
        let mut s = Settings::default();
        s.hud.layout = Some(crate::hud::Layout {
            version: 1,
            widgets: Vec::new(),
        });
        s.encode.include_map = true;
        s.encode.include_video = false;
        let prep = EncodePrep {
            maps: vec![crate::gui::PreparedMap {
                rgba: Vec::new(),
                w: 0,
                h: 0,
                start_ms: 0.0,
                rm: Default::default(),
                rms: Vec::new(),
            }],
            gpx: None,
        };
        let mut img = crate::qtwrap::qimage_new_argb32(64, 36);
        {
            let mut p = crate::qtwrap::qpainter_on_image(&mut img);
            draw_active_map_layers(&mut p, &s, None, &prep, 0, None, 64.0, 36.0, 0.0);
        }
        let rgba = crate::qtwrap::qimage_to_rgba(&img);
        assert_eq!(&rgba[..4], &[0, 0, 0, 255], "占位地图应画成黑底");
    }

    /// 多框导出路径像素回归：`EncodePrep::build` → `draw_active_map_layers`。
    ///
    /// 用真实的小地图文件（临时纯红 PNG）跑 build，确保「每张地图解析一次 → 每框一套
    /// 取景器（rms）→ 逐框绘制」这条导出链路是通的：两个自由框各自画出地图，
    /// 两框之间的空白保持黑底。
    #[test]
    fn multi_box_export_paints_every_box() {
        let dir = std::env::temp_dir().join(format!(
            "tracereel-multibox-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let map_path = dir.join("map.png");
        let map_src =
            crate::qtwrap::qimage_from_rgba(1000, 1000, &[220, 60, 40, 255].repeat(1_000_000));
        assert!(crate::qtwrap::qimage_save(
            &map_src,
            map_path.to_str().unwrap()
        ));

        let mut s = Settings::default();
        s.map.image_file_path = map_path.to_string_lossy().into_owned();
        s.map.layout = crate::settings::MapLayout {
            version: crate::settings::MAP_LAYOUT_VERSION,
            boxes: vec![
                crate::settings::MapBox {
                    mode: 4,
                    x: 0.05,
                    y: 0.1,
                    w: 0.35,
                    h: 0.7,
                    rotation: 15.0,
                    ..Default::default()
                },
                crate::settings::MapBox {
                    mode: 4,
                    x: 0.55,
                    y: 0.15,
                    w: 0.35,
                    h: 0.7,
                    rotation: -25.0,
                    ..Default::default()
                },
            ],
        };
        s.map.sync_legacy_from_layout(&mut s.route);
        s.encode.include_video = false;
        s.encode.map_only = true;
        s.encode.include_hud = false;
        s.hud.layout = Some(crate::hud::Layout {
            version: 1,
            widgets: Vec::new(),
        });

        let (w, h) = (400usize, 240usize);
        let prep = EncodePrep::build(&s, w, h).expect("EncodePrep::build 失败");
        assert_eq!(prep.maps.len(), 1, "只配了一张地图");
        assert_eq!(prep.maps[0].rms.len(), 2, "两个框应各自有一套取景器");
        let map_img =
            crate::qtwrap::qimage_from_rgba(prep.maps[0].w, prep.maps[0].h, &prep.maps[0].rgba);
        let mut canvas = crate::qtwrap::qimage_new_argb32(w as i32, h as i32);
        {
            let mut p = crate::qtwrap::qpainter_on_image(&mut canvas);
            draw_active_map_layers(
                &mut p,
                &s,
                None,
                &prep,
                0,
                Some(&map_img),
                w as f64,
                h as f64,
                0.0,
            );
        }
        let rgba = crate::qtwrap::qimage_to_rgba(&canvas);
        let boxes = crate::composition::scene_layout(
            &crate::gui::effective_placements(&s),
            w as f64,
            h as f64,
            true,
            false,
        )
        .boxes;
        assert_eq!(boxes.len(), 2);
        for (i, b) in boxes.iter().enumerate() {
            let cx = ((b.rect.x + b.rect.width / 2.0) as usize).min(w - 1);
            let cy = ((b.rect.y + b.rect.height / 2.0) as usize).min(h - 1);
            let k = (cy * w + cx) * 4;
            assert!(
                rgba[k] > 200 && rgba[k + 1] < 70 && rgba[k + 2] < 70,
                "第 {i} 框中心 ({cx},{cy}) 不是地图红: {:?}",
                &rgba[k..k + 4]
            );
        }
        // 两框之间（x 方向中间、y 居中）没有框覆盖 → 黑底。
        let k = (h / 2 * w + w / 2) * 4;
        assert!(
            rgba[k] < 60 && rgba[k + 1] < 60 && rgba[k + 2] < 60,
            "两框之间应为黑底: {:?}",
            &rgba[k..k + 4]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 多框导出使用**逐框轨迹样式**：`EncodePrep::build`（真实读图）+ 生产
    /// `gui::build_frame_managers`（每框样式写进对应 RM）→ `draw_active_map_layers`。
    ///
    /// 两个自由框：框0 红路线/线宽 12/完整渲染，框1 绿路线/线宽 6/单段渲染；
    /// 时间校准（按表-出发 3s、录像-出发 1s → runner 偏移 2s）必须保持全局一致。
    #[test]
    fn multi_box_export_uses_per_box_styles() {
        let dir = std::env::temp_dir().join(format!(
            "tracereel-multibox-style-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let map_path = dir.join("map.png");
        let map_src =
            crate::qtwrap::qimage_from_rgba(1000, 1000, &[60, 60, 60, 255].repeat(1_000_000));
        assert!(crate::qtwrap::qimage_save(
            &map_src,
            map_path.to_str().unwrap()
        ));

        let mut s = Settings::default();
        let mut red = crate::settings::MapBoxStyle::from_route(&s.route);
        red.render_mode = 1;
        red.render_scope = 0;
        red.width = 12.0;
        red.tail_render_mode = 0;
        red.show_controls = false;
        red.show_runner = false;
        red.route_color = "#ff0000".into();
        red.route_opacity = 255.0;
        red.pace_auto = false;
        red.tail_pace_auto = false;
        let mut green = red.clone();
        green.render_scope = 1;
        green.width = 6.0;
        green.route_color = "#00ff00".into();

        s.map.image_file_path = map_path.to_string_lossy().into_owned();
        s.map.layout = crate::settings::MapLayout {
            version: crate::settings::MAP_LAYOUT_VERSION,
            boxes: vec![
                crate::settings::MapBox {
                    mode: 4,
                    x: 0.02,
                    y: 0.03,
                    w: 0.3,
                    h: 0.94,
                    view_mode: 1,
                    view_zoom: 0.5,
                    style: Some(red),
                    ..Default::default()
                },
                crate::settings::MapBox {
                    mode: 4,
                    x: 0.68,
                    y: 0.03,
                    w: 0.3,
                    h: 0.94,
                    view_mode: 1,
                    view_zoom: 0.5,
                    style: Some(green),
                    ..Default::default()
                },
            ],
        };
        s.map.sync_legacy_from_layout(&mut s.route);
        s.route.control_time_offset = 3.0;
        s.video.start_time_offset_s = 1.0;
        s.encode.include_video = false;
        s.encode.map_only = true;
        s.encode.include_hud = false;
        s.hud.layout = Some(crate::hud::Layout {
            version: 1,
            widgets: Vec::new(),
        });

        let (w, h) = (900usize, 300usize);
        let mut prep = EncodePrep::build(&s, w, h).expect("EncodePrep::build 失败");
        assert_eq!(prep.maps.len(), 1, "只配了一张地图");
        // 生产路径：每框一套取景器，且每框自己的样式写进对应 RM。
        let mut base = tracereel_core::route_manager::RouteManager::default();
        let points: Vec<tracereel_core::route::RoutePoint> = (0..=60)
            .map(|i| {
                let mut p = tracereel_core::route::RoutePoint::new(
                    100.0 + i as f64 * 10.0,
                    300.0 + (i % 10) as f64 * 4.0,
                    i as f64,
                );
                p.pace = 5.0 + (i % 7) as f64;
                p
            })
            .collect();
        base.initialize_from_points(
            points,
            "0:10|0:20|0:40",
            false,
            crate::gui::route_style_from_settings(&s),
            tracereel_core::route_manager::ViewMode::RunnerCentered,
            1.0,
        );
        prep.maps[0].rms = crate::gui::build_frame_managers(&s, &base, w as f64, h as f64);
        assert_eq!(prep.maps[0].rms.len(), 2, "两个框应各自有一套取景器");

        let st0 = prep.maps[0].rms[0].routes[0].style.clone();
        let st1 = prep.maps[0].rms[1].routes[0].style.clone();
        assert!(
            (st0.route_width - 12.0).abs() < 1e-9 && (st1.route_width - 6.0).abs() < 1e-9,
            "逐框线宽未生效: {} / {}",
            st0.route_width,
            st1.route_width
        );
        assert_eq!(
            (st0.render_scope, st1.render_scope),
            (0, 1),
            "逐框渲染范围未生效"
        );
        assert!(
            st0.route_color.0 > 0.9 && st0.route_color.1 < 0.1,
            "框0 路线应红: {:?}",
            st0.route_color
        );
        assert!(
            st1.route_color.1 > 0.9 && st1.route_color.0 < 0.1,
            "框1 路线应绿: {:?}",
            st1.route_color
        );
        // 校准时间保持全局：逐框样式不得改它。
        for (i, rm) in prep.maps[0].rms.iter().enumerate() {
            let route = &rm.routes[0];
            assert!(
                (route.control_time_offset - 3.0).abs() < 1e-9,
                "框{i} control_time_offset 被逐框样式改掉"
            );
            assert!(
                (route.runner_time_offset - 2.0).abs() < 1e-9,
                "框{i} runner_time_offset 被逐框样式改掉"
            );
            assert!((route.style.control_time_offset - 3.0).abs() < 1e-9);
            assert!((route.style.runner_time_offset - 2.0).abs() < 1e-9);
        }

        // 渲染 t=15s（第 2 段）：框0 红（完整渲染）、框1 绿（单段渲染）。
        for rm in prep.maps[0].rms.iter_mut() {
            rm.instant_split_transition_requested = true;
            rm.update(15.0, 0.0);
        }
        let map_img =
            crate::qtwrap::qimage_from_rgba(prep.maps[0].w, prep.maps[0].h, &prep.maps[0].rgba);
        let mut canvas = crate::qtwrap::qimage_new_argb32(w as i32, h as i32);
        {
            let mut p = crate::qtwrap::qpainter_on_image(&mut canvas);
            draw_active_map_layers(
                &mut p,
                &s,
                None,
                &prep,
                0,
                Some(&map_img),
                w as f64,
                h as f64,
                15_000.0,
            );
        }
        let rgba = crate::qtwrap::qimage_to_rgba(&canvas);
        let boxes = crate::composition::scene_layout(
            &crate::gui::effective_placements(&s),
            w as f64,
            h as f64,
            true,
            false,
        )
        .boxes;
        assert_eq!(boxes.len(), 2);
        let in_box = |x: usize, y: usize, b: &crate::composition::SceneBox| {
            let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
            px >= b.rect.x
                && px <= b.rect.x + b.rect.width
                && py >= b.rect.y
                && py <= b.rect.y + b.rect.height
        };
        let (mut red0, mut green0, mut red1, mut green1) = (0usize, 0usize, 0usize, 0usize);
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 4;
                let (r, g, b) = (rgba[i], rgba[i + 1], rgba[i + 2]);
                let is_red = r > 150 && g < 100 && b < 100;
                let is_green = g > 150 && r < 100 && b < 100;
                if in_box(x, y, &boxes[0]) {
                    if is_red {
                        red0 += 1;
                    }
                    if is_green {
                        green0 += 1;
                    }
                }
                if in_box(x, y, &boxes[1]) {
                    if is_red {
                        red1 += 1;
                    }
                    if is_green {
                        green1 += 1;
                    }
                }
            }
        }
        assert!(red0 > 150, "框0 没画出红路线（{red0} px）");
        assert!(green1 > 30, "框1 没画出绿路线（{green1} px）");
        assert_eq!(green0, 0, "框0 出现绿像素：逐框样式串了");
        assert_eq!(red1, 0, "框1 出现红像素：逐框样式串了");
        let _ = std::fs::remove_dir_all(&dir);
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

    #[test]
    fn finalize_remux_is_a_stream_copy_with_faststart() {
        let cmd = finalize_remux_command("ffmpeg.exe", "out.mp4", "out.mobile.tmp.mp4", None);
        let joined = cmd.join(" ");
        assert!(joined.contains("-c copy"), "{joined}");
        assert!(joined.contains("-movflags +faststart"), "{joined}");
        assert!(joined.trim_end().ends_with("out.mobile.tmp.mp4"), "{joined}");
        // 完整导出不裁剪。
        assert!(!joined.contains("-t "), "{joined}");
    }

    /// 中止导出：按视频轨结尾裁剪，`-t` 必须是**输出**选项（在输出路径之前）。
    #[test]
    fn finalize_remux_trims_to_the_video_end_when_asked() {
        let cmd = finalize_remux_command(
            "ffmpeg.exe",
            "out.mp4",
            "out.mobile.tmp.mp4",
            Some(3.0666),
        );
        let joined = cmd.join(" ");
        assert!(joined.contains("-t 3.067"), "{joined}");
        let t_at = cmd.iter().position(|a| a == "-t").unwrap();
        let out_at = cmd.len() - 1;
        assert!(t_at < out_at, "{joined}");
    }

    #[test]
    fn tmp_path_stays_next_to_the_output_and_keeps_the_extension() {
        let tmp = tmp_sibling_path(Path::new(r"D:\v\测试导出1x.mp4"));
        assert_eq!(
            tmp.file_name().and_then(|s| s.to_str()),
            Some("测试导出1x.mobile.tmp.mp4")
        );
        assert_eq!(tmp.parent().and_then(|p| p.to_str()), Some(r"D:\v"));
    }

    /// 8 字节头的最小盒子。
    fn bx(typ: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut v = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(typ);
        v.extend_from_slice(payload);
        v
    }

    /// 64 位长度头（size==1）的盒子。
    fn bx64(typ: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut v = 1u32.to_be_bytes().to_vec();
        v.extend_from_slice(typ);
        v.extend_from_slice(&((payload.len() + 16) as u64).to_be_bytes());
        v.extend_from_slice(payload);
        v
    }

    /// 分片 MP4（顶层有 moof）必须被认出来；标准 MP4（moov+mdat）不能误判。
    #[test]
    fn moof_detection_distinguishes_fragmented_mp4() {
        let dir = std::env::temp_dir().join(format!("tracereel-moof-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let frag = dir.join("frag.mp4");
        let mut data = bx(b"ftyp", b"isom\x00\x00\x02\x00");
        data.extend(bx(b"moov", &vec![0u8; 32]));
        data.extend(bx(b"moof", &vec![0u8; 16]));
        data.extend(bx(b"mdat", &vec![0u8; 64]));
        std::fs::write(&frag, &data).unwrap();
        assert!(mp4_has_moof(&frag), "分片 MP4 应判为分片");

        let plain = dir.join("plain.mp4");
        let mut data = bx(b"ftyp", b"isom\x00\x00\x02\x00");
        data.extend(bx(b"moov", &vec![0u8; 32]));
        data.extend(bx64(b"mdat", &vec![0u8; 4096]));
        std::fs::write(&plain, &data).unwrap();
        assert!(!mp4_has_moof(&plain), "标准 MP4 不应判为分片");

        let _ = std::fs::remove_dir_all(&dir);
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
            range_start_ms: 0.0,
            range_end_ms: 0.0,
            clip_mode: "none".into(),
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
            range_start_ms: 0.0,
            range_end_ms: 0.0,
            clip_mode: "none".into(),
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
