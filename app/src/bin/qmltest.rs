//! qmltest — minimal qmetaobject + Qt6 smoke test. Validates that the Rust crate
//! can link Qt6, create a QmlEngine and load QML. This is the M4 GUI feasibility gate.
//!
//! Usage: qmltest (needs Qt6 bin + QML plugins on PATH)

extern crate qmetaobject;

use qmetaobject::QmlEngine;

fn main() {
    // MSVC/Qt 断言默认弹模态对话框，自动化跑会一直卡住等人点「忽略」；
    // 这里统一改成打到 stderr（与 hudcheck / gui.rs 的断言静默钩子一致）。
    tracereel::qtwrap::asserts_silent_continue();

    // Force a link to QtCore/QtGui/QtQml transitively.
    let mut engine = QmlEngine::new();
    engine.load_data(
        r#"
        import QtQuick 2.15
        Rectangle { width: 200; height: 100; color: "steelblue" }
    "#
        .into(),
    );
    println!("qmltest: QmlEngine created and QML loaded OK (qmetaobject+Qt6 linked)");
}
