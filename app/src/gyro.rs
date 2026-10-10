//! Gyroflow stabilization integration (in-process, no TCP frame server).
//!
//! This is the authoritative port of `gyroflow-frameserver/src/main.rs`: it loads the
//! video + preset, runs the gyro-video autosync (feeding decoded gray frames), applies
//! the found offsets to the gyro source, recomputes smoothness/zoom/undistortion, and
//! then stabilizes frames via `process_pixels`. The stabilizer is cached per
//! (video + config) signature by the caller.

use std::fs::File;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use gyroflow_core::filesystem::path_to_url;
use gyroflow_core::gpu::{BufferDescription, BufferSource, Buffers};
use gyroflow_core::gyro_source::{FileLoadOptions, Quat64, TimeQuat};
use gyroflow_core::stabilization::RGBA8;
use gyroflow_core::synchronization::{AutosyncProcess, SyncParams};
use gyroflow_core::StabilizationManager;

use crate::settings::GyroSettings;

/// An error while loading/initializing the stabilizer.
#[derive(Debug)]
pub enum GyroError {
    NoVideo(String),
    OpenVideo(String),
    LoadGyro(String),
    AutosyncInit(String),
    Gpu(String),
    Unknown(String),
}

impl std::fmt::Display for GyroError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GyroError::NoVideo(s) => write!(f, "no video stream: {s}"),
            GyroError::OpenVideo(s) => write!(f, "open video: {s}"),
            GyroError::LoadGyro(s) => write!(f, "gyro load: {s}"),
            GyroError::AutosyncInit(s) => write!(f, "autosync init: {s}"),
            GyroError::Gpu(s) => write!(f, "gpu: {s}"),
            GyroError::Unknown(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for GyroError {}

/// A fully-loaded stabilizer ready to process frames.
pub struct Stabilizer {
    pub manager: Arc<StabilizationManager>,
    pub width: usize,
    pub height: usize,
    pub fps: f64,
    pub duration_ms: f64,
    /// True when autosync found and applied offsets.
    pub autosync_done: bool,
}

impl Stabilizer {
    /// Raw (unsmoothed) quaternion track in file-local µs, for building cross-file
    /// warmup context. See [`apply_seam_warmup_smoothing`].
    pub fn raw_quaternions(&self) -> TimeQuat {
        self.manager.gyro.read().quaternions.clone()
    }

    /// Stabilize one decoded RGBA8 frame.
    ///
    /// `in_rgba` is the (src_w × src_h) input frame; `out_rgba` receives the
    /// (out_w × out_h) stabilized frame. `frame_idx` is the frame number.
    pub fn process_frame(
        &self,
        t_ms: f64,
        frame_idx: usize,
        in_rgba: &mut [u8],
        src_w: usize,
        src_h: usize,
        out_rgba: &mut [u8],
        out_w: usize,
        out_h: usize,
    ) -> Result<(), GyroError> {
        let ts_us = (t_ms * 1000.0).round() as i64;
        let r = self.manager.process_pixels::<RGBA8>(
            ts_us,
            Some(frame_idx),
            &mut Buffers {
                input: BufferDescription {
                    size: (src_w, src_h, src_w * 4),
                    data: BufferSource::Cpu { buffer: in_rgba },
                    texture_copy: true,
                    ..Default::default()
                },
                output: BufferDescription {
                    size: (out_w, out_h, out_w * 4),
                    data: BufferSource::Cpu { buffer: out_rgba },
                    texture_copy: true,
                    ..Default::default()
                },
            },
        );
        match r {
            Ok(_) => Ok(()),
            Err(e) => Err(GyroError::Unknown(format!("process_pixels: {e:?}"))),
        }
    }

    /// Change the output size. Refreshes compute_params (cheap) and clears the
    /// expensive recompute flags so per-frame requests don't redo smoothness/zoom.
    pub fn set_output_size(&self, out_w: usize, out_h: usize) {
        if self.manager.set_output_size(out_w, out_h) {
            self.manager.recompute_undistortion();
            self.manager
                .smoothing_invalidated
                .store(false, Ordering::SeqCst);
            self.manager
                .zooming_invalidated
                .store(false, Ordering::SeqCst);
            self.manager
                .undistortion_invalidated
                .store(false, Ordering::SeqCst);
        }
    }
}

/// Pipeline stage of a progress report. The load phases that have a *measurable* fraction
/// are `MotionData` (telemetry-parser reports the container offset it has consumed) and the
/// post-passes, which report their boundary. Nothing here fakes a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Idle = 0,
    Init,
    MotionData,
    Preset,
    Autosync,
    Smoothing,
    Zooming,
    Undistortion,
    Done,
}

impl Phase {
    /// Short user-facing (Chinese) label for the loading overlay / status line.
    pub fn label(self) -> &'static str {
        match self {
            Phase::Idle | Phase::Done => "",
            Phase::Init => "初始化",
            Phase::MotionData => "读取运动数据",
            Phase::Preset => "应用预设",
            Phase::Autosync => "视频同步",
            Phase::Smoothing => "计算平滑轨迹",
            Phase::Zooming => "计算自适应变焦",
            Phase::Undistortion => "镜头校正",
        }
    }

    /// Every variant in discriminant order — `ProgressSink` stores the phase as its
    /// discriminant and looks it back up here, so this list must stay in sync with the enum.
    const ALL: [Phase; 9] = [
        Phase::Idle,
        Phase::Init,
        Phase::MotionData,
        Phase::Preset,
        Phase::Autosync,
        Phase::Smoothing,
        Phase::Zooming,
        Phase::Undistortion,
        Phase::Done,
    ];

    pub fn from_u8(v: u8) -> Phase {
        Self::ALL.get(v as usize).copied().unwrap_or(Phase::Idle)
    }
}

/// One progress report: `frac` is 0..1 inside the range the caller asked for (a segment's
/// slice of the overall bar) and `phase` names the stage it belongs to.
#[derive(Clone, Copy, Debug)]
pub struct LoadProgress {
    pub frac: f64,
    pub phase: Phase,
}

impl LoadProgress {
    pub fn new(frac: f64, phase: Phase) -> Self {
        Self { frac, phase }
    }

