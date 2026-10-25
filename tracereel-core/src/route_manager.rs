//! Route manager (ported from RouteManager.cpp / route_manager.py).
//!
//! Holds the route data, aligns/interpolates it to one-second intervals, computes
//! per-split camera transformations and the current runner position. This is pure
//! math — all visual/color values are retained as plain float triples so the UI can
//! render without needing Qt types here.
//!
//! The per-route computations are free functions operating on route *fields* rather
//! than methods borrowing `self`, so the Rust borrow-checker is satisfied while the
//! logic stays a 1:1 port of the original.

use crate::quickroute::{parse_quickroute, QuickRouteData};
use crate::route::RoutePoint;
use crate::splits::{build_runner_info, RunnerInfo, Split};

/// Route render mode (RENDER_MODE_*).
///
/// 原版还有 `Highlight`（编号 2）。本程序按需求只保留 **无 / 简洁 / 配速**，
/// 但沿用原版的编号（0 = 无、1 = 简洁、3 = 配速，2 号空出），这样同一个 `.orv`
/// 工程在原版 C++ OrientView 里打开时渲染模式仍然对得上。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderMode {
    None,
    Discreet,
    Pace,
}

/// 运动员在画面上的样式（本程序新增，原版只有圆形）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunnerStyle {
    /// 圆形（原版行为）。
    Circle,
    /// 箭头，指向当前运动方向（随路线转动）。
    Arrow,
}

/// Split view mode (VIEW_MODE_*).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    FixedSplit,
    RunnerCentered,
    RunnerCenteredSplitOriented,
}

/// A per-split camera transformation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SplitTransformation {
    pub x: f64,
    pub y: f64,
    pub angle: f64,
    pub angle_delta: f64,
    pub scale: f64,
}

impl Default for SplitTransformation {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            angle: 0.0,
            angle_delta: 0.0,
            scale: 1.0,
        }
    }
}

/// Config/visual properties applied to a route.
#[derive(Debug, Clone)]
pub struct RouteStyle {
    /// 路线颜色（简洁模式），RGBA 0..1；不透明度来自界面上的「路线透明度」。
    pub route_color: (f64, f64, f64, f64),
    /// 尾巴颜色（简洁模式），RGBA 0..1；不透明度来自「尾巴透明度」。
    pub tail_color: (f64, f64, f64, f64),
    pub route_render_mode: RenderMode,
    pub route_width: f64,
    /// 渲染范围：0 = 完整（整条路线），1 = 单段（只画当前所在的那一段）。
    /// 逐地图框独立：固定分段框常配「单段」，跟跑者居中框可配「完整」。
    pub render_scope: i32,
    pub tail_render_mode: RenderMode,
    pub tail_width: f64,
    pub tail_length: f64,
    /// 控制点颜色，RGBA 0..1（不透明度来自「控制点透明度」）。
    pub control_border_color: (f64, f64, f64, f64),
    pub control_radius: f64,
    pub control_border_width: f64,
    pub show_controls: bool,
    /// 运动员颜色，RGBA 0..1（不透明度来自「运动员透明度」）。
    pub runner_color: (f64, f64, f64, f64),
    pub runner_border_color: (f64, f64, f64),
    pub runner_radius: f64,
    pub runner_border_width: f64,
    pub runner_scale: f64,
    /// 运动员样式：圆形（原版）或箭头（跟着运动方向转）。
    pub runner_style: RunnerStyle,
    pub show_runner: bool,
    pub control_time_offset: f64,
    pub runner_time_offset: f64,
    pub user_scale: f64,
    /// 配速色带端点（分钟/公里）：`low_pace` = 慢端（红）、`high_pace` = 快端（绿）。
    pub low_pace: f64,
    pub high_pace: f64,
    /// 配速色带是否按**实际轨迹**的配速分布自适应（开启时 `low_pace`/`high_pace`
    /// 会在建好路线后被 `adaptive_pace_range` 覆盖）。
    pub pace_auto: bool,
    /// 尾巴独立的配速色带设置（慢端红色、快端绿色）。
    pub tail_low_pace: f64,
    pub tail_high_pace: f64,
    pub tail_pace_auto: bool,
}

impl Default for RouteStyle {
    fn default() -> Self {
        Self {
            route_color: (0.0, 0.0, 0.0, 80.0 / 255.0),
            tail_color: (0.0, 0.0, 0.0, 80.0 / 255.0),
            route_render_mode: RenderMode::Discreet,
            route_width: 10.0,
            render_scope: 0,
            tail_render_mode: RenderMode::None,
            tail_width: 10.0,
            tail_length: 60.0,
            control_border_color: (0.0, 0.0, 0.0, 1.0),
            control_radius: 15.0,
            control_border_width: 5.0,
            show_controls: true,
            runner_color: (1.0, 1.0, 1.0, 1.0),
            runner_border_color: (0.0, 0.0, 0.0),
            runner_radius: 10.0,
            runner_border_width: 1.0,
            runner_scale: 1.0,
            runner_style: RunnerStyle::Circle,
            show_runner: true,
            control_time_offset: 0.0,
            runner_time_offset: 0.0,
            user_scale: 1.0,
            low_pace: 15.0,
            high_pace: 5.0,
            pace_auto: true,
            tail_low_pace: 15.0,
            tail_high_pace: 5.0,
            tail_pace_auto: true,
        }
    }
}

/// A route with its aligned points and computed data.
#[derive(Debug, Clone)]
pub struct Route {
    pub route_points: Vec<RoutePoint>,
    pub aligned_route_points: Vec<RoutePoint>,
    /// `index -> (position_x, position_y)` control positions.
    pub control_positions: Vec<(f64, f64)>,
    pub split_transformations: Vec<SplitTransformation>,
    pub runner_position: (f64, f64),
    pub style: RouteStyle,
    pub control_time_offset: f64,
    pub runner_time_offset: f64,
}

impl Default for Route {
    fn default() -> Self {
        Self {
            route_points: Vec::new(),
            aligned_route_points: Vec::new(),
            control_positions: Vec::new(),
            split_transformations: Vec::new(),
            runner_position: (0.0, 0.0),
            style: RouteStyle::default(),
            control_time_offset: 0.0,
            runner_time_offset: 0.0,
        }
    }
}

/// Exponential moving average with a measurement-time decay.
#[derive(Debug, Clone)]
struct MovingAverage {
    alpha: f64,
    value: f64,
    initialized: bool,
}

impl MovingAverage {
    fn new(alpha: f64) -> Self {
        Self {
            alpha,
            value: 0.0,
            initialized: false,
        }
    }
    fn reset(&mut self, value: f64) {
        self.value = value;
        self.initialized = true;
    }
    fn add_measurement(&mut self, measurement: f64, delta_time_ms: f64) {
        if !self.initialized {
            self.value = measurement;
            self.initialized = true;
            return;
        }
        let effective = (self.alpha * delta_time_ms.max(1.0)).min(1.0);
        self.value = self.value * (1.0 - effective) + measurement * effective;
    }
    fn get_average(&self) -> f64 {
        self.value
    }
}

