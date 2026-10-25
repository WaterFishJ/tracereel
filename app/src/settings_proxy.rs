//! SettingsProxy: a QObject exposing the app's `.orv` settings to QML so the settings
//! panel can read/edit them and save back. Kept focused on the fields the settings UI
//! surfaces; the full `Settings` model lives in `crate::settings`.

use qmetaobject::*;

use crate::settings::{MapLayout, Settings};

#[derive(QObject, Default)]
pub struct SettingsProxy {
    base: qt_base_class!(trait QObject),

    // ---- project / paths ----
    file_path: qt_property!(QString),
    video_path: qt_property!(QString),
    /// 续接分段视频列表（'|' 拼接），与 video_path 共同构成完整时间轴。
    video_extras: qt_property!(QString),
    map_path: qt_property!(QString),
    /// 额外地图文件列表（'|' 拼接），与 map_path 共同构成换图序列。
    map_extras: qt_property!(QString),
    /// 换图时间（'|' 拼接，按索引与 map_extras 对齐，从出发时刻算起；支持
    /// 分段时间格式 `h:mm:ss` / `mm:ss` / 纯秒数）。
    map_change_times: qt_property!(QString),
    route_path: qt_property!(QString),
    split_times: qt_property!(QString),
    split_point_codes: qt_property!(QString),
    hud_gpx_path: qt_property!(QString),
    gyro_preset: qt_property!(QString),
    /// 搜索框选中的镜头标定 id（Gyroflow 镜头库 key，或 `.json` 路径）。
    gyro_lens_profile: qt_property!(QString),
    /// 上面那条的显示名（UI 用；库 key 对人不可读）。
    gyro_lens_profile_name: qt_property!(QString),
    encode_output_path: qt_property!(QString),

    // ---- video ----
    video_start_offset: qt_property!(f64),
    loop_playback: qt_property!(bool),
    video_rescale_shader: qt_property!(QString),
    video_verbose: qt_property!(bool),
    video_seek_any: qt_property!(bool),
    video_frame_count_div: qt_property!(i32),
    video_frame_duration_div: qt_property!(i32),

    // ---- window ----
    window_w: qt_property!(i32),
    window_h: qt_property!(i32),
    window_fullscreen: qt_property!(bool),
    window_hide_cursor: qt_property!(bool),
    window_multisamples: qt_property!(i32),
    preview_perf: qt_property!(bool),

    // ---- map ----
    /// 多地图框布局（紧凑 JSON，`{"version":1,"boxes":[...]}`）。
    ///
    /// 与 `hud_layout` 同一套模式：编辑器（`MapEditor.qml`）自己 parse/编辑/stringify，
    /// 拖框/改取景时直接写这个属性并调 `renderer.apply_map_frames(...)`；增删框再走
    /// `collect_settings` 落临时 `.trr` 触发完整重载。逐框的旧属性（relative_width、
    /// map_placement、map_frame_*、route_view_mode/route_lr_margin/route_tb_margin/
    /// route_view_zoom/route_runner_averaging_factor）已被它取代并删除 —— 单一真相，
    /// 避免两处 box0 不一致。
    map_boxes: qt_property!(QString),
    map_scale: qt_property!(f64),
    map_header_crop: qt_property!(i32),
    map_background_color: qt_property!(QString),

    // ---- route ----
    route_render_mode: qt_property!(i32),
    route_render_scope: qt_property!(i32),
    route_tail_render_mode: qt_property!(i32),
    route_width: qt_property!(f64),
    route_tail_width: qt_property!(f64),
    route_tail_length: qt_property!(f64),
    route_control_radius: qt_property!(f64),
    route_runner_radius: qt_property!(f64),
    route_show_controls: qt_property!(bool),
    route_show_runner: qt_property!(bool),
    route_color: qt_property!(QString),
    route_opacity: qt_property!(f64),
    route_tail_color: qt_property!(QString),
    route_tail_opacity: qt_property!(f64),
    route_control_color: qt_property!(QString),
    route_control_opacity: qt_property!(f64),
    route_runner_color: qt_property!(QString),
    route_runner_opacity: qt_property!(f64),
    route_runner_style: qt_property!(i32),
    route_low_pace: qt_property!(f64),
    route_high_pace: qt_property!(f64),
    route_pace_auto: qt_property!(bool),
    route_tail_low_pace: qt_property!(f64),
    route_tail_high_pace: qt_property!(f64),
    route_tail_pace_auto: qt_property!(bool),
    route_control_time_offset: qt_property!(f64),
    route_runner_time_offset: qt_property!(f64),

    // ---- splits ----
    split_is_relative: qt_property!(bool),

