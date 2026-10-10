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
    pub runner_vertical_offset: f64,
    pub runner_averaging_factor: f64,
    pub window_width: f64,
    pub window_height: f64,
    pub map_relative_width: f64,

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
            runner_vertical_offset: 0.0,
            runner_averaging_factor: RUNNER_AVERAGING_FACTOR,
            window_width: 1280.0,
            window_height: 720.0,
            map_relative_width: 0.25,
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
            &splits,
            route.control_time_offset,
            self.window_width,
            self.window_height,
            self.map_relative_width,
            self.left_right_margin,
            self.top_bottom_margin,
            self.maximum_automatic_zoom,
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
                    &splits,
                    route.control_time_offset,
                    self.window_width,
                    self.window_height,
                    self.map_relative_width,
                    self.left_right_margin,
                    self.top_bottom_margin,
                    self.maximum_automatic_zoom,
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
    splits: &[Split],
    cto: f64,
    window_width: f64,
    window_height: f64,
    map_relative_width: f64,
    left_right_margin: f64,
    top_bottom_margin: f64,
    max_auto_zoom: f64,
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
        if start_index != stop_index {
            let start_rp = aligned[start_index];
            let stop_rp = aligned[stop_index];
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
            for j in start_index..=stop_index {
                let p = aligned[j];
                let (rx, ry) = rot(p.position_x, p.position_y);
                min_x = min_x.min(rx);
                max_x = max_x.max(rx);
                min_y = min_y.min(ry);
                max_y = max_y.max(ry);
            }
            let (start_rx, _start_ry) = rot(start_rp.position_x, start_rp.position_y);
            let mid_x = (start_rp.position_x + stop_rp.position_x) / 2.0;
            let mid_y = (start_rp.position_y + stop_rp.position_y) / 2.0;

            let split_width_left = (min_x - start_rx).abs() * 2.0 + 2.0 * left_right_margin;
            let split_width_right = (max_x - start_rx).abs() * 2.0 + 2.0 * left_right_margin;
            let split_width = split_width_left.max(split_width_right);
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
            &aligned, &splits, 0.0, 1280.0, 720.0, 0.25, 10.0, 30.0, 100.0,
        );
        assert_eq!(sts.len(), 1);
        let st = sts[0];
        assert!((st.x - -50.0).abs() < 1e-6, "x {}", st.x);
        assert!((st.y - 0.0).abs() < 1e-6);
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