    /// Remap this report into an outer range (used to fold per-segment progress into the
    /// overall multi-segment bar).
    pub fn scaled(self, base: f64, span: f64) -> Self {
        Self {
            frac: base + self.frac.clamp(0.0, 1.0) * span,
            phase: self.phase,
        }
    }
}

/// A progress sink (0.0..1.0) invoked from the loading thread. Implementations must be
/// cheap and lock-free (the GUI reads them every tick from another thread).
pub type Progress<'a> = &'a (dyn Fn(LoadProgress) + Send + Sync);

/// A no-op sink for callers (CLI tools) that do not display progress.
pub fn no_progress() -> Progress<'static> {
    &|_p: LoadProgress| {}
}

/// Lock-free sink the loader thread writes and the GUI polls. `frac` is stored as f64 bits;
/// -1.0 means "unknown" (indeterminate).
pub struct ProgressSink {
    frac: std::sync::atomic::AtomicU64,
    phase: std::sync::atomic::AtomicU64,
}

impl Default for ProgressSink {
    fn default() -> Self {
        Self {
            frac: std::sync::atomic::AtomicU64::new(f64::to_bits(-1.0)),
            phase: std::sync::atomic::AtomicU64::new(Phase::Idle as u64),
        }
    }
}

impl ProgressSink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&self, p: LoadProgress) {
        self.phase
            .store(p.phase as u64, std::sync::atomic::Ordering::Relaxed);
        self.frac
            .store(p.frac.to_bits(), std::sync::atomic::Ordering::Relaxed);
    }

    /// 0..1, or -1.0 when nothing has been reported yet.
    pub fn frac(&self) -> f64 {
        f64::from_bits(self.frac.load(std::sync::atomic::Ordering::Relaxed))
    }

    pub fn phase(&self) -> Phase {
        Phase::from_u8(self.phase.load(std::sync::atomic::Ordering::Relaxed) as u8)
    }
}

/// Phase boundaries for one segment's load, as fractions of that segment's 0..1 range.
///
/// `load_gyro_data` dominates wall time: telemetry-parser walks the metadata ("none")
/// stream sample by sample and reports the file offset it has consumed, so for a DJI file
/// this is literally "how much of the container has been scanned" — a genuine progress
/// bar, exactly like the native Gyroflow app. The post-passes (smoothing / adaptive zoom /
/// undistortion) are comparatively cheap but not free, so they get their own slices.
/// Measured on the 16 GB / 23-min DJI demo (cold cache): init 0.3s, motion-data 9.3s,
/// smooth 2.2s, zoom 0.7s.
const PROG_INIT_END: f64 = 0.03;
const PROG_LOAD_END: f64 = 0.78;
const PROG_PRESET_END: f64 = 0.79;
const PROG_SMOOTH_END: f64 = 0.95;
const PROG_ZOOM_END: f64 = 0.995;

/// Warmup window taken from each adjacent segment when re-smoothing at file seams (µs).
const SEAM_WARMUP_US: i64 = 3_000_000;
/// A seam whose raw orientation jumps more than this (degrees) is treated as two
/// unrelated recordings and gets no warmup — smoothing across a real discontinuation
/// would manufacture a phantom camera pan.
const SEAM_MAX_ANGLE_DEG: f64 = 30.0;

fn seam_angle_deg(a: &Quat64, b: &Quat64) -> f64 {
    (a.inverse() * b).angle().to_degrees()
}

/// Remap the tail of `prev`'s raw track onto `cur`'s local timeline (just before `cur`'s
/// first sample). `None` when there is no adjacent file or the seam is discontinuous.
fn warmup_head(prev: Option<&TimeQuat>, cur: &TimeQuat) -> Option<TimeQuat> {
    let prev = prev?;
    let (&cur_lo, cur_first) = cur.iter().next()?;
    let (&prev_hi, prev_last) = prev.iter().next_back()?;
    let ang = seam_angle_deg(prev_last, cur_first);
    if ang > SEAM_MAX_ANGLE_DEG {
        eprintln!("[stab] seam vs previous segment: raw jump {ang:.1}° > {SEAM_MAX_ANGLE_DEG}°, warmup skipped");
        return None;
    }
    let mut h = TimeQuat::new();
    for (ts, q) in prev.range(prev_hi - SEAM_WARMUP_US..) {
        h.insert(ts - prev_hi + cur_lo - 1, *q);
    }
    (!h.is_empty()).then_some(h)
}

/// Remap the head of `next`'s raw track onto `cur`'s local timeline (just after `cur`'s
/// last sample). `None` when there is no adjacent file or the seam is discontinuous.
fn warmup_tail(cur: &TimeQuat, next: Option<&TimeQuat>) -> Option<TimeQuat> {
    let next = next?;
    let (&cur_hi, cur_last) = cur.iter().next_back()?;
    let (&next_lo, next_first) = next.iter().next()?;
    let ang = seam_angle_deg(cur_last, next_first);
    if ang > SEAM_MAX_ANGLE_DEG {
        eprintln!("[stab] seam vs next segment: raw jump {ang:.1}° > {SEAM_MAX_ANGLE_DEG}°, warmup skipped");
        return None;
    }
    let mut t = TimeQuat::new();
    for (ts, q) in next.range(..next_lo + SEAM_WARMUP_US) {
        t.insert(ts - next_lo + cur_hi + 1, *q);
    }
    (!t.is_empty()).then_some(t)
}

