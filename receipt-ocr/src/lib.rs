//! Offline OCR in Rust. Only ONNX Runtime and the three Paddle models are deployed.
mod geometry;
mod grid;
mod inference;
mod time;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
pub struct Request {
    pub path: PathBuf,
    /// 原图上归一化的四角：左上、右上、右下、左下。
    pub corners: geometry::Quad,
    /// 整 90° 挡位：0 不转、1 顺时针 90°、2 180°、3 逆时针 90°。
    #[serde(default)]
    pub rotation: u8,
}
#[derive(Serialize, Default)]
pub struct Recognition {
    pub rows: Vec<Row>,
    pub columns: usize,
    pub preview: String,
}
#[derive(Serialize)]
pub struct Row {
    pub cells: Vec<String>,
    pub scores: Vec<f32>,
}
pub struct Engine {
    models: inference::Models,
}
impl Engine {
    pub fn new(root: &Path) -> Result<Self, String> {
        let dll = root.join(if cfg!(windows) {
            "onnxruntime.dll"
        } else {
            "libonnxruntime.so"
        });
        if !dll.is_file() {
            return Err(format!(
                "OCR 运行时缺失：{}，请运行 ocr/setup.ps1",
                dll.display()
            ));
        }
        ort::init_from(dll.to_string_lossy())
            .with_name("TraceReel OCR")
            .with_telemetry(false)
            .commit()
            .map_err(|e| e.to_string())?;
        let models = inference::Models::load(root)?;
        Ok(Self { models })
    }
    pub fn recognize(&mut self, request: &Request) -> Result<Recognition, String> {
        let reader =
            image::ImageReader::open(&request.path).map_err(|e| format!("无法读取图片：{e}"))?;
        let image = reader
            .with_guessed_format()
            .map_err(|e| e.to_string())?
            .decode()
            .map_err(|e| format!("无法解码图片：{e}"))?
            .into_rgb8();
        let corrected = geometry::rectify(&image, request.corners, request.rotation)?;
        let items = self.models.lines(&corrected)?;
        let mut result = grid::grid(&corrected, &items, &mut self.models)?;
        let preview = std::env::temp_dir().join("tracereel-ocr");
        std::fs::create_dir_all(&preview).map_err(|e| e.to_string())?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let target = preview.join(format!("rust-{}-{stamp}.png", std::process::id()));
        corrected.save(&target).map_err(|e| e.to_string())?;
        result.preview = url::Url::from_file_path(&target)
            .map_err(|_| "无法生成预览地址")?
            .to_string();
        // Remove only our Rust-generated previews, bounded to ten files.
        let mut old: Vec<_> = std::fs::read_dir(&preview)
            .map_err(|e| e.to_string())?
            .flatten()
            .filter(|e| {
                e.file_name().to_string_lossy().starts_with("rust-")
                    && e.path().extension().is_some_and(|e| e == "png")
            })
            .collect();
        old.sort_by_key(|e| e.metadata().and_then(|m| m.modified()).ok());
        let excess = old.len().saturating_sub(10);
        for entry in old.into_iter().take(excess) {
            let _ = std::fs::remove_file(entry.path());
        }
        Ok(result)
    }
}
pub fn runtime_dir() -> Result<PathBuf, String> {
    if let Some(root) = std::env::var_os("TRACEREEL_OCR_DIR") {
        return Ok(root.into());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let bundled = exe.parent().ok_or("无法定位程序目录")?.join("ocr");
    if bundled.join("onnxruntime.dll").is_file() {
        return Ok(bundled);
    }
    Ok(Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tools/ocr"))
}
