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
                if fields.len() > 8
                    || fields.iter().any(|field| {
                        ![
                            "speed",
                            "heading",
                            "elevation",
                            "heart_rate",
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
/// HUD coordinates belong to the fitted video, not the composite map/video canvas.
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
    let left = if map_configured {
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
pub fn draw(
    p: &mut QPainter,
    settings: &Settings,
    track: Option<&GpxTrack>,
    t_ms: f64,
    width: f64,
    height: f64,
) {
    let elapsed = (t_ms / 1000.0 - settings.video.start_time_offset_s).max(0.0);
    let sample = track.and_then(|t| t.sample_at(t_ms / 1000.0, settings.hud.gpx_time_offset));
    let ht = track.and_then(|t| t.hud_at(t_ms / 1000.0, settings.hud.gpx_time_offset));
    let raw = settings
        .hud
        .layout
        .as_ref()
        .map(|l| l.json())
        .unwrap_or_else(|| Layout::classic(settings).json());
    let layout = QString::from(raw.as_str());
    let values = serde_json::json!({
        "speed":sample.map(|s|s.speed_kmh), "heading":ht.and_then(|v|v.0),
        "elevation":sample.and_then(|s|s.altitude_m),"heart_rate":sample.and_then(|s|s.heart_rate),
        "distance":sample.map(|s|s.distance_km),"pace":sample.map(|s| if s.pace_min_km>0.0 {format!("{} /km",duration(s.pace_min_km*60.0))} else {"-- /km".into()}),
        "elapsed":duration(elapsed),"segment":segment(elapsed,settings),
        "phase":ht.map(|v|v.1).unwrap_or(0.0),"max_speed":track.map(|t|t.hud_max_speed()).unwrap_or(30.0),
        "track_time":t_ms/1000.0+settings.hud.gpx_time_offset,
        "classic_flags":[settings.hud.show_elapsed_time,settings.hud.show_segment_time,settings.hud.show_speed,
            settings.hud.show_pace,settings.hud.show_heart_rate,settings.hud.show_distance,settings.hud.show_azimuth,settings.hud.show_elevation]
    });
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
