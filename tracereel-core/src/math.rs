//! Geometry and color math shared by the route/QuickRoute code.

/// Earth radius (meters) used by the QuickRoute orthographic projection & haversine.
pub const EARTH_RADIUS: f64 = 6_378_200.0;

/// Great-circle distance in meters using the Haversine formula.
pub fn haversine_distance(lon1: f64, lat1: f64, lon2: f64, lat2: f64) -> f64 {
    let r = EARTH_RADIUS;
    let phi1 = lat1.to_radians();
    let phi2 = lat2.to_radians();
    let dphi = (lat2 - lat1).to_radians();
    let dlambda = (lon2 - lon1).to_radians();
    let a = (dphi / 2.0).sin().powi(2) + phi1.cos() * phi2.cos() * (dlambda / 2.0).sin().powi(2);
    2.0 * r * a.sqrt().atan2((1.0 - a).sqrt())
}

/// Orthographic cartographic projection (meters) relative to a projection origin.
pub fn project_coordinate(lon: f64, lat: f64, origin_lon: f64, origin_lat: f64) -> (f64, f64) {
    let r = EARTH_RADIUS;
    let lambda0 = origin_lon.to_radians();
    let phi0 = origin_lat.to_radians();
    let lambda = lon.to_radians();
    let phi = lat.to_radians();
    let x = r * phi.cos() * (lambda - lambda0).sin();
    let y = r * (phi0.cos() * phi.sin() - phi0.sin() * phi.cos() * (lambda - lambda0).cos());
    (x, y)
}

/// Interpolate a green→red color by `value` normalized to `[green_value, red_value]`.
/// Returns (r, g, b) in [0,1]; matches `interpolate_from_green_to_red`.
pub fn interpolate_from_green_to_red(
    green_value: f64,
    red_value: f64,
    value: f64,
) -> (f64, f64, f64) {
    let alpha = (value - green_value) / (red_value - green_value);
    let alpha = alpha.clamp(0.0, 1.0);
    let r = if alpha > 0.5 { 1.0 } else { 2.0 * alpha };
    let g = if alpha > 0.5 {
        1.0 - 2.0 * (alpha - 0.5)
    } else {
        1.0
    };
    (r, g, 0.0)
}

/// Continuous-angle unwrap: make `angle_delta` the shortest signed rotation to reach `angle`
/// from the previous running angle. Returns the wrapped delta.
pub fn wrap_angle_delta(angle_delta: f64) -> f64 {
    let mut final_delta = angle_delta;
    while final_delta.abs() > 360.0 {
        final_delta += if final_delta > 0.0 { -360.0 } else { 360.0 };
    }
    if final_delta.abs() > 180.0 {
        final_delta = 360.0 - final_delta.abs();
        final_delta *= if angle_delta < 0.0 { 1.0 } else { -1.0 };
    }
    final_delta
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn haversine_known_distance() {
        // ~111.19 km per degree of latitude at the equator.
        let d = haversine_distance(0.0, 0.0, 0.0, 1.0);
        assert!((d - 111_194.0).abs() < 1000.0, "got {d}");
    }

    #[test]
    fn green_red_banding() {
        // At extremes the color should be pure green / pure red.
        assert_eq!(
            interpolate_from_green_to_red(5.0, 15.0, 5.0),
            (0.0, 1.0, 0.0)
        );
        assert_eq!(
            interpolate_from_green_to_red(5.0, 15.0, 15.0),
            (1.0, 0.0, 0.0)
        );
    }
}
