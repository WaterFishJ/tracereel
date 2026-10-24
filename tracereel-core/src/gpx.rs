//! GPX parsing + per-sample metrics for the video HUD overlay.
//!
//! Reads a standard GPX 1.1 track (lat/lon/elevation/time, optional Garmin heart-rate
//! extension) and precomputes the derived metrics the HUD shows: instantaneous speed,
//! pace (min/km), cumulative distance, bearing (course degrees) and altitude.
//! `sample_at` maps a video timestamp to the nearest track sample.
//!
//! 手表在低功耗 / 自动暂停 / 信号不良时会**停止写点**，恢复后常常从新的 `<trkseg>`
//! 接着写，时间轴上留下 6~35 秒的空洞（位置却几乎没变）。这类中断在
//! [`GpxTrack::from_points`] 里按原有采样间隔补齐，HUD 的配速/距离/航向因此保持连续。
//!
//! GPX uses haversine distance with R = 6371000 (different from QuickRoute's R).

use std::path::Path;

use crate::route::RoutePoint;

const EARTH_RADIUS_GPX: f64 = 6_371_000.0;

/// 时间缺口小到什么程度就不值得补（小于 3 秒的抖动不影响 HUD 观感）。
const GAP_MIN_SECS: f64 = 3.0;
/// 超过这个跨度的中断按「真实中断」处理（隧道、手表关表），只记数、不伪造轨迹点。
const GAP_MAX_FILL_SECS: f64 = 60.0;
/// 缺口两端推算速度的上限（米/秒）：约 54 km/h，跑步/骑行都够，GPS 漂移和瞬移会被挡掉。
const GAP_MAX_FILL_SPEED: f64 = 15.0;
/// 缺口平均速度低于这个值（米/秒，约 3.6 km/h）时按「原地等待/自动暂停」处理：
/// 位置照样补连续，但速度记 0，HUD 显示「-- /km」，而不是算出一个几小时的配速。
const GAP_PAUSE_SPEED: f64 = 1.0;
/// 一条轨迹最多补多少点，防止病态文件把轨迹撑爆。
const GAP_MAX_INSERTED: usize = 2400;

/// 手表轨迹「记录中断」的补齐结果。
///
/// 部分手表（COROS、部分 Garmin 型号）在低功耗/自动暂停/信号不良时会停止写点，恢复后
/// 从新的 `<trkseg>` 或一个更大的时间间隔接着写。补齐后 HUD 的配速、距离、航向在中断
/// 期间仍然连续推进，不会原地冻结几秒到几十秒。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GapRepair {
    /// 已按采样间隔补点的中断处数。
    pub filled_gaps: usize,
    /// 补进去的轨迹点数。
    pub inserted_points: usize,
    /// 跨度或推算速度超限、保持原样的中断处数。
    pub skipped_gaps: usize,
}

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
    /// Recorded cadence per minute, unchanged (never implicitly doubled).
    pub cadence: Option<f64>,
}

