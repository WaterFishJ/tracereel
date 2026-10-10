//! gyrotest — CLI that loads the Gyroflow stabilizer for a video+preset, runs
//! autosync, recomputes the stabilization, decodes + stabilizes a run of frames,
//! validates them (not all-black) and writes the first as a PNG. This validates
//! acceptance A (stable-frame correctness) + the in-player stabilize loop headlessly.
//!
//! Usage: gyrotest <video> --preset <preset> --at <ms> [--out <png>] [--proc-h <n>] [--n-frames N]

use std::path::Path;

use tracereel::gyro;

/// Quick "is this a real frame" check: the pixel luminance must have variance.
fn has_variance(rgba: &[u8]) -> bool {
    let mut mean = 0.0f64;
    let n = rgba.len() / 4;
    if n == 0 {
        return false;
    }
    for px in rgba.chunks_exact(4) {
        mean += (px[0] as f64 + px[1] as f64 + px[2] as f64) / 3.0;
    }
    mean /= n as f64;
    let mut var = 0.0f64;
    for px in rgba.chunks_exact(4) {
        let l = (px[0] as f64 + px[1] as f64 + px[2] as f64) / 3.0;
        let d = l - mean;
        var += d * d;
    }
    var / n as f64 > 50.0
}

fn main() {
    // MSVC/Qt 断言默认弹模态对话框，自动化跑会一直卡住等人点「忽略」；
    // 这里统一改成打到 stderr（与 hudcheck / gui.rs 的断言静默钩子一致）。
    tracereel::qtwrap::asserts_silent_continue();

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args: Vec<String> = std::env::args().collect();
    let mut video = String::new();
    let mut preset = String::new();
    // 镜头标定（Gyroflow 镜头库 key 或 `.json` 路径）：覆盖预设里的 calibration_data。
    let mut lens_profile = String::new();
    let mut at_ms = 5000.0f64;
    let mut out = "stab_frame.png".to_string();
    let mut proc_h = 0usize;
    let mut n_frames = 1usize;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--preset" => {
                i += 1;
                preset = args.get(i).cloned().unwrap_or_default();
            }
            "--lens" => {
                i += 1;
                lens_profile = args.get(i).cloned().unwrap_or_default();
            }
            "--at" => {
                i += 1;
                at_ms = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(at_ms);
            }
            "--out" => {
                i += 1;
                out = args.get(i).cloned().unwrap_or_default();
            }
            "--proc-h" => {
                i += 1;
                proc_h = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(0);
            }
            "--n-frames" => {
                i += 1;
                n_frames = args.get(i).and_then(|s| s.parse().ok()).unwrap_or(1);
            }
            _ if video.is_empty() => video = args[i].clone(),
            _ => {}
        }
        i += 1;
    }
    if video.is_empty() {
        eprintln!("usage: gyrotest <video> --preset <preset> [--lens <库key|.json路径>] --at <ms> [--out <png>] [--proc-h <n>] [--n-frames N]");
        std::process::exit(2);
    }

    let info = match tracereel::probe::probe_video(&video) {
        Some(x) => x,
        None => {
            eprintln!("probe failed for {video}");
            std::process::exit(1);
        }
    };
    println!(
        "video: {}x{} @ {:.3}fps, {}ms, has_motion={}",
        info.width, info.height, info.fps, info.duration_ms, info.has_motion_data
    );

    let (src_w, src_h) = if proc_h > 0 && info.height > proc_h {
        let nh = proc_h - (proc_h % 2);
        let nw = ((info.width as f64 * nh as f64 / info.height as f64).round() as usize).max(2);
        (nw - nw % 2, nh)
    } else {
        (info.width, info.height)
    };
    println!("processing resolution: {src_w}x{src_h}");

    // 稳定化参数来自 GyroSettings：CLI 里用默认值 + 从 --preset 导入 stabilization 段
    // （与 GUI 选中预设的行为一致），再用 --lens 覆盖标定。
    let mut gyro_settings = tracereel::settings::GyroSettings::default();
    gyro_settings.preset_file_path = preset.clone();
    gyro_settings.lens_profile = lens_profile.clone();
    if !preset.is_empty() {
        if let Ok(text) = std::fs::read_to_string(&preset) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                gyro_settings.apply_preset_json(&v);
                println!("[gyrotest] stabilization imported from {preset}");
            }
        }
    }

    let t0 = std::time::Instant::now();
    let stab = gyro::load_stabilizer(
        &video,
        &gyro_settings,
        src_w,
        src_h,
        info.fps,
        info.duration_ms,
        info.frame_count,
        true,
        0.0,
        // Print the loader's real progress so the CLI shows the same signal as the GUI bar.
        &|p: gyro::LoadProgress| {
            if (0.0..=1.0).contains(&p.frac) {
                print!("\r[gyrotest] {} {:5.1}%   ", p.phase.label(), p.frac * 100.0);
                use std::io::Write;
                let _ = std::io::stdout().flush();
            }
        },
    )
    .unwrap_or_else(|e| {
        eprintln!("stabilizer load failed: {e}");
        std::process::exit(1);
    });
    println!(
        "stabilizer loaded in {:.1}s (autosync_done={})",
        t0.elapsed().as_secs_f64(),
        stab.autosync_done
    );

    let mut soft = tracereel::video::SoftDecoder::open(&video, src_w, src_h).unwrap_or_else(|e| {
        eprintln!("decoder open failed: {e}");
        std::process::exit(1);
    });
    let dt = 1000.0 / info.fps;
    let mut ok_frames = 0usize;
    let mut black_frames = 0usize;
    let mut first_out: Option<Vec<u8>> = None;
    let mut total_ms = 0.0f64;

    for k in 0..n_frames {
        let t_ms = at_ms + k as f64 * dt;
        let mut rgba = vec![0u8; src_w * src_h * 4];
        if soft.decode_at(t_ms, &mut rgba).is_err() {
            continue;
        }
        let fi = ((t_ms / 1000.0) * info.fps).round() as usize;
        let mut out_rgba = vec![0u8; src_w * src_h * 4];
        let now = std::time::Instant::now();
        match stab.process_frame(
            t_ms,
            fi,
            &mut rgba,
            src_w,
            src_h,
            &mut out_rgba,
            src_w,
            src_h,
        ) {
            Ok(_) => {
                total_ms += now.elapsed().as_millis() as f64;
                if has_variance(&out_rgba) {
                    ok_frames += 1;
                } else {
                    black_frames += 1;
                }
                if first_out.is_none() {
                    first_out = Some(out_rgba.clone());
                }
            }
            Err(e) => {
                eprintln!("frame {k} process failed: {e}");
                black_frames += 1;
            }
        }
    }
    println!(
        "stabilized {ok_frames}/{n_frames} frames (black={black_frames}), avg {:.1}ms/frame",
        if ok_frames > 0 {
            total_ms / ok_frames as f64
        } else {
            0.0
        }
    );

    if let Some(out_rgba) = first_out {
        let path = Path::new(&out);
        let img: image::RgbaImage =
            image::RgbaImage::from_raw(src_w as u32, src_h as u32, out_rgba).unwrap();
        img.save(path).unwrap_or_else(|e| {
            eprintln!("save failed: {e}");
            std::process::exit(1);
        });
        println!("wrote {out} ({src_w}x{src_h})");
    }
    if black_frames > 0 || ok_frames == 0 {
        eprintln!("FAIL: some/all frames were black");
        std::process::exit(1);
    }
}
