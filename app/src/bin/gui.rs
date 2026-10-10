//! tracereel-gui — launches the TraceReel GUI (QML + qmetaobject + Qt6).
//!
//! Usage: tracereel-gui [path.trr]
//! Existing .orv projects are also supported. No argument starts an empty project.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use qmetaobject::*;
use std::ffi::CString;

fn main() {
    // Keep the same customizable controls when launched without run.ps1.
    tracereel::qtwrap::configure_gui_style();
    let _ =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    // Qt 的 debug 运行库在 gui.qml 实例化期间会触发 `Q_ASSERT("bit < 2 * props")`
    // （qqmlengine.cpp:1441）。默认它会弹一个模态框，点「忽略」才继续 —— 无人值守时
    // 就是死等。这里装一个 CRT 报告钩子，等价于**自动一直点忽略**：不弹框、进程继续，
    // 断言原文仍然打到 stderr（见 app/src/asserts_silent.h）。
    tracereel::qtwrap::asserts_silent_continue();

    // Resolve a settings file: only when the user passes one on the command line.
    // No argument = clean start (empty preview; load a project via 文件 → 加载设置).
    let orv = std::env::args().nth(1).unwrap_or_default();

    // QML types must be registered before the first QQmlEngine is constructed.
    // Qt's release runtime otherwise reports qqmlengine.cpp:1441 and only
    // continues when the assertion dialog is ignored.
    let uri = CString::new("TraceReel").unwrap();
    let name = CString::new("TraceReelItem").unwrap();
    qml_register_type::<tracereel::gui::TraceReelItem>(&uri, 1, 0, &name);
    let ocr_name = CString::new("ReceiptOcr").unwrap();
    qml_register_type::<tracereel::receipt_ocr::ReceiptOcr>(&uri, 1, 0, &ocr_name);
    let proxy_name = CString::new("SettingsProxy").unwrap();
    qml_register_type::<tracereel::settings_proxy::SettingsProxy>(&uri, 1, 0, &proxy_name);
    // 主题单例（对齐原版 Gyroflow 的 style* 调色板）：所有 QML 文件都能直接 Theme.xxx。
    let theme_name = CString::new("Theme").unwrap();
    qml_register_singleton_type::<tracereel::theme::Theme>(&uri, 1, 0, &theme_name);

    let mut engine = QmlEngine::new();
    tracereel::qtwrap::configure_application_icon();

    // Expose the settings path as a context property the QML can read.
    engine.set_property(QString::from("orvPath"), QString::from(orv.as_str()).into());

    // Deployment ships the application QML next to the executable. In the
    // development tree, QML edits still do not require rebuilding.
    let bundled_qml = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|dir| dir.join("qml/app/gui.qml")));
    let qml_file = match bundled_qml.filter(|p| p.is_file()) {
        Some(path) => path,
        None => {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bin/gui.qml")
        }
    };
    if !qml_file.exists() {
        eprintln!("gui.qml not found at {}", qml_file.display());
        std::process::exit(1);
    }
    let qml_path = qml_file.to_string_lossy().into_owned();
    engine.load_file(QString::from(qml_path.as_str()));

    engine.exec();
}
