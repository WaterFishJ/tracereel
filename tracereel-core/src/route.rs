//! Route point data model (ported from RoutePoint.h / route_data.py).

use crate::math::interpolate_from_green_to_red;

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

#[cfg(test)]
mod tests {
    use super::*;

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
