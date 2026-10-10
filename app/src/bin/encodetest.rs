//! encodetest — CLI that encodes a source video through the ffmpeg encoder pipeline,
//! validating the fragmented-mp4 command and audio copy (M6 acceptance D).
//!
//! Usage: encodetest <src_video> --out <mp4> [--max-frames N]

use tracereel::encode::{build_command, FfmpegEncoder};
use tracereel::probe::probe_video;
use tracereel::settings::EncodeSettings;

fn main() {
    // MSVC/Qt 断言默认弹模态对话框，自动化跑会一直卡住等人点「忽略」；
    // 这里统一改成打到 stderr（与 hudcheck / gui.rs 的断言静默钩子一致）。
    tracereel::qtwrap::asserts_silent_continue();

    let args: Vec<String> = std::env::args().collect();
    let mut src = String::new();
    let mut out = "encodetest_out.mp4".to_string();
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
            _ if src.is_empty() => src = args[i].clone(),
            _ => {}
        }
        i += 1;
    }
    if src.is_empty() {
        eprintln!("usage: encodetest <src_video> --out <mp4> [--max-frames N]");
        std::process::exit(2);
    }

    let info = match probe_video(&src) {
        Some(x) => x,
        None => {
            eprintln!("probe failed");
            std::process::exit(1);
        }
    };
    println!(
        "src: {}x{} @ {:.2}fps ({}ms)",
        info.width, info.height, info.fps, info.duration_ms
    );

    // Use the source resolution as output.
    let (w, h) = (info.width, info.height);
    let enc = EncodeSettings {
        backend: "software".into(),
        format: "h264".into(),
        rate_control: "crf".into(),
        quality: 21.0,
        bitrate: "20M".into(),
        preset: "veryfast".into(),
        speed: "1".into(),
        audio_codec: "copy".into(),
        audio_bitrate: "128k".into(),
        clip_start_ms: 0.0,
        clip_end_ms: 0.0,
        output_file_path: out.clone(),
        resolution: "source".into(),
        frame_rate: "source".into(),
        map_only: false,
        include_map: true,
        include_video: true,
        include_hud: true,
        seg_enabled: false,
        start_seg: 0,
        end_seg: 0,
        all_seg_enabled: false,
    };

    println!(
        "command: {:?}",
        build_command(
            &out,
            &src,
            &[],
            w,
            h,
            info.fps,
            info.duration_ms,
            None,
            &enc
        )
    );

    let mut enc = FfmpegEncoder::open(&out, &src, &[], w, h, info.fps, info.duration_ms, &enc)
        .unwrap_or_else(|e| {
            eprintln!("open failed: {e}");
            std::process::exit(1);
        });
    let mut soft = tracereel::video::SoftDecoder::open(&src, w, h).unwrap_or_else(|e| {
        eprintln!("decoder open failed: {e}");
        std::process::exit(1);
    });

    let frame_dt = 1000.0 / info.fps;
    let mut count = 0usize;
    let cap = if max_frames > 0 {
        max_frames
    } else {
        (info.duration_ms / frame_dt).round() as usize
    };
    let t0 = std::time::Instant::now();
    let mut t_ms = 0.0;
    while count < cap {
        let mut rgba = vec![0u8; w * h * 4];
        if soft.decode_at(t_ms, &mut rgba).is_err() {
            break;
        }
        // RGBA -> RGB24 (drop alpha)
        let mut rgb = Vec::with_capacity(w * h * 3);
        for px in rgba.chunks_exact(4) {
            rgb.extend_from_slice(&px[..3]);
        }
        enc.write_frame(&rgb).unwrap_or_else(|e| {
            eprintln!("write failed: {e}");
            std::process::exit(1);
        });
        count += 1;
        t_ms += frame_dt;
    }
    let dur = t0.elapsed();
    println!(
        "encoded {} frames in {:.1}s ({:.0} fps)",
        count,
        dur.as_secs_f64(),
        count as f64 / dur.as_secs_f64()
    );

    match enc.close() {
        Ok(out) => {
            println!("encode finished -> {out}");
            let sz = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
            println!("output size: {} bytes (playable if > 0)", sz);
        }
        Err(e) => {
            eprintln!("close failed: {e}");
            std::process::exit(1);
        }
    }
}
