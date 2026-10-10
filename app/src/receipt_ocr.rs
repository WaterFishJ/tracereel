//! Background Rust PaddleOCR with cached CPU sessions. No image data leaves the machine.
use qmetaobject::*;
use std::sync::{Arc, Mutex};

#[derive(QObject, Default)]
pub struct ReceiptOcr {
    base: qt_base_class!(trait QObject),
    busy: qt_property!(bool; NOTIFY changed),
    result: qt_property!(QString; NOTIFY changed),
    error: qt_property!(QString; NOTIFY changed),
    changed: qt_signal!(),
    pending: Arc<Mutex<Option<Result<String, String>>>>,
    start: qt_method!(
        fn start(&mut self, request: QString) {
            if self.busy {
                return;
            }
            self.busy = true;
            self.result = QString::default();
            self.error = QString::default();
            self.changed();
            let pending = self.pending.clone();
            let request = request.to_string();
            std::thread::spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&request)))
                    .unwrap_or_else(|_| Err("OCR 引擎异常，请重启应用后重试".into()));
                *pending.lock().unwrap() = Some(result);
            });
        }
    ),
    poll: qt_method!(
        fn poll(&mut self) {
            let value = self.pending.lock().unwrap().take();
            if let Some(value) = value {
                self.busy = false;
                match value {
                    Ok(s) => self.result = s.into(),
                    Err(e) => self.error = e.into(),
                }
                self.changed();
            }
        }
    ),
}

fn run(request: &str) -> Result<String, String> {
    // The mutex serializes requests across dialogs and keeps the three sessions
    // alive. Image processing and inference always run outside the GUI thread.
    static ENGINE: std::sync::OnceLock<Mutex<Option<receipt_ocr::Engine>>> = std::sync::OnceLock::new();
    let request: receipt_ocr::Request = serde_json::from_str(request)
        .map_err(|e| format!("OCR 请求无效：{e}"))?;
    let mut engine = ENGINE.get_or_init(|| Mutex::new(None)).lock()
        .map_err(|_| "OCR 引擎异常，请重启应用".to_string())?;
    if engine.is_none() {
        *engine = Some(receipt_ocr::Engine::new(&receipt_ocr::runtime_dir()?)?);
    }
    let result = engine.as_mut().unwrap().recognize(&request)?;
    serde_json::to_string(&result).map_err(|e| e.to_string())
}