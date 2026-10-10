//! QuickRoute `.qrt` project file reader.
//!
//! A `.qrt` file is a .NET `BinaryFormatter` stream written by QuickRoute 2.4.
//! It contains three things we care about:
//!
//! 1. **The map image, embedded in the file** (`Map.rawData`, a `byte[]` holding
//!    the original JPEG/PNG bytes).  It is *not* an external file reference.
//! 2. **The route waypoints** (longitude/latitude pairs with their sample
//!    times, recorded at roughly 1 Hz) plus the **handles** that georeference
//!    them onto the map (see [`QuickRouteProject::map_positions`]).
//! 3. **The laps** (`LapCollection`), i.e. the control punches — what the rest
//!    of the app calls "splits".  `HandleCollection` is *not* the splits.
//!
//! The object graph is decoded by scanning for the byte-level invariants of each
//! record rather than by implementing the full NRBF type system: the waypoints
//! are a fixed 25-byte record, the sample times a fixed 34-byte one, and a
//! handle is recognised from its map-pixel coordinates plus the fields written
//! immediately around them.  Every invariant here was verified byte-by-byte
//! against `demo/qrt_dump.ps1`'s independent .NET-side dump; see
//! `docs/qrt格式逆向笔记.md` for the whole trail, including why the naive
//! "count the handles" figure is inflated.

use crate::route::RoutePoint;

/// Errors from reading a `.qrt` project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QrtError {
    /// The file could not be read.
    Io(String),
    /// No embedded map image was found.
    NoMapImage,
}

impl std::fmt::Display for QrtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QrtError::Io(e) => write!(f, "read error: {e}"),
            QrtError::NoMapImage => write!(f, "no embedded map image found"),
        }
    }
}

impl std::error::Error for QrtError {}

/// Image container format of the embedded map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapImageKind {
    Jpeg,
    Png,
}

impl MapImageKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MapImageKind::Jpeg => "jpeg",
            MapImageKind::Png => "png",
        }
    }
}

/// The embedded map image.
#[derive(Debug, Clone)]
pub struct MapImage {
    /// Raw image bytes (a complete, self-contained JPEG or PNG).
    pub bytes: Vec<u8>,
    pub kind: MapImageKind,
    pub width: u32,
    pub height: u32,
}

/// One route waypoint recovered from the project's waypoint array.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QrtWaypoint {
    /// Longitude in degrees.
    pub longitude: f64,
    /// Latitude in degrees.
    pub latitude: f64,
    /// Index of this waypoint inside the original route.
    pub index: u32,
    /// Sample time as .NET `DateTime.ToBinary()` ticks, or 0 when it could not
    /// be recovered.
    pub time_ticks: i64,
}

/// `.NET DateTime` ticks are 100 ns units since 0001-01-01.
pub const TICKS_PER_MS: i64 = 10_000;

/// Range that a plausible serialized `DateTime` falls into.
///
/// `DateTime.ToBinary()` stores the raw ticks offset by the "local kind" bias
/// and then ORs in the 2-bit kind, so a UTC sample from a modern track lands
/// around `5.25e18` (the demo project's are `5.250928608e18`).  The window spans
/// roughly 2020 through 2100.  Values outside it — image bytes, the gradient
/// table's own numbers — are rejected.
const TICKS_LO: i64 = 5_250_000_000_000_000_000;
const TICKS_HI: i64 = 5_260_000_000_000_000_000;

/// Tick offset between a `DateTime`'s raw ticks and its serialized value.
///
/// Equals `(bias << 62)` for `DateTimeKind.Utc`; it is constant for every
/// sample in a project, so subtracting it recovers the wall-clock ticks.
const TICKS_KIND_BIAS: i64 = 4_611_685_998_607_387_904;

/// Distance between two consecutive waypoint time records.
///
/// Each entry is the tick value plus the altitude/optional-field markers, and
/// the altitude double when present: 34 bytes for the demo project's points,
/// and the one spare byte at the end keeps the next record aligned.
const TIME_RECORD_LEN: usize = 34;

/// Recover the waypoint sample times.
///
/// QuickRoute stores them in a second array, in the same order as the
/// coordinates: each entry is the 8-byte tick value with the `DateTimeKind`
/// bits, followed by the altitude/optional-field markers and, when present, an
/// 8-byte altitude double — 34 bytes in total for a plain point.  Walking the
/// chain in order (each next time is within a few seconds of the previous one)
/// recovers all 1475 samples of the demo project exactly.
/// Recover the waypoint sample times.
///
/// QuickRoute stores them in a second array, in the same order as the
/// coordinates. Each entry is the serialized 8-byte `DateTime` followed by the
/// altitude/optional-field markers and (when present) the altitude double — 34
/// bytes in total — and the next entry begins immediately after. So a real
/// sample is a timestamp that has a matching timestamp exactly
/// [`TIME_RECORD_LEN`] bytes later, at most a few seconds further on; that
/// rejects the look-alike values scattered through the rest of the object
/// graph.
pub fn scan_waypoint_times(data: &[u8]) -> Vec<i64> {
    let tick = |p: usize| -> Option<i64> {
        data.get(p..p + 8)
            .map(|s| i64::from_le_bytes(s.try_into().unwrap()))
    };
    let plausible = |v: i64| (TICKS_LO..TICKS_HI).contains(&v);

    // Positions whose record runs to the next sample, i.e. a timestamp here with
    // another one `TIME_RECORD_LEN` bytes later. The time gap may be large — a
    // paused or sparsely-sampled track has multi-second (and longer) breaks —
    // so only "strictly later and not absurd" is required; the fixed record
    // stride is what actually identifies the array.
    let mut anchors: Vec<usize> = Vec::new();
    let mut p = 0usize;
    while p + TIME_RECORD_LEN + 8 <= data.len() {
        let (Some(a), Some(b)) = (tick(p), tick(p + TIME_RECORD_LEN)) else {
            break;
        };
        if plausible(a) && plausible(b) && b > a && b - a <= 3600 * TICKS_PER_MS * 1000 {
            anchors.push(p);
            p += TIME_RECORD_LEN;
            continue;
        }
        p += 1;
    }

    // The route's own array is the longest set of anchors exactly one record
    // apart.
    let mut best_start = 0usize;
    let mut best_len = 0usize;
    let mut i = 0usize;
    while i < anchors.len() {
        let mut j = i;
        while j + 1 < anchors.len() && anchors[j + 1] - anchors[j] == TIME_RECORD_LEN {
            j += 1;
        }
        let len = j - i + 1;
        if len > best_len {
            best_len = len;
            best_start = i;
        }
        i = j + 1;
    }
    if best_len == 0 {
        return Vec::new();
    }

    let mut out = Vec::with_capacity(best_len);
    let base = anchors[best_start];
    for k in 0..best_len {
        if let Some(t) = tick(base + k * TIME_RECORD_LEN) {
            out.push(t - TICKS_KIND_BIAS);
        }
    }
    out
}

/// Everything we could extract from a `.qrt` project.
#[derive(Debug, Clone, Default)]
pub struct QuickRouteProject {
    pub map: Option<MapImage>,
    pub waypoints: Vec<QrtWaypoint>,
    /// `true` when the waypoint array starts at index 0.
    pub waypoints_complete: bool,
    /// Number of splits in the project.
    ///
    /// QuickRoute always writes a `Start` and a `Stop` lap, so this is the count
    /// of laps excluding `Start` — and it stays `0` for a project where the user
    /// never placed a split at all (which is how
    /// `demo/AM 男子短距 第二面.qrt` is set up). Use it, not the handle count,
    /// to decide whether to offer the "import splits?" prompt.
    pub split_count: usize,
    /// Whether the file's type table mentions `HandleCollection` at all.
    ///
    /// This is only a cheap structural probe: every QuickRoute project has a
    /// handle collection, so it is `true` even when the project has no splits.
    /// Prefer [`Self::has_splits`].
    pub has_split_data: bool,
    /// The decoded `Handle`s (the draggable markers drawn on the map).
    pub handles: Vec<QrtHandle>,
    /// The decoded `Lap`s (QuickRoute's own split points), time-ordered.
    pub laps: Vec<QrtLap>,
    /// `Document.projectionOrigin` — the longitude/latitude that the projection
    /// from earth to map is centred on. Required to turn waypoint coordinates
    /// into map pixels; `None` when the value could not be located.
    pub projection_origin: Option<(f64, f64)>,
}