/// Re-run smoothing on every segment using the raw quaternion tracks of its adjacent
/// segments as warmup context. The IIR smoothing passes anchor on the first/last
/// quaternion of the sequence they receive, so a plain multi-segment export "resets"
/// stabilization at each file seam; re-smoothing with context makes each file's motion
/// near the seam a continuation of its neighbor instead of an edge effect.
///
/// `stabs` must be in recording order; `None` slots and non-adjacent pairs are skipped.
/// Segments are re-smoothed through the manager's interior locks, so no slot replacement
/// is needed — preview and export see the refined results.
pub fn apply_seam_warmup_smoothing(stabs: &[Option<Arc<Stabilizer>>], progress: Progress) {
    let idx: Vec<usize> = stabs
        .iter()
        .enumerate()
        .filter(|(_, s)| s.is_some())
        .map(|(i, _)| i)
        .collect();
    if idx.len() < 2 {
        return;
    }
    let raws: Vec<TimeQuat> = idx
        .iter()
        .map(|&i| stabs[i].as_ref().unwrap().raw_quaternions())
        .collect();
    let empty = TimeQuat::new();
    let t0 = std::time::Instant::now();
    for k in 0..idx.len() {
        progress(LoadProgress::new(
            k as f64 / idx.len() as f64,
            Phase::Smoothing,
        ));
        let adj_prev = if k > 0 && idx[k - 1] + 1 == idx[k] {
            Some(&raws[k - 1])
        } else {
            None
        };
        let adj_next = if k + 1 < idx.len() && idx[k] + 1 == idx[k + 1] {
            Some(&raws[k + 1])
        } else {
            None
        };
        let head = warmup_head(adj_prev, &raws[k]);
        let tail = warmup_tail(&raws[k], adj_next);
        if head.is_none() && tail.is_none() {
            continue;
        }
        let stab = stabs[idx[k]].as_ref().unwrap();
        stab.manager.recompute_smoothness_with_context(
            head.as_ref().unwrap_or(&empty),
            tail.as_ref().unwrap_or(&empty),
        );
        stab.manager.recompute_adaptive_zoom();
        stab.manager.recompute_undistortion();
        eprintln!(
            "[stab] seam warmup: seg {} re-smoothed (head={} tail={} samples)",
            idx[k],
            head.map_or(0, |h| h.len()),
            tail.map_or(0, |t| t.len())
        );
    }
    progress(LoadProgress::new(1.0, Phase::Smoothing));
    eprintln!(
        "[stab] seam warmup pass finished in {:.1}s",
        t0.elapsed().as_secs_f64()
    );
}

/// Dump the smoothed quaternions over [start_ms, end_ms] to a CSV (diagnostic: compare
/// with gyroflow's `--export-metadata 3` stabilized quaternions).
pub fn dump_smoothed_quats(
    stab: &Stabilizer,
    start_ms: f64,
    end_ms: f64,
    path: &str,
) -> Result<(), String> {
    use std::io::Write;
    let dt = if stab.fps > 0.0 {
        1000.0 / stab.fps
    } else {
        16.6833
    };
    let mut out = String::from("t_ms,w,x,y,z\n");
    let gyro = stab.manager.gyro.read();
    let mut t = start_ms;
    while t <= end_ms {
        // `smoothed_quaternions` holds the correction quats (smooth⁻¹·raw); reverse the
        // same calculation as gyroflow's metadata export to get the stabilized camera
        // orientation quaternion (comparable with --export-metadata 3 stab_quat).
        let corr = gyro.smoothed_quat_at_timestamp(t);
        let org = gyro.org_quat_at_timestamp(t);
        let smooth = (corr / org).inverse();
        let v = smooth.as_vector(); // [i, j, k, w]
        out.push_str(&format!(
            "{t:.3},{:.9},{:.9},{:.9},{:.9}\n",
            v[3], v[0], v[1], v[2]
        ));
        t += dt;
    }
    drop(gyro);
    let mut f = std::fs::File::create(path).map_err(|e| e.to_string())?;
    f.write_all(out.as_bytes()).map_err(|e| e.to_string())?;
    eprintln!("[stab] dumped smoothed quats -> {path}");
    Ok(())
}

/// Initialize the GPU contexts (wgpu). Call once before loading.
pub fn init_gpu() -> String {
    gyroflow_core::gpu::initialize_contexts()
        .map(|(name, _)| name)
        .unwrap_or_else(|| "unknown".to_string())
}

/// Apply the `calibration_data` (+ warp interpolation) of a `.gyroflow` preset.
/// 稳定化参数**不再**从这里读 —— 它们全部来自 `GyroSettings`（GUI 可配，见 `apply_gyro_settings`）；
/// 选中 `.gyroflow` 时只把它的 stabilization 段导入设置（`GyroSettings::apply_preset_json`）。
fn apply_preset_calibration(stab: &StabilizationManager, preset_path: &str) -> Result<(), GyroError> {
    let json = std::fs::read_to_string(preset_path)
        .map_err(|e| GyroError::Unknown(format!("read preset: {e}")))?;
    let v: serde_json::Value = serde_json::from_str(&json)
        .map_err(|e| GyroError::Unknown(format!("preset parse: {e}")))?;

    if let Some(cal) = v.get("calibration_data") {
        let tmp = std::env::temp_dir().join("tracereel_lens.json");
        let s = serde_json::to_string(cal)
            .map_err(|e| GyroError::Unknown(format!("calib serialize: {e}")))?;
        std::fs::write(&tmp, &s).map_err(|e| GyroError::Unknown(format!("write calib: {e}")))?;
        stab.load_lens_profile(&tmp.to_string_lossy())
            .map_err(|e| GyroError::Unknown(format!("lens: {e:?}")))?;
    }

    // Warp interpolation — the native project renders with Lanczos4 (output.interpolation).
    if let Some(o) = v.get("output") {
        if let Some(i) = o.get("interpolation").and_then(|x| x.as_str()) {
            stab.stabilization.write().interpolation = i.into();
            eprintln!("[stab] interpolation from preset: {i}");
        }
    } else {
        stab.stabilization.write().interpolation = "Lanczos4".into();
        eprintln!("[stab] interpolation: Lanczos4 (default)");
    }

    Ok(())
}

/// 地平线锁定：取值规则照抄原版 `updateHorizonLock()`（Stabilization.qml）：
/// * 关了总开关 → 锁定量 0、横滚/俯仰修正都送 0；
/// * 勾了「自动锁定」→ 锁定量按 100 送，转弯参数生效；
/// * 俯仰只在其勾选框打开时送值；
/// * 「限制倾斜加速度」没勾 → 上限送 ∞（不限制）。
fn apply_horizon_lock(stab: &StabilizationManager, g: &GyroSettings) {
    let lock_amount = if g.horizon_lock_enabled {
        if g.horizon_automatic_lock {
            100.0
        } else {
            g.horizon_lock
        }
    } else {
        0.0
    };
    let roll = if g.horizon_lock_enabled {
        g.horizon_lock_roll
    } else {
        0.0
    };
    let lock_pitch = g.horizon_lock_enabled && g.horizon_lock_pitch_enabled;
    let pitch = if lock_pitch { g.horizon_lock_pitch } else { 0.0 };
    let accel = if g.horizon_limit_tilt_accel {
        g.horizon_tilt_accel_limit
    } else {
        f64::INFINITY
    };
    stab.smoothing.write().horizon_lock.set_horizon(
        lock_amount,
        roll,
        lock_pitch,
        pitch,
        g.horizon_automatic_lock,
        g.horizon_turn_threshold,
        g.horizon_turn_smoothing_ms,
        g.horizon_turn_multiplier,
        accel,
    );
    stab.set_use_gravity_vectors(g.use_gravity_vectors);
    stab.set_horizon_lock_integration_method(g.horizon_lock_integration_method);
}

