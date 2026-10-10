//! 镜头标定库（Gyroflow 的 lens profile database）+ 预设搜索。
//!
//! **为什么需要它**：我们原来只能读用户自己从 Gyroflow 导出的 `.gyroflow` —— 那个文件是
//! 「某个镜头标定 + 手改过的稳定化参数」的快照，换一台相机就不适用了。真正可复用的是
//! Gyroflow 自带的镜头标定库（`profiles.cbor.gz`，实测 **12409 条**，含各品牌机型/镜头/分辨率），
//! 所以这里把它接进来：搜索框同时搜「镜头标定」与「.gyroflow 预设」，选中镜头标定只换标定，
//! 稳定化参数仍由当前 `.gyroflow` 预设提供（见 `gyro::load_stabilizer`）。
//!
//! **库从哪来**（与原版 Gyroflow 同一套）：
//! * 原版：`src/core/build.rs` 编译时从 `github.com/gyroflow/lens_profiles` 的最新 release 下载
//!   `profiles.cbor.gz` 到 `resources/camera_presets/`（该目录在 `.gitignore` 里，不进源码库）；
//!   运行时 `LensProfileDatabase::get_path()` 在 cwd / exe 同级找 `resources/camera_presets`、
//!   `camera_presets`、`lens_profiles`，找不到就创建 `camera_presets`。
//! * 我们：候选位置里找现成的 `profiles.cbor.gz`（开发时的 `gyroflow-src/resources/camera_presets`、
//!   你自己装的 Gyroflow 的 `camera_presets`、`%LOCALAPPDATA%\Gyroflow\lens_profiles` 等），
//!   然后**复制**到 `get_path()` 认的目录（`<exe>/camera_presets/`），让它能被加载。
//!
//! **性能**：整库解析实测 1.36 s（8.9 MB CBOR）+ 建 UI 列表 0.20 s。所以整个进程只解析一次
//! （[`shared`] 进程级单例），GUI 启动时用 [`warmup`] 在后台预热，搜索时纯内存过滤（毫秒级）。
//! 稳定化器也共用同一个实例（`StabilizationManager::lens_profile_db` 是 pub 字段），
//! 避免每段视频重新解析一遍。

use gyroflow_core::lens_profile_database::LensProfileDatabase;
use parking_lot::RwLock;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

/// 镜头库文件名（Gyroflow 官方发布物就是这个）。
pub const DB_FILE: &str = "profiles.cbor.gz";

static READY: AtomicBool = AtomicBool::new(false);
static TOTAL: AtomicUsize = AtomicUsize::new(0);
static DB: OnceLock<Arc<RwLock<LensProfileDatabase>>> = OnceLock::new();

fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|p| p.to_path_buf()))
}

/// Gyroflow 自己的用户数据目录（`settings::data_dir()` 的等价物，用于找它下载/保存的库）。
fn gyroflow_data_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    for key in ["LOCALAPPDATA", "APPDATA"] {
        if let Some(base) = std::env::var_os(key) {
            v.push(PathBuf::from(&base).join("Gyroflow").join("lens_profiles"));
        }
    }
    if let Some(base) = std::env::var_os("APPDATA") {
        // 我们自己的预设目录里也可以放一份
        v.push(crate::data_paths::migrated_dir(&PathBuf::from(base), "TraceReel", "OrientView").join("presets").join("camera_presets"));
    }
    v
}

/// `profiles.cbor.gz` 的全部候选位置（含开发时的 `gyroflow-src/resources/camera_presets`）。
fn candidate_db_files() -> Vec<PathBuf> {
    let rels = [
        "camera_presets",
        "resources/camera_presets",
        "gyroflow-src/resources/camera_presets",
        "presets/camera_presets",
    ];
    let mut v = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        for r in rels {
            v.push(cwd.join(r).join(DB_FILE));
        }
    }
    if let Some(exe) = exe_dir() {
        for r in rels {
            v.push(exe.join(r).join(DB_FILE));
        }
    }
    for d in gyroflow_data_dirs() {
        v.push(d.join(DB_FILE));
    }
    v
}

