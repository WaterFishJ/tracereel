//! GUI: a `QQuickPaintedItem` that composites the decoded video frame, the map image,
//! the route (with tail/controls/runner) and the HUD using QPainter.
//!
//! The item owns the software ffmpeg decoder and the hardware pipe decoder used for
//! playback, and paints the composed frame through QPainter inside a
//! QQuickPaintedItem so QML can display it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use qmetaobject::qttypes::{
    BrushStyle, PenStyle, QBrush, QColor, QImage, QPainter, QPen, QPointF, QRectF,
};
use qmetaobject::*;

use crate::encode::{EncodeHandle, EncodeProgress};
use crate::gyro::{
    apply_seam_warmup_smoothing, load_stabilizer, LoadProgress, ProgressSink, Stabilizer,
};
use crate::pipe::PipeDecoder;
use crate::probe::VideoInfo;
use crate::qtwrap;
use crate::settings::Settings;
use crate::video::SoftDecoder;
use tracereel_core::gpx::GpxTrack;
use tracereel_core::route_manager::{RenderMode, RouteManager, RouteStyle, RunnerStyle, ViewMode};

#[derive(Default, Clone)]
struct PreviewControl {
    request_id: u64,
    generation: u64,
    t_ms: f64,
    playing: bool,
    video: String,
    /// 拼接时间轴的各视频段（含首段）；为空时 worker 退化为单文件 `video`。
    segs: Vec<VideoSeg>,
    pw: usize,
    ph: usize,
    fps: f64,
    preview_perf: bool,
    /// 当前播放倍速（worker 用它决定要不要「不等正好那一帧」）。
    speed: f64,
    gyro: Option<Arc<Mutex<Vec<Option<Arc<Stabilizer>>>>>>,
}

/// One video file on the stitched preview timeline: a camera auto-split of the same
/// orienteering run (identical resolution/fps, own motion data). The global timeline is
/// the concatenation in recording order: `t_global = seg.start_ms + t_local`.
#[derive(Debug, Clone, Default)]
pub struct VideoSeg {
    pub path: String,
    pub start_ms: f64,
    pub duration_ms: f64,
    pub fps: f64,
    pub frame_count: usize,
    pub has_motion: bool,
}

impl VideoSeg {
    pub fn end_ms(&self) -> f64 {
        self.start_ms + self.duration_ms
    }
}

/// Locate the segment owning global time `t_ms`; returns (index, local_ms).
pub(crate) fn locate_video_seg(segs: &[VideoSeg], t_ms: f64) -> (usize, f64) {
    let mut i = 0;
    while i + 1 < segs.len() && t_ms >= segs[i].end_ms() {
        i += 1;
    }
    (i, (t_ms - segs[i].start_ms).max(0.0))
}

/// 预览该不该出声：有视频段，并且「导出 → 视频」勾着。
///
/// 没勾视频时预览画面里根本没有视频轨（见 `preview_layers`），声音也就没有对应的
/// 画面，一并关掉 —— 用户的原话是「如果没有勾选视频的时候，就不要有声音了」。
pub(crate) fn preview_audio_wanted(has_segments: bool, video_layer: bool) -> bool {
    has_segments && video_layer
}

/// Locate the map owning global time `t_ms` on the stitched map timeline: `starts` holds
/// each map's 换图时间 (ms, relative to the video timeline origin, non-decreasing, the
/// main map at 0). Returns the index of the active map (main map first).
pub(crate) fn locate_map(starts: &[f64], t_ms: f64) -> usize {
    let mut i = 0;
    while i + 1 < starts.len() && t_ms >= starts[i + 1] {
        i += 1;
    }
    i
}

/// GUI-thread state for one active map: the downsampled bitmap, the time (ms on the
/// global video timeline) at which it takes over, and its own RouteManager (each map
/// file carries a full-length, time-aligned track of the same run).
struct LiveMap {
    image: QImage,
    start_ms: f64,
    rm: RouteManager,
}

struct PreviewFrame {
    request_id: u64,
    t_ms: f64,
    seg_idx: usize,
    rgba: Vec<u8>,
    w: usize,
    h: usize,
    stabilized: bool,
}

/// Sets a gyro loader thread's `done` flag when dropped — including while unwinding from a
/// panic inside that thread. The loading overlay now stays up until the flag flips (see
/// `update_gyro_status`), so a panicking loader must not leave it stuck on screen forever.
struct LoadDoneGuard(Arc<std::sync::atomic::AtomicBool>);

impl Drop for LoadDoneGuard {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

/// The render item exposed to QML as `TraceReelItem`.
#[derive(QObject, Default)]
pub struct TraceReelItem {
    base: qt_base_class!(trait QQuickPaintedItem),

    settings_path: qt_property!(QString),
    t_ms: qt_property!(f64),
    show_hud: qt_property!(bool),
    playing: qt_property!(bool),
    gyro_status: qt_property!(QString; NOTIFY gyro_status_changed),
    gyro_status_changed: qt_signal!(),
    /// 「最大旋转 / 最大缩放」只读状态（对齐原版 Stabilization 面板底部），JSON 文本。
    gyro_stats: qt_property!(QString; NOTIFY gyro_stats_changed),
    gyro_stats_changed: qt_signal!(),
    /// 视频是否带逐帧焦距数据（原版 `has_per_frame_focal_length`）→ 决定「稳定焦距」是否显示。
    gyro_has_focal_length: qt_property!(bool; NOTIFY gyro_has_focal_length_changed),
    gyro_has_focal_length_changed: qt_signal!(),
    gyro_loading: qt_property!(bool; NOTIFY gyro_loading_changed),
    gyro_loading_changed: qt_signal!(),
    gyro_progress: qt_property!(f64; NOTIFY gyro_progress_changed),
    gyro_progress_changed: qt_signal!(),
    /// Current load phase label ("读取运动数据" / "计算平滑轨迹" / ...); empty when idle.
    gyro_phase: qt_property!(QString; NOTIFY gyro_phase_changed),
    gyro_phase_changed: qt_signal!(),
    busy: qt_property!(bool; NOTIFY busy_changed),
    busy_changed: qt_signal!(),
    busy_msg: qt_property!(QString; NOTIFY busy_msg_changed),
    busy_msg_changed: qt_signal!(),
    /// 上一次「应用设置」落地后的结果摘要（地图数 / 视频参数 / 是否带运动数据），
    /// 供 QML 写进日志 —— 这样"导入有没有完成、加载到了什么"在日志里看得见。
    /// 失败时是 `失败: ...`。
    apply_summary: qt_property!(QString; NOTIFY apply_summary_changed),
    apply_summary_changed: qt_signal!(),
    /// 「配速色带自适应」实际算出来的端点（分钟/公里，慢端/快端），供界面在关闭
    /// 自适应时把滑杆填成当前色带，避免手动值与画面上的色带脱节。
    pace_auto_low: qt_property!(f64; NOTIFY pace_auto_changed),
    pace_auto_high: qt_property!(f64; NOTIFY pace_auto_changed),
    pace_auto_changed: qt_signal!(),

    // ---- encode (export) state, polled from `tick` into QML-visible properties ----
    encoding: qt_property!(bool; NOTIFY encoding_changed),
    encoding_changed: qt_signal!(),
    encode_paused: qt_property!(bool; NOTIFY encode_state_changed),
    encode_frame: qt_property!(i64; NOTIFY encode_state_changed),
    encode_total_frames: qt_property!(i64; NOTIFY encode_state_changed),
    encode_elapsed_s: qt_property!(f64; NOTIFY encode_state_changed),
    encode_remaining_s: qt_property!(f64; NOTIFY encode_state_changed),
    encode_fps: qt_property!(f64; NOTIFY encode_state_changed),
    encode_out_size: qt_property!(i64; NOTIFY encode_state_changed),
    encode_current_t_ms: qt_property!(f64; NOTIFY encode_state_changed),
    encode_duration_ms: qt_property!(f64; NOTIFY encode_state_changed),
    /// 逐段导出时弹窗显示的当前分段说明（如「第 2/5 段 [1-2]」）；普通导出为空。
    encode_segment: qt_property!(QString; NOTIFY encode_state_changed),
    encode_done: qt_property!(bool; NOTIFY encode_done_changed),
    /// "" while running / on success; error text on failure or abort note.
    encode_error: qt_property!(QString; NOTIFY encode_done_changed),
    encode_warning: qt_property!(QString; NOTIFY encode_state_changed),
    /// 非致命提示：例如「显卡不支持该编码格式，已自动改用 CPU 重新导出」。
    /// 导出仍然成功，所以它不进 `encode_error`；界面按警告展示。
    encode_notice: qt_property!(QString; NOTIFY encode_state_changed),
    /// 本次导出实际使用的编码/解码组合说明（如
    /// 「编码：中央处理器（AV1 · libsvtav1）｜解码：显卡硬解」）。
    /// 让用户不必去翻进程命令行就能确认编码器是否选对。
    encode_encoder_info: qt_property!(QString; NOTIFY encode_state_changed),
    /// 这次导出用的配置说明（与「导出队列」里每项显示的是同一套文字，见
    /// `export_queue::configuration`）。直接导出和队列导出都在开始编码时写入，
    /// 进度弹窗直接显示它，两边就不必各写一份格式化代码。
    encode_config: qt_property!(QString; NOTIFY encode_state_changed),
    encode_output: qt_property!(QString; NOTIFY encode_done_changed),
    /// True when the user aborted (partial fragmented mp4 is kept and playable).
    encode_aborted: qt_property!(bool; NOTIFY encode_done_changed),
    encode_state_changed: qt_signal!(),
    encode_done_changed: qt_signal!(),

    // QML-invokable methods (markers; the derive generates the impl from the bodies).    /// Start an export: a worker thread decodes (+ stabilizes) frames into a slot, and
    /// the GUI thread composites + feeds ffmpeg ONE frame per tick (QPainter is
    /// main-thread-only), so the window stays responsive throughout.
    /// True when `path` is a QuickRoute `.qrt` project that carries split data
    /// worth offering to import. The 轨迹 page calls this before landing a picked
    /// path so it can offer the "import splits?" prompt; a non-`.qrt` path is
    /// rejected without I/O.
    ///
    /// The decision is made from the decoded laps rather than the file's type
    /// table: QuickRoute writes a `Start` and a `Stop` lap into *every* project,
    /// so only one with real user splits should prompt.
    qrt_has_splits: qt_method!(
        fn qrt_has_splits(&mut self, path: QString) -> bool {
            let p = path.to_string();
            if !is_qrt_path(&p) {
                return false;
            }
            match std::fs::read(&p) {
                Ok(data) => tracereel_core::qrt::read_project_bytes(&data).has_splits(),
                Err(_) => false,
            }
        }
    ),
    /// Split times stored in `path`, formatted for `settings.split_times`
    /// (`h:mm:ss` entries joined by `|`); an empty string when there are none.
    /// The 轨迹 page calls this when the user answers "导入分段", so the decoded
    /// `.qrt` laps land in the split list exactly like hand-entered splits.
    qrt_split_times: qt_method!(
        fn qrt_split_times(&mut self, path: QString) -> QString {
            let p = path.to_string();
            if !is_qrt_path(&p) {
                return QString::default();
            }
            let Ok(data) = std::fs::read(&p) else {
                return QString::default();
            };
            let proj = tracereel_core::qrt::read_project_bytes(&data);
            let parts: Vec<String> = proj
                .split_times_ms()
                .iter()
                .map(|ms| fmt_split_seconds(*ms as f64 / 1000.0))
                .collect();
            QString::from(parts.join("|"))
        }
    ),
    /// 诊断/回归：预览音频此刻是否有一条活着的解码会话。
    /// 「没勾「导出 → 视频」就不出声」这条规则靠它断言（见 app/tests/hud_scope_probe.qml）。
    debug_audio_running: qt_method!(fn debug_audio_running(&self) -> bool {
        self.preview_audio
            .as_ref()
            .map(|pa| pa.running())
            .unwrap_or(false)
    }),
    /// 诊断/回归：第 `index` 张地图上运动员当前的像素位置（"x,y"；没有这张地图
    /// 或还没算过时返回空串）。
    ///
    /// 「关掉视频预览后运动员不动、只有轨迹在动」那条 bug 就是靠它断言的：没有视频
    /// 帧落地时，运动员位置也必须跟着播放头往前走。
    debug_runner_position: qt_method!(fn debug_runner_position(&self, index: i32) -> QString {
        let Ok(i) = usize::try_from(index) else {
            return QString::default();
        };
        match self.maps.get(i) {
            Some(m) => match m.rm.routes.first() {
                Some(r) => QString::from(format!(
                    "{:.3},{:.3}",
                    r.runner_position.0, r.runner_position.1
                )),
                None => QString::default(),
            },
            None => QString::default(),
        }
    }),
    queue_json: qt_property!(QString; NOTIFY queue_changed),
    queue_error: qt_property!(QString; NOTIFY queue_changed),
    queue_running: qt_property!(bool; NOTIFY queue_changed),
    queue_progress: qt_property!(f64; NOTIFY encode_state_changed),
    queue_changed: qt_signal!(),
    load_queue: qt_method!(fn load_queue(&mut self) {
        if self.queue_loaded { return; }
        self.queue_loaded = true;
        match crate::export_queue::load(&crate::export_queue::storage_path()) {
            Ok(entries) => { self.export_entries = entries; self.persist_queue(); self.publish_queue(); }
            Err(e) => { self.queue_load_failed = true; self.queue_error = format!("无法恢复导出队列：{e}").into(); self.queue_changed(); }
        }
    }),
    add_to_queue: qt_method!(fn add_to_queue(&mut self, snapshot: String, project: String) {
        self.load_queue();
        if self.queue_load_failed { return; }
        let mut s = Settings::from_ini(&crate::settings::IniFile::parse(&snapshot));
        // Normalize paths in the snapshot so a future launch's working directory is irrelevant.
        let cwd = std::env::current_dir().unwrap_or_default();
        let project_path = std::path::Path::new(&project);
        let dir = if project_path.is_file() { project_path.parent().unwrap_or(&cwd) } else { &cwd };
        s.resolve_orv_paths(dir);
        let out = s.encode.output_file_path.clone();
        if let Err(e) = crate::export_queue::check_conflicts(&s, &out, &self.export_entries, None) {
            self.queue_error = e.into(); self.queue_changed(); return;
        }
        let old = self.export_entries.clone();
        self.export_entries.push(crate::export_queue::Entry { project, ini: s.to_ini(), output: out, status: "待导出".into(), ..Default::default() });
        if !self.persist_queue() { self.export_entries = old; }
        self.publish_queue();
    }),
    remove_queue_entry: qt_method!(fn remove_queue_entry(&mut self, index: i32) {
        if self.queue_running || self.queue_active.is_some() || self.queue_load_failed { return; }
        let i = index as usize;
        if i >= self.export_entries.len() { return; }
        let old = self.export_entries.clone(); self.export_entries.remove(i);
        if !self.persist_queue() { self.export_entries = old; }
        self.publish_queue();
    }),
    change_queue_output: qt_method!(fn change_queue_output(&mut self, index: i32, path: String) {
        if self.queue_running || self.queue_active.is_some() || self.queue_load_failed { return; }
        let i = index as usize;
        let Some(entry) = self.export_entries.get(i) else { return; };
        let mut s = entry.settings();
        s.encode.output_file_path = crate::settings::decode_path(&path);
        s.resolve_orv_paths(&std::env::current_dir().unwrap_or_default());
        let out = s.encode.output_file_path.clone();
        if let Err(e) = crate::export_queue::check_conflicts(&s, &out, &self.export_entries, Some(i)) {
            self.queue_error = e.into(); self.queue_changed(); return;
        }
        let old = self.export_entries.clone();
        let e = &mut self.export_entries[i]; e.output = out; e.ini = s.to_ini(); e.status = "待导出".into(); e.error.clear(); e.progress = 0.0;
        if !self.persist_queue() { self.export_entries = old; }
        self.publish_queue();
    }),
    start_queue: qt_method!(fn start_queue(&mut self) {
        self.load_queue();
        if self.encoding || self.queue_running || self.queue_load_failed { return; }
        self.queue_running = true; self.queue_error = QString::default(); self.queue_changed();
        self.advance_queue();
    }),
    stop_queue: qt_method!(fn stop_queue(&mut self) {
        self.queue_running = false;
        if self.queue_active.is_some() { self.abort_encode(); }
        self.queue_changed();
    }),
    start_encode: qt_method!(
        fn start_encode(&mut self, out_path: String) {
            if self.encoding || self.queue_running {
                return; // one encode at a time
            }
            let Some(mut settings) = self.settings.as_ref().cloned() else {
                self.finish_encode(false, "", "尚未加载设置/项目，无法编码");
                return;
            };
            if !settings.encode.include_map
                && !settings.encode.include_video
                && !settings.encode.include_hud
            {
                self.finish_encode(false, "", "请至少选择一个导出内容：地图、视频或 HUD");
                return;
            }
            // 预览和导出用同一套勾选（见 `preview_layers`）：不导视频就是「只导地图+HUD」，
            // 走 map-only 那条渲染路径，HUD 铺满整帧。
            settings.encode.map_only = !settings.encode.include_video;
            if !settings.encode.include_video {
                settings.encode.audio_codec = "none".into();
            }
            settings.resolve_encode_clip_ms();
            let out = crate::settings::decode_path(&out_path);
            if out.is_empty() {
                self.finish_encode(false, "", "请先设置输出视频文件路径（导出页）");
                return;
            }
            self.encode_warning = crate::export_queue::overwrite_warning(&settings, &out).into();
            if !self.encode_warning.to_string().is_empty() { eprintln!("[encode] {}", self.encode_warning); }
            // 告诉用户这次到底用什么编码、什么解码（CPU/QSV 都靠显卡解码，
            // 任务管理器里 GPU 一直在动是正常的，这行字能避免误会）。
            let info = crate::encode::encoder_status_text(&settings.encode);
            eprintln!("[encode] {info}");
            self.encode_encoder_info = QString::from(info.as_str());
            // 进度弹窗里的「导出配置」：和导出队列里每项显示的那几行完全同一套文字。
            self.encode_config =
                QString::from(crate::export_queue::configuration(&settings).join("\n").as_str());
            self.encode_state_changed();
            // 「全部逐段导出」：在「按分段导出」勾选的基础上，把每个分段各导出成一个独立文件
            // （文件名扩展名前追加 [起-止]）。所有分段 job 在同一个后台线程上串行编码、共享同一
            // 个 handle，只有最后一段完成时才翻转 done —— 见 encode::encode_export_segments。
            let seg_batch = settings.encode.seg_enabled && settings.encode.all_seg_enabled;
            let seg_count = if seg_batch {
                settings.encode_total_segments()
            } else {
                0
            };
            if seg_batch && seg_count < 1 {
                self.finish_encode(
                    false,
                    "",
                    "没有可用的分段（请先在「分段」页添加分段时间点）",
                );
                return;
            }
            // Map-only export needs no source video / stabilization: the map + route (from the
            // loaded project) drive a full-frame animation over the route timeline.
            if settings.encode.map_only {
                eprintln!("[encode] starting MAP-ONLY export -> {out}");
                let handle = Arc::new(EncodeHandle::default());
                self.encode_handle = Some(handle.clone());
                self.set_encoding(true);
                self.encode_done = false;
                self.encode_aborted = false;
                self.set_encode_state(EncodeProgress::default());
                self.encode_segment = QString::default();
                self.encode_error = QString::default();
                // 纯地图导出没有源视频解码，编码仍是上面那套。
                let info = crate::encode::encoder_status_text(&settings.encode);
                self.encode_encoder_info = QString::from(info.as_str());
                if seg_batch {
                    let mut jobs = Vec::new();
                    for i in 0..seg_count {
                        let mut s2 = settings.clone();
                        s2.encode.start_seg = i;
                        s2.encode.end_seg = i + 1;
                        s2.resolve_encode_clip_ms(); // map_only → 路线时间轴（不加视频偏移）
                        jobs.push(crate::encode::EncodeJob {
                            settings: s2,
                            out_path: crate::encode::segment_output_path(&out, i, i + 1),
                            src_w: 0,
                            src_h: 0,
                            src_fps: 0.0,
                            dec_w: 0,
                            dec_h: 0,
                            duration_ms: 0.0,
                            clip_start: 0.0,
                            clip_end: 0.0,
                            handle: handle.clone(),
                            stabs: Vec::new(),
                            segs: Vec::new(),
                        });
                    }
                    eprintln!("[encode] 全部逐段导出（仅地图）：{seg_count} 个分段 -> {out}");
                    std::thread::spawn(move || crate::encode::encode_export_segments(jobs));
                } else {
                    let job = crate::encode::EncodeJob {
                        settings: settings.clone(),
                        out_path: out.clone(),
                        src_w: 0,
                        src_h: 0,
                        src_fps: 0.0,
                        dec_w: 0,
                        dec_h: 0,
                        duration_ms: 0.0,
                        clip_start: 0.0,
                        clip_end: 0.0,
                        handle: handle.clone(),
                        stabs: Vec::new(),
                        segs: Vec::new(),
                    };
                    std::thread::spawn(move || crate::encode::encode_export(job));
                }
                self.encode_out_path = out;
                return;
            }
            let Some(info) = self.video_info.as_ref().cloned() else {
                self.finish_encode(
                    false,
                    "",
                    "无法探测视频信息（请先在「视频」页选择有效视频）",
                );
                return;
            };
            if info.width == 0 || info.fps <= 0.0 {
                self.finish_encode(false, "", "视频信息无效（请检查「视频」页的文件路径）");
                return;
            }
            // Resolve per-segment stabilizers aligned with `self.segs`: reuse the GUI-loaded
            // ones (same signature rule as the preview: enabled + motion data). The export
            // treats the stitched timeline as ONE video, so every motion-bearing segment
            // must be ready — exporting with a partially-loaded set would silently drop
            // stabilization on the missing files.
            let stabs: Vec<Option<Arc<Stabilizer>>> =
                if !settings.encode.include_video || !settings.gyro.enabled {
                    Vec::new()
                } else if !self.gyro_has_motion {
                    self.finish_encode(false, "", "该视频没有运动数据，无法启用防抖导出");
                    return;
                } else if self.gyro_load_failed {
                    self.finish_encode(false, "", "防抖数据加载失败，请检查预设/视频后重试");
                    return;
                } else {
                    let slots: Vec<Option<Arc<Stabilizer>>> = self
                        .gyro
                        .as_ref()
                        .and_then(|shared| shared.lock().ok().map(|g| g.clone()))
                        .unwrap_or_default();
                    let missing = self.segs.iter().enumerate().any(|(i, s)| {
                        s.has_motion && slots.get(i).map(|x| x.is_none()).unwrap_or(true)
                    });
                    if missing {
                        let msg = if self.gyro_load_pending() {
                            "防抖数据仍在加载中（部分分段尚未就绪），请稍候再试"
                        } else {
                            "部分分段的防抖数据未加载，请先在「视频」页勾选启用防抖并等待就绪"
                        };
                        self.finish_encode(false, "", msg);
                        return;
                    }
                    (0..self.segs.len())
                        .map(|i| slots.get(i).cloned().flatten())
                        .collect()
                };
            eprintln!("[encode] starting export -> {out}");
            let handle = Arc::new(EncodeHandle::default());
            self.encode_handle = Some(handle.clone());
            self.set_encoding(true);
            self.encode_done = false;
            self.encode_aborted = false;
            self.set_encode_state(EncodeProgress::default());
            self.encode_segment = QString::default();
            // The preview keeps running untouched: the export is a fully self-contained
            // background pipeline (decode + stabilize + composite + ffmpeg all on its own
            // thread). Editing/exporting never disturbs the preview.
            // 导出解码 + 防抖尺寸：按输出画布里面板实际需要的像素算（上限=源尺寸），
            // 见 `encode::export_decode_size`。比预览那套（限高 720）大时，导出会自己
            // 按这个尺寸重新加载一套稳定化器（`encode.rs` 里做，在编码线程上跑）。
            let (dec_w, dec_h) = crate::encode::export_decode_size(&settings, info.width, info.height);
            if seg_batch {
                let mut jobs = Vec::new();
                for i in 0..seg_count {
                    let mut s2 = settings.clone();
                    s2.encode.start_seg = i;
                    s2.encode.end_seg = i + 1;
                    s2.resolve_encode_clip_ms(); // 视频导出 → 边界加出发时刻偏移
                    let (clip_start, clip_end) =
                        crate::encode::clip_range(&s2.encode, info.duration_ms);
                    jobs.push(crate::encode::EncodeJob {
                        settings: s2,
                        out_path: crate::encode::segment_output_path(&out, i, i + 1),
                        src_w: info.width,
                        src_h: info.height,
                        src_fps: info.fps,
                        dec_w,
                        dec_h,
                        duration_ms: info.duration_ms,
                        clip_start,
                        clip_end,
                        handle: handle.clone(),
                        stabs: stabs.clone(),
                        segs: self.segs.clone(),
                    });
                }
                eprintln!("[encode] 全部逐段导出：{seg_count} 个分段 -> {out}");
                std::thread::spawn(move || crate::encode::encode_export_segments(jobs));
            } else {
                let (clip_start, clip_end) =
                    crate::encode::clip_range(&settings.encode, info.duration_ms);
                let job = crate::encode::EncodeJob {
                    settings: settings.clone(),
                    out_path: out.clone(),
                    src_w: info.width,
                    src_h: info.height,
                    src_fps: info.fps,
                    dec_w,
                    dec_h,
                    duration_ms: info.duration_ms,
                    clip_start,
                    clip_end,
                    handle: handle.clone(),
                    stabs,
                    segs: self.segs.clone(),
                };
                std::thread::spawn(move || crate::encode::encode_export(job));
            }
            self.encode_out_path = out;
        }
    ),
    pause_encode: qt_method!(
        fn pause_encode(&mut self, on: bool) {
            if let Some(h) = self.encode_handle.as_ref() {
                h.pause.store(on, Ordering::SeqCst);
            }
            self.encode_paused = on;
            self.encode_state_changed();
        }
    ),
    abort_encode: qt_method!(
        fn abort_encode(&mut self) {
            if let Some(h) = self.encode_handle.as_ref() {
                h.stop.store(true, Ordering::SeqCst);
                h.pause.store(false, Ordering::SeqCst);
            }
            self.encode_paused = false;
        }
    ),
    tick: qt_method!(
        fn tick(&mut self) {
            // Always service the background apply + gyro status (even while paused) so the
            // loading overlays clear and settings land promptly.
            self.poll_apply();
            self.poll_gyro_progress();
            self.poll_recompute();
            self.update_gyro_status();
            self.poll_encode();
            self.poll_queue_durations();
            if self.queue_running && !self.encoding { self.advance_queue(); }
            if !self.playing {
                self.was_playing = false;
                // While paused, request the frame each tick if none has been shown yet; this
                // retries a cold decoder. Because poll_preview_frame accepts any current-
                // generation frame while paused, re-bumping the request id here does not cause
                // published frames to be rejected (that was the "video never appears" bug).
                if self.video_layer_on() && self.video_image.is_none() {
                    self.request_preview_frame(self.t_ms);
                }
                self.poll_preview_frame();
                // 暂停时冻结预览音频（会话保留，恢复播放即续播）。
                if let Some(pa) = &self.preview_audio {
                    pa.set_paused(true);
                }
                return;
            }
            // Safety valve: never strand the playhead on a hold — if a frame hasn't landed
            // within ~4 s of seeking, resume the clock anyway (better a skip than a hard freeze).
            if self.seek_settle {
                if let Some(s) = self.seek_settle_since {
                    if s.elapsed().as_secs_f64() > 4.0 {
                        self.seek_settle = false;
                        self.seek_settle_since = None;
                        self.seam_hold = None;
                        self.play_anchor_wall = Some(std::time::Instant::now());
                        self.play_anchor_t = self.t_ms;
                    }
                }
            }
            // Real-time clock: t_ms follows the wall clock (1x speed). When a render tick is
            // slower than one frame, frames are skipped instead of the video slowing down.
            // While a seek is settling we HOLD the clock (see `seek_settle`) so the playhead
            // doesn't outrun the re-warming hardware pipe.
            if self.fps > 0.0 && !self.seek_settle {
                if let Some(aw) = self.play_anchor_wall {
                    self.t_ms = self.play_anchor_t
                        + aw.elapsed().as_secs_f64() * 1000.0 * self.play_speed();
                } else {
                    self.play_anchor_wall = Some(std::time::Instant::now());
                    self.play_anchor_t = self.t_ms;
                }
                if self.duration_ms > 0.0 && self.t_ms >= self.duration_ms {
                    self.t_ms = 0.0;
                    self.play_anchor_t = 0.0;
                    self.play_anchor_wall = Some(std::time::Instant::now());
                }
                // Frame-drop metric: count ticks that advanced more than 1.5 frames since
                // the previous PLAYING tick (skipped frames = the visible micro-stutter).
                if self.was_playing {
                    self.skip_ticks += 1;
                    let dt = 1000.0 / self.fps;
                    if self.t_ms - self.prev_tick_t > dt * 1.5 {
                        self.skip_drops += 1;
                    }
                    self.prev_tick_t = self.t_ms;
                    if self.skip_ticks % 600 == 0 {
                        if self.skip_drops > 0 {
                            eprintln!(
                                "[preview] dropped {} of last 600 frames (render too slow)",
                                self.skip_drops
                            );
                        }
                        self.skip_drops = 0;
                    }
                } else {
                    self.prev_tick_t = self.t_ms;
                }
            }
            // Seam hold: the clock just crossed into the next stitched file while the shown
            // frame is still from the previous one — freeze playback until the worker lands a
            // frame from the new segment (see `seam_hold`). Strictly-forward crossings only,
            // so a loop back to t=0 doesn't hold.
            if !self.seek_settle && self.segs.len() > 1 {
                if let Some(shown) = self.shown_seg {
                    let (cur, _) = locate_video_seg(&self.segs, self.t_ms);
                    if cur > shown {
                        self.seek_settle = true;
                        self.seek_settle_since = Some(std::time::Instant::now());
                        self.seam_hold = Some(cur);
                        eprintln!(
                            "[gui] seam hold start: shown seg {shown} -> cur seg {cur} (t={:.0})",
                            self.t_ms
                        );
                    }
                }
            }
            self.was_playing = true;
            // Retry the gyro load every tick while playing: covers the race where the user
            // presses play before the async project apply has landed. 没有视频层就不必加载
            // 防抖（它是逐帧处理视频用的）。
            if self.video_layer_on() {
                self.ensure_gyro_loaded();
            }
            self.pump_preview_frame();
            self.sync_preview_audio();
        }
    ),
    seek: qt_method!(
        fn seek(&mut self, t_ms: f64) {
            self.t_ms = t_ms.max(0.0);
            self.seek_settle = true;
            self.seek_settle_since = Some(std::time::Instant::now());
            // A user seek supersedes any pending seam hold: release it so the post-seek frame
            // (from whatever segment was seeked to) clears the hold immediately.
            self.seam_hold = None;
            if self.playing {
                self.play_anchor_wall = Some(std::time::Instant::now());
                self.play_anchor_t = self.t_ms;
            }
            // A real seek invalidates the decoder generation; worker drops any old in-flight
            // result and respawns once at the final slider position.
            if let Ok(mut c) = self.preview_control.lock() {
                c.generation = c.generation.wrapping_add(1);
            }
            self.pump_preview_frame();
        }
    ),
    set_playing: qt_method!(
        fn set_playing(&mut self, on: bool) {
            self.playing = on;
            if on {
                // (Re)anchor the real-time clock and start the gyro load if enabled but
                // not loaded (instead of silently playing raw).
                self.play_anchor_wall = Some(std::time::Instant::now());
                self.play_anchor_t = self.t_ms;
                self.ensure_gyro_loaded();
            } else {
                self.play_anchor_wall = None;
            }
            if let Ok(mut c) = self.preview_control.lock() {
                c.playing = on;
            }
        }
    ),
    // 预览倍速：在当前位置平滑切换时钟倍率；音频在随后的 tick 里检测到
    // 倍速不一致会自动重启解码会话（atempo 保音调）。
    set_speed: qt_method!(
        fn set_speed(&mut self, v: f64) {
            let nv = if v.is_finite() && v > 0.0 {
                v.clamp(0.5, 8.0)
            } else {
                1.0
            };
            if self.playing && !self.seek_settle {
                if let Some(aw) = self.play_anchor_wall {
                    let sp = self.play_speed();
                    self.t_ms = self.play_anchor_t + aw.elapsed().as_secs_f64() * 1000.0 * sp;
                }
                self.play_anchor_wall = Some(std::time::Instant::now());
            } else {
                self.play_anchor_wall = None;
            }
            self.play_anchor_t = self.t_ms;
            self.speed = nv;
        }
    ),
    reload: qt_method!(
        fn reload(&mut self, path: String) {
            let p = crate::settings::decode_path(&path);
            self.settings_path = QString::from(p);
            // Full reset. Startup / project load never starts the gyro load: the stabilizer
            // loads only when the user checks "启用防抖" or changes the video while it is on.
            self.settings = None;
            self.video_info = None;
            self.source_size = QString::default();
            self.source_size_changed();
            self.maps.clear();
            self.video_image = None;
            self.gyro = None;
            self.gyro_done = None;
            self.gyro_has_motion = false;
            self.gyro_load_failed = false;
            self.last_gyro_sig = String::new();
            self.pending_gyro_apply = None;
            self.gpx = None;
            self.t_ms = 0.0;
            self.set_gyro_loading(false);
            self.start_async_apply();
        }
    ),
    /// Apply changed settings (from an .orv file) to the preview WITHOUT blocking the GUI
    /// thread (background prepare + busy overlay). `reload_gyro=true` (re)starts the
    /// Gyroflow load when the prepared settings have it enabled and the video has motion
    /// data; `false` keeps the already-loaded stabilizer.
    /// 播放中应用时**保留当前播放位置**并重锚实时时钟（暂停时仍按旧语义从 0 重放）：
    /// 否则后台重载落地后播放头被清零，而 tick 立刻把请求跳回原位置，解码链永远追不上
    /// —— 预览黑屏、地图/轨迹与 HUD 冻住，只有进度条在走。详见 `keep_playhead_after_apply`。
    apply_settings: qt_method!(
        fn apply_settings(&mut self, path: String, reload_gyro: bool) {
            let p = crate::settings::decode_path(&path);
            self.debug_apply_kind = QString::from("完整重载");
            self.settings_path = QString::from(p);
            if reload_gyro {
                self.gyro = None;
                self.gyro_done = None;
                self.gyro_has_motion = false;
                self.gyro_load_failed = false;
                self.last_gyro_sig = String::new();
                self.pending_gyro_apply = None;
                self.set_gyro_loading(false);
            }
            self.settings = None;
            self.video_info = None;
            self.source_size = QString::default();
            self.source_size_changed();
            self.maps.clear();
            self.video_image = None;
            self.gpx = None;
            self.keep_playhead_after_apply(false);
            if let Some(pa) = &self.preview_audio {
                pa.stop();
            }
            // 地图列表刚被清空：立刻重画一次。否则在后台重载落地之前，
            // 面板上还留着上一张地图的画面 —— 看起来就像"删掉了却还在显示"。
            {
                let this: &dyn QQuickItem = &*self;
                this.update();
            }
            self.start_async_apply();
        }
    ),
    /// 只更新**稳定化参数**（「稳定化」页的滑杆/数值框）：复用已加载的 stabilizer，
    /// 重新应用参数 + 重算平滑/变焦/去畸变 —— 不重探测视频、不重载地图/路线、**不重读运动数据**、
    /// 不重置播放位置。对应原版 Gyroflow 拖滑杆时的 recompute。
    /// 只有「硬输入」（视频 / 预设 / 镜头标定 / 开关）变了、或还没加载过，才退回完整 apply_settings。
    apply_gyro_params: qt_method!(
        fn apply_gyro_params(&mut self, path: String, prop: String) {
            let p = crate::settings::decode_path(&path);
            let new = Settings::load_orv(&p);
            let prop = prop.to_string();
            let reusable = self.gyro.is_some()
                && !self.gyro_loading
                && self
                    .settings
                    .as_ref()
                    .map(|old| hard_gyro_sig(old) == hard_gyro_sig(&new))
                    .unwrap_or(false);
            self.settings_path = QString::from(p.clone());
            if !reusable {
                // 视频没变（只是预设/镜头标定/开关）→ 只重建稳定化器，不重探测视频/不重载地图。
                let same_video = self
                    .settings
                    .as_ref()
                    .map(|old| old.video.all_paths() == new.video.all_paths())
                    .unwrap_or(false);
                if same_video && self.video_info.is_some() {
                    let (pw, ph) = self.processing;
                    // 加载在途：不要为了应用参数把在途的运动数据丢掉重读（见字段注释）。
                    // 记下来，等这次加载落地后由 update_gyro_status 触发一次分级重算。
                    if self.gyro_loading {
                        eprintln!(
                            "[stab] gyro param '{prop}' queued: a motion-data load is already in flight"
                        );
                        self.settings = Some(new.clone());
                        self.pending_gyro_apply = Some((new, prop));
                        self.update_gyro_status();
                        return;
                    }
                    eprintln!("[stab] gyro-only reload (video unchanged): {prop}");
                    self.settings = Some(new.clone());
                    self.gyro = None;
                    self.gyro_done = None;
                    self.gyro_load_failed = false;
                    self.gyro_has_motion = new.gyro.enabled && self.gyro_has_motion;
                    self.last_gyro_sig = hard_gyro_sig(&new);
                    self.set_gyro_loading(false);
                    if new.gyro.enabled && self.gyro_has_motion {
                        self.start_gyro_load(&new, pw, ph);
                    }
                    self.update_gyro_status();
                    self.refresh_preview_config();
                    self.request_preview_frame(self.t_ms);
                    let this: &dyn QQuickItem = &*self;
                    this.update();
                    self.set_busy(false, "");
                    return;
                }
                self.apply_settings(p, true);
                return;
            }
            let snapshot: Vec<Option<Arc<Stabilizer>>> = self
                .gyro
                .as_ref()
                .and_then(|g| g.lock().ok().map(|g| g.clone()))
                .unwrap_or_default();
            self.settings = Some(new.clone());
            // 重算放到后台线程：长视频的分级重算仍可能要几秒（debug 构建几十秒），
            // 放 GUI 线程会把界面冻住。滑杆连续拖动时合并成一次（排队最新的）。
            if self.recompute_done.is_some() {
                self.recompute_pending = Some((new, prop));
            } else {
                self.start_recompute(new, prop, snapshot);
            }
            self.update_gyro_status();
        }
    ),
    /// Light-weight apply for encode-page changes ONLY: refresh the in-memory settings so
    /// the next export picks them up, WITHOUT touching the preview (no re-probe, no reload
    /// of map/route/decoder, no seek to 0, no busy overlay). The `[encode]` section is not
    /// read by `draw_scene`, so the composited preview is byte-for-byte unchanged.
    apply_encode_only: qt_method!(
        fn apply_encode_only(&mut self, path: String) {
            let p = crate::settings::decode_path(&path);
            self.settings_path = QString::from(p.clone());
            self.settings = Some(Settings::load_orv(&p));
            let this: &dyn QQuickItem = &*self;
            this.update();
        }
    ),

