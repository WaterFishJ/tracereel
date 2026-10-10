//! stabvid — stabilize-only video export (no map/route/HUD overlay), for comparing the
//! Gyroflow stabilization against the source. Ports the frame-server path.
//!
//! Usage: stabvid <path.trr> --out <mp4> [--seconds N] [--w N --h N] [--max-frames N]

use qmetaobject::QmlEngine;

fn main() {
    // MSVC/Qt 断言默认弹模态对话框，自动化跑会一直卡住等人点「忽略」；
    // 这里统一改成打到 stderr（与 hudcheck / gui.rs 的断言静默钩子一致）。
    tracereel::qtwrap::asserts_silent_continue();

    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .try_init();
    let args: Vec<String> = std::env::args().collect();
    let mut orv = String::new();
    let mut out = "stab_out.mp4".to_string();
    let mut seconds = 300.0f64;
    let mut w = 1280usize;
    let mut h = 960usize;
    let mut max_frames = 0usize;
    let mut skip = 0.0f64;
    let mut proc_h = 720usize;
    let mut no_autosync = true; // DJI gyro is GPS-synced (offset ~0); a spurious rs_sync offset makes it worse
    let mut offset_ms = 0.0f64;
    let mut offset_frames: i32 = 0;
    let mut dump_quats: Option<String> = None;
    let mut dump_frames: Option<String> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--out" => {
                i += 1;
                out = args.get(i).cloned().unwrap_or_default();
            }
            "--seconds" => {
                i += 1;
                seconds = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(seconds);
            }
            "--w" => {
                i += 1;
                w = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(w);
            }
            "--h" => {
                i += 1;
                h = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(h);
            }
            "--max-frames" => {
                i += 1;
                max_frames = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(0);
            }
            "--skip" => {
                i += 1;
                skip = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(skip);
            }
            "--proc-h" => {
                i += 1;
                proc_h = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(proc_h);
            }
            "--offset" => {
                i += 1;
                offset_ms = args
                    .get(i)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(offset_ms);
            }
            "--offset-frames" => {
                i += 1;
                offset_frames = args
                    .get(i)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(offset_frames);
            }
            "--dump-quats" => {
                i += 1;
                dump_quats = Some(args.get(i).cloned().unwrap_or_default());
            }
            "--dump-frames" => {
                i += 1;
                dump_frames = Some(args.get(i).cloned().unwrap_or_default());
            }
            "--no-autosync" => {
                no_autosync = true;
            }
            "--autosync" => {
                no_autosync = false;
            }
            _ if orv.is_empty() => orv = args[i].clone(),
            _ => {}
        }
        i += 1;
    }
    if orv.is_empty() {
        eprintln!("usage: stabvid <path.trr> --out <mp4> [--seconds N] [--w N --h N] [--max-frames N] [--skip N] [--proc-h N] [--offset MS] [--dump-quats PATH] [--dump-frames DIR] [--autosync]");
        std::process::exit(2);
    }

    let _engine = QmlEngine::new();
    let mut settings = tracereel::settings::Settings::load_orv(&orv);
    if offset_ms != 0.0 {
        settings.gyro.offset_ms = offset_ms;
    }
    if offset_frames != 0 {
        settings.gyro.offset_frames = offset_frames;
    }
    match tracereel::gui::stabilize_video(
        &settings,
        &out,
        seconds,
        w,
        h,
        max_frames,
        skip,
        proc_h,
        !no_autosync,
        settings.gyro.offset_ms,
        dump_quats.as_deref(),
        dump_frames.as_deref(),
    ) {
        Ok(path) => {
            let sz = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            println!("stabilized video -> {path} ({sz} bytes)");
        }
        Err(e) => {
            eprintln!("stabilize failed: {e}");
            std::process::exit(1);
        }
    }
}