/// The route manager that computes the camera transform for a given time.
#[derive(Debug, Clone)]
pub struct RouteManager {
    pub view_mode: ViewMode,
    pub use_smooth_split_transition: bool,
    pub smooth_split_transition_speed: f64,
    pub top_bottom_margin: f64,
    pub left_right_margin: f64,
    pub maximum_automatic_zoom: f64,
    /// 跟跑者居中的两个模式没有「取景边距」的概念（画面始终围着跑者），地图远近由这个
    /// 缩放倍数决定：1.0 = 地图坐标 1:1 显示。固定分段模式用不到它（缩放由分段取景表算）。
    pub centered_zoom: f64,
    pub runner_vertical_offset: f64,
    pub runner_averaging_factor: f64,
    pub window_width: f64,
    pub window_height: f64,
    pub map_relative_width: f64,
    /// 圆形地图框的取景半径（窗口像素，圆心 = 画面中心）；0 = 矩形取景。
    ///
    /// 圆形地图的可见区域是内切圆，所以分段取景不能只按窗口宽高来收：那样轨迹会贴到
    /// 矩形边（= 圆外）被裁掉。这里再按「当前分段所有点到段中点的最远距离」额外收一次，
    /// 保证整段轨迹（含边距）都落在圆内。
    pub circular_framing_radius: f64,

    pub routes: Vec<Route>,
    pub runner_info: RunnerInfo,

    pub full_update_requested: bool,
    pub instant_split_transition_requested: bool,
    smooth_split_transition_in_progress: bool,
    smooth_split_transition_alpha: f64,

    pub current_st: SplitTransformation,
    previous_st: SplitTransformation,
    next_st: SplitTransformation,
    pub current_split_transformation_index: i32,

    runner_average_x: MovingAverage,
    runner_average_y: MovingAverage,
    runner_average_angle: MovingAverage,
}

/// 跑者平均因子：镜头跟随跑者的平滑强度，等价时间常数 ≈ `1000 / 该值` 毫秒
/// （见 `MovingAverage`）。0.5 → 约 2 秒，够稳且不拖；用户不需要调整，
/// 所以固定成这个值，界面上也不再提供输入框。
pub const RUNNER_AVERAGING_FACTOR: f64 = 0.5;

impl Default for RouteManager {
    fn default() -> Self {
        Self {
            view_mode: ViewMode::FixedSplit,
            use_smooth_split_transition: true,
            smooth_split_transition_speed: 1.0,
            top_bottom_margin: 30.0,
            left_right_margin: 10.0,
            maximum_automatic_zoom: 100.0,
            centered_zoom: 1.0,
            runner_vertical_offset: 0.0,
            runner_averaging_factor: RUNNER_AVERAGING_FACTOR,
            window_width: 1280.0,
            window_height: 720.0,
            map_relative_width: 0.25,
            circular_framing_radius: 0.0,
            routes: vec![Route::default()],
            runner_info: RunnerInfo::default(),
            full_update_requested: true,
            instant_split_transition_requested: true,
            smooth_split_transition_in_progress: false,
            smooth_split_transition_alpha: 0.0,
            current_st: SplitTransformation::default(),
            previous_st: SplitTransformation::default(),
            next_st: SplitTransformation::default(),
            current_split_transformation_index: -1,
            runner_average_x: MovingAverage::new(0.0),
            runner_average_y: MovingAverage::new(0.0),
            runner_average_angle: MovingAverage::new(0.0),
        }
    }
}

impl RouteManager {
    /// Initialize from a QuickRoute jpeg + map dims + split-time string + style.
    pub fn initialize(
        &mut self,
        jpeg_path: &str,
        map_width: f64,
        map_height: f64,
        split_times: &str,
        split_relative: bool,
        style: RouteStyle,
        view_mode: ViewMode,
        map_relative_width: f64,
    ) -> Result<(), String> {
        if !std::path::Path::new(jpeg_path).exists() {
            return Err(format!("QuickRoute jpeg not found: {jpeg_path}"));
        }
        let data: QuickRouteData = {
            let bytes =
                std::fs::read(jpeg_path).map_err(|e| format!("read QuickRoute jpeg: {e}"))?;
            parse_quickroute(&bytes, map_width, map_height).map_err(|e| e.to_string())?
        };
        self.set_route_points(
            data.route_points,
            split_times,
            split_relative,
            style,
            view_mode,
            map_relative_width,
        );
        Ok(())
    }

    /// Initialize from route points that were already parsed elsewhere.
    ///
    /// This is the entry point for `.qrt` projects: their waypoints are recovered
    /// from the object graph by `crate::qrt`, then converted with
    /// [`crate::qrt::QuickRouteProject::to_route_points`].
    pub fn initialize_from_points(
        &mut self,
        route_points: Vec<RoutePoint>,
        split_times: &str,
        split_relative: bool,
        style: RouteStyle,
        view_mode: ViewMode,
        map_relative_width: f64,
    ) {
        self.set_route_points(
            route_points,
            split_times,
            split_relative,
            style,
            view_mode,
            map_relative_width,
        );
    }

    /// Shared body of both initializers: install the points and rebuild every
    /// derived structure (aligned points, control positions, split cameras).
    fn set_route_points(
        &mut self,
        route_points: Vec<RoutePoint>,
        split_times: &str,
        split_relative: bool,
        style: RouteStyle,
        view_mode: ViewMode,
        map_relative_width: f64,
    ) {
        self.view_mode = view_mode;
        self.map_relative_width = map_relative_width;
        self.runner_info = build_runner_info(split_times, split_relative);
        let splits = self.runner_info.splits.clone();

        let route = &mut self.routes[0];
        route.style = style.clone();
        route.route_points = route_points;
        route.control_time_offset = style.control_time_offset;
        route.runner_time_offset = style.runner_time_offset;

        route.aligned_route_points = calc_aligned_route_points(&route.route_points);
        route.control_positions = calc_control_positions(
            &route.aligned_route_points,
            &splits,
            route.control_time_offset,
        );
        route.split_transformations = calc_split_transformations(
            &route.aligned_route_points,
            &route.route_points,
            &splits,
            route.control_time_offset,
            self.window_width,
            self.window_height,
            self.map_relative_width,
            self.left_right_margin,
            self.top_bottom_margin,
            self.maximum_automatic_zoom,
            self.circular_framing_radius,
        );

        let a = self.runner_averaging_factor / 1000.0;
        self.runner_average_x = MovingAverage::new(a);
        self.runner_average_y = MovingAverage::new(a);
        self.runner_average_angle = MovingAverage::new(a);
        self.instant_split_transition_requested = true;
        self.current_split_transformation_index = -1;

        // `update()` 里的整表重算负责"第一帧落在分段之外就先按第 0 段取景"。
        self.update(0.0, 0.0);
    }

    /// Update during playback/export. Following the runner uses content time,
    /// while a fixed-split camera transition uses presentation time so its
    /// rotation, pan and zoom keep the same duration at any playback speed.
    pub fn update_playback(&mut self, current_time: f64, content_step_ms: f64, speed: f64) {
        let speed = if speed.is_finite() && speed > 0.0 { speed } else { 1.0 };
        let camera_step = if self.view_mode == ViewMode::FixedSplit {
            content_step_ms / speed
        } else {
            content_step_ms
        };
        self.update(current_time, camera_step);
    }