/// A parsed GPX track with precomputed metrics.
#[derive(Debug, Clone)]
pub struct GpxTrack {
    pub points: Vec<GpxPoint>,
    pub has_hr: bool,
    pub start_time: f64,
    /// 这次解析补掉了多少处手表记录中断（见 [`GapRepair`]）。
    pub gap_repair: GapRepair,
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
    /// Recorded cadence per minute; absent/invalid data stays unavailable.
    pub cadence: Option<f64>,
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
    /// Build a HUD track from route points that already carry a time axis.
    ///
    /// This is the「只导入了 .qrt / 轨迹图」case: a QuickRoute project stores the whole
    /// recorded track (coordinates + 1 Hz sample times), so the HUD can show
    /// speed / pace / distance / heading without a separate GPX export. `t` is taken
    /// as seconds since the route's first sample — the same convention a GPX ends up
    /// with once it is anchored at its own start — so both sources are read through the
    /// same offset (`hud::hud_track_offset`, anchored at 比赛出发时刻).
    ///
    /// Altitude and heart rate stay **unavailable** (`ele` is NaN, `hr` is `None`):
    /// neither the `.qrt` scanner nor the QuickRoute-jpeg reader decodes them, and
    /// reporting 0 m of elevation would be a lie. Non-finite elevation also keeps the
    /// altitude profile empty instead of drawing a flat line at zero.
    ///
    /// `None` means "this route has no usable time axis" — a course drawn by hand has
    /// every `time` = 0, and the HUD must stay at "no track" there.
    pub fn from_route_points(points: &[RoutePoint]) -> Option<Self> {
        let mut pts: Vec<GpxPoint> = points
            .iter()
            .filter(|p| {
                p.time.is_finite()
                    && p.coordinate_x.is_finite()
                    && p.coordinate_y.is_finite()
                    && (p.coordinate_x != 0.0 || p.coordinate_y != 0.0)
            })
            .map(|p| GpxPoint {
                t: p.time,
                lat: p.coordinate_y,
                lon: p.coordinate_x,
                ele: f64::NAN,
                hr: None,
                cadence: None,
            })
            .collect();
        // Stable order by sample time: `sample_at`/`hud_at` binary-search the series.
        pts.sort_by(|a, b| a.t.total_cmp(&b.t));
        let (first, last) = (pts.first()?, pts.last()?);
        if pts.len() < 2 || last.t <= first.t {
            return None;
        }
        Self::from_points(pts).ok()
    }