    // Internal rendering state (not exposed to QML).
    apply_hud: qt_method!(
        fn apply_hud(&mut self, json: QString) {
            if let Ok(layout) = crate::hud::Layout::parse(&json.to_string()) {
                if let Some(s) = self.settings.as_mut() {
                    s.hud.layout = Some(layout);
                }
                let this: &dyn QQuickItem = &*self;
                this.update();
            }
        }
    ),
    /// 只更新**轨迹/地图的显示**（轨迹页与地图页的外观项：渲染模式、渲染范围、宽度、
    /// 颜色/透明度、尾巴、控制点、运动员、视野模式、取景边距、画面占比…）：
    /// 就地更新每张地图的 `RouteManager` 样式与取景参数，复用已经探测好的视频、
    /// 已经解码的地图、已经解析的路线 —— 不重探测、不重解码、不重读运动数据、
    /// **不重置播放位置**，滑杆拖动毫秒级生效、预览不重载（对应原版 OrientView 的
    /// 「改外观不重载」体验）。只有确认「非显示项」（视频/地图/轨迹文件、分段时间、
    /// gpx、防抖、编码…）一字未改时才走这条路，否则立刻退回完整 `apply_settings`。
    apply_route_display: qt_method!(
        fn apply_route_display(&mut self, path: String) {
            let p = crate::settings::decode_path(&path);
            let new = Settings::load_orv(&p);
            let old = match self.settings.as_ref() {
                Some(o) if !self.maps.is_empty() && only_route_display_changed(o, &new) => o.clone(),
                _ => {
                    // 诊断：为什么这次没能就地更新（现场排查"改外观就重载"时先看这一行）。
                    eprintln!("[apply] 显示项之外还有变化（或还没有地图），退回完整重载");
                    return self.apply_settings(path, false);
                }
            };
            let style = route_style_from_settings(&new);
            let view = view_mode_from_settings(&new);
            // 影响控制点/分段取景表的项：就地改完必须让 RouteManager 整表重算
            // （表是按窗口尺寸+画面占比+边距+按表-出发时间算出来的）。
            let tables_dirty = old.route.control_time_offset != new.route.control_time_offset
                || old.route.left_right_margin != new.route.left_right_margin
                || old.route.top_bottom_margin != new.route.top_bottom_margin
                || old.route.user_scale != new.route.user_scale
                || old.map.relative_width != new.map.relative_width;
            self.settings_path = QString::from(p);
            self.settings = Some(new.clone());
            let ft = if self.fps > 0.0 { 1000.0 / self.fps } else { 16.0 };
            let t_sec = self.t_ms / 1000.0;
            for lm in self.maps.iter_mut() {
                let rm = &mut lm.rm;
                rm.set_view_mode(view);
                rm.map_relative_width = new.map.relative_width;
                rm.left_right_margin = new.route.left_right_margin;
                rm.top_bottom_margin = new.route.top_bottom_margin;
                for r in rm.routes.iter_mut() {
                    r.style = style.clone();
                    r.control_time_offset = style.control_time_offset;
                    r.runner_time_offset = style.runner_time_offset;
                }
                // 自适应配速色带由实际轨迹算出，和完整 apply 走同一个入口。
                apply_adaptive_pace(rm);
                if tables_dirty {
                    rm.request_full_update();
                }
                rm.update(t_sec, ft);
            }
            // 「自适应」关掉时 QML 用这两个端点把手动滑杆填成当前色带。
            if new.route.pace_auto || new.route.tail_pace_auto {
                let pace = self
                    .maps
                    .first()
                    .and_then(|m| m.rm.routes.first())
                    .and_then(|r| tracereel_core::route_manager::adaptive_pace_range(&r.route_points));
                if let Some((low, high)) = pace {
                    self.pace_auto_low = low;
                    self.pace_auto_high = high;
                }
            }
            self.debug_apply_kind = QString::from("就地更新");
            let this: &dyn QQuickItem = &*self;
            this.update();
        }
    ),
    hud_viewport: qt_method!(
        fn hud_viewport(&self, width: f64, height: f64) -> QString {
            let settings = preview_layers(self.settings.clone().unwrap_or_default());
            let size = self.video_image.as_ref().map(|image|
                (qtwrap::qimage_width(image) as f64, qtwrap::qimage_height(image) as f64))
                .or_else(|| self.video_info.as_ref().map(|info| (info.width as f64, info.height as f64)));
            let r = crate::hud::viewport(&settings, width, height, size);
            QString::from(serde_json::json!({"x":r.x,"y":r.y,"width":r.width,"height":r.height}).to_string().as_str())
        }
    ),
    video_image: Option<QImage>,
    /// 全部地图状态（主图 + 换图序列，按 `start_ms` 升序）：降采样底图 + 各自独立的
    /// RouteManager。任一全局时刻只有当前图活跃，由 `locate_map` 选出。
    maps: Vec<LiveMap>,
    video_info: Option<VideoInfo>,
    settings: Option<Settings>,
    // Background preview worker: latest request in, latest RGBA frame out.
    preview_control: Arc<Mutex<PreviewControl>>,
    preview_frame: Arc<Mutex<Option<PreviewFrame>>>,
    preview_stop: Arc<AtomicBool>,
    preview_worker_running: bool,
    preview_request_id: u64,
    fps: f64,
    duration_ms: qt_property!(f64),
    /// 源视频尺寸「宽x高」（探测到的原生尺寸）；未知时为空串。分辨率选「与原始视频一致」
    /// （`encode_resolution = "source"`）时，QML 用它算预览画布的宽高比 —— 导出的画布尺寸
    /// 就是源尺寸，预览必须同比例，否则地图/HUD 面板比例对不上。
    source_size: qt_property!(QString; NOTIFY source_size_changed),
    source_size_changed: qt_signal!(),
    /// 已落地到画面的预览帧数（只增不减）。自动化回归（`qmlprobe`）用它断言"改设置后
    /// 画面还在更新"——「黑屏」症状的本质就是这个数不再增长。
    debug_frames: qt_property!(f64),
    /// 最近一次设置改动走的是哪条路：「完整重载」还是「就地更新」。排查与自动化回归用
    /// —— 改一个轨迹显示项却看到「完整重载」，就说明白名单漏了键（或代理多写了别的差异）。
    debug_apply_kind: qt_property!(QString),
    /// 完整重载**落地**次数（`apply_prepared` 成功次数）。自动化回归用它等"这次重载真的
    /// 算完了"，而不是靠定时猜。
    debug_full_landed: qt_property!(f64),
    gpx: Option<GpxTrack>,
    // Gyroflow stabilization state.
    processing: (usize, usize),
    /// 按段 Stabilizer 槽位（与 `segs` 一一对应；无运动数据/加载失败的段为 None）。
    gyro: Option<Arc<Mutex<Vec<Option<Arc<Stabilizer>>>>>>,
    /// 拼接时间轴的各视频段（apply 时刷新）；单视频时长度为 1。
    segs: Vec<VideoSeg>,
    /// Set when a stabilizer load thread has been spawned; flipped to true on completion so
    /// the GUI thread can tell a finished-but-failed load from one still running.
    gyro_done: Option<Arc<std::sync::atomic::AtomicBool>>,
    /// Real load progress (0.0..1.0) + phase published by the loader thread through a
    /// lock-free sink (the GUI polls it every tick). Drives the determinate bar in the
    /// loading overlay instead of a pure animation; -1.0 = unknown/indeterminate.
    gyro_progress_slot: Option<Arc<ProgressSink>>,
    /// Some(..) while a background "只重算参数" thread is running (stableization params changed
    /// without touching the motion data). Polled by [`Self::poll_recompute`].
    recompute_done: Option<Arc<std::sync::atomic::AtomicBool>>,
    /// 重算期间又改了参数时，把最新的一份排队，等在跑的那次结束后接着跑（滑杆连续拖动合并成一次）。
    recompute_pending: Option<(Settings, String)>,
    /// 运动数据**加载在途**时到达的「只改参数」改动。等这次加载真正落地后再做一次分级重算把新参数
    /// 应用上去 —— 而不是把刚读进来的运动数据整个丢掉重读。启动/打开工程时 QML 的平滑参数初始化
    /// 一定会回写一次 `gyro_smoothing_params`，正好撞上在途加载；旧行为会因此把运动数据重读两遍
    /// （日志里两条 "gyro load finished"），大文件上白白多等一倍时间。
    pending_gyro_apply: Option<(Settings, String)>,
    /// Latched true when a stabilizer load finished but failed. Prevents the play-triggered
    /// `ensure_gyro_loaded` from retrying every tick (which would re-pop the loading overlay
    /// forever). Cleared when the video/preset/feature changes.
    gyro_load_failed: bool,
    /// Gyro-relevant input signature (video + preset + enabled) as of the last apply, so the
    /// failed latch is cleared only when the user actually changes what would be stabilized.
    last_gyro_sig: String,
    gyro_has_motion: bool,
    stab_ok_logged: bool,
    stab_failed: bool,
    // Async apply: background-prepared data, polled from `tick`.
    pending_apply: Option<Arc<Mutex<Option<PendingApply>>>>,
    // Real-time playback clock: t_ms follows the wall clock so slow renders drop frames
    // instead of slowing the video down.
    play_anchor_wall: Option<std::time::Instant>,
    play_anchor_t: f64,
    was_playing: bool,
    // After a seek/drag, hold the playhead fixed until the (re-seeking) hardware pipe
    // delivers its first frame at the new position. Without this, the 16 ms wall-clock
    // `tick` outruns the warming decoder: every request is a timestamp the pipe can't
    // serve yet → no frames → the video freezes while only the slider moves (and chasing
    // it with software decode stalls for seconds on the 16 GB source).
    seek_settle: bool,
    seek_settle_since: Option<std::time::Instant>,
    /// Segment index of the frame currently shown (from the last polled PreviewFrame).
    /// Used to detect the playback clock crossing a stitch seam into the next file.
    shown_seg: Option<usize>,
    /// While held (Some(seg)), the clock freezes at the seam until the worker publishes
    /// a frame from segment `seg` (or later); then the hold clears and the clock
    /// re-anchors. Without this the wall clock outruns the warming/cold-starting pipe of
    /// the next file, the decoder's stale threshold trips, and the ffmpeg chain restarts
    /// repeatedly — the visible "second video stalls for seconds" at each seam.
    seam_hold: Option<usize>,
    skip_ticks: u64,
    skip_drops: u64,
    prev_tick_t: f64,
    // Encode (export) worker state.
    encode_handle: Option<Arc<EncodeHandle>>,
    encode_out_path: String,
    export_entries: Vec<crate::export_queue::Entry>,
    queue_loaded: bool,
    queue_load_failed: bool,
    queue_active: Option<usize>,
    queue_duration_pending: std::collections::HashSet<String>,
    queue_duration_rx: Option<std::sync::mpsc::Receiver<(String, Result<Vec<f64>, String>)>>,
    queue_duration_tx: Option<std::sync::mpsc::Sender<(String, Result<Vec<f64>, String>)>>,
    // 预览播放倍速：1.0=原速；0.0（Default 初始值）按 1.0 处理。
    speed: f64,
    /// 预览音频引擎（ffmpeg→waveOut，atempo 保音调）；首次播放时创建。
    preview_audio: Option<crate::preview_audio::PreviewAudio>,
    /// 预览音频是否已经因为「没有视频层」被关掉（避免每个 tick 都去 stop 一次）。
    audio_off: bool,
}

impl TraceReelItem {
    fn publish_queue(&mut self) {
        if self.queue_duration_tx.is_none() {
            let (tx, rx) = std::sync::mpsc::channel();
            self.queue_duration_tx = Some(tx); self.queue_duration_rx = Some(rx);
        }
        for e in &self.export_entries {
            if e.durations_s.is_none() && e.duration_error.is_empty() && self.queue_duration_pending.insert(e.ini.clone()) {
                let key = e.ini.clone(); let settings = e.settings();
                let tx = self.queue_duration_tx.as_ref().unwrap().clone();
                std::thread::spawn(move || {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| crate::export_queue::durations(settings)))
                        .unwrap_or_else(|_| Err("时长计算失败".into()));
                    let _ = tx.send((key, result));
                });
            }
        }
        let rows: Vec<_> = self.export_entries.iter().enumerate().map(|(i, e)| {
            let s = e.settings();
            serde_json::json!({"project": e.project, "output": e.output, "status": e.status, "error": e.error,
                "configuration": crate::export_queue::configuration(&s), "durations": e.durations_s, "durationError": e.duration_error,
                "active": self.queue_active == Some(i), "progress": e.progress,
                "warning": if e.status == "已完成" { String::new() } else { crate::export_queue::overwrite_warning(&s, &e.output) },
                "summary": format!("{}x · {} · {} · {}", s.encode.speed, s.encode.resolution, s.encode.format, s.encode.backend)})
        }).collect();
        self.queue_json = serde_json::to_string(&rows).unwrap_or_else(|_| "[]".into()).into();
        self.queue_changed();
    }
    fn poll_queue_durations(&mut self) {
        let results: Vec<_> = self.queue_duration_rx.as_ref().map(|rx| rx.try_iter().collect()).unwrap_or_default();
        if results.is_empty() { return; }
        for (key, result) in results {
            self.queue_duration_pending.remove(&key);
            for e in self.export_entries.iter_mut().filter(|e| e.ini == key) {
                match &result { Ok(d) => { e.durations_s = Some(d.clone()); e.duration_error.clear(); }, Err(error) => e.duration_error = error.clone() }
            }
        }
        self.persist_queue(); self.publish_queue();
    }
    fn persist_queue(&mut self) -> bool {
        match crate::export_queue::save(&crate::export_queue::storage_path(), &self.export_entries) {
            Ok(()) => { self.queue_error = QString::default(); true }
            Err(e) => { self.queue_error = format!("保存导出队列失败：{e}").into(); self.queue_running = false; self.queue_changed(); false }
        }
    }
    fn advance_queue(&mut self) {
        let Some(i) = self.export_entries.iter().position(|e| e.status != "已完成") else {
            self.queue_running = false; self.queue_changed(); return;
        };
        let e = self.export_entries[i].clone();
        let s = e.settings();
        if let Err(error) = crate::export_queue::check_conflicts(&s, &e.output, &self.export_entries, Some(i)) {
            self.export_entries[i].status = "失败".into(); self.export_entries[i].error = error.clone();
            self.queue_running = false; self.persist_queue(); self.queue_error = error.into(); self.publish_queue(); return;
        }
        self.export_entries[i].status = "导出中".into(); self.export_entries[i].error.clear();
        if !self.persist_queue() { self.export_entries[i] = e; self.publish_queue(); return; }
        self.queue_active = Some(i);
        self.queue_progress = 0.0;
        let handle = Arc::new(EncodeHandle::default());
        self.encode_handle = Some(handle.clone()); self.encode_out_path = e.output.clone();
        self.encode_warning = crate::export_queue::overwrite_warning(&s, &e.output).into();
        self.set_encoding(true); self.encode_done = false; self.encode_aborted = false;
        self.encode_error = QString::default(); self.encode_notice = QString::default();
        let info = crate::encode::encoder_status_text(&s.encode);
        eprintln!("[encode] {info}");
        self.encode_encoder_info = QString::from(info.as_str());
        // 队列导出时弹窗显示的仍然是「这一项自己的配置」，不是当前工程的设置。
        self.encode_config =
            QString::from(crate::export_queue::configuration(&s).join("\n").as_str());
        self.set_encode_state(EncodeProgress::default());
        self.encode_segment = "正在准备队列任务…".into(); self.encode_state_changed(); self.publish_queue();
        std::thread::spawn(move || crate::export_queue::run(s, e.output, handle));
    }
    /// 当前预览倍速（未设置/非法时按 1.0x）。
    fn play_speed(&self) -> f64 {
        if self.speed.is_finite() && self.speed > 0.0 {
            self.speed.clamp(0.5, 8.0)
        } else {
            1.0
        }
    }

    /// 预览里的「视频层」开着没有：没勾「导出 → 视频」时预览根本不画视频
    /// （见 `preview_layers`），于是解码、防抖、声音全都白干。
    fn video_layer_on(&self) -> bool {
        self.settings
            .as_ref()
            .map(|s| s.encode.include_video)
            .unwrap_or(true)
    }

    /// 把每张地图的路线管理器推进到 `self.t_ms`：运动员位置与镜头取景都在
    /// `RouteManager::update` 里算。
    ///
    /// 这一步以前只挂在「有视频帧落地」那条路径上（`poll_preview_frame`），于是关掉
    /// 视频预览后没有任何帧落地，运动员和镜头就停在原处、只有轨迹在动 —— 用户
    /// 报的正是这个 bug。现在没有视频层时由 [`Self::pump_preview_frame`] 每 tick
    /// 推进一次。
    fn advance_map_runners(&mut self) {
        let (iw, ih) = (
            qtwrap::qquick_item_width(self.get_cpp_object()),
            qtwrap::qquick_item_height(self.get_cpp_object()),
        );
        let ft = if self.fps > 0.0 {
            1000.0 / self.fps
        } else {
            16.0
        };
        for lm in &mut self.maps {
            if iw > 0.0
                && ih > 0.0
                && ((lm.rm.window_width - iw).abs() > 1.0
                    || (lm.rm.window_height - ih).abs() > 1.0)
            {
                lm.rm.window_resized(iw, ih);
            }
            lm.rm.update(self.t_ms / 1000.0, ft);
        }
    }

    /// 让预览出一帧：视频层开着时照常向 worker 要点；关着时**不解码**，
    /// 只按当前播放头重画一次（地图/轨迹/HUD 照样跟着走，倍速也不会被
    /// 解码吞吐拖慢 —— 之前这里每帧都在解 4K，倍速越高越卡）。
    fn pump_preview_frame(&mut self) {
        if self.video_layer_on() {
            self.request_preview_frame(self.t_ms);
        }
        self.poll_preview_frame();
        if !self.video_layer_on() {
            // 没有帧落地，就没有人推进运动员/镜头（见 `advance_map_runners`）。
            self.advance_map_runners();
            let this: &dyn QQuickItem = &*self;
            this.update();
        }
    }

    /// 让预览音频跟随播放头：seek/接缝稳定期内暂停；正常播放时按当前段
    /// 与倍速下发播放请求（倍速变化或位置漂移会自动重启解码会话）。
    fn sync_preview_audio(&mut self) {
        if !preview_audio_wanted(!self.segs.is_empty(), self.video_layer_on()) {
            // 没勾「导出 → 视频」：预览画面里没有视频轨，声音也不该响；
            // 顺手把解码会话关掉（真停，不是暂停），别让它在后台空转。
            if !self.audio_off {
                self.audio_off = true;
                if let Some(pa) = &self.preview_audio {
                    pa.stop();
                }
            }
            return;
        }
        self.audio_off = false;
        let sp = self.play_speed();
        let (si, local) = locate_video_seg(&self.segs, self.t_ms);
        let path = self.segs[si].path.clone();
        let pa = self
            .preview_audio
            .get_or_insert_with(crate::preview_audio::PreviewAudio::new);
        if self.seek_settle {
            pa.set_paused(true);
            return;
        }
        pa.set_paused(false);
        pa.play(&path, local, sp);
    }

    /// Kick off an asynchronous (re)prepare of the whole preview from the current
    /// settings_path. Never blocks the GUI thread.
    fn start_async_apply(&mut self) {
        let path = self.settings_path.to_string();
        if path.is_empty() {
            return;
        }
        self.set_busy(true, "正在加载...");
        let shared: Arc<Mutex<Option<PendingApply>>> = Arc::new(Mutex::new(None));
        self.pending_apply = Some(shared.clone());
        std::thread::spawn(move || {
            let res = compute_apply(&path);
            let mut g = shared.lock().unwrap();
            *g = Some(res);
        });
    }

    /// Poll the pending background prepare (called from `tick` on the GUI thread) and
    /// land it when ready.
    fn poll_apply(&mut self) {
        let Some(shared) = self.pending_apply.clone() else {
            return;
        };
        let mut done: Option<PendingApply> = None;
        if let Ok(mut g) = shared.lock() {
            if g.is_some() {
                done = g.take();
            }
        }
        if let Some(res) = done {
            self.pending_apply = None;
            self.apply_prepared(res);
        }
    }

    /// Land a prepared apply result onto the item (GUI thread).
    fn apply_prepared(&mut self, res: PendingApply) {
        if let Some(e) = res.error {
            eprintln!("TraceReelItem apply failed: {e}");
            self.apply_summary = QString::from(format!("失败: {e}").as_str());
            self.set_busy(false, "");
            return;
        }
        self.video_image = None;
        // 完整重载落地计数（诊断/自动化回归用，见 `debug_full_landed`）。
        self.debug_full_landed += 1.0;
        // 视频源是否真的换了（预览时间轴由分段路径推出）：换了就没必要保留播放位置。
        let video_changed = {
            let old: Vec<&str> = self.segs.iter().map(|s| s.path.as_str()).collect();
            let new: Vec<&str> = res.segs.iter().map(|s| s.path.as_str()).collect();
            old != new
        };
        let n_maps = res.maps.len();
        let first_dim = res
            .maps
            .first()
            .map(|m| (m.w, m.h, m.rgba.len()))
            .unwrap_or((0, 0, 0));
        self.maps = res
            .maps
            .into_iter()
            .map(|m| LiveMap {
                image: qtwrap::qimage_from_rgba(m.w, m.h, &m.rgba),
                start_ms: m.start_ms,
                rm: m.rm,
            })
            .collect();
        self.video_info = Some(res.info.clone());
        self.segs = res.segs.clone();
        self.fps = res.info.fps;
        self.duration_ms = res.info.duration_ms;
        // 源尺寸：预览在「与原始视频一致」时按它排版（见字段注释）。
        self.source_size = QString::from(format!("{}x{}", res.info.width, res.info.height).as_str());
        self.source_size_changed();
        self.gpx = res.gpx;
        self.processing = res.processing;
        self.gyro_has_motion = res.info.has_motion_data;
        self.settings = Some(res.settings.clone());
        // 自适应配速的端点（QML 在取消「自适应」时用它把滑杆填成当前色带）。
        if let Some((low, high)) = res.pace_auto {
            self.pace_auto_low = low;
            self.pace_auto_high = high;
        }
        // 位置语义：播放中保留播放位置（新链会在该位置冷启动，ffmpeg -ss 直接到位），
        // 暂停时沿用旧语义从 0 重放。
        self.keep_playhead_after_apply(video_changed);
        // Clear the failed latch only when the gyro-relevant inputs actually changed
        // (video / preset / lens / enabled). A re-apply with the same inputs must NOT reset it,
        // otherwise play-triggered applies would keep re-loading a failing stabilizer.
        let gyro_sig = hard_gyro_sig(&res.settings);
        if gyro_sig != self.last_gyro_sig {
            self.gyro_load_failed = false;
            self.last_gyro_sig = gyro_sig;
        }
        // 完整 apply 落地：在途加载期间排队的「只改参数」改动已带着旧设置，作废。
        self.pending_gyro_apply = None;
        eprintln!("[apply] landed: maps={} main={}x{} main_bytes={} video={}x{} fps={} dur_ms={} motion={} processing={}x{}",
            n_maps, first_dim.0, first_dim.1, first_dim.2, res.info.width, res.info.height, res.info.fps, res.info.duration_ms, res.info.has_motion_data, res.processing.0, res.processing.1);
        let mut summary = format!(
            "地图 {} 张（主图 {}x{}）· 视频 {}x{} @{:.2}fps · 时长 {:.1}s · 运动数据 {}",
            n_maps,
            first_dim.0,
            first_dim.1,
            res.info.width,
            res.info.height,
            res.info.fps,
            res.info.duration_ms / 1000.0,
            if res.info.has_motion_data { "有" } else { "无" }
        );
        if res.settings.video.all_paths().iter().filter(|p| !p.is_empty()).count() > 1 {
            summary.push_str(&format!(" · 视频分段 {}", res.segs.len()));
        }
        if res.settings.map.extra_maps().iter().any(|(_, t)| t.is_none()) {
            summary.push_str(" · 有额外地图未设换图时间（未参与）");
        }
        self.apply_summary = QString::from(summary.as_str());

        // Gyro: load when it is enabled and the video has motion data. This covers:
        //   - checking the 启用防抖 box (reload_gyro=true),
        //   - the user pressing play (self.playing true),
        //   - loading an .orv that already has 启用防抖 on (reload_gyro=false but enabled).
        // A video with no motion data never triggers a load (see start_gyro_load guard).
        let should_load = res.settings.gyro.enabled && res.info.has_motion_data;
        if should_load && self.gyro.is_none() && !self.gyro_load_failed {
            self.start_gyro_load(&res.settings, res.processing.0, res.processing.1);
        } else if !res.settings.gyro.enabled || !res.info.has_motion_data {
            // Nothing to load (feature off or video has no motion data): make sure the
            // busy/loading overlay is not left covering the preview.
            self.set_gyro_loading(false);
        }
        self.update_gyro_status();
        if self.playing {
            // 播放中重载：把时钟按在当前位置，等第一帧从新链落地再放行（poll 收到帧会
            // 释放并重锚）。否则等待期间播放头会跑走，第一帧回来时画面对不上。
            self.seek_settle = true;
            self.seek_settle_since = Some(std::time::Instant::now());
            self.seam_hold = None;
        }
        // 地图/轨迹先按当前位置对齐一次再重画：否则要等第一帧预览到达才刷新，
        // 期间地图停在建链时的 t=0 机位 —— 看起来就是"地图显示不正确、也不动"。
        {
            let ft = if self.fps > 0.0 { 1000.0 / self.fps } else { 16.0 };
            let t_sec = self.t_ms / 1000.0;
            for lm in &mut self.maps {
                lm.rm.update(t_sec, ft);
            }
        }
        self.refresh_preview_config();
        self.request_preview_frame(self.t_ms);
        // Trigger a repaint now: the map/route were just loaded, but `paint` is only
        // re-entered from this.update(). If the preview worker never publishes a frame
        // (no video selected, decode still warm/blocked), the map would never draw.
        let this: &dyn QQuickItem = &*self;
        this.update();
        self.set_busy(false, "");
    }

    /// 应用新设置时如何安排播放头：暂停时沿用旧语义（从 0 重放），播放中**保留当前
    /// 位置**并把实时时钟重锚到它。这是「播放中改设置 → 重载后黑屏」的主修：
    /// 播放头一旦被清零，后台重载会通知 worker 在 t=0 建解码链，而 tick 立刻把请求
    /// 跳回原位（分钟级）→ 读线程把 `pts < last_req-1000` 的帧全丢掉，ring 永远是空的，
    /// 既没有画面、也没有新帧去驱动地图/HUD，只有 QML 的进度条在按墙钟走。
    fn keep_playhead_after_apply(&mut self, video_changed: bool) {
        if !self.playing || video_changed {
            self.t_ms = 0.0;
            return;
        }
        // 新设置的时长可能更短：贴到末尾会让 tick 的"到头回零"逻辑把画面甩回开头。
        if self.duration_ms > 0.0 && self.t_ms >= self.duration_ms {
            self.t_ms = (self.duration_ms - 1000.0).max(0.0);
        }
        self.t_ms = self.t_ms.max(0.0);
        self.play_anchor_wall = Some(std::time::Instant::now());
        self.play_anchor_t = self.t_ms;
    }

    /// 后台重算稳定化参数（不重读运动数据）。跑完由 [`Self::poll_recompute`] 收尾。
    fn start_recompute(
        &mut self,
        s: Settings,
        prop: String,
        snapshot: Vec<Option<Arc<Stabilizer>>>,
    ) {
        let offset_ms = gyro_offset_ms(&s, self.fps);
        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.recompute_done = Some(done.clone());
        self.set_busy(true, "正在重算稳定化参数…");
        let gyro = s.gyro.clone();
        std::thread::spawn(move || {
            let t0 = std::time::Instant::now();
            let n = crate::gyro::reapply_settings(&snapshot, &gyro, offset_ms, &prop);
            eprintln!(
                "[stab] re-applied '{prop}' to {n} segment(s) in {:.2}s (no motion-data reload)",
                t0.elapsed().as_secs_f64()
            );
            done.store(true, std::sync::atomic::Ordering::SeqCst);
        });
    }

    /// GUI 线程收尾：重算结束后请求新帧重绘；排队中的改动接着跑。
    fn poll_recompute(&mut self) {
        let Some(done) = self.recompute_done.clone() else {
            return;
        };
        if !done.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        self.recompute_done = None;
        self.set_busy(false, "");
        self.update_gyro_status();
        self.refresh_preview_config();
        self.request_preview_frame(self.t_ms);
        let this: &dyn QQuickItem = &*self;
        this.update();
        if let Some((s, prop)) = self.recompute_pending.take() {
            let snapshot: Vec<Option<Arc<Stabilizer>>> = self
                .gyro
                .as_ref()
                .and_then(|g| g.lock().ok().map(|g| g.clone()))
                .unwrap_or_default();
            self.start_recompute(s, prop, snapshot);
        }
    }

    fn ensure_preview_worker(&mut self) {
        if self.preview_worker_running {
            return;
        }
        self.preview_worker_running = true;
        self.preview_stop.store(false, Ordering::SeqCst);
        let control = self.preview_control.clone();
        let slot = self.preview_frame.clone();
        let stop = self.preview_stop.clone();
        std::thread::spawn(move || {
            let mut generation = u64::MAX;
            let mut decoder: Option<PipeDecoder> = None;
            let mut soft: Option<SoftDecoder> = None;
            // Which segment the current decoder/soft belongs to, plus a pre-warmed
            // decoder for the NEXT segment (eliminates the seam restart stall).
            let mut seg_idx: usize = usize::MAX;
            let mut next_pipe: Option<(usize, PipeDecoder)> = None;
            let mut seen_request = u64::MAX;
            let mut stab_logged = false;
            // Seam diagnostics: wall time of the last published frame; a publish gap much
            // larger than one frame interval IS the visible stutter at the stitch.
            let mut last_publish: Option<std::time::Instant> = None;
            while !stop.load(Ordering::SeqCst) {
                let req = control.lock().map(|g| g.clone()).unwrap_or_default();
                if req.video.is_empty() || req.pw == 0 || req.ph == 0 {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    continue;
                }
                // No stitch info (legacy single-video): treat `video` as the whole timeline.
                let segs: Vec<VideoSeg> = if req.segs.is_empty() {
                    vec![VideoSeg {
                        path: req.video.clone(),
                        start_ms: 0.0,
                        duration_ms: f64::INFINITY,
                        fps: req.fps,
                        frame_count: 0,
                        has_motion: false,
                    }]
                } else {
                    req.segs.clone()
                };
                if req.generation != generation {
                    generation = req.generation;
                    decoder = None;
                    soft = None;
                    next_pipe = None;
                    seg_idx = usize::MAX;
                    seen_request = u64::MAX;
                }
                if req.request_id == seen_request {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    continue;
                }
                seen_request = req.request_id;
                let requested_id = req.request_id;
                // Snap the request to the frame grid WITHIN the owning segment (all
                // segments share the same fps): the quaternion and the decoded frame
                // content must correspond to the SAME frame (see export path comments).
                let dt = if req.fps > 0.0 {
                    1000.0 / req.fps
                } else {
                    16.6833
                };
                let (i, t_local) = locate_video_seg(&segs, req.t_ms);
                let t_snap = ((t_local / dt).round() * dt).max(0.0);
                // Crossed into another file: switch decoders. Prefer the pre-warmed pipe
                // for this segment so playback does not stall at the stitch point.
                if i != seg_idx {
                    let warmed = next_pipe.take().filter(|(j, _)| *j == i).map(|(_, p)| p);
                    eprintln!(
                        "[worker] seam: seg {} -> {} at t_local={t_snap:.0} warmed={}",
                        seg_idx,
                        i,
                        warmed.is_some()
                    );
                    decoder = Some(match warmed {
                        Some(p) => p,
                        None => {
                            let mut p = PipeDecoder::new(&segs[i].path, req.pw, req.ph, req.fps);
                            p.spawn_at(t_snap);
                            p
                        }
                    });
                    soft = None;
                    seg_idx = i;
                }
                // Pre-warm the next file near the seam (within 4 s of its end) while
                // playing: the ffmpeg chain spawns and decodes its first frames, so the
                // crossing publishes a frame with no re-warm gap. Evaluated BEFORE frame
                // retrieval so a cold-decode miss (the `continue` below) can never bypass it.
                if req.playing
                    && seg_idx + 1 < segs.len()
                    && segs[seg_idx].duration_ms - t_snap <= 4000.0
                    && !matches!(&next_pipe, Some((j, _)) if *j == seg_idx + 1)
                {
                    let mut p = PipeDecoder::new(&segs[seg_idx + 1].path, req.pw, req.ph, req.fps);
                    if p.spawn_at(0.0) {
                        eprintln!("[pipe] pre-warm seg {}", seg_idx + 1);
                        next_pipe = Some((seg_idx + 1, p));
                    } else {
                        eprintln!("[pipe] pre-warm seg {} failed", seg_idx + 1);
                    }
                }
                let mut rgba: Option<Vec<u8>> = None;
                let mut pipe_dead = false;
                if let Some(p) = decoder.as_mut() {
                    // 1x 时要等"正好那一帧"（画面才准）；加速播放时时钟本来就跑在解码
                    // 前面、注定要跳帧，等满 180ms 只会把每次 tick 都拖慢 —— 直接拿最新的。
                    rgba = p
                        .get_with(t_snap, req.playing && req.speed > 1.01)
                        .map(|f| f.as_ref().clone());
                    pipe_dead = rgba.is_none() && p.dead();
                }
                if rgba.is_none() && pipe_dead {
                    // Only a permanently-dead hardware chain falls back to software; a
                    // transient miss during a seek is resolved by the playhead-hold in `tick`
                    // (see seek_settling), NOT by slow software decoding — a per-frame 16 GB
                    // software seek would stall playback for seconds at a time.
                    decoder = None;
                    if soft.is_none() {
                        soft = SoftDecoder::open(&segs[seg_idx].path, req.pw, req.ph).ok();
                    }
                    let mut buf = vec![0u8; req.pw * req.ph * 4];
                    if let Some(d) = soft.as_mut() {
                        if d.decode_at(t_snap, &mut buf).is_ok() {
                            rgba = Some(buf);
                        }
                    }
                }
                let Some(mut rgba) = rgba else {
                    // Cold decoder or latest seek not ready: let the GUI resubmit the same
                    // timestamp with a new request id; never block the GUI thread.
                    continue;
                };
                let mut stabilized = false;
                if !req.preview_perf {
                    if let Some(shared) = req.gyro.as_ref() {
                        // Per-segment stabilizer: each file's motion data is file-local, so
                        // the stabilizer is fed the LOCAL timestamp / frame index.
                        let stab = shared
                            .lock()
                            .ok()
                            .and_then(|g| g.get(seg_idx).cloned().flatten());
                        if let Some(stab) = stab {
                            let fi = ((t_snap / 1000.0) * req.fps).round() as usize;
                            let mut out = vec![0u8; req.pw * req.ph * 4];
                            if stab
                                .process_frame(
                                    t_snap, fi, &mut rgba, req.pw, req.ph, &mut out, req.pw, req.ph,
                                )
                                .is_ok()
                            {
                                rgba = out;
                                stabilized = true;
                                if !stab_logged {
                                    eprintln!(
                                        "[stab] preview worker stabilization active (pw={} ph={})",
                                        req.pw, req.ph
                                    );
                                    stab_logged = true;
                                }
                            }
                        }
                    }
                }
                // Drop stale results: only publish when this is still the latest request.
                let (same_generation, still_latest) = control
                    .lock()
                    .map(|g| {
                        (
                            g.generation == generation,
                            g.request_id == requested_id && g.generation == generation,
                        )
                    })
                    .unwrap_or((false, false));
                // During continuous playback, a request is naturally superseded while the
                // frame is being processed; publish the newest completed frame anyway. For
                // paused/seek requests, only publish if it is still the latest request.
                if same_generation && (req.playing || still_latest) {
                    let t_global = segs[seg_idx].start_ms + t_snap;
                    if let Some(lp) = last_publish {
                        let gap = lp.elapsed().as_millis();
                        if req.playing && gap > 150 {
                            eprintln!(
                                "[worker] PUBLISH GAP {gap}ms at seg {seg_idx} t={t_snap:.0}"
                            );
                        }
                    }
                    last_publish = Some(std::time::Instant::now());
                    if let Ok(mut s) = slot.lock() {
                        *s = Some(PreviewFrame {
                            request_id: requested_id,
                            t_ms: t_global,
                            seg_idx,
                            rgba,
                            w: req.pw,
                            h: req.ph,
                            stabilized,
                        });
                    }
                }
            }
        });
    }

    fn refresh_preview_config(&mut self) {
        self.ensure_preview_worker();
        // Segments may change; drop the seam bookkeeping until the first fresh frame lands.
        self.shown_seg = None;
        self.seam_hold = None;
        let Some(settings) = self.settings.as_ref().cloned() else {
            return;
        };
        let (pw, ph) = self.processing;
        if let Ok(mut c) = self.preview_control.lock() {
            c.generation = c.generation.wrapping_add(1);
            c.request_id = c.request_id.wrapping_add(1);
            c.video = settings.video.video_path.clone();
            c.segs = self.segs.clone();
            c.pw = pw;
            c.ph = ph;
            c.fps = self.fps;
            c.preview_perf = settings.preview_perf;
            c.gyro = self.gyro.clone();
            c.t_ms = self.t_ms;
            c.playing = self.playing;
            c.speed = self.play_speed();
            self.preview_request_id = c.request_id;
        }
        if let Ok(mut s) = self.preview_frame.lock() {
            *s = None;
        }
    }

    fn request_preview_frame(&mut self, t_ms: f64) {
        self.ensure_preview_worker();
        let speed = self.play_speed();
        if let Ok(mut c) = self.preview_control.lock() {
            c.t_ms = t_ms.max(0.0);
            c.playing = self.playing;
            c.speed = speed;
            c.gyro = self.gyro.clone();
            c.request_id = c.request_id.wrapping_add(1);
            self.preview_request_id = c.request_id;
        }
    }

    fn poll_preview_frame(&mut self) {
        let frame = self.preview_frame.lock().ok().and_then(|mut s| s.take());
        if let Some(frame) = frame {
            // 诊断计数：这个数不涨 = 画面没有新帧（"黑屏"症状的第一手判据）。
            self.debug_frames += 1.0;
            // Every frame the worker writes to the slot is from the current generation (the
            // worker only publishes when same_generation is true), so accept it regardless of
            // the exact request id. Requiring request_id == preview_request_id would reject
            // every frame during playback, because `tick` re-requests each frame (advancing
            // preview_request_id) before poll reads the published result — leaving the video
            // frozen. The `t_ms < self.t_ms` guard below still prevents the timeline from
            // regressing on an older frame.
            {
                // During continuous playback the worker may publish frames out of order (it
                // publishes the newest *completed* frame even when superseded), so a stale
                // frame whose snapped t is earlier than the current clock would yank the
                // runner/camera backward — the "advance then retreat" on the map near the
                // start. Only let the displayed time move forward while playing; when paused
                // (a seek) accept whatever was requested.
                if self.playing && frame.t_ms < self.t_ms {
                    // Keep the video frame for smoothness but do not regress the time used to
                    // drive the runner/camera (rm.update is called with self.t_ms below).
                } else {
                    self.t_ms = frame.t_ms;
                }
                self.stab_failed = !frame.stabilized
                    && self.gyro_slot_ready(frame.seg_idx)
                    && !self
                        .settings
                        .as_ref()
                        .map(|s| s.preview_perf)
                        .unwrap_or(false);
                self.video_image = Some(qtwrap::qimage_from_rgba(
                    frame.w as i32,
                    frame.h as i32,
                    &frame.rgba,
                ));
                // Update EVERY map's route manager, not just the one currently shown:
                // after a mid-video map switch the new manager's runner/camera state must
                // already be current (the cost is negligible compared to one frame).
                self.advance_map_runners();
                let this: &dyn QQuickItem = &*self;
                this.update();
                self.shown_seg = Some(frame.seg_idx);
                // A frame landed: release the post-seek hold and re-anchor the wall clock so
                // playback resumes smoothly from the frame actually shown (not from where the
                // clock would have been had it kept running during the pipe's re-warm).
                // During a seam hold only a frame from the awaited segment (or later)
                // releases it — a late tail frame from the previous file must not resume the
                // clock, or the stale-restart cascade at the seam continues.
                let releases = self.seam_hold.map_or(true, |h| frame.seg_idx >= h);
                if self.seek_settle && releases {
                    if let (Some(h), Some(s)) = (self.seam_hold, self.seek_settle_since) {
                        eprintln!("[gui] seam hold released: awaited seg {h}, frame seg {} arrived after {}ms", frame.seg_idx, s.elapsed().as_millis());
                    }
                    self.seek_settle = false;
                    self.seek_settle_since = None;
                    self.seam_hold = None;
                    if self.playing {
                        self.play_anchor_wall = Some(std::time::Instant::now());
                        self.play_anchor_t = self.t_ms;
                    }
                }
            }
        }
    }

    fn set_busy(&mut self, on: bool, msg: &str) {
        if self.busy != on {
            self.busy = on;
            self.busy_changed();
        }
        if self.busy_msg.to_string() != msg {
            self.busy_msg = QString::from(msg);
            self.busy_msg_changed();
        }
    }

    // ---------- encode (export) plumbing ----------

    fn set_encoding(&mut self, on: bool) {
        if self.encoding != on {
            self.encoding = on;
            self.encoding_changed();
        }
    }

    fn set_encode_state(&mut self, p: EncodeProgress) {
        let frame = p.frame as i64;
        let total = p.total_frames as i64;
        if self.encode_frame != frame
            || self.encode_total_frames != total
            || (self.encode_elapsed_s - p.elapsed_s).abs() > 0.25
            || self.encode_out_size != p.out_size
        {
            self.encode_frame = frame;
            self.encode_total_frames = total;
            self.encode_elapsed_s = p.elapsed_s;
            self.encode_remaining_s = p.remaining_s;
            self.encode_fps = p.fps;
            self.encode_out_size = p.out_size;
            self.encode_current_t_ms = p.current_t_ms;
            self.encode_duration_ms = p.duration_ms;
            self.encode_state_changed();
        }
    }

    /// Land a terminal encode state (also used for pre-flight validation failures).
    fn finish_encode(&mut self, aborted: bool, output: &str, error: &str) {
        if let Some(i) = self.queue_active.take() {
            if !aborted && error.is_empty() {
                self.export_entries.remove(i);
            } else {
                let e = &mut self.export_entries[i];
                e.status = if aborted { "已中止" } else { "失败" }.into();
                e.error = error.into(); e.progress = self.queue_progress;
                self.queue_running = false;
            }
            self.persist_queue(); self.publish_queue();
        }
        self.encode_handle = None;
        self.set_encoding(false);
        self.encode_paused = false;
        self.encode_aborted = aborted;
        self.encode_output = QString::from(output);
        self.encode_error = QString::from(error);
        self.encode_done = true;
        self.encode_done_changed();
        self.encode_state_changed();
        eprintln!("[encode] done: aborted={aborted} error={error:?}");
    }

    /// GUI-thread encode poll: read the worker's progress into the Qt properties and
    /// finalize when the worker reports done. It does no compositing itself.
    fn poll_encode(&mut self) {
        if !self.encoding {
            return;
        }
        let Some(handle) = self.encode_handle.clone() else {
            return;
        };
        // Reflect the progress the worker published since the last tick.
        self.set_encode_state(handle.get());
        let progress = handle.overall_progress();
        if (self.queue_progress - progress).abs() > 0.0001 {
            self.queue_progress = progress; self.encode_state_changed();
        }
        // 逐段导出：同步当前分段说明到弹窗（仅在变化时发信号，避免每 tick 刷新绑定）。
        let seg = QString::from(handle.segment_label().as_str());
        if self.encode_segment != seg {
            self.encode_segment = seg;
            self.encode_state_changed();
        }
        // 非致命提示（如自动改用 CPU）：变化时才发信号。
        let notice = QString::from(handle.notice().as_str());
        if self.encode_notice != notice {
            self.encode_notice = notice;
            self.encode_state_changed();
        }
        if handle.is_done() {
            if let Some(r) = handle.take_result() {
                self.finish_encode(r.aborted, &r.output, &r.error);
            } else {
                // Done without a result (shouldn't happen): treat as aborted-safe close.
                let p = self.encode_out_path.clone();
                self.finish_encode(true, &p, "编码已结束（无结果）");
            }
        }
    }

    fn start_gyro_load(&mut self, settings: &Settings, pw: usize, ph: usize) {
        if self.gyro.is_some() {
            return;
        }
        let segs = self.segs.clone();
        eprintln!(
            "[stab] start_gyro_load called: motion={} segs={} pw={} ph={}",
            self.gyro_has_motion,
            segs.len(),
            pw,
            ph
        );
        // Hard guard: never attempt a stabilizer load when NO video has motion data
        // (would otherwise run / freeze on an empty data stream).
        if !self.gyro_has_motion {
            eprintln!("[stab] gyro load skipped: video has no motion data");
            self.set_gyro_loading(false);
            self.update_gyro_status();
            return;
        }
        // 整份防抖设置（含镜头库标定、平滑/变焦/地平线等全部参数）交给 worker 线程。
        let gyro_settings = settings.gyro.clone();
        let offset_ms = gyro_offset_ms(settings, self.fps);
        // One slot per segment; each file's motion data is file-local, so every
        // stabilizer gets the SAME effective gyro<->video offset (no per-seg shift).
        let shared: Arc<Mutex<Vec<Option<Arc<Stabilizer>>>>> =
            Arc::new(Mutex::new(vec![None; segs.len()]));
        let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
        // Real progress sink: the loader thread stores frac + phase here and the GUI tick
        // reads them — lock-free, so the telemetry-parser byte-offset callback stays cheap
        // enough to call at ~10 Hz.
        let prog = Arc::new(ProgressSink::new());
        self.gyro = Some(shared.clone());
        self.gyro_done = Some(done.clone());
        self.gyro_progress_slot = Some(prog.clone());
        self.set_gyro_progress(0.0);
        self.set_gyro_loading(true);
        std::thread::spawn(move || {
            // Flip `done` even if this thread panics mid-load (the overlay waits on it).
            let _done_guard = LoadDoneGuard(done.clone());
            // Runs off the GUI thread; the manager is Qt-free and Send+Sync.
            // autosync=false: DJI motion data is GPS-synced (true offset ~0); running
            // the pure-Rust AKAZE autosync here is both unnecessary and extremely
            // expensive (decodes the video + seconds-per-frame feature matching), so it
            // is disabled for the interactive preview. The CLI (`stabvid`) exposes
            // `--autosync` for sources that genuinely need it.
            let t0 = std::time::Instant::now();
            // Overall progress is a per-segment budget. When several files must be loaded,
            // the last slice is reserved for the seam-warmup re-smoothing pass.
            let motion_segs = segs.iter().filter(|s| s.has_motion).count().max(1);
            let seg_share = if motion_segs > 1 { 0.9 } else { 1.0 };
            let mut seg_no = 0usize;
            for (i, seg) in segs.iter().enumerate() {
                if !seg.has_motion {
                    eprintln!(
                        "[stab] gyro seg {i}: no motion data, passthrough ({})",
                        seg.path
                    );
                    continue;
                }
                eprintln!("[stab] gyro seg {i}: loading for {}", seg.path);
                // Map this segment's 0..1 onto its slice of the overall bar.
                let base = seg_no as f64 / motion_segs as f64 * seg_share;
                let span = seg_share / motion_segs as f64;
                let report = |p: LoadProgress| prog.set(p.scaled(base, span));
                seg_no += 1;
                match load_stabilizer(
                    &seg.path,
                    &gyro_settings,
                    pw,
                    ph,
                    seg.fps,
                    seg.duration_ms,
                    seg.frame_count,
                    false,
                    offset_ms,
                    &report,
                ) {
                    Ok(s) => {
                        // Publish incrementally: earlier segments become usable while
                        // later ones still load.
                        if let Ok(mut g) = shared.lock() {
                            if let Some(slot) = g.get_mut(i) {
                                *slot = Some(Arc::new(s));
                            }
                        }
                    }
                    Err(e) => eprintln!("[stab] gyro seg {i} load FAILED: {e}"),
                }
            }
            eprintln!(
                "[stab] gyro load finished in {:.1}s",
                t0.elapsed().as_secs_f64()
            );
            // Seam continuity: re-smooth each segment with its adjacent segments' raw gyro
            // context, removing the "stabilization reset" at file seams. This mutates the
            // already-published stabilizers through their interior locks (no slot swap), so
            // preview and export both pick up the refined results. Snapshot first to keep
            // the slot mutex unlocked while re-smoothing (it can take a while).
            let snapshot: Vec<Option<Arc<Stabilizer>>> =
                shared.lock().map(|g| g.clone()).unwrap_or_default();
            let warmup = |p: LoadProgress| prog.set(p.scaled(seg_share, 1.0 - seg_share));
            apply_seam_warmup_smoothing(&snapshot, &warmup);
            prog.set(LoadProgress::new(1.0, crate::gyro::Phase::Done));
            done.store(true, std::sync::atomic::Ordering::SeqCst);
        });
    }

    fn set_gyro_loading(&mut self, on: bool) {
        if self.gyro_loading != on {
            self.gyro_loading = on;
            self.gyro_loading_changed();
            // Diagnostics: pair with the "[stab] progress" lines to see exactly where the
            // loading overlay was released (it must sit at ~100%, not at the first segment).
            eprintln!(
                "[stab] loading overlay {} (progress {:.0}%)",
                if on { "shown" } else { "hidden" },
                self.gyro_progress * 100.0
            );
        }
        if !on {
            // Loading ended (done or failed): drop the sink and park the bar at 100%.
            // While loading, the value comes from the loader thread via `poll_gyro_progress`.
            self.gyro_progress_slot = None;
            self.set_gyro_progress(1.0);
            if !self.gyro_phase.is_empty() {
                self.gyro_phase = QString::default();
                self.gyro_phase_changed();
            }
        }
    }

    fn set_gyro_progress(&mut self, p: f64) {
        if (self.gyro_progress - p).abs() > 1e-4 {
            self.gyro_progress = p;
            self.gyro_progress_changed();
        }
    }

    /// GUI-thread poll of the loader thread's real progress (0..1; -1 = unknown) + phase.
    fn poll_gyro_progress(&mut self) {
        if !self.gyro_loading {
            return;
        }
        let Some(sink) = self.gyro_progress_slot.clone() else {
            return;
        };
        let p = sink.frac();
        if (0.0..=1.0).contains(&p) {
            // Diagnostics: one line per phase change / 10% bucket (a long load is otherwise
            // silent in the console, which makes bug reports hard to interpret).
            let phase = sink.phase();
            if self.gyro_phase != QString::from(phase.label())
                || (self.gyro_progress * 10.0) as i32 != (p * 10.0) as i32
            {
                eprintln!("[stab] progress {:.0}% ({})", p * 100.0, phase.label());
            }
            self.set_gyro_progress(p);
        }
        let label = QString::from(sink.phase().label());
        if self.gyro_phase != label {
            self.gyro_phase = label;
            self.gyro_phase_changed();
        }
    }

    /// Start the Gyroflow load if enabled + motion data + not already loaded/loading.
    fn ensure_gyro_loaded(&mut self) {
        let Some(settings) = self.settings.as_ref().cloned() else {
            return;
        };
        let (pw, ph) = self.processing;
        // Never retry a stabilizer load that already failed (would re-pop the overlay and
        // spin forever on play). The user can re-try by toggling the feature / changing video.
        if self.gyro_load_failed {
            return;
        }
        if settings.gyro.enabled
            && self.gyro_has_motion
            && self.gyro.is_none()
            && !self.gyro_loading
        {
            eprintln!("[stab] auto-loading gyro (play-triggered)");
            self.start_gyro_load(&settings, pw, ph);
        }
    }

    fn gyro_ready(&self) -> Option<Arc<Stabilizer>> {
        // "Ready" = at least one segment stabilizer has landed.
        if let Some(shared) = self.gyro.as_ref() {
            if let Ok(g) = shared.lock() {
                return g.iter().find_map(|s| s.clone());
            }
        }
        None
    }

    /// Whether the stabilizer for a specific segment index is loaded.
    fn gyro_slot_ready(&self, idx: usize) -> bool {
        self.gyro
            .as_ref()
            .map(|shared| {
                shared
                    .lock()
                    .map(|g| g.get(idx).map(|s| s.is_some()).unwrap_or(false))
                    .unwrap_or(false)
            })
            .unwrap_or(false)
    }

    /// True while the background loader thread is still running.
    fn gyro_load_pending(&self) -> bool {
        self.gyro_done
            .as_ref()
            .map(|d| !d.load(std::sync::atomic::Ordering::SeqCst))
            .unwrap_or(false)
    }

    fn set_gyro_status(&mut self, s: &str) {
        if self.gyro_status.to_string() != s {
            self.gyro_status = QString::from(s);
            self.gyro_status_changed();
        }
    }

    fn update_gyro_status(&mut self) {
        let (gy_enabled, gy_has_motion, preview_perf) = match self.settings.as_ref() {
            Some(s) => (s.gyro.enabled, self.gyro_has_motion, s.preview_perf),
            None => return,
        };
        // A stabilizer load that finished but did not produce a ready stabilizer is a
        // failure: clear the gyro slot so `has_gyro` goes false and the loading overlay
        // is released (otherwise it would stick on "加载中" forever), and latch `failed` so
        // the play-triggered loader doesn't retry it every tick.
        if let Some(done) = self.gyro_done.clone() {
            if done.load(std::sync::atomic::Ordering::SeqCst) && self.gyro_ready().is_none() {
                eprintln!("[stab] gyro load failed; clearing so preview is not blocked");
                self.gyro_load_failed = true;
                self.gyro = None;
                self.gyro_done = None;
            }
        }
        let ready = self.gyro_ready().is_some();
        let has_gyro = self.gyro.is_some();
        // The loader thread's OWN completion flag: false until every motion-bearing segment
        // has landed AND the seam-warmup re-smoothing pass has finished. `ready` is NOT
        // enough — it only means the FIRST segment landed, which with several files happens
        // at ~1/n of the overall bar (2 files → 45%). Using `!ready` to release the overlay
        // made the bar stop at ~half and vanish while the rest was still loading (the "加载
        // 到一半就直接结束" report), even though the load did finish in the background.
        let pending = has_gyro && self.gyro_load_pending();
        // Any motion-bearing segment whose slot is still empty (loading or failed).
        let mut slot_missing = false;
        if let Some(shared) = self.gyro.as_ref() {
            if let Ok(g) = shared.lock() {
                for (i, seg) in self.segs.iter().enumerate() {
                    if seg.has_motion && g.get(i).map(|s| s.is_none()).unwrap_or(true) {
                        slot_missing = true;
                        break;
                    }
                }
            }
        }
        let status = if !gy_enabled {
            "防抖: 关闭".to_string()
        } else if !gy_has_motion {
            "防抖: 无运动数据（该视频没有传感器数据）".to_string()
        } else if self.gyro_load_failed {
            "防抖: 加载失败（请检查预设/视频，见日志）".to_string()
        } else if self.recompute_done.is_some() {
            // 参数改动后的重算（不重读运动数据）：原版 Gyroflow 这里也是一个「计算中」状态。
            "防抖: 正在重算参数…（不重读运动数据）".to_string()
        } else if !has_gyro {
            "防抖: 待加载".to_string()
        } else if pending {
            // Whole load still running (later segments and/or the seam-warmup pass): keep
            // reporting the real phase + percent instead of the "已就绪" line — the overlay
            // is up, so claiming "ready" there would contradict what the user sees.
            self.gyro_loading_status()
        } else if ready {
            if preview_perf {
                "防抖: 已就绪（预览时关闭）".to_string()
            } else if self.stab_failed {
                "防抖: 已就绪（逐帧应用失败，见日志）".to_string()
            } else if slot_missing {
                "防抖: 已就绪（部分分段无防抖，见日志）".to_string()
            } else {
                "防抖: 已就绪".to_string()
            }
        } else {
            // Not ready and no load pending: the thread died without a stabilizer (the
            // failure branch above normally clears `gyro` first). Show the plain text.
            "防抖: 加载中...".to_string()
        };
        self.set_gyro_status(&status);
        self.refresh_gyro_stats();
        // 加载期间排队的参数改动：等这次加载真正结束（含分段缝合预热）且成功后，做一次分级重算
        // 把新参数应用上去。加载失败就丢掉这条排队（参数已存进 self.settings，下次加载会用上）。
        if self.pending_gyro_apply.is_some() {
            if self.gyro_load_failed {
                eprintln!("[stab] dropping queued gyro param: the motion-data load failed");
                self.pending_gyro_apply = None;
            } else if ready && !pending {
                if let Some((s, prop)) = self.pending_gyro_apply.take() {
                    eprintln!("[stab] applying queued gyro param '{prop}' after the load landed");
                    // 与 apply_gyro_params 一致：已经有重算在跑就排队，别开第二个线程。
                    if self.recompute_done.is_some() {
                        self.recompute_pending = Some((s, prop));
                    } else {
                        let snapshot: Vec<Option<Arc<Stabilizer>>> = self
                            .gyro
                            .as_ref()
                            .and_then(|g| g.lock().ok().map(|g| g.clone()))
                            .unwrap_or_default();
                        self.start_recompute(s, prop, snapshot);
                    }
                }
            }
        }
        // Drive the loading overlay: it stays up until the loader THREAD is done — every
        // motion-bearing segment loaded plus the seam-warmup re-smoothing — so the bar runs
        // to the end instead of being cut off when the first segment lands.
        let loading = gy_enabled && gy_has_motion && has_gyro && pending;
        self.set_gyro_loading(loading);
    }

    /// 更新「最大旋转 / 最大缩放」只读状态 + 「是否有逐帧焦距」标志（每 tick 调一次，开销很小）。
    /// 对齐原版 Stabilization 面板底部的 `Max rotation: Pitch/Yaw/Roll` 与 `Max zoom`。
    fn refresh_gyro_stats(&mut self) {
        let (stats, has_fl) = match self.gyro_ready() {
            Some(s) => {
                let (pitch, yaw, roll) = s.manager.get_smoothing_max_angles();
                let min_fov = {
                    let m = s.manager.params.read();
                    m.fovs.iter().copied().fold(f64::INFINITY, f64::min)
                };
                let zoom = if min_fov.is_finite() && min_fov > 0.0001 {
                    100.0 / min_fov
                } else {
                    0.0
                };
                let has_fl = s.manager.gyro.read().file_metadata.read().lens_params.len() > 0;
                (
                    serde_json::json!({
                        "pitch": pitch, "yaw": yaw, "roll": roll, "zoom": zoom
                    })
                    .to_string(),
                    has_fl,
                )
            }
            None => ("{}".to_string(), false),
        };
        if self.gyro_stats.to_string() != stats {
            self.gyro_stats = QString::from(stats.as_str());
            self.gyro_stats_changed();
        }
        if self.gyro_has_focal_length != has_fl {
            self.gyro_has_focal_length = has_fl;
            self.gyro_has_focal_length_changed();
        }
    }

    /// Status text while the loader thread is still running: the real phase + percent when
    /// the loader published one, else a plain "加载中".
    fn gyro_loading_status(&self) -> String {
        let p = self.gyro_progress;
        if (0.0..=1.0).contains(&p) {
            let phase = self.gyro_phase.to_string();
            let phase = if phase.is_empty() {
                "加载运动数据".to_string()
            } else {
                phase
            };
            format!("防抖: {phase} {:.0}%", p * 100.0)
        } else {
            "防抖: 加载中...".to_string()
        }
    }
}