    /// Advance the manager to `current_time`, recomputing the camera transform.
    pub fn update(&mut self, current_time: f64, frame_time_ms: f64) {
        if self.full_update_requested {
            let splits = self.runner_info.splits.clone();
            for route in &mut self.routes {
                route.control_positions = calc_control_positions(
                    &route.aligned_route_points,
                    &splits,
                    route.control_time_offset,
                );
                route.split_transformations = calc_split_transformations(
                    &route.aligned_route_points,
                    &route.route_points,
                    &splits,
                    route.control_time_offset,
                    self.window_width,
                    self.window_height,
                    self.map_relative_width,
                    self.left_right_margin,
                    self.top_bottom_margin,
                    self.maximum_automatic_zoom,
                    self.circular_framing_radius,
                );
            }
            self.full_update_requested = false;
            // 原版 `calculateSplitTransformations()` 结尾就设了这面旗：整表重算之后
            // （窗口尺寸变化、路线/分段变化）当前取景必须**立刻**换成新表里的那一项。
            // 少了它，`current_st` 会一直停在重算前的那份——预览面板尺寸确定后，
            // 第一段就会用初始窗口尺寸算出来的取景，怎么播都不对，只有切到别的
            // 分段（索引变化触发切换）或来回跳转才会被纠正。
            self.instant_split_transition_requested = true;
            // 再把原版 `initialize()` 末尾那一手扩成"整表重算后就地补一次"：
            // 如果当前这一帧落在所有分段之外（视频起点早于出发时刻，`find_index`
            // 取不到任何一段），上面那面旗就没有落点，按旧窗口尺寸算出来的取景会
            // 一直留到出发时刻。这里按已有的索引（没有就用第 0 段）先把新表应用上。
            if self.view_mode == ViewMode::FixedSplit {
                let n = self.routes[0].split_transformations.len();
                if n > 0 {
                    let i = self.current_split_transformation_index;
                    let pick = if i < 0 {
                        0
                    } else if (i as usize) >= n {
                        n - 1
                    } else {
                        i as usize
                    };
                    self.current_st = self.routes[0].split_transformations[pick];
                    self.current_split_transformation_index = pick as i32;
                }
            }
        }
        let idx = 0;
        let rp = interp_route_point(
            &self.routes[idx].aligned_route_points,
            current_time + self.routes[idx].runner_time_offset,
        );
        self.routes[idx].runner_position = (rp.position_x, rp.position_y);
        self.calculate_current_split_transformation(idx, current_time, frame_time_ms);
    }

    fn calculate_current_split_transformation(
        &mut self,
        route_index: usize,
        current_time: f64,
        frame_time_ms: f64,
    ) {
        // Snapshot the values we need so we can mutate self without a conflicting borrow.
        let (sts, aligned, cto, rto) = {
            let route = &self.routes[route_index];
            (
                route.split_transformations.clone(),
                route.aligned_route_points.clone(),
                route.control_time_offset,
                route.runner_time_offset,
            )
        };
        match self.view_mode {
            ViewMode::FixedSplit => {
                if let Some(index) =
                    find_index(&sts, &self.runner_info.splits, cto, rto, current_time)
                {
                    if self.instant_split_transition_requested {
                        self.current_st = sts[index];
                        self.current_split_transformation_index = index as i32;
                        self.instant_split_transition_requested = false;
                        self.smooth_split_transition_in_progress = false;
                    } else if (index as i32) != self.current_split_transformation_index {
                        if self.use_smooth_split_transition {
                            self.previous_st = self.current_st;
                            self.next_st = sts[index];
                            self.smooth_split_transition_alpha = 0.0;
                            self.smooth_split_transition_in_progress = true;
                            let angle_delta = self.next_st.angle - self.previous_st.angle;
                            let abs_delta = angle_delta.abs();
                            let mut final_delta = angle_delta;
                            if abs_delta > 180.0 {
                                final_delta = 360.0 - abs_delta;
                                final_delta *= if angle_delta < 0.0 { 1.0 } else { -1.0 };
                            }
                            self.previous_st.angle_delta = final_delta;
                        } else {
                            self.current_st = sts[index];
                        }
                        self.current_split_transformation_index = index as i32;
                    }
                }
                if self.smooth_split_transition_in_progress {
                    if self.smooth_split_transition_alpha > 1.0 {
                        self.current_st = self.next_st;
                        self.smooth_split_transition_in_progress = false;
                    } else {
                        let mut alpha = self.smooth_split_transition_alpha;
                        alpha = alpha * alpha * alpha * (alpha * (alpha * 6.0 - 15.0) + 10.0);
                        self.current_st.x =
                            (1.0 - alpha) * self.previous_st.x + alpha * self.next_st.x;
                        self.current_st.y =
                            (1.0 - alpha) * self.previous_st.y + alpha * self.next_st.y;
                        self.current_st.angle =
                            self.previous_st.angle + alpha * self.previous_st.angle_delta;
                        self.current_st.scale =
                            (1.0 - alpha) * self.previous_st.scale + alpha * self.next_st.scale;
                        self.smooth_split_transition_alpha +=
                            self.smooth_split_transition_speed * frame_time_ms / 1000.0;
                    }
                }
            }
            ViewMode::RunnerCentered | ViewMode::RunnerCenteredSplitOriented => {
                let rp = interp_route_point(&aligned, current_time + rto);
                let mut x = -rp.position_x;
                let mut y = rp.position_y;
                let mut angle = rp.orientation;
                if self.view_mode == ViewMode::RunnerCenteredSplitOriented {
                    if let Some(index) =
                        find_index(&sts, &self.runner_info.splits, cto, rto, current_time)
                    {
                        angle = sts[index].angle;
                    }
                }
                x -= (angle.to_radians()).sin() * -self.runner_vertical_offset;
                y -= (angle.to_radians()).cos() * -self.runner_vertical_offset;
                if self.instant_split_transition_requested {
                    self.runner_average_x.reset(x);
                    self.runner_average_y.reset(y);
                    self.runner_average_angle.reset(angle);
                    self.instant_split_transition_requested = false;
                } else {
                    self.runner_average_x.add_measurement(x, frame_time_ms);
                    self.runner_average_y.add_measurement(y, frame_time_ms);
                    self.runner_average_angle
                        .add_measurement(angle, frame_time_ms);
                }
                self.current_st.x = self.runner_average_x.get_average();
                self.current_st.y = self.runner_average_y.get_average();
                self.current_st.angle = self.runner_average_angle.get_average();
                // 居中模式不按分段收景：缩放只认用户设置的倍数（取景边距在这里没有意义）。
                self.current_st.scale = self.centered_zoom.max(1e-4);
            }
        }
    }

    /// Tail route points between `[current+offset-tailLength, current+offset]`.
    ///
    /// 返回**完整的插值点**（含 `pace`）：`calculate_tail_path` 只取坐标画「简洁」尾巴，
    /// 「配速」尾巴需要按每个点的配速着色，所以两个模式共用这一份取点逻辑。
    pub fn calculate_tail_points(
        &self,
        route_index: usize,
        current_time: f64,
    ) -> Vec<RoutePoint> {
        let Some(route) = self.routes.get(route_index) else {
            return Vec::new();
        };
        let pts = &route.aligned_route_points;
        if pts.is_empty() {
            return Vec::new();
        }
        let offset_time = current_time + route.runner_time_offset;
        let start_time = offset_time - route.style.tail_length;
        let end_time = offset_time;
        let index_max = pts.len() - 1;
        let start_index = (start_time.floor() as i64).clamp(0, index_max as i64) as usize;
        let end_index = (end_time.floor() as i64).clamp(0, index_max as i64) as usize;
        if start_index == end_index {
            return Vec::new();
        }
        let mut points = vec![interp_route_point(pts, start_time)];
        points.extend_from_slice(&pts[(start_index + 1)..end_index]);
        points.push(interp_route_point(pts, end_time));
        points
    }

    /// Tail path points between `[current+offset-tailLength, current+offset]`.
    pub fn calculate_tail_path(&self, route_index: usize, current_time: f64) -> Vec<(f64, f64)> {
        self.calculate_tail_points(route_index, current_time)
            .iter()
            .map(|rp| (rp.position_x, rp.position_y))
            .collect()
    }