/// 「变焦方式」→ 传给 core 的 `adaptive_zoom_method`（**只表示动态变焦的算法**）：
/// 0 = GaussianFilter、1 = EnvelopeFollower。静态/关闭时算法没有意义，
/// 但 core 对 2 会打 "Invalid zooming method: 2"（虽然只是回退），所以这里只送 0/1。
fn zoom_algorithm(mode: i32) -> i32 {
    if mode == 1 {
        1
    } else {
        0
    }
}

/// 「变焦方式」→ 传给 gyroflow_core 的 `adaptive_zoom_window` 值。
///
/// **这是变焦的总开关**：`gyroflow_core/src/zooming/mod.rs:62-75` 只看这个值 ——
/// `< -0.9` = 静态缩放、`> 0.0001` = 动态缩放（窗口=秒）、其余（0）= 关闭。
/// `adaptive_zoom_method` 只是**动态变焦的算法**（0=GaussianFilter / 1=EnvelopeFollower），
/// 不决定开关（见 `zooming/mod.rs` 的 `impl From<i32> for ZoomMethod`）。
/// 曾经把模式写进 method、把速度写进 window，而速度默认 0 → 核心恒走「关闭」，三种模式都不缩放。
fn effective_zoom_window(g: &GyroSettings) -> f64 {
    match g.zoom_mode {
        2 => -1.0, // 静态缩放
        1 => {
            // 动态缩放：窗口必须有正值，否则核心判定为关闭；缺省用 gyroflow 的 4 秒。
            if g.adaptive_zoom_window > 0.0001 {
                g.adaptive_zoom_window
            } else {
                4.0
            }
        }
        _ => 0.0, // 不变焦
    }
}

