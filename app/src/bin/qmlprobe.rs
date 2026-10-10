//! qmlprobe — drive the real `gui.qml` headlessly to reproduce interaction bugs.
//!
//! It loads the same QML the GUI loads, then calls its own top-level JS functions
//! (`handleDrop`, `debugRemoveRow`, `debugState`, ...) through the QML engine's
//! meta-object, printing the resulting list/settings state after every step.
//! This is how "drop a file, delete it, drop another → three files" gets pinned
//! down without clicking.
//!
//! Usage: qmlprobe [script.json]
//!   without an argument it runs the built-in reproduction script.

use std::ffi::CString;

use qmetaobject::*;

fn invoke(engine: &mut QmlEngine, name: &str, args: Vec<QVariant>) -> String {
    let ret = engine.invoke_method(QByteArray::from(name), &args);
    QString::from_qvariant(ret).unwrap_or_default().to_string()
}

fn drop_files(engine: &mut QmlEngine, files: &[&str], label: &str) -> String {
    let list = QStringList::from(files.iter().map(|f| QString::from(*f)).collect::<Vec<_>>());
    let out = invoke(engine, "handleDrop", vec![list.to_qvariant()]);
    let state = invoke(engine, "debugState", vec![]);
    println!("[drop {label}] ret=[{out}]");
    println!("           {state}");
    state
}

fn remove_row(engine: &mut QmlEngine, which: &str, i: i32, label: &str) -> String {
    let _ = invoke(
        engine,
        "debugRemoveRow",
        vec![QString::from(which).to_qvariant(), i.to_qvariant()],
    );
    let state = invoke(engine, "debugState", vec![]);
    println!("[remove {label}]");
    println!("           {state}");
    state
}

fn append_row(engine: &mut QmlEngine, which: &str) -> String {
    let _ = invoke(
        engine,
        "debugAppendRow",
        vec![QString::from(which).to_qvariant()],
    );
    let state = invoke(engine, "debugState", vec![]);
    println!("[append {which}]");
    println!("           {state}");
    state
}

fn set_row(engine: &mut QmlEngine, which: &str, i: i32, path: &str) -> String {
    let _ = invoke(
        engine,
        "debugSetRow",
        vec![
            QString::from(which).to_qvariant(),
            i.to_qvariant(),
            QString::from(path).to_qvariant(),
        ],
    );
    let state = invoke(engine, "debugState", vec![]);
    println!("[set {which} #{i}]");
    println!("           {state}");
    state
}

/// 断言某个列表的行数：失败就报错退出，让这个 probe 能当回归测试跑。
fn expect_rows(state: &str, needle: &str, label: &str, failures: &mut usize) {
    if state.contains(needle) {
        println!("  ok   {label}: {needle}");
    } else {
        println!("  FAIL {label}: 期望 {needle}，实际 {state}");
        *failures += 1;
    }
}

