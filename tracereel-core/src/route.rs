//! Route point data model (ported from RoutePoint.h / route_data.py).

use crate::math::{haversine_distance, interpolate_from_green_to_red, wrap_angle_delta};

/// A single route waypoint.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoutePoint {
    /// Unix time in milliseconds (origin: .NET ticks for QuickRoute).
    pub date_time_ms: f64,
    /// Time in seconds since the start of the route.
    pub time: f64,
    /// Longitude (degrees).
    pub coordinate_x: f64,
    /// Latitude (degrees).
    pub coordinate_y: f64,
    /// Position in map pixel units (panel homogenous space).
    pub position_x: f64,
    pub position_y: f64,
    /// Elevation in meters.
    pub elevation: f64,
    /// Heart rate in bpm.
    pub heart_rate: f64,
    /// Pace in min/km.
    pub pace: f64,
    /// Orientation in degrees (continuous, not compass).
    pub orientation: f64,
}

impl Default for RoutePoint {
    fn default() -> Self {
        Self {
            date_time_ms: 0.0,
            time: 0.0,
            coordinate_x: 0.0,
            coordinate_y: 0.0,
            position_x: 0.0,
            position_y: 0.0,
            elevation: 0.0,
            heart_rate: 0.0,
            pace: 0.0,
            orientation: 0.0,
        }
    }
}

impl RoutePoint {
    pub fn new(x: f64, y: f64, time: f64) -> Self {
        Self {
            position_x: x,
            position_y: y,
            time,
            ..Self::default()
        }
    }

    /// Linear interpolation along a route segment.
    pub fn lerp(a: &RoutePoint, b: &RoutePoint, alpha: f64) -> RoutePoint {
        RoutePoint {
            date_time_ms: a.date_time_ms + (b.date_time_ms - a.date_time_ms) * alpha,
            time: a.time + (b.time - a.time) * alpha,
            coordinate_x: a.coordinate_x + (b.coordinate_x - a.coordinate_x) * alpha,
            coordinate_y: a.coordinate_y + (b.coordinate_y - a.coordinate_y) * alpha,
            position_x: a.position_x + (b.position_x - a.position_x) * alpha,
            position_y: a.position_y + (b.position_y - a.position_y) * alpha,
            elevation: a.elevation + (b.elevation - a.elevation) * alpha,
            heart_rate: a.heart_rate + (b.heart_rate - a.heart_rate) * alpha,
            pace: a.pace + (b.pace - a.pace) * alpha,
            orientation: a.orientation + (b.orientation - a.orientation) * alpha,
        }
    }

    /// Compute this point's color from a green→red pace ramp. Returns (r,g,b) in [0,1].
    pub fn pace_color(&self, low_pace: f64, high_pace: f64) -> (f64, f64, f64) {
        interpolate_from_green_to_red(high_pace, low_pace, self.pace)
    }
}

impl<'a> From<&'a RoutePoint> for RoutePoint {
    fn from(p: &'a RoutePoint) -> Self {
        *p
    }
}

