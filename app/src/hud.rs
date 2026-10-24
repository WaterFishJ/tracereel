//! Versioned, media-independent HUD layouts and deterministic telemetry rendering.
use crate::settings::Settings;
use cpp::cpp;
use tracereel_core::gpx::GpxTrack;
use qmetaobject::{
    qttypes::{QPainter, QRectF},
    QString,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Widget {
    pub id: String,
    pub metric: String,
    pub variant: String,
    pub style: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub rotation: f64,
    pub z: i32,
    pub opacity: f64,
    pub visible: bool,
    pub locked: bool,
    pub accent: String,
    pub background: String,
    pub foreground: String,
    pub font_size: f64,
    pub automatic: bool,
    pub maximum: f64,
    pub window: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<String>>,
}
impl Default for Widget {
    fn default() -> Self {
        Self {
            id: "widget".into(),
            metric: "speed".into(),
            variant: "digital".into(),
            style: "classic".into(),
            x: 0.7,
            y: 0.08,
            w: 0.22,
            h: 0.16,
            rotation: 0.0,
            z: 0,
            opacity: 0.9,
            visible: true,
            locked: false,
            accent: "#55c9e8".into(),
            background: "#b3101820".into(),
            foreground: "#ffffff".into(),
            font_size: 28.0,
            automatic: true,
            maximum: 30.0,
            window: 300.0,
            fields: None,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Layout {
    pub version: u32,
    pub widgets: Vec<Widget>,
}
impl Layout {
    pub fn parse(json: &str) -> Result<Self, String> {
        let value: Self = serde_json::from_str(json).map_err(|e| e.to_string())?;
        if value.version != 1 {
            return Err("不支持的 HUD 布局版本".into());
        }
        if value.widgets.len() > 100 {
            return Err("HUD 最多支持 100 个控件".into());
        }
        let mut ids = std::collections::HashSet::new();
        for w in &value.widgets {
            if !ids.insert(&w.id) || w.id.is_empty() {
                return Err("HUD 控件标识重复或为空".into());
            }
            if ![
                "speed",
                "heading",
                "elevation",
                "heart_rate",
                "cadence",
                "elapsed",
                "segment",
                "pace",
                "distance",
                "classic",
            ]
            .contains(&w.metric.as_str())
                || ![
                    "digital", "arc", "dial", "compass", "ribbon", "profile", "scroll", "heart",
                    "classic",
                ]
                .contains(&w.variant.as_str())
            {
                return Err("未知 HUD 控件".into());
            }
            if ![
                w.x,
                w.y,
                w.w,
                w.h,
                w.rotation,
                w.opacity,
                w.font_size,
                w.maximum,
                w.window,
            ]
            .iter()
            .all(|v| v.is_finite())
                || w.w < 0.02
                || w.h < 0.02
                || w.w > 1.0
                || w.h > 1.0
                || !(0.0..=1.0).contains(&w.opacity)
                || w.font_size < 8.0
                || w.font_size > 100.0
                || w.maximum <= 0.0
                || w.window < 1.0
            {
                return Err("HUD 尺寸或数值无效".into());
            }
            if let Some(fields) = &w.fields {
                let mut unique = std::collections::HashSet::new();
                if fields.len() > 9
                    || fields.iter().any(|field| {
                        ![
                            "speed",
                            "heading",
                            "elevation",
                            "heart_rate",
                            "cadence",
                            "elapsed",
                            "segment",
                            "pace",
                            "distance",
                        ]
                        .contains(&field.as_str())
                            || !unique.insert(field)
                    })
                {
                    return Err("经典面板数据项无效".into());
                }
            }
        }
        Ok(value)
    }
    pub fn json(&self) -> String {
        serde_json::to_string(self).unwrap()
    }
    pub fn classic(s: &Settings) -> Self {
        let flags = [
            ("elapsed", s.hud.show_elapsed_time),
            ("segment", s.hud.show_segment_time),
            ("speed", s.hud.show_speed),
            ("pace", s.hud.show_pace),
            ("heart_rate", s.hud.show_heart_rate),
            ("distance", s.hud.show_distance),
            ("heading", s.hud.show_azimuth),
            ("elevation", s.hud.show_elevation),
            ("cadence", s.hud.show_cadence),
        ];
        let w = Widget {
            id: "classic".into(),
            metric: "classic".into(),
            variant: "classic".into(),
            x: 0.74,
            y: 0.60,
            w: 0.23,
            h: 0.35,
            font_size: 24.0,
            style: "classic".into(),
            accent: "#55c9e8".into(),
            background: "#b3101820".into(),
            fields: Some(
                flags
                    .iter()
                    .filter_map(|(name, enabled)| enabled.then_some((*name).to_string()))
                    .collect(),
            ),
            ..Default::default()
        };
        Self {
            version: 1,
            widgets: vec![w],
        }
    }
    pub fn theme(name: &str) -> Self {
        if matches!(name, "simple" | "legacy") {
            let mut layout = Self::classic(&Settings::default());
            layout.widgets[0].fields = Some(
                [
                    "elapsed",
                    "segment",
                    "speed",
                    "pace",
                    "heart_rate",
                    "distance",
                    "heading",
                    "elevation",
                ]
                .iter()
                .map(|field| (*field).into())
                .collect(),
            );
            return layout;
        }
        // Aligned two-column data cards on the left and instruments on the right.
        // Old theme keys remain aliases for callers and previously saved workflows.
        let mut specs = vec![
            ("elapsed", "digital", 0.03, 0.76, 0.18, 0.09),
            ("segment", "digital", 0.22, 0.76, 0.18, 0.09),
            ("pace", "digital", 0.03, 0.86, 0.18, 0.09),
            ("distance", "digital", 0.22, 0.86, 0.18, 0.09),
            ("heading", "compass", 0.735, 0.68, 0.11, 0.27),
            ("heart_rate", "heart", 0.86, 0.68, 0.11, 0.27),
        ];
        if matches!(name, "park" | "outdoor") {
            specs.push(("elevation", "profile", 0.03, 0.57, 0.37, 0.17));
        }
        let mut layout = Self {
            version: 1,
            widgets: specs
                .iter()
                .enumerate()
                .map(|(i, (metric, variant, x, y, w, h))| Widget {
                    id: format!("theme-{i}"),
                    metric: (*metric).into(),
                    variant: (*variant).into(),
                    x: *x,
                    y: *y,
                    w: *w,
                    h: *h,
                    accent: if *metric == "heart_rate" {
                        "#ff6577"
                    } else {
                        "#55c9e8"
                    }
                    .into(),
                    font_size: if *metric == "segment" { 28.0 } else { 32.0 },
                    ..Default::default()
                })
                .collect(),
        };
        if matches!(name, "park" | "outdoor") {
            for w in &mut layout.widgets {
                // Shrink each corner group by 10%, preserving its outside/bottom edges.
                w.x = if w.x < 0.5 {
                    0.03 + (w.x - 0.03) * 0.9
                } else {
                    0.97 - (0.97 - w.x) * 0.9
                };
                w.y = 0.95 - (0.95 - w.y) * 0.9;
                w.w *= 0.9;
                w.h *= 0.9;
                w.font_size *= 0.9;
                // Keep preset JSON stable through save/load after fractional scaling.
                for value in [&mut w.x, &mut w.y, &mut w.w, &mut w.h, &mut w.font_size] {
                    *value = (*value * 1_000_000.0).round() / 1_000_000.0;
                }
            }
        }
        layout
    }
}

/// 总用时的封顶时刻（秒，出发后经过的秒数）：**最后一个分段点**。
///
/// 视频通常比比赛长（跑完还举着相机走一段、或录到收工），总用时若一直跟着视频涨，
/// 冲线之后画面上的「总用时」就变成没有意义的时长，回放时也看不出最终成绩。
/// 所以总用时的读数到最后一个分段点为止，之后冻结在终点成绩上。
///
/// 分段列表为空（没填分段点）时没有终点可依，返回 `None`，总用时按原样跟随视频时刻。
/// 扫描 GPX / 轨迹末点来决定终点是不对的：那两份数据往往一边多录了几秒、
/// 或者终点之后还在走，真正的终点成绩只有分段表说得清楚。
thread_local! {
    /// `elapsed_freeze_s` 的缓存：HUD 每帧都会问一次终点时刻，而解析分段串会分配内存。
    /// 键 =（分段串, 相对/绝对），分段设置一变就重算；正常播放时每帧只是字符串比较。
    static FREEZE: std::cell::RefCell<(String, bool, Option<f64>)> = Default::default();
}

pub fn elapsed_freeze_s(settings: &Settings) -> Option<f64> {
    FREEZE.with(|cache| {
        let mut c = cache.borrow_mut();
        if c.0 != settings.splits.split_times || c.1 != settings.splits.is_relative {
            *c = (
                settings.splits.split_times.clone(),
                settings.splits.is_relative,
                compute_elapsed_freeze_s(settings),
            );
        }
        c.2
    })
}

fn compute_elapsed_freeze_s(settings: &Settings) -> Option<f64> {
    let info = tracereel_core::splits::build_runner_info(
        &settings.splits.split_times,
        settings.splits.is_relative,
    );
    // splits[0] 是隐式起点(t=0)，至少要有一个真实分段点才谈得上终点。
    let last = info.splits.last()?;
    if info.splits.len() < 2 {
        return None;
    }
    Some(last.absolute_time.max(0.0))
}

pub fn duration(seconds: f64) -> String {
    let n = (seconds.max(0.0) * 10.0).round() as u64;
    let (h, m, s, d) = (n / 36000, n / 600 % 60, n / 10 % 60, n % 10);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}.{d}")
    } else if m > 0 {
        format!("{m}:{s:02}.{d}")
    } else {
        format!("{s}.{d}")
    }
}
thread_local! { static SPLITS: std::cell::RefCell<(String,bool,Vec<f64>)> = Default::default(); }
fn segment(elapsed: f64, s: &Settings) -> String {
    SPLITS.with(|cache| {
        let mut c = cache.borrow_mut();
        if c.0 != s.splits.split_times || c.1 != s.splits.is_relative {
            *c = (
                s.splits.split_times.clone(),
                s.splits.is_relative,
                tracereel_core::splits::build_runner_info(&s.splits.split_times, s.splits.is_relative)
                    .splits
                    .iter()
                    .map(|x| x.absolute_time)
                    .collect(),
            );
        }
        let pair =
            c.2.windows(2)
                .find(|v| elapsed < v[1])
                .or_else(|| c.2.windows(2).last());
        pair.map(|v| {
            let total = (v[1] - v[0]).max(0.0);
            format!(
                "{} | {}",
                duration((elapsed - v[0]).clamp(0.0, total)),
                duration(total)
            )
        })
        .unwrap_or_else(|| "-- | --".into())
    })
}

cpp! {{
    #include "hud_painter.h"
}}
/// HUD 的绘制范围（归一化坐标的参考系）。
///
/// 默认沿用老规矩：归到「贴合后的视频画面」里，不压地图也不压黑边；`hud.scope = "frame"`
/// 时改成整幅画面，HUD 就能摆到地图那半边。没有视频面板时（只导地图+HUD，或预览里取消
/// 勾选「视频」）一律按整幅画面算 —— 那时候视频区宽度是 0，再按老规矩画 HUD 会整块消失。
pub fn viewport(
    settings: &Settings,
    width: f64,
    height: f64,
    video_size: Option<(f64, f64)>,
) -> QRectF {
    let map_configured = !settings.map.image_file_path.trim().is_empty()
        || !settings.route.quick_route_jpeg_file_path.trim().is_empty()
        || settings
            .map
            .extra_map_files
            .iter()
            .any(|p| !p.trim().is_empty());
    if !settings.encode.include_video || settings.hud.scope.eq_ignore_ascii_case("frame") {
        return QRectF {
            x: 0.0,
            y: 0.0,
            width,
            height,
        };
    }
    let left = if map_configured && settings.encode.include_map {
        settings.map.relative_width.clamp(0.0, 1.0) * width
    } else {
        0.0
    };
    let available = (width - left).max(0.0);
    let (w, h) = video_size
        .filter(|(w, h)| *w > 0.0 && *h > 0.0)
        .map(|(w, h)| {
            let scale = (available / w).min(height / h);
            (w * scale, h * scale)
        })
        .unwrap_or((available, height));
    QRectF {
        x: left + (available - w) / 2.0,
        y: (height - h) / 2.0,
        width: w,
        height: h,
    }
}

pub fn draw_in_viewport(
    p: &mut QPainter,
    settings: &Settings,
    track: Option<&GpxTrack>,
    t_ms: f64,
    rect: QRectF,
) {
    if rect.width <= 0.0 || rect.height <= 0.0 {
        return;
    }
    let x = rect.x;
    let y = rect.y;
    let width = rect.width;
    let height = rect.height;
    unsafe {
        cpp!([p as "QPainter *", x as "double", y as "double", width as "double", height as "double"] {
            p->save(); p->setClipRect(QRectF(x,y,width,height),Qt::IntersectClip); p->translate(x,y);
        });
    }
    draw(p, settings, track, t_ms, width, height);
    p.restore();
}
/// HUD 轨迹的采样偏移（秒）：把**视频时刻**换算成**轨迹时刻**，基准是「比赛出发时刻」。
///
/// 「录像-出发时间」已经把视频时间轴对齐到出发时刻（出发时刻 = 视频第 `T_video` 秒），所以
/// 这里的输入时间是**出发后经过的秒数**；`gpx_time_offset` 则是这条 GPX 自己的时间轴相对
/// 出发时刻的偏移（出发后 8 秒手表才搜到 GPS 开始记录 → −8）。两者相减就是轨迹时刻。
///
/// 例（孝德湖）：录像在出发前 17 秒开录 → 「录像-出发时间」= 17；出发后 8 秒才开表 →
/// HUD 这一栏 = −8；于是 轨迹时刻 = 视频时刻 − 17 − 8 = 视频时刻 − 25。
///
/// **不要把「按表-出发时间」也自动加进来**：那个值描述的是轨迹（路线）文件自己的时间轴，
/// 而 HUD 用的 GPX 可能是另一份文件，两者必须各自指定 —— 自动代入会在同一份工程里重复计入
/// 一边，用户按自己的习惯填 −8 时就会变成 −33。
pub fn hud_track_offset(s: &Settings) -> f64 {
    s.hud.gpx_time_offset - s.video.start_time_offset_s
}

/// 某一帧上「总用时 / 分段用时」要显示的两个秒数（**HUD 显示的唯一真相**）。
///
/// 总用时 = `视频时刻 − 录像-出发时间`，并在最后一个分段点封顶（见 [`elapsed_freeze_s`]）；
/// 分段用时不受影响，照常跟随视频时刻 —— 冲线后镜头里其他读数不会集体停住。
pub fn elapsed_values(settings: &Settings, t_ms: f64) -> (f64, f64) {
    let elapsed = (t_ms / 1000.0 - settings.video.start_time_offset_s).max(0.0);
    let total = elapsed_freeze_s(settings).map_or(elapsed, |end| elapsed.min(end));
    (total, elapsed)
}

/// Shared preview/export telemetry; null metrics render as `--`.
pub fn telemetry_values(settings: &Settings, track: Option<&GpxTrack>, t_ms: f64) -> serde_json::Value {
    let (elapsed_total, elapsed) = elapsed_values(settings, t_ms);
    let gpx_offset = hud_track_offset(settings);
    let sample = track.and_then(|t| t.sample_at(t_ms / 1000.0, gpx_offset));
    let ht = track.and_then(|t| t.hud_at(t_ms / 1000.0, gpx_offset));
    serde_json::json!({
        "speed":sample.map(|s|s.speed_kmh), "heading":ht.and_then(|v|v.0),
        "elevation":sample.and_then(|s|s.altitude_m),"heart_rate":sample.and_then(|s|s.heart_rate),
        "cadence":sample.and_then(|s|s.cadence),
        "distance":sample.map(|s|s.distance_km),"pace":sample.map(|s| if s.pace_min_km>0.0 {format!("{} /km",duration(s.pace_min_km*60.0))} else {"-- /km".into()}),
        "elapsed":duration(elapsed_total),"segment":segment(elapsed,settings),
        "phase":ht.map(|v|v.1).unwrap_or(0.0),"max_speed":track.map(|t|t.hud_max_speed()).unwrap_or(30.0),
        "track_time":t_ms/1000.0+gpx_offset,
        "classic_flags":[settings.hud.show_elapsed_time,settings.hud.show_segment_time,settings.hud.show_speed,
            settings.hud.show_pace,settings.hud.show_heart_rate,settings.hud.show_distance,settings.hud.show_azimuth,settings.hud.show_elevation,settings.hud.show_cadence]
    })
}

pub fn draw(
    p: &mut QPainter,
    settings: &Settings,
    track: Option<&GpxTrack>,
    t_ms: f64,
    width: f64,
    height: f64,
) {
    let raw = settings
        .hud
        .layout
        .as_ref()
        .map(|l| l.json())
        .unwrap_or_else(|| Layout::classic(settings).json());
    let layout = QString::from(raw.as_str());
    let values = telemetry_values(settings, track, t_ms);
    let values = QString::from(values.to_string().as_str());
    let profile = track.map(|t| t.hud_profile()).unwrap_or(&[]);
    let count = profile.len() as i32;
    let points = profile.as_ptr() as *const f64;
    unsafe {
        cpp!([p as "QPainter *", layout as "QString", values as "QString", width as "double",height as "double",points as "const double *",count as "int"] {
            ovDrawHud(p,layout,values,width,height,points,count);
        });
    }
}

/// Store only layouts; generated IDs, never user names, are used as filenames.
pub fn template_dir() -> std::path::PathBuf {
    crate::data_paths::data_dir("APPDATA")
        .join("hud-templates")
}
#[derive(Serialize, Deserialize)]
pub struct Template {
    pub name: String,
    pub layout: Layout,
}
pub fn templates() -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(template_dir()) {
        for e in entries.flatten() {
            if e.path().extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            if let Ok(raw) = std::fs::read_to_string(e.path()) {
                if let Ok(t) = serde_json::from_str::<Template>(&raw) {
                    out.push(
                        serde_json::json!({"id":e.file_name().to_string_lossy(),"name":t.name}),
                    );
                }
            }
        }
    }
    out.sort_by_key(|v| v["name"].as_str().unwrap_or("").to_owned());
    out
}
pub fn template_action(action: &str, id: &str, name: &str, layout: &str) -> Result<String, String> {
    template_action_in(&template_dir(), action, id, name, layout)
}
fn template_action_in(
    dir: &std::path::Path,
    action: &str,
    id: &str,
    name: &str,
    layout: &str,
) -> Result<String, String> {
    if action == "save" {
        let layout = Layout::parse(layout)?;
        if name.trim().is_empty() {
            return Err("请输入模板名称".into());
        }
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let id = format!(
            "{}.json",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dir.join(&id))
            .map_err(|e| e.to_string())?;
        serde_json::to_writer(
            file,
            &Template {
                name: name.trim().into(),
                layout,
            },
        )
        .map_err(|e| e.to_string())?;
        return Ok(id);
    }
    if !id.ends_with(".json")
        || !id
            .trim_end_matches(".json")
            .chars()
            .all(|c| c.is_ascii_digit())
    {
        return Err("无效模板标识".into());
    }
    let path = dir.join(id);
    let raw = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let mut t: Template = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    match action {
        "load" => {
            let json = t.layout.json();
            Layout::parse(&json)?;
            Ok(json)
        }
        "rename" => {
            if name.trim().is_empty() {
                return Err("请输入模板名称".into());
            }
            t.name = name.trim().into();
            let tmp = path.with_extension("tmp");
            std::fs::write(&tmp, serde_json::to_vec(&t).unwrap()).map_err(|e| e.to_string())?;
            std::fs::rename(tmp, path).map_err(|e| e.to_string())?;
            Ok(String::new())
        }
        "delete" => {
            std::fs::remove_file(path).map_err(|e| e.to_string())?;
            Ok(String::new())
        }
        _ => Err("未知操作".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 孝德湖工程：录像在出发前 17 秒开录（视频页填 17），出发后 8 秒手表才搜到 GPS 开始
    /// 记录（HUD 这一栏填 −8，和「按表-出发时间」同值，因为 HUD 用的就是同一次记录）。
    /// 合计 −25：视频 600 秒处读轨迹 575 秒。
    #[test]
    fn hud_offset_is_anchored_at_the_race_start() {
        let mut s = Settings::default();
        s.video.start_time_offset_s = 17.0;
        s.hud.gpx_time_offset = -8.0;
        assert_eq!(hud_track_offset(&s), -25.0);

        // 填 0 时只按「录像-出发时间」换算，**不再偷偷带上轨迹那 −8 秒**：
        // 这一栏的含义就是「GPX 相对出发时刻的偏移」。
        s.hud.gpx_time_offset = 0.0;
        assert_eq!(hud_track_offset(&s), -17.0);

        // 没有「录像-出发时间」的工程行为跟以前完全一样：偏移就是这一栏的值。
        s.video.start_time_offset_s = 0.0;
        s.hud.gpx_time_offset = 3.0;
        assert_eq!(hud_track_offset(&s), 3.0);
    }

    /// 路线时钟顶替视频时钟（地图导出 / hudcheck）：帧时刻就是轨迹时刻，把「录像-出发时间」
    /// 设成「按表-出发时间」之后，采样偏移等于该栏自身 —— GPX 与轨迹同一条记录时正好为 0。
    #[test]
    fn route_clock_export_keeps_the_gpx_offset_only() {
        let mut s = Settings::default();
        s.route.control_time_offset = 3.0;
        s.video.start_time_offset_s = s.route.control_time_offset;
        s.hud.gpx_time_offset = 3.0;
        assert_eq!(hud_track_offset(&s), 0.0);
    }

    #[test]
    fn presets_have_requested_metrics_and_aligned_corners() {
        let campus = Layout::theme("campus");
        let metrics: Vec<_> = campus.widgets.iter().map(|w| w.metric.as_str()).collect();
        assert_eq!(
            metrics,
            [
                "elapsed",
                "segment",
                "pace",
                "distance",
                "heading",
                "heart_rate"
            ]
        );
        for pair in campus.widgets[..4].chunks(2) {
            assert_eq!(
                (pair[0].y, pair[0].w, pair[0].h),
                (pair[1].y, pair[1].w, pair[1].h)
            );
        }
        assert_eq!(campus.widgets[0].x, campus.widgets[2].x);
        assert_eq!(campus.widgets[1].x, campus.widgets[3].x);
        assert!(campus.widgets.iter().all(|w| w.style == "classic"));
        for w in &campus.widgets[2..] {
            assert!((w.y + w.h - 0.95).abs() < 1e-9);
        }
        let park = Layout::theme("park");
        assert_eq!(park.widgets.len(), 7);
        assert_eq!(park.widgets[6].metric, "elevation");
        assert_eq!(park.widgets[6].x, campus.widgets[0].x);
        assert!(
            (park.widgets[6].x + park.widgets[6].w - park.widgets[1].x - park.widgets[1].w).abs()
                < 1e-9
        );
        for (small, original) in park.widgets.iter().zip(&campus.widgets) {
            assert!((small.w - original.w * 0.9).abs() < 1e-9);
            assert!((small.h - original.h * 0.9).abs() < 1e-9);
        }
        let simple = Layout::theme("simple");
        assert_eq!(simple.widgets.len(), 1);
        assert_eq!(simple.widgets[0].fields.as_ref().unwrap().len(), 8);
        assert!((simple.widgets[0].x + simple.widgets[0].w - 0.97).abs() < 1e-9);
        assert!((simple.widgets[0].y + simple.widgets[0].h - 0.95).abs() < 1e-9);
    }
    #[test]
    fn hud_viewport_excludes_map_and_letterbox() {
        let mut s = Settings::default();
        s.map.relative_width = 0.25;
        let full = viewport(&s, 1920.0, 1080.0, Some((1920.0, 1080.0)));
        assert_eq!(
            (full.x, full.y, full.width, full.height),
            (0.0, 0.0, 1920.0, 1080.0)
        );
        s.map.image_file_path = "map.png".into();
        let video = viewport(&s, 1920.0, 1080.0, Some((1920.0, 1080.0)));
        assert_eq!(
            (video.x, video.y, video.width, video.height),
            (480.0, 135.0, 1440.0, 810.0)
        );
        let square = viewport(&s, 1920.0, 1080.0, Some((1080.0, 1080.0)));
        assert_eq!(
            (square.x, square.y, square.width, square.height),
            (660.0, 0.0, 1080.0, 1080.0)
        );
        s.map.image_file_path.clear();
        s.map.extra_map_files = vec!["second.png".into()];
        assert_eq!(viewport(&s, 1920.0, 1080.0, None).x, 480.0);
    }

    /// 「整幅画面」这一挡：HUD 能压到地图上，坐标从画布左上角起算。
    /// 以及没有视频面板时（只导地图+HUD）必须自动铺满，否则宽度为 0、HUD 整块看不见。
    #[test]
    fn hud_viewport_follows_the_scope_setting_and_needs_a_video_panel() {
        let mut s = Settings::default();
        s.map.image_file_path = "map.png".into();
        s.map.relative_width = 0.25;
        // 默认还是贴着视频画面。
        assert_eq!(viewport(&s, 1920.0, 1080.0, Some((1920.0, 1080.0))).x, 480.0);
        s.hud.scope = "frame".into();
        let frame = viewport(&s, 1920.0, 1080.0, Some((1920.0, 1080.0)));
        assert_eq!(
            (frame.x, frame.y, frame.width, frame.height),
            (0.0, 0.0, 1920.0, 1080.0)
        );
        // 回到「视频区」，但这次不导出视频（地图铺满整帧）：HUD 自动改成整幅画面。
        s.hud.scope = "video".into();
        s.encode.include_video = false;
        s.map.relative_width = 1.0;
        let map_only = viewport(&s, 1920.0, 1080.0, None);
        assert_eq!(
            (map_only.x, map_only.y, map_only.width, map_only.height),
            (0.0, 0.0, 1920.0, 1080.0)
        );
        // 不导出地图时，左边的黑带不该被算进 HUD 的范围（画面占比仍是 0.25）。
        s.encode.include_video = true;
        s.encode.include_map = false;
        s.map.relative_width = 0.0;
        assert_eq!(viewport(&s, 1920.0, 1080.0, Some((1920.0, 1080.0))).x, 0.0);
    }
    #[test]
    fn durations_and_segment_boundaries() {
        assert_eq!(duration(7.76), "7.8");
        assert_eq!(duration(67.76), "1:07.8");
        assert_eq!(duration(3723.36), "1:02:03.4");
        assert_eq!(duration(59.96), "1:00.0");
        let mut s = Settings::default();
        s.splits.split_times = "10|20|30".into();
        s.splits.is_relative = true;
        assert_eq!(segment(7.8, &s), "7.8 | 10.0");
        assert_eq!(segment(10.0, &s), "0.0 | 20.0");
        assert_eq!(segment(90.0, &s), "30.0 | 30.0");
        s.splits.is_relative = false;
        assert_eq!(segment(15.0, &s), "5.0 | 10.0");
        s.splits.split_times.clear();
        assert_eq!(segment(15.0, &s), "-- | --");
    }
    #[test]
    fn elapsed_freezes_at_the_last_split() {
        let mut s = Settings::default();
        // 绝对分段点：最后一个是终点成绩 1304 s（孝德湖实测值）。
        s.splits.split_times = "10|20|1304".into();
        s.splits.is_relative = false;
        assert_eq!(elapsed_freeze_s(&s), Some(1304.0));
        // 相对分段点按累加：33+63+41 = 137。
        s.splits.is_relative = true;
        s.splits.split_times = "33|63|41".into();
        assert_eq!(elapsed_freeze_s(&s), Some(137.0));
    }

    #[test]
    fn elapsed_has_no_freeze_point_without_splits() {
        let mut s = Settings::default();
        // 一个分段点都没有：没有终点可依，只能跟着视频走。
        assert_eq!(elapsed_freeze_s(&s), None);
        // 只有隐式起点（空串 / 全是分隔符）同样是 None，不能把总用时钉死在 0。
        s.splits.split_times = "||;".into();
        assert_eq!(elapsed_freeze_s(&s), None);
        // 只有一个分段点也是合法的终点成绩。
        s.splits.split_times = "1304".into();
        s.splits.is_relative = false;
        assert_eq!(elapsed_freeze_s(&s), Some(1304.0));
    }

    /// 总用时冲线即停，同一帧的分段用时（以及其他读数）仍跟着视频时刻走。
    /// 断言走的是 `draw` 用的同一条路径（`elapsed_values` → `duration`），
    /// 所以这里验的是画面上的读数，而不只是辅助函数。
    #[test]
    fn only_elapsed_freezes_past_the_last_split() {
        let mut s = Settings::default();
        s.splits.split_times = "10|20|30".into();
        s.splits.is_relative = false;
        // 视频 90 s 处已冲线：总用时停在 30.0 s，分段用时仍是 90.0 s。
        let (total, segment_s) = elapsed_values(&s, 90_000.0);
        assert_eq!(duration(total), "30.0");
        assert_eq!(duration(segment_s), "1:30.0");
        // 冲线之前两者一致，错过终点线不会提前封顶。
        let (total, segment_s) = elapsed_values(&s, 25_000.0);
        assert_eq!(duration(total), "25.0");
        assert_eq!(duration(segment_s), "25.0");
        // 恰好落在终点线上还是 30.0。
        let (total, _) = elapsed_values(&s, 30_000.0);
        assert_eq!(duration(total), "30.0");
    }

    /// 孝德湖实测：分段累加 1304 s、录像-出发时间 17 s ⇒ 冲线落在视频 1321 s。
    /// 视频还要多录一段时（这里取 1400 s），画面上的总用时必须停在 21:44.0。
    #[test]
    fn real_project_elapsed_stops_at_the_finish() {
        let mut s = Settings::default();
        s.video.start_time_offset_s = 17.0;
        s.splits.is_relative = true;
        s.splits.split_times =
            "0:33|1:03|1:29|0:41|2:05|0:37|1:17|2:19|0:43|1:08|0:25|2:05|0:58|0:45|1:05|0:32|1:07|0:36|0:37|0:24|0:34|0:41"
                .into();
        assert_eq!(elapsed_freeze_s(&s), Some(1304.0));
        // 冲线时刻：视频 1321 s（= 1304 + 17）读到的总用时正好是终点成绩。
        let (total, _) = elapsed_values(&s, 1_321_000.0);
        assert_eq!(duration(total), "21:44.0");
        // 之后视频继续跑到 1400 s，总用时不再增长。
        let (total, segment_s) = elapsed_values(&s, 1_400_000.0);
        assert_eq!(duration(total), "21:44.0");
        assert_eq!(duration(segment_s), "23:03.0");
    }

    #[test]
    fn cadence_payload_and_painter_keep_recorded_units_and_missing_marker() {
        let track = tracereel_core::gpx::parse_gpx_str(r#"<gpx xmlns:gpxdata="http://www.cluetrust.com/XML/GPXDATA/1/0"><trk><trkseg>
            <trkpt lat="30" lon="119"><time>2026-10-06T02:00:00Z</time><extensions><gpxdata:cadence>115</gpxdata:cadence></extensions></trkpt>
            <trkpt lat="30" lon="119"><time>2026-10-06T02:00:01Z</time></trkpt>
            </trkseg></trk></gpx>"#).unwrap();
        let mut s = Settings::default();
        s.video.start_time_offset_s = 17.0;
        s.hud.gpx_time_offset = -8.0;
        s.hud.show_cadence = true;
        let recorded = telemetry_values(&s, Some(&track), 25_000.0);
        assert_eq!(recorded["cadence"], 115.0);
        assert_eq!(recorded["classic_flags"][8], true);
        let painter_value = |values: serde_json::Value| {
            let data = QString::from(values.to_string().as_str());
            unsafe { cpp!([data as "QString"] -> QString as "QString" {
                return ovHudValue(QStringLiteral("cadence"), QJsonDocument::fromJson(data.toUtf8()).object());
            }) }.to_string()
        };
        assert_eq!(painter_value(recorded), "115 步/分");
        for values in [telemetry_values(&s, Some(&track), 26_000.0),
            telemetry_values(&s, Some(&track), 30_000.0), telemetry_values(&s, None, 25_000.0)] {
            assert!(values["cadence"].is_null());
            assert_eq!(painter_value(values), "--");
        }
        assert_eq!(painter_value(serde_json::json!({"cadence":0})), "0 步/分");
    }

    #[test]
    fn cadence_layout_accepts_digital_and_nine_classic_fields() {
        let mut s = Settings::default();
        s.hud.show_cadence = true;
        let classic = Layout::classic(&s);
        assert_eq!(classic.widgets[0].fields.as_ref().unwrap().len(), 9);
        assert_eq!(Layout::parse(&classic.json()).unwrap().json(), classic.json());
        let mut digital = Layout { version: 1, widgets: vec![Widget {
            metric: "cadence".into(), ..Default::default()
        }] };
        assert!(Layout::parse(&digital.json()).is_ok());
        digital.widgets[0].metric = "unknown_metric".into();
        assert!(Layout::parse(&digital.json()).is_err());
    }

    #[test]
    fn layout_roundtrip_and_validation() {
        for theme in ["campus", "park", "simple", "modern", "classic", "outdoor"] {
            let layout = Layout::theme(theme);
            assert_eq!(Layout::parse(&layout.json()).unwrap().json(), layout.json());
        }
        let mut l = Layout::theme("modern");
        l.widgets[1].id = l.widgets[0].id.clone();
        assert!(Layout::parse(&l.json()).is_err());
        assert!(Layout::parse(r#"{"version":2,"widgets":[]}"#).is_err());
        assert!(Layout::parse(r#"{"version":1,"widgets":[]}"#).is_ok());
        let mut settings = Settings::default();
        settings.hud.show_speed = false;
        let classic = Layout::classic(&settings);
        let fields = classic.widgets[0].fields.as_ref().unwrap();
        assert!(!fields.contains(&"speed".to_string()));
        assert!(fields.contains(&"elapsed".to_string()));
    }
    #[test]
    fn templates_do_not_carry_media_and_survive_rename() {
        let dir = std::env::temp_dir().join(format!("ov-hud-test-{}", std::process::id()));
        let l = Layout::theme("outdoor").json();
        template_action_in(&dir, "save", "", "户外测试", &l).unwrap();
        let path = std::fs::read_dir(&dir)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let id = path.file_name().unwrap().to_str().unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("gpx"));
        assert!(!raw.contains("offset"));
        assert!(!raw.contains("video"));
        template_action_in(&dir, "rename", id, "新名字", "").unwrap();
        assert_eq!(template_action_in(&dir, "load", id, "", "").unwrap(), l);
        assert!(template_action_in(&dir, "delete", "../bad.json", "", "").is_err());
        template_action_in(&dir, "delete", id, "", "").unwrap();
        std::fs::remove_dir(&dir).unwrap();
    }
}
