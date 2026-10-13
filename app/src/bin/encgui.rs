//! encgui — headless test for the FULL encode pipeline exactly as the GUI runs it:
//! spawn `tracereel::encode::encode_export` on a worker thread (decode + stabilize +
//! off-main QPainter compositing incl. HUD + ffmpeg feeding) and watch progress from the
//! shared handle, mirroring `TraceReelItem::start_encode` + `poll_encode`.
//!
//! Usage: encgui <project.trr> --out <mp4> [--max-frames N] [--abort-after N]

use std::sync::atomic::Ordering;
use std::sync::Arc;

use qmetaobject::QmlEngine;

use tracereel::encode::{clip_range, EncodeHandle, EncodeJob};
use tracereel::settings::Settings;

fn main() {
    // MSVC/Qt 断言默认弹模态对话框，自动化跑会一直卡住等人点「忽略」；
    // 这里统一改成打到 stderr（与 hudcheck / gui.rs 的断言静默钩子一致）。
    tracereel::qtwrap::asserts_silent_continue();

    let args: Vec<String> = std::env::args().collect();
    let mut orv = String::new();
    let mut out = "encgui_out.mp4".to_string();
    let mut max_frames = 0usize;
    let mut abort_after = 0usize;
    let mut resolution = String::new();
    let mut frame_rate = String::new();
    let mut start_s = -1.0f64;
    let mut end_s = -1.0f64;
    let mut map_only = false;
    let mut start_seg = 0i32;
    let mut end_seg = 0i32;
    // 编码后端 / 格式覆盖：用于验证"硬件编码不可用 → 自动回退 CPU"的真实导出路径
    // （例如 --backend qsv_bogus --format av1 制造一个必然打不开的编码器）。
    let mut backend = String::new();
    let mut format = String::new();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--out" => {
                i += 1;
                out = args.get(i).cloned().unwrap_or_default();
            }
            "--max-frames" => {
                i += 1;
                max_frames = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(0);
            }
            "--abort-after" => {
                i += 1;
                abort_after = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(0);
            }
            "--resolution" => {
                i += 1;
                resolution = args.get(i).cloned().unwrap_or_default();
            }
            "--fps" => {
                i += 1;
                frame_rate = args.get(i).cloned().unwrap_or_default();
            }
            "--start" => {
                i += 1;
                start_s = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(-1.0);
            }
            "--end" => {
                i += 1;
                end_s = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(-1.0);
            }
            "--map-only" => {
                map_only = true;
            }
            "--backend" => {
                i += 1;
                backend = args.get(i).cloned().unwrap_or_default();
            }
            "--format" => {
                i += 1;
                format = args.get(i).cloned().unwrap_or_default();
            }
            "--start-seg" => {
                i += 1;
                start_seg = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(0);
            }
            "--end-seg" => {
                i += 1;
                end_seg = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(0);
            }
            _ if orv.is_empty() => orv = args[i].clone(),
            _ => {}
        }
        i += 1;
    }
    if orv.is_empty() {
        eprintln!("usage: encgui <project.trr> --out <mp4> [--max-frames N] [--abort-after N] [--resolution <p>] [--fps <n>] [--start <s>] [--end <s>]");
        std::process::exit(2);
    }
    // QGuiApplication must exist before any QImage/QPainter use (the worker paints off-main).
    let _engine = QmlEngine::new();

    let mut settings = Settings::load_orv(&orv);
    if !resolution.is_empty() {
        settings.encode.resolution = resolution;
    }
    if !frame_rate.is_empty() {
        settings.encode.frame_rate = frame_rate;
    }
    if start_s >= 0.0 {
        settings.encode.clip_start_ms = start_s * 1000.0;
    }
    if end_s >= 0.0 {
        settings.encode.clip_end_ms = end_s * 1000.0;
    }
    if map_only {
        settings.encode.map_only = true;
    }
    if !backend.is_empty() {
        settings.encode.backend = backend.clone();
        eprintln!("[encgui] backend override = {backend}");
    }
    if !format.is_empty() {
        settings.encode.format = format.clone();
        eprintln!("[encgui] format override = {format}");
    }
    // Segment boundaries → concrete clip ms (segments win over --start/--end when given).
    if start_seg > 0 || end_seg > 0 {
        settings.encode.seg_enabled = true;
        settings.encode.start_seg = start_seg;
        settings.encode.end_seg = end_seg;
        settings.resolve_encode_clip_ms();
        eprintln!(
            "[encgui] segments {start_seg}..{end_seg} → clip {:.0}..{:.0}ms",
            settings.encode.clip_start_ms, settings.encode.clip_end_ms
        );
    }
    // Debug: dump the split-point table + resolved clip and exit (no encode) when
    // TRACEREEL_RESOLVE_ONLY is set — lets me verify the segment→video-time mapping on real data.
    if std::env::var("TRACEREEL_RESOLVE_ONLY").is_ok() {
        let info = tracereel_core::splits::build_runner_info(
            &settings.splits.split_times,
            settings.splits.is_relative,
        );
        let pts: Vec<String> = info
            .splits
            .iter()
            .enumerate()
            .map(|(i, s)| format!("{i}:{}", (s.absolute_time * 1000.0).round() as i64))
            .collect();
        eprintln!("[encgui] off_ms={} seg_enabled={} start_seg={start_seg} end_seg={end_seg} → clip {:.0}..{:.0}ms\n         points = [{}]",
            (settings.video.start_time_offset_s * 1000.0) as i64, settings.encode.seg_enabled,
            settings.encode.clip_start_ms, settings.encode.clip_end_ms, pts.join(" "));
        return;
    }

    let handle = Arc::new(EncodeHandle::default());

    if settings.encode.map_only {
        eprintln!(
            "[encgui] MAP-ONLY export -> {out} (res={}, fps={})",
            settings.encode.resolution, settings.encode.frame_rate
        );
        let job = EncodeJob {
            settings,
            out_path: out.clone(),
            src_w: 0,
            src_h: 0,
            src_fps: 0.0,
            dec_w: 0,
            dec_h: 0,
            duration_ms: 0.0,
            clip_start: 0.0,
            clip_end: 0.0,
            handle: handle.clone(),
            stabs: Vec::new(),
            segs: Vec::new(),
        };
        std::thread::spawn(move || tracereel::encode::encode_export(job));
    } else {
        let info = tracereel::probe::probe_video(&settings.video.video_path).expect("probe video");
        let fps = info.fps;
        if max_frames > 0 {
            settings.encode.clip_end_ms = (max_frames as f64 * 1000.0 / fps).min(info.duration_ms);
        }
        let (clip_start, clip_end) = clip_range(&settings.encode, info.duration_ms);
        let total = (((clip_end - clip_start) / 1000.0) * fps).round() as usize;
        eprintln!("[encgui] clip {clip_start}..{clip_end}ms total={total}");

        // Decode + stabilizer buffer size: the pixels the output canvas' video panel actually
        // needs, capped at the source size (see `encode::export_decode_size`). draw_scene then
        // scales this AR-preserved frame into the chosen output panel without stretching.
        let (dec_w, dec_h) = tracereel::encode::export_decode_size(&settings, info.width, info.height);

        // Optional gyro stabilizer — must be loaded at exactly the decode size: gyroflow's
        // stabilizer is bound to the size it was initialised with.
        let stab = if settings.gyro.enabled && info.has_motion_data {
            eprintln!("[encgui] loading stabilizer at {dec_w}x{dec_h}...");
            let s = tracereel::gyro::load_stabilizer(
                &settings.video.video_path,
                &settings.gyro,
                dec_w,
                dec_h,
                fps,
                info.duration_ms,
                info.frame_count,
                false,
                tracereel::gui::gyro_offset_ms(&settings, fps),
                tracereel::gyro::no_progress(),
            )
            .expect("stabilizer");
            tracereel::gyro::init_gpu();
            Some(Arc::new(s))
        } else {
            None
        };

        let job = EncodeJob {
            settings,
            out_path: out.clone(),
            src_w: info.width,
            src_h: info.height,
            src_fps: fps,
            dec_w,
            dec_h,
            duration_ms: info.duration_ms,
            clip_start,
            clip_end,
            handle: handle.clone(),
            stabs: vec![stab],
            segs: Vec::new(),
        };
        std::thread::spawn(move || tracereel::encode::encode_export(job));
    }

    if abort_after > 0 {
        let h = handle.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_millis(100));
            if h.get().frame >= abort_after {
                eprintln!("[encgui] requesting abort at frame {}", h.get().frame);
                h.stop.store(true, Ordering::SeqCst);
                break;
            }
            if h.is_done() {
                break;
            }
        });
    }

    // Poll progress from the handle (like poll_encode), printing occasionally.
    loop {
        let p = handle.get();
        if p.frame > 0 && p.frame % 30 == 0 && p.frame < p.total_frames {
            println!(
                "frame {}/{} elapsed={:.1}s remaining={:.1}s fps={:.1} t={:.1}s",
                p.frame,
                p.total_frames,
                p.elapsed_s,
                p.remaining_s,
                p.fps,
                p.current_t_ms / 1000.0
            );
        }
        if handle.is_done() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(16));
    }
    let r = handle.take_result().expect("result");
    let sz = std::fs::metadata(&r.output).map(|m| m.len()).unwrap_or(0);
    println!(
        "RESULT ok aborted={} path={} size={} frames={}",
        r.aborted,
        r.output,
        sz,
        handle.get().frame
    );
    // 非致命提示（例如"已自动改用 CPU"）：暴露出来供自动化验证断言。
    let notice = handle.notice();
    if !notice.is_empty() {
        println!("NOTICE {notice}");
    }
    if !r.error.is_empty() {
        println!("ERROR {}", r.error);
    }
    if r.error.is_empty() && sz == 0 {
        std::process::exit(1);
    }
}
