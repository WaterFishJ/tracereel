//! guirender — headless composite-render test. Renders the map+route+video composite
//! for a given time to a PNG using the SAME QPainter logic as the GUI window
//! (`tracereel::gui::render_composite_to_png`), so the rendering can be verified
//! without a visible window.
//!
//! Usage: guirender <path.trr> --at <ms> --out <png> [--w N --h N]

use qmetaobject::QmlEngine;

fn main() {
    // MSVC/Qt 断言默认弹模态对话框，自动化跑会一直卡住等人点「忽略」；
    // 这里统一改成打到 stderr（与 hudcheck / gui.rs 的断言静默钩子一致）。
    tracereel::qtwrap::asserts_silent_continue();

    let args: Vec<String> = std::env::args().collect();
    let mut orv = String::new();
    let mut at = 0.0f64;
    let mut out = "composite.png".to_string();
    let mut w = 1280i32;
    let mut h = 720i32;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--at" => {
                i += 1;
                at = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(at);
            }
            "--out" => {
                i += 1;
                out = args.get(i).cloned().unwrap_or_default();
            }
            "--w" => {
                i += 1;
                w = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(w);
            }
            "--h" => {
                i += 1;
                h = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(h);
            }
            _ if orv.is_empty() => orv = args[i].clone(),
            _ => {}
        }
        i += 1;
    }
    if orv.is_empty() {
        eprintln!("usage: guirender <path.trr> --at <ms> --out <png>");
        std::process::exit(2);
    }

    // Create a QGuiApplication (via QmlEngine) so QPainter works.
    let _engine = QmlEngine::new();

    // --minimal: isolate offscreen QPainter-on-QImage (fill + polylines + save).
    if args.iter().any(|a| a == "--minimal") {
        let w = 640;
        let h = 480;
        let mut img = tracereel::qtwrap::qimage_new_argb32(w, h);
        let mut p = tracereel::qtwrap::qpainter_on_image(&mut img);
        p.fill_rect(
            qmetaobject::qttypes::QRectF {
                x: 0.0,
                y: 0.0,
                width: w as f64,
                height: h as f64,
            },
            qmetaobject::qttypes::QBrush::from_color(qmetaobject::qttypes::QColor::from_rgba(
                0, 0, 0, 255,
            )),
        );
        p.draw_text(
            qmetaobject::qttypes::QPointF { x: 50.0, y: 60.0 },
            qmetaobject::qttypes::QString::from("Hello HUD 测测试"),
        );
        drop(p);
        let ok = tracereel::qtwrap::qimage_save(&img, &out);
        println!("ARGB32 text render + save: {ok}");
        return;
    }

    let settings = tracereel::settings::Settings::load_orv(&orv);
    let r = tracereel::gui::render_composite_to_png(&settings, at, w, h, &out);
    match r {
        Ok(path) => println!("wrote {path}"),
        Err(e) => {
            eprintln!("render failed: {e}");
            std::process::exit(1);
        }
    }
}
