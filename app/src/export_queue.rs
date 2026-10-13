//! Persistent configuration snapshots and isolated preparation for queued exports.
use crate::{
    encode::*,
    settings::{IniFile, Settings},
};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{atomic::Ordering, Arc},
};

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Entry {
    pub project: String,
    pub ini: String,
    pub output: String,
    pub status: String,
    pub error: String,
    #[serde(default)]
    pub durations_s: Option<Vec<f64>>,
    #[serde(default)]
    pub duration_error: String,
    #[serde(default)]
    pub progress: f64,
}
impl Entry {
    pub fn settings(&self) -> Settings {
        Settings::from_ini(&IniFile::parse(&self.ini))
    }
}

pub fn storage_path() -> PathBuf {
    crate::data_paths::data_dir("LOCALAPPDATA")
        .join("export-queue.json")
}
pub fn load(path: &Path) -> Result<Vec<Entry>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let mut entries: Vec<Entry> = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    entries.retain(|e| e.status != "已完成");
    for e in &mut entries {
        if e.status == "导出中" {
            e.status = "待导出".into();
            e.error = "上次导出被中断；请检查残留文件后重试".into();
            e.progress = 0.0;
        }
    }
    Ok(entries)
}
pub fn save(path: &Path, entries: &[Entry]) -> Result<(), String> {
    std::fs::create_dir_all(path.parent().ok_or("无效队列路径")?).map_err(|e| e.to_string())?;
    let temp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(entries).map_err(|e| e.to_string())?;
    use std::io::Write;
    let mut file = std::fs::File::create(&temp).map_err(|e| e.to_string())?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    drop(file);
    std::fs::rename(temp, path).map_err(|e| e.to_string())
}
pub fn outputs(s: &Settings, out: &str) -> Result<Vec<String>, String> {
    if out.trim().is_empty() {
        return Err("请先设置输出视频文件路径".into());
    }
    if !s.encode.include_map && !s.encode.include_video && !s.encode.include_hud {
        return Err("请至少选择一个导出内容：地图、视频或 HUD".into());
    }
    if s.encode.seg_enabled && s.encode.all_seg_enabled {
        let n = s.encode_total_segments();
        if n < 1 {
            return Err("没有可用的分段".into());
        }
        Ok((0..n).map(|i| segment_output_path(out, i, i + 1)).collect())
    } else {
        Ok(vec![out.into()])
    }
}
/// Frame-rounded output lengths, using the same clock/range/fps rules as the encoder.
pub fn durations(mut s: Settings) -> Result<Vec<f64>, String> {
    s.encode.map_only = !s.encode.include_video;
    let (duration_ms, fps) = if s.encode.map_only {
        let prep = EncodePrep::build(&s, 2, 2)?;
        let route_ms = prep
            .maps
            .iter()
            .map(|m| m.rm.duration_s())
            .fold(0.0_f64, f64::max)
            * 1000.0;
        let gpx_ms = prep
            .gpx
            .as_ref()
            .and_then(|g| {
                g.points
                    .last()
                    .map(|p| (p.t - g.start_time).max(0.0) * 1000.0)
            })
            .unwrap_or(0.0);
        (route_ms.max(gpx_ms), encode_output_fps(&s.encode, 30.0))
    } else {
        let mut duration = 0.0;
        let mut fps = None;
        for path in s.video.all_paths() {
            let info = crate::probe::probe_video(&path)
                .ok_or_else(|| format!("无法读取源视频：{path}"))?;
            duration += info.duration_ms;
            fps.get_or_insert(info.fps);
        }
        (
            duration,
            encode_output_fps(&s.encode, fps.ok_or("没有源视频")?),
        )
    };
    if duration_ms <= 0.0 || fps <= 0.0 {
        return Err("无法确定导出时长".into());
    }
    output_durations(&s, duration_ms, fps)
}
pub fn output_durations(s: &Settings, duration_ms: f64, fps: f64) -> Result<Vec<f64>, String> {
    let batch = s.encode.seg_enabled && s.encode.all_seg_enabled;
    let count = if batch { s.encode_total_segments() } else { 1 };
    if count < 1 {
        return Err("没有可用分段".into());
    }
    Ok((0..count)
        .map(|i| {
            let mut s = s.clone();
            if batch {
                s.encode.start_seg = i;
                s.encode.end_seg = i + 1;
            }
            s.resolve_encode_clip_ms();
            let (start, end) = if s.encode.map_only {
                let start = s.encode.clip_start_ms.max(0.0).min(duration_ms);
                let end = if s.encode.clip_end_ms > start {
                    s.encode.clip_end_ms.min(duration_ms)
                } else {
                    duration_ms
                };
                (start, end)
            } else {
                clip_range(&s.encode, duration_ms)
            };
            (((end - start) / 1000.0 * fps / encode_speed_ratio(&s.encode)).round() as usize).max(1)
                as f64
                / fps
        })
        .collect())
}