    // ---- gyro ----
    gyro_enabled: qt_property!(bool),
    fov: qt_property!(f64),
    max_zoom: qt_property!(f64),
    max_zoom_iterations: qt_property!(i32),
    gyro_zoom_mode: qt_property!(i32),
    gyro_zoom_window: qt_property!(f64),
    gyro_zoom_center_x: qt_property!(f64),
    gyro_zoom_center_y: qt_property!(f64),
    gyro_lens: qt_property!(f64),
    gyro_horizon: qt_property!(f64),
    gyro_horizon_roll: qt_property!(f64),
    /// 「锁定地平线」总开关。
    gyro_horizon_enabled: qt_property!(bool),
    /// 「自动锁定」（勾上后锁定量按 100 送 + 转弯参数生效）。
    gyro_auto_lock: qt_property!(bool),
    gyro_horizon_pitch_enabled: qt_property!(bool),
    gyro_horizon_pitch: qt_property!(f64),
    /// 原版「积分方法」：0 互补 / 1 VQF / 2 陀螺+加速度计 / 3 Mahony / 4 Madgwick。
    gyro_integration_method: qt_property!(i32),
    gyro_turn_threshold: qt_property!(f64),
    gyro_turn_smoothing: qt_property!(f64),
    gyro_turn_multiplier: qt_property!(f64),
    gyro_limit_tilt_accel: qt_property!(bool),
    gyro_tilt_accel_limit: qt_property!(f64),
    /// 卷帘快门校正开关（原版是包住帧读出时间的勾选框）。
    frame_readout_enabled: qt_property!(bool),
    /// 稳定焦距（原版 Stabilize focal length）：仅逐帧焦距的视频有效。
    gyro_fl_smoothing: qt_property!(bool),
    gyro_fl_strength: qt_property!(f64),
    gyro_use_gravity: qt_property!(bool),
    frame_readout_time: qt_property!(f64),
    gyro_readout_dir: qt_property!(QString),
    gyro_smoothing_method: qt_property!(QString),
    /// JSON 文本：`[{"name":"smoothness","value":1}, …]`（与 `.gyroflow` 同格式）。
    gyro_smoothing_params: qt_property!(QString),
    gyro_video_speed: qt_property!(f64),
    gyro_vs_smoothing: qt_property!(bool),
    gyro_vs_zooming: qt_property!(bool),
    gyro_vs_zoom_limit: qt_property!(bool),
    gyro_rot_x: qt_property!(f64),
    gyro_rot_y: qt_property!(f64),
    gyro_rot_z: qt_property!(f64),
    gyro_tr_x: qt_property!(f64),
    gyro_tr_y: qt_property!(f64),
    gyro_tr_z: qt_property!(f64),
    gyro_offset: qt_property!(f64),
    gyro_offset_frames: qt_property!(i32),
    /// 平滑算法名列表（JSON array），供「平滑方式」下拉。
    smoothing_methods: qt_method!(
        fn smoothing_methods(&self) -> QString {
            let stab = gyroflow_core::StabilizationManager::default();
            let names = stab.get_smoothing_algs();
            QString::from(serde_json::to_string(&names).unwrap_or_else(|_| "[]".into()).as_str())
        }
    ),
    /// 当前平滑方式的参数定义 + 当前值（`get_parameters_json()`）：name/description/from/to/value/…
    /// 供 QML 动态渲染滑杆；改完写回 `gyro_smoothing_params`。
    smoothing_params: qt_method!(
        fn smoothing_params(&self) -> QString {
            let stab = gyroflow_core::StabilizationManager::default();
            let method = self.gyro_smoothing_method.to_string();
            let algs = stab.get_smoothing_algs();
            let idx = algs
                .iter()
                .position(|m| m.eq_ignore_ascii_case(&method))
                .unwrap_or_else(|| if algs.len() > 1 { 1 } else { 0 });
            stab.smoothing.write().set_current(idx);
            if let Ok(serde_json::Value::Array(list)) =
                serde_json::from_str(&self.gyro_smoothing_params.to_string())
            {
                let mut sm = stab.smoothing.write();
                for p in list {
                    if let (Some(n), Some(v)) = (
                        p.get("name").and_then(|x| x.as_str()),
                        p.get("value").and_then(|x| x.as_f64()),
                    ) {
                        sm.current_mut().set_parameter(n, v);
                    }
                }
            }
            let v = stab.smoothing.read().current().get_parameters_json();
            QString::from(v.to_string().as_str())
        }
    ),
    /// 选中镜头标定后取它自带的相机属性（`{"frame_readout_time":…,"gyro_readout_dir":…}`），
    /// 由 QML 填进「稳定化」页 —— 参数以设置页为唯一真源，必须落到控件上才生效。
    lens_fields: qt_method!(
        fn lens_fields(&self, id: QString) -> QString {
            let v = crate::lens_db::lens_fields(&id.to_string());
            QString::from(v.to_string().as_str())
        }
    ),
    /// 读一个 `.gyroflow` 的 `stabilization` 段并映射成**本代理的属性名 → 值**的 JSON，
    /// 让 QML 选中/拖入预设时一次性填进设置页（之后照常手改，不再依赖该文件）。
    preset_fields: qt_method!(
        fn preset_fields(&self, path: QString) -> QString {
            let p = crate::settings::decode_path(&path.to_string());
            let Ok(text) = std::fs::read_to_string(&p) else {
                return QString::from("{}");
            };
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
                return QString::from("{}");
            };
            let mut g = Settings::default().gyro;
            g.apply_preset_json(&v);
            let out = serde_json::json!({
                "fov": g.fov,
                "max_zoom": g.max_zoom,
                "max_zoom_iterations": g.max_zoom_iterations,
                "gyro_zoom_mode": g.zoom_mode,
                "gyro_zoom_window": g.adaptive_zoom_window,
                "gyro_zoom_center_x": g.zoom_center_x,
                "gyro_zoom_center_y": g.zoom_center_y,
                "gyro_lens": g.lens_correction_amount,
                "gyro_horizon": g.horizon_lock,
                "gyro_horizon_roll": g.horizon_lock_roll,
                "gyro_horizon_enabled": g.horizon_lock_enabled,
                "gyro_auto_lock": g.horizon_automatic_lock,
                "gyro_horizon_pitch_enabled": g.horizon_lock_pitch_enabled,
                "gyro_horizon_pitch": g.horizon_lock_pitch,
                "gyro_integration_method": g.horizon_lock_integration_method,
                "gyro_turn_threshold": g.horizon_turn_threshold,
                "gyro_turn_smoothing": g.horizon_turn_smoothing_ms,
                "gyro_turn_multiplier": g.horizon_turn_multiplier,
                "gyro_limit_tilt_accel": g.horizon_limit_tilt_accel,
                "gyro_tilt_accel_limit": g.horizon_tilt_accel_limit,
                "frame_readout_enabled": g.frame_readout_enabled,
                "gyro_fl_smoothing": g.focal_length_smoothing_enabled,
                "gyro_fl_strength": g.focal_length_smoothing_strength,
                "gyro_use_gravity": g.use_gravity_vectors,
                "frame_readout_time": g.frame_readout_time,
                "gyro_readout_dir": g.frame_readout_direction,
                "gyro_smoothing_method": g.smoothing_method,
                "gyro_smoothing_params": g.smoothing_params,
                "gyro_video_speed": g.video_speed,
                "gyro_vs_smoothing": g.video_speed_affects_smoothing,
                "gyro_vs_zooming": g.video_speed_affects_zooming,
                "gyro_vs_zoom_limit": g.video_speed_affects_zooming_limit,
                "gyro_rot_x": g.additional_rotation[0],
                "gyro_rot_y": g.additional_rotation[1],
                "gyro_rot_z": g.additional_rotation[2],
                "gyro_tr_x": g.additional_translation[0],
                "gyro_tr_y": g.additional_translation[1],
                "gyro_tr_z": g.additional_translation[2],
            });
            QString::from(out.to_string().as_str())
        }
    ),

    // ---- encode ----
    encode_backend: qt_property!(QString),
    encode_format: qt_property!(QString),
    encode_rate_control: qt_property!(QString),
    encode_quality: qt_property!(f64),
    encode_bitrate: qt_property!(QString),
    encode_preset: qt_property!(QString),
    encode_audio_codec: qt_property!(QString),
    /// 「自定义时间」自己填的起止（毫秒；0 = 开头 / 到末尾）。读写的是 `encode.range_*`，
    /// 不是在导出时按模式推导的 `clip_*`，所以切到别的挡位再切回来值还在。
    encode_clip_start_ms: qt_property!(f64),
    encode_clip_end_ms: qt_property!(f64),
    encode_start_seg: qt_property!(i32),
    encode_end_seg: qt_property!(i32),
    /// 「导出范围」下拉：`none`（全部）/ `all`（逐段导出）/ `custom`（自定义分段 xx-xx）/
    /// `time`（自定义时间）。UI 的唯一真源，落盘成 `clipMode` 并推导底层的
    /// `seg_enabled` / `all_seg`（.orv 向后兼容）。
    encode_seg_mode: qt_property!(QString),
    encode_seg_enabled: qt_property!(bool),
    /// 「全部逐段导出」：勾选后忽略开始/结束分段，导出每个分段各一个文件。
    encode_all_seg: qt_property!(bool),
    encode_total_segs: qt_property!(i32),
    encode_resolution: qt_property!(QString),
    encode_fps: qt_property!(QString),
    /// 导出倍速 token："0.8"|"1"|"1.5"|"2"|"3"|"4"。
    encode_speed: qt_property!(QString),
    encode_include_map: qt_property!(bool),
    encode_include_video: qt_property!(bool),
    encode_include_hud: qt_property!(bool),

    // ---- hud ----
    hud_layout: qt_property!(QString),
    hud_scope: qt_property!(QString),
    hud_gpx_offset: qt_property!(f64),
    hud_theme: qt_method!(
        fn hud_theme(&self, name: QString) -> QString {
            let layout = crate::hud::Layout::theme(&name.to_string());
            QString::from(layout.json().as_str())
        }
    ),
    hud_templates: qt_method!(
        fn hud_templates(&self) -> QString {
            QString::from(
                serde_json::to_string(&crate::hud::templates())
                    .unwrap()
                    .as_str(),
            )
        }
    ),
    hud_template_action: qt_method!(
        fn hud_template_action(&self, action: QString, id: QString, name: QString) -> QString {
            let result = crate::hud::template_action(
                &action.to_string(),
                &id.to_string(),
                &name.to_string(),
                &self.hud_layout.to_string(),
            );
            let response = match result {
                Ok(value) => serde_json::json!({"ok":true,"value":value}),
                Err(e) => serde_json::json!({"ok":false,"error":e}),
            };
            QString::from(response.to_string().as_str())
        }
    ),
    hud_show_elapsed_time: qt_property!(bool),
    hud_show_segment_time: qt_property!(bool),
    hud_show_speed: qt_property!(bool),
    hud_show_pace: qt_property!(bool),
    hud_show_hr: qt_property!(bool),
    hud_show_cadence: qt_property!(bool),
    hud_show_distance: qt_property!(bool),
    hud_show_azimuth: qt_property!(bool),
    hud_show_elevation: qt_property!(bool),

    /// 防抖预设（`.gyroflow`）下拉列表的候选：扫描固定目录（exe 同级 `presets/`、
    /// 运行目录 `presets/`、`%APPDATA%\TraceReel\presets`）里的全部 `.gyroflow`，
    /// 返回 JSON `{dirs:[...], presets:[{name,path,note,current}]}`。做法对应原版
    /// Gyroflow 的镜头预设列表（见 `crate::presets`）。`current` 传设置里当前的
    /// 预设路径 —— 它即使不在任何目录里也会被列出来，不会被下拉列表丢掉。
    preset_list: qt_method!(
        fn preset_list(&self, current: QString) -> QString {
            let v = crate::presets::list(&current.to_string());
            QString::from(v.to_string().as_str())
        }
    ),
    /// 防抖搜索框：同时搜「.gyroflow 预设」（我们的预设目录）与「镜头标定」
    /// （Gyroflow 镜头库，实测 12409 条）。返回 JSON
    /// `{ready, total, db, items:[{kind:"preset"|"lens", id, name, note}]}`。
    /// 库里选中的 `id` 存进 `gyro_lens_profile`（`gyro.rs` 装载时覆盖预设的 `calibration_data`）。
    /// 整库在后台只解析一次（1.4s），未就绪时 `ready=false`、`items` 只有预设文件命中。
    search_profiles: qt_method!(
        fn search_profiles(&self, text: QString, limit: i32) -> QString {
            let v = crate::lens_db::search(&text.to_string(), limit.max(1) as usize);
            QString::from(v.to_string().as_str())
        }
    ),
    /// 后台预热镜头库（GUI 启动时调用一次，避免第一次输入卡 1.4s）。
    profiles_warmup: qt_method!(
        fn profiles_warmup(&self) {
            crate::lens_db::warmup();
        }
    ),
    /// 「分段 i 在视频时间轴上的毫秒位置」（供「分段」页的跳转按钮）：
    /// idx=0 = 出发时刻，idx=k = 第 k 个分段点；换算与导出用的边界完全一致。
    split_video_ms: qt_method!(
        fn split_video_ms(&self, index: i32) -> f64 {
            self.collect_settings().split_video_ms(index.max(0) as usize)
        }
    ),
    /// 「按分段设置换图时间」用：每一段的**起点时刻**，`|` 拼接、已格式化成
    /// `h:mm:ss` / `m:ss`。第 1 段起点恒为 `0:00`，第 k 段起点 = 第 k-1 个分段时间
    /// （相对时间模式按累加处理），所以条目数 = 分段数。
    split_starts: qt_method!(
        fn split_starts(&self) -> QString {
            let starts = tracereel_core::splits::segment_start_times(
                &self.split_times.to_string(),
                self.split_is_relative,
            );
            let text: Vec<String> = starts
                .iter()
                .map(|s| crate::gui::fmt_split_seconds(*s))
                .collect();
            QString::from(text.join("|").as_str())
        }
    ),
    /// Serialize the current settings to a TEMP file and return its path, so the preview
    /// can apply changes immediately WITHOUT auto-saving to the user's project file.
    /// The only way the project file is written is the explicit 保存设置 / 另存为 (`save_to`).
    export_snapshot: qt_method!(fn export_snapshot(&self) -> QString {
        QString::from(self.collect_settings().to_ini())
    }),
    temp_apply_path: qt_method!(
        fn temp_apply_path(&self) -> QString {
            let s = self.collect_settings();
            let path = std::env::temp_dir()
                .join("tracereel_autosave.trr")
                .to_string_lossy()
                .into_owned();
            let _ = s.save_orv(&path);
            QString::from(path.as_str())
        }
    ),
    /// Write current proxy fields to an explicit path (Save As).
    save_to: qt_method!(
        fn save_to(&mut self, path: QString) {
            let s = self.collect_settings();
            let p = crate::settings::decode_path(&path.to_string());
            let _ = s.save_orv(&p);
            self.apply_settings(&s);
            self.file_path = QString::from(p.as_str());
        }
    ),
    /// Populate the proxy from a settings file at `path` (no app reload).
    load_settings_from: qt_method!(
        fn load_settings_from(&mut self, path: QString) {
            let p = crate::settings::decode_path(&path.to_string());
            let s = Settings::load_orv(&p);
            self.apply_settings(&s);
            self.file_path = QString::from(p.as_str());
        }
    ),
    /// Re-read from the current file_path.
    reload_settings: qt_method!(
        fn reload_settings(&mut self) {
            self.load_settings_from(QString::from(self.file_path.to_string().as_str()));
        }
    ),
    /// Reset all proxy fields to defaults (does not touch the file on disk).
    reset_defaults: qt_method!(
        fn reset_defaults(&mut self) {
            self.apply_settings(&Settings::default());
        }
    ),
}

