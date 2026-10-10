//! GPX parsing + per-sample metrics for the video HUD overlay.
//!
//! Reads a standard GPX 1.1 track (lat/lon/elevation/time, optional Garmin heart-rate
//! extension) and precomputes the derived metrics the HUD shows: instantaneous speed,
//! pace (min/km), cumulative distance, bearing (course degrees) and altitude.
//! `sample_at` maps a video timestamp to the nearest track sample.
//!
//! GPX uses haversine distance with R = 6371000 (different from QuickRoute's R).

use std::path::Path;

const EARTH_RADIUS_GPX: f64 = 6_371_000.0;

/// A single parsed track point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GpxPoint {
    /// Unix time in seconds.
    pub t: f64,
    pub lat: f64,
    pub lon: f64,
    /// Altitude in meters.
    pub ele: f64,
    /// Heart rate in bpm, `None` when absent.
    pub hr: Option<f64>,
}

/// A parsed GPX track with precomputed metrics.
#[derive(Debug, Clone)]
pub struct GpxTrack {
    pub points: Vec<GpxPoint>,
    pub has_hr: bool,
    pub start_time: f64,
    // per-sample derived metrics
    cumdist: Vec<f64>,
    speed_smooth: Vec<f64>,
    bearing: Vec<f64>,
    hud_heading: Vec<Option<f64>>,
    hud_phase: Vec<f64>,
    hud_profile: Vec<[f64; 2]>,
    hud_max_speed: f64,
}

/// A single HUD sample at an arbitrary time.
#[derive(Debug, Clone, Copy)]
pub struct GpxSample {
    pub speed_kmh: f64,
    /// Pace in min/km (0 when below a small speed threshold).
    pub pace_min_km: f64,
    /// Cumulative distance in km.
    pub distance_km: f64,
    /// Bearing in degrees [0, 360).
    pub bearing: f64,
    pub altitude_m: Option<f64>,
    pub heart_rate: Option<f64>,
}

fn hav(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let r = EARTH_RADIUS_GPX;
    let p1 = lat1.to_radians();
    let p2 = lat2.to_radians();
    let dp = (lat2 - lat1).to_radians();
    let dl = (lon2 - lon1).to_radians();
    let a = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * r * a.sqrt().min(1.0).asin()
}

fn bearing_deg(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let p1 = lat1.to_radians();
    let p2 = lat2.to_radians();
    let dl = (lon2 - lon1).to_radians();
    let y = dl.sin() * p2.cos();
    let x = p1.cos() * p2.sin() - p1.sin() * p2.cos() * dl.cos();
    (y.atan2(x).to_degrees() + 360.0) % 360.0
}

/// Find a likely `.gpx` companion to a video file.
///
/// The common action-camera convention is `<full-video-name>.gpx` (e.g. `20260823.mp4.gpx`),
/// then falls back to the extension-less name.
pub fn find_gpx_for_video(video_path: &str) -> String {
    if video_path.is_empty() {
        return String::new();
    }
    for cand in [format!("{video_path}.gpx"), format!("{video_path}.GPX")] {
        if Path::new(&cand).exists() {
            return cand;
        }
    }
    let base = Path::new(video_path).with_extension("");
    let base = base.to_string_lossy().into_owned();
    for cand in [format!("{base}.gpx"), format!("{base}.GPX")] {
        if Path::new(&cand).exists() {
            return cand;
        }
    }
    String::new()
}