    /// Compute derived metrics from raw points.
    ///
    /// 解析结果先按时间排好（`index_for_time` / `hud_at` 用二分查找，顺序错了会取到错误
    /// 样本），再把手表停止写点造成的**时间空洞**按原有采样间隔补成连续样本：位置线性
    /// 插值，速度取「缺口两端距离 ÷ 缺口时长」。只补可信的洞（跨度 ≤ [`GAP_MAX_FILL_SECS`]、
    /// 推算速度 ≤ [`GAP_MAX_FILL_SPEED`]）；隧道、关表这类真正的长中断保持原样，只在
    /// [`GpxTrack::gap_repair`] 里记一笔，绝不伪造几百个点。
    pub fn from_points(mut points: Vec<GpxPoint>) -> Result<Self, String> {
        if points.is_empty() {
            return Err("GPX has no valid track points".to_string());
        }
        points.sort_by(|a, b| a.t.total_cmp(&b.t));
        let (points, gap_repair, pause_mask) = repair_track_gaps(points);
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
        // 中断期是原地等待（自动暂停/红绿灯）：速度记 0，HUD 显示「-- /km」而不是几小时的
        // 配速；航向沿用上一个有效值，免得静止时航向乱跳。
        for (i, paused) in pause_mask.iter().enumerate() {
            if *paused && i > 0 {
                speed[i] = 0.0;
                bearing[i] = bearing[i - 1];
            }
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
        // 暂停样本的平滑速度也压成 0：否则窗口会混进缺口两侧的正常速度，在暂停边缘显示
        // 出一个「80 分钟/公里」的过渡配速。
        for (i, paused) in pause_mask.iter().enumerate() {
            if *paused {
                speed_smooth[i] = 0.0;
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
            gap_repair,
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
            cadence: self.points[i].cadence.filter(|v| v.is_finite() && *v >= 0.0),
        })
    }
}

/// 把时间轴上的「记录中断」补成连续样本。
///
/// 采样间隔取相邻点时间差的**中位数**（1 Hz 手表 = 1 秒，稀疏记录也能自适应），缺口阈值
/// 是该间隔的 2.5 倍且不小于 [`GAP_MIN_SECS`]。判定为缺口后按原间隔把中间点补齐：位置、
/// 海拔、心率线性插值。返回补好的点、统计，以及按位置对齐的「暂停样本」掩码（补出来的
/// 样本里，缺口平均速度低于 [`GAP_PAUSE_SPEED`] 的算作原地等待）。
fn repair_track_gaps(points: Vec<GpxPoint>) -> (Vec<GpxPoint>, GapRepair, Vec<bool>) {
    let mut repair = GapRepair::default();
    let pause_mask = vec![false; points.len()];
    if points.len() < 3 {
        return (points, repair, pause_mask);
    }
    let mut steps: Vec<f64> = points
        .windows(2)
        .map(|w| w[1].t - w[0].t)
        .filter(|d| d.is_finite() && *d > 0.0)
        .collect();
    if steps.is_empty() {
        return (points, repair, pause_mask);
    }
    steps.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let interval = steps[steps.len() / 2].clamp(0.2, 30.0);
    let gap_threshold = (interval * 2.5).max(GAP_MIN_SECS);

    let mut out: Vec<GpxPoint> = Vec::with_capacity(points.len() + 32);
    let mut mask: Vec<bool> = Vec::with_capacity(points.len() + 32);
    out.push(points[0]);
    mask.push(false);
    for i in 1..points.len() {
        let (a, b) = (points[i - 1], points[i]);
        let dt = b.t - a.t;
        // dt <= 0 说明时间乱序或重复：不补，原样保留。
        if dt > gap_threshold {
            let chord = hav(a.lat, a.lon, b.lat, b.lon);
            let steps_n = (dt / interval).round().max(2.0) as usize;
            let infill = steps_n - 1;
            let safe = dt <= GAP_MAX_FILL_SECS
                && chord / dt <= GAP_MAX_FILL_SPEED
                && repair.inserted_points + infill <= GAP_MAX_INSERTED;
            if safe {
                let paused = chord / dt < GAP_PAUSE_SPEED;
                for k in 1..steps_n {
                    let f = k as f64 / steps_n as f64;
                    out.push(GpxPoint {
                        t: a.t + dt * f,
                        lat: a.lat + (b.lat - a.lat) * f,
                        lon: a.lon + (b.lon - a.lon) * f,
                        ele: lerp_field(a.ele, b.ele, f),
                        // No cadence was recorded inside the gap: do not invent steps,
                        // especially across auto-pauses. Original endpoint values survive.
                        cadence: None,
                        hr: match (a.hr, b.hr) {
                            (Some(x), Some(y)) if x.is_finite() && y.is_finite() => {
                                Some((x + (y - x) * f).round())
                            }
                            _ => None,
                        },
                    });
                    mask.push(paused);
                }
                repair.filled_gaps += 1;
                repair.inserted_points += infill;
            } else {
                repair.skipped_gaps += 1;
            }
        }
        out.push(b);
        mask.push(false);
    }
    (out, repair, mask)
}

/// 海拔只在两端都有效时线性插值；一端缺失就保持缺失（`NaN`），不能凭空补 0。
fn lerp_field(a: f64, b: f64, f: f64) -> f64 {
    if a.is_finite() && b.is_finite() {
        a + (b - a) * f
    } else {
        f64::NAN
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
        let mut cadence: Option<f64> = None;
        for child in el.children() {
            if !child.is_element() {
                continue;
            }
            match child.tag_name().name() {
                "ele" => ele = child.text().and_then(|s| s.trim().parse().ok()),
                "time" => t = child.text().map(|s| parse_iso(s.trim())),
                "hr" => hr = child.text().and_then(|s| s.trim().parse().ok()),
                "cadence" | "cad" => cadence = parse_cadence(child.text()),
                _ => {
                    // Garmin extensions hold <hr> nested under <extensions>.
                    for cc in child.descendants() {
                        if cc.is_element() {
                            match cc.tag_name().name() {
                                "hr" => hr = cc.text().and_then(|s| s.trim().parse().ok()),
                                // roxmltree resolves namespaces: prefixes may vary.
                                "cadence" | "cad" => cadence = parse_cadence(cc.text()),
                                _ => {},
                            }
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
                    cadence,
                });
            }
        }
    }
    GpxTrack::from_points(points)
}

fn parse_cadence(text: Option<&str>) -> Option<f64> {
    text?.trim().parse::<f64>().ok().filter(|v| v.is_finite() && *v >= 0.0)
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

    fn cadence_gpx(values: &[&str]) -> String {
        let mut xml = String::from(r#"<gpx xmlns:gpxdata="http://www.cluetrust.com/XML/GPXDATA/1/0" xmlns:gpxtpx="http://www.garmin.com/xmlschemas/TrackPointExtension/v1"><trk><trkseg>"#);
        for (i, extension) in values.iter().enumerate() {
            xml.push_str(&format!("<trkpt lat=\"30\" lon=\"119\"><time>2026-10-06T02:00:{i:02}Z</time><extensions>{extension}</extensions></trkpt>"));
        }
        xml.push_str("</trkseg></trk></gpx>");
        xml
    }

    #[test]
    fn parses_gpxdata_and_garmin_cadence_without_doubling() {
        let track = parse_gpx_str(&cadence_gpx(&[
            "<gpxdata:cadence>115</gpxdata:cadence>",
            "<gpxtpx:TrackPointExtension><gpxtpx:cad>87</gpxtpx:cad></gpxtpx:TrackPointExtension>",
            "<gpxdata:cadence>0</gpxdata:cadence>",
        ])).unwrap();
        for (i, expected) in [115.0, 87.0, 0.0].iter().enumerate() {
            assert_eq!(track.points[i].cadence, Some(*expected));
            assert_eq!(track.sample_at(i as f64, 0.0).unwrap().cadence, Some(*expected));
        }
        assert_eq!(track.sample_at(0.0, 1.0).unwrap().cadence, Some(87.0));
        assert!(track.sample_at(3.0, 0.0).is_none());
    }

    #[test]
    fn missing_or_invalid_cadence_is_not_zero_or_carried_forward() {
        let track = parse_gpx_str(&cadence_gpx(&[
            "<gpxdata:cadence>115</gpxdata:cadence>", "",
            "<gpxtpx:cad>bad</gpxtpx:cad>", "<gpxtpx:cad>-1</gpxtpx:cad>",
            "<gpxtpx:cad>NaN</gpxtpx:cad>", "<gpxtpx:cad>inf</gpxtpx:cad>",
        ])).unwrap();
        for i in 1..6 {
            assert_eq!(track.points[i].cadence, None);
            assert_eq!(track.sample_at(i as f64, 0.0).unwrap().cadence, None);
        }
        let route = GpxTrack::from_route_points(&(0..4).map(|i| route_point(i, true)).collect::<Vec<_>>()).unwrap();
        assert_eq!(route.sample_at(1.0, 0.0).unwrap().cadence, None);
    }

    #[test]
    fn gap_repair_preserves_recorded_cadence_but_does_not_invent_it() {
        let mut points: Vec<_> = (0..10).map(|i| GpxPoint {
            t: 1000.0 + i as f64 + if i >= 5 { 10.0 } else { 0.0 },
            lat: 30.0, lon: 119.0, ele: 40.0, hr: None, cadence: Some(115.0),
        }).collect();
        let track = GpxTrack::from_points(points.clone()).unwrap();
        assert_eq!(track.gap_repair.inserted_points, 10);
        assert_eq!(track.sample_at(10.0, 0.0).unwrap().cadence, None);
        assert_eq!(track.sample_at(15.0, 0.0).unwrap().cadence, Some(115.0));
        points[0].cadence = Some(f64::NAN);
        assert_eq!(GpxTrack::from_points(points).unwrap().sample_at(0.0, 0.0).unwrap().cadence, None);
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
                    cadence: None,
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
                    cadence: None,
                })
                .collect(),
        )
        .unwrap();
        assert_eq!(t.hud_at(1.5, 0.0), Some((None, 0.0)));
        assert!(t.hud_profile().iter().all(|p| p[1] == 50.0));
        assert_eq!(t.hud_max_speed(), 10.0);
    }