/// 按 QuickRoute 自己的 `Route.CalculateSpeeds` / `Route.CalculateDirections` 补全逐点
/// `pace`（分钟/公里）与 `orientation`（连续角度，度）。
///
/// 两个路线来源必须共用这一份实现：QuickRoute JPEG 导出（`quickroute.rs`）与 `.qrt`
/// 工程（`qrt.rs`）。以前只有 JPEG 那条路算，`.qrt` 工程导进来的轨迹 `pace` 与
/// `orientation` 全是 0 —— 配速色带整条退化成同一个颜色、「配速色带自适应」拿不到端点
/// （`adaptive_pace_range` 只统计 `pace > 0` 的点），按航向取景（`RunnerCentered`）与
/// 静止时的朝向兜底也恒为 0°。
///
/// 前置条件（本函数不改坐标）：`date_time_ms`（毫秒、单调不减）、`coordinate_x/y`
/// （经纬度）、`position_x/y`（地图像素）都已填好。`time` 按首点归零重算，
/// `pace`/`orientation` 就地覆盖；少于两个点时没有区间可算，保持 0 哨兵。
pub fn fill_route_metrics(points: &mut [RoutePoint]) {
    if points.is_empty() {
        return;
    }
    let t0 = points[0].date_time_ms;
    for p in points.iter_mut() {
        p.time = (p.date_time_ms - t0) / 1000.0;
    }
    if points.len() < 2 {
        return;
    }

    // QuickRoute 用**居中**时间窗算瞬时速度，即使关掉平滑也一样：内部点取前后两段
    // 速度的平均，端点只有一侧可用。别改成「沿用上一段的配速」。
    let mut speeds = vec![0.0; points.len() - 1];
    for i in 1..points.len() {
        let dt = (points[i].date_time_ms - points[i - 1].date_time_ms) / 1000.0;
        let dist = haversine_distance(
            points[i - 1].coordinate_x,
            points[i - 1].coordinate_y,
            points[i].coordinate_x,
            points[i].coordinate_y,
        );
        if dt > 0.0 {
            speeds[i - 1] = dist / dt;
        }
    }
    for i in 0..points.len() {
        let speed = if i == 0 {
            speeds[0]
        } else if i == points.len() - 1 {
            speeds[i - 1]
        } else {
            (speeds[i - 1] + speeds[i]) * 0.5
        };
        // ConvertUtil.ToPace 的零哨兵 + 60 分钟/公里上限：原地不动、时间倒挂或
        // 坐标缺失都退化成 0（表示「没有配速」），不能产生 NaN/负值污染色带统计。
        points[i].pace = if speed.is_finite() && speed > 0.0 {
            (1000.0 / (60.0 * speed)).min(60.0)
        } else {
            0.0
        };
    }

    // 航向按地图像素差算成**连续**角度（不折返到 [0,360)）。原始实现是
    // `a = 90° + atan2(dy, dx)`，因此 `(dx, dy) ∝ (sin a, -cos a)`。
    let mut current_angle = 0.0;
    for i in 0..points.len() - 1 {
        let dx = points[i + 1].position_x - points[i].position_x;
        let dy = points[i + 1].position_y - points[i].position_y;
        let mut angle = (-dy).atan2(dx).to_degrees();
        angle = 90.0 - angle;
        if angle > 180.0 {
            angle -= 360.0;
        }
        current_angle += wrap_angle_delta(angle - current_angle);
        points[i].orientation = current_angle;
    }
    let last = points.len() - 1;
    points[last].orientation = current_angle;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(lon: f64, lat: f64, x: f64, y: f64, ms: f64) -> RoutePoint {
        RoutePoint {
            date_time_ms: ms,
            coordinate_x: lon,
            coordinate_y: lat,
            position_x: x,
            position_y: y,
            ..RoutePoint::default()
        }
    }

    /// 与 `quickroute.rs::pace_uses_quickroute_centred_speed_and_endpoint_values` 同一套
    /// 期望值：居中速度、端点取单侧、`time` 按首点归零。
    #[test]
    fn fill_route_metrics_uses_centred_speed_and_single_sided_endpoints() {
        let mut points: Vec<RoutePoint> = [0.0, 0.001, 0.003]
            .into_iter()
            .enumerate()
            .map(|(i, lon)| point(lon, 0.0, i as f64 * 100.0, 0.0, i as f64 * 10_000.0))
            .collect();
        fill_route_metrics(&mut points);

        let speed = haversine_distance(0.0, 0.0, 0.001, 0.0) / 10.0;
        for (p, expected_speed) in points.iter().zip([speed, 1.5 * speed, 2.0 * speed]) {
            assert!(
                (p.pace - 1000.0 / (60.0 * expected_speed)).abs() < 1e-9,
                "pace was {}",
                p.pace
            );
        }
        assert_eq!(points[0].time, 0.0);
        assert_eq!(points[2].time, 20.0);
        // 一路向东：a = 90° + atan2(dy, dx) = 90°，且必须三段连续同一个角。
        for p in &points {
            assert!((p.orientation - 90.0).abs() < 1e-9, "{}", p.orientation);
        }
    }

    #[test]
    fn fill_route_metrics_keeps_zero_sentinel_when_standing_still() {
        let mut points: Vec<RoutePoint> = (0..3)
            .map(|i| point(113.0, 23.0, 0.0, 0.0, i as f64 * 1000.0))
            .collect();
        fill_route_metrics(&mut points);
        assert!(points.iter().all(|p| p.pace == 0.0));
        assert!(points.iter().all(|p| p.pace.is_finite()));
    }

    /// 时间倒挂（.qrt 里丢掉时间戳的点以前会被写成 0）不能产生 NaN/负数配速。
    #[test]
    fn fill_route_metrics_ignores_backwards_time() {
        let mut points = vec![
            point(0.0, 0.0, 0.0, 0.0, 0.0),
            point(0.001, 0.0, 100.0, 0.0, 1000.0),
            point(0.002, 0.0, 200.0, 0.0, 0.0),
        ];
        fill_route_metrics(&mut points);
        assert!(points.iter().all(|p| p.pace.is_finite() && p.pace >= 0.0));
        assert_eq!(points[2].time, 0.0, "首点为 0 后这一段是倒挂的");
        assert_eq!(points[2].pace, 0.0, "倒挂区间没有速度 → 零哨兵");
    }

    #[test]
    fn lerp_midpoint() {
        let a = RoutePoint::new(0.0, 0.0, 0.0);
        let b = RoutePoint::new(10.0, 20.0, 1000.0);
        let m = RoutePoint::lerp(&a, &b, 0.5);
        assert!((m.position_x - 5.0).abs() < 1e-9);
        assert!((m.position_y - 10.0).abs() < 1e-9);
        assert!((m.time - 500.0).abs() < 1e-9);
    }
}
