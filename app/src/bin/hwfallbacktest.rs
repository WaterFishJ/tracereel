//! hwfallbacktest —— 端到端验证「硬件编码器不可用 → 提示原因 + 自动改用 CPU」。
//!
//! 真实场景（朋友的核显不支持 AV1）在本机无法复现，因为本机的 av1_qsv 是可用的。
//! 所以这里用同一套生产代码路径（build_command / FfmpegEncoder / classify_hw_failure）
//! 去跑一个**确定性不可用**的硬件编码器组合，验证：
//!
//!   1. 失败被判成「硬件编码器不可用」，且给用户的一句话原因里没有 ffmpeg 内部堆栈；
//!   2. 完整 stderr 被写进了输出视频目录旁的日志文件；
//!   3. 把后端换成软件编码后，同一个导出能成功产出文件（= 回退路径真实可用）。
//!
//! Usage: hwfallbacktest --out <dir>

use tracereel::encode::{
    build_command, classify_hw_failure, hw_failure_reason, FfmpegEncoder,
};
use tracereel::settings::EncodeSettings;

fn main() {
    tracereel::qtwrap::asserts_silent_continue();

    let args: Vec<String> = std::env::args().collect();
    let mut out_dir = std::env::temp_dir().join("ov_hwfallback");
    let mut i = 1;
    while i < args.len() {
        if args[i] == "--out" {
            i += 1;
            out_dir = std::path::PathBuf::from(args.get(i).cloned().unwrap_or_default());
        }
        i += 1;
    }
    let _ = std::fs::create_dir_all(&out_dir);
    // 日志写到"输出视频所在目录"——正是生产代码的行为。
    tracereel::encode_log::set_log_dir(&out_dir);

    let (w, h, fps) = (320usize, 240usize, 30.0f64);
    let mut failures = 0;

    // 先生成一段真实素材：所有场景共用，确保失败原因只可能是编码器本身
    // （而不是"输入文件不存在"这种无关错误）。
    let src = out_dir.join("src.mp4").to_string_lossy().into_owned();
    let gen = std::process::Command::new(tracereel::encode::find_ffmpeg())
        .args([
            "-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi",
            "-i", &format!("testsrc=size={w}x{h}:rate={fps}:duration=2"),
            "-c:v", "libx264", "-pix_fmt", "yuv420p", &src,
        ])
        .status();
    if !matches!(gen, Ok(s) if s.success()) {
        println!("✗ 无法生成测试素材，后续场景无意义");
        std::process::exit(1);
    }

    // ── 场景 1：硬件编码器打不开（模拟核显不支持 AV1）──────────────────────────
    println!("\n=== 场景 1：硬件编码器不可用（av1 + 不存在的硬件后端）===");
    let bad = EncodeSettings {
        backend: "qsv_bogus".into(), // 不存在的后端 → 编码器名 av1_qsv_bogus → 必然打不开
        format: "av1".into(),
        rate_control: "crf".into(),
        quality: 40.0,
        bitrate: "20M".into(),
        preset: "medium".into(),
        speed: "1".into(),
        audio_codec: "none".into(),
        audio_bitrate: "128k".into(),
        clip_start_ms: 0.0,
        clip_end_ms: 0.0,
        output_file_path: String::new(),
        resolution: "source".into(),
        frame_rate: "source".into(),
        map_only: false,
        include_map: false,
        include_video: true,
        include_hud: false,
        seg_enabled: false,
        start_seg: 0,
        end_seg: 0,
        all_seg_enabled: false,
    };
    let bad_out = out_dir.join("hw_fail.mp4").to_string_lossy().into_owned();
    let cmd = build_command(&bad_out, "dummy_src.mp4", &[], w, h, fps, 1000.0, None, &bad);
    println!("编码器名 = {}", cmd[cmd.iter().position(|a| a == "-c:v").unwrap() + 1]);

    // 真实的报错路径：实测即使编码器是坏的，只要 stdin 是管道且没有写入，ffmpeg 也不会
    // 在启动窗口（300ms）内退出 —— 所以结论出现在第一帧写入时（write_frame），
    // 这也正是用户截图里的路径（"写入帧失败: 管道已结束"）。
    let err = match FfmpegEncoder::open(&bad_out, &src, &[], w, h, fps, 2000.0, &bad) {
        Err(e) => {
            println!("open 阶段就失败的错误串：\n  {e}");
            Some(e)
        }
        Ok(mut e) => {
            println!("open 成功（符合预期：坏编码器要等第一帧写入才报错）");
            let frame = vec![128u8; w * h * 3];
            let mut first: Option<String> = None;
            // 坏编码器的报错可能要等几帧才出现在 stderr 里，多写几帧确保拿到。
            for _ in 0..30 {
                if let Err(w_err) = e.write_frame(&frame) {
                    first = Some(w_err);
                    break;
                }
            }
            let _ = e.abort();
            match &first {
                Some(x) => println!("write_frame 返回的错误串：\n  {x}"),
                None => println!("✗ 写了 30 帧都没报错 —— 这个场景没能复现硬件失败"),
            }
            first
        }
    };

    match err {
        None => {
            println!("✗ 未能复现失败 —— 判定逻辑没有机会被验证");
            failures += 1;
        }
        Some(e) => {
            // 必须被标记为"硬件不可用"，否则上层不会回退。
            match tracereel::encode::unmark_hw_failure(&e) {
                Some(reason) => {
                    println!("✓ 被判定为硬件编码器不可用，用户可见原因：\n  {reason}");
                    // 给用户的一句话不能包含 ffmpeg 的内部堆栈文本。
                    let noisy = [
                        "Error sending frames",
                        "Task finished",
                        "Nothing was written",
                        "Terminating thread",
                        "Function not implemented",
                        "os error",
                        "管道已结束",
                    ];
                    let leaked: Vec<&str> = noisy
                        .iter()
                        .copied()
                        .filter(|m| reason.contains(m))
                        .collect();
                    if leaked.is_empty() {
                        println!("✓ 用户可见原因里没有 ffmpeg 内部报错/管道错误文本");
                    } else {
                        println!("✗ 用户可见原因泄漏了内部报错: {leaked:?}");
                        failures += 1;
                    }
                }
                None => {
                    println!("✗ 未被标记为硬件失败 —— 不会触发 CPU 回退");
                    failures += 1;
                }
            }
        }
    }

    // ── 场景 2：完整 stderr 落在输出目录旁的日志里 ─────────────────────────────
    println!("\n=== 场景 2：完整报错写入日志文件 ===");
    let log = tracereel::encode_log::log_file();
    match std::fs::read_to_string(&log) {
        Ok(body) => {
            println!("日志文件：{}（{} 字节）", log.display(), body.len());
            let has_raw = body.contains("Could not open encoder")
                || body.contains("Nothing was written into output file")
                || body.contains("错误");
            if has_raw {
                println!("✓ 日志里含 ffmpeg 原始输出（排查用信息没有丢失）");
            } else {
                println!("✗ 日志里没有 ffmpeg 原始输出：\n{body}");
                failures += 1;
            }
        }
        Err(e) => {
            println!("✗ 日志文件不可读: {e}");
            failures += 1;
        }
    }

    // ── 场景 3：回退到 CPU 后导出真的能成功 ───────────────────────────────────
    println!("\n=== 场景 3：改用软件编码后导出成功 ===");
    let mut good = bad.clone();
    good.backend = "software".into();
    // 用生产代码的同一个挑选函数（本机 ffmpeg 没有 libaom-av1，应换成 H.264）。
    let (fb_fmt, sw) = tracereel::encode::pick_software_fallback(
        &good.format,
        &tracereel::encode::available_encoders(),
    );
    good.format = fb_fmt.clone();
    println!("回退编码器 = {sw}（格式 {fb_fmt}）");
    let good_out = out_dir.join("cpu_ok.mp4").to_string_lossy().into_owned();
    {
        match FfmpegEncoder::open(&good_out, &src, &[], w, h, fps, 1000.0, &good) {
            Ok(mut e) => {
                let frame = vec![128u8; w * h * 3];
                let mut wrote = 0;
                for _ in 0..10 {
                    if e.write_frame(&frame).is_err() {
                        break;
                    }
                    wrote += 1;
                }
                match e.close() {
                    Ok(p) => {
                        let sz = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
                        if sz > 0 && wrote == 10 {
                            println!("✓ CPU 回退导出成功：{wrote} 帧，{sz} 字节 -> {p}");
                        } else {
                            println!("✗ 输出异常：{wrote} 帧，{sz} 字节");
                            failures += 1;
                        }
                    }
                    Err(err) => {
                        println!("✗ 收尾失败: {err}");
                        failures += 1;
                    }
                }
            }
            Err(err) => {
                println!("✗ 软件编码器打开失败: {err}");
                failures += 1;
            }
        }
    }

    // ── 场景 4：分类器不会把普通错误误判成显卡问题 ─────────────────────────────
    println!("\n=== 场景 4：普通错误不触发回退 ===");
    for s in [
        "av_interleaved_write_frame(): No space left on device",
        "Error opening output file x.mp4: Permission denied",
    ] {
        if classify_hw_failure(s).is_none() && hw_failure_reason(s, "av1_qsv", "av1").is_none() {
            println!("✓ 未误判: {s}");
        } else {
            println!("✗ 误判为硬件失败: {s}");
            failures += 1;
        }
    }

    println!("\n=== 结果：{} ===", if failures == 0 { "全部通过" } else { "有失败项" });
    if failures > 0 {
        std::process::exit(1);
    }
}