fn main() {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).try_init();
    tracereel::qtwrap::configure_gui_style();
    // MSVC/Qt 断言默认弹模态对话框，自动化跑会一直卡住等人点「忽略」；
    // 这里统一改成打到 stderr（与 hudcheck / gui.rs 的断言静默钩子一致）。
    tracereel::qtwrap::test_runtime_errors_to_stderr();
    // 二分定位期间：断言静默终止，永不弹框。
    tracereel::qtwrap::asserts_silent_continue();

    // Same registration order as the real launcher.
    let uri = CString::new("TraceReel").unwrap();
    qml_register_type::<tracereel::receipt_ocr::ReceiptOcr>(
        &uri, 1, 0, &CString::new("ReceiptOcr").unwrap(),
    );
    qml_register_type::<tracereel::gui::TraceReelItem>(
        &uri,
        1,
        0,
        &CString::new("TraceReelItem").unwrap(),
    );
    qml_register_type::<tracereel::settings_proxy::SettingsProxy>(
        &uri,
        1,
        0,
        &CString::new("SettingsProxy").unwrap(),
    );
    qml_register_singleton_type::<tracereel::theme::Theme>(
        &uri,
        1,
        0,
        &CString::new("Theme").unwrap(),
    );

    let mut engine = QmlEngine::new();
    engine.set_property(QString::from("orvPath"), QString::from("").into());
    engine.set_property(QString::from("ocrTestImage"), QString::from(std::env::var("OCR_TEST_IMAGE").unwrap_or_default()).into());

    let qml_file = match std::env::var("QMLPROBE_FILE") {
        Ok(p) => std::path::PathBuf::from(p),
        Err(_) => std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/bin/gui.qml"),
    };
    println!("loading {}", qml_file.display());
    eprintln!("[marker] before load_file");
    engine.load_file(QString::from(qml_file.to_string_lossy().as_ref()));
    eprintln!("[marker] after load_file");
    // Allow a supplied QML harness to drive asynchronous export/restart checks.
    if std::env::var_os("QMLPROBE_RUN_LOOP").is_some() {
        if let Ok(ms) = std::env::var("QMLPROBE_TIMEOUT_MS") {
            if let Ok(ms) = ms.parse::<u64>() {
                qmetaobject::single_shot(std::time::Duration::from_millis(ms), || {
                    qmetaobject::qtcore::core_application::QCoreApplication::quit();
                });
            }
        }
        engine.exec();
        println!("[loop final] {}", invoke(&mut engine, "debugState", vec![]));
        std::process::exit(0);
    }
    if std::env::var_os("QMLPROBE_LOAD_ONLY").is_some() {
        eprintln!("[marker] load-only mode, exiting");
        std::process::exit(0);
    }

    println!("[start]     {}", invoke(&mut engine, "debugState", vec![]));
    eprintln!("[marker] after first invoke_method");
    let mut failures = 0usize;
    let tmp = std::env::temp_dir().to_string_lossy().replace('\\', "/");

    // ---- 复现：拖入第二个文件 → 删除它 → 再拖入一个（期望 2 行）----
    let a = format!("{tmp}/probe_a.mp4");
    let b = format!("{tmp}/probe_b.mp4");
    let c = format!("{tmp}/probe_c.mp4");
    let s = drop_files(&mut engine, &[a.as_str()], "1st video");
    expect_rows(&s, "视频 1 行", "视频: 拖入第 1 个", &mut failures);
    let s = drop_files(&mut engine, &[b.as_str()], "2nd video");
    expect_rows(&s, "视频 2 行", "视频: 拖入第 2 个", &mut failures);
    let s = remove_row(&mut engine, "video", 1, "2nd video row");
    expect_rows(&s, "视频 1 行", "视频: 删除第 2 行", &mut failures);
    let s = drop_files(&mut engine, &[c.as_str()], "3rd video");
    expect_rows(&s, "视频 2 行", "视频: 再拖入一个（不能变成 3 行）", &mut failures);

    println!();
    // ---- 轨迹页同样一遍 ----
    let m1 = format!("{tmp}/probe_1.qrt");
    let m2 = format!("{tmp}/probe_2.qrt");
    let m3 = format!("{tmp}/probe_3.qrt");
    let s = drop_files(&mut engine, &[m1.as_str()], "1st map");
    expect_rows(&s, "轨迹图 1 行", "轨迹: 拖入第 1 个", &mut failures);
    let s = drop_files(&mut engine, &[m2.as_str()], "2nd map");
    expect_rows(&s, "轨迹图 2 行", "轨迹: 拖入第 2 个", &mut failures);
    let s = remove_row(&mut engine, "track", 1, "2nd map row");
    expect_rows(&s, "轨迹图 1 行", "轨迹: 删除第 2 行", &mut failures);
    let s = drop_files(&mut engine, &[m3.as_str()], "3rd map");
    expect_rows(&s, "轨迹图 2 行", "轨迹: 再拖入一个（不能变成 3 行）", &mut failures);

    println!();
    // ---- 空行（点过「添加」还没选文件）：拖入必须填空行，而不是再多一行 ----
    let d = format!("{tmp}/probe_d.mp4");
    let _ = append_row(&mut engine, "video");
    let _ = append_row(&mut engine, "video"); // 故意留两个空行
    let s = set_row(&mut engine, "video", 1, &d);
    expect_rows(&s, "视频 4 行", "视频: 已有 2 行 + 加两个空行 + 填一个", &mut failures);
    let s = remove_row(&mut engine, "video", 1, "row 1 (manual)");
    expect_rows(&s, "视频 3 行", "视频: 删掉刚填的行（剩下两个空行）", &mut failures);
    let s = drop_files(&mut engine, &[format!("{tmp}/probe_e.mp4").as_str()], "4th video");
    expect_rows(&s, "视频 2 行", "视频: 有残留空行时拖入（不能变成 3 行）", &mut failures);

    println!();
    println!("=== log ===");
    println!("{}", invoke(&mut engine, "logText", vec![]));
    println!();
    if failures > 0 {
        println!("PROBE FAILED: {failures} 处行数不符");
        std::process::exit(1);
    }
    println!("PROBE OK");

    // Qt's teardown can block on a worker thread; the probe has nothing left to do.
    std::process::exit(0);
}