impl SettingsProxy {
    /// Copy a loaded Settings into the proxy properties.
    pub fn apply_settings(&mut self, s: &Settings) {
        self.video_path = QString::from(s.video.video_path.as_str());
        self.video_extras = QString::from(s.video.extra_videos.join("|").as_str());
        self.video_start_offset = s.video.start_time_offset_s;
        self.loop_playback = s.video.loop_playback;
        self.video_rescale_shader = QString::from(s.video.rescale_shader.as_str());
        self.video_verbose = s.video.enable_verbose_logging;
        self.video_seek_any = s.video.seek_to_any_frame;
        self.video_frame_count_div = s.video.frame_count_divisor;
        self.video_frame_duration_div = s.video.frame_duration_divisor;

        self.window_w = s.window.2;
        self.window_h = s.window.3;
        self.window_fullscreen = s.window_fullscreen;
        self.window_hide_cursor = s.window_hide_cursor;
        self.window_multisamples = s.window_multisamples;
        self.preview_perf = s.preview_perf;

        // 多地图框：box0 由 Settings 的旧单框字段/取景字段反推（单一真相），
        // boxes[1..] 原样；QML 编辑器直接 parse 这个 JSON。
        self.map_boxes = QString::from(s.map.effective_layout(&s.route).json().as_str());
        self.map_scale = s.map.scale;
        self.map_header_crop = s.map.header_crop;
        self.map_background_color = QString::from(s.map.background_color.as_str());
        self.map_path = QString::from(s.map.image_file_path.as_str());
        self.map_extras = QString::from(s.map.extra_map_files.join("|").as_str());
        self.map_change_times = QString::from(
            s.map
                .extra_map_change_times
                .iter()
                .map(|t| t.map(crate::gui::fmt_split_seconds).unwrap_or_default())
                .collect::<Vec<_>>()
                .join("|")
                .as_str(),
        );

        self.route_render_mode = s.route.render_mode;
        self.route_render_scope = s.route.render_scope;
        self.route_tail_render_mode = s.route.tail_render_mode;
        self.route_width = s.route.width;
        self.route_tail_width = s.route.tail_width;
        self.route_tail_length = s.route.tail_length;
        self.route_control_radius = s.route.control_radius;
        self.route_runner_radius = s.route.runner_radius;
        self.route_show_controls = s.route.show_controls;
        self.route_show_runner = s.route.show_runner;
        self.route_color = QString::from(s.route.route_color.as_str());
        self.route_opacity = s.route.route_opacity;
        self.route_tail_color = QString::from(s.route.tail_color.as_str());
        self.route_tail_opacity = s.route.tail_opacity;
        self.route_control_color = QString::from(s.route.control_color.as_str());
        self.route_control_opacity = s.route.control_opacity;
        self.route_runner_color = QString::from(s.route.runner_color.as_str());
        self.route_runner_opacity = s.route.runner_opacity;
        self.route_runner_style = s.route.runner_style;
        self.route_low_pace = s.route.low_pace;
        self.route_tail_low_pace = s.route.tail_low_pace;
        self.route_high_pace = s.route.high_pace;
        self.route_tail_high_pace = s.route.tail_high_pace;
        self.route_pace_auto = s.route.pace_auto;
        self.route_tail_pace_auto = s.route.tail_pace_auto;
        self.route_control_time_offset = s.route.control_time_offset;
        self.route_runner_time_offset = s.route.runner_time_offset;
        self.route_path = QString::from(s.route.quick_route_jpeg_file_path.as_str());

        self.split_times = QString::from(s.splits.split_times.as_str());
        self.split_point_codes = QString::from(s.splits.point_codes.as_str());
        self.split_is_relative = s.splits.is_relative;

        self.gyro_enabled = s.gyro.enabled;
        self.gyro_preset = QString::from(s.gyro.preset_file_path.as_str());
        self.gyro_lens_profile = QString::from(s.gyro.lens_profile.as_str());
        self.gyro_lens_profile_name = QString::from(s.gyro.lens_profile_name.as_str());
        self.fov = s.gyro.fov;
        self.max_zoom = s.gyro.max_zoom;
        self.max_zoom_iterations = s.gyro.max_zoom_iterations;
        self.gyro_zoom_mode = s.gyro.zoom_mode;
        self.gyro_zoom_window = s.gyro.adaptive_zoom_window;
        self.gyro_zoom_center_x = s.gyro.zoom_center_x;
        self.gyro_zoom_center_y = s.gyro.zoom_center_y;
        self.gyro_lens = s.gyro.lens_correction_amount;
        self.gyro_horizon = s.gyro.horizon_lock;
        self.gyro_horizon_roll = s.gyro.horizon_lock_roll;
        self.gyro_horizon_enabled = s.gyro.horizon_lock_enabled;
        self.gyro_auto_lock = s.gyro.horizon_automatic_lock;
        self.gyro_horizon_pitch_enabled = s.gyro.horizon_lock_pitch_enabled;
        self.gyro_horizon_pitch = s.gyro.horizon_lock_pitch;
        self.gyro_integration_method = s.gyro.horizon_lock_integration_method;
        self.gyro_turn_threshold = s.gyro.horizon_turn_threshold;
        self.gyro_turn_smoothing = s.gyro.horizon_turn_smoothing_ms;
        self.gyro_turn_multiplier = s.gyro.horizon_turn_multiplier;
        self.gyro_limit_tilt_accel = s.gyro.horizon_limit_tilt_accel;
        self.gyro_tilt_accel_limit = s.gyro.horizon_tilt_accel_limit;
        self.frame_readout_enabled = s.gyro.frame_readout_enabled;
        self.gyro_fl_smoothing = s.gyro.focal_length_smoothing_enabled;
        self.gyro_fl_strength = s.gyro.focal_length_smoothing_strength;
        self.gyro_use_gravity = s.gyro.use_gravity_vectors;
        self.frame_readout_time = s.gyro.frame_readout_time;
        self.gyro_readout_dir = QString::from(s.gyro.frame_readout_direction.as_str());
        self.gyro_smoothing_method = QString::from(s.gyro.smoothing_method.as_str());
        self.gyro_smoothing_params = QString::from(s.gyro.smoothing_params.as_str());
        self.gyro_video_speed = s.gyro.video_speed;
        self.gyro_vs_smoothing = s.gyro.video_speed_affects_smoothing;
        self.gyro_vs_zooming = s.gyro.video_speed_affects_zooming;
        self.gyro_vs_zoom_limit = s.gyro.video_speed_affects_zooming_limit;
        self.gyro_rot_x = s.gyro.additional_rotation[0];
        self.gyro_rot_y = s.gyro.additional_rotation[1];
        self.gyro_rot_z = s.gyro.additional_rotation[2];
        self.gyro_tr_x = s.gyro.additional_translation[0];
        self.gyro_tr_y = s.gyro.additional_translation[1];
        self.gyro_tr_z = s.gyro.additional_translation[2];
        self.gyro_offset = s.gyro.offset_ms;
        self.gyro_offset_frames = s.gyro.offset_frames;

        self.encode_backend = QString::from(s.encode.backend.as_str());
        self.encode_format = QString::from(s.encode.format.as_str());
        self.encode_rate_control = QString::from(s.encode.rate_control.as_str());
        self.encode_quality = s.encode.quality;
        self.encode_bitrate = QString::from(s.encode.bitrate.as_str());
        self.encode_preset = QString::from(s.encode.preset.as_str());
        self.encode_audio_codec = QString::from(s.encode.audio_codec.as_str());
        self.encode_clip_start_ms = s.encode.range_start_ms;
        self.encode_clip_end_ms = s.encode.range_end_ms;
        self.encode_start_seg = s.encode.start_seg;
        self.encode_end_seg = s.encode.end_seg;
        self.encode_seg_enabled = s.encode.seg_enabled;
        self.encode_all_seg = s.encode.all_seg_enabled;
        // 「导出范围」的四种取值以 clip_mode 为准（"time" 无法从两个旧开关反推）。
        let mode = s.encode.clip_mode.trim().to_ascii_lowercase();
        self.encode_seg_mode = QString::from(if matches!(mode.as_str(), "none" | "all" | "custom" | "time") {
            mode.as_str()
        } else if !s.encode.seg_enabled {
            "none"
        } else if s.encode.all_seg_enabled {
            "all"
        } else {
            "custom"
        });
        self.encode_total_segs = s.encode_total_segments();
        self.encode_resolution = QString::from(s.encode.resolution.as_str());
        self.encode_fps = QString::from(s.encode.frame_rate.as_str());
        self.encode_speed = QString::from(s.encode.speed.as_str());
        self.encode_include_map = s.encode.include_map;
        self.encode_include_video = s.encode.include_video;
        self.encode_include_hud = s.encode.include_hud;
        self.encode_output_path = QString::from(s.encode.output_file_path.as_str());

        self.hud_scope = QString::from(s.hud.scope.as_str());
        self.hud_gpx_path = QString::from(s.hud.gpx_file_path.as_str());
        self.hud_layout = QString::from(
            s.hud
                .layout
                .as_ref()
                .map(|l| l.json())
                .unwrap_or_else(|| crate::hud::Layout::classic(s).json())
                .as_str(),
        );
        self.hud_gpx_offset = s.hud.gpx_time_offset;
        self.hud_show_elapsed_time = s.hud.show_elapsed_time;
        self.hud_show_segment_time = s.hud.show_segment_time;
        self.hud_show_speed = s.hud.show_speed;
        self.hud_show_pace = s.hud.show_pace;
        self.hud_show_hr = s.hud.show_heart_rate;
        self.hud_show_cadence = s.hud.show_cadence;
        self.hud_show_distance = s.hud.show_distance;
        self.hud_show_azimuth = s.hud.show_azimuth;
        self.hud_show_elevation = s.hud.show_elevation;
    }

