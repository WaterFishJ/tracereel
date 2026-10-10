//! Hardware-accelerated video decode via an ffmpeg subprocess streaming RGBA frames.
//!
//! Ported from the golden reference `gyroflow-frameserver/src/main.rs` `PipeDecoder`.
//! The subprocess decodes with QSV (falling back to d3d11va) and writes RGBA rawvideo
//! to stdout. A reader thread keeps a small ring of the most recent frames keyed by
//! their position on the video frame grid, paced tightly to the consumer position, so
//! sequential requests are served from memory at interactive speed. Seeking respawns
//! the subprocess.
//!
//! Everything here is NON-BLOCKING for the caller: `spawn_at` only launches the ffmpeg
//! process (no wait for the first frame) and `get` waits at most a few tens/hundreds of
//! milliseconds for live delivery, so the GUI thread never freezes on the decoder.
//! This decodes ~6.7ms/frame (d3d11va at 1440×1080) vs ~50ms for the in-process software
//! decoder, which is the difference between choppy and smooth 59.94fps playback.

use std::collections::BTreeMap;
use std::io::Read;
use std::process::Stdio;
use crate::process::background_command;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

/// Process-wide learned hardware mode: 0 = unknown (try QSV first), 1 = d3d11va is the
/// working chain. Persisted across `PipeDecoder` instances so a seek (which builds a fresh
/// decoder) does NOT re-attempt the QSV chain every time — on some ffmpeg builds the QSV
/// `hwdownload→rgba` path neither emits frames nor exits (it just hangs alive), which
/// starves the ring and freezes playback. Once we learn d3d11va, we start on it.
static HW_KNOWN: AtomicUsize = AtomicUsize::new(0);

fn find_ffmpeg_exe() -> String {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let cand = dir.join("ffmpeg.exe");
            if cand.exists() {
                return cand.to_string_lossy().into_owned();
            }
        }
    }
    "ffmpeg.exe".to_string()
}

/// Hardware pipe decoder (d3d11va/QSV via an ffmpeg subprocess).
pub struct PipeDecoder {
    video: String,
    w: usize,
    h: usize,
    frame_bytes: usize,
    fps: f64,
    ffmpeg: String,
    child: Option<std::process::Child>,
    ring: Arc<std::sync::Mutex<BTreeMap<i64, Arc<Vec<u8>>>>>,
    alive: Arc<AtomicBool>,
    last_req: Arc<AtomicI64>,
    gen: Arc<AtomicU64>,
    /// 0 = unknown (try qsv first), 1 = d3d11va is the working chain.
    known_mode: usize,
    /// True once at least one frame was delivered (cold start uses a shorter wait).
    started: bool,
    /// When the current chain was (re)spawned — for the QSV-starve watchdog.
    spawned_at: Option<Instant>,
    /// Position (ms) the current chain was seeked to by `spawn_at`. A request that has
    /// drifted far away from it can never be served by this chain (the reader thread drops
    /// every frame older than `last_req - 1000`), so `get` respawns instead of spinning.
    spawn_t: i64,
    pub mode: String,
}

impl PipeDecoder {
    pub fn new(video: &str, w: usize, h: usize, fps: f64) -> Self {
        Self {
            video: video.to_string(),
            w,
            h,
            frame_bytes: w * h * 4,
            fps: if fps > 0.0 { fps } else { 30.0 },
            ffmpeg: find_ffmpeg_exe(),
            child: None,
            ring: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
            alive: Arc::new(AtomicBool::new(false)),
            last_req: Arc::new(AtomicI64::new(0)),
            gen: Arc::new(AtomicU64::new(0)),
            known_mode: HW_KNOWN.load(Ordering::SeqCst),
            started: false,
            spawned_at: None,
            spawn_t: 0,
            mode: String::new(),
        }
    }