pub fn configuration(s: &Settings) -> Vec<String> {
    let e = &s.encode;
    let backend = match e.backend.as_str() {
        "software" => "CPU",
        "qsv" => "QSV",
        "nvenc" => "NVENC",
        "amf" => "AMF",
        "videotoolbox" => "VideoToolbox",
        x => x,
    };
    let rate = match e.rate_control.as_str() {
        "cbr" => "CBR恒定码率",
        "vbr" => "VBR可变码率",
        _ => "CRF恒定质量",
    };
    let audio = match e.audio_codec.as_str() {
        "copy" => "复制原声",
        "none" => "无音频",
        x => x,
    };
    let seg = if !e.seg_enabled {
        "不分段".into()
    } else if e.all_seg_enabled {
        format!("逐段导出（{} 段）", s.encode_total_segments())
    } else {
        format!("自定义（{}-{}）", e.start_seg, e.end_seg)
    };
    let rate_value = match e.rate_control.as_str() {
        "cbr" | "vbr" => format!("码率：{} Mbps", e.bitrate.trim().trim_end_matches(['M', 'm'])),
        "crf" => format!("质量：{}", e.quality),
        _ => String::new(),
    };
    // 存储的是 token，界面上不给用户看英文（与「帧率」同样式）。
    let resolution = if e.resolution.eq_ignore_ascii_case("source") {
        "与原始视频一致".to_string()
    } else {
        e.resolution.clone()
    };
    vec![
        format!(
            "编码器：{backend}    格式：{}    码率控制：{rate}",
            e.format.to_uppercase()
        ),
        format!("{rate_value}    音频：{audio}"),
        format!(
            "导出内容：地图 {} · 视频 {} · HUD {}",
            if e.include_map { "✓" } else { "×" },
            if e.include_video { "✓" } else { "×" },
            if e.include_hud { "✓" } else { "×" }
        ),
        format!(
            "分段导出：{seg}    分辨率：{resolution}    帧率：{}    导出倍速：{}x",
            if e.frame_rate == "source" {
                "与原始视频一致"
            } else {
                &e.frame_rate
            },
            e.speed
        ),
    ]
}

pub fn path_key(p: &str) -> String {
    // Canonicalize the parent even when the output does not exist (case/relative aliases).
    let p = PathBuf::from(p);
    let p = if p.is_absolute() {
        p
    } else {
        std::env::current_dir().unwrap_or_default().join(p)
    };
    let parent = p.parent().and_then(|d| d.canonicalize().ok());
    parent
        .and_then(|d| p.file_name().map(|n| d.join(n)))
        .unwrap_or(p)
        .to_string_lossy()
        .replace('\\', "/")
        .to_lowercase()
}
pub fn overwrite_warning(s: &Settings, out: &str) -> String {
    let existing: Vec<_> = outputs(s, out)
        .unwrap_or_default()
        .into_iter()
        .filter(|p| Path::new(p).exists())
        .collect();
    if existing.is_empty() {
        String::new()
    } else {
        format!("以下已有文件将在导出时覆盖：\n{}", existing.join("\n"))
    }
}

pub fn check_conflicts(
    s: &Settings,
    out: &str,
    entries: &[Entry],
    except: Option<usize>,
) -> Result<(), String> {
    let paths = outputs(s, out)?;
    for p in paths {
        let sources = s.video.all_paths();
        if sources.iter().any(|src| path_key(src) == path_key(&p)) {
            return Err(format!("输出路径与源视频冲突：{p}"));
        }
        for (i, e) in entries.iter().enumerate() {
            if Some(i) == except {
                continue;
            }
            if outputs(&e.settings(), &e.output)?
                .iter()
                .any(|q| path_key(q) == path_key(&p))
            {
                return Err(format!(
                    "与队列第 {} 项输出冲突：{p}。请更换文件名。",
                    i + 1
                ));
            }
        }
    }
    Ok(())
}

