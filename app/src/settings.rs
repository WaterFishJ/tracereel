//! Application settings: the in-memory model plus persistence to a `.trr` project (legacy `.orv` is also supported).
//!
//! The `.trr` / `.orv` file is a QSettings-style INI document (the previous PyQt implementation
//! stored colors as Qt's `@Variant(...)`-wrapped strings; we keep the same key names so
//! existing project files load unchanged). Keys are grouped by the same path prefixes the
//! old build used (e.g. `map/imageFilePath`, `route/quickRouteJpegFilePath`).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Parse a color string (hex `#rrggbb`, `#rrggbbaa`, or `rr,gg,bb[,aa]`) into a canonical
/// `#rrggbb` / `#rrggbbaa` hex for storage; falls back to a given default if unparseable.
///
/// 8 位十六进制一律按本程序自己的约定解释为 **RRGGBBAA**（末尾两位是不透明度），这与
/// QML 颜色字面量 `"#AARRGGBB"` 的顺序**相反**：写进 QML 或手输颜色时必须经过
/// `app/src/bin/gui.qml` 的 `qtColor()` / `colorHex()` 转换。全不透明时收敛成 6 位，
/// 于是既有的 .orv（只有 6 位颜色）读写成原文不变。
pub fn color_to_hex(c: &str, default: &str) -> String {
    let hex = c.trim().trim_start_matches('#');
    if hex.is_empty() {
        return default.to_string();
    }
    if hex.contains(',') {
        // 只认十进制 "rr,gg,bb[,aa]"；其它带逗号的串（如 Qt 的 @Variant(...) 二进制垃圾）一律回退。
        let parts: Vec<&str> = hex.split(',').map(|s| s.trim()).collect();
        let decimal = parts.len() >= 3
            && parts.len() <= 4
            && parts
                .iter()
                .all(|p| !p.is_empty() && p.chars().all(|ch| ch.is_ascii_digit()));
        if decimal {
            let a = match parts.get(3) {
                Some(s) => format_hex(s),
                None => "ff".to_string(),
            };
            return pack_rgba(
                &format_hex(parts[0]),
                &format_hex(parts[1]),
                &format_hex(parts[2]),
                &a,
            );
        }
        return default.to_string();
    }
    if !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return default.to_string();
    }
    match hex.len() {
        6 => format!("#{}", hex.to_ascii_lowercase()),
        8 => pack_rgba(&hex[0..2], &hex[2..4], &hex[4..6], &hex[6..8]),
        _ => default.to_string(),
    }
}

/// Assemble a canonical `#rrggbb[aa]` string (alpha suffix only when not fully opaque).
fn pack_rgba(r: &str, g: &str, b: &str, a: &str) -> String {
    let rgb = format!("#{}{}{}", r, g, b).to_ascii_lowercase();
    if a.eq_ignore_ascii_case("ff") {
        rgb
    } else {
        format!("{}{}", rgb, a.to_ascii_lowercase())
    }
}

/// `a` 为空串时回退到 `b`（读新颜色键时的老键兜底）。
fn non_empty(a: String, b: &str) -> String {
    if a.trim().is_empty() {
        b.to_string()
    } else {
        a
    }
}

/// `#rrggbb[aa]` → `#rrggbb`：界面上的颜色项只存 RGB，不透明度是独立的设置项。
fn rgb_of(hex: &str) -> String {
    let t = hex.trim_start_matches('#');
    if t.len() >= 6 && t[..6].chars().all(|c| c.is_ascii_hexdigit()) {
        format!("#{}", &t[..6]).to_ascii_lowercase()
    } else {
        "#000000".to_string()
    }
}

/// `#rrggbbaa` 的末尾两位（0..255）；只有 6 位（或不合法）时用 `default`。
fn alpha_of(hex: &str, default: f64) -> f64 {
    let t = hex.trim_start_matches('#');
    if t.len() == 8 {
        if let Ok(a) = u8::from_str_radix(&t[6..8], 16) {
            return a as f64;
        }
    }
    default
}

/// 反向：`#rrggbb` + 不透明度（0..255）→ 原版 C++ 认的 `#rrggbb[aa]`（不透明时省掉后缀）。
fn legacy_color(hex: &str, opacity: f64) -> String {
    let a = opacity.round().clamp(0.0, 255.0) as u8;
    if a == 255 {
        rgb_of(hex)
    } else {
        format!("{}{:02x}", rgb_of(hex), a)
    }
}