    fn route_point(i: i64, with_time: bool) -> crate::route::RoutePoint {
        crate::route::RoutePoint {
            time: if with_time { i as f64 } else { 0.0 },
            coordinate_x: 113.0 + i as f64 * 0.0001,
            coordinate_y: 23.0,
            ..crate::route::RoutePoint::default()
        }
    }

    /// 「只导入了 .qrt」的工程：HUD 轨迹直接由轨迹点自带的秒级时间轴搭出来。
    #[test]
    fn route_points_become_a_hud_track() {
        let pts: Vec<_> = (0..5).map(|i| route_point(i, true)).collect();
        let track = GpxTrack::from_route_points(&pts).expect("有秒级时间轴就该给出 HUD 轨迹");
        assert_eq!(track.points.len(), 5);
        assert_eq!(track.start_time, 0.0);
        assert!(!track.has_hr);
        let s = track.sample_at(4.0, 0.0).expect("sample");
        assert!(s.speed_kmh > 20.0, "速度应由坐标差算出来：{}", s.speed_kmh);
        assert!(s.distance_km > 0.0);
        assert!(s.bearing.is_finite());
        // .qrt 不解码海拔/心率：不能假装是 0，也不能画出一条平的高度剖面。
        assert!(s.altitude_m.unwrap().is_nan());
        assert!(s.heart_rate.is_none());
        assert!(track.hud_profile().is_empty());
    }