/// 把设置页里那套稳定化参数应用到 stabilizer（与 `.gyroflow` 的 stabilization 段一一对应）。
/// 原版 Gyroflow 的「Stabilization」面板就是这些项；现在它们都能在 GUI 里调，
/// 不再必须依赖某个 `.gyroflow` 文件。
pub fn apply_gyro_settings(stab: &StabilizationManager, g: &GyroSettings) {
    // 视野 / 变焦
    stab.set_fov(g.fov);
    stab.set_zooming_method(zoom_algorithm(g.zoom_mode));
    // 变焦模式必须翻译成 window（核心按它决定 静态/动态/关闭）。
    stab.set_adaptive_zoom(effective_zoom_window(g));
    stab.set_max_zoom(g.max_zoom, g.max_zoom_iterations.max(1) as usize);
    stab.set_zooming_center_x(g.zoom_center_x);
    stab.set_zooming_center_y(g.zoom_center_y);
    // 镜头 / 地平线（对齐原版 updateHorizonLock 的取值规则）
    stab.set_lens_correction_amount(g.lens_correction_amount);
    apply_horizon_lock(stab, g);
    // 卷帘快门（勾选框关掉时送 0 毫秒 = 不做卷帘校正）
    stab.set_frame_readout_time(if g.frame_readout_enabled {
        g.frame_readout_time
    } else {
        0.0
    });
    stab.set_frame_readout_direction(g.frame_readout_direction.as_str());
    // 稳定焦距（仅逐帧焦距的视频有效；核心直接读 params）
    {
        let mut p = stab.params.write();
        p.focal_length_smoothing_enabled = g.focal_length_smoothing_enabled;
        p.focal_length_smoothing_strength = g.focal_length_smoothing_strength;
    }
    // 视频速度联动
    stab.set_video_speed(
        g.video_speed,
        g.video_speed_affects_smoothing,
        g.video_speed_affects_zooming,
        g.video_speed_affects_zooming_limit,
    );
    // 附加 3D 旋转 / 平移
    stab.set_additional_rotation_x(g.additional_rotation[0]);
    stab.set_additional_rotation_y(g.additional_rotation[1]);
    stab.set_additional_rotation_z(g.additional_rotation[2]);
    stab.set_additional_translation_x(g.additional_translation[0]);
    stab.set_additional_translation_y(g.additional_translation[1]);
    stab.set_additional_translation_z(g.additional_translation[2]);
    // 平滑算法 + 该算法的参数（`.gyroflow` 的 method / smoothing_params 同格式）。
    let algs = stab.get_smoothing_algs();
    let idx = algs
        .iter()
        .position(|m| m.eq_ignore_ascii_case(&g.smoothing_method))
        .unwrap_or_else(|| if algs.len() > 1 { 1 } else { 0 });
    stab.smoothing.write().set_current(idx);
    if let Ok(serde_json::Value::Array(list)) = serde_json::from_str(&g.smoothing_params) {
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
    eprintln!(
        "[stab] settings applied: fov={:.3} readout={:.3}ms/{} zoom_method={} max_zoom={} window={} lens_correction={} horizon={:.2} smoothing={}",
        g.fov,
        g.frame_readout_time,
        g.frame_readout_direction,
        g.zoom_mode,
        g.max_zoom,
        g.adaptive_zoom_window,
        g.lens_correction_amount,
        g.horizon_lock,
        g.smoothing_method
    );
}

/// 手动陀螺<->视频时间偏移（与 `load_stabilizer` 里同一套调用），供「只重算」路径复用。
/// 传 0 会把之前的偏移复位（因为 `set_offset` 是绝对偏移）。
pub fn apply_gyro_offset(stab: &StabilizationManager, offset_ms: f64) {
    {
        let mut gyro = stab.gyro.write();
        gyro.set_offset(0, offset_ms);
        gyro.adjust_offsets();
    }
    stab.keyframes.write().update_gyro(&stab.gyro.read());
    stab.invalidate_zooming();
}

/// 只把**一个**设置字段应用回 stabilizer。
/// 「只重算」路径专用：如果把全部字段都重设一遍，那些没变的字段也会触发
/// `invalidate_smoothing()/invalidate_zooming()`，白白重算上百万帧。
fn apply_gyro_field(stab: &StabilizationManager, g: &GyroSettings, prop: &str) {
    match prop {
        "fov" => stab.set_fov(g.fov),
        "max_zoom" | "max_zoom_iterations" => {
            stab.set_max_zoom(g.max_zoom, g.max_zoom_iterations.max(1) as usize)
        }
        "gyro_zoom_mode" => {
            // 换模式同时要更新 window（核心按 window 决定开关），否则切到「动态缩放」也不生效。
            stab.set_zooming_method(zoom_algorithm(g.zoom_mode));
            stab.set_adaptive_zoom(effective_zoom_window(g));
        }
        "gyro_zoom_window" => stab.set_adaptive_zoom(effective_zoom_window(g)),
        "gyro_zoom_center_x" => stab.set_zooming_center_x(g.zoom_center_x),
        "gyro_zoom_center_y" => stab.set_zooming_center_y(g.zoom_center_y),
        "gyro_lens" => stab.set_lens_correction_amount(g.lens_correction_amount),
        "gyro_horizon"
        | "gyro_horizon_roll"
        | "gyro_horizon_pitch"
        | "gyro_horizon_pitch_enabled"
        | "gyro_horizon_lock_enabled"
        | "gyro_horizon_automatic_lock"
        | "gyro_horizon_turn_threshold"
        | "gyro_horizon_turn_smoothing"
        | "gyro_horizon_turn_multiplier"
        | "gyro_horizon_limit_tilt_accel"
        | "gyro_horizon_tilt_accel_limit" => {
            // 这九项是一起送的（原版 updateHorizonLock 也是整体重送），单项改动也走同一函数。
            apply_horizon_lock(stab, g);
        }
        "gyro_use_gravity" => apply_horizon_lock(stab, g),
        "gyro_integration_method" => {
            stab.set_horizon_lock_integration_method(g.horizon_lock_integration_method)
        }
        "frame_readout_time" | "frame_readout_enabled" => stab.set_frame_readout_time(
            if g.frame_readout_enabled {
                g.frame_readout_time
            } else {
                0.0
            },
        ),
        "gyro_fl_smoothing" | "gyro_fl_strength" => {
            let mut p = stab.params.write();
            p.focal_length_smoothing_enabled = g.focal_length_smoothing_enabled;
            p.focal_length_smoothing_strength = g.focal_length_smoothing_strength;
        }
        "gyro_readout_dir" => stab.set_frame_readout_direction(g.frame_readout_direction.as_str()),
        "gyro_smoothing_method" | "gyro_smoothing_params" => {
            let algs = stab.get_smoothing_algs();
            let idx = algs
                .iter()
                .position(|m| m.eq_ignore_ascii_case(&g.smoothing_method))
                .unwrap_or_else(|| if algs.len() > 1 { 1 } else { 0 });
            stab.smoothing.write().set_current(idx);
            if let Ok(serde_json::Value::Array(list)) = serde_json::from_str(&g.smoothing_params) {
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
        }
        "gyro_video_speed" | "gyro_vs_smoothing" | "gyro_vs_zooming" | "gyro_vs_zoom_limit" => {
            stab.set_video_speed(
                g.video_speed,
                g.video_speed_affects_smoothing,
                g.video_speed_affects_zooming,
                g.video_speed_affects_zooming_limit,
            );
        }
        "gyro_rot_x" => stab.set_additional_rotation_x(g.additional_rotation[0]),
        "gyro_rot_y" => stab.set_additional_rotation_y(g.additional_rotation[1]),
        "gyro_rot_z" => stab.set_additional_rotation_z(g.additional_rotation[2]),
        "gyro_tr_x" => stab.set_additional_translation_x(g.additional_translation[0]),
        "gyro_tr_y" => stab.set_additional_translation_y(g.additional_translation[1]),
        "gyro_tr_z" => stab.set_additional_translation_z(g.additional_translation[2]),
        // 偏移由 apply_gyro_offset 处理（它是绝对偏移，0 也要写一次复位）。
        "gyro_offset" | "gyro_offset_frames" => {}
        // 未知字段（含硬输入触发的整份应用）：全量。
        _ => apply_gyro_settings(stab, g),
    }
}

/// **只重算，不重载**：把新的稳定化参数应用到**已经加载好**的 stabilizer 上，然后按需重算
/// 平滑 / 自适应变焦（+ 刷新每帧参数），多段时再做一遍接缝平滑。
///
/// 这就是原版 Gyroflow 拖动 Stabilization 面板滑杆时发生的事 —— 陀螺运动数据早已在内存里。
/// 而且只重算**被改的那一项需要**的部分（各 setter 自己会 invalidate）：
/// * 视野 / 帧读出时间 / 读出方向：只影响每帧计算 → 一次预计算都不做，即时生效；
/// * 镜头校正 / 变焦方式·速度·中心 / 附加平移：只重算自适应变焦；
/// * 平滑方式·参数 / 地平线 / 视频速度 / 附加旋转 / 变焦上限 / 陀螺偏移：重算平滑（连带变焦）。
///
/// 返回实际处理的段数。
pub fn reapply_settings(
    snapshot: &[Option<Arc<Stabilizer>>],
    gyro: &GyroSettings,
    offset_ms: f64,
    prop: &str,
) -> usize {
    let (need_smooth, need_zoom) = match prop {
        // 只影响每帧计算（写进 params 即生效）→ 一次预计算都不做
        "fov" | "frame_readout_time" | "frame_readout_enabled" | "gyro_readout_dir" => (false, false),
        "gyro_lens"
        | "gyro_zoom_mode"
        | "gyro_zoom_window"
        | "gyro_zoom_center_x"
        | "gyro_zoom_center_y"
        | "gyro_tr_x"
        | "gyro_tr_y"
        | "gyro_tr_z" => (false, true),
        _ => (true, true),
    };
    let mut n = 0usize;
    for s in snapshot.iter().flatten() {
        let m = &s.manager;
        apply_gyro_field(m, gyro, prop);
        if matches!(prop, "gyro_offset" | "gyro_offset_frames" | "") {
            apply_gyro_offset(m, offset_ms);
        }
        if need_smooth {
            m.recompute_smoothness();
        }
        if need_zoom {
            m.recompute_adaptive_zoom();
        }
        // 每帧参数（fov / 裁切 / 去畸变映射）必须刷新，否则改了视野也不会生效。
        m.recompute_undistortion();
        n += 1;
    }
    // 多段视频：重算后仍要保持接缝处的平滑连续性（与加载完成时做的是同一件事）。
    if n > 1 {
        apply_seam_warmup_smoothing(snapshot, no_progress());
    }
    n
}

/// Load and initialize the stabilizer for a video + settings. Runs the full pipeline:
/// probe → load video meta → load gyro data → preset calibration → settings → autosync → recompute.
///
/// `progress` receives 0.0..1.0 for THIS segment plus the current [`Phase`] (the caller
/// maps the fraction onto the overall multi-segment range). The motion-data slice is driven
/// by telemetry-parser's real byte-offset callback, so the bar moves with actual work.
#[allow(clippy::too_many_arguments)]
pub fn load_stabilizer(
    video: &str,
    gyro: &GyroSettings,
    width: usize,
    height: usize,
    fps: f64,
    duration_ms: f64,
    frame_count: usize,
    autosync: bool,
    offset_ms: f64,
    progress: Progress,
) -> Result<Stabilizer, GyroError> {
    if !std::path::Path::new(video).exists() {
        return Err(GyroError::NoVideo(video.to_string()));
    }
    let t_phase = std::time::Instant::now();
    progress(LoadProgress::new(0.0, Phase::Init));
    let _gpu_name = init_gpu();

    let mut stab = StabilizationManager::default();
    // 共享进程级镜头库（整库解析约 1.4 s / 12409 条）：每段视频/每次重建都重新解析太浪费，
    // GUI 的搜索框用的也是同一个实例。首次调用会把它解析出来（这里在 worker 线程里）。
    stab.lens_profile_db = crate::lens_db::shared();
    let stab = Arc::new(stab);

    *stab.input_file.write() = gyroflow_core::InputFile {
        url: path_to_url(video),
        project_file_url: None,
        image_sequence_fps: if fps > 0.0 { fps } else { 30.0 },
        image_sequence_start: 0,
        preset_name: None,
        preset_output_size: None,
    };
    stab.init_from_video_data(duration_ms, fps, frame_count, (width, height));
    stab.set_size(width, height);
    // Key: set output_size before recompute, otherwise output_width=1 → fov=2284.8 → all black.
    stab.set_output_size(width, height);
    progress(LoadProgress::new(PROG_INIT_END, Phase::MotionData));
    let t_init = t_phase.elapsed();

    {
        let md = std::fs::metadata(video).map_err(|e| GyroError::OpenVideo(e.to_string()))?;
        let mut f = File::open(video).map_err(|e| GyroError::OpenVideo(e.to_string()))?;
        // Real progress: telemetry-parser reports the file offset it has consumed while
        // walking the metadata ("none") track samples, so for a DJI container this is the
        // fraction of the file scanned for gyro data. gyroflow_core throttles it to ~10 Hz.
        stab.load_gyro_data(
            &mut f,
            md.len() as usize,
            &path_to_url(video),
            true,
            &FileLoadOptions::default(),
            |p| {
                progress(LoadProgress::new(
                    PROG_INIT_END + p.clamp(0.0, 1.0) * (PROG_LOAD_END - PROG_INIT_END),
                    Phase::MotionData,
                ))
            },
            Arc::new(AtomicBool::new(false)),
        )
        .map_err(|e| GyroError::LoadGyro(format!("{e:?}")))?;
        let gyro = stab.gyro.read();
        let md = gyro.file_metadata.read();
        log::info!(
            "gyro loaded: quats={} raw_imu={} has_motion={}",
            gyro.quaternions.len(),
            gyro.raw_imu(&md).len(),
            gyro.has_motion()
        );
        eprintln!(
            "[stab] per_frame_time_offsets len={} first5={:?} lens_params len={}",
            md.per_frame_time_offsets.len(),
            md.per_frame_time_offsets
                .iter()
                .take(5)
                .cloned()
                .collect::<Vec<_>>(),
            md.lens_params.len()
        );
    }
    progress(LoadProgress::new(PROG_LOAD_END, Phase::Preset));
    let t_load_done = t_phase.elapsed();

    // 预设只负责标定（+ 插值方式）；稳定化参数一律来自设置页，见下面的 apply_gyro_settings。
    if !gyro.preset_file_path.is_empty() && std::path::Path::new(&gyro.preset_file_path).exists() {
        apply_preset_calibration(&stab, &gyro.preset_file_path)?;
        eprintln!("[stab] preset calibration applied: {}", gyro.preset_file_path);
    } else {
        eprintln!(
            "[stab] preset SKIPPED (missing file): {}",
            gyro.preset_file_path
        );
    }

    // 搜索框选中的镜头标定（Gyroflow 镜头库 id，或用户给的 .json 路径）：**覆盖**预设里带的
    // `calibration_data` —— 这正是「换相机」时需要的。
    // 与原版 Gyroflow 载入镜头标定的行为一致，这里也顺带应用标定自带的
    // `frame_readout_time` / `frame_readout_direction`（它们是相机属性）。
    if !gyro.lens_profile.trim().is_empty() {
        match crate::lens_db::resolve_lens_json(&gyro.lens_profile) {
            Some(json) => {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&json) {
                    if let Some(t) = v.get("frame_readout_time").and_then(|x| x.as_f64()) {
                        if t.abs() > 0.0 {
                            stab.set_frame_readout_time(t);
                            if let Some(dir) =
                                v.get("frame_readout_direction").and_then(|x| x.as_str())
                            {
                                stab.set_frame_readout_direction(dir);
                            }
                        }
                    }
                }
                let tmp = std::env::temp_dir().join("tracereel_lens_selected.json");
                std::fs::write(&tmp, &json)
                    .map_err(|e| GyroError::Unknown(format!("write lens profile: {e}")))?;
                stab.load_lens_profile(&tmp.to_string_lossy())
                    .map_err(|e| GyroError::Unknown(format!("lens profile: {e:?}")))?;
                eprintln!("[stab] lens profile applied: {}", gyro.lens_profile);
            }
            None => eprintln!("[stab] lens profile NOT FOUND: {}", gyro.lens_profile),
        }
    }

    // 稳定化参数（设置页可调）：视野/变焦/镜头校正/地平线/卷帘/平滑算法与参数/附加旋转平移…
    apply_gyro_settings(&stab, gyro);

    // Manual gyro<->video time offset (ms). DJI data is GPS-synced (true offset ~0);
    // autosync used to find a spurious -3.6ms that amplified foot-strike jitter. A few
    // milliseconds of manual offset here fine-tunes the residual high-frequency shake.
    if offset_ms != 0.0 {
        eprintln!("[stab] applying manual offset {offset_ms:.3} ms");
        {
            let mut gyro = stab.gyro.write();
            gyro.set_offset(0, offset_ms);
            gyro.adjust_offsets();
        }
        stab.keyframes.write().update_gyro(&stab.gyro.read());
        stab.invalidate_zooming();
    }
    progress(LoadProgress::new(PROG_PRESET_END, Phase::Preset));
    let t_preset = t_phase.elapsed();

    let autosync_done = if autosync {
        progress(LoadProgress::new(PROG_PRESET_END, Phase::Autosync));
        run_autosync(&stab, video, fps, duration_ms)?
    } else {
        eprintln!("[stab] autosync DISABLED (assume 0 offset)");
        false
    };
    let t_sync = t_phase.elapsed();

    progress(LoadProgress::new(PROG_PRESET_END, Phase::Smoothing));
    stab.recompute_smoothness();
    progress(LoadProgress::new(PROG_SMOOTH_END, Phase::Zooming));
    let t_smooth = t_phase.elapsed();
    stab.recompute_adaptive_zoom();
    progress(LoadProgress::new(PROG_ZOOM_END, Phase::Undistortion));
    let t_zoom = t_phase.elapsed();
    stab.recompute_undistortion();
    progress(LoadProgress::new(1.0, Phase::Done));
    // 阶段耗时必须全部以同一个起点（t_phase）计量。这里以前用 `t_load`（读取运动数据前才创建，
    // 起点更晚）去算后面几个阶段，再拿它和以 t_phase 为起点的 `t_init` 相减 —— 只要「初始化」
    // （GPU + 镜头库 + init_from_video_data）比「读运动数据」慢，`Duration - Duration` 就会溢出
    // panic（core/time.rs:1277），加载线程连同刚建好的 Stabilizer 一起崩掉，界面于是显示
    // 「防抖: 加载失败」——**小视频/首个视频必现，大文件因为读数据耗时长反而侥幸正常**。
    // saturating_sub 是第二道保险：这类日志绝不能把加载线程带崩。
    let t_end = t_phase.elapsed();
    eprintln!(
        "[stab] phase timings: init {:.2}s | motion-data {:.2}s | preset {:.2}s | autosync {:.2}s | smooth {:.2}s | zoom {:.2}s | undistort {:.2}s",
        t_init.as_secs_f64(),
        t_load_done.saturating_sub(t_init).as_secs_f64(),
        t_preset.saturating_sub(t_load_done).as_secs_f64(),
        t_sync.saturating_sub(t_preset).as_secs_f64(),
        t_smooth.saturating_sub(t_sync).as_secs_f64(),
        t_zoom.saturating_sub(t_smooth).as_secs_f64(),
        t_end.saturating_sub(t_zoom).as_secs_f64(),
    );

    Ok(Stabilizer {
        manager: stab,
        width,
        height,
        fps,
        duration_ms,
        autosync_done,
    })
}

/// Run autosync (pure-Rust AKAZE + Almeida + rs_sync), feeding contiguous decoded
/// gray frames around the 50% sync point. Applies the found offsets to the gyro source.
fn run_autosync(
    stab: &Arc<StabilizationManager>,
    video: &str,
    fps: f64,
    dur_ms: f64,
) -> Result<bool, GyroError> {
    let cancel = Arc::new(AtomicBool::new(false));
    // Pure-Rust methods only: AKAZE flow (0), Almeida pose (1), rs_sync offset (2).
    let sp = SyncParams {
        every_nth_frame: 1,
        search_size: 5000.0,
        max_sync_points: 32,
        time_per_syncpoint: 1000.0,
        of_method: 0,
        pose_method: 1,
        offset_method: 2,
        auto_sync_points: true,
        ..Default::default()
    };
    let tfract = [0.5f64];
    let mut sync =
        AutosyncProcess::from_manager(stab, &tfract, sp, "normal".to_string(), cancel.clone())
            .map_err(|e| GyroError::AutosyncInit(format!("{e:?}")))?;

    let sync_done = Arc::new(AtomicBool::new(false));
    {
        let sd = sync_done.clone();
        let stab2 = stab.clone();
        sync.on_finished(move |r| {
            if let Some(offsets) = r.left() {
                log::info!("autosync found {} offsets", offsets.len());
                let mut gyro = stab2.gyro.write();
                gyro.prevent_recompute = true;
                for (ts_ms, off_ms, _cost) in offsets {
                    log::info!("autosync offset at {:.3}ms: {:.3}ms", ts_ms, off_ms);
                    let new_ts = ((ts_ms - off_ms) * 1000.0) as i64;
                    gyro.remove_offsets_near(new_ts, 100.0);
                    gyro.set_offset(new_ts, off_ms);
                }
                gyro.prevent_recompute = false;
                gyro.adjust_offsets();
                drop(gyro);
                stab2.keyframes.write().update_gyro(&stab2.gyro.read());
                stab2.invalidate_zooming();
            }
            sd.store(true, Ordering::SeqCst);
        });
    }

    let sync = Arc::new(std::sync::Mutex::new(sync));
    let v2 = video.to_string();
    let sd = sync_done.clone();
    let stab2 = stab.clone();
    std::thread::spawn(move || {
        let _ = feed_autosync_frames(&stab2, &sync, &v2, fps, dur_ms);
        sd.store(true, Ordering::SeqCst);
        if let Ok(s) = sync.lock() {
            s.finished_feeding_frames();
        }
    });

    // Wait up to 30s (autosync on a 23-min video takes several seconds).
    let wait = std::time::Instant::now();
    while !sync_done.load(Ordering::SeqCst) && wait.elapsed().as_secs() < 60 {
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    Ok(sync_done.load(Ordering::SeqCst))
}

#[allow(clippy::too_many_arguments)]
fn feed_autosync_frames(
    stab: &Arc<StabilizationManager>,
    sync: &Arc<std::sync::Mutex<AutosyncProcess>>,
    video: &str,
    fps: f64,
    dur_ms: f64,
) -> Result<(), String> {
    use ffmpeg_next::format::{input, Pixel};
    use ffmpeg_next::media::Type;
    use ffmpeg_next::software::scaling::{context::Context as SContext, flag::Flags};
    use ffmpeg_next::util::frame::video::Video;

    ffmpeg_next::init().map_err(|e| e.to_string())?;
    let mut ictx = input(video).map_err(|e| e.to_string())?;
    let st = ictx.streams().best(Type::Video).ok_or("no video")?;
    let idx = st.index();
    let cd = ffmpeg_next::codec::context::Context::from_parameters(st.parameters())
        .map_err(|e| e.to_string())?;
    let mut decoder = cd.decoder().video().map_err(|e| e.to_string())?;
    let mut scaler = SContext::get(
        decoder.format(),
        decoder.width(),
        decoder.height(),
        Pixel::GRAY8,
        640,
        360,
        Flags::BILINEAR,
    )
    .map_err(|e| e.to_string())?;
    let tb_f = st.time_base().numerator() as f64 / st.time_base().denominator() as f64;
    let frame_dt_ms = 1000.0 / fps;
    let mid = 0.5 * dur_ms;
    let start_s = ((mid - 31.0 * frame_dt_ms) / 1000.0).max(0.0);
    let end_s = (mid + 31.0 * frame_dt_ms) / 1000.0;
    // avformat_seek_file with stream_index=-1 uses AV_TIME_BASE (µs).
    let to = (start_s * 1_000_000.0) as i64;
    ictx.seek(to.min(ictx.duration()), ..to.min(ictx.duration()))
        .map_err(|e| e.to_string())?;

    let mut decoded = Video::empty();
    let mut nfed = 0usize;
    for (stream, packet) in ictx.packets() {
        if stream.index() != idx {
            continue;
        }
        if let Some(dts) = packet.dts() {
            if dts > (end_s / tb_f) as i64 {
                break;
            }
        }
        if decoder.send_packet(&packet).is_err() {
            continue;
        }
        while decoder.receive_frame(&mut decoded).is_ok() {
            let mut g = Video::empty();
            if scaler.run(&decoded, &mut g).is_err() {
                continue;
            }
            let mut gray = vec![0u8; 640 * 360];
            for y in 0..360 {
                let src = y * g.stride(0);
                let dst = y * 640;
                gray[dst..dst + 640].copy_from_slice(&g.data(0)[src..src + 640]);
            }
            if let Some(pts) = decoded.pts() {
                let t_s = pts as f64 * tb_f;
                let tms = t_s * 1000.0;
                let tus = (tms * 1000.0).round() as i64;
                let fi = (t_s * fps).round() as usize;
                if let Ok(s) = sync.lock() {
                    s.feed_frame(tus, fi, 640, 360, 640, &gray);
                }
                nfed += 1;
            }
            if nfed > 120 {
                break;
            }
        }
        if nfed > 120 {
            break;
        }
    }
    let _ = stab; // manager is pre-populated; the sync consumes its own refs
    log::info!("autosync feed: {nfed} frames");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nalgebra::Vector3;

    /// Quaternion track: X-axis rotation of `angle_deg` at each ts (µs, +step).
    fn track(start_us: i64, count: i64, step: i64, angle_deg: f64) -> TimeQuat {
        let mut m = TimeQuat::new();
        for i in 0..count {
            let q = Quat64::from_axis_angle(&Vector3::x_axis(), angle_deg.to_radians());
            m.insert(start_us + i * step, q);
        }
        m
    }

    #[test]
    fn warmup_head_maps_into_window_before_first_sample() {
        // prev spans 5 s (step 1 ms) ending at 4_999_000; cur starts at 10 s.
        let prev = track(0, 5000, 1000, 5.0);
        let cur = track(10_000_000, 5000, 1000, 5.0);
        let h = warmup_head(Some(&prev), &cur).expect("continuous seam");
        // 3 s window (inclusive endpoints): ts 1_999_000..=4_999_000.
        assert_eq!(h.len(), 3001);
        let (&lo, _) = h.iter().next().unwrap();
        let (&hi, _) = h.iter().next_back().unwrap();
        assert_eq!(hi, 10_000_000 - 1); // prev's last sample lands right before cur's first
        assert!(lo < 10_000_000 && hi < 10_000_000);
    }

    #[test]
    fn warmup_head_rejects_discontinuous_seam() {
        let prev = track(0, 5000, 1000, 0.0);
        let cur = track(10_000_000, 5000, 1000, 60.0); // 60° > 30° guard
        assert!(warmup_head(Some(&prev), &cur).is_none());
        assert!(warmup_head(None, &cur).is_none());
    }

    #[test]
    fn warmup_tail_maps_into_window_after_last_sample() {
        let cur = track(0, 5000, 1000, 5.0); // ends at 4_999_000
        let next = track(10_000_000, 5000, 1000, 5.0); // spans 10 s..15 s
        let t = warmup_tail(&cur, Some(&next)).expect("continuous seam");
        assert_eq!(t.len(), 3000); // next_lo..next_lo+3 s (upper bound exclusive)
        let (&lo, _) = t.iter().next().unwrap();
        let (&hi, _) = t.iter().next_back().unwrap();
        assert_eq!(lo, 4_999_000 + 1); // next's first sample lands right after cur's last
        assert!(hi > 4_999_000);
        // Discontinuity guard applies symmetrically.
        let bad = track(10_000_000, 5000, 1000, -60.0);
        assert!(warmup_tail(&cur, Some(&bad)).is_none());
    }
}