/// Convert a QML-provided file path/URL into a plain native path: strips the `file://`
/// scheme, removes the leading slash before a Windows drive letter, and percent-decodes
/// UTF-8 sequences (Qt encodes non-ASCII characters as %XX in urls).
pub fn decode_path(p: &str) -> String {
    let mut t = p.trim_start_matches("file://");
    if t.len() > 2 && t.as_bytes()[0] == b'/' && t.as_bytes()[2] == b':' {
        t = &t[1..];
    }
    let bytes = t.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&t[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn format_hex(s: &str) -> String {
    if let Ok(v) = s.parse::<u32>() {
        return format!("{:02x}", v.min(255));
    }
    "00".to_string()
}

/// A raw key/value pair read from an INI section.
#[derive(Debug, Clone, Default)]
pub struct IniFile {
    /// section -> (key -> value)
    sections: BTreeMap<String, BTreeMap<String, String>>,
}

impl IniFile {
    pub fn parse(text: &str) -> Self {
        let mut sections: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        let mut cur = String::new();
        for line in text.lines() {
            let line = line.trim_end();
            if line.is_empty()
                || line.starts_with(';')
                || line.starts_with('#')
                || line.starts_with('[') && line.ends_with(']') == false && line.starts_with('#')
            {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                cur = line[1..line.len() - 1].trim().to_string();
                sections.entry(cur.clone()).or_default();
                continue;
            }
            if let Some(eq) = line.find('=') {
                let key = line[..eq].trim().to_string();
                let val = line[eq + 1..].trim().to_string();
                sections.entry(cur.clone()).or_default().insert(key, val);
            }
        }
        Self { sections }
    }

    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.sections
            .get(section)
            .and_then(|m| m.get(key))
            .map(|s| s.as_str())
    }

    pub fn get_string(&self, section: &str, key: &str, def: &str) -> String {
        self.get(section, key)
            .map(|s| s.to_string())
            .unwrap_or_else(|| def.to_string())
    }

    pub fn get_f64(&self, section: &str, key: &str, def: f64) -> f64 {
        self.get(section, key)
            .and_then(|s| s.parse().ok())
            .unwrap_or(def)
    }

    pub fn get_bool(&self, section: &str, key: &str, def: bool) -> bool {
        self.get(section, key)
            .map(|s| {
                matches!(
                    s.trim().to_ascii_lowercase().as_str(),
                    "true" | "1" | "yes" | "on"
                )
            })
            .unwrap_or(def)
    }

    pub fn set(&mut self, section: &str, key: &str, value: &str) {
        self.sections
            .entry(section.to_string())
            .or_default()
            .insert(key.to_string(), value.to_string());
    }

    pub fn to_ini_string(&self) -> String {
        let mut out = String::new();
        for (section, kv) in &self.sections {
            out.push('[');
            out.push_str(section);
            out.push_str("]\r\n");
            for (k, v) in kv {
                out.push_str(k);
                out.push('=');
                out.push_str(v);
                out.push_str("\r\n");
            }
            out.push_str("\r\n");
        }
        out
    }
}

/// Video display / playback settings.
#[derive(Debug, Clone)]
pub struct VideoSettings {
    pub video_path: String,
    /// 同一场定向的续接分段（运动相机自动分段）：按录制顺序排在 video_path 之后，
    /// 参数（分辨率/帧率）与首段一致，各自带运动数据。预览时间轴 = 各段之和。
    pub extra_videos: Vec<String>,
    /// 「录像-出发时间」= 出发时刻在视频时间轴上的秒数。出发前几十秒开始录像则为正值（例：40）。
    pub start_time_offset_s: f64,
    pub loop_playback: bool,
    pub rescale_shader: String, // default | 双线性 | 双三次插值
    pub enable_verbose_logging: bool,
    pub seek_to_any_frame: bool,
    pub frame_count_divisor: i32,
    pub frame_duration_divisor: i32,
    /// Panel position/transform (video panel).
    pub x: f64,
    pub y: f64,
    pub angle: f64,
    pub scale: f64,
}

impl VideoSettings {
    /// 全部视频文件（首段 + 续接分段），过滤空路径，保持录制顺序。
    pub fn all_paths(&self) -> Vec<String> {
        let mut v = Vec::with_capacity(1 + self.extra_videos.len());
        if !self.video_path.is_empty() {
            v.push(self.video_path.clone());
        }
        v.extend(self.extra_videos.iter().filter(|p| !p.is_empty()).cloned());
        v
    }
}

/// Map (background image) settings.
#[derive(Debug, Clone)]
pub struct MapSettings {
    pub image_file_path: String,
    /// 额外地图（一场定向多张图）：按换图时间排序排在主图之后，每个文件同样是
    /// QuickRoute JPEG（地图+轨迹合一）。'|' 拼接存储，仿 `VideoSettings::extra_videos`。
    pub extra_map_files: Vec<String>,
    /// 换图时间（秒，从出发时刻算起）：`extra_map_change_times[i]` 对应
    /// `extra_map_files[i]`。出发时刻 + 换图时间即换图那一刻，底图切换为该地图，
    /// 其轨迹从该时刻开始绘制。
    ///
    /// **`None` = 还没设置换图时间** —— 那张图暂不参与播放（主图继续显示），
    /// 而不是"从 0 秒开始接管整条时间轴"。不这样区分的话，忘填换图时间的地图
    /// 会与主图同为 0 ms 起点，定位规则取后者，于是从头到尾都显示那张图。
    pub extra_map_change_times: Vec<Option<f64>>,
    pub relative_width: f64,      // 0..1, default 0.25
    pub scale: f64,               // initial scale
    pub header_crop: i32,         // pixels cropped from the top of the map image
    pub background_color: String, // hex "#rrggbb"
    pub x: f64,
    pub y: f64,
    pub user_x: f64,
    pub user_y: f64,
    pub angle: f64,
    pub user_angle: f64,
    pub user_scale: f64,
}

impl MapSettings {
    /// 额外地图 (路径, 换图时间) 对齐序列：跳过空路径；换图时间为 `None`（没填）
    /// 的条目也会返回，调用方据此决定"暂不排程"。
    pub fn extra_maps(&self) -> Vec<(String, Option<f64>)> {
        let mut out = Vec::new();
        for (i, p) in self.extra_map_files.iter().enumerate() {
            if p.trim().is_empty() {
                continue;
            }
            let t = self.extra_map_change_times.get(i).copied().flatten();
            out.push((p.clone(), t));
        }
        out
    }
}

/// Route (QuickRoute jpeg) settings.
#[derive(Debug, Clone)]
pub struct RouteSettings {
    pub quick_route_jpeg_file_path: String,
    pub user_scale: f64,
    /// Render mode: 0 = 无, 1 = 简洁, 3 = 配速
    ///
    /// 沿用**原版 C++ 的编号**（原版 0=无、1=简洁、2=高亮、3=配速）——本程序去掉了「高亮」，
    /// 但 2 号继续空着，于是同一个 `.orv` 在原版里打开时渲染模式仍然一致。
    pub render_mode: i32,
    /// Tail render mode: 0 = 无, 1 = 简洁（原版 2 = 高亮，读入时按「简洁」处理）。
    pub tail_render_mode: i32,
    /// 渲染范围：0 = 完整（整条路线，默认），1 = 单段（只画当前所在的这一分段）。
    ///
    /// 「当前段」= 当前时刻落在哪个分段区间（控制点 i → 控制点 i+1），判定与取景用的
    /// `RouteManager::find_index` 一致；出发前按第一段、冲线后按最后一段画。
    pub render_scope: i32,
    pub width: f64,       // route width in pixels
    pub tail_width: f64,  // tail width in pixels
    pub tail_length: f64, // tail length in seconds (reused as route.tail_length)
    pub control_radius: f64,
    pub runner_radius: f64,
    pub show_controls: bool,
    pub show_runner: bool,
    /// 路线颜色（简洁模式），hex `#rrggbb`。**不透明度不在颜色串里**，见 `route_opacity`。
    pub route_color: String,
    /// 路线不透明度 0..255（与原版 QColor alpha 同一量纲）。
    pub route_opacity: f64,
    /// 尾巴颜色（简洁模式），hex `#rrggbb`。
    pub tail_color: String,
    /// 尾巴不透明度 0..255。
    pub tail_opacity: f64,
    /// 控制点颜色，hex `#rrggbb`。
    pub control_color: String,
    /// 控制点不透明度 0..255。
    pub control_opacity: f64,
    /// 运动员颜色，hex `#rrggbb`。
    pub runner_color: String, // hex
    /// 运动员不透明度 0..255。
    pub runner_opacity: f64,
    /// 运动员样式：0 = 圆形（原版行为）、1 = 箭头（跟着运动方向转）。
    pub runner_style: i32,
    /// 配速渲染的色带上限/下限（分钟/公里）：低配速（慢）→ 红，高配速（快）→ 绿。
    pub low_pace: f64,
    pub high_pace: f64,
    /// 配速色带是否按**实际轨迹**的配速分布自适应（开启时上面两项由轨迹算出来）。
    pub pace_auto: bool,
    /// 尾巴独立的配速色带设置（慢端红色、快端绿色）。
    pub tail_low_pace: f64,
    pub tail_high_pace: f64,
    pub tail_pace_auto: bool,
    pub frame_readout: f64,
    /// View mode: 0 = FIXED_SPLIT, 1 = RUNNER_CENTERED, 2 = RUNNER_CENTERED_SPLIT_ORIENTED
    pub view_mode: i32,
    pub runner_averaging_factor: f64,
    pub runner_time_offset: f64, // legacy .orv field; replaced by the derived model (see gui.rs)
    pub control_time_offset: f64, // 按表-出发时间（秒）：出发前按表为负，出发后按表为正
    pub left_right_margin: f64,
    pub top_bottom_margin: f64,
}

/// Splits (time markers) settings.
#[derive(Debug, Clone)]
pub struct SplitSettings {
    pub split_times: String, // "1:23|2:55|1:13:22"
    /// Optional control codes, aligned with split_times; empty slots are retained.
    pub point_codes: String,
    pub is_relative: bool,   // false = absolute, true = relative accumulation
}

/// HUD (GPX overlay) settings.
#[derive(Debug, Clone)]
pub struct HudSettings {
    pub layout: Option<crate::hud::Layout>,
    /// HUD 的绘制范围：`"video"`（跟随视频画面，默认，兼容旧工程）/ `"frame"`（整幅画面，
    /// 可以摆到地图那半边）。没有视频面板时（只导地图+HUD，或预览里取消勾选「视频」）
    /// 不论取哪个值都按整幅画面画 —— 否则 HUD 会被挤进宽度为 0 的视频区而整块消失。
    pub scope: String,
    pub gpx_file_path: String,
    pub gpx_time_offset: f64,
    pub show_elapsed_time: bool,
    pub show_segment_time: bool,
    pub show_speed: bool,
    pub show_pace: bool,
    pub show_heart_rate: bool,
    pub show_distance: bool,
    pub show_azimuth: bool,
    pub show_elevation: bool,
}

/// Encoder (encode page) settings.
#[derive(Debug, Clone)]
pub struct EncodeSettings {
    pub backend: String,      // software|qsv|nvenc|amf|videotoolbox
    pub format: String,       // h264|hevc|mpeg4|av1
    pub rate_control: String, // crf|cbr|vbr
    pub quality: f64,         // CRF value
    pub bitrate: String,      // e.g. "20M"
    pub preset: String,
    /// Export playback speed multiplier: "0.8" | "1" | "1.5" | "2" | "3" | "4" (default "1").
    /// The frame loop samples source time N times faster (output fps unchanged) and the
    /// audio is time-compressed with an atempo chain so pitch is preserved.
    pub speed: String,
    pub audio_codec: String, // copy|none|aac|mp3|opus
    pub audio_bitrate: String,
    pub clip_start_ms: f64,
    pub clip_end_ms: f64,
    /// 「自定义时间」自己填的起止（毫秒，0 = 开头 / 到末尾）。
    /// 独立于 `clip_start_ms`/`clip_end_ms`：那一对是**导出时**按模式推导出来的结果，
    /// 切到「全部」会被归零；这里存的是用户填的值，切来切去都还在。
    pub range_start_ms: f64,
    pub range_end_ms: f64,
    /// 导出范围（界面「导出范围」下拉的唯一真源）：
    /// `"none"` 全部（整段导出，`clip_*` 归零）| `"all"` 逐段导出（每段一个文件）|
    /// `"custom"` 自定义分段（按 `start_seg`/`end_seg` 推导 `clip_*`）|
    /// `"time"` 自定义时间（`clip_start_ms`/`clip_end_ms` 就是界面上填的时间，不做推导）。
    /// 旧文件没有这个键时按 `all_seg_enabled`/`seg_enabled` 反推（见 `from_ini`）。
    pub clip_mode: String,
    /// Segment-based export boundaries. When `seg_enabled`, `clip_start_ms`/`clip_end_ms`
    /// are DERIVED from these point indices: `start_seg = a` → export begins at split point a
    /// (0 = the start line / departure time); `end_seg = b` → export ends at split point b
    /// (0 = until the video tail). Both 0 (or `seg_enabled = false`) → full export.
    pub seg_enabled: bool,
    pub start_seg: i32,
    pub end_seg: i32,
    /// 「全部逐段导出」：与 `seg_enabled` 同时勾选时，导出不再使用 `start_seg`/`end_seg`，
    /// 而是把每个分段单独导成一个文件；输出文件名在扩展名前插入 `[起-止]`
    /// （如 `test.mp4` → `test[0-1].mp4`、`test[1-2].mp4` …）。见 `encode::encode_export_segments`。
    pub all_seg_enabled: bool,
    pub output_file_path: String,
    /// Output resolution: "source"（与原始视频一致，输出画布 = 源视频尺寸）| 显式 "宽x高"
    /// （如 "1920x1080"）；更早的档位名（"1080p" | "720p" | "4k"）仍由
    /// `encode::encode_output_size` 兜底解析。
    /// When not "source", the composite is re-laid-out at that size; the video itself is
    /// never stretched (aspect is preserved by letterboxing in `draw_scene`).
    pub resolution: String,
    /// Output frame rate: "source" | "60" | "30" | "24" | a raw number.
    pub frame_rate: String,
    /// Export the map + route + runner as a full-frame video with NO source video
    /// (for users who only have a map/track). Audio and stabilization are skipped; the
    /// timeline is the route duration. See `encode::encode_export`.
    pub map_only: bool,
    /// Independently selected output layers. `map_only` is the derived route-clock mode.
    pub include_map: bool,
    pub include_video: bool,
    pub include_hud: bool,
}

/// Gyroflow stabilization settings.
///
/// **所有稳定化参数都在这里**（不再依赖 `.gyroflow` 文件）：GUI 的「稳定化」页直接编辑这些字段，
/// 渲染时由 `gyro::load_stabilizer` 全部应用到 `StabilizationManager`。选中/拖入一个 `.gyroflow`
/// 只是把它 `stabilization` 段的值**导入**到这些字段（`apply_preset_json`），之后照常可改。
#[derive(Debug, Clone)]
pub struct GyroSettings {
    pub enabled: bool,
    /// `.gyroflow` 预设文件路径（提供镜头标定 + 稳定化参数的初始值）。
    pub preset_file_path: String,
    /// 搜索框选中的镜头标定：Gyroflow 镜头库的条目 key（见 `crate::lens_db`），
    /// 或用户给的 `.json` 标定文件路径。非空时**覆盖** `.gyroflow` 预设里的 `calibration_data`。
    pub lens_profile: String,
    /// 上一条的显示名（库 key 对人不可读，UI 直接显示这个）。
    pub lens_profile_name: String,

    // ---- 平滑 ----
    /// 平滑算法名（`Default` / `Plain 3D` / `Fixed` / `None`，见 `StabilizationManager::get_smoothing_algs`）。
    pub smoothing_method: String,
    /// 该算法的参数，JSON 文本 `[{"name":"smoothness","value":1}, …]`（与 `.gyroflow` 同格式）。
    pub smoothing_params: String,

    // ---- 视野 / 变焦 ----
    pub fov: f64,
    /// 变焦方式（= gyroflow 的 `adaptive_zoom_method`）：0=不变焦 1=动态变焦 2=静态变焦。
    pub zoom_mode: i32,
    /// 变焦速度（秒）= `adaptive_zoom_window`；0 = 尽量快。
    pub adaptive_zoom_window: f64,
    pub max_zoom: f64,
    pub max_zoom_iterations: i32,
    pub zoom_center_x: f64,
    pub zoom_center_y: f64,

    // ---- 镜头 / 地平线 ----
    pub lens_correction_amount: f64,
    pub horizon_lock: f64,
    pub horizon_lock_roll: f64,
    pub use_gravity_vectors: bool,

    // ---- 卷帘快门 ----
    pub frame_readout_time: f64,
    pub frame_readout_direction: String,

    // ---- 视频速度联动 ----
    pub video_speed: f64,
    pub video_speed_affects_smoothing: bool,
    pub video_speed_affects_zooming: bool,
    pub video_speed_affects_zooming_limit: bool,

    // ---- 附加 3D 旋转 / 平移 ----
    pub additional_rotation: [f64; 3],
    pub additional_translation: [f64; 3],

    // ---- 地平线锁定（对齐原版：锁定地平线 / 锁定量 / 横滚角度校正 / 锁定俯仰角 / 俯仰角度校正 /
    //      自动锁定（转弯阈值·转弯平滑·转弯倍率·限制倾斜加速度） / 积分方法） ----
    /// 「锁定地平线」总开关：关掉时送给核心的锁定量为 0（原版就是这个语义）。
    pub horizon_lock_enabled: bool,
    /// 「自动锁定」：勾上后锁定量按 100 送，转弯参数生效。
    pub horizon_automatic_lock: bool,
    pub horizon_lock_pitch_enabled: bool,
    pub horizon_lock_pitch: f64,
    /// 原版「积分方法」= 地平线锁定的 IMU 积分方式（0 互补 / 1 VQF / 2 陀螺+加速度计 / 3 Mahony / 4 Madgwick）。
    pub horizon_lock_integration_method: i32,
    pub horizon_turn_threshold: f64,
    pub horizon_turn_smoothing_ms: f64,
    pub horizon_turn_multiplier: f64,
    pub horizon_limit_tilt_accel: bool,
    pub horizon_tilt_accel_limit: f64,

    // ---- 卷帘快门校正开关（原版是一个勾选框包住帧读出时间） ----
    pub frame_readout_enabled: bool,

    // ---- 稳定焦距（原版 Stabilize focal length；只有逐帧焦距的视频才有意义） ----
    pub focal_length_smoothing_enabled: bool,
    pub focal_length_smoothing_strength: f64,

    // ---- 同步 ----
    /// Manual gyro<->video time offset in milliseconds (fine-tunes the residual
    /// high-frequency jitter; DJI GPS-synced data is ~0, autosync found -3.6ms noise).
    pub offset_ms: f64,
    /// Manual whole-frame offset. When non-zero this takes precedence over offset_ms.
    pub offset_frames: i32,
}

/// `Default` 平滑算法的出厂参数（TraceReel 的内置默认值，不依赖任何外部预设文件）。
pub const DEFAULT_SMOOTHING_PARAMS: &str = r#"[{"name":"smoothness","value":1},{"name":"smoothness_pitch","value":0.5},{"name":"smoothness_yaw","value":0.5},{"name":"smoothness_roll","value":0.5},{"name":"per_axis","value":0},{"name":"trim_range_only","value":0},{"name":"max_smoothness","value":2.5},{"name":"alpha_0_1s","value":1}]"#;

/// Root settings object.
#[derive(Debug, Clone)]
pub struct Settings {
    pub video: VideoSettings,
    pub map: MapSettings,
    pub route: RouteSettings,
    pub splits: SplitSettings,
    pub hud: HudSettings,
    pub encode: EncodeSettings,
    pub gyro: GyroSettings,
    /// Window geometry (x, y, w, h).
    pub window: (i32, i32, i32, i32),
    pub window_fullscreen: bool,
    pub window_hide_cursor: bool,
    pub window_multisamples: i32,
    /// Preview performance mode: when true, the live preview uses a lower resolution
    /// cap and skips Gyroflow stabilization per frame (encode still uses full settings).
    pub preview_perf: bool,
    /// 界面外观：`light` / `dark` / `system`（对齐原版 Gyroflow 的主题设置）。
    pub theme: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            video: VideoSettings {
                video_path: String::new(),
                extra_videos: Vec::new(),
                start_time_offset_s: 0.0,
                loop_playback: false,
                rescale_shader: "default".to_string(),
                enable_verbose_logging: false,
                seek_to_any_frame: false,
                frame_count_divisor: 1,
                frame_duration_divisor: 1,
                x: 0.0,
                y: 0.0,
                angle: 0.0,
                scale: 1.0,
            },
            map: MapSettings {
                image_file_path: String::new(),
                extra_map_files: Vec::new(),
                extra_map_change_times: Vec::new(),                relative_width: 0.25,
                scale: 1.0,
                header_crop: 0,
                background_color: "#ffffff".to_string(),
                x: 0.0,
                y: 0.0,
                user_x: 0.0,
                user_y: 0.0,
                angle: 0.0,
                user_angle: 0.0,
                user_scale: 1.0,
            },
            route: RouteSettings {
                quick_route_jpeg_file_path: String::new(),
                user_scale: 1.0,
                render_mode: 0,
                tail_render_mode: 3,
                render_scope: 0,
                width: 10.0,
                tail_width: 10.0,
                tail_length: 30.0,
                control_radius: 15.0,
                runner_radius: 15.0,
                show_controls: false,
                show_runner: true,
                // 显示与取景默认值参考仓库内的 demo/example.trr；控制点为不透明黑。
                // 颜色与不透明度拆成两项之后，这里的 opacity 就是原版那个 alpha（0..255）。
                route_color: "#000000".to_string(),
                route_opacity: 80.0,
                tail_color: "#000000".to_string(),
                tail_opacity: 204.0,
                control_color: "#000000".to_string(),
                control_opacity: 255.0,
                runner_color: "#ff5d00".to_string(),
                runner_opacity: 255.0,
                runner_style: 1,
                low_pace: 15.0,
                high_pace: 5.0,
                // 与原版不同的刻意的默认：原版固定 15/5，本程序默认按实际轨迹自适应，
                // 否则绝大多数真实轨迹都会挤在色带某一端、看起来像单色。
                pace_auto: true,
                tail_low_pace: 16.204162961116385,
                tail_high_pace: 5.117530921453715,
                tail_pace_auto: true,
                frame_readout: 0.0,
                view_mode: 0,
                runner_averaging_factor: 0.5,
                runner_time_offset: 0.0,
                control_time_offset: 0.0,
                left_right_margin: 20.0,
                top_bottom_margin: 20.0,
            },
            splits: SplitSettings {
                split_times: String::new(),
                point_codes: String::new(),
                is_relative: false,
            },
            hud: HudSettings {
                layout: Some(crate::hud::Layout::theme("campus")),
                scope: "video".into(),
                gpx_file_path: String::new(),
                gpx_time_offset: 0.0,
                show_elapsed_time: true,
                show_segment_time: true,
                show_speed: true,
                show_pace: true,
                show_heart_rate: true,
                show_distance: true,
                show_azimuth: true,
                show_elevation: true,
            },
            encode: EncodeSettings {
                backend: "software".to_string(),
                format: "h264".to_string(),
                rate_control: "crf".to_string(),
                quality: 21.0,
                bitrate: "20M".to_string(),
                preset: "medium".to_string(),
                speed: "1".to_string(),
                audio_codec: "copy".to_string(),
                audio_bitrate: "128k".to_string(),
                clip_start_ms: 0.0,
                clip_end_ms: 0.0,
                range_start_ms: 0.0,
                range_end_ms: 0.0,
                clip_mode: "none".to_string(),
                seg_enabled: false,
                start_seg: 0,
                end_seg: 0,
                all_seg_enabled: false,
                output_file_path: String::new(),
                resolution: "1920x1080".to_string(),
                frame_rate: "source".to_string(),
                map_only: false,
                include_map: true,
                include_video: true,
                include_hud: true,
            },
            gyro: GyroSettings::default(),
            window: (0, 0, 1280, 720),
            window_fullscreen: false,
            window_hide_cursor: false,
            window_multisamples: 16,
            preview_perf: false,
            theme: "light".to_string(),
        }
    }
}

