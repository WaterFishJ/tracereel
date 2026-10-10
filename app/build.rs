use std::env;
use std::path::PathBuf;
use std::process::Command;

fn compile_windows_icon() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    println!("cargo:rerun-if-env-changed=RC");
    println!("cargo:rerun-if-changed=tracereel.rc");
    println!("cargo:rerun-if-changed=../assets/branding/tracereel.ico");
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("tracereel.res");
    let mut compiler = env::var_os("RC").map(PathBuf::from);
    if compiler.is_none() && Command::new("rc.exe").arg("/?").output().is_ok() {
        compiler = Some(PathBuf::from("rc.exe"));
    }
    // Cargo's MSVC compiler discovery does not put the SDK resource compiler
    // on PATH. Find it in the installed Windows SDK without a new crate.
    if compiler.is_none() {
        if let Some(program_files) = env::var_os("ProgramFiles(x86)") {
            let bin = PathBuf::from(program_files).join("Windows Kits/10/bin");
            let mut versions: Vec<_> = std::fs::read_dir(bin)
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .collect();
            versions.sort();
            compiler = versions
                .into_iter()
                .rev()
                .map(|version| version.join("x64/rc.exe"))
                .find(|path| path.is_file());
        }
    }
    let compiler = compiler.expect("Windows SDK rc.exe not found; set RC to its path");
    let status = Command::new(compiler)
        .current_dir(&manifest)
        .arg("/nologo")
        .arg("/fo")
        .arg(&output)
        .arg("tracereel.rc")
        .status()
        .expect("Could not launch the Windows resource compiler");
    assert!(status.success(), "Could not compile the TraceReel EXE icon");
    println!(
        "cargo:rustc-link-arg-bin=tracereel-gui={}",
        output.display()
    );
}

fn main() {
    compile_windows_icon();
    for asset in [
        "tracereel-icon.png",
        "tracereel-mark.png",
        "tracereel-white.png",
    ] {
        println!("cargo:rerun-if-changed=../assets/branding/{asset}");
    }
    // rust-cpp requires cpp_build to compile the `cpp!` blocks in this crate.
    // Call it ONCE on the crate root: it recursively processes `mod` files (lib.rs
    // declares `pub mod qtwrap;`), so qtwrap.rs's cpp! blocks are compiled too.
    //
    // Qt headers must be on the C++ include path, and MSVC needs Qt's required flags:
    // /permissive-, /std:c++17 and /Zc:__cplusplus (matching how qttypes builds itself).
    let qt_include = env::var("QT_INCLUDE_PATH")
        .unwrap_or_else(|_| "C:\\Qt\\6.8.2\\msvc2022_64\\include".to_string());
    println!("cargo:rerun-if-env-changed=QT_INCLUDE_PATH");
    // Re-run cpp_build whenever a source file with cpp! blocks changes, so the
    // generated rust-cpp metadata stays in sync.
    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=src/qtwrap.rs");
    println!("cargo:rerun-if-changed=src/gui.rs");
    println!("cargo:rerun-if-changed=src/hud.rs");
    println!("cargo:rerun-if-changed=src/hud_painter.h");

    let mut c = cpp_build::Config::new();
    c.include(&qt_include);
    c.include("src");
    c.flag("/permissive-");
    c.flag("/utf-8");
    c.flag("/std:c++17");
    c.flag("/Zc:__cplusplus");
    c.build("src/lib.rs");
}