    /// 手画的线路（每个点的 `time` 都是 0）没有配速可言 —— 必须保持「没有 HUD 轨迹」。
    #[test]
    fn drawn_course_has_no_hud_track() {
        let pts: Vec<_> = (0..4).map(|i| route_point(i, false)).collect();
        assert!(GpxTrack::from_route_points(&pts).is_none());
        assert!(GpxTrack::from_route_points(&pts[..1]).is_none());
        assert!(GpxTrack::from_route_points(&[]).is_none());
    }

    /// 造一条多段 GPX：每段是 `(相对秒数, 纬度)`，经度固定。
    fn gap_test_gpx(segments: &[Vec<(u32, f64)>]) -> String {
        let mut xml = String::from(
            r#"<?xml version="1.0"?><gpx version="1.1" xmlns="http://www.topografix.com/GPX/1/1"><trk>"#,
        );
        for seg in segments {
            xml.push_str("<trkseg>");
            for (t, lat) in seg {
                let (h, m, s) = (2 + t / 3600, (t % 3600) / 60, t % 60);
                xml.push_str(&format!(
                    "<trkpt lat=\"{lat:.7}\" lon=\"119.0\"><ele>40</ele>\
                     <time>2026-10-06T{h:02}:{m:02}:{s:02}Z</time></trkpt>"
                ));
            }
            xml.push_str("</trkseg>");
        }
        xml.push_str("</trk></gpx>");
        xml
    }

    /// 每秒约 3 米的纬度步长。
    const LAT_STEP: f64 = 0.000027;

    /// COROS 自动暂停的典型形状：中断 9 秒、两端只差 3 米 —— 必须补成连续样本。
    #[test]
    fn fills_paused_recording_gaps() {
        let seg0: Vec<(u32, f64)> = (0..10).map(|i| (i, 30.0 + i as f64 * LAT_STEP)).collect();
        let seg1: Vec<(u32, f64)> = (0..10)
            .map(|i| (18 + i, 30.0 + (10 + i) as f64 * LAT_STEP))
            .collect();
        let track = parse_gpx_str(&gap_test_gpx(&[seg0, seg1])).expect("parse");
        assert_eq!(
            track.gap_repair,
            GapRepair {
                filled_gaps: 1,
                inserted_points: 8,
                skipped_gaps: 0
            }
        );
        assert_eq!(track.points.len(), 28);
        for w in track.points.windows(2) {
            let dt = w[1].t - w[0].t;
            assert!(dt <= 1.0 + 1e-6, "中断里还留着 {dt} 秒的空洞");
        }
        // 原地等待补出来的样本速度记 0：HUD 显示「-- /km」，不会算出几小时的配速。
        let s = track.sample_at(13.0, 0.0).expect("缺口里应有样本");
        assert_eq!(s.speed_kmh, 0.0);
        assert_eq!(s.pace_min_km, 0.0);
    }

    /// 跑动中丢 GPS：中断 9 秒但位置按 3 m/s 正常前进 —— 补出来的速度必须也是 3 m/s，
    /// 不能把配速写成 0（那会让 HUD 在中断期间显示成「站着不动」）。
    #[test]
    fn fills_running_gps_outage_with_constant_speed() {
        let seg0: Vec<(u32, f64)> = (0..10).map(|i| (i, 30.0 + i as f64 * LAT_STEP)).collect();
        let seg1: Vec<(u32, f64)> = (0..10)
            .map(|i| (18 + i, 30.0 + (18 + i) as f64 * LAT_STEP))
            .collect();
        let track = parse_gpx_str(&gap_test_gpx(&[seg0, seg1])).expect("parse");
        assert_eq!(track.gap_repair.filled_gaps, 1);
        let s = track.sample_at(13.0, 0.0).expect("缺口里应有样本");
        assert!(
            (s.speed_kmh - 10.8).abs() < 1.0,
            "中断里的配速不对：{} km/h",
            s.speed_kmh
        );
    }