impl QuickRouteProject {
    /// True when the project carries splits the user placed.
    pub fn has_splits(&self) -> bool {
        self.split_count > 0
    }

    /// Orthographically project every waypoint around `projectionOrigin`.
    ///
    /// Returns offsets in **metres** relative to the origin (+east, +north), or
    /// an empty vector when the projection origin is unknown.  These are the
    /// coordinates QuickRoute's `Handle.TransformationMatrix` values expect.
    pub fn projected_positions(&self) -> Vec<(f64, f64)> {
        let Some((olon, olat)) = self.projection_origin else {
            return Vec::new();
        };
        // QuickRoute uses the same orthographic projection as the QuickRoute
        // jpeg path (`crate::math::project_coordinate`), not a flat-earth
        // metres-per-degree approximation.
        self.waypoints
            .iter()
            .map(|w| crate::math::project_coordinate(w.longitude, w.latitude, olon, olat))
            .collect()
    }

    /// The `Handle` records that contribute a transformation, in route order.
    ///
    /// The original reader only ever uses the first map segment
    /// (`QuickRouteReader::readHandles`), and the handles are ordered by
    /// `ParameterizedLocation`, which is exactly the order [`scan_handles`]
    /// returns them in.
    pub fn ordered_handles(&self) -> Vec<QrtHandle> {
        self.handles
            .iter()
            .copied()
            .filter(|h| h.segment_index == 0)
            .collect()
    }

    /// One pixel transform per handle, in `ordered_handles()` order.
    ///
    /// QuickRoute stores a `TransformationMatrix` on every handle, but it is
    /// always *derived*: moving a handle rewrites the matrix of that handle and
    /// of its predecessor so that the pair of handles stays mapped onto their
    /// recorded pixel positions (`Session.UpdateTransformationMatrices`).  The
    /// matrix is therefore reproducible from the handle positions alone, which
    /// is what this does — it avoids having to decode `GeneralMatrix` objects
    /// (a 3×3 `double[][]` buried in the object graph at no fixed offset from
    /// its handle).
    pub fn handle_transforms(&self, projected: &[(f64, f64)]) -> Vec<PixelTransform> {
        let handles = self.ordered_handles();
        let map = self.map.as_ref();
        let mut out = Vec::with_capacity(handles.len());
        if handles.is_empty() || projected.len() < 2 {
            return out;
        }
        let Some(initial) = map.and_then(|m| {
            initial_pixel_transform(projected, m.width as f64, m.height as f64)
        }) else {
            return out;
        };

        if handles.len() == 1 {
            // `Session.UpdateTransformationMatrices`, single-handle case: the
            // whole route (including the initial fit) is translated so that the
            // handle lands on its recorded pixel position.
            let h = &handles[0];
            let here = initial.apply(project_at(projected, h.value));
            out.push(PixelTransform {
                a: initial.a,
                b: initial.b,
                tx: initial.tx + (h.x - here.0),
                ty: initial.ty + (h.y - here.1),
            });
            return out;
        }

        for i in 0..handles.len() - 1 {
            let h0 = &handles[i];
            let h1 = &handles[i + 1];
            let p0 = project_at(projected, h0.value);
            let p1 = project_at(projected, h1.value);
            match pair_pixel_transform(p0, (h0.x, h0.y), p1, (h1.x, h1.y)) {
                Some(m) => out.push(m),
                // Degenerate pair (two handles on the same projected point):
                // QuickRoute would collapse the route onto one pixel, which is
                // never what the user wants, so keep the previous transform.
                None => out.push(*out.last().unwrap_or(&initial)),
            }
        }
        // The last handle's own matrix is never applied by the original reader
        // ("do not use the last handle at all"), so mirroring the previous one
        // is exact rather than an approximation.
        if let Some(last) = out.last().copied() {
            out.push(last);
        }
        out
    }

    /// Which handle transform applies to each route point.
    ///
    /// A port of `QuickRouteReader::processRoutePoints`: the transform of a
    /// handle starts applying *after* the route point it is attached to, the
    /// first handle covers everything before the second one, and the last
    /// handle is never used at all ("do not use the last handle at all, use
    /// second to last handle instead for the last points").
    ///
    /// Returns an index into [`Self::handle_transforms`] per route point, or an
    /// empty vector when there is no usable handle chain (the caller then uses
    /// the session's initial fit).
    pub fn transform_indices(&self, route_points: usize) -> Vec<usize> {
        let handles = self.ordered_handles();
        let n = handles.len();
        if n < 2 {
            return Vec::new();
        }
        let mut out = Vec::with_capacity(route_points);
        let mut handle_index = 1usize;
        let mut current = 0usize;
        let mut next_rpi = handles[1].value;
        for i in 0..route_points {
            if (i as f64) > next_rpi {
                handle_index += 1;
                if handle_index < n {
                    current = handle_index - 1;
                    next_rpi = handles[handle_index].value;
                } else {
                    next_rpi = f64::MAX;
                }
            }
            out.push(current);
        }
        out
    }

