//! Preview audio engine: decodes the segment under playback with `ffmpeg.exe`
//! into raw s16le PCM (48 kHz stereo) and renders it through the WinMM
//! `waveOut*` API — no new crates, no Qt Multimedia dependency.
//!
//! Speed changes keep the pitch: the ffmpeg command gets an `atempo` chain
//! (see [`crate::encode::atempo_chain`]), which warps tempo only.
//!
//! The GUI thread only ever touches the lock-protected request slot:
//! [`PreviewAudio::play`] is called from the 16 ms tick with the desired
//! (path, source-time, speed) triple and is a no-op while the running session
//! already covers that position (±drift window). A larger drift — a seek, a
//! speed change, a loop restart — transparently respawns the decoder session.
//!
//! Non-Windows builds compile to a silent no-op stub.

/// Handle owned by the GUI item; cheap to call from the tick loop.
pub struct PreviewAudio(#[cfg(windows)] engine::Engine);

impl PreviewAudio {
    /// Spawn the audio worker thread (lives for the whole process; the worker
    /// only decodes when a session is active and parks otherwise).
    pub fn new() -> Self {
        #[cfg(windows)]
        {
            Self(engine::Engine::start())
        }
        #[cfg(not(windows))]
        {
            Self
        }
    }

    /// Make the audio play `path` from `start_ms` (file-local time) at `speed`.
    /// Idempotent while the current session already covers that position.
    pub fn play(&self, path: &str, start_ms: f64, speed: f64) {
        #[cfg(windows)]
        self.0.play(path, start_ms, speed);
        #[cfg(not(windows))]
        {
            let _ = (path, start_ms, speed);
        }
    }

    /// Freeze/resume the active session without respawning the decoder.
    pub fn set_paused(&self, paused: bool) {
        #[cfg(windows)]
        self.0.set_paused(paused);
        #[cfg(not(windows))]
        let _ = paused;
    }

    /// Tear the active session down immediately (project reload / shutdown).
    pub fn stop(&self) {
        #[cfg(windows)]
        self.0.stop();
    }

    /// 当前是否有一条活着的解码会话（诊断/回归用；暂停中仍算活着）。
    pub fn running(&self) -> bool {
        #[cfg(windows)]
        {
            self.0.running()
        }
        #[cfg(not(windows))]
        {
            false
        }
    }
}

#[cfg(windows)]
mod engine {
    use std::collections::VecDeque;
    use std::ffi::c_void;
    use std::io::Read;
    use std::mem::size_of;
    use std::process::{Child, ChildStdout, Stdio};
    use crate::process::background_command;
    use std::ptr;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use crate::encode::{atempo_chain, find_ffmpeg};

    // ------------------------------------------------------------------ winmm

    type MMRESULT = u32;
    type HWAVEOUT = *mut c_void;
    const WAVE_MAPPER: usize = 0xFFFF_FFFF;
    const WHDR_DONE: u32 = 0x0001;
    const WAVE_FORMAT_PCM: u16 = 1;
    const CALLBACK_NULL: u32 = 0;
    /// 20 ms of 48 kHz stereo s16le.
    const CHUNK: usize = 3840;
    /// Queue depth in chunks (~160 ms prebuffer).
    const QUEUE: usize = 8;

    #[repr(C)]
    struct WAVEFORMATEX {
        w_format_tag: u16,
        n_channels: u16,
        n_samples_per_sec: u32,
        n_avg_bytes_per_sec: u32,
        n_block_align: u16,
        w_bits_per_sample: u16,
        cb_size: u16,
    }

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct WAVEHDR {
        lp_buffer: *mut i8,
        dw_buffer_length: u32,
        dw_bytes_recorded: u32,
        dw_user: usize,
        dw_flags: u32,
        dw_loops: u32,
        lp_next: *mut WAVEHDR,
        reserved: usize,
    }

    extern "system" {
        fn waveOutOpen(
            phwo: *mut HWAVEOUT,
            u_device_id: usize,
            pwfx: *const WAVEFORMATEX,
            dw_callback: usize,
            dw_instance: usize,
            fdw_open: u32,
        ) -> MMRESULT;
        fn waveOutClose(hwo: HWAVEOUT) -> MMRESULT;
        fn waveOutPrepareHeader(hwo: HWAVEOUT, pwh: *mut WAVEHDR, cb: u32) -> MMRESULT;
        fn waveOutUnprepareHeader(hwo: HWAVEOUT, pwh: *mut WAVEHDR, cb: u32) -> MMRESULT;
        fn waveOutWrite(hwo: HWAVEOUT, pwh: *mut WAVEHDR, cb: u32) -> MMRESULT;
        fn waveOutReset(hwo: HWAVEOUT) -> MMRESULT;
        fn waveOutPause(hwo: HWAVEOUT) -> MMRESULT;
        fn waveOutRestart(hwo: HWAVEOUT) -> MMRESULT;
    }

    // ------------------------------------------------------------- shared state

    #[derive(Clone)]
    struct Current {
        path: String,
        start_ms: f64,
        speed: f64,
    }

    #[derive(Default)]
    struct Shared {
        /// Latest pending request: (path, start_ms, speed, generation).
        pending: Option<(String, f64, f64, u64)>,
        generation: u64,
        paused: bool,
        current: Option<Current>,
        /// PCM bytes handed to the device by the *current* session.
        played: Arc<AtomicU64>,
        /// (path, speed, when): spawn failure cooldown.
        last_fail: Option<(String, f64, Instant)>,
        /// (path, speed, when): last restart issued while a session was alive
        /// (protects against restart storms on slow cold starts).
        last_req: Option<(String, f64, Instant)>,
    }

    impl Shared {
        /// Estimated source-timeline position of the running session.
        fn position_ms(cur: &Current, played: &AtomicU64) -> f64 {
            let bytes = played.load(Relaxed) as f64;
            cur.start_ms + bytes * cur.speed / 192.0
        }
    }

    // ------------------------------------------------------------------ engine

    pub struct Engine {
        shared: Arc<Mutex<Shared>>,
        quit: Arc<AtomicBool>,
        kill: Arc<AtomicBool>,
    }

    impl Engine {
        pub fn start() -> Self {
            let shared = Arc::new(Mutex::new(Shared::default()));
            let quit = Arc::new(AtomicBool::new(false));
            let kill = Arc::new(AtomicBool::new(false));
            let (s, q, k) = (shared.clone(), quit.clone(), kill.clone());
            std::thread::spawn(move || run(s, q, k));
            Self { shared, quit, kill }
        }

        pub fn play(&self, path: &str, start_ms: f64, speed: f64) {
            let sp = if speed.is_finite() && speed > 0.0 {
                speed
            } else {
                1.0
            };
            let mut s = self.shared.lock().unwrap();
            if let Some(cur) = &s.current {
                if cur.path == path && (cur.speed - sp).abs() < 1e-3 {
                    let pos = Shared::position_ms(cur, &s.played);
                    if (pos - start_ms).abs() <= 350.0 {
                        return; // already covering the requested position
                    }
                    // Drift correction, but at most once per second for the
                    // same (path, speed) to avoid respawning in a tight loop.
                    if let Some((p, v, t)) = &s.last_req {
                        if p == path
                            && (v - sp).abs() < 1e-3
                            && t.elapsed() < Duration::from_secs(1)
                        {
                            return;
                        }
                    }
                }
            }
            if s.pending.is_some() {
                return; // a request is already queued for the worker
            }
            if let Some((p, v, t)) = &s.last_fail {
                if p == path && (v - sp).abs() < 1e-3 && t.elapsed() < Duration::from_secs(2) {
                    return; // decoder spawn failed recently: cool down
                }
            }
            s.generation += 1;
            s.last_req = Some((path.to_string(), sp, Instant::now()));
            s.pending = Some((path.to_string(), start_ms, sp, s.generation));
        }

        pub fn set_paused(&self, paused: bool) {
            self.shared.lock().unwrap().paused = paused;
        }

        /// 当前是否有一条活着的解码会话（供诊断/回归；暂停中仍算活着）。
        pub fn running(&self) -> bool {
            self.shared
                .lock()
                .map(|s| s.current.is_some())
                .unwrap_or(false)
        }

        pub fn stop(&self) {
            let mut s = self.shared.lock().unwrap();
            s.pending = None;
            s.current = None;
            s.played.store(0, Relaxed);
            self.kill.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    impl Drop for Engine {
        fn drop(&mut self) {
            // Tell the worker to finish up and exit its loop.
            self.quit.store(true, std::sync::atomic::Ordering::SeqCst);
            self.kill.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    // ----------------------------------------------------------------- session

    /// One chunk buffer + its header; boxed so the addresses winmm sees are
    /// stable for the whole life of the session.
    struct Chunk {
        hdr: WAVEHDR,
        buf: Vec<u8>,
        len: usize,
    }

    impl Chunk {
        fn new() -> Box<Self> {
            Box::new(Self {
                hdr: WAVEHDR::default(),
                buf: vec![0u8; CHUNK],
                len: 0,
            })
        }
    }

    struct Session {
        child: Child,
        stdout: Option<ChildStdout>,
        hwave: HWAVEOUT,
        /// Submitted / playing chunks, FIFO (winmm consumes in submission order).
        queued: VecDeque<Box<Chunk>>,
        free: VecDeque<Box<Chunk>>,
        paused: bool,
        eof: bool,
    }

    impl Session {
        fn start(path: &str, start_ms: f64, speed: f64) -> Result<Self, String> {
            let ffmpeg = find_ffmpeg();
            let chain = atempo_chain(speed);
            let mut c = background_command(&ffmpeg);
            c.args(["-hide_banner", "-loglevel", "error"]);
            c.args(["-ss", &format!("{:.3}", start_ms.max(0.0) / 1000.0)]);
            c.arg("-i").arg(path);
            c.args(["-vn", "-sn", "-dn"]);
            if !chain.is_empty() {
                c.args(["-af", &chain]);
            }
            c.args(["-f", "s16le", "-ar", "48000", "-ac", "2", "pipe:1"]);
            c.stdin(Stdio::null())
                .stderr(Stdio::null())
                .stdout(Stdio::piped());
            let mut child = c.spawn().map_err(|e| format!("spawn ffmpeg: {e}"))?;
            let stdout = child.stdout.take().ok_or("ffmpeg stdout unavailable")?;
            let fmt = WAVEFORMATEX {
                w_format_tag: WAVE_FORMAT_PCM,
                n_channels: 2,
                n_samples_per_sec: 48000,
                n_avg_bytes_per_sec: 192000,
                n_block_align: 4,
                w_bits_per_sample: 16,
                cb_size: 0,
            };
            let hwave = unsafe {
                let mut h: HWAVEOUT = ptr::null_mut();
                let r = waveOutOpen(&mut h, WAVE_MAPPER, &fmt, 0, 0, CALLBACK_NULL);
                if r != 0 {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("waveOutOpen failed: {r}"));
                }
                h
            };
            Ok(Self {
                child,
                stdout: Some(stdout),
                hwave,
                queued: VecDeque::new(),
                free: (0..QUEUE).map(|_| Chunk::new()).collect(),
                paused: false,
                eof: false,
            })
        }

        fn set_paused(&mut self, p: bool) {
            if p && !self.paused {
                unsafe { waveOutPause(self.hwave) };
                self.paused = true;
            } else if !p && self.paused {
                unsafe { waveOutRestart(self.hwave) };
                self.paused = false;
            }
        }

        fn pump(&mut self, played: &AtomicU64) {
            // Reclaim finished chunks in FIFO order; count them as played.
            loop {
                let done = match self.queued.front() {
                    Some(c) => (c.hdr.dw_flags & WHDR_DONE) != 0,
                    None => break,
                };
                if !done {
                    break;
                }
                let mut c = self.queued.pop_front().unwrap();
                played.fetch_add(c.len as u64, Relaxed);
                unsafe {
                    waveOutUnprepareHeader(self.hwave, &mut c.hdr, size_of::<WAVEHDR>() as u32)
                };
                c.len = 0;
                self.free.push_back(c);
            }
            // Feed the device while it has room; the (blocking) read is bounded
            // to one chunk, so the worker still reacts to requests quickly.
            while !self.paused && !self.eof && !self.free.is_empty() {
                let mut c = self.free.pop_front().unwrap();
                let mut n = 0usize;
                let mut stream_done = false;
                match self.stdout.as_mut() {
                    Some(o) => match o.read(&mut c.buf[..CHUNK]) {
                        Ok(0) => stream_done = true,
                        Ok(k) => n = k,
                        Err(_) => stream_done = true,
                    },
                    None => stream_done = true,
                }
                if stream_done {
                    self.eof = true;
                    self.free.push_back(c);
                    break;
                }
                c.len = n;
                unsafe {
                    c.hdr = WAVEHDR::default();
                    c.hdr.lp_buffer = c.buf.as_mut_ptr() as *mut i8;
                    c.hdr.dw_buffer_length = n as u32;
                    let cb = size_of::<WAVEHDR>() as u32;
                    if waveOutPrepareHeader(self.hwave, &mut c.hdr, cb) == 0 {
                        waveOutWrite(self.hwave, &mut c.hdr, cb);
                    }
                }
                self.queued.push_back(c);
            }
        }

        fn finished(&self) -> bool {
            self.eof && self.queued.is_empty()
        }

        fn close(mut self) {
            unsafe {
                waveOutReset(self.hwave);
                for c in self.queued.iter_mut() {
                    waveOutUnprepareHeader(self.hwave, &mut c.hdr, size_of::<WAVEHDR>() as u32);
                }
                waveOutClose(self.hwave);
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    // ------------------------------------------------------------------ worker

    fn run(shared: Arc<Mutex<Shared>>, quit: Arc<AtomicBool>, kill: Arc<AtomicBool>) {
        let mut session: Option<Session> = None;
        while !quit.load(Relaxed) {
            let req;
            let want_kill;
            let paused;
            {
                let mut s = shared.lock().unwrap();
                req = s.pending.take();
                want_kill = kill.load(std::sync::atomic::Ordering::SeqCst);
                if want_kill {
                    kill.store(false, std::sync::atomic::Ordering::SeqCst);
                }
                paused = s.paused;
            }
            if want_kill {
                if let Some(se) = session.take() {
                    se.close();
                }
            }
            if let Some((path, start_ms, speed, _gen)) = req {
                if let Some(se) = session.take() {
                    se.close();
                }
                match Session::start(&path, start_ms, speed) {
                    Ok(mut se) => {
                        se.set_paused(paused);
                        let mut s = shared.lock().unwrap();
                        s.played.store(0, Relaxed);
                        s.current = Some(Current {
                            path,
                            start_ms,
                            speed,
                        });
                        session = Some(se);
                    }
                    Err(_) => {
                        let mut s = shared.lock().unwrap();
                        s.current = None;
                        s.last_fail = Some((path, speed, Instant::now()));
                    }
                }
            }
            if let Some(se) = session.as_mut() {
                se.set_paused(paused);
                let played = shared.lock().unwrap().played.clone();
                se.pump(&played);
                if se.finished() {
                    let se = session.take().unwrap();
                    se.close();
                    shared.lock().unwrap().current = None;
                }
            } else {
                std::thread::sleep(Duration::from_millis(15));
            }
        }
        if let Some(se) = session.take() {
            se.close();
        }
    }
}
