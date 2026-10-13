//! QuickRoute JPEG reader (ported from QuickRouteReader.cpp / quickroute_reader.py).
//!
//! QuickRoute embeds a binary blob across JPEG APP0 (0xFFE0) markers whose payloads
//! start with the 10-byte magic `QuickRoute`. Concatenate their data before parsing
//! the blob as a sequence of tagged blocks:
//!
//! ```text
//! tag      : uint8
//! tagLength: uint32 little-endian
//! payload  : tagLength bytes
//! ```
//!
//! Top-level tags: 4 = image dims (x,y,w,h u16), 5 = sessions, 6 = session, 7 = route,
//! 8 = handles, 9 = projection origin (2× int32 / 3600000 degrees).

use crate::math::project_coordinate;
use crate::route::RoutePoint;
use crate::route::RoutePoint as RP;

pub const QUICKROUTE_MAGIC: &[u8; 10] = b"QuickRoute";

const TAG_IMAGE_DIMENSIONS: u8 = 4;
const TAG_SESSIONS: u8 = 5;
const TAG_SESSION: u8 = 6;
const TAG_ROUTE: u8 = 7;
const TAG_HANDLES: u8 = 8;
const TAG_PROJECTION_ORIGIN: u8 = 9;

/// Errors that can occur while reading QuickRoute data.
#[derive(Debug, Clone, PartialEq)]
pub enum QuickRouteError {
    NotJpeg,
    NoData,
    NoProjectionOrigin,
    Truncated(&'static str),
}

impl std::fmt::Display for QuickRouteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QuickRouteError::NotJpeg => write!(f, "Not a supported JPEG file format"),
            QuickRouteError::NoData => write!(f, "Could not find QuickRoute JPEG extension data"),
            QuickRouteError::NoProjectionOrigin => write!(f, "Could not find projection origin"),
            QuickRouteError::Truncated(s) => write!(f, "Truncated: {s}"),
        }
    }
}

impl std::error::Error for QuickRouteError {}