    /// 运动员当前的前进方向，返回**路线坐标系**里的单位向量 `(dx, dy)`。
    ///
    /// 用 `current_time ± dt` 两个插值点做中心差分（比拿相邻路线点稳，对齐后的点每秒一个，
    /// 而差分窗口可以与画笔/半径无关）；原地不动（起点、终点、轨迹停顿时）退回插值点自带的
    /// `orientation`：`quickroute.rs` 算的是 `a = 90° + atan2(dy, dx)`，因此
    /// `(dx, dy) ∝ (sin a, -cos a)`。
    pub fn runner_direction(&self, route_index: usize, current_time: f64, dt: f64) -> (f64, f64) {
        let Some(route) = self.routes.get(route_index) else {
            return (1.0, 0.0);
        };
        let aligned = &route.aligned_route_points;
        if aligned.is_empty() {
            return (1.0, 0.0);
        }
        let t = current_time + route.runner_time_offset;
        let dt = dt.abs().max(0.001);
        let a = interp_route_point(aligned, t - dt);
        let b = interp_route_point(aligned, t + dt);
        let (dx, dy) = (b.position_x - a.position_x, b.position_y - a.position_y);
        let len = (dx * dx + dy * dy).sqrt();
        if len > 1e-9 {
            return (dx / len, dy / len);
        }
        // 停住/只剩一个点：用轨迹自带的航向角兜底。
        let rp = interp_route_point(aligned, t);
        let rad = rp.orientation.to_radians();
        let (sx, sy) = (rad.sin(), -rad.cos());
        if sx.abs() + sy.abs() > 1e-9 {
            (sx, sy)
        } else {
            (1.0, 0.0)
        }
    }

    pub fn get_x(&self) -> f64 {
        self.current_st.x
    }
    pub fn get_y(&self) -> f64 {
        self.current_st.y
    }
    pub fn get_scale(&self) -> f64 {
        self.current_st.scale
    }
    pub fn get_angle(&self) -> f64 {
        self.current_st.angle
    }
    /// Total route duration in seconds (time of the last aligned route point). Used for
    /// map-only exports (no video) to size the timeline when the user leaves end = 0.
    pub fn duration_s(&self) -> f64 {
        self.routes
            .get(0)
            .and_then(|r| r.aligned_route_points.last())
            .map(|p| p.time.max(0.0))
            .unwrap_or(0.0)
    }
    pub fn get_view_mode(&self) -> ViewMode {
        self.view_mode
    }
    pub fn set_view_mode(&mut self, mode: ViewMode) {
        self.view_mode = mode;
        self.instant_split_transition_requested = true;
    }
    /// 就地更新「跑者平均因子」：重建三个位置均值器（与 `set_route_points` 里同一套
    /// `factor/1000` 算法）。只改显示参数的轻量更新靠它立刻生效，不必整链重载。
    pub fn set_runner_averaging_factor(&mut self, factor: f64) {
        self.runner_averaging_factor = factor;
        let a = factor / 1000.0;
        self.runner_average_x = MovingAverage::new(a);
        self.runner_average_y = MovingAverage::new(a);
        self.runner_average_angle = MovingAverage::new(a);
    }
    /// Ask for a full recomputation of the derived tables on the next `update()` (aligned
    /// points are assumed to already be final). Mirrors the original
    /// `RouteManager::requestFullUpdate()`; used after the map bitmap has been downscaled
    /// and the route geometry rescaled into the new pixel space, so that the *cached*
    /// `current_st` is rebuilt from the rescaled coordinates instead of keeping the values
    /// computed before the rescale.
    pub fn request_full_update(&mut self) {
        self.full_update_requested = true;
    }
    pub fn window_resized(&mut self, w: f64, h: f64) {
        self.window_width = w;
        self.window_height = h;
        self.full_update_requested = true;
    }
}

/// 从**实际轨迹**的逐点配速里取自适应色带端点，返回 `(low_pace, high_pace)`。
///
/// 语义与原版 `interpolateFromGreenToRed(highPace, lowPace, pace)` 一致：`low_pace` 是
/// **慢端**（红）、`high_pace` 是**快端**（绿）。取 **P90 = 慢端 / P10 = 快端**，让常见配速
/// 落在色带中段、把极端段（爬坡、冲刺）留给两端；直接用 min/max 的话一个 GPS 跳点就能把
/// 整条色带压扁。只统计 `pace > 0` 且有限的点：没有 GPS 配速的「地图 + 线路」工程整条为 0，
/// 返回 `None`，调用方保持原设定。
pub fn adaptive_pace_range(points: &[RoutePoint]) -> Option<(f64, f64)> {
    let mut paces: Vec<f64> = points
        .iter()
        .map(|p| p.pace)
        .filter(|p| p.is_finite() && *p > 0.0)
        .collect();
    if paces.is_empty() {
        return None;
    }
    paces.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let quantile = |q: f64| -> f64 {
        let idx = (q * (paces.len() - 1) as f64).round() as usize;
        paces[idx.min(paces.len() - 1)]
    };
    let (fast, slow) = (quantile(0.10), quantile(0.90));
    // 匀速跑（快慢差 < 0.5 分钟/公里）时给色带一点跨度，否则整条路线一个颜色、看不出快慢。
    if slow - fast <= 0.5 {
        let mid = (slow + fast) / 2.0;
        return Some((mid + 0.25, (mid - 0.25).max(0.05)));
    }
    Some((slow, fast))
}

/// Align and interpolate route points to one-second intervals (exact port).
///
/// The "current" point for the next segment must be the alpha=1 endpoint (position
/// interpolated to the next route point), otherwise the aligned path lags one step
/// per segment and the split bounding boxes / automatic zoom come out wrong.
fn calc_aligned_route_points(route_points: &[RoutePoint]) -> Vec<RoutePoint> {
    if route_points.len() < 2 {
        return Vec::new();
    }
    let mut aligned_time = 0.0;
    let mut current_rp = route_points[0];
    let mut aligned = Vec::new();
    let mut aligned_rp = RoutePoint::default();
    let mut i = 0usize;
    while i < route_points.len() - 1 {
        let mut next_index = 0usize;
        for j in (i + 1)..route_points.len() {
            if route_points[j].time - current_rp.time > 1.0 {
                next_index = j;
                break;
            }
        }
        if next_index <= i {
            break;
        }
        i = next_index;
        let next_rp = route_points[next_index];
        let time_delta = next_rp.time - current_rp.time;
        let alpha_step = if time_delta > 0.0 {
            1.0 / time_delta
        } else {
            0.0
        };
        let mut alpha = 0.0;
        let step_count = time_delta as usize;
        for k in 0..=step_count {
            aligned_rp.time = aligned_time;
            aligned_rp.position_x =
                (1.0 - alpha) * current_rp.position_x + alpha * next_rp.position_x;
            aligned_rp.position_y =
                (1.0 - alpha) * current_rp.position_y + alpha * next_rp.position_y;
            aligned_rp.elevation = (1.0 - alpha) * current_rp.elevation + alpha * next_rp.elevation;
            aligned_rp.heart_rate =
                (1.0 - alpha) * current_rp.heart_rate + alpha * next_rp.heart_rate;
            aligned_rp.pace = (1.0 - alpha) * current_rp.pace + alpha * next_rp.pace;
            aligned_rp.orientation =
                (1.0 - alpha) * current_rp.orientation + alpha * next_rp.orientation;
            alpha += alpha_step;
            if k < step_count {
                aligned.push(RoutePoint {
                    time: aligned_rp.time,
                    position_x: aligned_rp.position_x,
                    position_y: aligned_rp.position_y,
                    elevation: aligned_rp.elevation,
                    heart_rate: aligned_rp.heart_rate,
                    pace: aligned_rp.pace,
                    orientation: aligned_rp.orientation,
                    ..RoutePoint::default()
                });
                aligned_time += 1.0;
            }
        }
        current_rp = RoutePoint {
            time: aligned_rp.time,
            position_x: aligned_rp.position_x,
            position_y: aligned_rp.position_y,
            elevation: aligned_rp.elevation,
            heart_rate: aligned_rp.heart_rate,
            pace: aligned_rp.pace,
            orientation: aligned_rp.orientation,
            date_time_ms: next_rp.date_time_ms,
            coordinate_x: next_rp.coordinate_x,
            coordinate_y: next_rp.coordinate_y,
            ..RoutePoint::default()
        };
    }
    if !aligned.is_empty() {
        aligned.push(RoutePoint {
            time: aligned_rp.time,
            position_x: aligned_rp.position_x,
            position_y: aligned_rp.position_y,
            elevation: aligned_rp.elevation,
            heart_rate: aligned_rp.heart_rate,
            pace: aligned_rp.pace,
            orientation: aligned_rp.orientation,
            ..RoutePoint::default()
        });
    }
    aligned
}