impl GpxTrack {
    /// Compute derived metrics from raw points.
    pub fn from_points(points: Vec<GpxPoint>) -> Result<Self, String> {
        if points.is_empty() {
            return Err("GPX has no valid track points".to_string());
        }
        let n = points.len();
        let mut cumdist = vec![0.0; n];
        let mut speed = vec![0.0; n];
        let mut bearing = vec![0.0; n];
        for i in 1..n {
            let d = hav(
                points[i - 1].lat,
                points[i - 1].lon,
                points[i].lat,
                points[i].lon,
            );
            cumdist[i] = cumdist[i - 1] + d;
            let dt = (points[i].t - points[i - 1].t).max(1e-3);
            speed[i] = d / dt;
            bearing[i] = bearing_deg(
                points[i - 1].lat,
                points[i - 1].lon,
                points[i].lat,
                points[i].lon,
            );
        }
        // Smooth speed with a small window to damp GPS jitter.
        let mut speed_smooth = vec![0.0; n];
        if n > 0 {
            let win = 1usize.max(5usize.min(n / 3usize.max(1)));
            let half = win / 2;
            for i in 0..n {
                let lo = i.saturating_sub(half);
                let hi = (i + half + 1).min(n);
                let mut sum = 0.0;
                let mut cnt = 0;
                for j in lo..hi {
                    sum += speed[j];
                    cnt += 1;
                }
                speed_smooth[i] = if cnt > 0 { sum / cnt as f64 } else { 0.0 };
            }
        }
        let has_hr = points.iter().any(|p| p.hr.is_some());
        let start_time = points[0].t;
        let mut hud_heading = Vec::with_capacity(n);
        let mut last_heading = None;
        let mut hud_phase = vec![0.0; n];
        for i in 0..n {
            if i > 0 && speed_smooth[i] * 3.6 >= 1.0 {
                last_heading = Some(bearing[i]);
            }
            hud_heading.push(last_heading);
            if i > 0 {
                // Integrate BPM on the track clock; seeks and exports have identical phase.
                let a = points[i - 1].hr.filter(|h| h.is_finite() && *h > 0.0);
                let b = points[i].hr.filter(|h| h.is_finite() && *h > 0.0);
                hud_phase[i] = hud_phase[i - 1]
                    + match (a, b) {
                        (Some(a), Some(b)) => {
                            (points[i].t - points[i - 1].t).max(0.0) * (a + b) / 120.0
                        }
                        _ => 0.0,
                    };
            }
        }
        // Fixed-size profile cache: min/max per bucket preserves narrow hills.
        let mut hud_profile = Vec::new();
        for chunk in points.chunks((n / 1024).max(1)) {
            let min = chunk
                .iter()
                .enumerate()
                .filter(|(_, p)| p.ele.is_finite())
                .min_by(|a, b| a.1.ele.total_cmp(&b.1.ele));
            let max = chunk
                .iter()
                .enumerate()
                .filter(|(_, p)| p.ele.is_finite())
                .max_by(|a, b| a.1.ele.total_cmp(&b.1.ele));
            if let (Some(a), Some(b)) = (min, max) {
                let pair = if a.0 <= b.0 { [a.1, b.1] } else { [b.1, a.1] };
                for p in pair {
                    hud_profile.push([p.t - start_time, p.ele]);
                }
            }
        }
        let peak = speed_smooth.iter().copied().fold(0.0, f64::max) * 3.6;
        let hud_max_speed = ((peak / 10.0).ceil() * 10.0).max(10.0);
        Ok(Self {
            points,
            has_hr,
            start_time,
            cumdist,
            speed_smooth,
            bearing,
            hud_heading,
            hud_phase,
            hud_profile,
            hud_max_speed,
        })
    }

