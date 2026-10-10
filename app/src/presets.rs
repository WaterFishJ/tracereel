//! 视频防抖预设（`.gyroflow`）列表 —— 对应原版 Gyroflow 的「镜头预设」下拉列表。
//!
//! 原版做法（`gyroflow-src/src/core/lens_profile_database.rs`）：
//! * `load_all()` 启动时 walk 若干固定目录（程序自带 `resources/camera_presets`、
//!   运行目录 `./camera_presets` / `./lens_profiles`、用户数据目录 `lens_profiles`），
//!   把目录里的 `*.json`（镜头标定）和 `*.gyroflow`（整份预设）一起编入数据库；
//!   `.gyroflow` 条目只记「file_stem 作名字 + 完整路径 + 路径 crc32」，不解析内容
//!   （`DataSource::String` 存原文，选中时才用 `get_preset_by_id` 取出来）。
//! * `prepare_list_for_ui()` 生成 `(名称, 路径, crc, 官方?, 评分, 画幅比, 作者)`
//!   元组并按名称排序；`search()` 在该列表上按关键词过滤，Qt 侧
//!   `search_lens_profile_finished(QVariantList)` 把结果推给 QML 的搜索框下拉。
//! * 选中一个 `.gyroflow` 条目 → `get_preset_contents(path)` → `loadGyroflowData()`
//!   整份应用；另有「Open file」按钮可以选目录外的任意文件。
//!
//! 本模块照搬同一套扫描逻辑：扫描固定目录（exe 同级 `presets/`、运行目录 `presets/`、
//! `%APPDATA%\TraceReel\presets`），递归收集 `*.gyroflow`，按规范路径去重、按名字排序，
//! 并把「当前正在用的预设」也一并列出（它可能来自「浏览…」，不在任何目录里）。
//! 结果以 JSON 交给 QML 渲染成下拉列表。

use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// 递归扫描的最大深度（防止误把预设目录指到盘根时把整盘走一遍）。
const MAX_DEPTH: usize = 4;

/// 被扫描的目录之一：`(类别标签, 目录, 是否存在)`。
fn candidate_dirs() -> Vec<(&'static str, PathBuf)> {
    let mut dirs: Vec<(&'static str, PathBuf)> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    if let Some(exe) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|p| p.to_path_buf()))
    {
        let d = exe.join("presets");
        seen.insert(norm_key(&d));
        dirs.push(("程序目录", d));
    }
    if let Ok(cwd) = std::env::current_dir() {
        let d = cwd.join("presets");
        if seen.insert(norm_key(&d)) {
            dirs.push(("工作目录", d));
        }
    }
    if let Some(d) = user_preset_dir() {
        if seen.insert(norm_key(&d)) {
            dirs.push(("用户目录", d));
        }
    }
    dirs
}

/// 用户自己的预设目录：Windows 用 `%APPDATA%\TraceReel\presets`，其它平台
/// 退回 `~/.config/tracereel/presets`（对应 Gyroflow 的 `data_dir()/lens_profiles`）。
fn user_preset_dir() -> Option<PathBuf> {
    if let Some(appdata) = std::env::var_os("APPDATA") {
        return Some(crate::data_paths::migrated_dir(&PathBuf::from(appdata), "TraceReel", "OrientView").join("presets"));
    }
    std::env::var_os("HOME").map(|h| crate::data_paths::migrated_dir(&PathBuf::from(h).join(".config"), "tracereel", "orientview").join("presets"))
}

/// 路径比较用的规范键：优先规范化（消掉 `..`/大小写/短名差异），失败就退回小写原文。
fn norm_key(p: &Path) -> String {
    std::fs::canonicalize(p)
        .unwrap_or_else(|_| p.to_path_buf())
        .to_string_lossy()
        .to_lowercase()
}

/// 列表里显示的名字：目录内相对路径去掉 `.gyroflow` 后缀（子目录用 `/` 分隔）。
fn display_name(path: &Path, root: &Path) -> String {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let parent = path
        .parent()
        .and_then(|p| p.strip_prefix(root).ok())
        .map(|r| r.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();
    if parent.is_empty() {
        stem
    } else {
        format!("{parent}/{stem}")
    }
}

/// 收集目录下的 `*.gyroflow`（含子目录，深度上限 [`MAX_DEPTH`]）。
/// `out` 里是 `(显示名, 完整路径)`。
fn collect_in(dir: &Path, root: &Path, depth: usize, out: &mut Vec<(String, PathBuf)>) {
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_in(&path, root, depth + 1, out);
            continue;
        }
        let is_preset = path
            .extension()
            .map(|e| e.eq_ignore_ascii_case("gyroflow"))
            .unwrap_or(false);
        if !is_preset {
            continue;
        }
        out.push((display_name(&path, root), path));
    }
}

