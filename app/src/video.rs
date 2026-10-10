//! Software video decoder built on ffmpeg-next (in-process).
//!
//! Ported from the golden reference `gyroflow-frameserver/src/main.rs` `VideoDecoder`:
//! a persistent decoder that stays open across requests and only seeks when the
//! request moves out of range. This is the fallback/software path; the hardware
//! pipe decoder lands in M5.

/// Decode a single RGBA frame at an arbitrary time.
pub struct SoftDecoder {
    pub ictx: ffmpeg_next::format::context::Input,
    decoder: ffmpeg_next::decoder::Video,
    scaler: ffmpeg_next::software::scaling::Context,
    idx: usize,
    tb_f: f64,
    out_w: usize,
    out_h: usize,
    last_pts_ms: f64,
}

impl SoftDecoder {
    pub fn open(path: &str, out_w: usize, out_h: usize) -> Result<Self, String> {
        use ffmpeg_next::format::{input, Pixel};
        use ffmpeg_next::media::Type;
        use ffmpeg_next::software::scaling::{context::Context as SContext, flag::Flags};

        ffmpeg_next::init().map_err(|e| e.to_string())?;
        let ictx = input(path).map_err(|e| e.to_string())?;
        let st = ictx.streams().best(Type::Video).ok_or("no video stream")?;
        let idx = st.index();
        let cd = ffmpeg_next::codec::context::Context::from_parameters(st.parameters())
            .map_err(|e| e.to_string())?;
        let decoder = cd.decoder().video().map_err(|e| e.to_string())?;
        let scaler = SContext::get(
            decoder.format(),
            decoder.width(),
            decoder.height(),
            Pixel::RGBA,
            out_w as u32,
            out_h as u32,
            Flags::BILINEAR,
        )
        .map_err(|e| e.to_string())?;
        let tb_f = st.time_base().numerator() as f64 / st.time_base().denominator() as f64;
        Ok(Self {
            ictx,
            decoder,
            scaler,
            idx,
            tb_f,
            out_w,
            out_h,
            last_pts_ms: -1e9,
        })
    }

    /// Decode the frame at `t_ms` into `rgba` (out_w × out_h × 4).
    pub fn decode_at(&mut self, t_ms: f64, rgba: &mut [u8]) -> Result<(), String> {
        use ffmpeg_next::util::frame::video::Video;
        let seek = t_ms < self.last_pts_ms || t_ms - self.last_pts_ms > 2000.0;
        if seek {
            // avformat_seek_file with stream_index=-1 uses AV_TIME_BASE (µs).
            let to = ((t_ms / 1000.0) * 1_000_000.0) as i64;
            self.ictx
                .seek(to.min(self.ictx.duration()), ..to.min(self.ictx.duration()))
                .map_err(|e| e.to_string())?;
            self.last_pts_ms = -1e9;
        }
        let mut decoded = Video::empty();
        for (stream, packet) in self.ictx.packets() {
            if stream.index() != self.idx {
                continue;
            }
            if let Err(e) = self.decoder.send_packet(&packet) {
                log::debug!("send_packet err {e}");
                continue;
            }
            while self.decoder.receive_frame(&mut decoded).is_ok() {
                if let Some(pts) = decoded.pts() {
                    let p_ms = pts as f64 * self.tb_f * 1000.0;
                    // Stale pre-seek frames: skip (don't flush; that breaks send_packet).
                    if p_ms < t_ms - 1.0 {
                        continue;
                    }
                    let mut rgb = ffmpeg_next::util::frame::video::Video::empty();
                    self.scaler
                        .run(&decoded, &mut rgb)
                        .map_err(|e| e.to_string())?;
                    for y in 0..self.out_h {
                        let src = y as usize * rgb.stride(0);
                        let dst = y * self.out_w * 4;
                        if dst + self.out_w * 4 <= rgba.len() {
                            rgba[dst..dst + self.out_w * 4]
                                .copy_from_slice(&rgb.data(0)[src..src + self.out_w * 4]);
                        }
                    }
                    self.last_pts_ms = p_ms;
                    return Ok(());
                }
            }
        }
        Err("no frame".to_string())
    }
}

/// Format a time in milliseconds as `HH:MM:SS.mmm` (used for CLI/tests).
pub fn fmt_ms(ms: f64) -> String {
    let total = (ms / 1000.0).max(0.0);
    let h = (total / 3600.0) as u64;
    let m = ((total as u64) / 60) % 60;
    let s = (total as u64) % 60;
    let frac = ((total.fract()) * 1000.0).round() as u64;
    format!("{h:02}:{m:02}:{s:02}.{frac:03}")
}