/// Cursor over a byte buffer in stream order.
struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    fn read(&mut self, n: usize) -> Result<&'a [u8], QuickRouteError> {
        if self.remaining() < n {
            return Err(QuickRouteError::Truncated("read"));
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn read_u8(&mut self) -> Result<u8, QuickRouteError> {
        Ok(self.read(1)?[0])
    }

    fn read_u16(&mut self) -> Result<u16, QuickRouteError> {
        let b = self.read(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn read_u32(&mut self) -> Result<u32, QuickRouteError> {
        let b = self.read(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn read_i32(&mut self) -> Result<i32, QuickRouteError> {
        let b = self.read(4)?;
        Ok(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn read_u64(&mut self) -> Result<u64, QuickRouteError> {
        let b = self.read(8)?;
        Ok(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    fn read_f64(&mut self) -> Result<f64, QuickRouteError> {
        let b = self.read(8)?;
        Ok(f64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    fn skip(&mut self, n: usize) -> Result<(), QuickRouteError> {
        if self.remaining() < n {
            return Err(QuickRouteError::Truncated("skip"));
        }
        self.pos += n;
        Ok(())
    }
}

/// Parsed QuickRoute data.
#[derive(Debug, Clone)]
pub struct QuickRouteData {
    /// Route points (with positions computed against the given map dims).
    pub route_points: Vec<RoutePoint>,
    /// QuickRoute image intrinsic dimensions.
    pub route_image_width: f64,
    pub route_image_height: f64,
    pub projection_origin: (f64, f64),
}

pub fn parse_quickroute(
    jpeg_data: &[u8],
    map_width: f64,
    map_height: f64,
) -> Result<QuickRouteData, QuickRouteError> {
    let blob = extract_data_part(jpeg_data)?;
    let mut qr = QuickRouteBuilder {
        route_points: Vec::new(),
        route_image_width: 0.0,
        route_image_height: 0.0,
        projection_origin: None,
        handles: Vec::new(),
    };
    qr.process_data_part(&blob)?;
    match qr.projection_origin {
        Some(origin) => {
            qr.finish(origin, map_width, map_height)?;
            Ok(QuickRouteData {
                route_points: qr.route_points,
                route_image_width: qr.route_image_width,
                route_image_height: qr.route_image_height,
                projection_origin: origin,
            })
        }
        None => Err(QuickRouteError::NoProjectionOrigin),
    }
}

fn extract_data_part(data: &[u8]) -> Result<Vec<u8>, QuickRouteError> {
    if data.len() < 2 || data[0] != 0xFF || data[1] != 0xD8 {
        return Err(QuickRouteError::NotJpeg);
    }
    let mut pos = 2usize;
    let mut blob = Vec::new();
    let mut found = false;
    while pos < data.len() {
        if data[pos] != 0xFF {
            break;
        }
        // JPEG permits fill bytes before a marker.
        while pos < data.len() && data[pos] == 0xFF {
            pos += 1;
        }
        let marker = *data
            .get(pos)
            .ok_or(QuickRouteError::Truncated("JPEG marker"))?;
        pos += 1;
        match marker {
            // Metadata ends before the compressed image data.
            0xD9 | 0xDA => break,
            // Standalone markers have no length word.
            0xD8 | 0x01 | 0xD0..=0xD7 => continue,
            _ => {}
        }
        let length_bytes = data
            .get(pos..pos + 2)
            .ok_or(QuickRouteError::Truncated("JPEG segment length"))?;
        let length = u16::from_be_bytes([length_bytes[0], length_bytes[1]]) as usize;
        if length < 2 {
            return Err(QuickRouteError::Truncated("JPEG segment length"));
        }
        let payload = data
            .get(pos + 2..pos + length)
            .ok_or(QuickRouteError::Truncated("JPEG segment"))?;
        if marker == 0xE0 {
            if let Some(part) = payload.strip_prefix(QUICKROUTE_MAGIC) {
                found = true;
                blob.extend_from_slice(part);
            }
        }
        pos += length;
    }
    if found {
        Ok(blob)
    } else {
        Err(QuickRouteError::NoData)
    }
}

struct QuickRouteBuilder {
    route_points: Vec<RoutePoint>,
    route_image_width: f64,
    route_image_height: f64,
    projection_origin: Option<(f64, f64)>,
    // handles: (route_point_index, [9] matrix for segment 0)
    handles: Vec<(f64, [f64; 9])>,
}

impl QuickRouteBuilder {
    fn process_data_part(&mut self, blob: &[u8]) -> Result<(), QuickRouteError> {
        let mut reader = Cursor::new(blob);
        while reader.remaining() >= 5 {
            let tag = reader.read_u8()?;
            let tag_length = reader.read_u32()? as usize;
            let payload_start = reader.pos;
            if reader.remaining() < tag_length {
                // tolerate truncated final block
                break;
            }
            let payload = &reader.data[payload_start..payload_start + tag_length];
            reader.skip(tag_length)?;

            match tag {
                TAG_SESSIONS => {
                    let mut r = Cursor::new(payload);
                    let _session_count = r.read_u32()?;
                    while r.remaining() >= 5 {
                        let st = r.read_u8()?;
                        let sl = r.read_u32()? as usize;
                        let sp_start = r.pos;
                        if r.remaining() < sl {
                            break;
                        }
                        if st == TAG_SESSION {
                            self.read_session(&r.data[sp_start..sp_start + sl])?;
                        }
                        r.skip(sl)?;
                    }
                }
                TAG_IMAGE_DIMENSIONS => {
                    let _x = u16::from_le_bytes([payload[0], payload[1]]);
                    let _y = u16::from_le_bytes([payload[2], payload[3]]);
                    self.route_image_width = u16::from_le_bytes([payload[4], payload[5]]) as f64;
                    self.route_image_height = u16::from_le_bytes([payload[6], payload[7]]) as f64;
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Parse one session block as a continuous little-endian stream.
    fn read_session(&mut self, data: &[u8]) -> Result<(), QuickRouteError> {
        let mut reader = Cursor::new(data);
        while reader.remaining() >= 5 {
            let tag = reader.read_u8()?;
            let tag_length = reader.read_u32()? as usize;
            let payload_start = reader.pos;
            if reader.remaining() < tag_length {
                break;
            }
            match tag {
                TAG_ROUTE => self.read_route(&data[payload_start..payload_start + tag_length])?,
                TAG_HANDLES => {
                    self.read_handles(&data[payload_start..payload_start + tag_length])?
                }
                TAG_PROJECTION_ORIGIN => {
                    // Stored as two int32 / 3600000 degrees (NOT doubles).
                    let lon = i32::from_le_bytes([
                        data[payload_start],
                        data[payload_start + 1],
                        data[payload_start + 2],
                        data[payload_start + 3],
                    ]) as f64
                        / 3_600_000.0;
                    let lat = i32::from_le_bytes([
                        data[payload_start + 4],
                        data[payload_start + 5],
                        data[payload_start + 6],
                        data[payload_start + 7],
                    ]) as f64
                        / 3_600_000.0;
                    self.projection_origin = Some((lon, lat));
                }
                _ => {}
            }
            reader.skip(tag_length)?;
        }
        Ok(())
    }

    fn read_route(&mut self, data: &[u8]) -> Result<(), QuickRouteError> {
        let mut reader = Cursor::new(data);
        let attributes = reader.read_u16()?;
        let extra_waypoint_attributes_length = reader.read_u16()? as usize;
        let segment_count = reader.read_u32()?;

        let mut previous_ticks_ms = 0i64;
        for seg in 0..segment_count {
            let waypoint_count = reader.read_u32()?;
            for _ in 0..waypoint_count {
                let lon = (reader.read_i32()?) as f64 / 3_600_000.0;
                let lat = (reader.read_i32()?) as f64 / 3_600_000.0;

                let time_type = reader.read_u8()?;
                let ticks_ms = if time_type == 0 {
                    let ticks = reader.read_u64()?;
                    let mut t = ticks & 0x3FFF_FFFF_FFFF_FFFF;
                    t = t.wrapping_sub(621355968000000000);
                    (t / 10000) as i64
                } else {
                    let rel_ms = reader.read_u16()? as i64;
                    previous_ticks_ms + rel_ms
                };
                previous_ticks_ms = ticks_ms;

                let mut heart_rate = 0.0;
                let mut elevation = 0.0;
                if attributes & 0x04 != 0 {
                    heart_rate = reader.read_u8()? as f64;
                }
                if attributes & 0x08 != 0 {
                    let _h = reader.read_u16()?;
                    // elevation stored as uint16; matches PyQt which also reads 2 bytes
                    elevation = _h as f64;
                }
                reader.skip(extra_waypoint_attributes_length)?;

                if seg == 0 {
                    self.route_points.push(RP {
                        date_time_ms: ticks_ms as f64,
                        coordinate_x: lon,
                        coordinate_y: lat,
                        elevation,
                        heart_rate,
                        ..RP::default()
                    });
                }
            }
        }
        Ok(())
    }

    fn read_handles(&mut self, data: &[u8]) -> Result<(), QuickRouteError> {
        let mut reader = Cursor::new(data);
        let handle_count = reader.read_u32()?;
        for _ in 0..handle_count {
            let mut m = [0.0f64; 9];
            for j in 0..9 {
                m[j] = reader.read_f64()?;
            }
            let segment_index = reader.read_u32()?;
            let route_point_index = reader.read_f64()?;
            // ignore handle pixel location and type
            reader.skip(18)?;

            if segment_index == 0 {
                // QMatrix::setMatrix(m11, m12, m21, m22, dx, dy)
                self.handles.push((route_point_index, m));
            }
        }
        Ok(())
    }

    fn finish(
        &mut self,
        origin: (f64, f64),
        map_width: f64,
        map_height: f64,
    ) -> Result<(), QuickRouteError> {
        let points = &mut self.route_points;
        if points.is_empty() {
            return Ok(());
        }
        let w = self.route_image_width;
        let h = self.route_image_height;
        let sx = if w > 0.0 { map_width / w } else { 1.0 };
        let sy = if h > 0.0 { map_height / h } else { 1.0 };

        // sort handles by route-point index
        self.handles
            .sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        let mut handle_index = 0usize;
        let mut current: Option<(f64, [f64; 9])> = self.handles.first().copied();
        let mut nxt: Option<(f64, [f64; 9])> = if self.handles.len() > 1 {
            Some(self.handles[1])
        } else {
            None
        };

        for (i, rp) in points.iter_mut().enumerate() {
            if let Some((ni, _)) = nxt {
                if (i as f64) > ni && handle_index + 1 < self.handles.len() {
                    handle_index += 1;
                    current = nxt;
                    nxt = self.handles.get(handle_index + 1).copied();
                }
            }

            let (px, py) = project_coordinate(rp.coordinate_x, rp.coordinate_y, origin.0, origin.1);
            let (x1, y1) = match current {
                Some((_, m)) => (m[0] * px + m[3] * py + m[2], m[1] * px + m[4] * py + m[5]),
                None => (px, py),
            };
            rp.position_x = x1 * sx - w / 2.0;
            rp.position_y = y1 * sy - h / 2.0;
        }

        if points.len() < 2 {
            return Ok(());
        }

        // QuickRoute `Route.CalculateSpeeds` / `CalculateDirections`：居中时间窗的瞬时
        // 速度（即使关掉平滑也一样，内部点取前后两段速度的平均、端点只有一侧可用）与
        // 连续航向角。与 `.qrt` 工程那条路（`qrt::to_route_points`）共用同一份实现，
        // 同一份轨迹从不同来源进来必须得到相同的 pace / orientation。
        crate::route::fill_route_metrics(points);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::haversine_distance;

    #[test]
    fn pace_uses_quickroute_centred_speed_and_endpoint_values() {
        let mut builder = QuickRouteBuilder {
            route_points: Vec::new(), route_image_width: 1000.0, route_image_height: 1000.0,
            projection_origin: Some((0.0, 0.0)), handles: Vec::new(),
        };
        builder.route_points = [0.0, 0.001, 0.003].into_iter().enumerate().map(|(i, lon)| {
            let mut point = RoutePoint::default();
            point.coordinate_x = lon;
            point.coordinate_y = 0.0;
            point.date_time_ms = i as f64 * 10000.0;
            point
        }).collect();
        builder.finish((0.0, 0.0), 1000.0, 1000.0).unwrap();
        let points = &builder.route_points;
        let speed = haversine_distance(0.0, 0.0, 0.001, 0.0) / 10.0;
        let expected = [speed, 1.5 * speed, 2.0 * speed];
        for (point, speed) in points.iter().zip(expected) {
            assert!((point.pace - 1000.0 / (60.0 * speed)).abs() < 1e-9);
        }
        // Stationary or invalid-time samples must not introduce NaN/infinity into interpolation.
        for point in &mut builder.route_points { point.coordinate_x = 0.0; }
        builder.finish((0.0, 0.0), 1000.0, 1000.0).unwrap();
        assert!(builder.route_points.iter().all(|point| point.pace == 0.0));
    }

    fn tagged(tag: u8, payload: &[u8]) -> Vec<u8> {
        let mut data = vec![tag];
        data.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        data.extend_from_slice(payload);
        data
    }

    fn route_blob(point_count: u32) -> Vec<u8> {
        let mut route = Vec::new();
        route.extend_from_slice(&0u16.to_le_bytes()); // attributes
        route.extend_from_slice(&0u16.to_le_bytes()); // extra attributes
        route.extend_from_slice(&1u32.to_le_bytes()); // segment count
        route.extend_from_slice(&point_count.to_le_bytes());
        for i in 0..point_count {
            route.extend_from_slice(&(360_000_000i32 + i as i32).to_le_bytes());
            route.extend_from_slice(&108_000_000i32.to_le_bytes());
            if i == 0 {
                route.push(0); // absolute .NET timestamp: Unix epoch
                route.extend_from_slice(&621_355_968_000_000_000u64.to_le_bytes());
            } else {
                route.push(1);
                route.extend_from_slice(&1000u16.to_le_bytes());
            }
        }
        let mut session = tagged(TAG_ROUTE, &route);
        let mut origin = 360_000_000i32.to_le_bytes().to_vec();
        origin.extend_from_slice(&108_000_000i32.to_le_bytes());
        session.extend(tagged(TAG_PROJECTION_ORIGIN, &origin));
        let mut sessions = 1u32.to_le_bytes().to_vec();
        sessions.extend(tagged(TAG_SESSION, &session));
        let mut dims = Vec::new();
        for value in [0u16, 0, 1000, 1000] {
            dims.extend_from_slice(&value.to_le_bytes());
        }
        let mut blob = tagged(TAG_IMAGE_DIMENSIONS, &dims);
        blob.extend(tagged(TAG_SESSIONS, &sessions));
        blob
    }

    fn jpeg_segment(jpeg: &mut Vec<u8>, marker: u8, payload: &[u8]) {
        assert!(payload.len() <= 65533);
        jpeg.extend_from_slice(&[0xFF, marker]);
        jpeg.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        jpeg.extend_from_slice(payload);
    }

    fn chunked_jpeg(blob: &[u8], chunk_size: usize) -> Vec<u8> {
        let mut jpeg = vec![0xFF, 0xD8];
        jpeg_segment(&mut jpeg, 0xE0, b"JFIF\0");
        jpeg_segment(&mut jpeg, 0xE1, b"Exif\0\0");
        for chunk in blob.chunks(chunk_size) {
            let mut payload = QUICKROUTE_MAGIC.to_vec();
            payload.extend_from_slice(chunk);
            jpeg_segment(&mut jpeg, 0xE0, &payload);
        }
        jpeg_segment(&mut jpeg, 0xFE, b"unrelated comment");
        jpeg.extend_from_slice(&[0xFF, 0xDA, 0, 2, 0xFF, 0xE0]);
        jpeg
    }

    #[test]
    fn parses_route_larger_than_one_quickroute_jpeg_segment() {
        let blob = route_blob(7000);
        assert!(blob.len() > 65523);
        let jpeg = chunked_jpeg(&blob, 65523);
        assert_eq!(extract_data_part(&jpeg).unwrap(), blob);
        let qr = parse_quickroute(&jpeg, 1000.0, 1000.0).unwrap();
        assert_eq!(qr.route_points.len(), 7000);
        assert_eq!(qr.projection_origin, (100.0, 30.0));
        assert_eq!(qr.route_image_width, 1000.0);
        assert_eq!(qr.route_image_height, 1000.0);
        assert_eq!(qr.route_points[0].date_time_ms, 0.0);
        assert_eq!(qr.route_points[6999].date_time_ms, 6_999_000.0);
        assert_eq!(qr.route_points[6999].time, 6999.0);
        assert_eq!(
            qr.route_points[6999].coordinate_x,
            360_006_999.0 / 3_600_000.0
        );
    }

    #[test]
    fn chunk_boundaries_do_not_change_parsed_route() {
        let blob = route_blob(3);
        let single = parse_quickroute(&chunked_jpeg(&blob, 65523), 1000.0, 1000.0).unwrap();
        // One-byte chunks also split tag headers, timestamps and the origin.
        for chunk_size in [1, 7, 17] {
            let jpeg = chunked_jpeg(&blob, chunk_size);
            assert_eq!(extract_data_part(&jpeg).unwrap(), blob);
            let split = parse_quickroute(&jpeg, 1000.0, 1000.0).unwrap();
            assert_eq!(split.projection_origin, single.projection_origin);
            assert_eq!(split.route_points.len(), single.route_points.len());
            for (a, b) in split.route_points.iter().zip(&single.route_points) {
                assert_eq!(a.date_time_ms, b.date_time_ms);
                assert_eq!(a.position_x, b.position_x);
                assert_eq!(a.position_y, b.position_y);
                assert_eq!(a.pace, b.pace);
            }
        }
    }

    #[test]
    fn reports_truncated_jpeg_segments_without_panicking() {
        for jpeg in [
            vec![0xFF, 0xD8, 0xFF],
            vec![0xFF, 0xD8, 0xFF, 0xE0, 0],
            vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 1],
            vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 20, b'Q'],
        ] {
            assert!(matches!(
                extract_data_part(&jpeg),
                Err(QuickRouteError::Truncated(_))
            ));
        }
    }

    #[test]
    fn ordinary_jpeg_has_no_quickroute_data() {
        let mut jpeg = vec![0xFF, 0xD8];
        jpeg_segment(&mut jpeg, 0xE0, b"JFIF\0");
        jpeg.extend_from_slice(&[0xFF, 0xD9]);
        assert_eq!(extract_data_part(&jpeg), Err(QuickRouteError::NoData));
        assert_eq!(
            extract_data_part(b"not a jpeg"),
            Err(QuickRouteError::NotJpeg)
        );
    }

    #[test]
    fn parses_black_relay_quickroute() {
        let p = std::env::var("TRACEREEL_TEST_QUICKROUTE_JPEG")
            .unwrap_or_else(|_| r"..\demo\黑色接力.gpx.jpg".to_string());
        if std::path::Path::new(&p).exists() {
            let data = std::fs::read(&p).expect("read jpeg");
            // Use 1000x1000 map dims; validation is that it returns a populated result.
            let qr = parse_quickroute(&data, 432.0, 614.0).expect("parse");
            assert!(
                qr.route_points.len() > 10,
                "expected many route points, got {}",
                qr.route_points.len()
            );
            assert!(qr.route_image_width > 0.0);
            // First point time must be 0 after finishing.
            assert!((qr.route_points[0].time).abs() < 1e-6);
        }
    }
}