impl QQuickItem for TraceReelItem {
    fn component_complete(&mut self) {
        // Initial load: asynchronous and WITHOUT gyro (see reload rules).
        self.start_async_apply();
    }
}

impl QQuickPaintedItem for TraceReelItem {
    fn paint(&mut self, p: &mut QPainter) {
        let Some(settings) = self.settings.as_ref().cloned() else {
            return;
        };
        // Pick the active map by global timeline: the one with the greatest start_ms
        // not exceeding the current time (index 0 / the main map before any switch).
        // With no map at all the scene still renders (video only), like before.
        let (rm, map_image, route_start_ms) = if self.maps.is_empty() {
            (None, None, 0.0)
        } else {
            let starts: Vec<f64> = self.maps.iter().map(|m| m.start_ms).collect();
            let idx = locate_map(&starts, self.t_ms);
            (
                Some(self.maps[idx].rm.clone()),
                Some(self.maps[idx].image.clone()),
                self.maps[idx].start_ms,
            )
        };
        let (W, H) = (
            qtwrap::qquick_item_width(self.get_cpp_object()),
            qtwrap::qquick_item_height(self.get_cpp_object()),
        );
        if W <= 0.0 || H <= 0.0 {
            return;
        }
        // `draw_scene` only touches `rm` inside the map block, so a default manager is
        // a safe placeholder when no map is configured (video-only preview).
        let rm = rm.unwrap_or_default();
        let has_project_content = self.video_image.is_some()
            || map_image.is_some()
            || self.gpx.is_some()
            || !settings.video.video_path.trim().is_empty()
            || !settings.map.image_file_path.trim().is_empty()
            || !settings.route.quick_route_jpeg_file_path.trim().is_empty()
            || !settings.hud.gpx_file_path.trim().is_empty();
        // 预览按导出页的三个勾选合成：没勾的层不画，剩下的铺满画布。
        let mut settings = preview_layers(settings);
        if !has_project_content {
            settings.hud.layout = Some(crate::hud::Layout {
                version: 1,
                widgets: Vec::new(),
            });
        }
        draw_scene(
            p,
            &rm,
            &settings,
            if settings.encode.include_map {
                map_image
            } else {
                None
            },
            if settings.encode.include_video {
                self.video_image.clone()
            } else {
                None
            },
            W,
            H,
            self.t_ms,
            route_start_ms,
            self.gpx.as_ref(),
        );
    }
}