/// 复刻 `LensProfileDatabase::get_path()` 的候选判定（Windows 版）：它只看**目录是否存在**，
/// 目录在但里面没库文件时会加载到 0 条 —— 所以我们要往它选中的那个目录里放文件。
fn visible_dir() -> PathBuf {
    let rels = ["./resources/camera_presets/", "./camera_presets/", "./lens_profiles/"];
    let cwd = std::env::current_dir().unwrap_or_default();
    for r in rels {
        let p = cwd.join(r);
        if p.is_dir() {
            return p;
        }
    }
    if let Some(exe) = exe_dir() {
        for r in rels {
            let p = exe.join(r);
            if p.is_dir() {
                return p;
            }
        }
        return exe.join("camera_presets");
    }
    cwd.join("camera_presets")
}

/// 让 `LensProfileDatabase::load_all()` 能看见镜头库：把找到的 `profiles.cbor.gz`
/// 复制进它认的目录（一次性，已存在就跳过）。
fn ensure_db_visible() -> Option<PathBuf> {
    let target_dir = visible_dir();
    let target = target_dir.join(DB_FILE);
    if target.exists() {
        return Some(target);
    }
    let src = candidate_db_files().into_iter().find(|p| p.exists())?;
    if let Some(parent) = target.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::copy(&src, &target) {
        Ok(n) => {
            log::info!(
                "[lens_db] copied {} -> {} ({} bytes)",
                src.display(),
                target.display(),
                n
            );
            Some(target)
        }
        Err(e) => {
            log::warn!("[lens_db] copy {} -> {} failed: {e}", src.display(), target.display());
            None
        }
    }
}

/// 进程级共享的镜头库（首次调用解析整库，约 1.4 s）。
pub fn shared() -> Arc<RwLock<LensProfileDatabase>> {
    DB.get_or_init(|| {
        let src = ensure_db_visible();
        let db = Arc::new(RwLock::new(LensProfileDatabase::default()));
        let t = std::time::Instant::now();
        db.write().load_all();
        db.write().prepare_list_for_ui();
        let n = db.read().get_all_filenames().len();
        TOTAL.store(n, Ordering::SeqCst);
        log::info!(
            "[lens_db] loaded {n} lens profiles in {:.2}s (db: {})",
            t.elapsed().as_secs_f64(),
            src.map(|p| p.display().to_string())
                .unwrap_or_else(|| "<not found>".to_string())
        );
        READY.store(true, Ordering::SeqCst);
        db
    })
    .clone()
}

/// 后台预热：GUI 启动时调用，避免第一次输入时卡 1.4 s。
pub fn warmup() {
    std::thread::spawn(|| {
        let _ = shared();
    });
}

/// 库是否已经加载完（供 UI 显示「镜头库加载中…」）。
pub fn is_ready() -> bool {
    READY.load(Ordering::SeqCst)
}

/// 库文件路径（诊断用）。
pub fn db_file() -> Option<PathBuf> {
    let p = visible_dir().join(DB_FILE);
    p.exists().then_some(p)
}

fn db_if_ready() -> Option<Arc<RwLock<LensProfileDatabase>>> {
    is_ready().then(shared)
}

/// 把一个镜头 id（库里的 key、或用户拖进来的 `.json` / `.gyroflow` 路径）解析成标定 JSON。
/// 供 `gyro::load_stabilizer` 应用（写临时 `.json` → `load_lens_profile`）。
pub fn resolve_lens_json(id: &str) -> Option<String> {
    let id = id.trim();
    if id.is_empty() {
        return None;
    }
    let p = Path::new(id);
    if p.is_file() {
        let text = std::fs::read_to_string(p).ok()?;
        if id.to_ascii_lowercase().ends_with(".gyroflow") {
            let v: Value = serde_json::from_str(&text).ok()?;
            return serde_json::to_string(v.get("calibration_data")?).ok();
        }
        return Some(text);
    }
    let db = shared();
    let db = db.read();
    let profile = db.get_by_id(id).or_else(|| db.find(id))?;
    profile.get_json().ok()
}