    /// Build a Settings from the current proxy properties.
    pub fn collect_settings(&self) -> Settings {
        let mut s = Settings::default();
        s.video.video_path = self.video_path.to_string();
        s.video.extra_videos = self
            .video_extras
            .to_string()
            .split('|')
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect();
        s.video.start_time_offset_s = self.video_start_offset;
        s.video.loop_playback = self.loop_playback;
        s.video.rescale_shader = self.video_rescale_shader.to_string();
        s.video.enable_verbose_logging = self.video_verbose;
        s.video.seek_to_any_frame = self.video_seek_any;
        s.video.frame_count_divisor = self.video_frame_count_div;
        s.video.frame_duration_divisor = self.video_frame_duration_div;

        s.window.2 = self.window_w;
        s.window.3 = self.window_h;
        s.window_fullscreen = self.window_fullscreen;
        s.window_hide_cursor = self.window_hide_cursor;
        s.window_multisamples = self.window_multisamples;
        s.preview_perf = self.preview_perf;

        s.map.image_file_path = self.map_path.to_string();
        {
            // 额外地图与换图时间按索引对齐；跳过空路径行，但**空时间要保留成 `None`**
            // （没填 = 那张图暂不参与播放），否则它会退化成 0 秒并抢占整条时间轴。
            // 换图时间按分段时间格式解析（h:mm:ss / mm:ss / 纯秒数），与"分段时间"一致。
            let files_raw: Vec<String> = self
                .map_extras
                .to_string()
                .split('|')
                .map(|p| p.trim().to_string())
                .collect();
            let times_raw: Vec<Option<f64>> = self
                .map_change_times
                .to_string()
                .split('|')
                .map(|p| tracereel_core::splits::parse_time_string(p.trim()).ok())
                .collect();
            let mut files = Vec::new();
            let mut times = Vec::new();
            for (i, p) in files_raw.iter().enumerate() {
                if p.is_empty() {
                    continue;
                }
                files.push(p.clone());
                times.push(times_raw.get(i).copied().flatten());
            }
            s.map.extra_map_files = files;
            s.map.extra_map_change_times = times;
        }
        // 多地图框：`map_boxes` 是新 UI 的唯一真相。为空串（旧代码/异常路径）或 JSON
        // 坏掉时回退为单框，避免把工程变成 0 框；不会让已存在的第 1 框配置丢失。
        s.map.layout = match MapLayout::parse(&self.map_boxes.to_string()) {
            Ok(layout) => layout,
            Err(e) => {
                if !self.map_boxes.to_string().trim().is_empty() {
                    ::log::warn!("[settings] 设置代理的地图框布局解析失败，回退为单框: {e}");
                }
                MapLayout::from_legacy(&s.map, &s.route)
            }
        };
        s.map.scale = self.map_scale;
        s.map.header_crop = self.map_header_crop;
        s.map.background_color = self.map_background_color.to_string();

        s.route.quick_route_jpeg_file_path = self.route_path.to_string();
        s.route.render_mode = self.route_render_mode;
        s.route.render_scope = self.route_render_scope;
        s.route.tail_render_mode = self.route_tail_render_mode;
        s.route.width = self.route_width;
        s.route.tail_width = self.route_tail_width;
        s.route.tail_length = self.route_tail_length;
        s.route.control_radius = self.route_control_radius;
        s.route.runner_radius = self.route_runner_radius;
        s.route.show_controls = self.route_show_controls;
        s.route.show_runner = self.route_show_runner;
        s.route.route_color = self.route_color.to_string();
        s.route.route_opacity = self.route_opacity;
        s.route.tail_color = self.route_tail_color.to_string();
        s.route.tail_opacity = self.route_tail_opacity;
        s.route.control_color = self.route_control_color.to_string();
        s.route.control_opacity = self.route_control_opacity;
        s.route.runner_color = self.route_runner_color.to_string();
        s.route.runner_opacity = self.route_runner_opacity;
        s.route.runner_style = self.route_runner_style;
        s.route.low_pace = self.route_low_pace;
        s.route.tail_low_pace = self.route_tail_low_pace;
        s.route.high_pace = self.route_high_pace;
        s.route.tail_high_pace = self.route_tail_high_pace;
        s.route.pace_auto = self.route_pace_auto;
        s.route.tail_pace_auto = self.route_tail_pace_auto;
        s.route.control_time_offset = self.route_control_time_offset;
        s.route.runner_time_offset = self.route_runner_time_offset;
        // 镜像不变量：逐框取景的 box0 → 旧的 route 取景字段（预览/导出在逐框改造
        // 完成前仍只读 route.view_*）。
        s.map.sync_legacy_from_layout(&mut s.route);

        s.splits.split_times = self.split_times.to_string();
        s.splits.point_codes = self.split_point_codes.to_string();
        s.splits.is_relative = self.split_is_relative;

        s.gyro.enabled = self.gyro_enabled;
        s.gyro.preset_file_path = self.gyro_preset.to_string();
        s.gyro.lens_profile = self.gyro_lens_profile.to_string();
        s.gyro.lens_profile_name = self.gyro_lens_profile_name.to_string();
        s.gyro.fov = self.fov;
        s.gyro.max_zoom = self.max_zoom;
        s.gyro.max_zoom_iterations = self.max_zoom_iterations.max(1);
        s.gyro.zoom_mode = self.gyro_zoom_mode;
        s.gyro.adaptive_zoom_window = self.gyro_zoom_window;
        s.gyro.zoom_center_x = self.gyro_zoom_center_x;
        s.gyro.zoom_center_y = self.gyro_zoom_center_y;
        s.gyro.lens_correction_amount = self.gyro_lens;
        s.gyro.horizon_lock = self.gyro_horizon;
        s.gyro.horizon_lock_roll = self.gyro_horizon_roll;
        s.gyro.horizon_lock_enabled = self.gyro_horizon_enabled;
        s.gyro.horizon_automatic_lock = self.gyro_auto_lock;
        s.gyro.horizon_lock_pitch_enabled = self.gyro_horizon_pitch_enabled;
        s.gyro.horizon_lock_pitch = self.gyro_horizon_pitch;
        s.gyro.horizon_lock_integration_method = self.gyro_integration_method;
        s.gyro.horizon_turn_threshold = self.gyro_turn_threshold;
        s.gyro.horizon_turn_smoothing_ms = self.gyro_turn_smoothing;
        s.gyro.horizon_turn_multiplier = self.gyro_turn_multiplier;
        s.gyro.horizon_limit_tilt_accel = self.gyro_limit_tilt_accel;
        s.gyro.horizon_tilt_accel_limit = self.gyro_tilt_accel_limit;
        s.gyro.frame_readout_enabled = self.frame_readout_enabled;
        s.gyro.focal_length_smoothing_enabled = self.gyro_fl_smoothing;
        s.gyro.focal_length_smoothing_strength = self.gyro_fl_strength;
        s.gyro.use_gravity_vectors = self.gyro_use_gravity;
        s.gyro.frame_readout_time = self.frame_readout_time;
        s.gyro.frame_readout_direction = self.gyro_readout_dir.to_string();
        s.gyro.smoothing_method = self.gyro_smoothing_method.to_string();
        s.gyro.smoothing_params = self.gyro_smoothing_params.to_string();
        s.gyro.video_speed = self.gyro_video_speed;
        s.gyro.video_speed_affects_smoothing = self.gyro_vs_smoothing;
        s.gyro.video_speed_affects_zooming = self.gyro_vs_zooming;
        s.gyro.video_speed_affects_zooming_limit = self.gyro_vs_zoom_limit;
        s.gyro.additional_rotation = [self.gyro_rot_x, self.gyro_rot_y, self.gyro_rot_z];
        s.gyro.additional_translation = [self.gyro_tr_x, self.gyro_tr_y, self.gyro_tr_z];
        s.gyro.offset_ms = self.gyro_offset;
        s.gyro.offset_frames = self.gyro_offset_frames;

        s.encode.backend = self.encode_backend.to_string();
        s.encode.format = self.encode_format.to_string();
        s.encode.rate_control = self.encode_rate_control.to_string();
        s.encode.quality = self.encode_quality;
        s.encode.bitrate = self.encode_bitrate.to_string();
        s.encode.preset = self.encode_preset.to_string();
        s.encode.audio_codec = self.encode_audio_codec.to_string();
        // 「自定义时间」的那两个框：存到 range_*（clip_* 是导出时按模式推导的，会被归零）。
        s.encode.range_start_ms = self.encode_clip_start_ms;
        s.encode.range_end_ms = self.encode_clip_end_ms;
        s.encode.start_seg = self.encode_start_seg;
        s.encode.end_seg = self.encode_end_seg;
        // 分段导出：下拉模式是唯一真源，由它推出底层两个开关（老工程按开关推出模式，见 apply_settings）。
        // 只认四种取值；下拉之外（脚本、手改 QML）传进来的未知值按「全部」处理。
        let mut seg_mode = self.encode_seg_mode.to_string().to_ascii_lowercase();
        if !matches!(seg_mode.as_str(), "none" | "all" | "custom" | "time") {
            seg_mode = "none".to_string();
        }
        s.encode.clip_mode = seg_mode.clone();
        match seg_mode.as_str() {
            "none" => {
                s.encode.seg_enabled = false;
                s.encode.all_seg_enabled = false;
            }
            "all" => {
                s.encode.seg_enabled = true;
                s.encode.all_seg_enabled = true;
            }
            // 「自定义时间」不走分段推导：起止时间就是 encode_clip_start_ms/end_ms（上面已写入）。
            "time" => {
                s.encode.seg_enabled = false;
                s.encode.all_seg_enabled = false;
            }
            _ => {
                s.encode.seg_enabled = true;
                s.encode.all_seg_enabled = false;
            }
        }
        s.encode.resolution = self.encode_resolution.to_string();
        s.encode.frame_rate = self.encode_fps.to_string();
        s.encode.speed = self.encode_speed.to_string();
        s.encode.include_map = self.encode_include_map;
        s.encode.include_video = self.encode_include_video;
        s.encode.include_hud = self.encode_include_hud;
        s.encode.map_only = !s.encode.include_video;
        s.encode.output_file_path = self.encode_output_path.to_string();

        // 白名单：HUD 只认「视频区 / 整幅画面」，未知值回到默认「视频区」。
        s.hud.scope = match self.hud_scope.to_string().trim().to_ascii_lowercase().as_str() {
            "frame" => "frame".to_string(),
            _ => "video".to_string(),
        };
        s.hud.gpx_file_path = self.hud_gpx_path.to_string();
        s.hud.layout = crate::hud::Layout::parse(&self.hud_layout.to_string()).ok();
        s.hud.gpx_time_offset = self.hud_gpx_offset;
        s.hud.show_elapsed_time = self.hud_show_elapsed_time;
        s.hud.show_segment_time = self.hud_show_segment_time;
        s.hud.show_speed = self.hud_show_speed;
        s.hud.show_pace = self.hud_show_pace;
        s.hud.show_heart_rate = self.hud_show_hr;
        s.hud.show_cadence = self.hud_show_cadence;
        s.hud.show_distance = self.hud_show_distance;
        s.hud.show_azimuth = self.hud_show_azimuth;
        s.hud.show_elevation = self.hud_show_elevation;

        // Segment selections → concrete clip bounds (needs splits + video offset, set above).
        s.resolve_encode_clip_ms();

        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 旧代码只改 `map.frame_*`（单框字段）时：代理的 `map_boxes` 必须能看到同一个
    /// 第 1 框，且保存/加载后不丢（走「单框工程不写 boxes 键」的旧键路径）。
    #[test]
    fn legacy_map_frame_flows_through_map_boxes_and_roundtrips() {
        let mut s = Settings::default();
        for mode in [0, 1, 4] {
            s.map.placement = mode;
            s.map.frame_x = -0.1;
            s.map.frame_y = 0.2;
            s.map.frame_width = 0.45;
            s.map.frame_height = 0.7;
            s.map.frame_rotation = -37.0;
            s.map.frame_opacity = 0.65;
            s.map.frame_shape = 1;
            s.map.frame_feather = 0.12;
            s.map.frame_corner = 0.2;
            let mut proxy = SettingsProxy::default();
            proxy.apply_settings(&s);
            let boxes = MapLayout::parse(&proxy.map_boxes.to_string()).unwrap();
            assert_eq!(boxes.boxes.len(), 1, "旧单框字段应映射成 1 个框");
            assert_eq!(boxes.boxes[0].mode, mode);
            assert_eq!(boxes.boxes[0].x, -0.1);
            let collected = proxy.collect_settings();
            let loaded = Settings::from_ini(&crate::settings::IniFile::parse(&collected.to_ini()));
            assert_eq!(loaded.map.placement, mode);
            assert_eq!(loaded.map.frame_x, -0.1);
            assert_eq!(loaded.map.frame_y, 0.2);
            assert_eq!(loaded.map.frame_width, 0.45);
            assert_eq!(loaded.map.frame_height, 0.7);
            assert_eq!(loaded.map.frame_rotation, -37.0);
            assert_eq!(loaded.map.frame_opacity, 0.65);
            assert_eq!(loaded.map.frame_shape, 1);
            assert_eq!(loaded.map.frame_feather, 0.12);
            assert_eq!(loaded.map.frame_corner, 0.2);
        }
    }

    /// 两框（box0 贴边固定分段 + box1 自由摆放跟跑者居中）经代理 + `.trr` 双向往返。
    #[test]
    fn two_map_boxes_survive_proxy_and_project_roundtrip() {
        let mut s = Settings::default();
        let mut layout = MapLayout::new_single();
        layout.boxes[0] = crate::settings::MapBox {
            mode: 0,
            share: 0.3,
            view_mode: 0,
            lr_margin: 12.5,
            tb_margin: 47.0,
            ..Default::default()
        };
        layout.boxes.push(crate::settings::MapBox {
            mode: 4,
            x: 0.62,
            y: 0.55,
            w: 0.34,
            h: 0.4,
            shape: 1,
            feather: 0.1,
            view_mode: 1,
            view_zoom: 0.37,
            sensitivity: 2.5,
            ..Default::default()
        });
        s.map.layout = layout;
        s.map.sync_legacy_from_layout(&mut s.route);

        let mut proxy = SettingsProxy::default();
        proxy.apply_settings(&s);
        let from_proxy = MapLayout::parse(&proxy.map_boxes.to_string()).unwrap();
        assert_eq!(from_proxy.boxes.len(), 2);
        assert_eq!(from_proxy.boxes[1].mode, 4);
        assert_eq!(from_proxy.boxes[1].view_mode, 1);
        assert_eq!(from_proxy.boxes[1].view_zoom, 0.37);

        let collected = proxy.collect_settings();
        assert_eq!(collected.map.layout.boxes.len(), 2, "多框必须随 collect 带上");
        assert_eq!(collected.route.view_mode, 0, "box0 的取景要镜像回旧字段");
        assert_eq!(collected.route.left_right_margin, 12.5);

        let loaded = Settings::from_ini(&crate::settings::IniFile::parse(&collected.to_ini()));
        assert_eq!(loaded.map.layout.boxes.len(), 2, "两框要写进 [map] boxes");
        assert_eq!(loaded.map.layout.boxes[1].x, 0.62);
        assert_eq!(loaded.map.layout.boxes[1].view_zoom, 0.37);
        assert_eq!(loaded.map.layout.boxes[1].sensitivity, 2.5);
    }

    /// 逐框轨迹样式经代理 + `.trr` 双向往返；box0 样式镜像回全局 route 显示字段。
    #[test]
    fn two_box_styles_survive_proxy_and_project_roundtrip() {
        use crate::settings::{MapBox, MapBoxStyle};
        let mut s = Settings::default();
        let mut layout = MapLayout::new_single();
        layout.boxes[0].style = Some(MapBoxStyle {
            render_mode: 1,
            width: 18.0,
            route_color: "#112233".to_string(),
            show_runner: false,
            ..Default::default()
        });
        layout.boxes.push(MapBox {
            mode: 4,
            x: 0.6,
            y: 0.1,
            w: 0.3,
            h: 0.3,
            style: Some(MapBoxStyle {
                render_mode: 3,
                tail_render_mode: 1,
                tail_width: 14.0,
                runner_color: "#00ff00".to_string(),
                low_pace: 6.0,
                high_pace: 4.0,
                pace_auto: false,
                ..Default::default()
            }),
            ..Default::default()
        });
        s.map.layout = layout;
        s.map.sync_legacy_from_layout(&mut s.route);

        let mut proxy = SettingsProxy::default();
        proxy.apply_settings(&s);
        // 全局 route_* 代理属性保留为 box0 镜像（兼容旧测试/旧代码）。
        assert_eq!(proxy.route_width, 18.0);
        assert_eq!(proxy.route_render_mode, 1);
        assert!(!proxy.route_show_runner);
        let parsed = MapLayout::parse(&proxy.map_boxes.to_string()).unwrap();
        assert_eq!(parsed.boxes.len(), 2);
        assert_eq!(parsed.boxes[0].style.as_ref().unwrap().width, 18.0);
        let b1 = parsed.boxes[1].style.as_ref().unwrap();
        assert_eq!(b1.render_mode, 3);
        assert_eq!(b1.tail_width, 14.0);
        assert_eq!(b1.runner_color, "#00ff00");

        let collected = proxy.collect_settings();
        // box0 样式写回全局显示字段；box1 样式留在 layout 里。
        assert_eq!(collected.route.width, 18.0);
        assert_eq!(collected.route.render_mode, 1);
        assert_eq!(collected.route.route_color, "#112233");
        assert!(!collected.route.show_runner);
        let loaded = Settings::from_ini(&crate::settings::IniFile::parse(&collected.to_ini()));
        let eff = loaded.map.effective_layout(&loaded.route);
        assert_eq!(eff.boxes.len(), 2);
        assert_eq!(eff.boxes[0].style.as_ref().unwrap().render_mode, 1);
        let b1 = eff.boxes[1].style.as_ref().unwrap();
        assert_eq!(b1.tail_width, 14.0);
        assert_eq!(b1.runner_color, "#00ff00");
        assert_eq!(b1.pace_auto, false);
    }

    #[test]
    fn route_framing_survives_proxy_and_project_roundtrip() {
        let mut s = Settings::default();
        s.route.view_mode = 1;
        s.route.left_right_margin = 12.5;
        s.route.top_bottom_margin = 47.0;
        s.route.view_zoom = 0.37;
        s.route.runner_averaging_factor = 2.5;
        let mut proxy = SettingsProxy::default();
        proxy.apply_settings(&s);
        // 代理只暴露聚合 JSON：取景参数应在 map_boxes 的第 1 框里。
        let boxes = MapLayout::parse(&proxy.map_boxes.to_string()).unwrap();
        assert_eq!(boxes.boxes[0].view_mode, 1);
        assert_eq!(boxes.boxes[0].view_zoom, 0.37);
        assert_eq!(boxes.boxes[0].lr_margin, 12.5);
        assert_eq!(boxes.boxes[0].tb_margin, 47.0);
        assert_eq!(boxes.boxes[0].sensitivity, 2.5);
        let collected = proxy.collect_settings();
        let loaded = Settings::from_ini(&crate::settings::IniFile::parse(&collected.to_ini()));
        assert_eq!(loaded.route.view_mode, 1);
        assert_eq!(loaded.route.left_right_margin, 12.5);
        assert_eq!(loaded.route.top_bottom_margin, 47.0);
        assert_eq!(loaded.route.view_zoom, 0.37);
        assert_eq!(
            loaded.route.runner_averaging_factor, 2.5,
            "灵敏度要随工程保存/加载"
        );
    }

    /// 代理的 `map_boxes` 为空/坏掉时必须回退成「单框」，不能把工程变成 0 框。
    #[test]
    fn empty_or_bad_map_boxes_falls_back_to_single_box() {
        let mut proxy = SettingsProxy::default();
        let collected = proxy.collect_settings();
        assert_eq!(collected.map.layout.boxes.len(), 1);
        assert_eq!(collected.map.effective_layout(&collected.route).boxes.len(), 1);

        proxy.map_boxes = QString::from(r#"{"version":1,"boxes":"oops"}"#);
        let collected = proxy.collect_settings();
        assert_eq!(collected.map.layout.boxes.len(), 1, "坏 JSON 回退单框");
    }

    #[test]
    fn cadence_survives_proxy_and_snapshot_roundtrip() {
        let mut s = Settings::default();
        s.hud.show_cadence = true;
        s.hud.layout = Some(crate::hud::Layout { version: 1, widgets: vec![crate::hud::Widget {
            metric: "cadence".into(), ..Default::default()
        }] });
        let mut proxy = SettingsProxy::default();
        proxy.apply_settings(&s);
        let collected = proxy.collect_settings();
        assert!(collected.hud.show_cadence);
        assert_eq!(collected.hud.layout.as_ref().unwrap().json(), s.hud.layout.as_ref().unwrap().json());
        let loaded = Settings::from_ini(&crate::settings::IniFile::parse(&collected.to_ini()));
        assert_eq!(loaded.hud.layout.unwrap().widgets[0].metric, "cadence");
        assert!(loaded.hud.show_cadence);
    }
}