/// 预览画面按「导出页勾了哪几层」合成：没勾的层不画，勾了的铺满画布。
///
/// 预览以前有自己的「HUD 预览」开关，和导出勾选各管各的，结果勾掉 HUD 导出、预览却还在画，
/// 或者只导地图+HUD 时预览画不出成品的样子。现在只有一套真相（`encode.include_*`）：
/// 这里把它翻译成 `draw_scene` 认的版面参数 —— 去掉地图就把地图带宽归零（视频铺满），
/// 去掉视频就让地图占满整帧，去掉 HUD 就把布局清空。
pub fn preview_layers(mut s: Settings) -> Settings {
    if !s.encode.include_map {
        s.map.relative_width = 0.0;
    }
    if !s.encode.include_video {
        s.map.relative_width = 1.0;
    }
    if !s.encode.include_hud {
        s.hud.layout = Some(crate::hud::Layout {
            version: 1,
            widgets: Vec::new(),
        });
    }
    s
}

/// Composite the map + route (left) and video (right) onto `p`. Shared by the GUI
/// window, the headless renderer, and the encode pump.
/// 「硬」防抖输入：**只有这些变化才需要真正重载稳定化器**（重探测视频 + 重读运动数据）。
/// 其余 gyro 参数（视野/变焦/平滑/地平线/卷帘/附加旋转平移/陀螺偏移…）都走
/// `apply_gyro_params` 的「只重算」路径 —— 与原版 Gyroflow 一致。
pub fn hard_gyro_sig(s: &Settings) -> String {
    format!(
        "{}\u{0}{}\u{0}{}\u{0}{}",
        s.video.all_paths().join("\u{1}"),
        s.gyro.preset_file_path,
        s.gyro.lens_profile,
        s.gyro.enabled
    )
}

/// 有没有配置任何地图/轨迹图片（主图或额外地图）。`draw_scene` 用它决定左侧地图带：
/// 没配地图时视频铺满整帧。判断依据是 SETTINGS 而非已加载的图，这样「配置了地图但还在
/// 加载」时布局不会跳动。导出解码尺寸（`encode::export_decode_size`）必须用同一个判据，
/// 否则算出来的视频面板和实际绘制的不一致。
pub fn map_configured(settings: &Settings) -> bool {
    !settings.map.image_file_path.trim().is_empty()
        || !settings.route.quick_route_jpeg_file_path.trim().is_empty()
        || settings
            .map
            .extra_map_files
            .iter()
            .any(|p| !p.trim().is_empty())
}

pub fn draw_scene(
    p: &mut QPainter,
    rm: &RouteManager,
    settings: &Settings,
    map: Option<QImage>,
    video: Option<QImage>,
    W: f64,
    H: f64,
    t_ms: f64,
    route_start_ms: f64,
    gpx: Option<&GpxTrack>,
) {
    // 没有配置任何地图/轨迹图片时（只做防抖 + HUD 的项目）左侧不留空带：视频铺满整帧。
    let map_configured = map_configured(settings);
    let rel_w = if map_configured {
        settings.map.relative_width
    } else {
        0.0
    };

    // Match the original renderer's quality hints: SmoothPixmapTransform keeps the map
    // crisp when it is scaled (this is the main sharpness fix alongside a higher map
    // downscale cap), antialiasing smooths the route polylines.
    p.set_render_hint(qmetaobject::qttypes::QPainterRenderHint::Antialiasing, true);
    p.set_render_hint(
        qmetaobject::qttypes::QPainterRenderHint::TextAntialiasing,
        true,
    );
    p.set_render_hint(
        qmetaobject::qttypes::QPainterRenderHint::SmoothPixmapTransform,
        true,
    );
    p.set_render_hint(
        qmetaobject::qttypes::QPainterRenderHint::HighQualityAntialiasing,
        true,
    );
    p.fill_rect(
        QRectF {
            x: 0.0,
            y: 0.0,
            width: W,
            height: H,
        },
        QBrush::from_color(QColor::from_rgba(0, 0, 0, 255)),
    );

    if let Some(map) = map {
        let mp_off_x = -(W / 2.0 - (rel_w * W) / 2.0);
        p.save();
        p.translate(QPointF {
            x: W / 2.0 + mp_off_x,
            y: H / 2.0,
        });
        p.rotate(-(settings.map.angle + settings.map.user_angle + rm.get_angle()));
        let s = settings.map.user_scale * rm.get_scale();
        p.scale(s, s);
        p.translate(QPointF {
            x: settings.map.x + settings.map.user_x + rm.get_x(),
            y: -(settings.map.y + settings.map.user_y + rm.get_y()),
        });
        let (tw, th) = (
            qtwrap::qimage_width(&map) as f64,
            qtwrap::qimage_height(&map) as f64,
        );
        p.draw_image_fit_rect(
            QRectF {
                x: -tw / 2.0,
                y: -th / 2.0,
                width: tw,
                height: th,
            },
            map,
        );
        draw_route(p, rm, settings, t_ms / 1000.0, route_start_ms);
        p.restore();
    }

    // Black out the video region so the map (which may overflow right) never shows under it.
    p.fill_rect(
        QRectF {
            x: rel_w * W,
            y: 0.0,
            width: (1.0 - rel_w) * W,
            height: H,
        },
        QBrush::from_color(QColor::from_rgba(0, 0, 0, 255)),
    );

    let hud_rect = crate::hud::viewport(settings, W, H, video.as_ref().map(|image|
        (qtwrap::qimage_width(image) as f64, qtwrap::qimage_height(image) as f64)));
    if let Some(video) = video {
        let (vw, vh) = (
            qtwrap::qimage_width(&video) as f64,
            qtwrap::qimage_height(&video) as f64,
        );
        // Letterbox: keep the video's own aspect ratio, fit the largest rectangle inside
        // the right region, centre it, and fill the remainder with black.
        let vp_w = (1.0 - rel_w) * W;
        let vp_h = H;
        let base = if vw > 0.0 && vh > 0.0 {
            (vp_w / vw).min(vp_h / vh)
        } else {
            1.0
        };
        let s = base * settings.video.scale;
        p.save();
        p.translate(QPointF {
            x: rel_w * W + vp_w / 2.0,
            y: H / 2.0,
        });
        p.rotate(settings.video.angle);
        p.scale(s, s);
        p.translate(QPointF {
            x: settings.video.x,
            y: settings.video.y,
        });
        p.draw_image_fit_rect(
            QRectF {
                x: -vw / 2.0,
                y: -vh / 2.0,
                width: vw,
                height: vh,
            },
            video,
        );
        p.restore();
    }

    crate::hud::draw_in_viewport(p, settings, gpx, t_ms, hud_rect);
}

/// Headless render: probe video, load map + route, decode a frame at `t_ms`, composite
/// to `out_w × out_h` and save as a PNG. Requires a QGuiApplication (create one via
/// QmlEngine). Returns the output path.
pub fn render_composite_to_png(
    settings: &Settings,
    t_ms: f64,
    out_w: i32,
    out_h: i32,
    out_path: &str,
) -> Result<String, String> {
    let video = settings.video.video_path.clone();
    let map = settings.map.image_file_path.clone();
    let info = crate::probe::probe_video(&video).ok_or("probe video failed")?;

    let map_img = load_map_image(&map)?;
    let (mw, mh) = (
        qtwrap::qimage_width(&map_img) as f64,
        qtwrap::qimage_height(&map_img) as f64,
    );
    let route_style = route_style_from_settings(settings);
    let (splits, rel) = (
        settings.splits.split_times.clone(),
        settings.splits.is_relative,
    );
    let view_mode = view_mode_from_settings(settings);
    let mut rm = RouteManager::default();
    rm.window_width = out_w as f64;
    rm.window_height = out_h as f64;
    apply_framing_settings(&mut rm, settings);
    init_route_manager(
        &mut rm,
        &settings.route.quick_route_jpeg_file_path,
        mw,
        mh,
        &splits,
        rel,
        route_style,
        view_mode,
        settings.map.relative_width,
    )?;
    // Advance the camera/runner to the requested time (matches the GUI playback path).
    let ft = if info.fps > 0.0 {
        1000.0 / info.fps
    } else {
        16.0
    };
    rm.update(t_ms / 1000.0, ft);

    // HUD 轨迹：与实时预览/导出同一条解析路径（工程里的 GPX → 视频同名 .gpx →
    // 轨迹文件自带的时间轴）。
    let gpx = resolve_hud_track(&settings.hud.gpx_file_path, &video, Some(&rm));

    let mut decoder =
        SoftDecoder::open(&video, info.width, info.height).map_err(|e| format!("decoder: {e}"))?;
    let mut rgba = vec![0u8; info.width * info.height * 4];
    decoder
        .decode_at(t_ms, &mut rgba)
        .map_err(|e| format!("decode: {e}"))?;
    let video_img = qtwrap::qimage_from_rgba(info.width as i32, info.height as i32, &rgba);

    let mut img = qtwrap::qimage_from_rgba(
        out_w,
        out_h,
        &vec![0u8; (out_w as usize) * (out_h as usize) * 4],
    );
    let mut p = qtwrap::qpainter_on_image(&mut img);
    draw_scene(
        &mut p,
        &rm,
        settings,
        Some(map_img),
        Some(video_img),
        out_w as f64,
        out_h as f64,
        t_ms,
        0.0,
        gpx.as_ref(),
    );
    drop(p);
    if qtwrap::qimage_save(&img, out_path) {
        Ok(out_path.to_string())
    } else {
        Err(format!("failed to save {out_path}"))
    }
}

/// Encode the composited (+ optionally Gyroflow-stabilized) video to a fragmented mp4.
/// Renders each frame offscreen at the settings window size, then pipes RGB24 to ffmpeg.
/// A `QGuiApplication` must already exist. Returns the output path.
pub fn encode_composite(
    settings: &Settings,
    out_path: &str,
    max_frames: usize,
) -> Result<String, String> {
    let video = settings.video.video_path.clone();
    let info = crate::probe::probe_video(&video).ok_or("probe video failed")?;
    let (out_w, out_h) = (settings.window.2 as usize, settings.window.3 as usize);
    let fps = info.fps;

    // Map + route.
    let map_img = load_map_image(&settings.map.image_file_path)?;
    let (mw, mh) = (
        qtwrap::qimage_width(&map_img) as f64,
        qtwrap::qimage_height(&map_img) as f64,
    );
    let route_style = route_style_from_settings(settings);
    let (splits, rel) = (
        settings.splits.split_times.clone(),
        settings.splits.is_relative,
    );
    let mut rm = RouteManager::default();
    rm.window_width = out_w as f64;
    rm.window_height = out_h as f64;
    apply_framing_settings(&mut rm, settings);
    init_route_manager(
        &mut rm,
        &settings.route.quick_route_jpeg_file_path,
        mw,
        mh,
        &splits,
        rel,
        route_style,
        view_mode_from_settings(settings),
        settings.map.relative_width,
    )?;

    // HUD 轨迹（导出用）：工程里的 GPX → 视频同名 .gpx → 轨迹文件自带的时间轴。
    let gpx = resolve_hud_track(&settings.hud.gpx_file_path, &video, Some(&rm));

    // Decoder at processing resolution.
    let (pw, ph) = processing_size(info.width, info.height, 1440);
    let mut decoder = SoftDecoder::open(&video, pw, ph).map_err(|e| format!("decoder: {e}"))?;

    // Stabilizer (if enabled + motion data).
    let stab = if settings.gyro.enabled && info.has_motion_data {
        eprintln!("[encode] loading stabilizer (this can take ~90s in debug)...");
        let s = load_stabilizer(
            &video,
            &settings.gyro,
            pw,
            ph,
            fps,
            info.duration_ms,
            info.frame_count,
            true,
            gyro_offset_ms(settings, fps),
            crate::gyro::no_progress(),
        )
        .map_err(|e| format!("stabilizer: {e}"))?;
        Some(std::sync::Arc::new(s))
    } else {
        None
    };

    // Encoder.
    let mut enc = crate::encode::FfmpegEncoder::open(
        out_path,
        &video,
        &[],
        out_w,
        out_h,
        fps,
        info.duration_ms,
        &settings.encode,
    )
    .map_err(|e| format!("encoder: {e}"))?;

    let dt = 1000.0 / fps;
    let cap = if max_frames > 0 {
        max_frames
    } else {
        (info.duration_ms / dt).round() as usize
    };
    let t0 = std::time::Instant::now();
    let mut t_ms = 0.0;
    let mut count = 0usize;
    while count < cap {
        let mut rgba = vec![0u8; pw * ph * 4];
        if decoder.decode_at(t_ms, &mut rgba).is_err() {
            break;
        }
        // Stabilize if available.
        if let Some(stab) = &stab {
            let fi = ((t_ms / 1000.0) * fps).round() as usize;
            let mut out = vec![0u8; pw * ph * 4];
            if stab
                .process_frame(t_ms, fi, &mut rgba, pw, ph, &mut out, pw, ph)
                .is_ok()
            {
                rgba = out;
            }
        }
        let video_img = qtwrap::qimage_from_rgba(pw as i32, ph as i32, &rgba);
        // Composite to the output size.
        let mut img =
            qtwrap::qimage_from_rgba(out_w as i32, out_h as i32, &vec![0u8; out_w * out_h * 4]);
        let mut p = qtwrap::qpainter_on_image(&mut img);
        draw_scene(
            &mut p,
            &rm,
            settings,
            Some(map_img.clone()),
            Some(video_img.clone()),
            out_w as f64,
            out_h as f64,
            t_ms,
            0.0,
            gpx.as_ref(),
        );
        drop(p);
        // Convert to RGB24 and feed.
        let rgba_img = qtwrap::qimage_to_rgba(&img);
        let mut rgb = Vec::with_capacity(out_w * out_h * 3);
        for px in rgba_img.chunks_exact(4) {
            rgb.extend_from_slice(&px[..3]);
        }
        enc.write_frame(&rgb)
            .map_err(|e| format!("write_frame: {e}"))?;
        count += 1;
        t_ms += dt;
        if count % 100 == 0 {
            let avg = t0.elapsed().as_secs_f64() / count as f64;
            eprintln!("[encode] frame {count}/{cap}, {:.2}/frame", avg);
        }
    }
    let out = enc.close().map_err(|e| format!("close: {e}"))?;
    eprintln!(
        "[encode] finished {count} frames in {:.1}s",
        t0.elapsed().as_secs_f64()
    );
    Ok(out)
}

/// Stabilize-only video export (no map/route/HUD overlay): decode each frame, run the
/// Gyroflow stabilizer, and encode the stabilized RGBA to a video. Port of the
/// frame-server path, for comparing the stabilization against the source.
/// Effective gyro<->video time offset in ms: whole-frame offset takes precedence.
pub fn gyro_offset_ms(settings: &Settings, fps: f64) -> f64 {
    if settings.gyro.offset_frames != 0 && fps > 0.0 {
        settings.gyro.offset_frames as f64 * 1000.0 / fps
    } else {
        settings.gyro.offset_ms
    }
}

