//! dumpargs — 打印某个 .orv 在导出时会用的 ffmpeg 命令行（不真正编码）。
//!
//! Usage: dumpargs <path.trr>

use tracereel::encode::build_command;
use tracereel::settings::Settings;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let Some(orv) = args.get(1) else {
        eprintln!("usage: dumpargs <path.trr>");
        std::process::exit(2);
    };
    let s = Settings::load_orv(orv);
    let mut s = s;
    s.resolve_encode_clip_ms();
    let enc = s.encode.clone();
    println!("backend  = {}", enc.backend);
    println!("format   = {}", enc.format);
    println!("rateCtl  = {}", enc.rate_control);
    println!("quality  = {}", enc.quality);
    println!("bitrate  = {}", enc.bitrate);
    println!("preset   = {}", enc.preset);
    println!("audio    = {}", enc.audio_codec);
    // 与导出对话框显示的是同一行文字。
    println!("界面显示 = {}", tracereel::encode::encoder_status_text(&enc));
    println!("--- command ---");
    let cmd = build_command(
        &enc.output_file_path,
        &s.video.video_path,
        &[],
        1920,
        1080,
        30.0,
        120_000.0,
        None,
        &enc,
    );
    for a in &cmd {
        print!("{a} ");
    }
    println!();
}