    fn index_for_time(&self, target: f64) -> usize {
        let n = self.points.len();
        if n == 0 {
            return 0;
        }
        let mut lo = 0;
        let mut hi = n;
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.points[mid].t < target {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        if lo == 0 {
            return 0;
        }
        if lo >= n {
            return n - 1;
        }
        if (target - self.points[lo - 1].t) < (self.points[lo].t - target) {
            lo - 1
        } else {
            lo
        }
    }

    pub fn hud_max_speed(&self) -> f64 {
        self.hud_max_speed
    }
    pub fn hud_profile(&self) -> &[[f64; 2]] {
        &self.hud_profile
    }
    pub fn hud_at(&self, time: f64, offset: f64) -> Option<(Option<f64>, f64)> {
        let t = self.start_time + time + offset;
        if t < self.points.first()?.t || t > self.points.last()?.t {
            return None;
        }
        let b = self
            .points
            .partition_point(|p| p.t < t)
            .min(self.points.len() - 1);
        let a = b.saturating_sub(1);
        let dt = self.points[b].t - self.points[a].t;
        let f = if dt > 0.0 {
            ((t - self.points[a].t) / dt).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let heading = match (self.hud_heading[a], self.hud_heading[b]) {
            (Some(x), Some(y)) => Some((x + ((y - x + 540.0) % 360.0 - 180.0) * f + 360.0) % 360.0),
            (_, y) if f >= 1.0 => y,
            (x, _) => x,
        };
        let phase = match (self.points[a].hr, self.points[b].hr) {
            (Some(x), Some(y)) if x > 0.0 && y > 0.0 => {
                self.hud_phase[a] + dt * (x * f + (y - x) * f * f / 2.0) / 60.0
            }
            _ => self.hud_phase[a],
        };
        Some((heading, phase.fract()))
    }

    /// Sample metrics at a video timestamp (seconds). `time_offset` shifts the GPX
    /// timeline relative to the video. Returns `None` when out of range / empty.
    pub fn sample_at(&self, video_time: f64, time_offset: f64) -> Option<GpxSample> {
        if self.points.is_empty() {
            return None;
        }
        let target = self.start_time + time_offset + video_time;
        if target < self.points[0].t || target > self.points[self.points.len() - 1].t {
            return None;
        }
        let i = self.index_for_time(target);
        let speed_ms = self.speed_smooth[i];
        let speed_kmh = speed_ms * 3.6;
        let pace = if speed_kmh > 0.1 {
            60.0 / speed_kmh
        } else {
            0.0
        };
        Some(GpxSample {
            speed_kmh,
            pace_min_km: pace,
            distance_km: self.cumdist[i] / 1000.0,
            bearing: self.bearing[i],
            altitude_m: Some(self.points[i].ele),
            heart_rate: if self.has_hr { self.points[i].hr } else { None },
        })
    }
}

/// Parse a GPX file into a `GpxTrack`.
pub fn parse_gpx(path: &str) -> Result<GpxTrack, String> {
    let xml = std::fs::read_to_string(path).map_err(|e| format!("read GPX: {e}"))?;
    parse_gpx_str(&xml)
}

/// Parse GPX XML text into a `GpxTrack`.
pub fn parse_gpx_str(xml: &str) -> Result<GpxTrack, String> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| format!("XML parse: {e}"))?;
    let mut points = Vec::new();
    for el in doc.descendants() {
        let is_trkpt = el.is_element() && el.tag_name().name() == "trkpt";
        if !is_trkpt {
            continue;
        }
        let lat: f64 = el
            .attribute("lat")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.0);
        let lon: f64 = el
            .attribute("lon")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.0);
        let mut t: Option<f64> = None;
        let mut ele: Option<f64> = None;
        let mut hr: Option<f64> = None;
        for child in el.children() {
            if !child.is_element() {
                continue;
            }
            match child.tag_name().name() {
                "ele" => ele = child.text().and_then(|s| s.trim().parse().ok()),
                "time" => t = child.text().map(|s| parse_iso(s.trim())),
                "hr" => hr = child.text().and_then(|s| s.trim().parse().ok()),
                _ => {
                    // Garmin extensions hold <hr> nested under <extensions>.
                    for cc in child.descendants() {
                        if cc.is_element() && cc.tag_name().name() == "hr" {
                            hr = cc.text().and_then(|s| s.trim().parse().ok());
                        }
                    }
                }
            }
        }
        if let Some(tt) = t {
            if tt > 0.0 {
                points.push(GpxPoint {
                    t: tt,
                    lat,
                    lon,
                    ele: ele.unwrap_or(0.0),
                    hr,
                });
            }
        }
    }
    GpxTrack::from_points(points)
}