pub fn stabilize_video(
    settings: &Settings,
    out_path: &str,
    seconds: f64,
    out_w: usize,
    out_h: usize,
    max_frames: usize,
    start_s: f64,
    proc_h: usize,
    autosync: bool,
    _offset_ms: f64,
    dump_quats: Option<&str>,
    dump_frames: Option<&str>,
) -> Result<String, String> {
    let video = settings.video.video_path.clone();
    let info = crate::probe::probe_video(&video).ok_or("probe video failed")?;
    let fps = info.fps;
    // Input processing size: cap the height unless proc_h == 0 (native).
    let (pw, ph) = if proc_h > 0 {
        processing_size(info.width, info.height, proc_h)
    } else {
        (info.width, info.height)
    };

    eprintln!("[stab] loading stabilizer (input {pw}x{ph})...");
    let stab = std::sync::Arc::new(
        load_stabilizer(
            &video,
            &settings.gyro,
            pw,
            ph,
            fps,
            info.duration_ms,
            info.frame_count,
            autosync,
            gyro_offset_ms(settings, fps),
            crate::gyro::no_progress(),
        )
        .map_err(|e| format!("stabilizer: {e}"))?,
    );
    stab.set_output_size(out_w, out_h);

    // Diagnostic: dump the smoothed quaternions and stop (no render/encode).
    if let Some(p) = dump_quats {
        crate::gyro::dump_smoothed_quats(
            &stab,
            start_s * 1000.0,
            start_s * 1000.0 + seconds * 1000.0,
            p,
        )?;
        return Ok(p.to_string());
    }
    let mut enc_settings = settings.encode.clone();
    enc_settings.audio_codec = "none".to_string();
    enc_settings.preset = "veryfast".to_string(); // fast compare export; quality is plenty for judging stabilization
    let mut enc = crate::encode::FfmpegEncoder::open(
        out_path,
        &video,
        &[],
        out_w,
        out_h,
        fps,
        info.duration_ms,
        &enc_settings,
    )
    .map_err(|e| format!("encoder: {e}"))?;

    let mut decoder = SoftDecoder::open(&video, pw, ph).map_err(|e| format!("decoder: {e}"))?;
    // Prefer the hardware d3d11va pipe decoder (fast for HEVC); fall back to software.
    let mut pipe = crate::pipe::PipeDecoder::new(&video, pw, ph, fps);
    let pipe_ok = pipe.spawn_at(start_s * 1000.0);
    let dt = 1000.0 / fps;
    let cap = if max_frames > 0 {
        max_frames
    } else {
        (seconds * fps).round() as usize
    };
    let mut count = 0usize;
    let mut t_ms = start_s * 1000.0;
    let t0 = std::time::Instant::now();
    while count < cap {
        let mut rgba: Vec<u8> = if pipe_ok {
            if let Some(fr) = pipe.get(t_ms) {
                fr.as_ref().clone()
            } else {
                let mut buf = vec![0u8; pw * ph * 4];
                if decoder.decode_at(t_ms, &mut buf).is_err() {
                    break;
                }
                buf
            }
        } else {
            let mut buf = vec![0u8; pw * ph * 4];
            if decoder.decode_at(t_ms, &mut buf).is_err() {
                break;
            }
            buf
        };
        let mut out = vec![0u8; out_w * out_h * 4];
        if stab
            .process_frame(t_ms, count, &mut rgba, pw, ph, &mut out, out_w, out_h)
            .is_err()
        {
            eprintln!("[stab] frame {count} process failed");
            break;
        }
        let mut rgb = Vec::with_capacity(out_w * out_h * 3);
        for px in out.chunks_exact(4) {
            rgb.extend_from_slice(&px[..3]);
        }
        // Diagnostic: dump every 30th stabilized frame as a PNG (pixel-compare vs native).
        if let Some(dir) = dump_frames {
            if count % 30 == 0 {
                let img = qtwrap::qimage_from_rgba(out_w as i32, out_h as i32, &out);
                let path = std::path::Path::new(dir).join(format!("frame_{:.0}ms.png", t_ms));
                let _ = qtwrap::qimage_save(&img, &path.to_string_lossy());
                eprintln!("[stab] dumped frame -> {}", path.display());
            }
        }
        enc.write_frame(&rgb)
            .map_err(|e| format!("write_frame: {e}"))?;
        count += 1;
        t_ms += dt;
        if count % 300 == 0 {
            let avg = t0.elapsed().as_secs_f64() / count as f64;
            eprintln!(
                "[stab] frame {}/{} ({:.1}s elapsed, {:.2}s/frame)",
                count,
                cap,
                t0.elapsed().as_secs_f64(),
                avg
            );
        }
    }
    let out = enc.close().map_err(|e| format!("close: {e}"))?;
    eprintln!(
        "[stab] stabilized {count} frames in {:.1}s",
        t0.elapsed().as_secs_f64()
    );
    Ok(out)
}

/// 「轨迹/地图显示」白名单，写作 `[段] 键` 对。只有这些键变化时才允许走
/// [`TraceReelItem::apply_route_display`] 的就地更新；其余任何一个键变了都必须走
/// 完整 `apply_settings`（视频/地图/轨迹文件、分段时间、GPX、防抖预设、编码、窗口…）。
///
/// 注意两点：`discreetColor`/`highlightColor` 是 `routeColor`/`tailColor` 的**旧版派生键**
/// （见 `Settings::to_ini`），必须一起进白名单，否则调一次颜色就会被判成"非显示项变了"
/// 而退回重载；`frameReadout` 之类会影响防抖/导出的量则**故意不进**白名单。
const ROUTE_DISPLAY_KEYS: &[(&str, &str)] = &[
    ("route", "userScale"),
    ("route", "routeRenderMode"),
    ("route", "tailRenderMode"),
    ("route", "routeRenderScope"),
    ("route", "routeWidth"),
    ("route", "tailWidth"),
    ("route", "tailLength"),
    ("route", "controlRadius"),
    ("route", "runnerRadius"),
    ("route", "showControls"),
    ("route", "showRunner"),
    ("route", "routeColor"),
    ("route", "routeOpacity"),
    ("route", "tailColor"),
    ("route", "tailOpacity"),
    ("route", "controlColor"),
    ("route", "controlOpacity"),
    ("route", "runnerColor"),
    ("route", "runnerOpacity"),
    ("route", "discreetColor"),
    ("route", "highlightColor"),
    ("route", "runnerStyle"),
    ("route", "lowPace"),
    ("route", "highPace"),
    ("route", "paceAuto"),
    ("route", "tailLowPace"),
    ("route", "tailHighPace"),
    ("route", "tailPaceAuto"),
    ("route", "viewMode"),
    ("route", "runnerAveragingFactor"),
    ("route", "controlTimeOffset"),
    ("route", "leftRightMargin"),
    ("route", "topBottomMargin"),
    ("map", "relativeWidth"),
    ("map", "x"),
    ("map", "y"),
    ("map", "userX"),
    ("map", "userY"),
    ("map", "angle"),
    ("map", "userAngle"),
    ("map", "userScale"),
    // HUD 的显示范围只决定 HUD 落在哪块区域（视频区/整幅画面），不碰任何素材，
    // 就地重画就够，不必为下拉一下重载视频。
    ("hud", "scope"),
];

/// 比较两份设置的规范化 `.orv` 文本（`IniFile` 用 `BTreeMap`，顺序稳定），忽略
/// [`ROUTE_DISPLAY_KEYS`]。为真 = "只有轨迹/地图显示项变了"。
///
/// 保守失败：只要有任何**没进白名单**的差异（也包括解析不出键名的行），一律返回假，
/// 调用方退回完整重载 —— 宁可慢一次，也不能悄悄吞掉一个真正影响预览的设置。
fn only_route_display_changed(old: &Settings, new: &Settings) -> bool {
    fn non_display_lines(s: &Settings) -> Vec<String> {
        let mut section = String::new();
        let mut keep: Vec<String> = Vec::new();
        for line in s.to_ini().lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                section = trimmed.trim_start_matches('[').trim_end_matches(']').to_string();
                continue;
            }
            let key = match trimmed.split_once('=') {
                Some((k, _)) => k.trim(),
                None => {
                    keep.push(line.to_string());
                    continue;
                }
            };
            if ROUTE_DISPLAY_KEYS.contains(&(section.as_str(), key)) {
                continue;
            }
            keep.push(line.to_string());
        }
        keep
    }
    non_display_lines(old) == non_display_lines(new)
}

/// 设置里的「视野模式」→ 取景模式。**唯一**的映射入口：预览（`compute_apply`）、导出
/// （`EncodePrep::build`）、离屏渲染（`render_composite_to_png` / `encode_composite`）都必须
/// 走这里。
///
/// 曾经预览这条路径硬编码 `FixedSplit`，于是「跟跑者居中」只在导出时生效：不填分段时间时
/// `split_transformations` 为空表，`current_st` 一直停在 (0,0,0,1) → 地图/轨迹一动不动
/// （原版此时是跟着跑者走的）。三处各写一份 match 正是漂移的温床，故合并成一个函数。
pub fn view_mode_from_settings(s: &Settings) -> ViewMode {
    match s.route.view_mode {
        1 => ViewMode::RunnerCentered,
        2 => ViewMode::RunnerCenteredSplitOriented,
        _ => ViewMode::FixedSplit,
    }
}

pub fn route_style_from_settings(s: &Settings) -> RouteStyle {
    let mut st = RouteStyle::default();
    st.user_scale = s.route.user_scale;
    // 统一时间校准模型（正值约定，与界面提示一致）：
    //   T_watch = 「按表-出发时间」= 出发时刻在轨迹时间轴上的秒数（轨迹开始几秒后出发 → +几秒）
    //   T_video = 「录像-出发时间」= 出发时刻在视频时间轴上的秒数（录像开始几十秒后出发 → +几十秒）
    //   分段时间的 0 点 = 出发时刻。
    //   - 控制点(分段 s)：路线时刻 = s + T_watch
    //   - 运动员(视频时刻 t)：路线时刻 = T_watch + (t - T_video)
    //       => runner_time_offset = T_watch - T_video（自动换算，无需单独输入）
    st.control_time_offset = s.route.control_time_offset;
    st.runner_time_offset = s.route.control_time_offset - s.video.start_time_offset_s;
    st.tail_length = s.route.tail_length;
    st.show_controls = s.route.show_controls;
    st.show_runner = s.route.show_runner;
    st.route_render_mode = match s.route.render_mode {
        1 => RenderMode::Discreet,
        // 3 = 配速（沿用原版编号）。2 是原版的「高亮」——本程序没有这个模式，
        // 老工程里存到 2 的一律按单色的「简洁」渲染。
        2 => RenderMode::Discreet,
        3 => RenderMode::Pace,
        _ => RenderMode::None,
    };
    st.tail_render_mode = match s.route.tail_render_mode {
        // 1 = 简洁；2 是原版尾巴的「高亮」——本程序没有这个模式，按单色的「简洁」渲染。
        1 | 2 => RenderMode::Discreet,
        // 3 = 配速（与路线渲染共用同一套编号与色带）。
        3 => RenderMode::Pace,
        _ => RenderMode::None,
    };
    st.route_width = s.route.width;
    st.tail_width = s.route.tail_width;
    st.control_radius = s.route.control_radius;
    st.runner_radius = s.route.runner_radius;
    // 运动员样式：0 = 圆形（原版）、1 = 箭头（跟着运动方向转）。
    st.runner_style = match s.route.runner_style {
        1 => RunnerStyle::Arrow,
        _ => RunnerStyle::Circle,
    };
    // 颜色与不透明度是两个独立设置项：颜色只给 RGB，alpha 一律取那个 0..255 的项
    // （原版 Renderer.cpp 里颜色自带 alpha，这里拆开是为了让界面能单独调透明度）。
    st.route_color = color_with_opacity(
        &s.route.route_color,
        s.route.route_opacity,
        (0.0, 0.0, 0.0, 80.0 / 255.0),
    );
    st.tail_color = color_with_opacity(
        &s.route.tail_color,
        s.route.tail_opacity,
        (0.0, 0.0, 0.0, 80.0 / 255.0),
    );
    st.control_border_color = color_with_opacity(
        &s.route.control_color,
        s.route.control_opacity,
        (0.0, 0.0, 0.0, 1.0),
    );
    st.runner_color = color_with_opacity(
        &s.route.runner_color,
        s.route.runner_opacity,
        (1.0, 1.0, 1.0, 1.0),
    );
    st.low_pace = s.route.low_pace;
    st.tail_low_pace = s.route.tail_low_pace;
    st.high_pace = s.route.high_pace;
    st.tail_high_pace = s.route.tail_high_pace;
    st.pace_auto = s.route.pace_auto;
    st.tail_pace_auto = s.route.tail_pace_auto;
    st
}

/// 把一串**带配速的点**画成配速色带：先按全不透明画进一张离屏位图，再整张按 `alpha` 合成一次。
///
/// 整条路径一次描边得到覆盖率，整张按设定不透明度合成一次，避免逐段半透明绘制造成
/// 接头叠深。圆头仅用于整条路线的两端，内部使用圆角接头。
///
/// 参考 QuickRoute Session.RouteLine.CreatePath：内部接点拼成连续色带，不能把每段的
/// 圆头覆盖到邻段上。先一次描边得到抗锯齿覆盖率，再由像素在路线上的投影插值配速并取色。
/// 这样短段不会被圆头刷成色块，跨过黄色节点的长段也不会变成绿红混合的暗黄色。
///
/// 同一个函数既画整条路线（`cache_kind` = 0）也画尾巴（`cache_kind` = 1）：位图内容只取决于
/// （点位、配速、线宽、配速范围、分辨率），与时间无关，播放/编码时每帧直接复用。
fn draw_pace_band(
    p: &mut QPainter,
    points: &[&tracereel_core::route::RoutePoint],
    width: f64,
    alpha: f64,
    low: f64,
    high: f64,
    cache_kind: u64,
) {
    p.set_brush(QBrush::from_style(BrushStyle::NoBrush));
    if points.len() < 2 {
        return;
    }
    // 位图范围 = 轨迹包围盒 + 半个线宽（圆头端向外鼓出的量）+ 1 个路线单位余量。
    let margin = width * 0.5 + 1.0;
    let (mut min_x, mut min_y) = (f64::INFINITY, f64::INFINITY);
    let (mut max_x, mut max_y) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for rp in points.iter() {
        min_x = min_x.min(rp.position_x);
        min_y = min_y.min(rp.position_y);
        max_x = max_x.max(rp.position_x);
        max_y = max_y.max(rp.position_y);
    }
    min_x -= margin;
    min_y -= margin;
    max_x += margin;
    max_y += margin;
    let bw = (max_x - min_x).max(1e-9);
    let bh = (max_y - min_y).max(1e-9);
    // 位图分辨率：默认跟父画家 1:1（1 个位图像素 = 1 个设备像素，线宽、边缘与直接画在画布上
    // 完全一致，合成就是一次 1:1 拷贝）；只在这三种情况下让步 —— 长边超过 1600 像素（大范围
    // 轨迹，位图太大不值当）、线宽不足 1 个位图像素（会淡成发丝）、总面积超过 400 万像素。
    let mut ls = qtwrap::painter_scale(p).max(1e-6);
    let long_side = bw.max(bh);
    if long_side * ls > 1600.0 {
        ls = 1600.0 / long_side;
    }
    ls = ls.max(1.0 / width.max(1e-9));
    let area = bw * ls * bh * ls;
    if area > 4.0e6 {
        ls *= (4.0e6 / area).sqrt();
    }
    let iw = (bw * ls).ceil().clamp(1.0, 4096.0) as i32;
    let ih = (bh * ls).ceil().clamp(1.0, 4096.0) as i32;

    let key = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        cache_kind.hash(&mut h);
        points.len().hash(&mut h);
        for rp in points.iter() {
            rp.position_x.to_bits().hash(&mut h);
            rp.position_y.to_bits().hash(&mut h);
            rp.pace.to_bits().hash(&mut h);
        }
        width.to_bits().hash(&mut h);
        low.to_bits().hash(&mut h);
        high.to_bits().hash(&mut h);
        ls.to_bits().hash(&mut h);
        h.finish()
    };
    // 每个 cache_kind 各留一份：路线一份、尾巴一份。只留一槽的话，播放中每帧都在变的
    // 尾巴位图会把路线那份挤掉，而路线带子可能有几千个点，重新生成它的代价正好落在每帧上。
    // 也**不能**按插入顺序整体淘汰（那样路线会被每帧新来的尾巴挤走、隔帧重建）：按 kind 淘汰，
    // 同一 kind 只保留最新的一份，于是路线在它的点位/配速/线宽不变时一直命中。
    thread_local! {
        static PACE_LAYER_CACHE: std::cell::RefCell<Vec<(u64, u64, QImage)>> =
            std::cell::RefCell::new(Vec::new());
    }
    let cached = PACE_LAYER_CACHE.with(|c| {
        c.borrow()
            .iter()
            .find(|(_, k, _)| *k == key)
            .map(|(_, _, img)| qtwrap::qimage_clone(img))
    });
    let layer = match cached {
        Some(img) => img,
        None => {
            let mut layer = qtwrap::qimage_new_transparent(iw, ih);
            {
                let mut handle = qtwrap::qpainter_on_image(&mut layer);
                let lp: &mut QPainter = &mut handle;
                lp.set_render_hint(
                    qmetaobject::qttypes::QPainterRenderHint::Antialiasing,
                    true,
                );
                lp.scale(ls, ls);
                lp.translate(QPointF { x: -min_x, y: -min_y });
                lp.set_brush(QBrush::from_style(BrushStyle::NoBrush));
                let mut pen = QPen::from_color(QColor::from_rgba(255, 255, 255, 255));
                pen.set_width_f(width);
                qtwrap::pen_set_cap_join(&mut pen, qtwrap::PEN_CAP_ROUND, qtwrap::PEN_JOIN_ROUND);
                lp.set_pen(pen);
                let path: Vec<QPointF> = points.iter().map(|rp| QPointF {
                    x: rp.position_x, y: rp.position_y,
                }).collect();
                lp.draw_polyline(&path);
            }
            let mut pixels = qtwrap::qimage_to_rgba(&layer);
            let mut distances = vec![f64::INFINITY; iw as usize * ih as usize];
            // Only visit each segment's stroke bounds, not every pixel for every GPS point.
            // Closest projection partitions adjacent strokes at their angle bisector, like
            // QuickRoute's joined ribbon. At crossings, equal distances prefer the later pass.
            let radius = width * ls * 0.5 + 2.0;
            for seg in points.windows(2) {
                let (a, b) = (seg[0], seg[1]);
                let (ax, ay) = ((a.position_x - min_x) * ls, (a.position_y - min_y) * ls);
                let (bx, by) = ((b.position_x - min_x) * ls, (b.position_y - min_y) * ls);
                let (dx, dy) = (bx - ax, by - ay);
                let length_sq = dx * dx + dy * dy;
                if length_sq <= 1e-12 { continue; }
                let x0 = (ax.min(bx) - radius).floor().max(0.0) as i32;
                let x1 = (ax.max(bx) + radius).ceil().min(iw as f64 - 1.0) as i32;
                let y0 = (ay.min(by) - radius).floor().max(0.0) as i32;
                let y1 = (ay.max(by) + radius).ceil().min(ih as f64 - 1.0) as i32;
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        let index = y as usize * iw as usize + x as usize;
                        if pixels[index * 4 + 3] == 0 { continue; }
                        let (px, py) = (x as f64 + 0.5 - ax, y as f64 + 0.5 - ay);
                        let t = ((px * dx + py * dy) / length_sq).clamp(0.0, 1.0);
                        let distance = (px - t * dx).powi(2) + (py - t * dy).powi(2);
                        if distance > distances[index] { continue; }
                        distances[index] = distance;
                        let pace = a.pace + (b.pace - a.pace) * t;
                        let (r, g, blue) = tracereel_core::math::interpolate_from_green_to_red(high, low, pace);
                        for (channel, value) in [r, g, blue].into_iter().enumerate() {
                            pixels[index * 4 + channel] = (value.clamp(0.0, 1.0) * 255.0).round() as u8;
                        }
                    }
                }
            }
            layer = qtwrap::qimage_from_rgba(iw, ih, &pixels);
            PACE_LAYER_CACHE.with(|c| {
                let mut b = c.borrow_mut();
                // 同一 kind 的旧位图直接替换掉；另一个 kind（路线 / 尾巴）留着。
                b.retain(|(kind, k, _)| *kind != cache_kind && *k != key);
                b.push((cache_kind, key, qtwrap::qimage_clone(&layer)));
            });
            layer
        }
    };
    // 整张位图按设定不透明度合成一次：位图内部已经完全连成一片，所以这里不会有任何叠加加深，
    // 整条带子的不透明度处处等于设定值。
    p.set_opacity(alpha);
    p.draw_image_fit_rect(
        QRectF {
            x: min_x,
            y: min_y,
            width: qtwrap::qimage_width(&layer) as f64 / ls,
            height: qtwrap::qimage_height(&layer) as f64 / ls,
        },
        layer,
    );
    p.set_opacity(1.0);
}

/// 颜色（`#rrggbb` / `#rrggbbaa`）+ 独立的不透明度（0..255）→ RGBA 0..1。
/// 颜色串里即使带了 alpha 也忽略：不透明度由界面上的那一项单独控制，免得两个真值源。
fn color_with_opacity(hex: &str, opacity: f64, def: (f64, f64, f64, f64)) -> (f64, f64, f64, f64) {
    let (r, g, b, _) = hex_to_rgba(hex, def);
    let a = if opacity.is_finite() {
        (opacity / 255.0).clamp(0.0, 1.0)
    } else {
        def.3
    };
    (r, g, b, a)
}

/// `#rrggbb` / `#rrggbbaa`（本程序约定：**尾部**两位是不透明度）→ RGBA 0..1 四元组。
fn hex_to_rgba(hex: &str, def: (f64, f64, f64, f64)) -> (f64, f64, f64, f64) {
    let t = hex.trim_start_matches('#');
    let byte = |i: usize| u8::from_str_radix(&t[i..i + 2], 16).map(|v| v as f64 / 255.0).ok();
    if t.len() == 6 {
        if let (Some(r), Some(g), Some(b)) = (byte(0), byte(2), byte(4)) {
            return (r, g, b, 1.0);
        }
    } else if t.len() == 8 {
        if let (Some(r), Some(g), Some(b), Some(a)) = (byte(0), byte(2), byte(4), byte(6)) {
            return (r, g, b, a);
        }
    }
    def
}

/// RGBA 0..1 → QColor（含不透明度）。
fn rgba_qcolor(c: (f64, f64, f64, f64)) -> QColor {
    let ch = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as i32;
    QColor::from_rgba(ch(c.0), ch(c.1), ch(c.2), ch(c.3))
}

fn draw_route(p: &mut QPainter, rm: &RouteManager, s: &Settings, t_sec: f64, route_start_ms: f64) {
    draw_route_pub(p, rm, s, t_sec, route_start_ms);
}

/// Public route painter (used by the encode pump and headless harnesses).
/// 当前所在的那一段（控制点 i → 控制点 i+1）在**路线点时间轴**上的时间窗 `[起, 止]`。
///
/// 判定式与取景用的 `find_index`（tracereel-core/src/route_manager.rs:870）完全一致：
/// 路线点时间 = 全局时间 + `runner_time_offset`，分段 `i` 覆盖
/// `splits[i].absolute_time + control_time_offset` 到 `splits[i+1].absolute_time + control_time_offset`
/// （左闭右开）。这样「只画当前段」与「画面框住当前段」永远是同一段。
///
/// 出发前（早于第一段）按第一段画、冲线之后（晚于最后一段）按最后一段画 —— 取景在这些时刻
/// 用的也是同一段，否则会出现「画面已经框住这一段、路线却一片空白」。
/// 没有分段（`split_times` 为空或只有一个控制点）时返回 `None`：整条路线就是唯一的一段。
fn current_leg_window(rm: &RouteManager, t_sec: f64) -> Option<(f64, f64)> {
    let route = rm.routes.first()?;
    let splits = &rm.runner_info.splits;
    if splits.len() < 2 {
        return None;
    }
    let cto = route.control_time_offset;
    let runner_t = t_sec + route.runner_time_offset;
    let last_leg = splits.len() - 2;
    let lo_at = |i: usize| splits[i].absolute_time + cto;
    let hi_at = |i: usize| splits[i + 1].absolute_time + cto;
    for i in 0..=last_leg {
        if hi_at(i) > lo_at(i) && lo_at(i) <= runner_t && runner_t < hi_at(i) {
            return Some((lo_at(i), hi_at(i)));
        }
    }
    // 落在所有分段之外：出发前用第一段、冲线后用最后一段（与取景的兜底一致）。
    if runner_t < lo_at(0) {
        Some((lo_at(0), hi_at(0)))
    } else {
        Some((lo_at(last_leg), hi_at(last_leg)))
    }
}

/// `route_start_ms` is the global time at which this map becomes active; the full
/// polyline is clipped to points at or after that moment (converted into route time
/// through `runner_time_offset`: a route point shown at global `t` satisfies
/// `point.time == t + runner_time_offset`). 0.0 keeps the historical whole-track draw.
pub fn draw_route_pub(
    p: &mut QPainter,
    rm: &RouteManager,
    s: &Settings,
    t_sec: f64,
    route_start_ms: f64,
) {
    let Some(route) = rm.routes.first() else {
        return;
    };
    if route.route_points.is_empty() {
        return;
    }
    let scale_factor = s.route.user_scale;
    let style = &route.style;

    // 路线的起点限制：`route_start_ms` 之后的路段才画（多地图分段用；0 表示整条）。
    let start_route_t = route_start_ms / 1000.0 + route.runner_time_offset;
    // 渲染范围：完整 = 整条路线（默认）；单段 = 只画当前所在的那一段，其余各段一个像素都不画。
    let leg = if s.route.render_scope == 1 {
        current_leg_window(rm, t_sec)
    } else {
        None
    };
    let visible_points: Vec<&tracereel_core::route::RoutePoint> = route
        .route_points
        .iter()
        .filter(|rp| {
            rp.time >= start_route_t && leg.map_or(true, |(lo, hi)| lo <= rp.time && rp.time <= hi)
        })
        .collect();

    // 整条路线（对齐原版 Renderer.cpp:505-587）：
    //   简洁 = 一条单色路径一次画完（RoundJoin+RoundCap，色 = 路线颜色 + 路线透明度）；
    //   配速 = 在连续色带上插值配速取色，绿(快)→黄→红(慢)渐变；
    //          不透明度同样取路线透明度。
    match style.route_render_mode {
        RenderMode::None => {}
        RenderMode::Pace => {
            draw_pace_band(
                p,
                &visible_points,
                style.route_width * scale_factor,
                style.route_color.3,
                style.low_pace,
                style.high_pace,
                0,
            );
        }
        RenderMode::Discreet => {
            let width = style.route_width * scale_factor;
            let mut pen = QPen::from_color(rgba_qcolor(style.route_color));
            pen.set_width_f(width);
            pen.set_style(PenStyle::SolidLine);
            qtwrap::pen_set_cap_join(&mut pen, qtwrap::PEN_CAP_ROUND, qtwrap::PEN_JOIN_ROUND);
            p.set_pen(pen);
            p.set_brush(QBrush::from_style(BrushStyle::NoBrush));
            let pts: Vec<QPointF> = visible_points
                .iter()
                .map(|rp| QPointF {
                    x: rp.position_x,
                    y: rp.position_y,
                })
                .collect();
            if pts.len() >= 2 {
                p.draw_polyline(&pts);
            }
        }
    }

    // Tail (recent portion behind the runner). 原版尾巴只有 简洁/高亮 两个分支、没有配速，
    // 颜色分别取 discreetColor / highlightColor。本程序把尾巴也做成「无 / 简洁 / 配速」：
    //   无   = 不画（尾巴长度自然也就不起作用）；
    //   简洁 = 一条单色路径，颜色 = 尾巴颜色 + 尾巴透明度；
    //   配速 = 和路线配速完全一样的分段渐变色带，只是用尾巴的线宽与透明度。
    if style.tail_length > 0.0 {
        match style.tail_render_mode {
            RenderMode::None => {}
            RenderMode::Discreet => {
                let tail = rm.calculate_tail_path(0, t_sec);
                if tail.len() >= 2 {
                    let tw = style.tail_width * scale_factor;
                    let mut pen = QPen::from_color(rgba_qcolor(style.tail_color));
                    pen.set_width_f(tw);
                    pen.set_style(PenStyle::SolidLine);
                    qtwrap::pen_set_cap_join(
                        &mut pen,
                        qtwrap::PEN_CAP_ROUND,
                        qtwrap::PEN_JOIN_ROUND,
                    );
                    p.set_pen(pen);
                    p.set_brush(QBrush::from_style(BrushStyle::NoBrush));
                    let pts: Vec<QPointF> =
                        tail.iter().map(|(x, y)| QPointF { x: *x, y: *y }).collect();
                    p.draw_polyline(&pts);
                }
            }
            RenderMode::Pace => {
                let tail = rm.calculate_tail_points(0, t_sec);
                let refs: Vec<&tracereel_core::route::RoutePoint> = tail.iter().collect();
                draw_pace_band(
                    p,
                    &refs,
                    style.tail_width * scale_factor,
                    style.tail_color.3,
                    style.tail_low_pace,
                    style.tail_high_pace,
                    1,
                );
            }
        }
    }

    // Control points.
    if style.show_controls {
        let cw = style.control_border_width * scale_factor;
        let crad = style.control_radius * scale_factor;
        let mut cpen = QPen::from_color(rgba_qcolor(style.control_border_color));
        cpen.set_width_f(cw);
        p.set_pen(cpen);
        p.set_brush(QBrush::from_style(BrushStyle::NoBrush));
        for (cx, cy) in &route.control_positions {
            p.draw_ellipse(QRectF {
                x: cx - crad,
                y: cy - crad,
                width: crad * 2.0,
                height: crad * 2.0,
            });
        }
    }

    // Runner.
    if style.show_runner {
        let rrad = style.runner_radius * style.runner_scale * scale_factor;
        let (rx, ry) = route.runner_position;
        let c = rgba_qcolor(style.runner_color);
        match style.runner_style {
            RunnerStyle::Circle => {
                p.set_pen(QPen::from_color(c.clone()));
                p.set_brush(QBrush::from_color(c));
                p.draw_ellipse(QRectF {
                    x: rx - rrad,
                    y: ry - rrad,
                    width: rrad * 2.0,
                    height: rrad * 2.0,
                });
            }
            RunnerStyle::Arrow => {
                // 箭头顺着运动方向。顶点直接按方向向量在**路线坐标系**里算，不用
                // `painter.rotate`：顶点法对任何仿射变换（分段取景里的旋转/镜像/缩放）
                // 都是协变的，所以箭头永远和画出来的路线走向一致。
                let (dx, dy) = rm.runner_direction(0, t_sec, 0.5);
                let (px, py) = (-dy, dx); // 与前进方向垂直
                let at = |fwd: f64, side: f64| QPointF {
                    x: rx + (dx * fwd + px * side) * rrad,
                    y: ry + (dy * fwd + py * side) * rrad,
                };
                // 正三角形：箭尖在正前方 r 处，底边两角在后方 0.6r、左右各 0.8r。
                let tip = at(1.0, 0.0);
                let left = at(-0.6, 0.8);
                let right = at(-0.6, -0.8);
                // 只填色：`QPen::from_color` 的线宽是 1 个**路线单位**，在地图缩放
                // （`scale_factor`）之后会把箭头描成一团糊（实测 0.4 半径的圆被描成 26px）。
                // 线宽 0 在 Qt 里是「cosmetic」= 1 个设备像素。
                let mut pen = QPen::from_color(c.clone());
                pen.set_width_f(0.0);
                p.set_pen(pen);
                p.set_brush(QBrush::from_color(c));
                p.draw_polygon(&[tip, left, right]);
            }
        }
    }
}

