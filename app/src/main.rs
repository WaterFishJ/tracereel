//! tracereel — entry point.
//!
//! M1: a debug CLI that probes a video file and prints metadata. The Qt widget-based
//! GUI is layered on later (M4+). Running `tracereel <path>` prints the probe result.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: tracereel <video> [--motion] [--load-orv <file>]");
        std::process::exit(2);
    }

    // Simple flag scan.
    let mut video = String::new();
    let mut orv_path: Option<String> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--motion" => {}
            "--load-orv" => {
                i += 1;
                orv_path = args.get(i).cloned();
            }
            _ if video.is_empty() => video = args[i].clone(),
            _ => {}
        }
        i += 1;
    }

    if let Some(orv) = &orv_path {
        let s = tracereel::settings::Settings::load_orv(orv);
        println!(
            "loaded settings from {orv}: video={} map={}",
            s.video.video_path, s.map.image_file_path
        );
    }

    match tracereel::probe::probe_video(&video) {
        Some(info) => {
            println!("video: {video}");
            println!("  resolution: {}x{}", info.width, info.height);
            println!("  fps:        {:.3}", info.fps);
            println!(
                "  duration:   {:.0} ms ({:.1} s)",
                info.duration_ms,
                info.duration_ms / 1000.0
            );
            println!("  frames:     {}", info.frame_count);
            println!(
                "  motion:     {}",
                if info.has_motion_data { "yes" } else { "no" }
            );
            if let Some(id) = tracereel::probe::video_codec_id(&video) {
                println!("  codec_id:   {id:?}");
            }
        }
        None => {
            eprintln!("error: could not probe video: {video}");
            std::process::exit(1);
        }
    }
}