impl Default for GyroSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            preset_file_path: String::new(),
            lens_profile: String::new(),
            lens_profile_name: String::new(),
            smoothing_method: "Default".to_string(),
            smoothing_params: DEFAULT_SMOOTHING_PARAMS.to_string(),
            fov: 0.85,
            zoom_mode: 0,
            adaptive_zoom_window: 0.0,
            max_zoom: 130.0,
            max_zoom_iterations: 5,
            zoom_center_x: 0.0,
            zoom_center_y: 0.0,
            lens_correction_amount: 0.0,
            horizon_lock: 0.0,
            horizon_lock_roll: 0.0,
            // 默认「锁定地平线」是开的、锁定量由滑杆决定（旧工程只有 horizonLock 值时行为不变）。
            horizon_lock_enabled: true,
            horizon_automatic_lock: false,
            horizon_lock_pitch_enabled: false,
            horizon_lock_pitch: 0.0,
            horizon_lock_integration_method: 0,
            horizon_turn_threshold: 5.0,
            horizon_turn_smoothing_ms: 500.0,
            horizon_turn_multiplier: 1.0,
            horizon_limit_tilt_accel: false,
            horizon_tilt_accel_limit: 3.0,
            // 旧工程只要有 frameReadoutTime 就继续用它（勾选默认开着，与旧行为一致）。
            frame_readout_enabled: true,
            focal_length_smoothing_enabled: false,
            focal_length_smoothing_strength: 30.0,
            use_gravity_vectors: false,
            frame_readout_time: 11.09,
            frame_readout_direction: "TopToBottom".to_string(),
            video_speed: 1.0,
            video_speed_affects_smoothing: true,
            video_speed_affects_zooming: true,
            video_speed_affects_zooming_limit: true,
            additional_rotation: [0.0; 3],
            additional_translation: [0.0; 3],
            offset_ms: 0.0,
            offset_frames: 0,
        }
    }
}

impl GyroSettings {
    /// 把 `.gyroflow` 的 `stabilization` 段导入这些字段（GUI 选中/拖入预设、CLI 用它）。
    /// 只覆盖预设里**存在**的字段，缺的保持原值 —— 于是「先选预设、再手改一两项」是安全的。
    pub fn apply_preset_json(&mut self, preset: &serde_json::Value) {
        let Some(s) = preset.get("stabilization") else {
            return;
        };
        let f = |k: &str| s.get(k).and_then(|v| v.as_f64());
        let i = |k: &str| s.get(k).and_then(|v| v.as_i64()).map(|v| v as i32);
        let b = |k: &str| s.get(k).and_then(|v| v.as_bool());
        if let Some(v) = f("fov") {
            self.fov = v;
        }
        if let Some(v) = i("adaptive_zoom_method") {
            self.zoom_mode = v;
        }
        if let Some(v) = f("adaptive_zoom_window") {
            self.adaptive_zoom_window = v;
        }
        // `adaptive_zoom_window <= 0` 在 gyroflow_core 里表示「关闭变焦」（< -0.9 才是静态缩放）。
        // 模式选了「动态缩放」时必须给一个正窗口，否则面板看着是动态、核心按关闭处理（变焦失效）。
        if self.zoom_mode == 1 && self.adaptive_zoom_window <= 0.0001 {
            self.adaptive_zoom_window = 4.0;
        }
        // 静态缩放：window 由 effective_zoom_window() 统一翻译成 -1.0，这里把速度存值归到默认，
        // 免得面板滑杆显示 -1 这种无意义的值。
        if self.zoom_mode == 2 && self.adaptive_zoom_window <= 0.0001 {
            self.adaptive_zoom_window = 4.0;
        }
        if let Some(v) = f("max_zoom") {
            self.max_zoom = v;
        }
        if let Some(v) = i("max_zoom_iterations") {
            self.max_zoom_iterations = v.max(1);
        }
        if let Some(a) = s.get("adaptive_zoom_center_offset").and_then(|v| v.as_array()) {
            self.zoom_center_x = a.first().and_then(|v| v.as_f64()).unwrap_or(self.zoom_center_x);
            self.zoom_center_y = a.get(1).and_then(|v| v.as_f64()).unwrap_or(self.zoom_center_y);
        }
        if let Some(v) = f("lens_correction_amount") {
            self.lens_correction_amount = v;
        }
        if let Some(v) = f("horizon_lock_amount") {
            self.horizon_lock = v;
        }
        if let Some(v) = f("horizon_lock_roll") {
            self.horizon_lock_roll = v;
        }
        if let Some(v) = b("use_gravity_vectors") {
            self.use_gravity_vectors = v;
        }
        // ---- 地平线锁定进阶 / 卷帘开关 / 焦距平滑（键名与原版 .gyroflow 完全一致） ----
        if let Some(v) = b("horizon_lock_pitch_enabled") {
            self.horizon_lock_pitch_enabled = v;
        }
        if let Some(v) = f("horizon_lock_pitch") {
            self.horizon_lock_pitch = v;
        }
        if let Some(v) = i("horizon_lock_integration_method") {
            self.horizon_lock_integration_method = v;
        }
        if let Some(v) = b("focal_length_smoothing_enabled") {
            self.focal_length_smoothing_enabled = v;
        }
        if let Some(v) = f("focal_length_smoothing_strength") {
            self.focal_length_smoothing_strength = v;
        }
        // 原版预设里只要写了非 0 的 frame_readout_time，就视为「卷帘校正开着」。
        if let Some(v) = f("frame_readout_time") {
            if v.abs() > 0.0 {
                self.frame_readout_enabled = true;
            }
        }
        if let Some(v) = f("frame_readout_time") {
            self.frame_readout_time = v;
        }
        if let Some(v) = s.get("frame_readout_direction").and_then(|v| v.as_str()) {
            self.frame_readout_direction = v.to_string();
        }
        if let Some(v) = s.get("method").and_then(|v| v.as_str()) {
            self.smoothing_method = v.to_string();
        }
        if let Some(v) = s.get("smoothing_params") {
            if v.is_array() {
                self.smoothing_params = v.to_string();
            }
        }
        if let Some(v) = f("video_speed") {
            self.video_speed = v;
        }
        if let Some(v) = b("video_speed_affects_smoothing") {
            self.video_speed_affects_smoothing = v;
        }
        if let Some(v) = b("video_speed_affects_zooming") {
            self.video_speed_affects_zooming = v;
        }
        if let Some(v) = b("video_speed_affects_zooming_limit") {
            self.video_speed_affects_zooming_limit = v;
        }
        if let Some(a) = s.get("additional_rotation").and_then(|v| v.as_array()) {
            for (n, dst) in self.additional_rotation.iter_mut().enumerate() {
                if let Some(v) = a.get(n).and_then(|v| v.as_f64()) {
                    *dst = v;
                }
            }
        }
        if let Some(a) = s.get("additional_translation").and_then(|v| v.as_array()) {
            for (n, dst) in self.additional_translation.iter_mut().enumerate() {
                if let Some(v) = a.get(n).and_then(|v| v.as_f64()) {
                    *dst = v;
                }
            }
        }
    }
}