    /// 停表 10 分钟：绝不伪造 600 个点，只记一次「跳过」。
    #[test]
    fn leaves_long_outages_alone() {
        let seg0: Vec<(u32, f64)> = (0..10).map(|i| (i, 30.0 + i as f64 * LAT_STEP)).collect();
        let seg1: Vec<(u32, f64)> = (0..10)
            .map(|i| (610 + i, 30.0 + i as f64 * LAT_STEP))
            .collect();
        let track = parse_gpx_str(&gap_test_gpx(&[seg0, seg1])).expect("parse");
        assert_eq!(track.points.len(), 20);
        assert_eq!(
            track.gap_repair,
            GapRepair {
                filled_gaps: 0,
                inserted_points: 0,
                skipped_gaps: 1
            }
        );
    }

    /// 6 秒跳 500 米（约 83 m/s）：那是 GPS 漂移/瞬移，不是记录中断。
    #[test]
    fn does_not_bridge_gps_teleport() {
        let seg0: Vec<(u32, f64)> = (0..10).map(|i| (i, 30.0)).collect();
        let seg1: Vec<(u32, f64)> = (0..10).map(|i| (15 + i, 30.0045)).collect();
        let track = parse_gpx_str(&gap_test_gpx(&[seg0, seg1])).expect("parse");
        assert_eq!(track.points.len(), 20);
        assert_eq!(track.gap_repair.skipped_gaps, 1);
    }

    /// 连续记录（没有中断）必须原样保留，不能因为改动能被触发就多插点。
    #[test]
    fn leaves_continuous_track_untouched() {
        let seg: Vec<(u32, f64)> = (0..30).map(|i| (i, 30.0 + i as f64 * LAT_STEP)).collect();
        let track = parse_gpx_str(&gap_test_gpx(&[seg])).expect("parse");
        assert_eq!(track.points.len(), 30);
        assert_eq!(track.gap_repair, GapRepair::default());
    }

    /// 稀疏采样（5 秒一个点）也要自适应：20 秒中断按 5 秒间隔补 3 个点。
    #[test]
    fn adapts_to_sparse_sampling() {
        let seg0: Vec<(u32, f64)> = (0..5)
            .map(|i| (i * 5, 30.0 + i as f64 * 5.0 * LAT_STEP))
            .collect();
        let seg1: Vec<(u32, f64)> = (0..5)
            .map(|i| (40 + i * 5, 30.0 + (i as f64 + 9.0) * 5.0 * LAT_STEP))
            .collect();
        let track = parse_gpx_str(&gap_test_gpx(&[seg0, seg1])).expect("parse");
        assert_eq!(track.gap_repair.filled_gaps, 1);
        assert_eq!(track.gap_repair.inserted_points, 3);
        assert_eq!(track.points.len(), 13);
    }

    /// `.qrt` 轨迹自带的时间轴同样可能有中断（走的是同一个 `from_points`）。
    #[test]
    fn route_points_with_lost_samples_get_filled_too() {
        let mut pts: Vec<crate::route::RoutePoint> = (0..10)
            .map(|i| crate::route::RoutePoint {
                time: i as f64,
                coordinate_x: 113.0 + i as f64 * 0.00003,
                coordinate_y: 23.0,
                ..crate::route::RoutePoint::default()
            })
            .collect();
        for p in pts.iter_mut().skip(5) {
            p.time += 10.0;
        }
        let track = GpxTrack::from_route_points(&pts).expect("有秒级时间轴就该给出 HUD 轨迹");
        assert_eq!(track.gap_repair.filled_gaps, 1);
        assert_eq!(track.points.len(), 20);
    }
}