/// 镜头标定自带的、属于「相机属性」的字段 → 设置页属性名。
/// 选中一个镜头标定后由 QML 填进「稳定化」页（`frame_readout_time` 等），
/// 因为稳定化参数以设置页为唯一真源 —— 只把标定塞进 stabilizer 会被设置页覆盖。
pub fn lens_fields(id: &str) -> Value {
    let mut out = serde_json::Map::new();
    if let Some(json) = resolve_lens_json(id) {
        if let Ok(v) = serde_json::from_str::<Value>(&json) {
            if let Some(t) = v.get("frame_readout_time").and_then(|x| x.as_f64()) {
                out.insert("frame_readout_time".into(), json!(t));
            }
            if let Some(d) = v.get("frame_readout_direction").and_then(|x| x.as_str()) {
                out.insert("gyro_readout_dir".into(), json!(d));
            }
        }
    }
    Value::Object(out)
}

/// 全词匹配（与 Gyroflow 的 `search` 同规则：空格分隔的每个词都要出现）。
fn matches_all(text: &str, words: &[String]) -> bool {
    let t = text.to_lowercase();
    words.iter().all(|w| t.contains(w.as_str()))
}

/// 搜索：`.gyroflow`/`.json` 预设文件（我们自己的目录）+ 镜头标定库。
///
/// 返回 JSON：`{"ready":bool,"total":N,"db":"<路径>","items":[{kind,id,name,note}]}`，
/// `kind` = `preset`（整份 `.gyroflow`）或 `lens`（镜头标定）。
pub fn search(query: &str, limit: usize) -> Value {
    let q = query.trim().to_lowercase();
    let words: Vec<String> = q.split_whitespace().map(|s| s.to_string()).collect();
    let limit = limit.clamp(1, 200);

    let mut items: Vec<Value> = Vec::new();
    // 已收录的 id（小写 + 正斜杠）：用于跨来源去重。
    let mut seen_ids: HashSet<String> = HashSet::new();
    let norm = |s: &str| s.replace('\\', "/").to_lowercase();

    // 1) 预设文件（.gyroflow = 整份预设；.json = 单个镜头标定）—— 排前面（同 Gyroflow 的优先级）。
    if !words.is_empty() {
        for (name, note, path) in crate::presets::scan_files() {
            let is_preset = path
                .extension()
                .map(|e| e.eq_ignore_ascii_case("gyroflow"))
                .unwrap_or(false);
            let full = path.to_string_lossy().to_lowercase();
            if matches_all(&name, &words) || matches_all(&full, &words) {
                seen_ids.insert(norm(&path.to_string_lossy()));
                items.push(json!({
                    "kind": if is_preset { "preset" } else { "lens" },
                    "id": path.to_string_lossy(),
                    "name": name,
                    "note": if is_preset { "预设".to_string() } else { format!("{note} · 标定文件") },
                }));
            }
        }
    }

    // 2) 镜头标定库。
    let db = db_if_ready();
    if let (Some(db), false) = (&db, words.is_empty()) {
        let hits = db
            .read()
            .search(&q, &HashSet::<String>::new(), 0, 0);
        for (name, key, _crc, official, rating, _ar, author) in hits {
            if items.len() >= limit {
                break;
            }
            // 库里也会索引 camera_presets 目录下的 .gyroflow：与上面自己扫到的去重。
            if !seen_ids.insert(norm(&key)) {
                continue;
            }
            let note = if official {
                "官方".to_string()
            } else if !author.is_empty() {
                author
            } else if rating > 0.0 {
                format!("{rating:.1}★")
            } else {
                "社区".to_string()
            };
            items.push(json!({
                "kind": "lens",
                "id": key,
                "name": name,
                "note": note,
            }));
        }
    }

    items.truncate(limit);
    json!({
        "ready": is_ready(),
        "total": TOTAL.load(Ordering::SeqCst),
        "db": db_file().map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
        // 镜头库应该待的目录（没找到库时 UI 用它提示用户放文件）。
        "dir": visible_dir().to_string_lossy().to_string(),
        "items": items,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_dir_prefers_cwd_candidates() {
        // 只验证「总能给出一个目录」——真实候选取决于运行时 cwd。
        assert!(!visible_dir().to_string_lossy().is_empty());
    }

    #[test]
    fn matches_all_needs_every_word() {
        let words = vec!["hero11".to_string(), "black".to_string()];
        assert!(matches_all("GoPro Hero11 Black / Wide", &words));
        assert!(!matches_all("GoPro Hero11", &words));
    }
}