/// Prepare from the immutable snapshot, never from the active preview project.
pub fn run(mut s: Settings, out: String, handle: Arc<EncodeHandle>) {
    let result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<(), String> {
            s.encode.map_only = !s.encode.include_video;
            if s.encode.map_only {
                s.encode.audio_codec = "none".into();
            }
            s.resolve_encode_clip_ms();
            let paths = outputs(&s, &out)?;
            let mut info = crate::probe::VideoInfo {
                width: 0,
                height: 0,
                fps: 0.0,
                duration_ms: 0.0,
                has_motion_data: false,
                frame_count: 0,
            };
            let mut segs = Vec::new();
            let mut stabs = Vec::new();
            let mut dec = (0, 0);
            if !s.encode.map_only {
                for p in s.video.all_paths() {
                    if handle.stop.load(Ordering::SeqCst) {
                        return Err("已中止".into());
                    }
                    let i = crate::probe::probe_video(&p)
                        .ok_or_else(|| format!("无法读取源视频：{p}"))?;
                    if i.width == 0 || i.fps <= 0.0 {
                        return Err(format!("视频信息无效：{p}"));
                    }
                    if segs.is_empty() {
                        info = i.clone();
                        info.duration_ms = 0.0;
                        info.frame_count = 0;
                        // 与直接导出同一条规则：按输出画布里面板实际需要的像素解码（上限源尺寸）。
                        dec = crate::encode::export_decode_size(&s, i.width, i.height);
                    }
                    let seg = crate::gui::VideoSeg {
                        path: p.clone(),
                        start_ms: info.duration_ms,
                        duration_ms: i.duration_ms,
                        fps: i.fps,
                        frame_count: i.frame_count,
                        has_motion: i.has_motion_data,
                    };
                    handle.set_segment_label(format!("正在准备视频 {}", segs.len() + 1));
                    let stab = if s.gyro.enabled && i.has_motion_data {
                        crate::gyro::init_gpu();
                        Some(Arc::new(
                            crate::gyro::load_stabilizer(
                                &p,
                                &s.gyro,
                                dec.0,
                                dec.1,
                                i.fps,
                                i.duration_ms,
                                i.frame_count,
                                false,
                                crate::gui::gyro_offset_ms(&s, info.fps),
                                crate::gyro::no_progress(),
                            )
                            .map_err(|e| e.to_string())?,
                        ))
                    } else {
                        None
                    };
                    stabs.push(stab);
                    segs.push(seg);
                    info.duration_ms += i.duration_ms;
                    info.frame_count += i.frame_count;
                }
                if segs.is_empty() {
                    return Err("请配置有效的源视频".into());
                }
                if s.gyro.enabled && !segs.iter().any(|seg| seg.has_motion) {
                    return Err("视频没有运动数据，无法启用防抖".into());
                }
                crate::gyro::apply_seam_warmup_smoothing(&stabs, crate::gyro::no_progress());
            }
            if handle.stop.load(Ordering::SeqCst) {
                return Err("已中止".into());
            }
            let batch = s.encode.seg_enabled && s.encode.all_seg_enabled;
            let mut jobs = Vec::new();
            for (idx, p) in paths.into_iter().enumerate() {
                let mut settings = s.clone();
                if batch {
                    settings.encode.start_seg = idx as i32;
                    settings.encode.end_seg = idx as i32 + 1;
                    settings.resolve_encode_clip_ms();
                }
                let (clip_start, clip_end) = clip_range(&settings.encode, info.duration_ms);
                jobs.push(EncodeJob {
                    settings,
                    out_path: p,
                    src_w: info.width,
                    src_h: info.height,
                    src_fps: info.fps,
                    dec_w: dec.0,
                    dec_h: dec.1,
                    duration_ms: info.duration_ms,
                    clip_start,
                    clip_end,
                    handle: handle.clone(),
                    stabs: stabs.clone(),
                    segs: segs.clone(),
                });
            }
            handle.set_segment_label(String::new());
            if batch {
                encode_export_segments(jobs);
            } else {
                encode_export(jobs.remove(0));
            }
            Ok(())
        }));
    let error = match result {
        Ok(Ok(())) => return,
        Ok(Err(e)) => e,
        Err(_) => "导出线程异常，任务已保留".into(),
    };
    handle.set_result(EncodeResult {
        output: out,
        aborted: handle.stop.load(Ordering::SeqCst),
        error,
    });
    handle.mark_done();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configuration_shows_resolution_in_chinese() {
        let mut s = Settings::default();
        // 「与原始视频一致」在队列里不能露出 token "source"（界面文字不许出现英文）。
        s.encode.resolution = "source".into();
        let text = configuration(&s).join("\n");
        assert!(text.contains("分辨率：与原始视频一致"), "{text}");
        assert!(!text.contains("source"), "{text}");
        // 显式尺寸照旧显示。
        s.encode.resolution = "2688x2016".into();
        let text = configuration(&s).join("\n");
        assert!(text.contains("分辨率：2688x2016"), "{text}");
    }

    #[test]
    fn output_length_uses_clip_speed_and_frame_rounding() {
        let mut s = Settings::default();
        s.encode.speed = "2".into();
        s.encode.seg_enabled = true;
        s.encode.start_seg = 1;
        s.encode.end_seg = 2;
        s.splits.split_times = "1|5".into();
        s.splits.is_relative = false;
        s.video.start_time_offset_s = 0.0;
        assert_eq!(output_durations(&s, 10000.0, 30.0).unwrap(), vec![2.0]);
        s.encode.seg_enabled = false;
        s.encode.speed = "4".into();
        assert_eq!(
            output_durations(&s, 1000.0, 30.0).unwrap(),
            vec![8.0 / 30.0]
        );
        s.encode.seg_enabled = true;
        s.encode.all_seg_enabled = true;
        s.splits.split_times = "2|5".into();
        s.splits.is_relative = false;
        s.video.start_time_offset_s = 0.0;
        let lengths = output_durations(&s, 10000.0, 30.0).unwrap();
        assert_eq!(lengths, vec![0.5, 23.0 / 30.0]);
    }
    #[test]
    fn restart_discards_successful_entries() {
        let path =
            std::env::temp_dir().join(format!("ov-queue-completed-{}.json", std::process::id()));
        let done = Entry {
            status: "已完成".into(),
            ..Default::default()
        };
        let pending = Entry {
            status: "待导出".into(),
            progress: 0.25,
            ..Default::default()
        };
        save(&path, &[done, pending]).unwrap();
        let restored = load(&path).unwrap();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].status, "待导出");
        assert_eq!(restored[0].progress, 0.25);
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn snapshot_and_restart() {
        let dir = std::env::temp_dir().join(format!("ov-queue-test-{}", std::process::id()));
        let path = dir.join("queue.json");
        let mut s = Settings::default();
        s.encode.speed = "2".into();
        let e = Entry {
            project: "项目.orv".into(),
            ini: s.to_ini(),
            output: "视频.mp4".into(),
            status: "导出中".into(),
            error: String::new(),
            ..Default::default()
        };
        s.encode.speed = "4".into();
        save(&path, &[e]).unwrap();
        let restored = load(&path).unwrap();
        assert_eq!(restored[0].settings().encode.speed, "2");
        assert_eq!(restored[0].status, "待导出");
        save(&path, &restored).unwrap(); // replacing an existing manifest on Windows
        assert_eq!(load(&path).unwrap().len(), 1);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
    #[test]
    fn duplicate_outputs() {
        let s = Settings::default();
        let e = Entry {
            project: String::new(),
            ini: s.to_ini(),
            output: "queue-test.mp4".into(),
            status: "待导出".into(),
            error: String::new(),
            ..Default::default()
        };
        assert!(check_conflicts(&s, "QUEUE-TEST.mp4", &[e], None).is_err());
    }
    #[test]
    fn existing_file_is_warning_not_conflict() {
        let path =
            std::env::temp_dir().join(format!("ov-existing-output-{}.mp4", std::process::id()));
        std::fs::write(&path, b"existing").unwrap();
        let out = path.to_string_lossy().into_owned();
        let s = Settings::default();
        assert!(check_conflicts(&s, &out, &[], None).is_ok());
        assert!(overwrite_warning(&s, &out).contains(&out));
        let mut source = s.clone();
        source.video.video_path = out.clone();
        assert!(check_conflicts(&source, &out, &[], None).is_err());
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn segment_output_conflict() {
        let mut s = Settings::default();
        s.encode.seg_enabled = true;
        s.encode.all_seg_enabled = true;
        s.splits.split_times = "0|10|20".into();
        let paths = outputs(&s, "batch.mp4").unwrap();
        assert!(paths.len() > 1);
        let other = Settings::default();
        let e = Entry {
            project: String::new(),
            ini: other.to_ini(),
            output: paths[1].clone(),
            status: "待导出".into(),
            error: String::new(),
            ..Default::default()
        };
        assert!(check_conflicts(&s, "batch.mp4", &[e], None).is_err());
    }
}