    /// Map every waypoint to map-pixel coordinates, **centred on the map**.
    ///
    /// This is a port of QuickRoute's `Session.CreateAdjustedRoute`: each
    /// waypoint is transformed with the `TransformationMatrix` of the handle
    /// that precedes it on the route (or the session's initial fit when there
    /// are no handles), and the last handle is never used.
    ///
    /// The renderer draws the map image centred on the panel origin, so the
    /// returned coordinates have the map centre subtracted — the same convention
    /// `crate::quickroute::parse_quickroute` uses for exported QuickRoute
    /// jpegs.
    ///
    /// Returns `(x, y, index)` triples, or an empty vector when the projection
    /// origin or the map size is unavailable.
    pub fn map_positions(&self) -> Vec<(f64, f64, u32)> {
        let Some(map) = self.map.as_ref() else {
            return Vec::new();
        };
        let projected = self.projected_positions();
        if self.waypoints.is_empty() || projected.is_empty() || map.width == 0 || map.height == 0 {
            return Vec::new();
        }
        let (w, h) = (map.width as f64, map.height as f64);
        let (cx, cy) = (w / 2.0, h / 2.0);

        let transforms = self.handle_transforms(&projected);
        let indices = self.transform_indices(projected.len());
        if indices.len() != projected.len() || indices.is_empty() {
            // No usable handle chain: QuickRoute falls back to the session's
            // initial fit for the whole route.
            let Some(initial) = initial_pixel_transform(&projected, w, h) else {
                return Vec::new();
            };
            return projected
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let q = initial.apply(*p);
                    (q.0 - cx, q.1 - cy, self.waypoints[i].index)
                })
                .collect();
        }

        projected
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let q = transforms[indices[i]].apply(*p);
                (q.0 - cx, q.1 - cy, self.waypoints[i].index)
            })
            .collect()
    }

    /// Total route length in map pixels, summed along the recovered track.
    pub fn route_length(&self) -> f64 {
        let pos = self.map_positions();
        let mut total = 0.0;
        for w in pos.windows(2) {
            let dx = w[1].0 - w[0].0;
            let dy = w[1].1 - w[0].1;
            let d = (dx * dx + dy * dy).sqrt();
            if d.is_finite() {
                total += d;
            }
        }
        total
    }

    /// Convert the recovered laps into split offsets, in milliseconds from the
    /// route start.
    ///
    /// QuickRoute writes a `Start` lap and a `Stop` lap for every project, so
    /// only a project that also carries `Lap` entries has splits the user
    /// actually placed: `demo/AM 男子短距 第二面.qrt` has just start+stop and
    /// yields none, while `第一面` has 27 `Lap` entries plus the finish.
    ///
    /// The returned list is the segment **end** times (the departure is not a
    /// segment), i.e. every lap except `Start`, which is 28 entries for `第一面`.
    pub fn split_times_ms(&self) -> Vec<i64> {
        if !self.laps.iter().any(|l| l.kind == LAP_SPLIT) {
            return Vec::new();
        }
        let Some(t0) = self.laps.iter().map(|l| l.time_ticks).min() else {
            return Vec::new();
        };
        let mut out: Vec<i64> = self
            .laps
            .iter()
            .filter(|l| l.kind != LAP_START)
            .map(|l| (l.time_ticks - t0) / TICKS_PER_MS)
            .filter(|ms| *ms > 0)
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Convert the recovered waypoints into the core route-point model.
    ///
    /// `position_x` / `position_y` are filled with map-pixel coordinates when the
    /// projection origin is known (see [`Self::map_positions`]); without it the
    /// route cannot be drawn on the map, so the caller should treat empty
    /// positions as "map unavailable".
    ///
    /// `time` is in **seconds** since the first sample and `date_time_ms` in
    /// milliseconds, matching what `parse_quickroute` produces for an exported
    /// QuickRoute jpeg — the route manager resamples at 1-second intervals and
    /// indexes split times in seconds, so getting this unit wrong collapses the
    /// whole route into a single step.
    pub fn to_route_points(&self) -> Vec<RoutePoint> {
        let positions = self.map_positions();
        let t0 = self
            .waypoints
            .iter()
            .find(|w| w.time_ticks > 0)
            .map(|w| w.time_ticks)
            .unwrap_or(0);
        self.waypoints
            .iter()
            .enumerate()
            .map(|(i, w)| {
                let (px, py) = positions
                    .get(i)
                    .map(|(x, y, _)| (*x, *y))
                    .unwrap_or((0.0, 0.0));
                let ms = if w.time_ticks > 0 && t0 > 0 {
                    (w.time_ticks - t0) / TICKS_PER_MS
                } else {
                    0
                };
                RoutePoint {
                    coordinate_x: w.longitude,
                    coordinate_y: w.latitude,
                    position_x: px,
                    position_y: py,
                    date_time_ms: ms as f64,
                    time: ms as f64 / 1000.0,
                    ..RoutePoint::default()
                }
            })
            .collect()
    }
}

// ------------------------------------------------------ route ↔ pixel geometry

/// A 3×3 transform mapping projected metres to map **pixels**.
///
/// Every matrix QuickRoute writes for a handle has the shape
/// `[[a, b, tx], [b, -a, ty], [0, 0, 1]]` — a rotation plus a uniform scale plus
/// the y flip that turns north-up metres into y-down pixels (verified against
/// the demo project: the 9 stored doubles are `a, b, tx, b, -a, ty, 0, 0, 1`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PixelTransform {
    /// `m00`, i.e. scale × cos(rotation).
    pub a: f64,
    /// `m01`, i.e. −scale × sin(rotation).
    pub b: f64,
    /// `m02`, the pixel x of the projected origin.
    pub tx: f64,
    /// `m12`, the pixel y of the projected origin.
    pub ty: f64,
}

impl PixelTransform {
    /// Apply the transform to a projected (metre) coordinate.
    pub fn apply(&self, p: (f64, f64)) -> (f64, f64) {
        (
            self.a * p.0 + self.b * p.1 + self.tx,
            self.b * p.0 - self.a * p.1 + self.ty,
        )
    }

    /// Uniform pixel scale.
    pub fn scale(&self) -> f64 {
        (self.a * self.a + self.b * self.b).sqrt()
    }
}

/// The projected location of the route point at a `ParameterizedLocation`.
///
/// `value` is a **fractional route-point index** inside the segment (not a
/// distance in metres); a fractional value interpolates between the two
/// surrounding points — `Route.GetProjectedLocationFromParameterizedLocation`.
pub fn project_at(projected: &[(f64, f64)], value: f64) -> (f64, f64) {
    if projected.is_empty() {
        return (0.0, 0.0);
    }
    if projected.len() == 1 || !value.is_finite() {
        return projected[0];
    }
    let mut i = value.floor();
    if i < 0.0 {
        i = 0.0;
    }
    if i >= (projected.len() - 1) as f64 {
        i = (projected.len() - 2) as f64;
    }
    let idx = i as usize;
    let d = value - i;
    let (p0, p1) = (projected[idx], projected[idx + 1]);
    (p0.0 + d * (p1.0 - p0.0), p0.1 + d * (p1.1 - p0.1))
}

/// Angle of a vector in radians within `(-π, π]` — `LinearAlgebraUtil.GetAngleR`.
fn angle_r(x: f64, y: f64) -> f64 {
    let len = (x * x + y * y).sqrt();
    if len == 0.0 {
        return 0.0;
    }
    let dp = (x / len).clamp(-1.0, 1.0);
    let mut a = if y < 0.0 {
        2.0 * std::f64::consts::PI - dp.acos()
    } else {
        dp.acos()
    };
    if a > std::f64::consts::PI {
        a -= 2.0 * std::f64::consts::PI;
    }
    a
}

/// Signed angle from `v0` to `v1` — `LinearAlgebraUtil.GetAngleR(v0, v1)`.
fn angle_between(v0: (f64, f64), v1: (f64, f64)) -> f64 {
    let a0 = angle_r(v0.0, v0.1);
    let a1 = angle_r(v1.0, v1.1) + 2.0 * std::f64::consts::PI;
    let mut diff = (a1 - a0) % (2.0 * std::f64::consts::PI);
    if diff > std::f64::consts::PI {
        diff -= 2.0 * std::f64::consts::PI;
    }
    diff
}

/// The similarity that maps `p0 → q0` and `p1 → q1`, with the y flip applied to
/// the pixel side — `LinearAlgebraUtil.CalculateTransformationMatrix(…, useRotation: true)`.
///
/// Returns `None` when the two route points coincide (the original would then
/// produce a degenerate zero-scale matrix).
pub fn pair_pixel_transform(
    p0: (f64, f64),
    q0: (f64, f64),
    p1: (f64, f64),
    q1: (f64, f64),
) -> Option<PixelTransform> {
    let len_p = ((p1.0 - p0.0).powi(2) + (p1.1 - p0.1).powi(2)).sqrt();
    if !(len_p > 1e-9) {
        return None;
    }
    let len_q = ((q1.0 - q0.0).powi(2) + (q1.1 - q0.1).powi(2)).sqrt();
    let s = len_q / len_p;
    let theta = angle_between((p1.0 - p0.0, p1.1 - p0.1), (q1.0 - q0.0, -(q1.1 - q0.1)));
    let a = s * theta.cos();
    let b = -s * theta.sin();
    Some(PixelTransform {
        a,
        b,
        tx: q0.0 - (a * p0.0 + b * p0.1),
        ty: q0.1 - (b * p0.0 - a * p0.1),
    })
}

