//! Video probing via ffmpeg: codec parameters (width/height/fps/duration) and the
//! presence of embedded motion data (DJI stores gyro data in "none"-codec data
//! streams), which drives the Gyroflow "has motion data" hint.
//!
//! We use ffmpeg-next (in-process) rather than spawning ffprobe so the app has no
//! external runtime dependency beyond the side-by-side ffmpeg DLLs.

use std::path::Path;

use ffmpeg_next as ffmpeg;
use ffmpeg_next::format::Pixel;
use ffmpeg_next::media::Type;

/// Probe result for a video file.
#[derive(Debug, Clone)]
pub struct VideoInfo {
    pub width: usize,
    pub height: usize,
    pub fps: f64,
    /// Duration in milliseconds.
    pub duration_ms: f64,
    /// True when the container carries a data stream (DJI "none"-codec motion data).
    pub has_motion_data: bool,
    /// Number of frames (rounded from duration * fps).
    pub frame_count: usize,
}

/// Probe a video file. Returns `None` when it cannot be opened or has no video stream.
pub fn probe_video(path: &str) -> Option<VideoInfo> {
    let _ = ffmpeg::init();

    let ictx = ffmpeg::format::input(path).ok()?;
    let st = ictx.streams().best(Type::Video)?;
    let cd = ffmpeg::codec::context::Context::from_parameters(st.parameters()).ok()?;
    let dec = cd.decoder().video().ok()?;
    let width = dec.width() as usize;
    let height = dec.height() as usize;

    let fr = st.avg_frame_rate();
    let fps = if fr.denominator() > 0 {
        fr.numerator() as f64 / fr.denominator() as f64
    } else {
        0.0
    };

    // Duration: prefer the VIDEO STREAM duration. The container duration also counts
    // trailing audio/metadata packets and can overshoot the last video frame (DJI files:
    // +70ms). Stitched timelines built from it run past decodable video, and the export
    // loop freezes on the held last frame at every seam. st.duration() is in
    // st.time_base() units (the container's is in AV_TIME_BASE microseconds).
    let tb = st.time_base();
    let stream_dur_us = if st.duration() > 0 && tb.denominator() > 0 {
        st.duration() as f64 * (tb.numerator() as f64 / tb.denominator() as f64) * 1_000_000.0
    } else {
        0.0
    };
    let duration_ms = if stream_dur_us > 0.0
        && (ictx.duration() <= 0 || stream_dur_us <= ictx.duration() as f64)
    {
        stream_dur_us / 1000.0
    } else {
        ictx.duration() as f64 / 1000.0
    };

    // Motion data: any data stream (DJI "none"-codec IMU stream). ffmpeg-next
    // exposes the stream media kind via Parameters::medium().
    let has_motion_data = ictx
        .streams()
        .any(|s| s.parameters().medium() == Type::Data);

    let frame_count = if duration_ms > 0.0 && fps > 0.0 {
        (duration_ms / 1000.0 * fps).round() as usize
    } else {
        0
    };

    Some(VideoInfo {
        width,
        height,
        fps,
        duration_ms,
        has_motion_data,
        frame_count,
    })
}

/// Return the codec `Id` (e.g. H264, HEVC) of the primary video stream.
pub fn video_codec_id(path: &str) -> Option<ffmpeg::codec::Id> {
    let _ = ffmpeg::init();
    let ictx = ffmpeg::format::input(path).ok()?;
    let st = ictx.streams().best(Type::Video)?;
    Some(st.parameters().id())
}

/// Gather the RGBA format that ffmpeg-next's software scaler should target.
pub fn rgba_pixel_format() -> Pixel {
    Pixel::RGBA
}

/// Convenience: true when the path looks like a video file (by extension).
pub fn is_video_path(p: &str) -> bool {
    let ext = Path::new(p)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(ext.as_str(), "mp4" | "mov" | "mkv" | "avi" | "m4v" | "webm")
}

#[cfg(test)]
mod tests {
    use super::*;

    // The demo DJI video should report motion data; the no-motion demo should not.
    // These require the demo assets present; guarded so CI without them still passes.
    #[test]
    fn probes_dji_video_when_present() {
        let p = std::env::var("TRACEREEL_TEST_DJI_VIDEO")
            .unwrap_or_else(|_| r"..\demo\DJI_20260823091939_0196_D.MP4".to_string());
        if std::path::Path::new(&p).exists() {
            let info = probe_video(&p).expect("probe dji");
            assert_eq!(info.width, 2688);
            assert_eq!(info.height, 2016);
            assert!(info.has_motion_data, "DJI video must report motion data");
        }
    }

    // Regression for the stitched-export seam freeze: this DJI clip's container
    // duration overshoots its video stream by ~70ms. The probe must report the
    // VIDEO STREAM duration so stitched timelines never run past decodable frames.
    #[test]
    fn prefers_stream_duration_over_container_when_present() {
        let p = std::env::var("TRACEREEL_TEST_DJI_0176")
            .unwrap_or_else(|_| r"..\demo\DJI_20260530095726_0176_D.MP4".to_string());
        if std::path::Path::new(&p).exists() {
            let info = probe_video(&p).expect("probe 0176");
            // ffprobe ground truth: stream=1461860.4ms, container=1461930.7ms.
            assert!(
                (info.duration_ms - 1461860.4).abs() < 5.0,
                "duration must be the video-stream length, got {}",
                info.duration_ms
            );
        }
    }
}