fn calc_control_positions(aligned: &[RoutePoint], splits: &[Split], cto: f64) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    if aligned.is_empty() {
        return out;
    }
    for split in splits {
        let rp = interp_route_point(aligned, split.absolute_time + cto);
        out.push((rp.position_x, rp.position_y));
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn calc_split_transformations(
    aligned: &[RoutePoint],
    original: &[RoutePoint],
    splits: &[Split],
    cto: f64,
    window_width: f64,
    window_height: f64,
    map_relative_width: f64,
    left_right_margin: f64,
    top_bottom_margin: f64,
    max_auto_zoom: f64,
    circular_framing_radius: f64,
) -> Vec<SplitTransformation> {
    let mut out = Vec::new();
    if splits.len() < 2 || aligned.is_empty() {
        return out;
    }
    let index_max = aligned.len() - 1;
    for i in 0..splits.len() - 1 {
        let split1 = splits[i];
        let split2 = splits[i + 1];
        let start_index =
            ((split1.absolute_time + cto).round() as i64).clamp(0, index_max as i64) as usize;
        let stop_index =
            ((split2.absolute_time + cto).round() as i64).clamp(0, index_max as i64) as usize;

        let mut st = SplitTransformation::default();
        if split2.absolute_time > split1.absolute_time {
            let start_time = split1.absolute_time + cto;
            let stop_time = split2.absolute_time + cto;
            let start_rp = interp_route_point(aligned, start_time);
            let stop_rp = interp_route_point(aligned, stop_time);
            let dx = stop_rp.position_x - start_rp.position_x;
            let dy = stop_rp.position_y - start_rp.position_y;
            let mut angle = (-dy).atan2(dx).to_degrees();
            angle = 90.0 - angle;
            if angle > 180.0 {
                angle -= 360.0;
            }
            let rad = -angle.to_radians();
            let rot = |px: f64, py: f64| -> (f64, f64) {
                (
                    px * rad.cos() - py * rad.sin(),
                    px * rad.sin() + py * rad.cos(),
                )
            };

            let mut min_x = f64::INFINITY;
            let mut max_x = f64::NEG_INFINITY;
            let mut min_y = f64::INFINITY;
            let mut max_y = f64::NEG_INFINITY;
            // Include both the displayed interpolated path and the original samples:
            // sub-second bends must not disappear from the framing calculation.
            let points: Vec<_> = std::iter::once(start_rp)
                .chain(std::iter::once(stop_rp))
                .chain(aligned[start_index..=stop_index].iter().copied())
                .chain(original.iter().filter(|p| p.time >= start_time && p.time <= stop_time).copied())
                .collect();
            for p in &points {
                let (rx, ry) = rot(p.position_x, p.position_y);
                min_x = min_x.min(rx);
                max_x = max_x.max(rx);
                min_y = min_y.min(ry);
                max_y = max_y.max(ry);
            }
            // Center on the whole leg's rotated bounds, rather than its controls'
            // midpoint. A detour can extend well beyond either control.
            let center_rx = (min_x + max_x) / 2.0;
            let center_ry = (min_y + max_y) / 2.0;
            let mid_x = center_rx * rad.cos() + center_ry * rad.sin();
            let mid_y = -center_rx * rad.sin() + center_ry * rad.cos();
            let split_width = (max_x - min_x) + 2.0 * left_right_margin;
            let split_height = (max_y - min_y) + 2.0 * top_bottom_margin;

            let scale_x = if split_width > 0.0 {
                (window_width * map_relative_width) / split_width
            } else {
                1.0
            };
            let scale_y = if split_height > 0.0 {
                window_height / split_height
            } else {
                1.0
            };
            let final_scale = scale_x.min(scale_y).min(max_auto_zoom);

            // 圆形地图框：可见区域是内切圆（圆心 = 画面中心 = 整段轨迹包围盒的中心）。
            // 只按窗口宽高收景会让轨迹贴到矩形边、被圆裁掉，所以再按「这一段里离取景中心最远
            // 的点」收一次，让整段轨迹连边距一起留在圆内。旋转 / 镜像不改变到中心的距离，
            // 直接在路线坐标里量即可。
            let final_scale = if circular_framing_radius > 0.0 {
                let mut radius_map = 0.0_f64;
                for p in &points {
                    let dx = p.position_x - mid_x;
                    let dy = p.position_y - mid_y;
                    radius_map = radius_map.max((dx * dx + dy * dy).sqrt());
                }
                if radius_map > 1e-9 {
                    let margin = left_right_margin.max(top_bottom_margin);
                    final_scale.min(circular_framing_radius / (radius_map + margin))
                } else {
                    final_scale
                }
            } else {
                final_scale
            };

            st.x = -mid_x;
            st.y = mid_y;
            st.angle = angle;
            st.scale = final_scale;
        }
        out.push(st);
    }
    out
}

/// Interpolate a route point at an arbitrary time from the aligned (1s) points.
fn interp_route_point(aligned: &[RoutePoint], time: f64) -> RoutePoint {
    if aligned.is_empty() {
        return RoutePoint::default();
    }
    // Clamp the queried time to the track's valid range. For times before the track
    // starts (recorded before departure) the runner must sit at the first point and NOT
    // oscillate: with a negative time, floor() stays negative and alpha still advances 0..1
    // and wraps each whole second, so an unclamped interp would saw (back-and-forth jitter).
    let index_max = aligned.len() - 1;
    let time = time.max(0.0).min(index_max as f64);
    let prev_whole = time.floor();
    let alpha = time - prev_whole;
    let first = (prev_whole as i64).clamp(0, index_max as i64) as usize;
    let second = ((first + 1) as i64).clamp(0, index_max as i64) as usize;
    if first == second {
        return aligned[first];
    }
    RoutePoint::lerp(&aligned[first], &aligned[second], alpha)
}

/// Find the split transformation index for a given time, or None.
fn find_index(
    sts: &[SplitTransformation],
    splits: &[Split],
    cto: f64,
    rto: f64,
    current_time: f64,
) -> Option<usize> {
    for (i, split) in splits.iter().enumerate() {
        if i + 1 >= splits.len() {
            break;
        }
        let first_off = split.absolute_time + cto;
        let second_off = splits[i + 1].absolute_time + cto;
        let runner_off = current_time + rto;
        if second_off > first_off && first_off <= runner_off && runner_off < second_off {
            if i >= sts.len() {
                return None;
            }
            return Some(i);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aligns_and_lerps_points() {
        // Two points 2 seconds apart -> 3 aligned points at 0s, 1s, 2s.
        let mut a = RoutePoint::new(0.0, 0.0, 0.0);
        a.time = 0.0;
        let mut b = RoutePoint::new(100.0, 50.0, 0.0);
        b.time = 2.0;
        let aligned = calc_aligned_route_points(&[a, b]);
        assert_eq!(aligned.len(), 3);
        assert_eq!(aligned[0].time, 0.0);
        assert_eq!(aligned[1].time, 1.0);
        assert!((aligned[1].position_x - 50.0).abs() < 1e-6);
        assert_eq!(aligned[2].time, 2.0);
    }

    #[test]
    fn calculates_split_camera() {
        // A straight horizontal segment; the split scale must frame it and center mid.
        let mut a = RoutePoint::new(0.0, 0.0, 0.0);
        a.time = 0.0;
        let mut b = RoutePoint::new(100.0, 0.0, 0.0);
        b.time = 2.0;
        let aligned = calc_aligned_route_points(&[a, b]);
        let splits = vec![
            Split {
                position: 0,
                absolute_time: 0.0,
            },
            Split {
                position: 1,
                absolute_time: 2.0,
            },
        ];
        let sts = calc_split_transformations(
            &aligned, &[a, b], &splits, 0.0, 1280.0, 720.0, 0.25, 10.0, 30.0, 100.0, 0.0,
        );
        assert_eq!(sts.len(), 1);
        let st = sts[0];
        assert!((st.x - -50.0).abs() < 1e-6, "x {}", st.x);
        assert!((st.y - 0.0).abs() < 1e-6);
    }

    #[test]
    fn split_camera_contains_detours_and_subsecond_points_with_margins() {
        for rotation in [0.0_f64, 0.6, 1.8, -2.4] {
            let (sin, cos) = rotation.sin_cos();
            let original: Vec<_> = [(0.0, 0.0, 0.0), (0.0, -250.0, 2.0),
                (180.0, 320.0, 3.5), (-90.0, 30.0, 6.0), (100.0, 0.0, 10.0)]
                .iter().map(|&(x, y, t)| RoutePoint::new(x*cos-y*sin, x*sin+y*cos, t)).collect();
            let aligned = calc_aligned_route_points(&original);
            let splits = [Split {position: 0, absolute_time: 0.0},
                Split {position: 1, absolute_time: 10.0}];
            for radius in [0.0, 120.0] {
                let st = calc_split_transformations(&aligned, &original, &splits, 0.0,
                    480.0, 720.0, 0.5, 20.0, 40.0, 100.0, radius)[0];
                let (s, c) = (-st.angle.to_radians()).sin_cos();
                for p in original.iter().chain(aligned.iter()) {
                    let dx = p.position_x + st.x;
                    let dy = p.position_y - st.y;
                    let x = dx*c-dy*s;
                    let y = dx*s+dy*c;
                    assert!((x.abs()+20.0)*st.scale <= 120.0+1e-6, "horizontal detour clipped");
                    assert!((y.abs()+40.0)*st.scale <= 360.0+1e-6, "vertical detour clipped");
                    if radius > 0.0 {
                        assert!((dx.hypot(dy)+40.0)*st.scale <= radius+1e-6);
                    }
                }
            }
        }
    }

    /// 圆形地图框：分段取景要按内切圆再收一次，整段轨迹（含边距）都得落在圆内。
    ///
    /// 复现用户反馈：竖长地图框里矩形高度大于内切圆直径，矩形取景会把轨迹撑到上下边，
    /// 圆形框就把贴边的部分裁掉了。这里用 120x210 的窗口（圆半径 60）验证两件事：
    /// ①矩形取景确实会让轨迹越出半径 60；②给了取景圆之后整段都在圆内。
    #[test]
    fn circular_framing_pulls_the_whole_leg_inside_the_circle() {
        // 一条直线轨迹：每段 300 个路线单位。
        let points: Vec<RoutePoint> = (0..=30)
            .map(|i| RoutePoint::new(i as f64 * 30.0, 0.0, i as f64))
            .collect();
        let mut style = RouteStyle::default();
        style.route_render_mode = RenderMode::Discreet;
        style.tail_render_mode = RenderMode::None;
        style.show_controls = false;
        style.show_runner = false;

        let fresh = || {
            let mut rm = RouteManager::default();
            rm.window_width = 120.0; // 竖长窗口：高 210 > 内切圆直径 120
            rm.window_height = 210.0;
            rm.map_relative_width = 1.0;
            rm.left_right_margin = 10.0;
            rm.top_bottom_margin = 30.0;
            rm.initialize_from_points(
                points.clone(),
                "0:10|0:20",
                false,
                style.clone(),
                ViewMode::FixedSplit,
                1.0,
            );
            rm
        };

        // 取景把段中点放到画面中心（st.x = -mid_x、st.y = mid_y），所以到中心的距离
        // 就是 |p - mid| * scale。逐段量「最远点画出来离中心多远」。
        let drawn_radius = |rm: &RouteManager| -> Vec<f64> {
            let route = &rm.routes[0];
            route
                .split_transformations
                .iter()
                .enumerate()
                .map(|(i, st)| {
                    let (mid_x, mid_y) = (-st.x, st.y);
                    let mut max_radius = 0.0_f64;
                    for p in &route.aligned_route_points {
                        if p.time >= (i * 10) as f64 && p.time <= ((i + 1) * 10) as f64 {
                            max_radius = max_radius.max(
                                ((p.position_x - mid_x).powi(2)
                                    + (p.position_y - mid_y).powi(2))
                                .sqrt(),
                            );
                        }
                    }
                    max_radius * st.scale
                })
                .collect()
        };

        let mut rect = fresh();
        rect.request_full_update();
        rect.update(0.0, 0.0);
        let rect_radii = drawn_radius(&rect);
        assert_eq!(rect_radii.len(), 2, "测试前提：两段取景");
        assert!(
            rect_radii.iter().any(|r| *r > 60.0),
            "测试前提不成立：矩形取景没有让轨迹越过半径 60：{rect_radii:?}"
        );

        let mut circle = fresh();
        circle.circular_framing_radius = 60.0; // 120x120 内切圆
        circle.request_full_update();
        circle.update(0.0, 0.0);
        for (i, r) in drawn_radius(&circle).iter().enumerate() {
            assert!(*r < 60.0, "第 {i} 段有轨迹越出取景圆：{r:.1} > 60");
        }
    }

    /// 跟跑者居中的两个模式没有取景边距：地图远近只由 `centered_zoom` 决定。
    ///
    /// 从「固定分段」切过来时也不能沿用分段取景表算出来的缩放（那是旧的错误行为），
    /// 必须在下一帧就被居中缩放覆盖。
    #[test]
    fn runner_centered_modes_use_the_configured_zoom() {
        let points: Vec<RoutePoint> = (0..=20)
            .map(|i| RoutePoint::new(i as f64 * 10.0, i as f64 * 5.0, i as f64))
            .collect();
        let mut style = RouteStyle::default();
        style.show_controls = false;
        style.show_runner = false;

        for mode in [ViewMode::RunnerCentered, ViewMode::RunnerCenteredSplitOriented] {
            let mut rm = RouteManager::default();
            rm.window_width = 640.0;
            rm.window_height = 360.0;
            // 边距给一个夸张值：居中模式不该受它影响。
            rm.left_right_margin = 999.0;
            rm.top_bottom_margin = 999.0;
            rm.centered_zoom = 0.25;
            rm.initialize_from_points(
                points.clone(),
                "0:10|0:20",
                false,
                style.clone(),
                mode,
                1.0,
            );
            // 跳到 t=5 秒：置 instant 旗让跑者平均位置直接落到新位置（等价于拖动进度条）。
            rm.instant_split_transition_requested = true;
            rm.update(5.0, 33.0);
            assert!(
                (rm.get_scale() - 0.25).abs() < 1e-9,
                "{mode:?} 的缩放应等于设置值 0.25，实际 {}",
                rm.get_scale()
            );
            // 画面中心对着跑者：取景 x = -跑者位置 x。
            assert!(
                (rm.get_x() + 50.0).abs() < 5.0,
                "{mode:?} 没把跑者放到画面中心：get_x={}",
                rm.get_x()
            );

            // 先按固定分段算出一份缩放，再切到居中：下一帧必须换成居中缩放。
            rm.set_view_mode(ViewMode::FixedSplit);
            rm.update(5.0, 0.0);
            let split_scale = rm.get_scale();
            rm.set_view_mode(mode);
            rm.update(5.0, 0.0);
            assert!(
                (rm.get_scale() - 0.25).abs() < 1e-9,
                "{mode:?} 切换后还留着分段取景的缩放 {split_scale}"
            );
            // 缩放参数随时改，下一帧就生效。
            rm.centered_zoom = 0.5;
            rm.update(5.0, 0.0);
            assert!((rm.get_scale() - 0.5).abs() < 1e-9);
        }
    }

    /// 「灵敏度」（跑者平均因子）的平滑时间常数按**内容时间**算：同样的内容时长里，
    /// 喂多少次 update（帧率高低/倍速）结果都该一致 —— 这样倍速播放时镜头不会更滞后。
    ///
    /// 以前调用方固定按 `1000/帧率` 喂增量：倍速时内容时间按倍速前进、平滑器却按原速
    /// 积分，等效平滑时间被放大「倍速」倍。这个测试就是拿「同样的内容时间、两种更新
    /// 频率」对镜头位置，并证明「固定喂小增量」会明显落后（旧写法会被抓出来）。
    #[test]
    fn runner_smoothing_depends_on_content_time_not_call_rate() {
        let points: Vec<RoutePoint> = (0..=400)
            .map(|i| RoutePoint::new(i as f64 * 10.0, 0.0, i as f64 * 0.1))
            .collect();
        // 从 t=0 推进到 t=1000ms（内容时间），每步喂 `step_ms` 增量。
        let camera_x = |step_ms: f64, feed_ms: f64| {
            let mut rm = RouteManager::default();
            rm.window_width = 640.0;
            rm.window_height = 360.0;
            rm.centered_zoom = 1.0;
            rm.set_runner_averaging_factor(2.0); // 时间常数 500ms
            rm.initialize_from_points(
                points.clone(),
                "",
                false,
                RouteStyle::default(),
                ViewMode::RunnerCentered,
                1.0,
            );
            let mut t = 0.0;
            while t < 1000.0 - 1e-9 {
                t = (t + step_ms).min(1000.0);
                rm.update(t / 1000.0, feed_ms);
            }
            rm.get_x()
        };
        let fine = camera_x(5.0, 5.0); // 200 帧/内容秒
        let coarse = camera_x(50.0, 50.0); // 20 帧/内容秒（掉帧/低帧率）
        assert!(
            (fine - coarse).abs() < 8.0,
            "更新频率不该改变镜头位置（同样内容时间）：细={fine} 粗={coarse}"
        );
        let wrong = camera_x(50.0, 5.0); // 旧写法：内容按 50ms 前进，增量只喂 5ms
        assert!(
            (fine - wrong).abs() > 20.0,
            "固定喂小增量会让镜头明显滞后（正是倍速跟不上跑者的原因）：正确={fine} 旧写法={wrong}"
        );
    }

    #[test]
    fn fixed_split_transition_has_the_same_presentation_duration_at_any_speed() {
        let points: Vec<_> = (0..=120).map(|i| {
            if i <= 10 { RoutePoint::new(i as f64 * 10.0, 0.0, i as f64) }
            else { RoutePoint::new(100.0, (i - 10) as f64 * 3.0, i as f64) }
        }).collect();
        let mut base = RouteManager::default();
        base.window_width = 640.0;
        base.window_height = 360.0;
        base.initialize_from_points(points, "10|100", false,
            RouteStyle::default(), ViewMode::FixedSplit, 1.0);
        base.update(9.0, 0.0);
        let sequence = |speed: f64| {
            let mut rm = base.clone();
            rm.update_playback(10.01, 0.0, speed);
            let mut frames = Vec::new();
            for frame in 1..=60 {
                rm.update_playback(10.01 + frame as f64 * 0.02 * speed, 20.0 * speed, speed);
                frames.push((rm.current_st, rm.smooth_split_transition_in_progress));
            }
            frames
        };
        let normal = sequence(1.0);
        assert!(normal[24].1, "transition should still be moving at half a second");
        assert!(!normal[59].1, "transition should finish after about one second");
        for speed in [0.5, 2.0, 4.0, 8.0] {
            for ((expected, moving), (actual, actual_moving)) in normal.iter().zip(sequence(speed)) {
                assert_eq!(*moving, actual_moving, "transition duration changed at {speed}x");
                for (a, b) in [(expected.x, actual.x), (expected.y, actual.y),
                    (expected.angle, actual.angle), (expected.scale, actual.scale)] {
                    assert!((a-b).abs() < 1e-8, "camera transform changed at {speed}x");
                }
            }
        }
        for mode in [ViewMode::RunnerCentered, ViewMode::RunnerCenteredSplitOriented] {
            let mut expected = base.clone();
            expected.set_view_mode(mode);
            let mut actual = expected.clone();
            for frame in 1..=60 {
                let t = 10.01 + frame as f64 * 0.08;
                expected.update(t, 80.0);
                actual.update_playback(t, 80.0, 4.0);
                assert_eq!(expected.current_st.x, actual.current_st.x);
                assert_eq!(expected.current_st.y, actual.current_st.y);
                assert_eq!(expected.current_st.angle, actual.current_st.angle);
            }
        }
    }

    /// Before departure the runner sits at the first track point and does not saw.
    /// A negative query time used to floor() negative and let alpha advance 0..1, wrapping
    /// every whole second -> back-and-forth jitter. Clamping the time removes it.
    #[test]
    fn interp_clamps_negative_time_no_jitter() {
        // Build two aligned points directly (times 0 and 1), positions (0,0) and (100,50).
        let mut a = RoutePoint::new(0.0, 0.0, 0.0);
        a.time = 0.0;
        let mut b = RoutePoint::new(100.0, 50.0, 0.0);
        b.time = 1.0;
        let aligned = [a, b];
        let p0 = interp_route_point(&aligned, -31.0);
        let p1 = interp_route_point(&aligned, -30.5);
        let p2 = interp_route_point(&aligned, -30.0);
        let p3 = interp_route_point(&aligned, -0.5);
        // All pre-start samples must stay at the very first point (no back-and-forth).
        assert_eq!(p0.position_x, aligned[0].position_x);
        assert_eq!(p1.position_x, aligned[0].position_x);
        assert_eq!(p2.position_x, aligned[0].position_x);
        assert_eq!(p3.position_x, aligned[0].position_x);
        // Once on the track it interpolates normally.
        let p4 = interp_route_point(&aligned, 0.5);
        assert!((p4.position_x - 50.0).abs() < 1e-6);
    }

    /// 预览面板尺寸确定后（`window_resized`）整表重算，当前取景必须立刻跟着换。
    ///
    /// 复现 GUI 的真实次序：先用工程里的窗口尺寸建管理器 → 拿到面板真实尺寸 →
    /// `window_resized` → `update`。修复前 `current_st` 停在初始尺寸那份，于是
    /// 第一段一直用错的缩放，只有切段/来回跳转（索引变化）才会被纠正。
    #[test]
    fn resize_reapplies_current_split_transform() {
        // 一条直线轨迹，3 个分段点 ⇒ 2 段。
        let mut points = Vec::new();
        for i in 0..=30 {
            let mut p = RoutePoint::new(i as f64 * 10.0, 0.0, i as f64);
            p.time = i as f64;
            points.push(p);
        }
        let splits = "10|20";

        let mut rm = RouteManager::default();
        rm.window_width = 1280.0;
        rm.window_height = 720.0;
        rm.initialize_from_points(
            points.clone(),
            splits,
            false,
            RouteStyle::default(),
            ViewMode::FixedSplit,
            0.25,
        );
        let at_init = rm.get_scale();
        assert!(at_init > 0.0, "first split must be framed at init");
        rm.update(0.0, 16.0);
        assert!(
            (rm.get_scale() - at_init).abs() < 1e-9,
            "update must not drift while the size is unchanged"
        );

        // 面板真实尺寸只有一半 ⇒ 取景缩放必须按新尺寸重算并立刻生效。
        rm.window_resized(640.0, 360.0);
        rm.update(0.0, 16.0);
        let after = rm.get_scale();
        assert!(
            (after - at_init * 0.5).abs() < 1e-6,
            "scale must follow the new window: {at_init} -> {after}"
        );
    }

    /// 视频起点早于出发时刻（`runner_time_offset` 为负）时，t=0 落在所有分段之外，
    /// `find_index` 取不到任何一段——这时也必须按新窗口尺寸取第 0 段的景，
    /// 而不是留着初始尺寸那份。
    #[test]
    fn resize_reapplies_split_transform_before_departure() {
        let mut points = Vec::new();
        for i in 0..=30 {
            let mut p = RoutePoint::new(i as f64 * 10.0, 0.0, i as f64);
            p.time = i as f64;
            points.push(p);
        }
        let mut style = RouteStyle::default();
        style.runner_time_offset = -31.0; // 录像比出发早 31 秒

        let mut rm = RouteManager::default();
        rm.window_width = 1280.0;
        rm.window_height = 720.0;
        rm.initialize_from_points(
            points,
            "10|20",
            false,
            style,
            ViewMode::FixedSplit,
            0.25,
        );
        let at_init = rm.get_scale();
        assert!(at_init > 0.0, "must still frame the first leg before departure");

        rm.window_resized(640.0, 360.0);
        rm.update(0.0, 16.0);
        let after = rm.get_scale();
        assert!(
            (after - at_init * 0.5).abs() < 1e-6,
            "pre-departure framing must follow the new window: {at_init} -> {after}"
        );
    }

    /// 自适应色带端点：慢端取 P90、快端取 P10（配速单位分钟/公里），
    /// 没有 GPS 配速（全 0）的工程返回 None（调用方保持原设定）。
    #[test]
    fn adaptive_pace_range_uses_percentiles() {
        let mut points = Vec::new();
        let mut p = RoutePoint::default();
        p.pace = 0.0; // 没有配速的点必须被忽略
        points.push(p);
        for i in 0..=100 {
            let mut p = RoutePoint::default();
            p.pace = 4.0 + 16.0 * (i as f64) / 100.0; // 4.0 .. 20.0 分钟/公里
            points.push(p);
        }
        let mut p = RoutePoint::default();
        p.pace = f64::NAN; // 非法值也不能污染统计
        points.push(p);

        let (low, high) = adaptive_pace_range(&points).expect("有配速就该给出端点");
        assert!(low > high, "慢端(low)必须大于快端(high): {low} {high}");
        assert!((low - 18.4).abs() < 0.5, "慢端应接近 P90=18.4，实际 {low}");
        assert!((high - 5.6).abs() < 0.5, "快端应接近 P10=5.6，实际 {high}");
        // 端点真的能拉开颜色：慢端比快端红、快端比慢端绿（与渲染端同一条色带公式）。
        let mut slow_pt = RoutePoint::default();
        slow_pt.pace = low;
        let mut fast_pt = RoutePoint::default();
        fast_pt.pace = high;
        let red = slow_pt.pace_color(low, high);
        let green = fast_pt.pace_color(low, high);
        assert!(red.0 > red.1 && green.1 > green.0, "色带两端必须是红→绿");

        // 匀速跑：快慢差不足 0.5 分钟/公里时也要给出一段跨度，否则整条一个颜色。
        let flat: Vec<RoutePoint> = (0..10)
            .map(|_| {
                let mut p = RoutePoint::default();
                p.pace = 6.0;
                p
            })
            .collect();
        let (low, high) = adaptive_pace_range(&flat).expect("匀速也要有端点");
        assert!(low - high >= 0.5, "匀速时也要有可见跨度: {low} {high}");

        // 「地图 + 线路」工程没有逐点配速 -> None。
        let no_pace: Vec<RoutePoint> = (0..5).map(|_| RoutePoint::default()).collect();
        assert!(adaptive_pace_range(&no_pace).is_none());
        assert!(adaptive_pace_range(&[]).is_none());
    }

    /// 箭头用的方向向量：直线段沿着走向，拐弯后跟着转，静止/退化时退回航向角。
    #[test]
    fn runner_direction_follows_the_track() {
        // 前 10 秒往 +x 走，之后往 +y 走。
        let mut points = Vec::new();
        for i in 0..=10 {
            let mut p = RoutePoint::new(i as f64 * 10.0, 0.0, i as f64);
            p.time = i as f64;
            points.push(p);
        }
        for i in 1..=10 {
            let mut p = RoutePoint::new(100.0, i as f64 * 10.0, (10 + i) as f64);
            p.time = (10 + i) as f64;
            points.push(p);
        }
        let mut rm = RouteManager::default();
        rm.window_width = 1280.0;
        rm.window_height = 720.0;
        rm.initialize_from_points(
            points,
            "10|20",
            false,
            RouteStyle::default(),
            ViewMode::FixedSplit,
            0.25,
        );
        rm.update(5.0, 16.0);

        let (dx, dy) = rm.runner_direction(0, 5.0, 0.5);
        assert!(
            (dx - 1.0).abs() < 1e-6 && dy.abs() < 1e-6,
            "第一段应指向 +x: ({dx}, {dy})"
        );
        let (dx, dy) = rm.runner_direction(0, 15.0, 0.5);
        assert!(
            dx.abs() < 1e-6 && (dy - 1.0).abs() < 1e-6,
            "拐弯后应指向 +y: ({dx}, {dy})"
        );
        // 单位向量，且越界的时间不会 panic（夹到轨迹两端）。
        for t in [-5.0, 0.0, 40.0] {
            let (dx, dy) = rm.runner_direction(0, t, 0.5);
            assert!(
                ((dx * dx + dy * dy).sqrt() - 1.0).abs() < 1e-6,
                "t={t} 时方向必须是单位向量: ({dx}, {dy})"
            );
        }
    }
}