/// The session's initial fit: the route's projected bounding box mapped onto the
/// middle 75 % of the map, preserving the route's aspect ratio.
///
/// This is `RouteAdjustmentManager.CreateInitialTransformationMatrix`, which
/// QuickRoute applies to a project before any handle exists.  It is computed
/// rather than read from the file because it is a pure function of the route,
/// the projection origin and the map size.
///
/// Note `useRotation: false`: the initial fit scales x and y independently, so
/// the result is a diagonal matrix (`b == 0`).
pub fn initial_pixel_transform(
    projected: &[(f64, f64)],
    map_w: f64,
    map_h: f64,
) -> Option<PixelTransform> {
    if projected.is_empty() || !(map_w > 0.0) || !(map_h > 0.0) {
        return None;
    }
    let (mut min_x, mut min_y) = (f64::INFINITY, f64::INFINITY);
    let (mut max_x, mut max_y) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for p in projected {
        if !p.0.is_finite() || !p.1.is_finite() {
            continue;
        }
        min_x = min_x.min(p.0);
        max_x = max_x.max(p.0);
        min_y = min_y.min(p.1);
        max_y = max_y.max(p.1);
    }
    let (route_w, route_h) = (max_x - min_x, max_y - min_y);
    if !(route_w > 1e-9) || !(route_h > 1e-9) {
        return None;
    }

    // mapRectangle: the inner 3/4 of the map, widened or narrowed to the
    // route's aspect ratio around its centre.
    let mut left = map_w / 8.0;
    let mut top = map_h / 8.0;
    let (mut width, mut height) = (map_w * 3.0 / 4.0, map_h * 3.0 / 4.0);
    let route_ratio = route_w / route_h;
    if width / height < route_ratio {
        let new_h = width / route_ratio;
        top += height / 2.0 - new_h / 2.0;
        height = new_h;
    } else {
        let new_w = height * route_ratio;
        left += width / 2.0 - new_w / 2.0;
        width = new_w;
    }

    // routeRectangle.LowerLeft → mapRectangle.UpperLeft and
    // routeRectangle.UpperRight → mapRectangle.LowerRight.
    let p0 = (min_x, max_y);
    let p1 = (max_x, min_y);
    let q0 = (left, top);
    let q1 = (left + width, top + height);

    let sx = (q1.0 - q0.0) / (p1.0 - p0.0);
    let tx = q0.0 - sx * p0.0;
    let sy = (q1.1 - q0.1) / (p1.1 - p0.1);
    let ty = q0.1 - sy * p0.1;
    // The aspect-ratio fit above makes `sx == -sy` exactly, so the diagonal fit
    // is representable in the same `[[a, b], [b, -a]]` shape: `apply` then uses
    // `-a == sy` for the y half.
    Some(PixelTransform {
        a: sx,
        b: 0.0,
        tx,
        ty,
    })
}

// ---------------------------------------------------------------- image scan

/// Walk a JPEG marker chain from `data[start]` (must be a SOI) and return
/// `(total_len, width, height)`.
///
/// This deliberately does **not** use `find(b"\xff\xd9")`: a `.qrt` header
/// embeds a Photoshop thumbnail whose entropy data contains `FF FF D9`, which
/// would truncate the map to a few kilobytes.
fn jpeg_extent(data: &[u8], start: usize) -> Option<(usize, u32, u32)> {
    if data.len() < start + 2 || data[start] != 0xFF || data[start + 1] != 0xD8 {
        return None;
    }
    let mut p = start + 2;
    let (mut w, mut h) = (0u32, 0u32);
    while p + 1 < data.len() {
        if data[p] != 0xFF {
            return None;
        }
        let m = data[p + 1];
        if m == 0xFF {
            p += 1;
            continue;
        }
        if m == 0xD8 || m == 0x01 || (0xD0..=0xD7).contains(&m) {
            p += 2;
            continue;
        }
        if m == 0xD9 {
            return Some((p + 2 - start, w, h));
        }
        if p + 4 > data.len() {
            return None;
        }
        let seg_len = u16::from_be_bytes([data[p + 2], data[p + 3]]) as usize;
        if m == 0xDA {
            // Start of scan: skip entropy-coded data until the next real marker.
            p += 2 + seg_len;
            while p + 1 < data.len() {
                let is_marker = data[p] == 0xFF
                    && data[p + 1] != 0x00
                    && !(0xD0..=0xD7).contains(&data[p + 1]);
                if is_marker {
                    break;
                }
                p += 1;
            }
            continue;
        }
        if (0xC0..=0xCF).contains(&m) && m != 0xC4 && m != 0xC8 && m != 0xCC {
            if p + 9 > data.len() {
                return None;
            }
            h = u16::from_be_bytes([data[p + 5], data[p + 6]]) as u32;
            w = u16::from_be_bytes([data[p + 7], data[p + 8]]) as u32;
        }
        p += 2 + seg_len;
    }
    None
}

/// Walk PNG chunks from `data[start]` and return `(total_len, width, height)`.
fn png_extent(data: &[u8], start: usize) -> Option<(usize, u32, u32)> {
    const SIG: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if data.len() < start + 8 || &data[start..start + 8] != SIG {
        return None;
    }
    let mut p = start + 8;
    let (mut w, mut h) = (0u32, 0u32);
    while p + 12 <= data.len() {
        let len = u32::from_be_bytes([data[p], data[p + 1], data[p + 2], data[p + 3]]) as usize;
        let typ = &data[p + 4..p + 8];
        if typ == b"IHDR" && p + 16 <= data.len() {
            w = u32::from_be_bytes([data[p + 8], data[p + 9], data[p + 10], data[p + 11]]);
            h = u32::from_be_bytes([data[p + 12], data[p + 13], data[p + 14], data[p + 15]]);
        }
        p += 12 + len;
        if typ == b"IEND" {
            return Some((p - start, w, h));
        }
    }
    None
}

/// Find the largest embedded JPEG or PNG and return it.
pub fn find_map_image(data: &[u8]) -> Option<MapImage> {
    const MIN_SIZE: usize = 10_000;
    let mut best: Option<MapImage> = None;

    let mut i = 0;
    while let Some(pos) = find_from(data, b"\xff\xd8\xff", i) {
        if let Some((len, w, h)) = jpeg_extent(data, pos) {
            if len >= MIN_SIZE && best.as_ref().map_or(true, |b| len > b.bytes.len()) {
                best = Some(MapImage {
                    bytes: data[pos..pos + len].to_vec(),
                    kind: MapImageKind::Jpeg,
                    width: w,
                    height: h,
                });
            }
        }
        i = pos + 1;
    }

    let mut i = 0;
    while let Some(pos) = find_from(data, b"\x89PNG\r\n\x1a\n", i) {
        if let Some((len, w, h)) = png_extent(data, pos) {
            if len >= MIN_SIZE && best.as_ref().map_or(true, |b| len > b.bytes.len()) {
                best = Some(MapImage {
                    bytes: data[pos..pos + len].to_vec(),
                    kind: MapImageKind::Png,
                    width: w,
                    height: h,
                });
            }
        }
        i = pos + 1;
    }

    best
}