/// Format seconds as `h:mm:ss` (or `m:ss` under an hour), the syntax the split
/// list and its tooltip document.
pub(crate) fn fmt_split_seconds(secs: f64) -> String {
    let total = secs.max(0.0).round() as i64;
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// True when a path points at a QuickRoute project file (`.qrt`).
///
/// A `.qrt` is a whole project: the map image is embedded inside it (as
/// `Map.rawData`) together with the track, so the 轨迹 page can take a single
/// `.qrt` instead of a pre-exported QuickRoute JPEG.
pub fn is_qrt_path(path: &str) -> bool {
    let p = path.trim();
    !p.is_empty() && p.to_ascii_lowercase().ends_with(".qrt")
}

/// 把「取景」类设置同步进取景器：左右/上下边距决定分段取景表（`calc_split_transformations`），
/// 必须在建路线（`init_route_manager`）**之前**写好，否则这一帧用的还是取景器自带默认边距。
///
/// 之前预览准备和离屏渲染（导出、单帧截图）都只建了取景器、没同步这几项：界面上改边距
/// 只有当场生效一次（走「就地更新」那条路），重新打开工程或导出时又回到代码默认的
/// 10/30，和工程里存的值对不上。跑者平均因子按 `RUNNER_AVERAGING_FACTOR` 固定。
fn apply_framing_settings(rm: &mut RouteManager, settings: &Settings) {
    rm.left_right_margin = settings.route.left_right_margin;
    rm.top_bottom_margin = settings.route.top_bottom_margin;
    rm.set_runner_averaging_factor(tracereel_core::route_manager::RUNNER_AVERAGING_FACTOR);
}

/// Point a `RouteManager` at a track source, accepting either a QuickRoute JPEG
/// export or a `.qrt` project.
///
/// Both the live GUI prepare and the headless composite render go through here,
/// so `.qrt` support cannot drift between the two paths.
#[allow(clippy::too_many_arguments)]
pub(crate) fn init_route_manager(
    rm: &mut RouteManager,
    route_path: &str,
    map_width: f64,
    map_height: f64,
    splits: &str,
    rel: bool,
    route_style: RouteStyle,
    view_mode: ViewMode,
    map_relative_width: f64,
) -> Result<(), String> {
    if is_qrt_path(route_path) {
        let proj = tracereel_core::qrt::read_project(route_path).map_err(|e| e.to_string())?;
        if proj.waypoints.is_empty() {
            return Err("qrt 内未找到轨迹点".to_string());
        }
        rm.initialize_from_points(
            proj.to_route_points(),
            splits,
            rel,
            route_style,
            view_mode,
            map_relative_width,
        );
    } else {
        rm.initialize(
            route_path,
            map_width,
            map_height,
            splits,
            rel,
            route_style,
            view_mode,
            map_relative_width,
        )?;
    }
    // 自适应配速色带放在这里（唯一的建路线入口）：预览 apply、导出、单帧截图都经过它。
    apply_adaptive_pace(rm);
    Ok(())
}

/// HUD 轨迹的解析：工程里指定的 GPX → 视频同名 `.gpx` → **退回轨迹文件自带的时间轴**。
///
/// 第三档服务「只导入了 .qrt / 轨迹图」的工程：QuickRoute 工程里本来就存着整条轨迹和
/// 逐秒时间，没有它 HUD 上的配速/距离/航向一律显示「--」，看起来像程序没算出来。
/// 两条来源的时间轴同一套约定（轨迹第一个样本 = 0 秒），采样偏移统一由
/// [`crate::hud::hud_track_offset`] 按「出发时刻」换算。海拔/心率仍然只有 GPX 才有。
pub fn resolve_hud_track(
    gpx_path: &str,
    video_path: &str,
    rm: Option<&RouteManager>,
) -> Option<GpxTrack> {
    let path = if gpx_path.trim().is_empty() {
        tracereel_core::gpx::find_gpx_for_video(video_path)
    } else {
        gpx_path.to_string()
    };
    if !path.is_empty() {
        match tracereel_core::gpx::parse_gpx(&path) {
            Ok(track) => return Some(track),
            // 指了 GPX 却读不出来：退回轨迹自带的时间轴，同时把原因打出来，别悄悄空着。
            Err(e) => eprintln!("[hud] GPX 读取失败（改用轨迹文件自带的时间轴）：{path}：{e}"),
        }
    }
    let route = rm?.routes.first()?;
    GpxTrack::from_route_points(&route.route_points)
}

/// 路线和尾巴分别按各自的自适应开关更新色带端点（慢端 P90 / 快端 P10）。
/// 轨迹没有逐点配速（「地图 + 线路」工程）时保持原设定不变。
pub(crate) fn apply_adaptive_pace(rm: &mut RouteManager) {
    for route in rm.routes.iter_mut() {
        if !route.style.pace_auto && !route.style.tail_pace_auto {
            continue;
        }
        if let Some((low, high)) = tracereel_core::route_manager::adaptive_pace_range(&route.route_points) {
            if route.style.pace_auto {
                route.style.low_pace = low;
                route.style.high_pace = high;
            }
            if route.style.tail_pace_auto {
                route.style.tail_low_pace = low;
                route.style.tail_high_pace = high;
            }
        }
    }
}

/// Decode a map source that is either an image file or a `.qrt` project.
///
/// Returns the RGBA image together with whether it still carries the QuickRoute
/// header band.  A QuickRoute *JPEG export* has a ~65px metadata strip at the
/// top, but a `.qrt` stores the **raw map image** (`Map.rawData`) with no such
/// strip — cropping it there would shave real map content off the top.
fn decode_map_source(path: &str) -> Result<(image::RgbaImage, bool), String> {
    if is_qrt_path(path) {
        let data = std::fs::read(path).map_err(|e| format!("read qrt: {e}"))?;
        let img = tracereel_core::qrt::find_map_image(&data)
            .ok_or_else(|| "qrt 内未找到内嵌地图图片".to_string())?;
        let decoded =
            image::load_from_memory(&img.bytes).map_err(|e| format!("decode qrt map: {e}"))?;
        Ok((decoded.to_rgba8(), false))
    } else {
        let decoded = image::open(path).map_err(|e| format!("open map: {e}"))?;
        Ok((decoded.to_rgba8(), true))
    }
}

/// QuickRoute writes an N-pixel header band into its exported JPEGs; the `.qrt`
/// embedded image does not have it.
const QUICKROUTE_HEADER_CROP: usize = 65;

/// Optionally crop the QuickRoute header band and return the rows as RGBA8.
fn crop_qrt_header(img: &image::RgbaImage, has_header: bool) -> (Vec<u8>, i32, i32) {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let crop = if has_header {
        QUICKROUTE_HEADER_CROP.min(h)
    } else {
        0
    };
    let new_h = h.saturating_sub(crop).max(40);
    let mut rgba = Vec::with_capacity(w * new_h * 4);
    for y in crop..h {
        let row = y * w * 4;
        rgba.extend_from_slice(&img.as_raw()[row..row + w * 4]);
    }
    (rgba, w as i32, new_h as i32)
}

pub fn load_map_image(path: &str) -> Result<QImage, String> {
    let (img, has_header) = decode_map_source(path)?;
    let (rgba, w, h) = crop_qrt_header(&img, has_header);
    Ok(qtwrap::qimage_from_rgba(w, h, &rgba))
}

/// Decode the map image to raw RGBA8 (with the QuickRoute header crop when the
/// source has one), so the background prepare can transfer plain bytes (Send)
/// instead of a QImage.
pub fn load_map_rgba(path: &str) -> Result<(Vec<u8>, i32, i32), String> {
    let (img, has_header) = decode_map_source(path)?;
    Ok(crop_qrt_header(&img, has_header))
}

/// One prepared map (base bitmap + its route manager) on the global timeline.
/// `start_ms` is when this map becomes active (0 for the main map).
pub(crate) struct PreparedMap {
    pub rgba: Vec<u8>,
    pub w: i32,
    pub h: i32,
    pub start_ms: f64,
    pub rm: RouteManager,
}

/// Result of the background prepare: everything the GUI thread needs to land the apply,
/// containing only Send data (raw RGBA bytes instead of QImage).
pub(crate) struct PendingApply {
    settings: Settings,
    info: VideoInfo,
    /// Segments (multi-video stitch) laid out end-to-end on the global timeline.
    segs: Vec<VideoSeg>,
    /// All maps (main + extras) ordered by `start_ms`; the first is the main map.
    pub(crate) maps: Vec<PreparedMap>,
    gpx: Option<GpxTrack>,
    processing: (usize, usize),
    /// 「配速色带自适应」算出来的端点（慢端, 快端）；关闭自适应或轨迹没有配速时为 None。
    pace_auto: Option<(f64, f64)>,
    error: Option<String>,
}

/// Prepare everything heavy (probe, map decode, route init, gpx) off the GUI thread.
/// Missing pieces are NOT fatal: the preview simply renders what is available (video
/// without map, map without route, etc.), matching the original's continue-anyway
/// behaviour — so selecting a single file in a fresh session still shows something.
fn compute_apply(path: &str) -> PendingApply {
    let mut settings = Settings::load_orv(path);
    let video = settings.video.video_path.clone();

    // Probe EVERY selected file (main +续接分段) and lay the segments end-to-end on a
    // global timeline. Parameters (size/fps) are identical across segments by design,
    // so the merged VideoInfo takes them from the first successful probe and sums the
    // per-segment durations / frame counts.
    let mut segs: Vec<VideoSeg> = Vec::new();
    let mut base: Option<VideoInfo> = None;
    for p in settings.video.all_paths() {
        match crate::probe::probe_video(&p) {
            Some(i) => {
                if base.is_none() {
                    base = Some(i.clone());
                }
                segs.push(VideoSeg {
                    path: p,
                    start_ms: segs.last().map(|s| s.end_ms()).unwrap_or(0.0),
                    duration_ms: i.duration_ms,
                    fps: i.fps,
                    frame_count: i.frame_count,
                    has_motion: i.has_motion_data,
                });
            }
            None => eprintln!("[apply] probe video failed: {p}"),
        }
    }
    let info: VideoInfo = match base {
        Some(mut i) => {
            i.duration_ms = segs.iter().map(|s| s.duration_ms).sum();
            i.frame_count = segs.iter().map(|s| s.frame_count).sum();
            i.has_motion_data = segs.iter().any(|s| s.has_motion);
            i
        }
        None => {
            if !video.is_empty() {
                eprintln!("[apply] probe video failed: {video}");
            } else {
                eprintln!("[apply] probe video skipped (no video selected)");
            }
            VideoInfo {
                width: 0,
                height: 0,
                fps: 0.0,
                duration_ms: 0.0,
                has_motion_data: false,
                frame_count: 0,
            }
        }
    };

    let route_style = route_style_from_settings(&settings);
    let (splits, rel) = (
        settings.splits.split_times.clone(),
        settings.splits.is_relative,
    );

    let jobs = map_jobs(&settings);
    // 「视野模式」必须在这里生效：预览用的是这批 RouteManager，导出用的是 encode.rs 里
    // 另建的一份，两边必须按同一个设置取景。
    let view_mode = view_mode_from_settings(&settings);
    let mut maps: Vec<PreparedMap> = Vec::with_capacity(jobs.len());
    for (map_path, route_path, start_ms) in jobs {
        let (m, err) = prepare_one_map(
            &map_path,
            &route_path,
            start_ms,
            &settings,
            &splits,
            rel,
            route_style.clone(),
            (settings.window.2 as f64, settings.window.3 as f64),
            view_mode,
        );
        if let Some(e) = err {
            eprintln!("[apply] {e}");
        }
        maps.push(m);
    }

    // 配速色带自适应：`init_route_manager` 已经按实际轨迹的配速分布把端点写进每条
    // route 的 style，这里再同步回 settings —— .orv 里存的端点、界面上的滑杆、
    // 导出时另建的那份 RouteManager 都以它为准（导出走同一个 init_route_manager）。
    let mut pace_auto = None;
    if settings.route.pace_auto || settings.route.tail_pace_auto {
        if let Some((low, high)) = maps
            .first()
            .and_then(|m| m.rm.routes.first())
            .and_then(|r| tracereel_core::route_manager::adaptive_pace_range(&r.route_points))
        {
            if settings.route.pace_auto {
                settings.route.low_pace = low;
                settings.route.high_pace = high;
            }
            if settings.route.tail_pace_auto {
                settings.route.tail_low_pace = low;
                settings.route.tail_high_pace = high;
            }
            pace_auto = Some((low, high));
        }
    }

    // HUD 轨迹：工程里的 GPX → 视频同名 .gpx → 轨迹文件自带的时间轴（只导入 .qrt 的工程
    // 靠第三档才有配速/距离/航向）。
    let gpx = resolve_hud_track(
        &settings.hud.gpx_file_path,
        &video,
        maps.first().map(|m| &m.rm),
    );

    let processing = {
        // Heavy work now runs in the dedicated preview worker; 720p fits the 16.7ms
        // budget there without blocking QML/painting. preview_perf remains 480p and
        // skips stabilization entirely.
        let cap = if settings.preview_perf { 480 } else { 720 };
        processing_size(info.width, info.height, cap)
    };

    PendingApply {
        settings,
        info,
        segs,
        maps,
        gpx,
        processing,
        pace_auto,
        error: None,
    }
}

/// The map jobs on the global timeline: (map bitmap path, QuickRoute track path,
/// start_ms), sorted by start_ms. The QuickRoute JPEG IS the map + track: use it as
/// the map bitmap too when no separate map image is configured, so the 轨迹 page needs
/// only one file field. Each extra map file carries its own QuickRoute track covering
/// the SAME time span; its polyline is clipped to `start_ms` at draw time (draw_route_pub).
pub(crate) fn map_jobs(settings: &Settings) -> Vec<(String, String, f64)> {
    let map_src = if !settings.map.image_file_path.is_empty() {
        settings.map.image_file_path.clone()
    } else {
        settings.route.quick_route_jpeg_file_path.clone()
    };
    let mut jobs: Vec<(String, String, f64)> = Vec::new();
    if !map_src.is_empty() || !settings.route.quick_route_jpeg_file_path.is_empty() {
        jobs.push((
            map_src,
            settings.route.quick_route_jpeg_file_path.clone(),
            0.0,
        ));
    }
    // 换图时间相对出发时刻：出发时刻落在视频时间轴的第 start_time_offset_s 秒，故
    // 全局换图毫秒 = (录像-出发时间 + 换图时间) × 1000。换图时间本身从 0 起算（负值按 0）。
    //
    // 换图时间**没填**（`None`）的额外地图不参与播放：它既没有起点可排，
    // 又不能让 `locate_map` 因为同为 0 起点而把它选成"全程生效"。
    let start_s = settings.video.start_time_offset_s;
    for (path, t_s) in settings.map.extra_maps() {
        let Some(t_s) = t_s else {
            eprintln!("[apply] 额外地图未设置换图时间，本次不参与播放: {path}");
            continue;
        };
        let global_s = start_s + t_s.max(0.0);
        jobs.push((path.clone(), path, global_s * 1000.0));
    }
    jobs.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));
    jobs
}

/// Decode + downscale one map bitmap and initialise its route manager.
/// `map_path` is the base image, `route_path` the embedded-track file (empty = no
/// route on this map). Returns the prepared map plus the first non-fatal error (if
/// any) — callers log it (preview) or abort with it (export).
pub(crate) fn prepare_one_map(
    map_path: &str,
    route_path: &str,
    start_ms: f64,
    settings: &Settings,
    splits: &str,
    rel: bool,
    route_style: RouteStyle,
    window: (f64, f64),
    view_mode: ViewMode,
) -> (PreparedMap, Option<String>) {
    let mut err: Option<String> = None;
    let (map_rgba, map_w0, map_h0) = match load_map_rgba(map_path) {
        Ok(v) => v,
        Err(e) => {
            if !map_path.is_empty() {
                err = Some(format!("地图加载失败 [{map_path}]: {e}"));
            }
            (Vec::new(), 0, 0)
        }
    };
    // The preview paints the map with QPainter every frame; a 7000x5000 source makes the
    // GUI thread miss its 16ms budget (the original used GPU textures). Downscale once to
    // a height cap and rescale the route coordinates by the same factor so the math stays
    // consistent (split scale = window/split_width => scale /= k; midpoints *= k).
    // The cap used to be 1536px which left the map visibly soft when a split zooms in;
    // 4096 keeps substantially more native detail while still fitting in memory.
    let map_max = 4096.0f64;
    let map_k = if map_w0.max(map_h0) as f64 > map_max && map_w0 > 0 && map_h0 > 0 {
        map_max / map_w0.max(map_h0) as f64
    } else {
        1.0
    };
    let (map_rgba, map_w, map_h) = if map_k < 0.999 {
        if let Some(src) = image::RgbaImage::from_raw(map_w0 as u32, map_h0 as u32, map_rgba) {
            let nw = ((map_w0 as f64 * map_k).round() as u32).max(2);
            let nh = ((map_h0 as f64 * map_k).round() as u32).max(2);
            // Lanczos3 keeps the map sharp when the split camera magnifies it.
            let resized =
                image::imageops::resize(&src, nw, nh, image::imageops::FilterType::Lanczos3);
            (resized.into_raw(), nw as i32, nh as i32)
        } else {
            (Vec::new(), 0, 0)
        }
    } else {
        (map_rgba, map_w0, map_h0)
    };

    let mut rm = RouteManager::default();
    rm.window_width = window.0;
    rm.window_height = window.1;
    apply_framing_settings(&mut rm, settings);
    if !route_path.is_empty() && map_w0 > 0 {
        let init = init_route_manager(
            &mut rm,
            route_path,
            map_w0 as f64,
            map_h0 as f64,
            splits,
            rel,
            route_style,
            view_mode,
            settings.map.relative_width,
        );
        if let Err(e) = init {
            if err.is_none() {
                err = Some(format!("路线加载失败 [{route_path}]: {e}"));
            }
        }
    }
    // Rescale the route geometry into the downsampled map space.
    if map_k < 0.999 {
        for r in &mut rm.routes {
            for p in &mut r.route_points {
                p.position_x *= map_k;
                p.position_y *= map_k;
            }
            for p in &mut r.aligned_route_points {
                p.position_x *= map_k;
                p.position_y *= map_k;
            }
            for c in &mut r.control_positions {
                c.0 *= map_k;
                c.1 *= map_k;
            }
            for st in &mut r.split_transformations {
                st.x *= map_k;
                st.y *= map_k;
                if st.scale > 0.0 {
                    st.scale /= map_k;
                }
            }
            r.runner_position.0 *= map_k;
            r.runner_position.1 *= map_k;
        }
        // 上面的循环只改了数据表；`current_st` 是初始化时用**缩放前**坐标算出来的副本
        // （FixedSplit 那份还可能停在 `initialize()` 的补丁上）。而 `apply_prepared` 在
        // 第一帧 `rm.update()` 之前就会先重绘一次，取景不在这里重新落一次的话，大地图
        // （>4096px 才走这条路）会先闪一帧错的画面：固定分段是缩放偏小，跟跑者居中是
        // 整体偏移。置 instant 旗 + 整表重算，等价于原版 `requestFullUpdate()` 之后紧接
        // 一次 update。
        rm.set_view_mode(view_mode);
        rm.request_full_update();
        rm.update(0.0, 0.0);
    }

    (
        PreparedMap {
            rgba: map_rgba,
            w: map_w,
            h: map_h,
            start_ms,
            rm,
        },
        err,
    )
}