impl Settings {
    /// Load settings from a `.trr` or legacy `.orv` INI file. Missing/null values fall back to defaults.
    /// Relative paths are resolved against the project file's own directory.
    pub fn load_orv(path: &str) -> Self {
        let text = fs::read_to_string(path).unwrap_or_default();
        let ini = IniFile::parse(&text);
        let mut s = Self::from_ini(&ini);
        if !path.is_empty() {
            if let Some(dir) = Path::new(path).parent() {
                s.resolve_orv_paths(dir);
            }
        }
        // 分辨率：下拉是**显式尺寸**（1920x1080 / 3840x2160 / 1920x1440 …）加上一项
        // 「与原始视频一致」（token 就是 `source`）。老工程里的档位名（1080p/720p/4k）归一成
        // 同尺寸的显式写法，否则下拉选不中、显示与实际导出尺寸会不一致。
        // `source` **不归一**：它是下拉里可选的一项，改掉会让用户选好的「与原始视频一致」
        // 在重新打开工程时被悄悄换成 1920x1080（保存 → 加载的往返必须保持原值）。
        s.encode.resolution = match s.encode.resolution.trim().to_ascii_lowercase().as_str() {
            "" | "1080p" | "fhd" => "1920x1080".to_string(),
            "720p" | "hd" => "1280x720".to_string(),
            "4k" | "uhd" | "2160p" => "3840x2160".to_string(),
            _ => s.encode.resolution.clone(),
        };
        // The format combo now offers only H264/HEVC/AV1: legacy "mpeg4" projects
        // export as H264 instead of showing a stale value the UI can no longer display.
        if s.encode.format.eq_ignore_ascii_case("mpeg4") {
            s.encode.format = "h264".to_string();
        }
        // 旧工程（在「稳定化」字段加入之前保存的 .orv）没有 smoothingMethod 等键：
        // 把预设的 stabilization 段导入一次，免得升级后稳定化参数悄悄回到出厂默认。
        if ini.get("gyro", "smoothingMethod").is_none() && !s.gyro.preset_file_path.is_empty() {
            if let Ok(text) = fs::read_to_string(&s.gyro.preset_file_path) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
                    s.gyro.apply_preset_json(&v);
                    log::info!(
                        "[settings] imported stabilization params from preset (legacy project): {}",
                        s.gyro.preset_file_path
                    );
                }
            }
        }
        s
    }

    pub fn from_ini(ini: &IniFile) -> Self {
        let mut s = Self::default();
        s.video.video_path = ini.get_string("video", "videoFilePath", &s.video.video_path);
        // 续接分段用 '|' 拼接（'|' 在 Windows 路径中非法，天然安全分隔符）。
        s.video.extra_videos = ini
            .get("video", "extraVideos")
            .map(|v| {
                v.split('|')
                    .map(|p| p.trim().to_string())
                    .filter(|p| !p.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        s.video.start_time_offset_s =
            ini.get_f64("video", "startTimeOffset", s.video.start_time_offset_s);
        s.video.loop_playback = ini.get_bool("video", "loopPlayback", s.video.loop_playback);
        s.video.rescale_shader = ini.get_string("video", "rescaleShader", &s.video.rescale_shader);
        s.video.enable_verbose_logging = ini.get_bool(
            "video",
            "enableVerboseLogging",
            s.video.enable_verbose_logging,
        );
        s.video.seek_to_any_frame =
            ini.get_bool("video", "seekToAnyFrame", s.video.seek_to_any_frame);
        s.video.frame_count_divisor = ini
            .get("video", "frameCountDivisor")
            .and_then(|v| v.parse().ok())
            .unwrap_or(1);
        s.video.frame_duration_divisor = ini
            .get("video", "frameDurationDivisor")
            .and_then(|v| v.parse().ok())
            .unwrap_or(1);

        s.map.image_file_path = ini.get_string("map", "imageFilePath", &s.map.image_file_path);
        // 额外地图与换图时间用 '|' 拼接（仿 extraVideos），按键名对齐索引。
        s.map.extra_map_files = ini
            .get("map", "extraMapFiles")
            .map(|v| {
                v.split('|')
                    .map(|p| p.trim().to_string())
                    .filter(|p| !p.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        s.map.extra_map_change_times = ini
            .get("map", "extraMapChangeTimes")
            .map(|v| {
                // 空项 = 未设置，必须原样保留占位，否则索引会与 extra_map_files 错位。
                v.split('|')
                    .map(|p| {
                        let p = p.trim();
                        if p.is_empty() {
                            None
                        } else {
                            p.parse::<f64>().ok()
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        s.map.relative_width = ini.get_f64("map", "relativeWidth", s.map.relative_width);
        s.map.scale = ini.get_f64("map", "scale", s.map.scale);
        s.map.header_crop = ini
            .get("map", "headerCrop")
            .and_then(|v| v.parse().ok())
            .unwrap_or(s.map.header_crop);
        s.map.background_color = color_to_hex(
            ini.get_string("map", "backgroundColor", "").as_str(),
            "#ffffff",
        );
        s.map.x = ini.get_f64("map", "x", s.map.x);
        s.map.y = ini.get_f64("map", "y", s.map.y);
        s.map.user_x = ini.get_f64("map", "userX", s.map.user_x);
        s.map.user_y = ini.get_f64("map", "userY", s.map.user_y);
        s.map.angle = ini.get_f64("map", "angle", s.map.angle);
        s.map.user_angle = ini.get_f64("map", "userAngle", s.map.user_angle);
        s.map.user_scale = ini.get_f64("map", "userScale", s.map.user_scale);

        s.route.quick_route_jpeg_file_path = ini.get_string(
            "route",
            "quickRouteJpegFilePath",
            &s.route.quick_route_jpeg_file_path,
        );
        s.route.user_scale = ini.get_f64("route", "userScale", s.route.user_scale);
        // 原版模式编号：0=无、1=简洁、2=高亮、3=配速。本程序不再有「高亮」（2 号空出），
        // 老工程里读到 2 一律按单色「简洁」处理：读进来就归一成 1，界面上显示的模式与实际
        // 画出来的一致；存回 .orv 时也写 1（这个工程确实没有高亮模式了）。
        let raw_route_mode: i32 = ini
            .get("route", "routeRenderMode")
            .and_then(|v| v.parse().ok())
            .unwrap_or(s.route.render_mode);
        s.route.render_mode = if raw_route_mode == 2 { 1 } else { raw_route_mode };
        let raw_tail_mode: i32 = ini
            .get("route", "tailRenderMode")
            .and_then(|v| v.parse().ok())
            .unwrap_or(s.route.tail_render_mode);
        s.route.tail_render_mode = if raw_tail_mode == 2 { 1 } else { raw_tail_mode };
        // 渲染范围：0 = 完整（整条路线）、1 = 单段（只画当前分段）。别的值一律当「完整」。
        s.route.render_scope = match ini
            .get("route", "routeRenderScope")
            .and_then(|v| v.trim().parse::<i32>().ok())
        {
            Some(1) => 1,
            _ => 0,
        };
        s.route.width = ini.get_f64("route", "routeWidth", s.route.width);
        s.route.tail_width = ini.get_f64("route", "tailWidth", s.route.tail_width);
        s.route.tail_length = ini.get_f64("route", "tailLength", s.route.tail_length);
        s.route.control_radius = ini.get_f64("route", "controlRadius", s.route.control_radius);
        s.route.runner_radius = ini.get_f64("route", "runnerRadius", s.route.runner_radius);
        s.route.show_controls = ini.get_bool("route", "showControls", s.route.show_controls);
        s.route.show_runner = ini.get_bool("route", "showRunner", s.route.show_runner);
        // 颜色 vs 不透明度：新模型把两者拆开（颜色只存 `#rrggbb`，不透明度是独立的 0..255 项）。
        // 老工程里颜色自带 alpha（`#rrggbbaa`），且「简洁/高亮」是两个键 —— 这里把老键的值
        // 拆成「颜色 + 不透明度」迁移过来（新键缺失时才看老键）。
        let legacy_discreet = color_to_hex(
            ini.get_string("route", "discreetColor", "").as_str(),
            "#00000050",
        );
        let legacy_highlight = color_to_hex(
            ini.get_string("route", "highlightColor", "").as_str(),
            "#0064ffc8",
        );
        // 原版的尾巴用「简洁色」还是「高亮色」取决于尾巴渲染模式（2=高亮），迁移时照做；
        // 路线同理：原版路线模式 2（高亮）用的是 highlightColor。
        let legacy_tail = if raw_tail_mode == 2 {
            &legacy_highlight
        } else {
            &legacy_discreet
        };
        let legacy_route = if raw_route_mode == 2 {
            &legacy_highlight
        } else {
            &legacy_discreet
        };
        let route_src = non_empty(ini.get_string("route", "routeColor", ""), legacy_route);
        s.route.route_color = rgb_of(&color_to_hex(&route_src, legacy_route));
        s.route.route_opacity = ini.get_f64("route", "routeOpacity", alpha_of(&route_src, 80.0));

        let tail_src = non_empty(ini.get_string("route", "tailColor", ""), legacy_tail);
        s.route.tail_color = rgb_of(&color_to_hex(&tail_src, legacy_tail));
        s.route.tail_opacity = ini.get_f64("route", "tailOpacity", alpha_of(&tail_src, 80.0));

        let control_src = ini.get_string("route", "controlColor", "");
        s.route.control_color = rgb_of(&color_to_hex(&control_src, "#000000"));
        s.route.control_opacity =
            ini.get_f64("route", "controlOpacity", alpha_of(&control_src, 255.0));

        let runner_src = ini.get_string("route", "runnerColor", "");
        s.route.runner_color = rgb_of(&color_to_hex(&runner_src, "#0064ff"));
        s.route.runner_opacity =
            ini.get_f64("route", "runnerOpacity", alpha_of(&runner_src, 255.0));
        s.route.runner_style = ini
            .get("route", "runnerStyle")
            .and_then(|v| v.parse().ok())
            .unwrap_or(s.route.runner_style);

        s.route.low_pace = ini.get_f64("route", "lowPace", s.route.low_pace);
        s.route.high_pace = ini.get_f64("route", "highPace", s.route.high_pace);
        s.route.pace_auto = match ini.get("route", "paceAuto") {
            // 空串等同于「没写这个键」：保留默认（自适应），别把空值当成 false。
            Some(v) if !v.trim().is_empty() => v.trim() == "1" || v.trim().eq_ignore_ascii_case("true"),
            _ => s.route.pace_auto,
        };
        // 老工程共用路线色带；缺少尾巴键时继承原值，保持原来的画面。
        s.route.tail_low_pace = ini.get_f64("route", "tailLowPace", s.route.low_pace);
        s.route.tail_high_pace = ini.get_f64("route", "tailHighPace", s.route.high_pace);
        s.route.tail_pace_auto = match ini.get("route", "tailPaceAuto") {
            Some(v) if !v.trim().is_empty() => v.trim() == "1" || v.trim().eq_ignore_ascii_case("true"),
            _ => s.route.pace_auto,
        };
        s.route.frame_readout = ini.get_f64("route", "frameReadout", s.route.frame_readout);
        s.route.view_mode = ini
            .get("route", "viewMode")
            .and_then(|v| v.parse().ok())
            .unwrap_or(s.route.view_mode);
        s.route.runner_averaging_factor = ini.get_f64(
            "route",
            "runnerAveragingFactor",
            s.route.runner_averaging_factor,
        );
        s.route.runner_time_offset =
            ini.get_f64("route", "runnerTimeOffset", s.route.runner_time_offset);
        s.route.control_time_offset =
            ini.get_f64("route", "controlTimeOffset", s.route.control_time_offset);
        s.route.left_right_margin =
            ini.get_f64("route", "leftRightMargin", s.route.left_right_margin);
        s.route.top_bottom_margin =
            ini.get_f64("route", "topBottomMargin", s.route.top_bottom_margin);

        s.splits.split_times = ini.get_string("splits", "splitTimes", &s.splits.split_times);
        s.splits.point_codes = ini.get_string("splits", "pointCodes", &s.splits.point_codes);
        s.splits.is_relative = ini.get_bool("splits", "isRelative", s.splits.is_relative);

        // 旧工程的经典 HUD 开关继续生效；空白启动保留校园村落默认布局。
        if ini.get("hud", "layout").is_some()
            || ["showElapsedTime", "showSegmentTime", "showSpeed", "showPace",
                "showHeartRate", "showDistance", "showAzimuth", "showElevation"]
                .iter().any(|key| ini.get("hud", key).is_some())
        {
            s.hud.layout = None;
        }
        s.hud.gpx_file_path = ini.get_string("hud", "gpxFilePath", &s.hud.gpx_file_path);
        if let Some(raw) = ini.get("hud", "layout") {
            match crate::hud::Layout::parse(raw) {
                Ok(layout) => s.hud.layout = Some(layout),
                Err(e) => log::warn!("HUD 布局加载失败，使用经典面板: {e}"),
            }
        }
        s.hud.scope = ini.get_string("hud", "scope", &s.hud.scope);
        s.hud.gpx_time_offset = ini.get_f64("hud", "gpxTimeOffset", s.hud.gpx_time_offset);
        s.hud.show_elapsed_time = ini.get_bool("hud", "showElapsedTime", s.hud.show_elapsed_time);
        s.hud.show_segment_time = ini.get_bool("hud", "showSegmentTime", s.hud.show_segment_time);
        s.hud.show_speed = ini.get_bool("hud", "showSpeed", s.hud.show_speed);
        s.hud.show_pace = ini.get_bool("hud", "showPace", s.hud.show_pace);
        s.hud.show_heart_rate = ini.get_bool("hud", "showHeartRate", s.hud.show_heart_rate);
        s.hud.show_distance = ini.get_bool("hud", "showDistance", s.hud.show_distance);
        s.hud.show_azimuth = ini.get_bool("hud", "showAzimuth", s.hud.show_azimuth);
        s.hud.show_elevation = ini.get_bool("hud", "showElevation", s.hud.show_elevation);

        s.encode.backend = ini.get_string("encode", "backend", &s.encode.backend);
        s.encode.format = ini.get_string("encode", "format", &s.encode.format);
        s.encode.rate_control = ini.get_string("encode", "rateControl", &s.encode.rate_control);
        s.encode.quality = ini.get_f64("encode", "quality", s.encode.quality);
        s.encode.bitrate = ini.get_string("encode", "bitrate", &s.encode.bitrate);
        s.encode.preset = ini.get_string("encode", "preset", &s.encode.preset);
        s.encode.speed = ini.get_string("encode", "speed", &s.encode.speed);
        s.encode.audio_codec = ini.get_string("encode", "audioCodec", &s.encode.audio_codec);
        s.encode.audio_bitrate = ini.get_string("encode", "audioBitrate", &s.encode.audio_bitrate);
        s.encode.clip_start_ms = ini.get_f64("encode", "clipStartMs", s.encode.clip_start_ms);
        s.encode.clip_end_ms = ini.get_f64("encode", "clipEndMs", s.encode.clip_end_ms);
        s.encode.range_start_ms = ini.get_f64("encode", "rangeStartMs", s.encode.range_start_ms);
        s.encode.range_end_ms = ini.get_f64("encode", "rangeEndMs", s.encode.range_end_ms);
        s.encode.output_file_path =
            ini.get_string("encode", "outputFilePath", &s.encode.output_file_path);
        s.encode.resolution = ini.get_string("encode", "resolution", &s.encode.resolution);
        s.encode.frame_rate = ini.get_string("encode", "frameRate", &s.encode.frame_rate);
        let legacy_map_only = ini.get_bool("encode", "mapOnly", s.encode.map_only);
        let has_layer_keys = ini.get("encode", "includeMap").is_some()
            || ini.get("encode", "includeVideo").is_some()
            || ini.get("encode", "includeHud").is_some();
        if has_layer_keys {
            s.encode.include_map = ini.get_bool("encode", "includeMap", s.encode.include_map);
            s.encode.include_video = ini.get_bool("encode", "includeVideo", s.encode.include_video);
            s.encode.include_hud = ini.get_bool("encode", "includeHud", s.encode.include_hud);
        } else if legacy_map_only {
            s.encode.include_map = true;
            s.encode.include_video = false;
            s.encode.include_hud = true;
        }
        s.encode.map_only = !s.encode.include_video;
        s.encode.start_seg = ini
            .get("encode", "startSeg")
            .and_then(|v| v.parse().ok())
            .unwrap_or(s.encode.start_seg);
        s.encode.end_seg = ini
            .get("encode", "endSeg")
            .and_then(|v| v.parse().ok())
            .unwrap_or(s.encode.end_seg);
        s.encode.seg_enabled = ini.get_bool("encode", "segEnabled", s.encode.seg_enabled);
        s.encode.all_seg_enabled =
            ini.get_bool("encode", "allSegEnabled", s.encode.all_seg_enabled);
        // 旧文件没有 clipMode：按两个开关反推（逐段 > 自定义分段 > 全部）。
        let derived_mode = if s.encode.all_seg_enabled {
            "all"
        } else if s.encode.seg_enabled {
            "custom"
        } else {
            "none"
        };
        s.encode.clip_mode = ini.get_string("encode", "clipMode", derived_mode);
        // 老文件只有 clipStartMs/clipEndMs（当时自定义时间直接写那一对），补到 range_* 上。
        if s.encode.clip_mode.eq_ignore_ascii_case("time")
            && s.encode.range_start_ms == 0.0
            && s.encode.range_end_ms == 0.0
        {
            s.encode.range_start_ms = s.encode.clip_start_ms;
            s.encode.range_end_ms = s.encode.clip_end_ms;
        }

        s.gyro.enabled = ini.get_bool("gyro", "enabled", s.gyro.enabled);
        s.gyro.fov = ini.get_f64("gyro", "fov", s.gyro.fov);
        s.gyro.zoom_mode = ini
            .get("gyro", "zoomMode")
            .and_then(|v| v.parse().ok())
            .unwrap_or(s.gyro.zoom_mode);
        s.gyro.max_zoom = ini.get_f64("gyro", "maxZoom", s.gyro.max_zoom);
        s.gyro.max_zoom_iterations = ini
            .get("gyro", "maxZoomIterations")
            .and_then(|v| v.parse().ok())
            .unwrap_or(s.gyro.max_zoom_iterations);
        s.gyro.adaptive_zoom_window =
            ini.get_f64("gyro", "adaptiveZoomWindow", s.gyro.adaptive_zoom_window);
        s.gyro.zoom_center_x = ini.get_f64("gyro", "zoomCenterX", s.gyro.zoom_center_x);
        s.gyro.zoom_center_y = ini.get_f64("gyro", "zoomCenterY", s.gyro.zoom_center_y);
        s.gyro.smoothing_method =
            ini.get_string("gyro", "smoothingMethod", &s.gyro.smoothing_method);
        s.gyro.smoothing_params =
            ini.get_string("gyro", "smoothingParams", &s.gyro.smoothing_params);
        s.gyro.video_speed = ini.get_f64("gyro", "videoSpeed", s.gyro.video_speed);
        s.gyro.video_speed_affects_smoothing =
            ini.get_bool("gyro", "videoSpeedAffectsSmoothing", s.gyro.video_speed_affects_smoothing);
        s.gyro.video_speed_affects_zooming =
            ini.get_bool("gyro", "videoSpeedAffectsZooming", s.gyro.video_speed_affects_zooming);
        s.gyro.video_speed_affects_zooming_limit = ini.get_bool(
            "gyro",
            "videoSpeedAffectsZoomingLimit",
            s.gyro.video_speed_affects_zooming_limit,
        );
        s.gyro.use_gravity_vectors =
            ini.get_bool("gyro", "useGravityVectors", s.gyro.use_gravity_vectors);
        s.gyro.horizon_lock_roll =
            ini.get_f64("gyro", "horizonLockRoll", s.gyro.horizon_lock_roll);
        // ---- 地平线锁定进阶 + 卷帘开关 + 焦距平滑（对齐原版 Stabilization 面板） ----
        s.gyro.horizon_lock_enabled =
            ini.get_bool("gyro", "horizonLockEnabled", s.gyro.horizon_lock_enabled);
        s.gyro.horizon_automatic_lock =
            ini.get_bool("gyro", "horizonAutomaticLock", s.gyro.horizon_automatic_lock);
        s.gyro.horizon_lock_pitch_enabled = ini.get_bool(
            "gyro",
            "horizonLockPitchEnabled",
            s.gyro.horizon_lock_pitch_enabled,
        );
        s.gyro.horizon_lock_pitch =
            ini.get_f64("gyro", "horizonLockPitch", s.gyro.horizon_lock_pitch);
        s.gyro.horizon_lock_integration_method = ini
            .get("gyro", "horizonLockIntegrationMethod")
            .and_then(|v| v.parse().ok())
            .unwrap_or(s.gyro.horizon_lock_integration_method);
        s.gyro.horizon_turn_threshold =
            ini.get_f64("gyro", "horizonTurnThreshold", s.gyro.horizon_turn_threshold);
        s.gyro.horizon_turn_smoothing_ms =
            ini.get_f64("gyro", "horizonTurnSmoothingMs", s.gyro.horizon_turn_smoothing_ms);
        s.gyro.horizon_turn_multiplier =
            ini.get_f64("gyro", "horizonTurnMultiplier", s.gyro.horizon_turn_multiplier);
        s.gyro.horizon_limit_tilt_accel =
            ini.get_bool("gyro", "horizonLimitTiltAccel", s.gyro.horizon_limit_tilt_accel);
        s.gyro.horizon_tilt_accel_limit =
            ini.get_f64("gyro", "horizonTiltAccelLimit", s.gyro.horizon_tilt_accel_limit);
        s.gyro.frame_readout_enabled =
            ini.get_bool("gyro", "frameReadoutEnabled", s.gyro.frame_readout_enabled);
        s.gyro.focal_length_smoothing_enabled = ini.get_bool(
            "gyro",
            "focalLengthSmoothingEnabled",
            s.gyro.focal_length_smoothing_enabled,
        );
        s.gyro.focal_length_smoothing_strength = ini.get_f64(
            "gyro",
            "focalLengthSmoothingStrength",
            s.gyro.focal_length_smoothing_strength,
        );
        s.gyro.frame_readout_direction = ini.get_string(
            "gyro",
            "frameReadoutDirection",
            &s.gyro.frame_readout_direction,
        );
        for (i, key) in ["additionalRotationX", "additionalRotationY", "additionalRotationZ"]
            .iter()
            .enumerate()
        {
            s.gyro.additional_rotation[i] = ini.get_f64("gyro", key, s.gyro.additional_rotation[i]);
        }
        for (i, key) in [
            "additionalTranslationX",
            "additionalTranslationY",
            "additionalTranslationZ",
        ]
        .iter()
        .enumerate()
        {
            s.gyro.additional_translation[i] =
                ini.get_f64("gyro", key, s.gyro.additional_translation[i]);
        }
        s.gyro.lens_correction_amount = ini.get_f64(
            "gyro",
            "lensCorrectionAmount",
            s.gyro.lens_correction_amount,
        );
        s.gyro.horizon_lock = ini.get_f64("gyro", "horizonLock", s.gyro.horizon_lock);
        s.gyro.frame_readout_time =
            ini.get_f64("gyro", "frameReadoutTime", s.gyro.frame_readout_time);
        s.gyro.preset_file_path =
            ini.get_string("gyro", "presetFilePath", &s.gyro.preset_file_path);
        s.gyro.lens_profile = ini.get_string("gyro", "lensProfile", &s.gyro.lens_profile);
        s.gyro.lens_profile_name =
            ini.get_string("gyro", "lensProfileName", &s.gyro.lens_profile_name);
        s.gyro.offset_ms = ini.get_f64("gyro", "offsetMs", s.gyro.offset_ms);
        s.gyro.offset_frames = ini
            .get("gyro", "offsetFrames")
            .and_then(|v| v.parse().ok())
            .unwrap_or(s.gyro.offset_frames);

        s.window_fullscreen = ini.get_bool("window", "fullscreen", s.window_fullscreen);
        s.window_hide_cursor = ini.get_bool("window", "hideCursor", s.window_hide_cursor);
        s.window_multisamples = ini
            .get("window", "multisamples")
            .and_then(|v| v.parse().ok())
            .unwrap_or(s.window_multisamples);
        s.window.2 = ini
            .get("window", "width")
            .and_then(|v| v.parse().ok())
            .unwrap_or(s.window.2);
        s.window.3 = ini
            .get("window", "height")
            .and_then(|v| v.parse().ok())
            .unwrap_or(s.window.3);
        s.preview_perf = ini.get_bool("window", "previewPerf", s.preview_perf);
        s.theme = ini.get_string("window", "theme", &s.theme);

        // Segments are the source of truth for the export clip when enabled; concrete clip ms
        // are derived so every consumer (GUI + CLI) sees a consistent range. Legacy files
        // (segEnabled = false) keep their stored clipStartMs/clipEndMs untouched — except
        // 「自定义时间」, whose clip ms 也要从 range_* 重新算出来，好让命令行导出看到同一个范围。
        if s.encode.seg_enabled || s.encode.clip_mode.eq_ignore_ascii_case("time") {
            s.resolve_encode_clip_ms();
        }

        s
    }

    /// Number of export segments = intervals between split boundaries (split points − 1).
    /// Used by the encode page to show the selectable range.
    pub fn encode_total_segments(&self) -> i32 {
        tracereel_core::splits::build_runner_info(&self.splits.split_times, self.splits.is_relative)
            .splits
            .len()
            .saturating_sub(1) as i32
    }

    /// Video/route milliseconds at split boundary `idx` (0-based; boundary 0 = route t=0).
    /// `add_video_offset` shifts by `video.start_time_offset` (the departure time on the
    /// video timeline) so the value is FILE time for a video export; pass false for a
    /// map-only export whose timeline is route time directly.
    fn boundary_ms(&self, idx: usize, add_video_offset: bool) -> f64 {
        let info =
            tracereel_core::splits::build_runner_info(&self.splits.split_times, self.splits.is_relative);
        if info.splits.is_empty() {
            return 0.0;
        }
        let idx = idx.min(info.splits.len() - 1);
        let mut ms = info.splits[idx].absolute_time * 1000.0;
        if add_video_offset {
            ms += self.video.start_time_offset_s * 1000.0;
        } else {
            ms += self.route.control_time_offset * 1000.0;
        }
        ms.max(0.0)
    }

    /// 「分段 i 在**视频时间轴**上的毫秒位置」（分段点绝对时间 + 录像-出发时间），
    /// 与导出使用的 `boundary_ms(idx, add_video_offset=true)` 完全一致 —— 供「分段」页的跳转按钮用。
    /// 约定：idx=0 = 出发时刻，idx=k = 第 k 个分段点（与分段时间列表一一对应）。
    pub fn split_video_ms(&self, idx: usize) -> f64 {
        self.boundary_ms(idx, true)
    }

    /// Recompute `clip_start_ms`/`clip_end_ms` from the split-point selections (only when
    /// `seg_enabled`; otherwise a full export). `start_seg = a` begins AT split point a
    /// (a=0 → the departure time, i.e. boundary 0 + `video.start_time_offset`); `end_seg = b`
    /// ends AT split point b (b=0 → until the tail). So with boundaries `0,10,30,60`:
    /// `start=0,end=1` → [departure, 10] (length = 分段1); `start=1,end=3` → [10, 60]
    /// (length = 分段2+分段3). Control-time offsets cancel — verified against RouteManager.
    pub fn resolve_encode_clip_ms(&mut self) {
        // 「自定义时间」：起止就是界面上填的那两个毫秒值（`range_*`），不参与分段推导。
        // 填反了（结束 ≤ 开始）当没填，即到末尾。`range_*` 本身不动，切挡位回来还在。
        if self.encode.clip_mode.eq_ignore_ascii_case("time") {
            let start = if self.encode.range_start_ms.is_finite() && self.encode.range_start_ms > 0.0
            {
                self.encode.range_start_ms
            } else {
                0.0
            };
            self.encode.clip_start_ms = start;
            self.encode.clip_end_ms =
                if self.encode.range_end_ms.is_finite() && self.encode.range_end_ms > start {
                    self.encode.range_end_ms
                } else {
                    0.0
                };
            return;
        }
        if !self.encode.seg_enabled {
            self.encode.clip_start_ms = 0.0;
            self.encode.clip_end_ms = 0.0;
            return;
        }
        let offset = !self.encode.map_only; // video export: boundary 0 = departure (add offset)
        self.encode.clip_start_ms = self.boundary_ms(self.encode.start_seg.max(0) as usize, offset);
        self.encode.clip_end_ms = if self.encode.end_seg > 0 {
            self.boundary_ms(self.encode.end_seg as usize, offset)
        } else {
            0.0
        };
    }

    /// Serialize to an INI document.
    pub fn to_ini(&self) -> String {
        let mut ini = IniFile::default();
        ini.set("video", "videoFilePath", &self.video.video_path);
        ini.set("video", "extraVideos", &self.video.extra_videos.join("|"));
        ini.set(
            "video",
            "startTimeOffset",
            &format!("{}", self.video.start_time_offset_s),
        );
        ini.set(
            "video",
            "loopPlayback",
            &format!("{}", self.video.loop_playback),
        );
        ini.set("video", "rescaleShader", &self.video.rescale_shader);
        ini.set(
            "video",
            "enableVerboseLogging",
            &format!("{}", self.video.enable_verbose_logging),
        );
        ini.set(
            "video",
            "seekToAnyFrame",
            &format!("{}", self.video.seek_to_any_frame),
        );
        ini.set(
            "video",
            "frameCountDivisor",
            &format!("{}", self.video.frame_count_divisor),
        );
        ini.set(
            "video",
            "frameDurationDivisor",
            &format!("{}", self.video.frame_duration_divisor),
        );

        ini.set("map", "imageFilePath", &self.map.image_file_path);
        ini.set("map", "extraMapFiles", &self.map.extra_map_files.join("|"));
        ini.set(
            "map",
            "extraMapChangeTimes",
            &self
                .map
                .extra_map_change_times
                .iter()
                .map(|t| t.map(|v| v.to_string()).unwrap_or_default())
                .collect::<Vec<_>>()
                .join("|"),
        );
        ini.set(
            "map",
            "relativeWidth",
            &format!("{}", self.map.relative_width),
        );
        ini.set("map", "scale", &format!("{}", self.map.scale));
        ini.set("map", "headerCrop", &format!("{}", self.map.header_crop));
        ini.set("map", "backgroundColor", &self.map.background_color);
        ini.set("map", "x", &format!("{}", self.map.x));
        ini.set("map", "y", &format!("{}", self.map.y));
        ini.set("map", "userX", &format!("{}", self.map.user_x));
        ini.set("map", "userY", &format!("{}", self.map.user_y));
        ini.set("map", "angle", &format!("{}", self.map.angle));
        ini.set("map", "userAngle", &format!("{}", self.map.user_angle));
        ini.set("map", "userScale", &format!("{}", self.map.user_scale));

        ini.set(
            "route",
            "quickRouteJpegFilePath",
            &self.route.quick_route_jpeg_file_path,
        );
        ini.set("route", "userScale", &format!("{}", self.route.user_scale));
        ini.set(
            "route",
            "routeRenderMode",
            &format!("{}", self.route.render_mode),
        );
        ini.set(
            "route",
            "tailRenderMode",
            &format!("{}", self.route.tail_render_mode),
        );
        ini.set(
            "route",
            "routeRenderScope",
            &format!("{}", self.route.render_scope),
        );
        ini.set("route", "routeWidth", &format!("{}", self.route.width));
        ini.set("route", "tailWidth", &format!("{}", self.route.tail_width));
        ini.set(
            "route",
            "tailLength",
            &format!("{}", self.route.tail_length),
        );
        ini.set(
            "route",
            "controlRadius",
            &format!("{}", self.route.control_radius),
        );
        ini.set(
            "route",
            "runnerRadius",
            &format!("{}", self.route.runner_radius),
        );
        ini.set(
            "route",
            "showControls",
            &format!("{}", self.route.show_controls),
        );
        ini.set(
            "route",
            "showRunner",
            &format!("{}", self.route.show_runner),
        );
        ini.set("route", "routeColor", &self.route.route_color);
        ini.set(
            "route",
            "routeOpacity",
            &format!("{}", self.route.route_opacity),
        );
        ini.set("route", "tailColor", &self.route.tail_color);
        ini.set(
            "route",
            "tailOpacity",
            &format!("{}", self.route.tail_opacity),
        );
        ini.set("route", "controlColor", &self.route.control_color);
        ini.set(
            "route",
            "controlOpacity",
            &format!("{}", self.route.control_opacity),
        );
        ini.set("route", "runnerColor", &self.route.runner_color);
        ini.set(
            "route",
            "runnerOpacity",
            &format!("{}", self.route.runner_opacity),
        );
        ini.set("route", "runnerStyle", &format!("{}", self.route.runner_style));
        // 回写一份原版 C++ OrientView 的老键（它只认 `#rrggbbaa` 的老模型）：
        // 简洁颜色 = 路线颜色，高亮颜色 = 尾巴颜色。这样同一个 .orv 在原版里打开时
        // 路线/尾巴的颜色不会跳回默认值。
        ini.set(
            "route",
            "discreetColor",
            &legacy_color(&self.route.route_color, self.route.route_opacity),
        );
        ini.set(
            "route",
            "highlightColor",
            &legacy_color(&self.route.tail_color, self.route.tail_opacity),
        );
        ini.set("route", "tailLowPace", &format!("{}", self.route.tail_low_pace));
        ini.set("route", "tailHighPace", &format!("{}", self.route.tail_high_pace));
        ini.set("route", "tailPaceAuto", if self.route.tail_pace_auto { "1" } else { "0" });
        ini.set("route", "lowPace", &format!("{}", self.route.low_pace));
        ini.set("route", "highPace", &format!("{}", self.route.high_pace));
        ini.set(
            "route",
            "paceAuto",
            if self.route.pace_auto { "1" } else { "0" },
        );
        ini.set(
            "route",
            "frameReadout",
            &format!("{}", self.route.frame_readout),
        );
        ini.set("route", "viewMode", &format!("{}", self.route.view_mode));
        ini.set(
            "route",
            "runnerAveragingFactor",
            &format!("{}", self.route.runner_averaging_factor),
        );
        ini.set(
            "route",
            "runnerTimeOffset",
            &format!("{}", self.route.runner_time_offset),
        );
        ini.set(
            "route",
            "controlTimeOffset",
            &format!("{}", self.route.control_time_offset),
        );
        ini.set(
            "route",
            "leftRightMargin",
            &format!("{}", self.route.left_right_margin),
        );
        ini.set(
            "route",
            "topBottomMargin",
            &format!("{}", self.route.top_bottom_margin),
        );

        ini.set("splits", "splitTimes", &self.splits.split_times);
        ini.set("splits", "pointCodes", &self.splits.point_codes);
        ini.set(
            "splits",
            "isRelative",
            &format!("{}", self.splits.is_relative),
        );

        ini.set("hud", "gpxFilePath", &self.hud.gpx_file_path);
        if let Some(layout) = &self.hud.layout {
            ini.set("hud", "layout", &layout.json());
        }
        ini.set(
            "hud",
            "gpxTimeOffset",
            &format!("{}", self.hud.gpx_time_offset),
        );
        ini.set(
            "hud",
            "showElapsedTime",
            &format!("{}", self.hud.show_elapsed_time),
        );
        ini.set(
            "hud",
            "showSegmentTime",
            &format!("{}", self.hud.show_segment_time),
        );
        ini.set("hud", "scope", &self.hud.scope);
        ini.set("hud", "showSpeed", &format!("{}", self.hud.show_speed));
        ini.set("hud", "showPace", &format!("{}", self.hud.show_pace));
        ini.set(
            "hud",
            "showHeartRate",
            &format!("{}", self.hud.show_heart_rate),
        );
        ini.set(
            "hud",
            "showDistance",
            &format!("{}", self.hud.show_distance),
        );
        ini.set("hud", "showAzimuth", &format!("{}", self.hud.show_azimuth));
        ini.set(
            "hud",
            "showElevation",
            &format!("{}", self.hud.show_elevation),
        );

        ini.set("encode", "backend", &self.encode.backend);
        ini.set("encode", "format", &self.encode.format);
        ini.set("encode", "rateControl", &self.encode.rate_control);
        ini.set("encode", "quality", &format!("{}", self.encode.quality));
        ini.set("encode", "bitrate", &self.encode.bitrate);
        ini.set("encode", "preset", &self.encode.preset);
        ini.set("encode", "speed", &self.encode.speed);
        ini.set("encode", "audioCodec", &self.encode.audio_codec);
        ini.set("encode", "audioBitrate", &self.encode.audio_bitrate);
        ini.set(
            "encode",
            "clipStartMs",
            &format!("{}", self.encode.clip_start_ms),
        );
        ini.set(
            "encode",
            "clipEndMs",
            &format!("{}", self.encode.clip_end_ms),
        );
        ini.set(
            "encode",
            "rangeStartMs",
            &format!("{}", self.encode.range_start_ms),
        );
        ini.set(
            "encode",
            "rangeEndMs",
            &format!("{}", self.encode.range_end_ms),
        );
        ini.set("encode", "startSeg", &format!("{}", self.encode.start_seg));
        ini.set("encode", "endSeg", &format!("{}", self.encode.end_seg));
        ini.set(
            "encode",
            "allSegEnabled",
            &format!("{}", self.encode.all_seg_enabled),
        );
        ini.set(
            "encode",
            "segEnabled",
            &format!("{}", self.encode.seg_enabled),
        );
        ini.set("encode", "clipMode", &self.encode.clip_mode);
        ini.set("encode", "outputFilePath", &self.encode.output_file_path);
        ini.set("encode", "resolution", &self.encode.resolution);
        ini.set("encode", "frameRate", &self.encode.frame_rate);
        ini.set(
            "encode",
            "mapOnly",
            &format!("{}", !self.encode.include_video),
        );
        ini.set(
            "encode",
            "includeMap",
            &format!("{}", self.encode.include_map),
        );
        ini.set(
            "encode",
            "includeVideo",
            &format!("{}", self.encode.include_video),
        );
        ini.set(
            "encode",
            "includeHud",
            &format!("{}", self.encode.include_hud),
        );

        ini.set("gyro", "enabled", &format!("{}", self.gyro.enabled));
        ini.set("gyro", "fov", &format!("{}", self.gyro.fov));
        ini.set("gyro", "zoomMode", &format!("{}", self.gyro.zoom_mode));
        ini.set("gyro", "maxZoom", &format!("{}", self.gyro.max_zoom));
        ini.set(
            "gyro",
            "maxZoomIterations",
            &format!("{}", self.gyro.max_zoom_iterations),
        );
        ini.set(
            "gyro",
            "adaptiveZoomWindow",
            &format!("{}", self.gyro.adaptive_zoom_window),
        );
        ini.set("gyro", "zoomCenterX", &format!("{}", self.gyro.zoom_center_x));
        ini.set("gyro", "zoomCenterY", &format!("{}", self.gyro.zoom_center_y));
        ini.set("gyro", "smoothingMethod", &self.gyro.smoothing_method);
        ini.set("gyro", "smoothingParams", &self.gyro.smoothing_params);
        ini.set("gyro", "videoSpeed", &format!("{}", self.gyro.video_speed));
        ini.set(
            "gyro",
            "videoSpeedAffectsSmoothing",
            &format!("{}", self.gyro.video_speed_affects_smoothing),
        );
        ini.set(
            "gyro",
            "videoSpeedAffectsZooming",
            &format!("{}", self.gyro.video_speed_affects_zooming),
        );
        ini.set(
            "gyro",
            "videoSpeedAffectsZoomingLimit",
            &format!("{}", self.gyro.video_speed_affects_zooming_limit),
        );
        ini.set(
            "gyro",
            "useGravityVectors",
            &format!("{}", self.gyro.use_gravity_vectors),
        );
        ini.set(
            "gyro",
            "horizonLockRoll",
            &format!("{}", self.gyro.horizon_lock_roll),
        );
        ini.set(
            "gyro",
            "horizonLockEnabled",
            &format!("{}", self.gyro.horizon_lock_enabled),
        );
        ini.set(
            "gyro",
            "horizonAutomaticLock",
            &format!("{}", self.gyro.horizon_automatic_lock),
        );
        ini.set(
            "gyro",
            "horizonLockPitchEnabled",
            &format!("{}", self.gyro.horizon_lock_pitch_enabled),
        );
        ini.set(
            "gyro",
            "horizonLockPitch",
            &format!("{}", self.gyro.horizon_lock_pitch),
        );
        ini.set(
            "gyro",
            "horizonLockIntegrationMethod",
            &format!("{}", self.gyro.horizon_lock_integration_method),
        );
        ini.set(
            "gyro",
            "horizonTurnThreshold",
            &format!("{}", self.gyro.horizon_turn_threshold),
        );
        ini.set(
            "gyro",
            "horizonTurnSmoothingMs",
            &format!("{}", self.gyro.horizon_turn_smoothing_ms),
        );
        ini.set(
            "gyro",
            "horizonTurnMultiplier",
            &format!("{}", self.gyro.horizon_turn_multiplier),
        );
        ini.set(
            "gyro",
            "horizonLimitTiltAccel",
            &format!("{}", self.gyro.horizon_limit_tilt_accel),
        );
        ini.set(
            "gyro",
            "horizonTiltAccelLimit",
            &format!("{}", self.gyro.horizon_tilt_accel_limit),
        );
        ini.set(
            "gyro",
            "frameReadoutEnabled",
            &format!("{}", self.gyro.frame_readout_enabled),
        );
        ini.set(
            "gyro",
            "focalLengthSmoothingEnabled",
            &format!("{}", self.gyro.focal_length_smoothing_enabled),
        );
        ini.set(
            "gyro",
            "focalLengthSmoothingStrength",
            &format!("{}", self.gyro.focal_length_smoothing_strength),
        );
        ini.set("gyro", "frameReadoutDirection", &self.gyro.frame_readout_direction);
        for (i, key) in ["additionalRotationX", "additionalRotationY", "additionalRotationZ"]
            .iter()
            .enumerate()
        {
            ini.set("gyro", key, &format!("{}", self.gyro.additional_rotation[i]));
        }
        for (i, key) in [
            "additionalTranslationX",
            "additionalTranslationY",
            "additionalTranslationZ",
        ]
        .iter()
        .enumerate()
        {
            ini.set("gyro", key, &format!("{}", self.gyro.additional_translation[i]));
        }
        ini.set(
            "gyro",
            "lensCorrectionAmount",
            &format!("{}", self.gyro.lens_correction_amount),
        );
        ini.set(
            "gyro",
            "horizonLock",
            &format!("{}", self.gyro.horizon_lock),
        );
        ini.set(
            "gyro",
            "frameReadoutTime",
            &format!("{}", self.gyro.frame_readout_time),
        );
        ini.set("gyro", "presetFilePath", &self.gyro.preset_file_path);
        ini.set("gyro", "lensProfile", &self.gyro.lens_profile);
        ini.set("gyro", "lensProfileName", &self.gyro.lens_profile_name);
        ini.set("gyro", "offsetMs", &format!("{}", self.gyro.offset_ms));
        ini.set(
            "gyro",
            "offsetFrames",
            &format!("{}", self.gyro.offset_frames),
        );

        ini.set("window", "width", &format!("{}", self.window.2));
        ini.set("window", "height", &format!("{}", self.window.3));
        ini.set(
            "window",
            "fullscreen",
            &format!("{}", self.window_fullscreen),
        );
        ini.set(
            "window",
            "hideCursor",
            &format!("{}", self.window_hide_cursor),
        );
        ini.set(
            "window",
            "multisamples",
            &format!("{}", self.window_multisamples),
        );
        ini.set("window", "previewPerf", &format!("{}", self.preview_perf));
        ini.set("window", "theme", &self.theme);

        ini.to_ini_string()
    }

    pub fn save_orv(&self, path: &str) -> std::io::Result<()> {
        fs::write(path, self.to_ini())
    }

    /// Resolve a path relative to the `.orv` file's directory when the stored path is
    /// relative and the orv location is known.
    pub fn resolve_orv_paths(&mut self, orv_dir: &Path) {
        let resolve = |p: &String| -> String {
            if p.is_empty() {
                return p.clone();
            }
            let pb = PathBuf::from(p);
            if pb.is_absolute() {
                return p.clone();
            }
            orv_dir.join(pb).to_string_lossy().into_owned()
        };
        self.video.video_path = resolve(&self.video.video_path);
        for p in &mut self.video.extra_videos {
            *p = resolve(p);
        }
        self.map.image_file_path = resolve(&self.map.image_file_path);
        for p in &mut self.map.extra_map_files {
            *p = resolve(p);
        }
        self.route.quick_route_jpeg_file_path = resolve(&self.route.quick_route_jpeg_file_path);
        self.hud.gpx_file_path = resolve(&self.hud.gpx_file_path);
        self.gyro.preset_file_path = resolve(&self.gyro.preset_file_path);
        self.encode.output_file_path = resolve(&self.encode.output_file_path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ini_sections() {
        let text = "[video]\r\nvideoFilePath=foo.mp4\r\nstartTimeOffset=100\r\n\r\n[map]\r\nrelativeWidth=0.25\r\n";
        let ini = IniFile::parse(text);
        assert_eq!(ini.get_string("video", "videoFilePath", ""), "foo.mp4");
        assert_eq!(ini.get_f64("video", "startTimeOffset", 0.0), 100.0);
        assert_eq!(ini.get_f64("map", "relativeWidth", 0.0), 0.25);
    }

    #[test]
    fn startup_display_framing_and_hud_defaults() {
        let reference = Settings::from_ini(&IniFile::parse(include_str!("../../demo/example.trr")));
        let defaults = Settings::default();
        let actual = IniFile::parse(&defaults.to_ini());
        let expected = IniFile::parse(&reference.to_ini());
        for key in [
            "routeRenderMode", "routeRenderScope", "routeWidth", "routeColor", "routeOpacity",
            "tailRenderMode", "tailWidth", "tailLength", "tailColor", "tailOpacity",
            "lowPace", "highPace", "paceAuto", "tailLowPace", "tailHighPace", "tailPaceAuto",
            "showControls", "controlRadius", "controlColor", "controlOpacity",
            "showRunner", "runnerStyle", "runnerRadius", "runnerColor", "runnerOpacity",
            "viewMode", "leftRightMargin", "topBottomMargin",
        ] {
            assert_eq!(actual.get("route", key), expected.get("route", key), "{key}");
        }
        assert_eq!(defaults.hud.scope, "video");
        let startup = Settings::load_orv("");
        assert_eq!(startup.hud.scope, "video");
        assert_eq!(startup.hud.layout.unwrap().json(), crate::hud::Layout::theme("campus").json());
        assert_eq!(defaults.hud.layout.unwrap().json(), crate::hud::Layout::theme("campus").json());
        assert!(defaults.video.video_path.is_empty());
        assert!(defaults.route.quick_route_jpeg_file_path.is_empty());
    }

    #[test]
    fn roundtrips_default_settings() {
        let s = Settings::default();
        let ini = s.to_ini();
        let s2 = Settings::from_ini(&IniFile::parse(&ini));
        assert_eq!(s2.map.relative_width, 0.25);
        assert_eq!(s2.gyro.fov, 0.85);
        assert_eq!(s2.gyro.frame_readout_time, 11.09);
        assert_eq!(s2.encode.audio_codec, "copy");
    }

    #[test]
    fn load_orv_keeps_source_resolution_and_normalizes_legacy_presets() {
        let dir = std::env::temp_dir().join(format!("tracereel-res-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let load = |name: &str, res: &str| -> String {
            let p = dir.join(name);
            std::fs::write(&p, format!("[encode]\nresolution={res}\n")).unwrap();
            Settings::load_orv(p.to_string_lossy().as_ref())
                .encode
                .resolution
        };
        // 「与原始视频一致」是下拉里可选的一项：存盘 → 重新打开必须原样保留，
        // 否则用户选好的「与原始视频一致」会被悄悄换成 1920x1080。
        assert_eq!(load("keep.trr", "source"), "source");
        // 老工程的档位名仍然归一成显式尺寸（下拉选不中会导致显示与实际导出不一致）。
        assert_eq!(load("p1080.trr", "1080p"), "1920x1080");
        assert_eq!(load("p720.trr", "720p"), "1280x720");
        assert_eq!(load("uhd.trr", "4k"), "3840x2160");
        // 显式尺寸原样保留。
        assert_eq!(load("explicit.trr", "2688x2016"), "2688x2016");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn split_codes_roundtrip_with_optional_and_repeated_points() {
        let mut settings = Settings::default();
        settings.splits.split_times = "1:21|2:55|22:03.1".into();
        settings.splits.point_codes = "120||120".into();
        let loaded = Settings::from_ini(&IniFile::parse(&settings.to_ini()));
        assert_eq!(loaded.splits.split_times, settings.splits.split_times);
        assert_eq!(loaded.splits.point_codes, "120||120");
        let legacy = Settings::from_ini(&IniFile::parse("[splits]\nsplitTimes=1:21|2:55"));
        assert!(legacy.splits.point_codes.is_empty());
        let info = tracereel_core::splits::build_runner_info(&loaded.splits.split_times, false);
        assert!((info.splits.last().unwrap().absolute_time - 1323.1).abs() < 1e-9);
    }

    #[test]
    fn export_layers_migrate_from_map_only_and_roundtrip() {
        let legacy = Settings::from_ini(&IniFile::parse("[encode]\nmapOnly=true"));
        assert!(legacy.encode.include_map);
        assert!(!legacy.encode.include_video);
        assert!(legacy.encode.include_hud);
        assert!(legacy.encode.map_only);

        let mut current = Settings::default();
        current.encode.include_map = false;
        current.encode.include_video = true;
        current.encode.include_hud = false;
        current.hud.scope = "frame".into();
        let loaded = Settings::from_ini(&IniFile::parse(&current.to_ini()));
        assert!(!loaded.encode.include_map);
        assert!(loaded.encode.include_video);
        assert!(!loaded.encode.include_hud);
        assert!(!loaded.encode.map_only);
        assert_eq!(loaded.hud.scope, "frame");
    }

    #[test]
    fn hud_layout_roundtrip_and_legacy() {
        let old = Settings::from_ini(&IniFile::parse(
            "[hud]\nshowSpeed=false\nshowElapsedTime=true",
        ));
        assert!(old.hud.layout.is_none());
        assert!(!old.hud.show_speed);
        let mut s = old;
        let mut layout = crate::hud::Layout::theme("modern");
        layout.widgets[0].rotation = 37.0;
        layout.widgets[0].locked = true;
        s.hud.layout = Some(layout);
        let loaded = Settings::from_ini(&IniFile::parse(&s.to_ini()));
        assert_eq!(
            loaded.hud.layout.unwrap().json(),
            s.hud.layout.unwrap().json()
        );
    }

    #[test]
    fn roundtrips_extra_maps() {
        let mut s = Settings::default();
        s.map.image_file_path = "C:/m1.jfif".into();
        s.map.extra_map_files = vec!["C:/m2.jfif".into(), "C:/m3.jfif".into()];
        s.map.extra_map_change_times = vec![Some(120.5), Some(600.0)];
        let ini = s.to_ini();
        let s2 = Settings::from_ini(&IniFile::parse(&ini));
        assert_eq!(
            s2.map.extra_map_files,
            vec!["C:/m2.jfif".to_string(), "C:/m3.jfif".to_string()]
        );
        assert_eq!(s2.map.extra_map_change_times, vec![Some(120.5), Some(600.0)]);
        let pairs = s2.map.extra_maps();
        assert_eq!(
            pairs,
            vec![
                ("C:/m2.jfif".to_string(), Some(120.5)),
                ("C:/m3.jfif".to_string(), Some(600.0))
            ]
        );
    }

    /// 换图时间没填的地图必须保持「未设置」，而不是退化成 0 秒去抢占整条时间轴；
    /// 空项也不能让后续地图的时间错位。
    #[test]
    fn unset_extra_map_time_stays_unset() {
        let mut s = Settings::default();
        s.map.image_file_path = "C:/m1.jfif".into();
        s.map.extra_map_files = vec!["C:/m2.jfif".into(), "C:/m3.jfif".into()];
        s.map.extra_map_change_times = vec![None, Some(600.0)];
        let ini = s.to_ini();
        assert!(
            ini.contains("extraMapChangeTimes=|600"),
            "empty slot must be kept as a placeholder:\n{ini}"
        );
        let s2 = Settings::from_ini(&IniFile::parse(&ini));
        assert_eq!(s2.map.extra_maps()[0].1, None);
        // 第三张图的时间必须仍对应第三张图，而不是滑到第二张。
        assert_eq!(s2.map.extra_maps()[1].1, Some(600.0));
    }

    #[test]
    fn segments_map_to_video_time() {
        let mut s = Settings::default();
        // Relative splits "10|20|30" + implicit departure (t=0) → split points 0,10,30,60.
        // So 分段1 = 10s, 分段2 = 20s, 分段3 = 30s.
        s.splits.split_times = "10|20|30".into();
        s.splits.is_relative = true;
        s.video.start_time_offset_s = 31.0; // departure (split point 0) is at file t=31s
        s.encode.seg_enabled = true;
        assert_eq!(s.encode_total_segments(), 3);

        // User example 1: start=0 (departure), end=1 → length = 分段1 = 10s.
        s.encode.start_seg = 0;
        s.encode.end_seg = 1;
        s.resolve_encode_clip_ms();
        assert!(
            (s.encode.clip_start_ms - 31_000.0).abs() < 1e-6,
            "got {}",
            s.encode.clip_start_ms
        );
        assert!(
            (s.encode.clip_end_ms - 41_000.0).abs() < 1e-6,
            "got {}",
            s.encode.clip_end_ms
        );

        // User example 2: start=1, end=3 → length = 分段2+分段3 = 50s.
        s.encode.start_seg = 1;
        s.encode.end_seg = 3;
        s.resolve_encode_clip_ms();
        assert!(
            (s.encode.clip_start_ms - 41_000.0).abs() < 1e-6,
            "got {}",
            s.encode.clip_start_ms
        );
        assert!(
            (s.encode.clip_end_ms - 91_000.0).abs() < 1e-6,
            "got {}",
            s.encode.clip_end_ms
        );

        // end_seg = 0 → export to the tail (clip_end_ms = 0 sentinel → source duration).
        s.encode.start_seg = 2;
        s.encode.end_seg = 0;
        s.resolve_encode_clip_ms();
        assert!(
            (s.encode.clip_start_ms - 61_000.0).abs() < 1e-6,
            "got {}",
            s.encode.clip_start_ms
        );
        assert_eq!(s.encode.clip_end_ms, 0.0);

        // Checkbox OFF → full export regardless of the stored segment values.
        s.encode.seg_enabled = false;
        s.resolve_encode_clip_ms();
        assert_eq!(s.encode.clip_start_ms, 0.0);
        assert_eq!(s.encode.clip_end_ms, 0.0);

        // Map-only timeline is route time (no departure offset).
        s.encode.map_only = true;
        s.encode.seg_enabled = true;
        s.encode.start_seg = 1;
        s.encode.end_seg = 3;
        s.resolve_encode_clip_ms();
        assert!(
            (s.encode.clip_start_ms - 10_000.0).abs() < 1e-6,
            "got {}",
            s.encode.clip_start_ms
        );
        assert!(
            (s.encode.clip_end_ms - 60_000.0).abs() < 1e-6,
            "got {}",
            s.encode.clip_end_ms
        );
    }

    /// 「导出范围 = 自定义时间」：起止就按填的时间用，不受分段开关影响，也不被归零；
    /// 填反了（结束 ≤ 开始）当没填（clip_end_ms = 0 → 到末尾）。
    #[test]
    fn time_range_mode_uses_the_typed_bounds() {
        let mut s = Settings::default();
        s.encode.clip_mode = "time".into();
        s.encode.range_start_ms = 12_500.0;
        s.encode.range_end_ms = 48_000.0;
        // 即使还留着分段开关，自定义时间也不该被分段推导覆盖。
        s.encode.seg_enabled = true;
        s.encode.start_seg = 0;
        s.encode.end_seg = 1;
        s.resolve_encode_clip_ms();
        assert_eq!(s.encode.clip_start_ms, 12_500.0);
        assert_eq!(s.encode.clip_end_ms, 48_000.0);

        // 填反了 → 结束归零，即导出到末尾；用户填的值本身不改（切挡位回来还在）。
        s.encode.range_start_ms = 30_000.0;
        s.encode.range_end_ms = 10_000.0;
        s.resolve_encode_clip_ms();
        assert_eq!(s.encode.clip_start_ms, 30_000.0);
        assert_eq!(s.encode.clip_end_ms, 0.0);
        assert_eq!(s.encode.range_end_ms, 10_000.0);

        // 只填开始 → 也是到末尾。
        s.encode.range_end_ms = 0.0;
        s.resolve_encode_clip_ms();
        assert_eq!(s.encode.clip_start_ms, 30_000.0);
        assert_eq!(s.encode.clip_end_ms, 0.0);

        // 换个模式回到「全部」就照旧归零，但填过的起止还留着。
        s.encode.clip_mode = "none".into();
        s.resolve_encode_clip_ms();
        assert_eq!(s.encode.clip_start_ms, 0.0);
        assert_eq!(s.encode.clip_end_ms, 0.0);
        assert_eq!(s.encode.range_start_ms, 30_000.0);
    }

    /// 「导出范围」四种模式要能从 .orv 里原样读回；老文件没有 clipMode 时按两个旧开关反推。
    #[test]
    fn clip_mode_roundtrips_and_defaults_from_legacy_flags() {
        let mut s = Settings::default();
        s.encode.clip_mode = "time".into();
        s.encode.clip_start_ms = 1_000.0;
        s.encode.clip_end_ms = 2_000.0;
        let ini = s.to_ini();
        assert!(ini.contains("clipMode=time"), "missing clipMode:\n{ini}");
        let back = Settings::from_ini(&IniFile::parse(&ini));
        assert_eq!(back.encode.clip_mode, "time");

        // 老文件：无 clipMode 键。
        let mut old = Settings::default();
        old.encode.seg_enabled = true;
        old.encode.all_seg_enabled = true;
        let old_ini: String = old
            .to_ini()
            .lines()
            .filter(|l| !l.starts_with("clipMode="))
            .collect::<Vec<_>>()
            .join("\n");
        let parsed = Settings::from_ini(&IniFile::parse(&old_ini));
        assert_eq!(parsed.encode.clip_mode, "all");
    }

    /// 颜色序列化必须保真不透明度（简洁/高亮的默认色都带 alpha），全不透明时收敛成 6 位，
    /// 于是既有的 6 位 .orv 读写成原文不变。
    #[test]
    fn colors_keep_alpha() {
        assert_eq!(color_to_hex("#0064ff", "#000000"), "#0064ff");
        assert_eq!(color_to_hex("#0064FFC8", "#000000"), "#0064ffc8");
        assert_eq!(color_to_hex("#00000050", "#000000"), "#00000050");
        assert_eq!(color_to_hex("0,100,255", "#000000"), "#0064ff");
        assert_eq!(color_to_hex("0,100,255,200", "#000000"), "#0064ffc8");
        assert_eq!(color_to_hex("", "#0064ffc8"), "#0064ffc8");
        assert_eq!(color_to_hex("不是颜色", "#0064ffc8"), "#0064ffc8");
        // Qt 的 @Variant(...) 二进制串里也可能带逗号，别把它当成 "r,g,b" 解析。
        assert_eq!(color_to_hex("@Variant(1,2,3,4,5)", "#0064ffc8"), "#0064ffc8");
    }

    /// 新增的路线配色字段（路线/尾巴/控制点/运动员各自的「颜色 + 不透明度」）走一轮
    /// to_ini → from_ini 必须原样回来，同时要回写原版两个老键。
    #[test]
    fn route_render_fields_roundtrip() {
        let mut s = Settings::default();
        s.route.route_color = "#112233".to_string();
        s.route.route_opacity = 64.0;
        s.route.tail_color = "#ff8800".to_string();
        s.route.tail_opacity = 192.0;
        s.route.control_color = "#00aa55".to_string();
        s.route.control_opacity = 128.0;
        s.route.runner_color = "#123456".to_string();
        s.route.runner_opacity = 200.0;
        s.route.tail_low_pace = 12.0;
        s.route.tail_high_pace = 4.0;
        s.route.tail_pace_auto = true;
        s.route.low_pace = 6.5;
        s.route.high_pace = 3.25;
        s.route.runner_style = 1;
        s.route.pace_auto = false;
        s.route.render_scope = 1;
        let ini = s.to_ini();
        let s2 = Settings::from_ini(&IniFile::parse(&ini));
        assert_eq!(s2.route.route_color, "#112233");
        assert_eq!(s2.route.route_opacity, 64.0);
        assert_eq!(s2.route.tail_color, "#ff8800");
        assert_eq!(s2.route.tail_opacity, 192.0);
        assert_eq!(s2.route.control_color, "#00aa55");
        assert_eq!(s2.route.control_opacity, 128.0);
        assert_eq!(s2.route.runner_color, "#123456");
        assert_eq!(s2.route.runner_opacity, 200.0);
        assert_eq!(s2.route.tail_low_pace, 12.0);
        assert_eq!(s2.route.tail_high_pace, 4.0);
        assert!(s2.route.tail_pace_auto);
        assert_eq!(s2.route.low_pace, 6.5);
        assert_eq!(s2.route.high_pace, 3.25);
        assert_eq!(s2.route.runner_style, 1);
        assert!(!s2.route.pace_auto);
        let legacy = Settings::from_ini(&IniFile::parse("[route]\nlowPace=9\nhighPace=3\npaceAuto=0\n"));
        assert_eq!((legacy.route.tail_low_pace, legacy.route.tail_high_pace), (9.0, 3.0));
        assert!(!legacy.route.tail_pace_auto);
        assert_eq!(s2.route.render_scope, 1, "渲染范围（单段）要能存回来");
        let written_scope = IniFile::parse(&ini);
        assert_eq!(
            written_scope.get_string("route", "routeRenderScope", ""),
            "1",
            "渲染范围要写进 .orv"
        );
        // 原版老键：颜色 + 不透明度合成 `#rrggbbaa`（不透明时省后缀），原版 OrientView
        // 打开同一个 .orv 才能看到一样的路线/尾巴配色。
        let written = IniFile::parse(&ini);
        assert_eq!(written.get_string("route", "discreetColor", ""), "#11223340");
        assert_eq!(written.get_string("route", "highlightColor", ""), "#ff8800c0");
        let mut opaque = Settings::default();
        opaque.route.route_color = "#112233".to_string();
        opaque.route.route_opacity = 255.0;
        let written = IniFile::parse(&opaque.to_ini());
        assert_eq!(written.get_string("route", "discreetColor", ""), "#112233");

        // 显示与取景默认值参考上午工程。
        let d = Settings::default();
        assert_eq!(d.route.route_color, "#000000");
        assert_eq!(d.route.route_opacity, 80.0);
        assert_eq!(d.route.tail_color, "#000000");
        assert_eq!(d.route.tail_opacity, 204.0);
        assert_eq!(d.route.control_color, "#000000");
        assert_eq!(d.route.control_opacity, 255.0);
        assert_eq!(d.route.runner_color, "#ff5d00");
        assert_eq!(d.route.runner_opacity, 255.0);
        assert_eq!(d.route.low_pace, 15.0);
        assert_eq!(d.route.high_pace, 5.0);
        assert_eq!(d.route.render_mode, 0);
        assert_eq!(d.route.tail_render_mode, 3);
        assert_eq!(d.route.render_scope, 0, "渲染范围默认完整渲染");
        // 运动员默认箭头，配速色带默认按轨迹自适应。
        assert_eq!(d.route.runner_style, 1);
        assert!(d.route.pace_auto);

        // 新键写盘：runnerStyle / paceAuto（paceAuto 用 "1"/"0"）。
        let written = IniFile::parse(&Settings::default().to_ini());
        assert_eq!(written.get_string("route", "runnerStyle", ""), "1");
        assert_eq!(written.get_string("route", "paceAuto", ""), "1");
        assert_eq!(written.get_string("route", "lowPace", ""), "15");
        assert_eq!(written.get_string("route", "highPace", ""), "5");
        // 老工程没有 paceAuto 这个键时不能被当成「关了自适应」。
        let mut without = IniFile::parse(&s.to_ini());
        without.set("route", "paceAuto", "");
        assert!(Settings::from_ini(&without).route.pace_auto);

        // 渲染范围：老工程没有这个键、或写了别的值时一律按「完整渲染」。
        let mut odd_scope = IniFile::parse(&Settings::default().to_ini());
        odd_scope.set("route", "routeRenderScope", "7");
        assert_eq!(Settings::from_ini(&odd_scope).route.render_scope, 0);
        odd_scope.set("route", "routeRenderScope", "");
        assert_eq!(Settings::from_ini(&odd_scope).route.render_scope, 0);
    }

    /// 老工程只有原版的 `discreetColor`/`highlightColor`（颜色自带 alpha）、没有新键：
    /// 必须拆成「颜色 + 不透明度」迁移过来，用户的老配色不能丢。
    #[test]
    fn legacy_colors_migrate_to_color_plus_opacity() {
        let mut old = Settings::default();
        old.route.tail_render_mode = 1; // 老工程的尾巴用「简洁色」
        let mut ini = IniFile::parse(&old.to_ini());
        ini.set("route", "routeColor", "");
        ini.set("route", "routeOpacity", "");
        ini.set("route", "tailColor", "");
        ini.set("route", "tailOpacity", "");
        ini.set("route", "discreetColor", "#11223340");
        ini.set("route", "highlightColor", "#ff8800c0");
        let m = Settings::from_ini(&ini);
        assert_eq!(m.route.route_color, "#112233");
        assert_eq!(m.route.route_opacity, 64.0);
        assert_eq!(m.route.tail_color, "#112233", "尾巴模式=简洁 时老工程取简洁色");
        assert_eq!(m.route.tail_opacity, 64.0);

        // 尾巴模式=高亮（原版 2）的老工程，尾巴取的是「高亮色」。
        let mut old = Settings::default();
        old.route.tail_render_mode = 2;
        let mut ini = IniFile::parse(&old.to_ini());
        ini.set("route", "tailColor", "");
        ini.set("route", "tailOpacity", "");
        ini.set("route", "discreetColor", "#11223340");
        ini.set("route", "highlightColor", "#ff8800c0");
        let m = Settings::from_ini(&ini);
        assert_eq!(m.route.tail_color, "#ff8800");
        assert_eq!(m.route.tail_opacity, 192.0);

        // 原版路线模式 2（高亮）的老工程：路线取「高亮色」，模式读进来归一成「简洁」
        // （本程序没有高亮模式），存回去也写 1 —— 界面显示的模式与实际画出来的必须一致。
        let mut old = Settings::default();
        old.route.render_mode = 2;
        let mut ini = IniFile::parse(&old.to_ini());
        ini.set("route", "routeColor", "");
        ini.set("route", "routeOpacity", "");
        ini.set("route", "discreetColor", "#11223340");
        ini.set("route", "highlightColor", "#ff8800c0");
        let m = Settings::from_ini(&ini);
        assert_eq!(m.route.render_mode, 1, "原版的 2（高亮）按简洁处理");
        assert_eq!(m.route.route_color, "#ff8800", "原版路线模式 2 用的是高亮色");
        assert_eq!(m.route.route_opacity, 192.0);
        assert_eq!(
            IniFile::parse(&m.to_ini()).get_string("route", "routeRenderMode", ""),
            "1",
            "存回去不该再有 2（本程序不提供高亮模式）"
        );

        // 什么都没有的老工程 → 原版默认配色。
        let m = Settings::from_ini(&IniFile::parse(""));
        assert_eq!(m.route.route_color, "#000000");
        assert_eq!(m.route.route_opacity, 80.0);
        assert_eq!(m.route.control_color, "#000000");
        assert_eq!(m.route.control_opacity, 255.0);
        assert_eq!(m.route.runner_color, "#0064ff");
        assert_eq!(m.route.runner_opacity, 255.0);
    }
}
