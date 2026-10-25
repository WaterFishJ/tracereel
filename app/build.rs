use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Locate the Windows SDK resource compiler without adding a new crate.
fn find_rc() -> PathBuf {
    if let Some(rc) = env::var_os("RC") {
        return PathBuf::from(rc);
    }
    if Command::new("rc.exe").arg("/?").output().is_ok() {
        return PathBuf::from("rc.exe");
    }
    if let Some(program_files) = env::var_os("ProgramFiles(x86)") {
        let bin = PathBuf::from(program_files).join("Windows Kits/10/bin");
        let mut versions: Vec<_> = std::fs::read_dir(bin)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .collect();
        versions.sort();
        if let Some(compiler) = versions
            .into_iter()
            .rev()
            .map(|version| version.join("x64/rc.exe"))
            .find(|path| path.is_file())
        {
            return compiler;
        }
    }
    panic!("Windows SDK rc.exe not found; set RC to its path");
}

fn run_rc(compiler: &Path, manifest_dir: &Path, input: &Path, output: &Path) {
    let status = Command::new(compiler)
        .current_dir(manifest_dir)
        .arg("/nologo")
        .arg("/fo")
        .arg(output)
        .arg(input)
        .status()
        .expect("Could not launch the Windows resource compiler");
    assert!(status.success(), "Windows resource compilation failed");
}

fn target_is_windows() -> bool {
    env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
}

fn compile_windows_icon() {
    if !target_is_windows() {
        return;
    }
    println!("cargo:rerun-if-env-changed=RC");
    println!("cargo:rerun-if-changed=tracereel.rc");
    println!("cargo:rerun-if-changed=../assets/branding/tracereel.ico");
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("tracereel.res");
    run_rc(
        &find_rc(),
        &manifest,
        Path::new("tracereel.rc"),
        &output,
    );
    println!(
        "cargo:rustc-link-arg-bin=tracereel-gui={}",
        output.display()
    );
}

/// Embed the application manifest (asInvoker + per-monitor DPI + long paths) and
/// VERSIONINFO into the GUI executable. The MSIX package gets its version from
/// AppxManifest.xml, but Explorer and the Store certification tooling read these.
fn compile_windows_metadata() {
    if !target_is_windows() {
        return;
    }
    println!("cargo:rerun-if-env-changed=RC");
    println!("cargo:rerun-if-env-changed=CARGO_PKG_VERSION");
    println!("cargo:rerun-if-changed=tracereel.manifest");
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let version = env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let numbers: Vec<u16> = version
        .split(|c: char| !c.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .take(3)
        .map(|part| part.parse().unwrap_or(0))
        .collect();
    let mut parts = [0u16, 0, 0];
    for (index, number) in numbers.iter().take(3).enumerate() {
        parts[index] = *number;
    }
    let numeric = format!("{}.{}.{}", parts[0], parts[1], parts[2]);
    let file_version = format!("{}, {}, {}, 0", parts[0], parts[1], parts[2]);
    let manifest_path = manifest.join("tracereel.manifest");
    // The resource compiler treats backslashes inside quoted strings as escape
    // sequences (\t, \a, ...), so hand it a forward-slash path instead.
    let manifest_rc_path = manifest_path.display().to_string().replace('\\', "/");
    let rc_source = out.join("tracereel-resources.rc");
    let rc_output = out.join("tracereel-resources.res");
    let content = format!(
        "1 24 \"{manifest}\"\r\n\r\n1 VERSIONINFO\r\nFILEVERSION {file_version}\r\nPRODUCTVERSION {file_version}\r\nFILEOS 0x00040004L\r\nFILETYPE 0x00000001L\r\nBEGIN\r\n  BLOCK \"StringFileInfo\"\r\n  BEGIN\r\n    BLOCK \"040904B0\"\r\n    BEGIN\r\n      VALUE \"FileDescription\", \"TraceReel\"\r\n      VALUE \"FileVersion\", \"{numeric}\"\r\n      VALUE \"InternalName\", \"TraceReel\"\r\n      VALUE \"OriginalFilename\", \"TraceReel.exe\"\r\n      VALUE \"ProductName\", \"TraceReel\"\r\n      VALUE \"ProductVersion\", \"{numeric}\"\r\n      VALUE \"LegalCopyright\", \"TraceReel contributors, GPL-3.0-or-later\"\r\n    END\r\n  END\r\n  BLOCK \"VarFileInfo\"\r\n  BEGIN\r\n    VALUE \"Translation\", 0x409, 0x04B0\r\n  END\r\nEND\r\n",
        manifest = manifest_rc_path
    );
    std::fs::write(&rc_source, content).expect("Could not write the generated resource script");
    run_rc(&find_rc(), &manifest, &rc_source, &rc_output);
    println!(
        "cargo:rustc-link-arg-bin=tracereel-gui={}",
        rc_output.display()
    );
}

fn main() {
    compile_windows_icon();
    compile_windows_metadata();
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
    println!("cargo:rerun-if-changed=src/user_preferences.h");
    println!("cargo:rerun-if-changed=src/gui.rs");
    println!("cargo:rerun-if-changed=src/hud.rs");
    println!("cargo:rerun-if-changed=src/hud_painter.h");
    println!("cargo:rerun-if-changed=src/map_mask.h");

    let mut c = cpp_build::Config::new();
    c.include(&qt_include);
    c.include("src");
    c.flag("/permissive-");
    c.flag("/utf-8");
    c.flag("/std:c++17");
    c.flag("/Zc:__cplusplus");
    c.build("src/lib.rs");
}