/// Parse an ISO-8601 timestamp (with `Z`) to unix seconds.
fn parse_iso(text: &str) -> f64 {
    // Handles "YYYY-MM-DDTHH:MM:SSZ" and optional fractional seconds.
    let t = text.replace('Z', "+00:00");
    // Use a simple manual parse: date + time separated by 'T'.
    if let Some((date_part, time_part)) = t.split_once('T') {
        let time_part = time_part.trim_end_matches("+00:00");
        let d: Vec<i32> = date_part
            .split('-')
            .filter_map(|s| s.parse().ok())
            .collect();
        let mut tm = time_part
            .split(':')
            .map(|s| s.parse::<f64>().unwrap_or(0.0));
        let h = tm.next().unwrap_or(0.0);
        let m = tm.next().unwrap_or(0.0);
        let s = tm.next().unwrap_or(0.0);
        if d.len() == 3 {
            return days_from_civil(d[0], d[1], d[2]) * 86400.0 + h * 3600.0 + m * 60.0 + s;
        }
    }
    0.0
}

/// Days since 1970-01-01 for a civil date (Howard Hinnant algorithm).
fn days_from_civil(y: i32, m: i32, d: i32) -> f64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u32;
    let mp = ((m + 9) % 12) as u32;
    let doy = (153 * mp + 2) / 5 + (d as u32 - 1);
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    (era as f64) * 146097.0 + doe as f64 - 719468.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gpx_hr_extension() {
        let xml = r#"<?xml version="1.0"?>
<gpx version="1.1" xmlns="http://www.topografix.com/GPX/1/1" xmlns:gpxtpx="http://www.garmin.com/xmlschemas/TrackPointExtension/v1">
  <trk>
    <trkseg>
      <trkpt lat="10.0" lon="20.0"><ele>100</ele><time>2026-08-23T00:00:00Z</time></trkpt>
      <trkpt lat="10.001" lon="20.001"><ele>101</ele><time>2026-08-23T00:00:01Z</time><extensions><gpxtpx:hr>120</gpxtpx:hr></extensions></trkpt>
    </trkseg>
  </trk>
</gpx>"#;
        let track = parse_gpx_str(xml).expect("parse");
        assert_eq!(track.points.len(), 2);
        assert!(track.has_hr);
        assert_eq!(track.points[1].hr, Some(120.0));
        // ~157 m displacement over 1s => speed smoothing near 157 m/s
        let s = track.sample_at(1.0, 0.0).expect("sample");
        assert!(s.speed_kmh > 100.0, "speed {}", s.speed_kmh);
    }

    #[test]
    fn parses_iso() {
        let t = parse_iso("2026-08-23T00:00:00Z");
        assert!(t > 1_700_000_000.0, "got {t}");
    }

    #[test]
    fn hud_phase_heading_are_seek_independent() {
        let mut t = GpxTrack::from_points(
            (0..4)
                .map(|i| GpxPoint {
                    t: 1000.0 + i as f64,
                    lat: 10.0 + i as f64 * 0.001,
                    lon: 20.0,
                    ele: 10.0,
                    hr: Some(120.0),
                })
                .collect(),
        )
        .unwrap();
        t.hud_heading = vec![None, Some(359.0), Some(1.0), Some(1.0)];
        assert!(t.hud_at(0.0, 0.0).unwrap().0.is_none());
        let a = t.hud_at(1.5, 0.0).unwrap();
        assert!((a.0.unwrap()).abs() < 1e-8);
        assert!((t.hud_at(1.125, 0.0).unwrap().1 - 0.25).abs() < 1e-8);
        let _ = t.hud_at(3.0, 0.0);
        assert_eq!(t.hud_at(1.5, 0.0).unwrap(), a);
        assert_eq!(t.hud_at(1.0, 0.5).unwrap(), a);
        assert!(t.hud_at(5.0, 0.0).is_none());
    }

    #[test]
    fn stationary_track_has_no_heading_or_pulse() {
        let t = GpxTrack::from_points(
            (0..3)
                .map(|i| GpxPoint {
                    t: 1000.0 + i as f64,
                    lat: 10.0,
                    lon: 20.0,
                    ele: 50.0,
                    hr: None,
                })
                .collect(),
        )
        .unwrap();
        assert_eq!(t.hud_at(1.5, 0.0), Some((None, 0.0)));
        assert!(t.hud_profile().iter().all(|p| p[1] == 50.0));
        assert_eq!(t.hud_max_speed(), 10.0);
    }
}