/// Cap a video size to a maximum height while keeping the aspect ratio (even dimensions).
pub fn processing_size(w: usize, h: usize, cap_h: usize) -> (usize, usize) {
    if h > cap_h && cap_h > 0 {
        let nh = cap_h - (cap_h % 2);
        let nw = ((w as f64 * nh as f64 / h as f64).round() as usize).max(2);
        (nw - (nw % 2), nh)
    } else {
        (w - (w % 2), h - (h % 2))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 预览版面完全由导出页的三个勾选决定：勾掉的层不出现，剩下的铺满画布。
    #[test]
    fn preview_layers_follow_export_selection() {
        let mut base = Settings::default();
        base.map.image_file_path = "map.png".into();
        base.map.relative_width = 0.3;
        // 三勾全开：版面就是设置里的样子。
        let all = preview_layers(base.clone());
        assert_eq!(all.map.relative_width, 0.3);
        assert!(all.hud.layout.as_ref().is_some_and(|l| !l.widgets.is_empty()));
        // 不导地图：左边不留黑带，视频铺满。
        let mut no_map = base.clone();
        no_map.encode.include_map = false;
        assert_eq!(preview_layers(no_map).map.relative_width, 0.0);
        // 不导视频：地图铺满整帧（只导地图+HUD 的场景）。
        let mut no_video = base.clone();
        no_video.encode.include_video = false;
        assert_eq!(preview_layers(no_video).map.relative_width, 1.0);
        // 不导 HUD：布局清空，画面上不再有 HUD。
        let mut no_hud = base.clone();
        no_hud.encode.include_hud = false;
        assert!(preview_layers(no_hud)
            .hud
            .layout
            .as_ref()
            .is_some_and(|l| l.widgets.is_empty()));
    }

    /// 预览出声要同时满足「有视频段」和「导出页勾了视频」：没勾视频时预览里
    /// 根本没有视频轨，声音也就没有了（用户 m01139 的原话）。
    #[test]
    fn preview_audio_needs_the_video_layer() {
        assert!(preview_audio_wanted(true, true));
        assert!(!preview_audio_wanted(true, false));
        assert!(!preview_audio_wanted(false, true));
        assert!(!preview_audio_wanted(false, false));
    }

    /// `.qrt` 路径识别：只认扩展名，大小写不敏感，且拒绝空路径。
    #[test]
    fn qrt_path_detection() {
        assert!(is_qrt_path("D:/maps/AM 男子短距 第一面.qrt"));
        assert!(is_qrt_path("C:\\x\\Y.qrt"));
        assert!(is_qrt_path("  a.qrt  "));
        assert!(!is_qrt_path("a.jpg"));
        assert!(!is_qrt_path(""));
        assert!(!is_qrt_path("qrt"));
    }

    /// 真实 `.qrt`：内嵌地图能被解码，且**不做** 65px 头部裁剪
    /// （`.qrt` 存的是原始地图，导出的 QuickRoute JPEG 才带头部条）。
    #[test]
    fn qrt_map_decodes_without_header_crop() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("demo")
            .join("AM 男子短距 第一面.qrt");
        if !path.exists() {
            eprintln!("demo .qrt not present; skipping");
            return;
        }
        let p = path.to_string_lossy().to_string();
        let (rgba, w, h) = load_map_rgba(&p).expect("decode qrt map");
        assert_eq!((w, h), (3000, 4243), "no header band to crop in a .qrt");
        assert_eq!(rgba.len(), (w as usize) * (h as usize) * 4);
    }

    /// 「轨迹/地图显示」白名单：只有外观项的差异才允许就地更新（不重载预览）；
    /// 碰一下视频/地图/轨迹文件、分段时间、GPX、帧读出、防抖、编码就必须退回完整重载。
    /// 这是修复"播放中改一下轨迹显示 → 重载后黑屏"时新增的就地更新入口的准入判据，
    /// 判错任何一边都会退化成老毛病（保守失败），所以两边都要钉住。
    #[test]
    fn adaptive_pace_keeps_route_and_tail_independent() {
        use tracereel_core::route::RoutePoint;
        use tracereel_core::route_manager::Route;
        let points: Vec<_> = (0..100).map(|i| RoutePoint {
            pace: 3.0 + i as f64 / 10.0,
            ..RoutePoint::default()
        }).collect();
        let expected = tracereel_core::route_manager::adaptive_pace_range(&points).unwrap();
        for (route_auto, tail_auto) in [(false, false), (true, false), (false, true), (true, true)] {
            let mut route = Route::default();
            route.route_points = points.clone();
            route.style.pace_auto = route_auto;
            route.style.tail_pace_auto = tail_auto;
            route.style.low_pace = 20.0;
            route.style.high_pace = 2.0;
            route.style.tail_low_pace = 10.0;
            route.style.tail_high_pace = 4.0;
            let mut rm = RouteManager::default();
            rm.routes = vec![route];
            apply_adaptive_pace(&mut rm);
            let style = &rm.routes[0].style;
            assert_eq!((style.low_pace, style.high_pace), if route_auto { expected } else { (20.0, 2.0) });
            assert_eq!((style.tail_low_pace, style.tail_high_pace), if tail_auto { expected } else { (10.0, 4.0) });
        }
    }

    #[test]
    fn route_display_whitelist_covers_only_appearance() {
        let base = Settings::default();
        // 一字未改也算"只有显示项"（重复应用同一份设置不该重载）。
        assert!(only_route_display_changed(&base, &base));

        // 全部外观项一起改：颜色/透明度（同时改写旧的派生键 discreetColor/highlightColor）、
        // 宽度、尾巴、控制点、跑者、渲染模式/范围、视野模式、取景边距、画面占比、
        // 按表-出发时间、跑者平均因子、配速色带。
        let mut s = base.clone();
        s.route.route_color = "#ff0000".into();
        s.route.route_opacity = 0.5;
        s.route.tail_color = "#00ff00".into();
        s.route.tail_opacity = 0.25;
        s.route.control_color = "#0000ff".into();
        s.route.control_opacity = 0.8;
        s.route.runner_color = "#ffff00".into();
        s.route.runner_opacity = 0.9;
        s.route.width = 8.0;
        s.route.tail_width = 3.0;
        s.route.tail_length = 30.0;
        s.route.control_radius = 5.0;
        s.route.runner_radius = 7.0;
        s.route.show_controls = !base.route.show_controls;
        s.route.show_runner = !base.route.show_runner;
        s.route.render_mode = 3;
        // 3 = 配速（尾巴新增的第三态，使用独立色带），也必须走就地更新那条路。
        s.route.tail_render_mode = 3;
        s.route.render_scope = 1;
        s.route.runner_style = 1;
        s.route.view_mode = 2;
        s.route.user_scale = 1.25;
        s.route.runner_averaging_factor = 300.0;
        s.route.low_pace = 4.0;
        s.route.high_pace = 9.0;
        s.route.pace_auto = !base.route.pace_auto;
        s.route.tail_low_pace = 12.0;
        s.route.tail_high_pace = 3.5;
        s.route.tail_pace_auto = !base.route.tail_pace_auto;
        s.route.control_time_offset = 3.0;
        s.route.left_right_margin = 0.05;
        s.route.top_bottom_margin = 0.1;
        s.map.relative_width = 0.3;
        assert!(
            only_route_display_changed(&base, &s),
            "外观项（含派生色键）应当判为可就地更新"
        );

        // 下面每一项都必须判"需要完整重载"。
        let mut s = base.clone();
        s.video.video_path = "D:/x/y.mp4".into();
        assert!(!only_route_display_changed(&base, &s), "换视频必须重载");

        let mut s = base.clone();
        s.video.extra_videos = vec!["D:/x/y2.mp4".into()];
        assert!(!only_route_display_changed(&base, &s), "换分段视频必须重载");

        let mut s = base.clone();
        s.map.image_file_path = "D:/x/m.jpg".into();
        assert!(!only_route_display_changed(&base, &s), "换地图必须重载");

        let mut s = base.clone();
        s.route.quick_route_jpeg_file_path = "D:/x/track.jpg".into();
        assert!(!only_route_display_changed(&base, &s), "换轨迹必须重载");

        let mut s = base.clone();
        s.splits.split_times = "1:23|2:55".into();
        assert!(!only_route_display_changed(&base, &s), "分段时间必须重载");

        let mut s = base.clone();
        s.hud.gpx_file_path = "D:/x/a.gpx".into();
        assert!(!only_route_display_changed(&base, &s), "换 GPX 必须重载");

        let mut s = base.clone();
        s.route.frame_readout = 12.0;
        assert!(
            !only_route_display_changed(&base, &s),
            "帧读出影响防抖，必须重载"
        );

        let mut s = base.clone();
        s.gyro.enabled = !base.gyro.enabled;
        assert!(!only_route_display_changed(&base, &s), "防抖开关必须重载");

        let mut s = base.clone();
        s.encode.backend = "nvenc".into();
        assert!(!only_route_display_changed(&base, &s), "编码后端必须重载");

        let mut s = base.clone();
        // 默认就是 "#ffffff"，这里换一个真正不同的值，否则等于什么都没改。
        s.map.background_color = "#123456".into();
        assert!(
            !only_route_display_changed(&base, &s),
            "未进白名单的地图项（底色）宁可重载，不能悄悄吞掉"
        );
    }

    /// 「取景」类设置必须在**建取景器时**同步进去：预览准备、导出、单帧截图三条路都只
    /// 建了一个 `RouteManager::default()`，以前没同步，于是工程里存的边距只在「当场改一次」
    /// 时生效（走就地更新那条路），重新打开工程/导出又回到代码默认（左右 10 / 上下 30）。
    /// 跑者平均因子按 `RUNNER_AVERAGING_FACTOR` 固定，工程里存的旧值（老工程常见 4.0）不生效。
    #[test]
    fn framing_settings_reach_the_manager() {
        let mut s = Settings::default();
        s.route.left_right_margin = 120.0;
        s.route.top_bottom_margin = 90.0;
        s.route.runner_averaging_factor = 4.0;

        let mut rm = RouteManager::default();
        apply_framing_settings(&mut rm, &s);
        assert_eq!(rm.left_right_margin, 120.0, "左右边距要跟着工程走");
        assert_eq!(rm.top_bottom_margin, 90.0, "上下边距要跟着工程走");
        assert_eq!(
            rm.runner_averaging_factor,
            tracereel_core::route_manager::RUNNER_AVERAGING_FACTOR,
            "跑者平均因子固定 0.5，不受工程里的旧值影响"
        );
        assert_eq!(rm.runner_averaging_factor, 0.5);
        assert_eq!(
            RouteManager::default().runner_averaging_factor,
            0.5,
            "取景器自己的默认值也要是 0.5（未同步的路径不许退回 4.0）"
        );
    }

    /// 统一时间校准模型（用户示例，正值约定）：
    /// 出发前几秒按表、出发前几十秒录像；「按表-出发时间」/「录像-出发时间」= 出发时刻在
    /// 对应时间轴上的秒数（正值 = 出发前就开始记录）。
    ///   T_watch = 3（轨迹开始 3s 后出发）、T_video = 40（视频开始 40s 后出发）
    ///   - 控制点(分段 s)：路线时刻 = s + T_watch
    ///   - 运动员(视频时刻 t)：路线时刻 = T_watch + (t - T_video) = t + (T_watch - T_video)
    #[test]
    fn unified_calibration_model() {
        let mut s = Settings::default();
        s.route.control_time_offset = 3.0; // 按表-出发时间
        s.video.start_time_offset_s = 40.0; // 录像-出发时间
        let st = route_style_from_settings(&s);
        assert_eq!(st.control_time_offset, 3.0);
        assert_eq!(st.runner_time_offset, -37.0);
        // 出发瞬间（视频 t = 40s）：运动员路线时刻 = 40 + (-37) = 3 = 出发时刻 ✓
        // 分段 0 点：控制点路线时刻 = 0 + 3 = 3 = 出发时刻 ✓（与运动员出发点重合）
    }

    /// 多地图时间轴定位：t 落在「start_ms <= t」的最后一张图上；换图时刻当帧即切换。
    #[test]
    fn locate_map_follows_timeline() {
        let starts = [0.0, 120_000.0, 300_000.0];
        assert_eq!(locate_map(&starts, 0.0), 0);
        assert_eq!(locate_map(&starts, 119_999.0), 0);
        assert_eq!(locate_map(&starts, 120_000.0), 1);
        assert_eq!(locate_map(&starts, 250_000.0), 1);
        assert_eq!(locate_map(&starts, 999_999.0), 2);
        assert_eq!(locate_map(&[], 5.0), 0);
    }

    /// 换图时间**没填**的额外地图不进入排程（否则它会与主图同为 0 ms 起点，
    /// 被 `locate_map` 取成"全程生效"，一路显示到片尾）。
    #[test]
    fn unscheduled_extra_map_never_takes_over() {
        let mut s = Settings::default();
        s.map.image_file_path = "C:/m1.qrt".into();
        s.map.extra_map_files = vec!["C:/m2.qrt".into()];
        s.map.extra_map_change_times = vec![None];
        let jobs = map_jobs(&s);
        assert_eq!(jobs.len(), 1, "unscheduled map must not be loaded: {jobs:?}");
        let starts: Vec<f64> = jobs.iter().map(|j| j.2).collect();
        assert_eq!(locate_map(&starts, 0.0), 0);
        assert_eq!(locate_map(&starts, 3_600_000.0), 0);

        // 设了时间就正常按时间轴切换。
        s.map.extra_map_change_times = vec![Some(60.0)];
        let jobs = map_jobs(&s);
        assert_eq!(jobs.len(), 2);
        let starts: Vec<f64> = jobs.iter().map(|j| j.2).collect();
        assert_eq!(starts, vec![0.0, 60_000.0]);
        assert_eq!(locate_map(&starts, 59_999.0), 0);
        assert_eq!(locate_map(&starts, 60_000.0), 1);
    }

    /// 旧模型兼容：demo.orv 原 runnerTimeOffset=-31 等价于 录像-出发时间=31、按表-出发时间=0。
    #[test]
    fn calibration_legacy_demo_equivalent() {
        let mut s = Settings::default();
        s.route.control_time_offset = 0.0;
        s.video.start_time_offset_s = 31.0;
        let st = route_style_from_settings(&s);
        assert_eq!(st.control_time_offset, 0.0);
        assert_eq!(st.runner_time_offset, -31.0);
    }

    /// 预览的取景模式必须跟设置走。原版「不填分段时间 + 跟跑者居中」时地图/轨迹是跟着
    /// 跑者动的（空分段下固定分段才静止）。回归点正是 `compute_apply`——预览的准备函数
    /// 曾经硬编码 `FixedSplit`，视野模式下拉对预览完全无效。
    #[test]
    fn preview_view_mode_drives_the_camera() {
        let qrt = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("demo")
            .join("AM 男子短距 第一面.qrt");
        if !qrt.exists() {
            eprintln!("demo .qrt not present; skipping");
            return;
        }
        let qrt = qrt.to_string_lossy().to_string();

        // 沿时间轴推进，返回 (x, y) 的移动量。跟跑者居中带平均器，必须逐帧推进——一步
        // 跳到终点的话平均器只走了一个 alpha（测试会误判成"没动"）。
        let run = |view_mode: i32| -> (f64, f64, f64) {
            let mut s = Settings::default();
            s.map.image_file_path = qrt.clone();
            s.route.quick_route_jpeg_file_path = qrt.clone();
            s.splits.split_times = String::new(); // 不填分段时间
            s.route.view_mode = view_mode;
            let p = std::env::temp_dir().join(format!("ov_viewmode_preview_{view_mode}.orv"));
            s.save_orv(&p.to_string_lossy()).expect("write temp orv");
            let pa = compute_apply(&p.to_string_lossy());
            assert!(pa.error.is_none(), "apply error: {:?}", pa.error);
            let mut rm = pa.maps.into_iter().next().expect("one map").rm;
            let ft = 1000.0 / 30.0;
            rm.update(0.0, ft);
            let (x0, y0) = (rm.get_x(), rm.get_y());
            // 走到轨迹时长的一半（上限 60s），保证这段时间里跑者确实在前进。
            let end = (rm.duration_s() * 0.5).clamp(1.0, 60.0);
            let mut t = 0.0;
            while t < end {
                t += 1.0 / 30.0;
                rm.update(t, ft);
            }
            (
                (rm.get_x() - x0).abs(),
                (rm.get_y() - y0).abs(),
                rm.duration_s(),
            )
        };

        // 固定分段 + 空分段：没有分段可切，取景必须原地不动。
        let (dx_fixed, dy_fixed, _) = run(0);
        // 跟跑者居中：取景跟着跑者走（原版行为，正是本轮要对齐的）。
        let (dx_runner, dy_runner, dur) = run(1);
        eprintln!(
            "[viewmode] 固定分段 dx={dx_fixed:.3} dy={dy_fixed:.3} | 跟跑者居中 dx={dx_runner:.3} dy={dy_runner:.3} | 轨迹时长={dur:.1}s"
        );
        assert!(
            dx_fixed < 1e-9 && dy_fixed < 1e-9,
            "空分段下固定分段不该动: dx={dx_fixed} dy={dy_fixed}"
        );
        assert!(
            dx_runner > 50.0 || dy_runner > 50.0,
            "跟跑者居中的预览取景必须随跑者移动: dx={dx_runner} dy={dy_runner} 轨迹时长={dur}s"
        );
    }

    /// 路线渲染模式必须真正画出不同的结果（对齐原版 Renderer.cpp:505-587）：
    /// 简洁 = 路线颜色单色整条描边；配速 = 逐段用**后一个点**的 pace 取色（绿=快、红=慢）。
    /// 另外颜色与不透明度是两个独立设置项，必须都传到样式里、并真的落到画面上。
    #[test]
    fn route_render_modes_paint_distinct_pixels() {
        // Settings → RouteStyle 的映射（apply 时烘焙）。原来这里的根因就是「只要不是无，
        // 就都当成简洁模式画」，所以要先把模式/颜色/配速的传递钉住。
        fn near_rgba(a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)) -> bool {
            let t = 1.5 / 255.0;
            (a.0 - b.0).abs() <= t
                && (a.1 - b.1).abs() <= t
                && (a.2 - b.2).abs() <= t
                && (a.3 - b.3).abs() <= t
        }
        let rgba = |r: u8, g: u8, b: u8, a: u8| {
            (
                r as f64 / 255.0,
                g as f64 / 255.0,
                b as f64 / 255.0,
                a as f64 / 255.0,
            )
        };
        // 模式编号沿用原版：0=无、1=简洁、3=配速；2 是原版的「高亮」，本程序没有这个模式，
        // 老工程里存到 2 的一律按单色的「简洁」渲染。
        for (m, want) in [
            (0, RenderMode::None),
            (1, RenderMode::Discreet),
            (2, RenderMode::Discreet),
            (3, RenderMode::Pace),
        ] {
            let mut sx = Settings::default();
            sx.route.render_mode = m;
            assert_eq!(
                route_style_from_settings(&sx).route_render_mode,
                want,
                "路线渲染模式 {m} 必须映射到 {want:?}"
            );
        }
        // 尾巴与路线同一套编号：无/简洁/配速（原版 2=高亮 → 简洁，老工程的尾巴不至于忽然消失）。
        for (m, want) in [
            (0, RenderMode::None),
            (1, RenderMode::Discreet),
            (2, RenderMode::Discreet),
            (3, RenderMode::Pace),
        ] {
            let mut sx = Settings::default();
            sx.route.tail_render_mode = m;
            assert_eq!(
                route_style_from_settings(&sx).tail_render_mode,
                want,
                "尾巴渲染模式 {m} 必须映射到 {want:?}"
            );
        }

        // 颜色 + 独立的不透明度：四组都要传进样式（「每个颜色项配一个透明度」的落点）。
        let mut st_src = Settings::default();
        st_src.route.route_color = "#112233".to_string();
        st_src.route.route_opacity = 64.0;
        st_src.route.tail_color = "#ff8800".to_string();
        st_src.route.tail_opacity = 192.0;
        st_src.route.control_color = "#00aa55".to_string();
        st_src.route.control_opacity = 128.0;
        st_src.route.runner_color = "#123456".to_string();
        st_src.route.runner_opacity = 200.0;
        st_src.route.low_pace = 4.5;
        st_src.route.high_pace = 3.0;
        let st = route_style_from_settings(&st_src);
        assert_eq!((st.low_pace, st.high_pace), (4.5, 3.0), "低/高配速没传进样式");
        assert!(
            near_rgba(st.route_color, rgba(0x11, 0x22, 0x33, 64)),
            "路线颜色 + 路线透明度没传进样式: {:?}",
            st.route_color
        );
        assert!(
            near_rgba(st.tail_color, rgba(0xff, 0x88, 0x00, 192)),
            "尾巴颜色 + 尾巴透明度没传进样式: {:?}",
            st.tail_color
        );
        assert!(
            near_rgba(st.control_border_color, rgba(0x00, 0xaa, 0x55, 128)),
            "控制点颜色 + 控制点透明度没传进样式: {:?}",
            st.control_border_color
        );
        assert!(
            near_rgba(st.runner_color, rgba(0x12, 0x34, 0x56, 200)),
            "运动员颜色 + 运动员透明度没传进样式: {:?}",
            st.runner_color
        );
        // 颜色串里自带 alpha 时一律忽略：不透明度只认那一项，免得两个真值源打架。
        let mut st_alpha = Settings::default();
        st_alpha.route.route_color = "#11223340".to_string();
        st_alpha.route.route_opacity = 255.0;
        assert!(
            near_rgba(
                route_style_from_settings(&st_alpha).route_color,
                rgba(0x11, 0x22, 0x33, 255)
            ),
            "不透明度只能来自「路线透明度」设置项"
        );

        let qrt = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("demo")
            .join("AM 男子短距 第一面.qrt");
        if !qrt.exists() {
            eprintln!("demo .qrt not present; skipping");
            return;
        }
        let qrt = qrt.to_string_lossy().to_string();

        let mut s = Settings::default();
        s.map.image_file_path = qrt.clone();
        s.route.quick_route_jpeg_file_path = qrt.clone();
        // 控制点是从**分段**推出来的（tracereel-core/src/route_manager.rs:669 calc_control_positions），
        // 没有分段时间就没有控制点可画，所以要给几个分段才有得测。
        s.splits.split_times = "1:00|2:00|3:00".to_string();
        s.video.start_time_offset_s = 0.0;
        s.route.control_time_offset = 0.0;
        // 只看路线本身：控制点/运动员/尾巴都会往画布上填色，这里全部关掉。
        s.route.show_controls = false;
        s.route.show_runner = false;
        s.route.tail_render_mode = 0;
        s.route.route_color = "#000000".to_string();
        s.route.route_opacity = 80.0;
        s.route.tail_color = "#ff0000".to_string();
        s.route.tail_opacity = 255.0;
        // 让笔画在 600px 画布上有足够粗的实心内核（不然全是抗锯齿过渡色，取不到"主色"）。
        const CANVAS: i32 = 600;
        const PEN_ON_CANVAS: f64 = 60.0;

        let orv = std::env::temp_dir().join("ov_route_modes.orv");
        s.save_orv(&orv.to_string_lossy()).expect("write temp orv");
        let pa = compute_apply(&orv.to_string_lossy());
        assert!(pa.error.is_none(), "apply error: {:?}", pa.error);
        let mut rm = pa.maps.into_iter().next().expect("one map").rm;
        rm.update(0.0, 1000.0 / 30.0);

        let (n_pts, minx, miny, maxx, maxy) = {
            let route = rm.routes.first().expect("route");
            let (mut minx, mut miny) = (f64::INFINITY, f64::INFINITY);
            let (mut maxx, mut maxy) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
            for rp in &route.route_points {
                minx = minx.min(rp.position_x);
                maxx = maxx.max(rp.position_x);
                miny = miny.min(rp.position_y);
                maxy = maxy.max(rp.position_y);
            }
            (route.route_points.len(), minx, miny, maxx, maxy)
        };
        assert!(n_pts > 2, "demo 路线点太少: {n_pts}");

        // 配速只在 **QuickRoute/GPS 轨迹**上才有值（原版 QuickRouteReader.cpp:368-377 与本仓库
        // tracereel-core/src/quickroute.rs:420-434 是同一条算法）。demo 这个「地图+线路」.qrt 没有逐点
        // 配速，所以这里自己铺一条 5→15 分钟/公里的线性配速，专门验证渲染端把「快=绿、慢=红」
        // 的色带逐段画出来；真实 GPS 轨迹的配速分布另有一段断言（见本测试末尾）。
        const FAST_PACE: f64 = 5.0;
        const SLOW_PACE: f64 = 15.0;
        {
            let route = rm.routes.first_mut().expect("route");
            let n = route.route_points.len() as f64;
            for (i, rp) in route.route_points.iter_mut().enumerate() {
                rp.pace = FAST_PACE + (SLOW_PACE - FAST_PACE) * (i as f64) / (n - 1.0).max(1.0);
            }
        }

        // 画笔用的样式是 apply 时烘焙进 route.style 的（draw_route_pub 读的是 route.style），
        // 所以下面按模式改写 route.style，而不是改 Settings（那只在下次 apply 时才生效）。
        let base_style = rm.routes.first().expect("route").style.clone();
        // 把路线点包围盒铺满画布（这里只测"怎么画"，不测取景）。
        let k = (CANVAS as f64 * 0.9) / (maxx - minx).max(maxy - miny).max(1.0);
        let (cx, cy) = ((minx + maxx) / 2.0, (miny + maxy) / 2.0);
        // 让笔画在画布上恒定 PEN_ON_CANVAS 像素粗，才有实心内核可数（否则全是抗锯齿过渡色）。
        s.route.user_scale = PEN_ON_CANVAS / (base_style.route_width * k);

        fn variant(base: &RouteStyle, mode: RenderMode) -> RouteStyle {
            let mut st = base.clone();
            st.route_render_mode = mode;
            // 只看路线本身：尾巴/控制点/运动员都会往画布上填色。
            st.tail_render_mode = RenderMode::None;
            st.show_controls = false;
            st.show_runner = false;
            st
        }

        // 画一次并统计：返回 (上色像素数, 出现最多的颜色, 全部上色像素的颜色)。
        // `t` 是全局时刻：跑者和尾巴的位置靠 update 决定，所以先 update 再画。
        fn render_with(
            rm: &mut RouteManager,
            style: RouteStyle,
            s: &Settings,
            k: f64,
            cx: f64,
            cy: f64,
            canvas: i32,
            t: f64,
        ) -> (usize, [u8; 3], Vec<[u8; 3]>) {
            rm.routes.first_mut().expect("route").style = style;
            rm.update(t, 1.0 / 30.0);
            let mut img = qtwrap::qimage_new_argb32(canvas, canvas);
            {
                let mut handle = qtwrap::qpainter_on_image(&mut img);
                let p: &mut QPainter = &mut handle;
                p.fill_rect(
                    QRectF {
                        x: 0.0,
                        y: 0.0,
                        width: canvas as f64,
                        height: canvas as f64,
                    },
                    QBrush::from_color(QColor::from_rgba(255, 255, 255, 255)),
                );
                p.save();
                p.scale(k, k);
                p.translate(QPointF {
                    x: (canvas as f64 / 2.0) / k - cx,
                    y: (canvas as f64 / 2.0) / k - cy,
                });
                draw_route_pub(p, rm, s, t, 0.0);
                p.restore();
            }
            let rgba = qtwrap::qimage_to_rgba(&img);
            let mut hist: std::collections::HashMap<[u8; 3], usize> = std::collections::HashMap::new();
            let mut painted: Vec<[u8; 3]> = Vec::new();
            for px in rgba.chunks_exact(4) {
                let c = [px[0], px[1], px[2]];
                if c == [255, 255, 255] {
                    continue;
                }
                painted.push(c);
                *hist.entry(c).or_insert(0) += 1;
            }
            let top = hist
                .iter()
                .max_by_key(|(_, n)| **n)
                .map(|(c, _)| *c)
                .unwrap_or([0, 0, 0]);
            (painted.len(), top, painted)
        }
        let near = |c: [u8; 3], e: [u8; 3]| (0..3).all(|i| (c[i] as i32 - e[i] as i32).abs() <= 3);
        // 偏绿（快）/偏红（慢）/纯红（控制点）/偏蓝（运动员）。
        let greenish = |px: &[[u8; 3]]| {
            px.iter()
                .filter(|c| c[1] as i32 > c[0] as i32 + 30 && c[1] as i32 > c[2] as i32 + 30)
                .count()
        };
        let reddish = |px: &[[u8; 3]]| {
            px.iter()
                .filter(|c| c[0] as i32 > c[1] as i32 + 30 && c[0] as i32 > c[2] as i32 + 30)
                .count()
        };
        let strong_red =
            |px: &[[u8; 3]]| px.iter().filter(|c| c[0] > 200 && c[1] < 60 && c[2] < 60).count();
        let blueish = |px: &[[u8; 3]]| {
            px.iter()
                .filter(|c| c[2] as i32 > c[0] as i32 + 30 && c[2] as i32 > c[1] as i32 + 30)
                .count()
        };

        // 无：路线一个像素都不画。
        let (n_none, ..) = render_with(
            &mut rm,
            variant(&base_style, RenderMode::None),
            &s,
            k,
            cx,
            cy,
            CANVAS,
            0.0,
        );
        assert_eq!(n_none, 0, "路线渲染模式=「无」时不该画路线");

        // 简洁：#000000 + 透明度 80（=原版 discreetColor(0,0,0,80)）压在白底上 = 175 灰，且没有配速色。
        let (n_d, top_d, px_d) = render_with(
            &mut rm,
            variant(&base_style, RenderMode::Discreet),
            &s,
            k,
            cx,
            cy,
            CANVAS,
            0.0,
        );
        assert!(n_d > 0, "简洁模式什么都没画");
        assert!(
            greenish(&px_d) == 0 && reddish(&px_d) == 0,
            "简洁模式是单色，不该出现配速色"
        );
        assert!(
            near(top_d, [175, 175, 175]),
            "简洁颜色应对齐 #000000 + 透明度 80，实际主色 {top_d:?}"
        );

        // 同样的颜色、透明度拉到 255 → 必须是纯黑：证明「路线透明度」真的进了画笔。
        let mut opaque_style = variant(&base_style, RenderMode::Discreet);
        opaque_style.route_color = (0.0, 0.0, 0.0, 1.0);
        let (n_o, top_o, _) =
            render_with(&mut rm, opaque_style, &s, k, cx, cy, CANVAS, 0.0);
        assert!(
            n_o > 0 && near(top_o, [0, 0, 0]),
            "路线透明度=255 时应为纯黑，实际 {top_o:?}"
        );
        assert!(top_o != top_d, "路线透明度必须影响画面: {top_o:?} vs {top_d:?}");

        // 配速：合成的配速正好从 5 走到 15，而样式里的色带端点就是 15/5（Settings 默认值），
        // 所以整条路线两端颜色（绿=快、红=慢）都必须出现。
        let mut pace_style = variant(&base_style, RenderMode::Pace);
        pace_style.low_pace = SLOW_PACE;
        pace_style.high_pace = FAST_PACE;
        pace_style.route_color = (0.0, 0.0, 0.0, 1.0);
        let (n_p, top_p, px_p) =
            render_with(&mut rm, pace_style.clone(), &s, k, cx, cy, CANVAS, 0.0);
        assert!(n_p > 0, "配速模式什么都没画");
        assert!(
            greenish(&px_p) > 0 && reddish(&px_p) > 0,
            "配速模式必须同时出现绿(快)与红(慢)两段: 绿={} 红={}",
            greenish(&px_p),
            reddish(&px_p)
        );
        assert!(
            top_p != top_d,
            "配速模式的主色不该等于单色模式: 配速={top_p:?} 简洁={top_d:?}"
        );
        // 逐段取色必须真的逐段变化（整条同色就说明还是单色画法）。
        let distinct = px_p.iter().collect::<std::collections::HashSet<_>>().len();
        assert!(
            distinct > 8,
            "配速模式的颜色应随配速渐变，实际只有 {distinct} 种颜色"
        );
        // 「路线透明度」也必须作用于配速（配速逐段线用它做 alpha）。
        let mut faint_pace = pace_style;
        faint_pace.route_color = (0.0, 0.0, 0.0, 60.0 / 255.0);
        let (n_fp, top_fp, _) =
            render_with(&mut rm, faint_pace, &s, k, cx, cy, CANVAS, 0.0);
        assert!(n_fp > 0, "半透明配速模式什么都没画");
        assert!(
            top_fp != top_p,
            "路线透明度必须也作用于配速: 浓={top_p:?} 淡={top_fp:?}"
        );

        // 尾巴：颜色取「尾巴颜色」而不是路线颜色——路线模式=无，只开尾巴。
        let tail_t = (rm.duration_s() * 0.5).max(60.0);
        let mut tail_style = variant(&base_style, RenderMode::None);
        tail_style.tail_render_mode = RenderMode::Discreet;
        tail_style.tail_color = (1.0, 0.0, 0.0, 1.0);
        let (n_t, top_t, px_t) =
            render_with(&mut rm, tail_style.clone(), &s, k, cx, cy, CANVAS, tail_t);
        assert!(n_t > 0, "尾巴模式=简洁 时应当画出尾巴（t={tail_t}）");
        assert!(
            reddish(&px_t) > 0 && near(top_t, [255, 0, 0]),
            "尾巴颜色没用到: 主色 {top_t:?}"
        );
        let mut faint_tail = tail_style;
        faint_tail.tail_color = (1.0, 0.0, 0.0, 60.0 / 255.0);
        let (n_ft, top_ft, _) =
            render_with(&mut rm, faint_tail, &s, k, cx, cy, CANVAS, tail_t);
        assert!(n_ft > 0, "半透明尾巴什么都没画");
        assert!(
            top_ft != top_t,
            "尾巴透明度必须影响画面: 浓={top_t:?} 淡={top_ft:?}"
        );

        // 尾巴=配速：尾巴复用路线那套色带画法（draw_pace_band），窗口内的配速必须着色。
        // 尾巴只有 60 秒左右，落在整条路线的线性配速上跨度太小（两端色差几乎看不出来），
        // 所以把尾巴窗口覆盖的那几十秒单独铺成「慢 → 快」，再断言绿(快)红(慢)都出现。
        // 注意尾巴取自 `aligned_route_points`（calculate_tail_points），和路线用的
        // `route_points` 不是同一个数组，铺配速要铺在对的那一份上。
        let mut pace_tail_style = variant(&base_style, RenderMode::None);
        pace_tail_style.tail_render_mode = RenderMode::Pace;
        pace_tail_style.tail_low_pace = SLOW_PACE;
        pace_tail_style.low_pace = 100.0; // 路线范围不同，尾巴仍应按自己的范围着色。
        pace_tail_style.tail_high_pace = FAST_PACE;
        pace_tail_style.high_pace = 90.0;
        // 配速模式下尾巴颜色不参与上色（只借它的 alpha 做整张色带的透明度）。
        pace_tail_style.tail_color = (1.0, 0.0, 0.0, 1.0);
        let tail_len = pace_tail_style.tail_length.max(1.0);
        let tail_win_start = tail_t - tail_len;
        let tail_pace_at = |t: f64| {
            SLOW_PACE + (FAST_PACE - SLOW_PACE) * ((t - tail_win_start) / tail_len).clamp(0.0, 1.0)
        };
        {
            let route = rm.routes.first_mut().expect("route");
            for rp in route.aligned_route_points.iter_mut() {
                rp.pace = tail_pace_at(rp.time);
            }
        }
        let (n_tp, top_tp, px_tp) = render_with(
            &mut rm,
            pace_tail_style.clone(),
            &s,
            k,
            cx,
            cy,
            CANVAS,
            tail_t,
        );
        assert!(n_tp > 0, "尾巴模式=配速 时应当画出尾巴（t={tail_t}）");
        assert!(
            greenish(&px_tp) > 0 && reddish(&px_tp) > 0,
            "尾巴=配速 必须同时出现绿(快)与红(慢): 绿={} 红={}",
            greenish(&px_tp),
            reddish(&px_tp)
        );
        assert!(
            top_tp != top_t,
            "尾巴=配速 的主色不该等于单色尾巴: 配速={top_tp:?} 简洁={top_t:?}"
        );
        let distinct_tp = px_tp.iter().collect::<std::collections::HashSet<_>>().len();
        assert!(
            distinct_tp > 8,
            "尾巴=配速 的颜色应随配速渐变，实际只有 {distinct_tp} 种颜色"
        );
        // 「尾巴透明度」也要作用于配速色带（位图整体 alpha 混入），不能只在单色尾巴上生效。
        let mut faint_pace_tail = pace_tail_style;
        faint_pace_tail.tail_color = (1.0, 0.0, 0.0, 60.0 / 255.0);
        let (n_ftp, top_ftp, _) =
            render_with(&mut rm, faint_pace_tail, &s, k, cx, cy, CANVAS, tail_t);
        assert!(n_ftp > 0, "半透明尾巴配速什么都没画");
        assert!(
            top_ftp != top_tp,
            "尾巴透明度必须也作用于配速: 浓={top_tp:?} 淡={top_ftp:?}"
        );

        // 控制点 / 运动员：颜色与透明度也要落到画面上。
        let mut cr_style = variant(&base_style, RenderMode::None);
        cr_style.show_controls = true;
        cr_style.show_runner = true;
        cr_style.control_border_color = (1.0, 0.0, 0.0, 1.0);
        cr_style.runner_color = (0.0, 0.0, 1.0, 1.0);
        let (n_cr, _, px_cr) =
            render_with(&mut rm, cr_style.clone(), &s, k, cx, cy, CANVAS, tail_t);
        assert!(n_cr > 0, "控制点/运动员什么都没画");
        assert!(strong_red(&px_cr) > 0, "控制点颜色没画出来");
        assert!(blueish(&px_cr) > 0, "运动员颜色没画出来");
        let mut faint_cr = cr_style;
        faint_cr.control_border_color = (1.0, 0.0, 0.0, 60.0 / 255.0);
        faint_cr.runner_color = (0.0, 0.0, 1.0, 60.0 / 255.0);
        let (n_fcr, _, px_fcr) =
            render_with(&mut rm, faint_cr, &s, k, cx, cy, CANVAS, tail_t);
        assert!(n_fcr > 0, "半透明控制点/运动员什么都没画");
        assert!(
            strong_red(&px_fcr) == 0,
            "控制点透明度调低后不该还有纯红像素"
        );
        assert!(
            blueish(&px_fcr) > 0,
            "运动员透明度调低后蓝色像素仍应在（只是变淡）"
        );

        // 配速模式的**数据源**：真实 QuickRoute 轨迹必须有快慢差，否则配速模式整条同色
        // （原版对没有 GPS 轨迹的项目也是这个表现）。
        let qr_jpg = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("demo")
            .join("黑色接力.gpx.jpg");
        if qr_jpg.exists() {
            let data = std::fs::read(&qr_jpg).expect("read quickroute jpeg");
            let qr = tracereel_core::quickroute::parse_quickroute(&data, 432.0, 614.0)
                .expect("parse quickroute");
            let (mut mn, mut mx) = (f64::INFINITY, f64::NEG_INFINITY);
            for rp in qr.route_points.iter().skip(1) {
                mn = mn.min(rp.pace);
                mx = mx.max(rp.pace);
            }
            assert!(mx > 0.0, "真实 QuickRoute 轨迹的配速不该全是 0: {mn}..{mx}");
            assert!(mx > mn, "真实 QuickRoute 轨迹必须有快慢差: {mn}..{mx}");
        } else {
            eprintln!("demo/黑色接力.gpx.jpg 不在，跳过真实配速数据检查");
        }

        eprintln!(
            "[route-modes] 无={n_none} 简洁={n_d}({top_d:?}) 不透明={n_o}({top_o:?}) 配速={n_p}({top_p:?}, 绿={} 红={}) 尾巴={n_t}({top_t:?}) 尾巴配速={n_tp}({top_tp:?}) 控制点/运动员={n_cr}",
            greenish(&px_p),
            reddish(&px_p)
        );
    }

    /// 运动员「箭头」样式：箭头必须**顺着运动方向**转 —— 在一条先向右、再向上的合成轨迹上，
    /// 箭头在前进方向伸得远（尖端 = 半径 r），反方向只到 0.6r；圆形则两个方向完全对称。
    /// 换到第二段（方向转了 90°）后，箭头也得跟着转过去。
    #[test]
    fn runner_arrow_follows_travel_direction() {
        use tracereel_core::route::RoutePoint;

        const CANVAS: i32 = 400;
        const K: f64 = 30.0; // 1 路线单位 = 30 像素
        const ROUTE_RADIUS: f64 = 0.4; // → 屏幕上 12 像素
        const R_PX: f64 = ROUTE_RADIUS * K;

        // 合成轨迹：0..10 秒沿 +x 直走，10..20 秒沿 +y 直走（每秒一个点）。
        let mut points: Vec<RoutePoint> = (0..=10)
            .map(|i| RoutePoint::new(i as f64, 0.0, i as f64))
            .collect();
        points.extend((1..=10).map(|i| RoutePoint::new(10.0, i as f64, 10.0 + i as f64)));

        let mut s = Settings::default();
        s.route.user_scale = 1.0;

        let mut style = RouteStyle::default();
        style.route_render_mode = RenderMode::None;
        style.tail_render_mode = RenderMode::None;
        style.show_controls = false;
        style.show_runner = true;
        style.runner_color = (0.0, 0.0, 1.0, 1.0);
        style.runner_radius = ROUTE_RADIUS;
        style.runner_scale = 1.0;

        let mut rm = RouteManager::default();
        rm.initialize_from_points(
            points,
            "",
            false,
            style.clone(),
            ViewMode::FixedSplit,
            0.75,
        );

        // 平移量：屏幕坐标 = K * (路线坐标 + off)。选它让两段轨迹上的运动员都落在画布里。
        let off = (-2.0, 1.0);

        // 画一次运动员，返回所有上色像素的屏幕坐标。
        fn render_runner(
            rm: &mut RouteManager,
            s: &Settings,
            t: f64,
            off: (f64, f64),
        ) -> Vec<(f64, f64)> {
            rm.update(t, 1.0 / 30.0);
            let mut img = qtwrap::qimage_new_argb32(CANVAS, CANVAS);
            {
                let mut handle = qtwrap::qpainter_on_image(&mut img);
                let p: &mut QPainter = &mut handle;
                p.fill_rect(
                    QRectF {
                        x: 0.0,
                        y: 0.0,
                        width: CANVAS as f64,
                        height: CANVAS as f64,
                    },
                    QBrush::from_color(QColor::from_rgba(255, 255, 255, 255)),
                );
                p.save();
                p.scale(K, K);
                p.translate(QPointF {
                    x: off.0,
                    y: off.1,
                });
                draw_route_pub(p, rm, s, t, 0.0);
                p.restore();
            }
            let rgba = qtwrap::qimage_to_rgba(&img);
            let mut out = Vec::new();
            for (i, px) in rgba.chunks_exact(4).enumerate() {
                if px[0] == 255 && px[1] == 255 && px[2] == 255 {
                    continue;
                }
                out.push((
                    (i % CANVAS as usize) as f64,
                    (i / CANVAS as usize) as f64,
                ));
            }
            out
        }

        // 沿 `dir` 方向：返回（顺方向的伸出量, 逆方向的伸出量），相对运动员位置。
        let extents = |px: &[(f64, f64)], origin: (f64, f64), dir: (f64, f64)| {
            let mut fwd = 0.0f64;
            let mut back = 0.0f64;
            for (x, y) in px {
                let v = (x - origin.0) * dir.0 + (y - origin.1) * dir.1;
                if v >= 0.0 {
                    fwd = fwd.max(v);
                } else {
                    back = back.max(-v);
                }
            }
            (fwd, back)
        };

        // ---- 第一段（t=5，朝 +x）：圆形对称，箭头前面长后面短 ----
        style.runner_style = RunnerStyle::Circle;
        rm.routes[0].style = style.clone();
        let circle = render_runner(&mut rm, &s, 5.0, off);
        let (rx, ry) = rm.routes[0].runner_position;
        let origin = (K * (rx + off.0), K * (ry + off.1));
        let dir1 = rm.runner_direction(0, 5.0, 0.5);
        assert!(
            dir1.0 > 0.98 && dir1.1.abs() < 0.05,
            "第一段运动方向应是 +x，实际 {dir1:?}"
        );
        let (cf, cb) = extents(&circle, origin, dir1);
        assert!(
            cf > R_PX * 0.8 && cb > R_PX * 0.8,
            "圆形运动员没画出来: 顺 {cf} 逆 {cb}"
        );
        assert!(
            (cf - cb).abs() < 1.5,
            "圆形在前进/后退方向必须对称: 顺 {cf} 逆 {cb}"
        );

        style.runner_style = RunnerStyle::Arrow;
        rm.routes[0].style = style.clone();
        let arrow1 = render_runner(&mut rm, &s, 5.0, off);
        let (rx, ry) = rm.routes[0].runner_position;
        let origin = (K * (rx + off.0), K * (ry + off.1));
        let (af, ab) = extents(&arrow1, origin, dir1);
        assert!(
            af > ab * 1.4,
            "箭头应在运动方向伸得更远: 顺 {af} 逆 {ab}"
        );
        assert!(
            (af - R_PX).abs() < 2.5,
            "箭头尖端应到半径处（{R_PX}），实际 {af}"
        );
        assert!(
            arrow1.len() < circle.len(),
            "箭头是有尖角的形状，面积应小于同半径的圆: {} vs {}",
            arrow1.len(),
            circle.len()
        );

        // ---- 第二段（t=15，朝 +y）：方向转 90°，箭头也必须跟着转 ----
        let dir2 = rm.runner_direction(0, 15.0, 0.5);
        assert!(
            dir2.1 > 0.98 && dir2.0.abs() < 0.05,
            "第二段运动方向应是 +y，实际 {dir2:?}"
        );
        let arrow2 = render_runner(&mut rm, &s, 15.0, off);
        let (rx, ry) = rm.routes[0].runner_position;
        let origin = (K * (rx + off.0), K * (ry + off.1));
        let (a2f, a2b) = extents(&arrow2, origin, dir2);
        assert!(
            a2f > a2b * 1.4,
            "第二段的箭头也要顺着 +y: 顺 {a2f} 逆 {a2b}"
        );
        // 转了 90° 之后，沿 +x 方向应该恢复对称（尖端已不在 +x 上）。
        let (xf, xb) = extents(&arrow2, origin, (1.0, 0.0));
        assert!(
            (xf - xb).abs() < 1.5,
            "箭头转过去后 x 方向应大致对称: 顺 {xf} 逆 {xb}"
        );

        eprintln!(
            "[runner-arrow] 圆={} 箭头={} 第一段 顺={af:.1} 逆={ab:.1} / 第二段 顺={a2f:.1} 逆={a2b:.1}",
            circle.len(),
            arrow1.len()
        );
    }

    /// 配速模式必须画成**连续的一条带子**，不能是「一个一个点」：
    /// 「逐段 draw_line + 圆头端」会在每个接头处把半透明色叠两遍，段长一短于线宽，
    /// 整条路线就变成一串珠。这里用「段长 8 < 线宽 10」这种最容易串珠的比例，断言
    /// ①没有任何像素比单次覆盖更暗（没有叠加）；②横向每一列都有像素（没有断点）。
    #[test]
    fn pace_route_is_continuous_without_beads() {
        use tracereel_core::route::RoutePoint;

        const CANVAS: i32 = 480;
        const K: f64 = 5.0; // 1 路线单位 = 5 像素
        const WIDTH: f64 = 10.0; // 线宽 10 路线单位 = 50 像素

        // 横向直线：每秒 8 个路线单位（段长 8 < 线宽 10），配速恒为 10（5..15 的正中）。
        let points: Vec<RoutePoint> = (0..=10)
            .map(|i| RoutePoint::new(i as f64 * 8.0, 0.0, i as f64))
            .collect();

        let mut s = Settings::default();
        s.route.user_scale = 1.0;

        let mut style = RouteStyle::default();
        style.route_render_mode = RenderMode::Pace;
        style.tail_render_mode = RenderMode::None;
        style.show_controls = false;
        style.show_runner = false;
        style.route_width = WIDTH;
        style.low_pace = 15.0;
        style.high_pace = 5.0;
        // 路线不透明度 80 → 单次覆盖压在白底上 = (255,255,175)，叠两次会掉到 ~(255,255,120)。
        style.route_color = (0.0, 0.0, 0.0, 80.0 / 255.0);

        let mut rm = RouteManager::default();
        rm.initialize_from_points(points, "", false, style, ViewMode::FixedSplit, 0.75);
        rm.update(0.0, 1.0 / 30.0);

        // 屏幕 = K * (路线坐标 + off)：把 y=0 的直线抬到画布中间。
        let off = (0.0, 48.0);
        let mut img = qtwrap::qimage_new_argb32(CANVAS, CANVAS);
        {
            let mut handle = qtwrap::qpainter_on_image(&mut img);
            let p: &mut QPainter = &mut handle;
            p.fill_rect(
                QRectF {
                    x: 0.0,
                    y: 0.0,
                    width: CANVAS as f64,
                    height: CANVAS as f64,
                },
                QBrush::from_color(QColor::from_rgba(255, 255, 255, 255)),
            );
            p.save();
            p.scale(K, K);
            p.translate(QPointF {
                x: off.0,
                y: off.1,
            });
            draw_route_pub(p, &rm, &s, 0.0, 0.0);
            p.restore();
        }

        let rgba = qtwrap::qimage_to_rgba(&img);
        let mut cols: std::collections::HashSet<usize> = std::collections::HashSet::new();
        let mut n = 0usize;
        let mut min_b = 255i32;
        for (i, px) in rgba.chunks_exact(4).enumerate() {
            if px[0] == 255 && px[1] == 255 && px[2] == 255 {
                continue;
            }
            n += 1;
            min_b = min_b.min(px[2] as i32);
            cols.insert(i % CANVAS as usize);
        }
        assert!(n > 0, "配速模式什么都没画");
        assert!(
            min_b >= 168,
            "配速带出现了叠加加深的接头（就是一串珠）：最暗蓝通道 {min_b}，单次覆盖应 ≈175"
        );
        let c0 = *cols.iter().min().expect("有上色像素");
        let c1 = *cols.iter().max().expect("有上色像素");
        let missing: Vec<usize> = (c0..=c1).filter(|c| !cols.contains(c)).collect();
        assert!(missing.is_empty(), "配速带不连续，缺列: {missing:?}");
        eprintln!("[pace-band] 像素={n} 列={c0}..={c1} 最暗蓝={min_b}");
    }

    /// 配速模式的颜色必须**沿着路线连续渐变**，不能一块一块：
    /// 用「配速从 5 线性升到 15」的直线轨迹（5 → 纯绿，15 → 纯红），断言
    /// ①逐列（每列取覆盖最满的那个像素）颜色的 L1 跳变不超过 6 —— 按弧长分板的版本在
    /// 板边界会跳 20 上下；②左端偏绿、右端偏红（渐变确实沿程铺开了）。
    #[test]
    fn pace_ribbon_preserves_short_segments_and_yellow_midpoint() {
        use tracereel_core::route::RoutePoint;
        // Width exceeds segment length: separate round caps used to erase these ramps.
        // Also test a single sparse GPS segment spanning the entire colour range.
        for (name, samples) in [
            ("dense", vec![(20.0, 5.0), (24.0, 7.5), (28.0, 10.0), (32.0, 12.5), (36.0, 15.0)]),
            ("sparse", vec![(20.0, 5.0), (36.0, 15.0)]),
        ] {
            let points: Vec<RoutePoint> = samples.iter().map(|&(x, pace)| {
                let mut point = RoutePoint::new(x, 20.0, x);
                point.pace = pace;
                point
            }).collect();
            let refs: Vec<_> = points.iter().collect();
            let mut img = qtwrap::qimage_new_transparent(280, 200);
            {
                let mut painter = qtwrap::qpainter_on_image(&mut img);
                painter.scale(5.0, 5.0);
                draw_pace_band(&mut painter, &refs, 20.0, 0.4, 15.0, 5.0, 99);
            }
            let pixels = qtwrap::qimage_to_rgba(&img);
            for y in [80, 100, 120] {
                for x in 105..175 {
                    let t = (x as f64 + 0.5 - 100.0) / 80.0;
                    let (r, g, _) = tracereel_core::math::interpolate_from_green_to_red(5.0, 15.0, 5.0 + 10.0 * t);
                    let pixel = &pixels[(y * 280 + x) * 4..][..4];
                    assert!((pixel[0] as f64 - r * 255.0).abs() <= 4.0 &&
                            (pixel[1] as f64 - g * 255.0).abs() <= 4.0,
                            "{name} at ({x},{y}): {pixel:?}, expected {r},{g}");
                    assert!((pixel[3] as i32 - 102).abs() <= 1, "opacity overlaps: {pixel:?}");
                }
            }
            let path = format!("pace_visual/quickroute-{name}.png");
            std::fs::create_dir_all("pace_visual").unwrap();
            assert!(qtwrap::qimage_save(&img, &path));
        }
    }

    #[test]
    fn pace_color_varies_continuously_along_route() {
        use tracereel_core::route::RoutePoint;

        const CANVAS_W: i32 = 1005;
        const CANVAS_H: i32 = 160;
        const K: f64 = 5.0;
        const WIDTH: f64 = 10.0; // 50 像素宽的带子
        const N: usize = 190;

        let mut points: Vec<RoutePoint> = Vec::new();
        for i in 0..N {
            let mut rp = RoutePoint::new(i as f64, 0.0, i as f64);
            rp.pace = 5.0 + 10.0 * (i as f64) / (N as f64 - 1.0);
            points.push(rp);
        }

        let mut s = Settings::default();
        s.route.user_scale = 1.0;

        let mut style = RouteStyle::default();
        style.route_render_mode = RenderMode::Pace;
        style.tail_render_mode = RenderMode::None;
        style.show_controls = false;
        style.show_runner = false;
        style.route_width = WIDTH;
        style.low_pace = 15.0;
        style.high_pace = 5.0;
        style.route_color = (0.0, 0.0, 0.0, 80.0 / 255.0);

        let mut rm = RouteManager::default();
        rm.initialize_from_points(points, "", false, style, ViewMode::FixedSplit, 0.75);
        rm.update(0.0, 1.0 / 30.0);

        let mut img = qtwrap::qimage_new_argb32(CANVAS_W, CANVAS_H);
        {
            let mut handle = qtwrap::qpainter_on_image(&mut img);
            let p: &mut QPainter = &mut handle;
            p.fill_rect(
                QRectF {
                    x: 0.0,
                    y: 0.0,
                    width: CANVAS_W as f64,
                    height: CANVAS_H as f64,
                },
                QBrush::from_color(QColor::from_rgba(255, 255, 255, 255)),
            );
            p.save();
            p.scale(K, K);
            p.translate(QPointF { x: 2.0, y: 16.0 });
            draw_route_pub(p, &rm, &s, 0.0, 0.0);
            p.restore();
        }

        // 每列取「覆盖最满」的那个像素（白底上 RGB 和最小），避开抗锯齿边缘。
        let rgba = qtwrap::qimage_to_rgba(&img);
        let w = CANVAS_W as usize;
        let mut samples: Vec<(usize, [u8; 3])> = Vec::new();
        for x in 0..w {
            let mut best: Option<([u8; 3], i32)> = None;
            for y in 0..CANVAS_H as usize {
                let i = (y * w + x) * 4;
                let px = &rgba[i..i + 4];
                if px[0] == 255 && px[1] == 255 && px[2] == 255 {
                    continue;
                }
                let sum = px[0] as i32 + px[1] as i32 + px[2] as i32;
                if best.map_or(true, |(_, s)| sum < s) {
                    best = Some(([px[0], px[1], px[2]], sum));
                }
            }
            if let Some((c, _)) = best {
                samples.push((x, c));
            }
        }
        assert!(
            samples.len() > 800,
            "配速带没有铺满画布，只有 {} 列有像素",
            samples.len()
        );

        let mut max_step = 0i32;
        let mut max_at = 0usize;
        for pair in samples.windows(2) {
            let (xa, a) = pair[0];
            let (xb, b) = pair[1];
            if xb != xa + 1 {
                continue;
            }
            let step = (a[0] as i32 - b[0] as i32).abs()
                + (a[1] as i32 - b[1] as i32).abs()
                + (a[2] as i32 - b[2] as i32).abs();
            if step > max_step {
                max_step = step;
                max_at = xa;
            }
        }
        let first = samples[0].1;
        let last = samples[samples.len() - 1].1;
        assert!(first[1] > first[0], "起点应该偏绿：{first:?}");
        assert!(last[0] > last[1], "终点应该偏红：{last:?}");
        assert!(
            max_step <= 6,
            "配速颜色一块一块（逐列最大跳变 {max_step} @ x={max_at}），应该连续渐变"
        );
        eprintln!(
            "[pace-gradient] 列={}..{} 最大逐列跳变={max_step}@x={max_at} 首={first:?} 末={last:?}",
            samples[0].0,
            samples[samples.len() - 1].0
        );
    }

    /// 配速带位图缓存必须随输入失效：同样的几何，只把配速从「全快」（绿）换成「全慢」（红），
    /// 第二次绘制必须重新生成位图。如果缓存键漏了配速，这里就会画出一张旧图。
    #[test]
    fn pace_layer_cache_invalidates_on_pace_change() {
        use tracereel_core::route::RoutePoint;

        const CANVAS: i32 = 480;
        const K: f64 = 4.0;

        let render = |pace: f64| -> Vec<u8> {
            let points: Vec<RoutePoint> = (0..=40)
                .map(|i| {
                    let mut rp = RoutePoint::new(i as f64 * 2.0, 0.0, i as f64);
                    rp.pace = pace;
                    rp
                })
                .collect();
            let mut s = Settings::default();
            s.route.user_scale = 1.0;

            let mut style = RouteStyle::default();
            style.route_render_mode = RenderMode::Pace;
            style.tail_render_mode = RenderMode::None;
            style.show_controls = false;
            style.show_runner = false;
            style.route_width = 6.0;
            style.low_pace = 15.0;
            style.high_pace = 5.0;
            style.route_color = (0.0, 0.0, 0.0, 1.0);

            let mut rm = RouteManager::default();
            rm.initialize_from_points(points, "", false, style, ViewMode::FixedSplit, 0.75);
            rm.update(0.0, 1.0 / 30.0);

            let mut img = qtwrap::qimage_new_argb32(CANVAS, CANVAS);
            {
                let mut handle = qtwrap::qpainter_on_image(&mut img);
                let p: &mut QPainter = &mut handle;
                p.fill_rect(
                    QRectF {
                        x: 0.0,
                        y: 0.0,
                        width: CANVAS as f64,
                        height: CANVAS as f64,
                    },
                    QBrush::from_color(QColor::from_rgba(255, 255, 255, 255)),
                );
                p.save();
                p.scale(K, K);
                p.translate(QPointF { x: 2.0, y: 30.0 });
                draw_route_pub(p, &rm, &s, 0.0, 0.0);
                p.restore();
            }
            qtwrap::qimage_to_rgba(&img)
        };

        let green = render(5.0);
        let red = render(15.0);

        let mut green_px = 0usize;
        let mut red_px = 0usize;
        let mut green_reddish = 0usize;
        let mut red_greenish = 0usize;
        for px in green.chunks_exact(4) {
            if px[0] == 255 && px[1] == 255 && px[2] == 255 {
                continue;
            }
            green_px += 1;
            if px[0] > px[1] {
                green_reddish += 1;
            }
        }
        for px in red.chunks_exact(4) {
            if px[0] == 255 && px[1] == 255 && px[2] == 255 {
                continue;
            }
            red_px += 1;
            if px[1] > px[0] {
                red_greenish += 1;
            }
        }
        assert!(green_px > 100 && red_px > 100, "配速带没画出来");
        assert_eq!(
            green_reddish, 0,
            "配速 5（全快）应当是纯绿，却有 {green_reddish} 个偏红像素"
        );
        assert_eq!(
            red_greenish, 0,
            "配速 15（全慢）应当是纯红，却有 {red_greenish} 个偏绿像素 —— 缓存没随配速失效"
        );
        eprintln!("[pace-cache] 绿={green_px} 红={red_px} 两张图各自纯色 ✓");
    }

    /// 渲染范围（`routeRenderScope`）：完整 = 整条路线（默认）；单段 = 只画当前所在的那一段。
    ///
    /// 轨迹是「每秒一个点、每秒走 10 个路线单位」的直线，控制点 60/120/180 秒
    /// （`build_runner_info` 还会在 t=0 隐式补一个，所以共 4 个控制点、3 段：
    /// `[0,60] [60,120] [120,180]`）。断言：
    /// ①完整渲染铺满整条路线；②单段渲染只画当前段，像素范围与控制点时间对得上；
    /// ③当前段随时间前移；④落在所有分段之外（冲线后）按最后一段画 —— 与取景兜底一致；
    /// ⑤没有分段（`split_times` 为空 → `splits` 为空）时不做限制，等同完整渲染。
    #[test]
    fn route_render_scope_limits_to_current_leg() {
        use tracereel_core::route::RoutePoint;

        const CANVAS_W: i32 = 700;
        const CANVAS_H: i32 = 60;
        const K: f64 = 0.2; // 路线单位 → 像素：整条 0..2990 刚好 598 像素宽
        const X_PER_SEC: f64 = 10.0;
        const N: usize = 300;
        const SPLITS: &str = "1:00|2:00|3:00";

        let mk_points = || -> Vec<RoutePoint> {
            (0..N)
                .map(|i| {
                    let t = i as f64;
                    RoutePoint::new(t * X_PER_SEC, 0.0, t)
                })
                .collect()
        };
        let mk_style = || {
            let mut st = RouteStyle::default();
            st.route_render_mode = RenderMode::Discreet;
            st.tail_render_mode = RenderMode::None;
            st.show_controls = false;
            st.show_runner = false;
            st.route_width = 10.0;
            st.route_color = (0.0, 0.0, 0.0, 1.0);
            st
        };

        let mut rm = RouteManager::default();
        rm.initialize_from_points(mk_points(), SPLITS, false, mk_style(), ViewMode::FixedSplit, 0.75);
        rm.update(0.0, 1.0 / 30.0);
        assert_eq!(
            rm.runner_info.splits.len(),
            4,
            "控制点数量不对（t=0 隐式一个 + 3 个），测试前提不成立"
        );

        // 画一次并返回「有像素的列」（升序）。
        let paint = |rm: &RouteManager, scope: i32, t: f64| -> Vec<usize> {
            let mut s = Settings::default();
            s.route.user_scale = 1.0;
            s.route.render_scope = scope;

            let mut img = qtwrap::qimage_new_argb32(CANVAS_W, CANVAS_H);
            {
                let mut handle = qtwrap::qpainter_on_image(&mut img);
                let p: &mut QPainter = &mut handle;
                p.fill_rect(
                    QRectF {
                        x: 0.0,
                        y: 0.0,
                        width: CANVAS_W as f64,
                        height: CANVAS_H as f64,
                    },
                    QBrush::from_color(QColor::from_rgba(255, 255, 255, 255)),
                );
                p.save();
                p.scale(K, K);
                // 路线 x=0 → 设备 x=1；路线 y=0 落在画布中线上。
                p.translate(QPointF {
                    x: 1.0 / K,
                    y: (CANVAS_H as f64 / 2.0) / K,
                });
                draw_route_pub(p, rm, &s, t, 0.0);
                p.restore();
            }
            let rgba = qtwrap::qimage_to_rgba(&img);
            let w = CANVAS_W as usize;
            let mut cols: Vec<usize> = Vec::new();
            for x in 0..w {
                for y in 0..CANVAS_H as usize {
                    let i = (y * w + x) * 4;
                    if rgba[i] != 255 || rgba[i + 1] != 255 || rgba[i + 2] != 255 {
                        cols.push(x);
                        break;
                    }
                }
            }
            cols
        };
        let lo_hi = |cols: &[usize]| (*cols.first().unwrap_or(&0), *cols.last().unwrap_or(&0));

        // ① 完整渲染 = 整条路线（路线 x 0..2990 → 设备 1..599）。
        let full = paint(&rm, 0, 90.0);
        let (full_lo, full_hi) = lo_hi(&full);
        assert!(
            full_lo <= 3 && full_hi >= 595,
            "完整渲染应该铺满整条路线，实际 {full_lo}..{full_hi}"
        );

        // ② 单段渲染：t=90 落在第二段 [60,120] → 设备 x = 0.2*600+1=121 .. 0.2*1200+1=241。
        let leg1 = paint(&rm, 1, 90.0);
        let (leg1_lo, leg1_hi) = lo_hi(&leg1);
        assert!(
            (118..=124).contains(&leg1_lo),
            "单段渲染当前段的左端不对：{leg1_lo}（应在 121 附近）"
        );
        assert!(
            (238..=244).contains(&leg1_hi),
            "单段渲染当前段的右端不对：{leg1_hi}（应在 241 附近）"
        );
        let missing: Vec<usize> = (leg1_lo..=leg1_hi).filter(|c| !leg1.contains(c)).collect();
        assert!(
            missing.is_empty(),
            "单段渲染的当前段自己就断了，缺列: {missing:?}"
        );
        assert!(
            leg1.len() * 3 < full.len(),
            "单段渲染应该只画当前段：{} 列 vs 完整 {} 列",
            leg1.len(),
            full.len()
        );

        // ③ 当前段随时间前移：t=150 → 第三段 [120,180] → 设备 241..361。
        let leg2 = paint(&rm, 1, 150.0);
        let (leg2_lo, leg2_hi) = lo_hi(&leg2);
        assert!(
            (238..=244).contains(&leg2_lo),
            "当前段没有随时间前移：左端 {leg2_lo}（应在 241 附近）"
        );
        assert!(
            (358..=364).contains(&leg2_hi),
            "当前段没有随时间前移：右端 {leg2_hi}（应在 361 附近）"
        );

        // ④ 早于第一个控制点（t=30 其实还在第一段内 [0,60]）与冲线后（t=250 > 180，
        //    按最后一段 [120,180]）都要有东西画 —— 取景在这些时刻用的也是同一段。
        let (pre_lo, pre_hi) = lo_hi(&paint(&rm, 1, 30.0));
        assert!(
            pre_lo <= 3 && (119..=125).contains(&pre_hi),
            "第一段 [0,60] 没画对：{pre_lo}..{pre_hi}"
        );
        let (post_lo, post_hi) = lo_hi(&paint(&rm, 1, 250.0));
        assert_eq!(
            (post_lo, post_hi),
            (leg2_lo, leg2_hi),
            "冲线之后应当按最后一段画（与取景兜底一致）"
        );

        // ⑤ 没有分段时「单段渲染」不做限制：整条路线就是唯一的一段。
        let mut rm_no_split = RouteManager::default();
        rm_no_split.initialize_from_points(
            mk_points(),
            "",
            false,
            mk_style(),
            ViewMode::FixedSplit,
            0.75,
        );
        rm_no_split.update(0.0, 1.0 / 30.0);
        assert!(rm_no_split.runner_info.splits.is_empty(), "没有分段时不该有 split");
        let (ns_lo, ns_hi) = lo_hi(&paint(&rm_no_split, 1, 90.0));
        assert_eq!(
            (ns_lo, ns_hi),
            (full_lo, full_hi),
            "没有分段时单段渲染应当等同完整渲染"
        );

        eprintln!(
            "[render-scope] 完整={full_lo}..{full_hi}({} 列) 段[60,120]={leg1_lo}..{leg1_hi} \
             段[120,180]={leg2_lo}..{leg2_hi} 无分段={ns_lo}..{ns_hi}",
            full.len()
        );
    }
}