    pub fn kill(&mut self) {
        self.alive.store(false, Ordering::SeqCst);
        self.gen.fetch_add(1, Ordering::SeqCst);
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    /// The current chain has permanently failed (process dead), i.e. the software
    /// decoder fallback should take over.
    pub fn dead(&self) -> bool {
        !self.alive.load(Ordering::SeqCst)
    }

    /// (Re)spawn the decoder at t_ms. NON-BLOCKING: only launches the process.
    pub fn spawn_at(&mut self, t_ms: f64) -> bool {
        self.kill();
        self.ring.lock().unwrap().clear();
        self.last_req.store(t_ms.round() as i64, Ordering::SeqCst);
        self.spawn_t = t_ms.round() as i64;
        let dt = 1000.0 / self.fps;
        // Snap the seek to the video's frame grid so the k-th delivered frame has time
        // (start_frame+k)*dt.
        let start_frame = ((t_ms / dt).round() as i64 - 1).max(0);
        let ss = (start_frame as f64 * dt / 1000.0).max(0.0);
        let w = self.w;
        let h = self.h;
        let (name, args) = if self.known_mode == 1 {
            (
                "d3d11va",
                vec![
                    "-hwaccel".into(),
                    "d3d11va".into(),
                    "-ss".into(),
                    format!("{ss:.6}"),
                    "-i".into(),
                    self.video.clone(),
                    "-vf".into(),
                    format!("scale={w}:{h},format=rgba"),
                    "-an".into(),
                    "-f".into(),
                    "rawvideo".into(),
                    "pipe:1".into(),
                ],
            )
        } else {
            (
                "qsv",
                vec![
                    "-hwaccel".into(),
                    "qsv".into(),
                    "-hwaccel_output_format".into(),
                    "qsv".into(),
                    "-ss".into(),
                    format!("{ss:.6}"),
                    "-i".into(),
                    self.video.clone(),
                    "-vf".into(),
                    format!("vpp_qsv=w={w}:h={h}:format=nv12,hwdownload,format=rgba"),
                    "-an".into(),
                    "-f".into(),
                    "rawvideo".into(),
                    "pipe:1".into(),
                ],
            )
        };
        self.spawn_chain(name, args)
    }

    fn spawn_chain(&mut self, name: &str, args: Vec<String>) -> bool {
        let mut cmd = background_command(&self.ffmpeg);
        cmd.args(["-hide_banner", "-loglevel", "error"])
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let Ok(mut child) = cmd.spawn() else {
            return false;
        };
        let Some(mut stdout) = child.stdout.take() else {
            let _ = child.kill();
            return false;
        };
        let Some(stderr) = child.stderr.take() else {
            let _ = child.kill();
            return false;
        };
        let my_gen = self.gen.load(Ordering::SeqCst);
        let ring = self.ring.clone();
        let alive = self.alive.clone();
        let last_req = self.last_req.clone();
        let gen = self.gen.clone();
        let dt = 1000.0 / self.fps;
        alive.store(true, Ordering::SeqCst);
        self.spawned_at = Some(Instant::now());

        // stderr tail for diagnostics.
        let err_tail: Arc<std::sync::Mutex<Vec<String>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        {
            let err_tail = err_tail.clone();
            std::thread::spawn(move || {
                use std::io::BufRead;
                let r = std::io::BufReader::new(stderr);
                for line in r.lines() {
                    let Ok(line) = line else { break };
                    if !line.trim().is_empty() {
                        let mut t = err_tail.lock().unwrap();
                        t.push(line);
                        if t.len() > 10 {
                            t.remove(0);
                        }
                    }
                }
            });
        }

        // stdout: read frames keyed by grid position, paced to the consumer.
        let frame_bytes = self.frame_bytes;
        let start_frame =
            ((self.last_req.load(Ordering::SeqCst) as f64 / dt).round() as i64 - 1).max(0);
        std::thread::spawn(move || {
            let mut buf = vec![0u8; frame_bytes];
            let mut k: i64 = 0;
            loop {
                if gen.load(Ordering::SeqCst) != my_gen {
                    return;
                }
                let mut got = 0usize;
                while got < frame_bytes {
                    match stdout.read(&mut buf[got..]) {
                        Ok(0) => {
                            if gen.load(Ordering::SeqCst) == my_gen {
                                alive.store(false, Ordering::SeqCst);
                            }
                            return;
                        }
                        Ok(n) => got += n,
                        Err(_) => {
                            if gen.load(Ordering::SeqCst) == my_gen {
                                alive.store(false, Ordering::SeqCst);
                            }
                            return;
                        }
                    }
                }
                let pts_ms = ((start_frame + k) as f64 * dt).round() as i64;
                k += 1;
                if pts_ms < last_req.load(Ordering::SeqCst) - 1000 {
                    continue;
                }
                while pts_ms > last_req.load(Ordering::SeqCst) + 100 {
                    if gen.load(Ordering::SeqCst) != my_gen {
                        return;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                let frame = Arc::new(std::mem::take(&mut buf));
                buf = vec![0u8; frame_bytes];
                {
                    let mut ring = ring.lock().unwrap();
                    ring.insert(pts_ms, frame);
                    if ring.len() > 8 {
                        let first = *ring.keys().next().unwrap();
                        ring.remove(&first);
                    }
                }
            }
        });
        self.child = Some(child);
        self.mode = name.to_string();
        eprintln!("[pipe] spawning {name}");
        true
    }

    /// Fetch the RGBA frame nearest t_ms, waiting briefly for live delivery.
    /// Never blocks long: cold start ~40ms, warm ~180ms per call.
    pub fn get(&mut self, t_ms: f64) -> Option<Arc<Vec<u8>>> {
        self.last_req.store(t_ms.round() as i64, Ordering::SeqCst);
        let dt = 1000.0 / self.fps;
        let lo = (t_ms - 0.55 * dt) as i64;
        let hi = (t_ms + 0.55 * dt) as i64;
        let mut ring_empty = true;
        {
            let ring = self.ring.lock().unwrap();
            ring_empty = ring.is_empty();
            if let Some((&_p, f)) = ring.range(lo..=hi).next() {
                if self.mode == "d3d11va" {
                    self.known_mode = 1;
                    HW_KNOWN.store(1, Ordering::SeqCst);
                }
                if !self.started {
                    if let Some(sp) = self.spawned_at {
                        eprintln!(
                            "[pipe] first frame after {}ms (mode={}, t={t_ms:.0})",
                            sp.elapsed().as_millis(),
                            self.mode
                        );
                    }
                }
                self.started = true;
                return Some(f.clone());
            }
            // 请求是否离这条链"能给得出帧"的范围太远（判据见 `ring_stale`）。
            let bounds = ring
                .iter()
                .next_back()
                .map(|(&maxp, _)| (*ring.keys().next().unwrap_or(&maxp), maxp));
            let stale = ring_stale(bounds, self.spawn_t, t_ms);
            if stale {
                drop(ring);
                eprintln!("[pipe] stale ring -> cold respawn at t={t_ms:.0}");
                self.spawn_at(t_ms);
                return None;
            }
        }
        // QSV-starve watchdog: on some ffmpeg builds the QSV `hwdownload→rgba` chain neither
        // emits a frame NOR exits — it hangs *alive*, so the `!alive` fallback below never
        // fires and playback wedges (the empty ring is also not "stale", so no respawn). If
        // QSV has produced nothing for a while, force-switch to d3d11va and remember it.
        // `ring_empty` matters: a pre-warmed pipe parks with frames buffered but
        // `started == false` and an old `spawned_at`; restarting on the first miss there
        // would throw away the warm start and re-stall playback exactly at the seam.
        if self.mode == "qsv" && !self.started && ring_empty {
            if let Some(sp) = self.spawned_at {
                if sp.elapsed() > std::time::Duration::from_millis(700) {
                    eprintln!("[pipe] qsv watchdog fired -> d3d11va respawn at t={t_ms:.0}");
                    self.known_mode = 1;
                    HW_KNOWN.store(1, Ordering::SeqCst);
                    self.spawn_at(t_ms);
                    return None;
                }
            }
        }
        let wait_ms = if self.started { 180 } else { 40 };
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(wait_ms);
        while std::time::Instant::now() < deadline {
            {
                let ring = self.ring.lock().unwrap();
                if let Some((&_p, f)) = ring.range(lo..=hi).next() {
                    if self.mode == "d3d11va" {
                        self.known_mode = 1;
                        HW_KNOWN.store(1, Ordering::SeqCst);
                    }
                    if !self.started {
                        if let Some(sp) = self.spawned_at {
                            eprintln!(
                                "[pipe] first frame after {}ms (mode={}, t={t_ms:.0}, waited)",
                                sp.elapsed().as_millis(),
                                self.mode
                            );
                        }
                    }
                    self.started = true;
                    return Some(f.clone());
                }
            }
            if !self.alive.load(Ordering::SeqCst) {
                // The qsv chain has a known avbuild defect (hwdownload rgba fails); when
                // it dies before the first frame, fall back to d3d11va once.
                if self.mode == "qsv" && self.known_mode == 0 {
                    eprintln!("[pipe] qsv chain died -> d3d11va respawn at t={t_ms:.0}");
                    self.known_mode = 1;
                    HW_KNOWN.store(1, Ordering::SeqCst);
                    self.spawn_at(t_ms);
                    return None;
                }
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        eprintln!(
            "[pipe] get({t_ms:.0}) miss: mode={} started={} alive={} ring={}",
            self.mode,
            self.started,
            self.alive.load(Ordering::SeqCst),
            self.ring.lock().map(|r| r.len()).unwrap_or(0)
        );
        None
    }

    /// Last buffered frame for a request that ran PAST the decodable end of the file
    /// (chain hit EOF, `t_ms` is beyond the newest ring frame). Lets an offline export
    /// hold the final frame instead of falling back to a software decode that also
    /// fails at EOF. Returns None while the chain is still live or when the request
    /// is not past the buffered end (a mid-file stall should go through software).
    pub fn eof_frame(&self, t_ms: f64) -> Option<Arc<Vec<u8>>> {
        if self.alive.load(Ordering::SeqCst) {
            return None;
        }
        let ring = self.ring.lock().unwrap();
        let (&maxp, f) = ring.iter().next_back()?;
        if t_ms > maxp as f64 {
            Some(f.clone())
        } else {
            None
        }
    }
}

impl Drop for PipeDecoder {
    fn drop(&mut self) {
        self.kill();
    }
}

/// 请求 `t_ms` 与当前链"能给得出帧"的范围是否已经远到这条链**永远**给不出来。
///
/// 判据分两种：
/// * ring 里有帧 → 与**最近/最远帧**比较（旧有逻辑，处理播放头前后跳太久）；
/// * ring 是空的（链还在预热/追赶）→ 与**建链位置** `spawn_t` 比较。这一步是必需的：
///   读线程会丢弃所有 `pts_ms < last_req - 1000` 的帧，所以一条建在 t=0 的链，在被请求
///   T（分钟级）时永远填不满 ring；旧代码在空 ring 时一律返回 `false`，于是既不复位也不
///   重开 —— 播放中改设置后"预览黑屏、地图/HUD 冻住、只有进度条在走"就是这么来的
///   （拖进度条 / 暂停再播放能救，因为那两条路会显式重开链）。
///
/// 阈值见下面两个常量；这里只做纯判断，便于单测覆盖两个分支。
///
/// ring 非空时沿用的旧判据：正常播放时相邻请求间隔远小于这个余量，只有播放头前后跳太久才触发。
const RING_BOUNDS_SLACK_MS: f64 = 1500.0;

/// ring 仍为空（链在预热/追赶）时，请求离建链位置多远才算"这条链永远给不出来"。
///
/// 这个值必须比非空分支宽得多：空 ring 期间链正忙着打开文件、起解码器，首帧慢是正常的
/// （实测 QSV 首次建链要数秒）。若照 1.5 s 判，慢启动会在出首帧之前就被反复杀掉重开，
/// 每 1.5 s 重开一次就永远等不到首帧。放到 5 s 后：`spawn_at` 会把 `spawn_t` 设成请求位置，
/// 重开一次要再漂 5 s 才会重开第二次；而真正需要自愈的大跳（链建在 0、请求已在 412 s）
/// 依旧立刻重开 —— 5 s 与分钟级的错位相比可以忽略。
const EMPTY_RING_DRIFT_MS: f64 = 5000.0;

/// 见上方常量与函数文档：返回 `true` 时调用方会冷启动到请求位置（自愈）。
fn ring_stale(bounds: Option<(i64, i64)>, spawn_t: i64, t_ms: f64) -> bool {
    match bounds {
        Some((minp, maxp)) => {
            t_ms > maxp as f64 + RING_BOUNDS_SLACK_MS || t_ms < minp as f64 - RING_BOUNDS_SLACK_MS
        }
        None => (t_ms - spawn_t as f64).abs() > EMPTY_RING_DRIFT_MS,
    }
}

#[cfg(test)]
mod tests {
    use super::ring_stale;

    /// 空 ring 活锁：请求离建链位置太远必须判"无效链"（回到 `get` 里就会冷启动到请求位置），
    /// 而正常播放/慢启动/小跳不能被误判（否则每帧都会重开链，画面会更糟）。
    #[test]
    fn empty_ring_far_jump_is_stale() {
        // 播放中改设置重载：链建在 0，请求已在 412_000ms（约 6 分 52 秒）—— 必须重开。
        assert!(ring_stale(None, 0, 412_000.0));
        // 慢启动：首帧还没出来、播放头才走了 1.4 s / 4.9 s，不能杀链重开。
        assert!(!ring_stale(None, 0, 1_400.0));
        assert!(!ring_stale(None, 0, 1_500.0));
        assert!(!ring_stale(None, 0, 4_900.0));
        // 建链点 5_000，请求 5_600：只漂了 0.6 s，不算。
        assert!(!ring_stale(None, 5_000, 5_600.0));
        // 请求落后建链点 2 s，同样在宽限内（慢启动期的正常追赶）。
        assert!(!ring_stale(None, 5_000, 3_000.0));
        // 超过 5 s 的错位（正、反两个方向）都要自愈。
        assert!(ring_stale(None, 5_000, 12_000.0));
        assert!(ring_stale(None, 12_000, 5_000.0));
    }

    /// ring 非空时沿用旧判据（只跟帧的范围比），与建链位置无关。
    #[test]
    fn non_empty_ring_uses_frame_bounds() {
        let bounds = Some((1_000, 2_000));
        assert!(!ring_stale(bounds, 0, 1_500.0));
        assert!(!ring_stale(bounds, 0, 3_400.0));
        assert!(ring_stale(bounds, 0, 3_600.0));
        assert!(ring_stale(bounds, 0, -700.0));
    }
}
