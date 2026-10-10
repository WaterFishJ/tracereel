//! encvideos — encode the composited (+ optionally Gyroflow-stabilized) video to a
//! fragmented mp4 via `tracereel::gui::encode_composite`. Validates acceptance D.
//!
//! Usage: encvideos <path.trr> --out <mp4> [--max-frames N]

use qmetaobject::QmlEngine;

fn main() {
    // MSVC/Qt 断言默认弹模态对话框，自动化跑会一直卡住等人点「忽略」；
    // 这里统一改成打到 stderr（与 hudcheck / gui.rs 的断言静默钩子一致）。
    tracereel::qtwrap::asserts_silent_continue();

    let args: Vec<String> = std::env::args().collect();
    let mut orv = String::new();
    let mut out = "out_enc.mp4".to_string();
    let mut max_frames = 0usize;
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
            _ if orv.is_empty() => orv = args[i].clone(),
            _ => {}
        }
        i += 1;
    }
    if orv.is_empty() {
        eprintln!("usage: encvideos <path.trr> --out <mp4> [--max-frames N]");
        std::process::exit(2);
    }

    let _engine = QmlEngine::new();
    let settings = tracereel::settings::Settings::load_orv(&orv);
    match tracereel::gui::encode_composite(&settings, &out, max_frames) {
        Ok(path) => {
            let sz = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            println!("encoded -> {path} ({sz} bytes)");
        }
        Err(e) => {
            eprintln!("encode failed: {e}");
            std::process::exit(1);
        }
    }
}