/// 扫描全部候选目录里的 `*.gyroflow`：去重、按名字排序，返回 `(显示名, 类别标签, 路径)`。
/// [`list`] 与镜头库搜索（`crate::lens_db::search`）共用。
pub fn scan_files() -> Vec<(String, String, PathBuf)> {
    let dirs = candidate_dirs();
    let mut found: Vec<(String, String, PathBuf)> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for (note, dir) in &dirs {
        let mut hits = Vec::new();
        collect_in(dir, dir, 0, &mut hits);
        for (name, path) in hits {
            if seen.insert(norm_key(&path)) {
                found.push((name, (*note).to_string(), path));
            }
        }
    }
    found.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    found
}

/// 扫描全部候选目录，返回 QML 用的 JSON：
/// ```json
/// {
///   "dirs":    [{"note":"工作目录","path":"D:/.../presets","exists":true}, ...],
///   "presets": [{"name":"我的预设","path":"D:/.../presets/我的预设.gyroflow",
///                "note":"工作目录","current":true}, ...]
/// }
/// ```
/// `current` 是设置里当前的预设路径：它若不在任何目录中（例如用「浏览…」选的），
/// 也会带 `"note":"当前"` 排在列表最前面，保证下拉里能看到、不会被丢掉。
pub fn list(current: &str) -> Value {
    let dirs = candidate_dirs();
    let found = scan_files();

    let cur = current.trim();
    let cur_key = if cur.is_empty() {
        None
    } else {
        Some(norm_key(Path::new(cur)))
    };

    let mut presets: Vec<Value> = Vec::new();
    // 当前预设不在任何目录里：单独列在最前面。
    if let Some(key) = &cur_key {
        let in_list = found.iter().any(|(_, _, p)| &norm_key(p) == key);
        if !in_list && Path::new(cur).exists() {
            let cur_path = Path::new(cur);
            let root = cur_path.parent().unwrap_or(Path::new(""));
            presets.push(json!({
                "name": display_name(cur_path, root),
                "path": cur,
                "note": "当前",
                "current": true,
            }));
        }
    }
    for (name, note, path) in &found {
        let is_current = cur_key
            .as_ref()
            .map(|k| &norm_key(path) == k)
            .unwrap_or(false);
        presets.push(json!({
            "name": name,
            "path": path.to_string_lossy(),
            "note": note,
            "current": is_current,
        }));
    }

    json!({
        "dirs": dirs
            .iter()
            .map(|(note, p)| json!({
                "note": note,
                "path": p.to_string_lossy(),
                "exists": p.is_dir(),
            }))
            .collect::<Vec<_>>(),
        "presets": presets,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_presets_recursively_and_strips_extension() {
        let base = std::env::temp_dir().join(format!("ov_presets_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("sub")).unwrap();
        std::fs::write(base.join("a.gyroflow"), "{}").unwrap();
        std::fs::write(base.join("sub").join("b.gyroflow"), "{}").unwrap();
        std::fs::write(base.join("note.txt"), "x").unwrap();

        let mut out = Vec::new();
        collect_in(&base, &base, 0, &mut out);
        out.sort();
        let names: Vec<String> = out.iter().map(|(n, _)| n.clone()).collect();
        assert_eq!(names, vec!["a".to_string(), "sub/b".to_string()]);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn list_reports_dirs_and_keeps_out_of_tree_current() {
        let base = std::env::temp_dir().join(format!("ov_presets_cur_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let alone = base.join("别处.gyroflow");
        std::fs::write(&alone, "{}").unwrap();

        let v = list(&alone.to_string_lossy());
        assert!(v.get("dirs").and_then(|d| d.as_array()).is_some());
        let presets = v.get("presets").unwrap().as_array().unwrap();
        // 目录外的当前预设必须出现在列表里（note = 当前）。
        assert!(presets
            .iter()
            .any(|p| p["note"] == "当前" && p["name"] == "别处"));

        let _ = std::fs::remove_dir_all(&base);
    }
}