fn find_from(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if from >= hay.len() || needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    hay[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

// ------------------------------------------------------------- waypoint scan

/// The waypoint records QuickRoute stores for the route tail are fixed-size:
///
/// ```text
/// +0    8B   longitude  (f64 little-endian)
/// +8    8B   latitude   (f64 little-endian)
/// +16   1B   constant 0x01
/// +17   4B   waypoint index (i32 little-endian)
/// +21   4B   constant 0
/// ```
pub const WAYPOINT_RECORD_LEN: usize = 25;

/// Latitude/longitude window used to reject false positives.
pub const COORD_LIMIT: f64 = 180.0;

/// Scan `data` for fixed-size waypoint records.
///
/// The records are consecutive, so once one is matched we skip a whole record;
/// this keeps the scan linear and avoids mis-aligning on the coordinates.
/// Scan `data` for fixed-size waypoint records inside a coordinate window.
///
/// The window matters: a `.qrt` object graph also contains marker-drawer and
/// gradient tables whose doubles look like small coordinates (e.g. `12, 3` or
/// `-10, 10`), and a global scan picks those up as false waypoints.
pub fn scan_waypoints_in(data: &[u8], lon_window: (f64, f64), lat_window: (f64, f64)) -> Vec<QrtWaypoint> {
    let mut out = Vec::new();
    let mut off = 0usize;
    while off + WAYPOINT_RECORD_LEN <= data.len() {
        let lon = f64::from_le_bytes(data[off..off + 8].try_into().unwrap());
        if lon > lon_window.0 && lon < lon_window.1 && lon.abs() > 1e-6 && data[off + 16] == 0x01 {
            let lat = f64::from_le_bytes(data[off + 8..off + 16].try_into().unwrap());
            if lat > lat_window.0 && lat < lat_window.1 && lat.abs() > 1e-6 {
                let idx = i32::from_le_bytes(data[off + 17..off + 21].try_into().unwrap());
                if (0..1_000_000).contains(&idx) {
                    out.push(QrtWaypoint {
                        longitude: lon,
                        latitude: lat,
                        index: idx as u32,
                        time_ticks: 0,
                    });
                    off += WAYPOINT_RECORD_LEN;
                    continue;
                }
            }
        }
        off += 1;
    }
    out.sort_by_key(|w| w.index);
    out
}

/// Two-pass waypoint recovery.
///
/// The first pass uses the full valid coordinate range; the median of what it
/// finds is taken as the map centre, and a ±0.5° window around that median is
/// rescanned.  This is the same strategy the reference implementation uses and
/// it removes the marker/gradient false positives.
pub fn scan_waypoints(data: &[u8]) -> Vec<QrtWaypoint> {
    let loose = scan_waypoints_in(data, (-COORD_LIMIT, COORD_LIMIT), (-COORD_LIMIT, COORD_LIMIT));
    if loose.is_empty() {
        return loose;
    }
    let mut lons: Vec<f64> = loose.iter().map(|w| w.longitude).collect();
    let mut lats: Vec<f64> = loose.iter().map(|w| w.latitude).collect();
    lons.sort_by(|a, b| a.partial_cmp(b).unwrap());
    lats.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let clon = lons[lons.len() / 2];
    let clat = lats[lats.len() / 2];
    const HALF: f64 = 0.5;
    scan_waypoints_in(
        data,
        (clon - HALF, clon + HALF),
        (clat - HALF, clat + HALF),
    )
}

// ------------------------------------------------------------------ entry

/// Read a `.qrt` project from a file path.
pub fn read_project<P: AsRef<std::path::Path>>(path: P) -> Result<QuickRouteProject, QrtError> {
    let path = path.as_ref();
    let data = std::fs::read(path).map_err(|e| QrtError::Io(e.to_string()))?;
    Ok(read_project_bytes(&data))
}

/// Read a `.qrt` project from an in-memory buffer.
pub fn read_project_bytes(data: &[u8]) -> QuickRouteProject {
    let map = find_map_image(data);
    let mut waypoints = scan_waypoints(data);
    let complete = waypoints.first().map_or(true, |w| w.index == 0);

    // Attach sample times. The time array is written in the same order as the
    // coordinates, so the two line up index by index; if one sample could not be
    // recovered the remainder still lines up from the start.
    let times = scan_waypoint_times(data);
    for (w, t) in waypoints.iter_mut().zip(times) {
        w.time_ticks = t;
    }

    let mut proj = QuickRouteProject {
        map,
        waypoints,
        waypoints_complete: complete,
        split_count: 0,
        has_split_data: detect_split_data(data),
        handles: Vec::new(),
        laps: Vec::new(),
        projection_origin: find_projection_origin(data),
    };

    let (mw, mh) = proj
        .map
        .as_ref()
        .map(|m| (m.width, m.height))
        .unwrap_or((0, 0));
    proj.handles = scan_handles(data, mw, mh);

    // Splits come from the laps: the handle collection holds the draggable map
    // markers, while a lap is a punched split.
    proj.laps = scan_laps(data);
    proj.split_count = proj.split_times_ms().len();
    proj
}

/// Locate `Document.projectionOrigin`.
///
/// The object graph writes the `LongLat` members in declaration order
/// (`longitude`, then `latitude`), and immediately after the latitude double
/// comes the embedded map JPEG.  So the origin is the last `(lon, lat)` double
/// pair that sits right before a JPEG/PNG signature and is close to the track
/// itself — which makes this both simple and robust.
pub fn find_projection_origin(data: &[u8]) -> Option<(f64, f64)> {
    // Bound the search to the region before the embedded image: that is where
    // the document header fields live.
    let image_start = find_from(data, b"\xff\xd8\xff", 0)
        .into_iter()
        .chain(find_from(data, b"\x89PNG\r\n\x1a\n", 0))
        .min()
        .unwrap_or(data.len());
    if image_start < 16 {
        return None;
    }

    // Walk back from the image looking for a (lon, lat) pair whose longitude is
    // a plausible degree value and whose bytes are preceded by the NRBF tag
    // that terminates the preceding member.
    let lo = image_start.saturating_sub(64);
    let mut best: Option<(usize, f64, f64)> = None;
    for off in lo..image_start.saturating_sub(16) {
        let lon = f64::from_le_bytes(data[off..off + 8].try_into().ok()?);
        if !(-180.0..180.0).contains(&lon) || lon.abs() < 1e-6 {
            continue;
        }
        let lat = f64::from_le_bytes(data[off + 8..off + 16].try_into().ok()?);
        if !(-90.0..90.0).contains(&lat) || lat.abs() < 1e-6 {
            continue;
        }
        // Keep the pair nearest the image (that is the projection origin).
        best = Some((off, lon, lat));
    }
    best.map(|(_, lon, lat)| (lon, lat))
}

/// Structurally detect whether the project carries split/handle data.
///
/// The class table names every type that will appear in the object graph, so a
/// `HandleCollection` entry means the file has handles. This is a substring
/// probe, which is safe here: class names are stored as plain ASCII in the
/// class table and cannot occur inside the entropy-coded map image by accident
/// (`HandleCollection` is 16 bytes of ASCII).
pub fn detect_split_data(data: &[u8]) -> bool {
    find_from(data, b"HandleCollection", 0).is_some()
}

/// One `Handle` — a route-adjustment marker placed on the map.
///
/// A handle is **not** a split: it records where a stretch of the route was
/// dragged to. Splits come from the `Lap` records (see [`QrtLap`]).
///
/// Byte layout, verified against the reference dump of the demo project (497
/// raw entries, 138 distinct objects, all at the same relative offsets):
///
/// ```text
///   p-24 : 0x01                       object marker of this handle
///   p-23 : int32                      handle id (monotonic)
///   p-17 : double  value              parameterizedLocation.value
///   p-9  : 0x01                       0x01, member terminator
///   p-8  : int32   markerDrawer id
///   p-4  : int32   type               HandleType: -1 start, 0 handle, 1 end
///   p    : double  x                  map pixel x of the marker
///   p+8  : double  y                  map pixel y of the marker
///   p+17 : int32   segmentIndex       which RouteSegment the handle is in
///   p+21 : int32   parameterizedValue same as `value`
///   p+25 : 0x01                       marker for the next handle (stride 68)
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QrtHandle {
    /// Map pixel x of the marker (origin: top-left of the map image).
    pub x: f64,
    /// Map pixel y of the marker (origin: top-left of the map image).
    pub y: f64,
    /// Which route segment the handle belongs to.
    pub segment_index: i32,
    /// `ParameterizedLocation.Value`: a **fractional route-point index** inside
    /// `segment_index`, not a distance. Its integer part selects a waypoint and
    /// the fraction interpolates towards the next one, which is how a handle
    /// records "between sample 119 and 120".
    pub value: f64,
    /// `HandleType`: `-1` start, `0` ordinary handle, `1` end.
    pub kind: i32,
}

/// Relative offsets of a `Handle`'s fields, measured from its **x** double.
const H_VALUE: isize = -17;
const H_SEGMENT: isize = -24;

/// Does the byte pattern at `p` look like a `PointD` class definition followed
/// by its instance values?
///
/// The first handle of a file carries the very first `PointD`, so the
/// serializer writes the class definition there instead of a plain value:
///
/// ```text
///   p-16 : int32    memberCount = 2
///   p-12 : "x" "y"  7-bit-prefixed member names
///   p-8  : 0x00 0x00                  binary types (both Primitive)
///   p-6  : 0x06 0x06                  primitive types (both Double)
///   p-4  : int32    object id of the PointD instance
///   p    : double   x
/// ```
fn inline_pointd_shape(data: &[u8], p: usize) -> bool {
    p >= 72
        && i32_at(data, p - 16) == Some(2)
        && data[p - 12..p - 8] == [0x01, b'x', 0x01, b'y']
        && data[p - 8..p - 6] == [0x00, 0x00]
        && data[p - 6..p - 4] == [0x06, 0x06]
        && i32_at(data, p - 4).is_some_and(|id| (1..1_000_000).contains(&id))
}

fn f64_at(data: &[u8], off: usize) -> Option<f64> {
    data.get(off..off + 8)
        .map(|s| f64::from_le_bytes(s.try_into().unwrap()))
}

fn i32_at(data: &[u8], off: usize) -> Option<i32> {
    data.get(off..off + 4)
        .map(|s| i32::from_le_bytes(s.try_into().unwrap()))
}

/// Smallest map pixel coordinate a handle may have.
///
/// A handle marker always sits well inside the map. Values below this are
/// denormal doubles read out of zero-filled areas of the object graph (for
/// example a `1e-310` "coordinate" made of two stray bytes), which is exactly
/// how the demo project's false positives present.
const HANDLE_MIN_PIXEL: f64 = 20.0;

/// Scan a `.qrt` object graph for `Handle` records.
///
/// A handle record is recognised by four invariants that hold for every handle
/// in a QuickRoute project (verified byte-by-byte against the demo project:
/// 137 of the 138 distinct handles recovered, **zero** false positives):
///
/// * the x and y doubles are finite and at least [`HANDLE_MIN_PIXEL`] pixels
///   into the map raster;
/// * the parameterized route index sits exactly 17 bytes before x and is
///   finite and non-negative;
/// * the `0x01` object marker sits exactly 9 bytes before x;
/// * `parameterizedLocation.segmentIndex` (int32, 24 bytes before x) is a small
///   non-negative number.
///
/// The **first** handle in the stream is written differently: it is the one
/// that defines `ParameterizedLocation` and its `PointD` inline, so instead of
/// the member bytes there is a class definition.  It is recognised by
/// [`inline_pointd_shape`] and read with its own offsets.
///
/// Note the dump helper's 497 count is an artefact of recursively walking the
/// serialized graph: it visits the same `Handle` object several times. The
/// project itself holds 138 distinct handles, all 138 of which carry pixel
/// coordinates once both shapes are handled.
pub fn scan_handles(data: &[u8], map_w: u32, map_h: u32) -> Vec<QrtHandle> {
    let x_hi = if map_w > 0 { map_w as f64 } else { f64::MAX };
    let y_hi = if map_h > 0 { map_h as f64 } else { f64::MAX };
    let mut out = Vec::new();
    let mut p = 32usize;
    while p + 16 <= data.len() {
        let h = (|| -> Option<QrtHandle> {
            let x = f64_at(data, p)?;
            let y = f64_at(data, p + 8)?;
            if !x.is_finite() || !y.is_finite() {
                return None;
            }
            // A handle marker always lies inside the map raster.
            if x < HANDLE_MIN_PIXEL || x > x_hi || y < HANDLE_MIN_PIXEL || y > y_hi {
                return None;
            }
            // Shape A: `parameterizedLocation.segmentIndex` (24 bytes before x)
            // and its value (17 bytes before x), terminated by the `0x01`
            // member marker 9 bytes before x.
            if data[p - 9] == 0x01 {
                let segment_index = i32_at(data, (p as isize + H_SEGMENT) as usize)?;
                if !(0..10_000).contains(&segment_index) {
                    return None;
                }
                let value = f64_at(data, (p as isize + H_VALUE) as usize)?;
                if !value.is_finite() || value < 0.0 || value > 1e6 {
                    return None;
                }
                return Some(QrtHandle {
                    x,
                    y,
                    segment_index,
                    value,
                    kind: 0,
                });
            }
            // Shape B: the first handle in the stream, whose
            // `ParameterizedLocation` and `PointD` are defined inline.
            // `segmentIndex` and `value` then sit 68 and 64 bytes before x,
            // because the two class definitions are written between them.
            if inline_pointd_shape(data, p) {
                let segment_index = i32_at(data, p - 68)?;
                let value = f64_at(data, p - 64)?;
                if !(0..10_000).contains(&segment_index) {
                    return None;
                }
                if !value.is_finite() || value < 0.0 || value > 1e6 {
                    return None;
                }
                return Some(QrtHandle {
                    x,
                    y,
                    segment_index,
                    value,
                    kind: 0,
                });
            }
            None
        })();
        if let Some(q) = h {
            out.push(q);
        }
        p += 1;
    }

    // Several handles may share pixel coordinates while differing in `value`;
    // identical (x, y, value, segment) tuples are the same object.
    out.sort_by(|a, b| {
        a.segment_index
            .cmp(&b.segment_index)
            .then(a.value.partial_cmp(&b.value).unwrap_or(std::cmp::Ordering::Equal))
            .then(a.x.partial_cmp(&b.x).unwrap_or(std::cmp::Ordering::Equal))
            .then(a.y.partial_cmp(&b.y).unwrap_or(std::cmp::Ordering::Equal))
    });
    out.dedup_by(|a, b| {
        a.segment_index == b.segment_index && a.value == b.value && a.x == b.x && a.y == b.y
    });
    out
}

/// One `Lap` — QuickRoute's split point.
///
/// Every project gets a `Start` lap and a `Stop` lap automatically; the user's
/// own split points are the `Lap` entries. So the split list comes from the
/// laps, **not** from the handles (which are the draggable markers drawn on the
/// map).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QrtLap {
    /// Punch time as .NET `DateTime.ToBinary()` ticks.
    pub time_ticks: i64,
    /// `LapType`: `0` start, `1` user split, `2` stop.
    pub kind: i32,
}

/// `LapType.Start`.
pub const LAP_START: i32 = 0;
/// `LapType.Lap` — a real split the user placed.
pub const LAP_SPLIT: i32 = 1;
/// `LapType.Stop`.
pub const LAP_STOP: i32 = 2;

/// The `LapType` enum's class name, written inline the first time a `Lap`
/// instance needs it.
const LAP_TYPE_NAME: &[u8] = b"QuickRoute.BusinessEntities.LapType";
/// The enum's single member name.
const LAP_VALUE_FIELD: &[u8] = b"value__";

/// Recover the project's `Lap`s.
///
/// A `Lap` is a plain `[Serializable]` class with the members `time`
/// (`DateTime`), `lapType` (a `LapType` enum, written as a nested object) and
/// `description` (usually `null`). Every instance therefore carries the shape
///
/// ```text
///   0x01 | objId(4) | lapClassId(4) | time(8)
///        | 0x01 | enumObjId(4) | lapTypeClassId(4) | kind(4) | 0x0A
/// ```
///
/// which is 31 bytes. Only the two class ids are file-specific, so the scan does
/// not hardcode them: it gathers every candidate and keeps the class id that the
/// most candidates share.
///
/// The **first** `Lap` of a file is written differently — its `lapType` enum is
/// defined inline rather than referenced, so the nested bytes become
/// `0x05 | enumId(4) | 0x23 "QuickRoute.BusinessEntities.LapType" …`. That shape
/// is matched separately; each demo project holds exactly one such instance and
/// skipping it would lose a real split.
pub fn scan_laps(data: &[u8]) -> Vec<QrtLap> {
    let plausible = |t: i64| (TICKS_LO..TICKS_HI).contains(&t);

    // ---- shape A: every instance after the first ----
    let mut cands: Vec<(i32, i64, i32)> = Vec::new(); // (classId, time, kind)
    let mut p = 0usize;
    while p + 31 <= data.len() {
        if data[p] == 0x01 && data[p + 17] == 0x01 && data[p + 30] == 0x0A {
            let t = i64::from_le_bytes(data[p + 9..p + 17].try_into().unwrap());
            let kind = i32::from_le_bytes(data[p + 26..p + 30].try_into().unwrap());
            if plausible(t) && (LAP_START..=LAP_STOP).contains(&kind) {
                let class_id = i32::from_le_bytes(data[p + 5..p + 9].try_into().unwrap());
                cands.push((class_id, t, kind));
                p += 31;
                continue;
            }
        }
        p += 1;
    }

    let mut out: Vec<QrtLap> = Vec::new();
    if !cands.is_empty() {
        let mut counts: std::collections::HashMap<i32, usize> = std::collections::HashMap::new();
        for c in &cands {
            *counts.entry(c.0).or_insert(0) += 1;
        }
        let best = counts.into_iter().max_by_key(|kv| kv.1).map(|kv| kv.0);
        if let Some(best) = best {
            for c in &cands {
                if c.0 == best {
                    out.push(QrtLap { time_ticks: c.1, kind: c.2 });
                }
            }
        }
    }

    // ---- shape B: the first instance, with the `LapType` enum defined inline ----
    let n = LAP_TYPE_NAME.len();
    let mut i = 14usize;
    while i + n <= data.len() {
        if &data[i..i + n] != LAP_TYPE_NAME {
            i += 1;
            continue;
        }
        // Preceded by `0x05 | enumId(4) | nameLen(1)`; the Lap's `time` sits
        // 8 bytes before that tag.
        if data[i - 6] == 0x05 && data[i - 1] as usize == n {
            let time = i64::from_le_bytes(data[i - 14..i - 6].try_into().unwrap());
            if plausible(time) {
                // After `0x07 "value__"` come a btype byte, a primitive-type
                // byte and a 4-byte library id, then the enum's value = the kind.
                let mut j = i + n;
                while j + 18 <= data.len() {
                    if data[j] == 0x07 && &data[j + 1..j + 8] == LAP_VALUE_FIELD {
                        let kind = i32::from_le_bytes(data[j + 14..j + 18].try_into().unwrap());
                        if (LAP_START..=LAP_STOP).contains(&kind) {
                            out.push(QrtLap { time_ticks: time, kind });
                        }
                        break;
                    }
                    j += 1;
                }
            }
        }
        i += n;
    }

    out.sort_by_key(|l| l.time_ticks);
    out.dedup_by_key(|l| (l.time_ticks, l.kind));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic record builder so the scanner can be tested without a file.
    fn record(lon: f64, lat: f64, index: i32) -> Vec<u8> {
        let mut r = Vec::new();
        r.extend_from_slice(&lon.to_le_bytes());
        r.extend_from_slice(&lat.to_le_bytes());
        r.push(0x01);
        r.extend_from_slice(&index.to_le_bytes());
        r.extend_from_slice(&[0, 0, 0, 0]);
        r
    }

    #[test]
    fn record_is_25_bytes() {
        assert_eq!(record(1.0, 2.0, 0).len(), WAYPOINT_RECORD_LEN);
    }

    #[test]
    fn scans_consecutive_records() {
        let mut data = vec![0u8; 40]; // leading padding
        for i in 0..5 {
            data.extend_from_slice(&record(113.0 + i as f64 * 1e-4, 23.1, i));
        }
        let pts = scan_waypoints(&data);
        assert_eq!(pts.len(), 5);
        assert_eq!(pts[0].index, 0);
        assert_eq!(pts[4].index, 4);
        assert!((pts[2].longitude - 113.0002).abs() < 1e-9);
    }

    #[test]
    fn rejects_zero_and_out_of_range() {
        let mut data = Vec::new();
        data.extend_from_slice(&record(0.0, 0.0, 3));       // degenerate
        data.extend_from_slice(&record(200.0, 23.0, 4));    // out of range
        data.extend_from_slice(&record(113.0, 23.0, 5));    // good
        let pts = scan_waypoints(&data);
        assert_eq!(pts.len(), 1);
        assert_eq!(pts[0].index, 5);
    }

    /// Reads the real demo projects when they are present in the workspace.
    fn demo_path(name: &str) -> Option<std::path::PathBuf> {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("demo")
            .join(name);
        if p.exists() {
            Some(p)
        } else {
            None
        }
    }

    #[test]
    fn reads_real_demo_project() {
        let Some(path) = demo_path("AM 男子短距 第一面.qrt") else {
            eprintln!("demo .qrt not present; skipping");
            return;
        };
        let proj = read_project(&path).expect("read project");

        let map = proj.map.as_ref().expect("embedded map image");
        assert_eq!(map.kind, MapImageKind::Jpeg);
        assert_eq!((map.width, map.height), (3000, 4243));
        assert!(map.bytes.len() > 4_000_000, "map is ~4.28 MB");
        // A JPEG must start with SOI and end with EOI.
        assert_eq!(&map.bytes[..2], b"\xff\xd8");
        assert_eq!(&map.bytes[map.bytes.len() - 2..], b"\xff\xd9");

        assert_eq!(proj.waypoints.len(), 1475);
        assert_eq!(proj.waypoints[0].index, 1808);
        assert!((proj.waypoints[0].longitude - 113.0240430).abs() < 1e-6);
        assert!((proj.waypoints[0].latitude - 23.1757680).abs() < 1e-6);
        assert!(!proj.waypoints_complete);

        // 29 laps: the automatic Start and Stop plus the 27 splits the user
        // placed. The split list is the segment ends, so it holds the 27 splits
        // and the finish — 28 entries.
        assert_eq!(proj.laps.len(), 29, "laps");
        assert_eq!(proj.laps.iter().filter(|l| l.kind == LAP_START).count(), 1);
        assert_eq!(proj.laps.iter().filter(|l| l.kind == LAP_SPLIT).count(), 27);
        assert_eq!(proj.laps.iter().filter(|l| l.kind == LAP_STOP).count(), 1);
        assert_eq!(proj.split_count, 28, "splits to import");
        assert!(proj.has_splits());
        assert!(proj.has_split_data);

        // Handles are the map markers, a different thing from the laps.
        assert_eq!(proj.handles.len(), 138);
        assert!(proj.handles.iter().any(|h| h.value == 0.0));
        // The first handle in the stream defines `PointD` inline; it is the
        // only one the plain shape-A scan would miss, so pin it.
        assert!(
            proj.handles
                .iter()
                .any(|h| (h.value - 498.2576607049354).abs() < 1e-6),
            "inline-PointD handle missing"
        );
        let vmax = proj
            .handles
            .iter()
            .map(|h| h.value)
            .fold(f64::NEG_INFINITY, f64::max);
        assert!(vmax > 1400.0 && vmax < 1500.0, "vmax was {vmax}");
        for h in &proj.handles {
            assert!(h.x >= 0.0 && h.x <= map.width as f64, "x={}", h.x);
            assert!(h.y >= 0.0 && h.y <= map.height as f64, "y={}", h.y);
            assert!(h.segment_index == 0, "seg={}", h.segment_index);
        }

        // projectionOrigin is written just before the embedded map image.
        let (olon, olat) = proj.projection_origin.expect("projection origin");
        assert!((olon - 113.0243020).abs() < 1e-6, "lon was {olon}");
        assert!((olat - 23.1763540).abs() < 1e-6, "lat was {olat}");

        let pts = proj.to_route_points();
        assert_eq!(pts.len(), 1475);
        assert!((pts[0].coordinate_x - 113.0240430).abs() < 1e-6);
        // Positions must be filled with map pixels, not left at 0.
        assert!(pts.iter().any(|p| p.position_x > 0.0));
        assert!(pts.iter().any(|p| p.position_y > 0.0));

        // Positions are centred on the map, because that is where the renderer
        // draws the image (see `map_positions`).
        let pos = proj.map_positions();
        assert_eq!(pos.len(), 1475);
        let (cx, cy) = (map.width as f64 / 2.0, map.height as f64 / 2.0);

        // The strongest available check on the georeferencing: QuickRoute
        // records, for every handle, both where it sits on the map and which
        // route point it belongs to, and derives each handle's matrix so that
        // the matrix maps *that handle and its successor* onto those recorded
        // pixel positions. Recovering the matrices from the handle positions
        // must reproduce that contract exactly.
        let handles = proj.ordered_handles();
        assert_eq!(handles.len(), 138, "handles with pixel coordinates");
        let projected = proj.projected_positions();
        let transforms = proj.handle_transforms(&projected);
        let indices = proj.transform_indices(projected.len());
        assert_eq!(transforms.len(), handles.len());
        assert_eq!(indices.len(), projected.len());
        for m in 0..handles.len() - 1 {
            let t = transforms[m];
            for h in [&handles[m], &handles[m + 1]] {
                let p = project_at(&projected, h.value);
                let q = t.apply(p);
                let d = ((q.0 - h.x).powi(2) + (q.1 - h.y).powi(2)).sqrt();
                assert!(d < 0.01, "transform {m} misses handle {} by {d} px", h.value);
            }
        }

        // The reader applies the transform of the *preceding* handle and can
        // advance at most one handle per route point (exactly like the reference
        // reader), so a marker squeezed into the same route point as its
        // neighbour gets drawn with the previous transform. Everything else has
        // to land exactly on its marker.
        let mut exact = 0usize;
        let mut worst_of_rest = 0.0f64;
        for h in &handles {
            let node = h.value.floor();
            if node < 0.0 || node >= projected.len() as f64 {
                continue;
            }
            let t = transforms[indices[node as usize]];
            let q = t.apply(project_at(&projected, h.value));
            let d = ((q.0 - h.x).powi(2) + (q.1 - h.y).powi(2)).sqrt();
            if d < 0.5 {
                exact += 1;
            } else {
                worst_of_rest = worst_of_rest.max(d);
            }
        }
        assert!(
            exact >= handles.len() - 3,
            "only {exact}/{} handles land on their marker",
            handles.len()
        );
        let _ = worst_of_rest;

        // The track must span a real part of the map, and stay on it.
        let xs: Vec<f64> = pos.iter().map(|p| p.0).collect();
        let ys: Vec<f64> = pos.iter().map(|p| p.1).collect();
        let span_x = xs.iter().cloned().fold(f64::MIN, f64::max)
            - xs.iter().cloned().fold(f64::MAX, f64::min);
        let span_y = ys.iter().cloned().fold(f64::MIN, f64::max)
            - ys.iter().cloned().fold(f64::MAX, f64::min);
        assert!(span_x > 100.0 && span_y > 100.0, "span {span_x}x{span_y}");
        assert!(
            span_x < map.width as f64 && span_y < map.height as f64,
            "track cannot be larger than the map"
        );
        for (x, y, _) in &pos {
            assert!(x.abs() <= cx + 1.0, "x={x} outside the map");
            assert!(y.abs() <= cy + 1.0, "y={y} outside the map");
        }

        // Sample times: the route's own time array is recovered index by index,
        // 1 Hz apart, and spans the recording.
        let timed = proj.waypoints.iter().filter(|w| w.time_ticks > 0).count();
        assert!(timed >= 1474, "only {timed} waypoints carried a time");
        let t0 = proj.waypoints[0].time_ticks;
        let t_last = proj.waypoints[timed - 1].time_ticks;
        assert!(t0 > 0);
        let span_s = (t_last - t0) as f64 / 1e7;
        // 1474 of the 1475 samples are recovered index by index, so the measured
        // span is one second short of the full 1483 s recording.
        assert!((span_s - 1482.0).abs() < 2.0, "span was {span_s}s");

        // Imported splits: laps excluding the departure, in milliseconds and
        // strictly inside the recording.
        let splits = proj.split_times_ms();
        assert_eq!(splits.len(), 28, "splits");
        assert!(splits.windows(2).all(|w| w[1] > w[0]), "splits not increasing");
        assert_eq!(splits[0], 74_000, "first split");
        assert_eq!(*splits.last().unwrap(), 1_483_000, "last split");
        let last = *splits.last().unwrap();
        assert!(
            last as f64 / 1000.0 <= span_s + 2.0,
            "last split {last} ms is past the route ({span_s}s)"
        );
    }

    /// A project where the user never placed a split holds only the automatic
    /// start and stop laps, so nothing must be offered for import.
    #[test]
    fn project_without_user_splits_has_none() {
        let Some(path) = demo_path("AM 男子短距 第二面.qrt") else {
            eprintln!("second demo .qrt not present; skipping");
            return;
        };
        let proj = read_project(&path).expect("read project");
        assert_eq!(proj.laps.len(), 2, "start + stop only");
        assert_eq!(proj.laps.iter().filter(|l| l.kind == LAP_SPLIT).count(), 0);
        assert_eq!(proj.split_count, 0);
        assert!(!proj.has_splits());
        assert!(proj.split_times_ms().is_empty());
    }

    #[test]
    fn projection_origin_absent_when_no_image() {
        assert_eq!(find_projection_origin(b""), None);
        assert_eq!(find_projection_origin(&[0u8; 32]), None);
    }

    #[test]
    fn split_detection_is_structural() {
        assert!(detect_split_data(b"...QuickRoute.BusinessEntities.HandleCollection..."));
        assert!(!detect_split_data(b"...QuickRoute.BusinessEntities.Document..."));
    }

    /// The imported splits are the lap times excluding the departure, so they
    /// must be strictly increasing and end at the finish.
    #[test]
    fn split_times_are_monotonic_and_bounded() {
        let Some(path) = demo_path("AM 男子短距 第一面.qrt") else {
            eprintln!("demo .qrt not present; skipping");
            return;
        };
        let proj = read_project(&path).expect("read project");
        let splits = proj.split_times_ms();
        assert_eq!(splits.len(), 28, "splits");
        assert!(
            splits.windows(2).all(|w| w[1] > w[0]),
            "split times must increase"
        );
        // The route lasts 1483 s and the final split is the finish.
        assert_eq!(*splits.last().unwrap(), 1_483_000);
        // The departure is not a segment.
        assert!(splits[0] > 0);
    }

    /// Sample times line up with the coordinates index by index.
    #[test]
    fn waypoint_times_are_recovered_in_order() {
        let Some(path) = demo_path("AM 男子短距 第一面.qrt") else {
            eprintln!("demo .qrt not present; skipping");
            return;
        };
        let proj = read_project(&path).expect("read project");
        let timed: Vec<i64> = proj
            .waypoints
            .iter()
            .map(|w| w.time_ticks)
            .filter(|t| *t > 0)
            .collect();
        assert!(timed.len() >= 1474, "only {} times", timed.len());
        assert!(timed.windows(2).all(|w| w[1] > w[0]), "times must increase");
        let span = (timed.last().unwrap() - timed[0]) as f64 / 1e7;
        assert!((span - 1482.0).abs() < 2.0, "span {span}s");
    }
}
