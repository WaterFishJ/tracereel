//! 导出日志：把完整的 ffmpeg stderr（可能上百行）写进日志文件，界面只显示一句
//! 具体原因。
//!
//! 日志目录 = **输出视频所在目录**，文件名 `tracereel-<yyyymmdd>.log`。这样用户
//! 把视频发给别人排查时，日志就在旁边；同一目录同一天的多次导出都会追加进去。
//!
//! 写日志永远是"尽力而为"：目录不可写、路径非法等任何失败都只会退回 stderr，
//! 绝不会因为写日志本身让导出失败。

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// 日志文件锁：多个编码线程（队列 / 逐段导出）会并发追加，必须串行化，
/// 否则 Windows 上两个 File::create 交错会互相截断。
static LOG_LOCK: Mutex<()> = Mutex::new(());

/// 进程内记住本次运行要用的日志目录。导出开始时由 [`set_log_dir`] 用输出路径
/// 的父目录设置一次；设置前（例如启动阶段）退回 `LOCALAPPDATA\TraceReel`。
static LOG_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

/// 指定本次运行的日志目录（通常是输出视频所在目录）。
pub fn set_log_dir(dir: impl AsRef<Path>) {
    if let Ok(mut g) = LOG_DIR.lock() {
        *g = Some(dir.as_ref().to_path_buf());
    }
}

/// 用输出视频路径推出日志目录（取其父目录）。没有父目录时忽略。
pub fn set_log_dir_for_output(output_path: &str) {
    if let Some(parent) = Path::new(output_path).parent() {
        if !parent.as_os_str().is_empty() {
            set_log_dir(parent);
        }
    }
}

/// 当前生效的日志目录。
fn current_dir() -> PathBuf {
    if let Ok(g) = LOG_DIR.lock() {
        if let Some(d) = g.as_ref() {
            return d.clone();
        }
    }
    crate::data_paths::data_dir("LOCALAPPDATA")
}

/// `tracereel-20261007.log` —— 按本地日期分文件。
fn log_file_path() -> PathBuf {
    current_dir().join(format!("tracereel-{}.log", today_stamp()))
}

/// 本地日期 `yyyymmdd`。UNIX 秒 + 本地时区偏移（不引入 chrono 依赖）。
fn today_stamp() -> String {
    let (y, m, d) = local_ymd();
    format!("{y:04}{m:02}{d:02}")
}

/// 本地时间 `yyyy-mm-dd hh:mm:ss`。
fn now_stamp() -> String {
    let secs = local_secs();
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let tod = secs.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}",
        tod / 3600,
        (tod % 3600) / 60,
        tod % 60
    )
}

/// 当前本地时间距 UNIX 纪元的总秒数（= UTC 秒 + 本地时区偏移）。
fn local_secs() -> i64 {
    let utc = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    utc + local_offset_secs()
}

/// 本地年月日。
fn local_ymd() -> (i64, u32, u32) {
    civil_from_days(local_secs().div_euclid(86_400))
}

/// 本地时区相对 UTC 的偏移秒数（含夏令时）。
///
/// Windows 上用 `GetLocalTime` / `GetSystemTime` 对比得出；这两个函数在
/// `kernel32` 里，因此这里直接声明 FFI，避免为一个时间戳引入 winapi 依赖。
/// 取不到时返回 0（退化成 UTC），日志时间戳永远可用。
#[cfg(windows)]
fn local_offset_secs() -> i64 {
    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct SystemTimeW {
        year: u16,
        month: u16,
        day_of_week: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        milliseconds: u16,
    }
    extern "system" {
        fn GetLocalTime(t: *mut SystemTimeW);
        fn GetSystemTime(t: *mut SystemTimeW);
    }
    // 把 SYSTEMTIME 换算成"自 1970-01-01 起的天数"，再乘 86400 加当天秒数。
    fn to_secs(t: &SystemTimeW) -> i64 {
        days_from_civil(t.year as i64, t.month as u32, t.day as u32) * 86_400
            + t.hour as i64 * 3600
            + t.minute as i64 * 60
            + t.second as i64
    }
    unsafe {
        let mut l = SystemTimeW::default();
        let mut u = SystemTimeW::default();
        GetLocalTime(&mut l);
        GetSystemTime(&mut u);
        if l.year == 0 || u.year == 0 {
            return 0;
        }
        // 本地时间 - UTC 时间 = 时区偏移（两者表示同一瞬间）。
        to_secs(&l) - to_secs(&u)
    }
}

#[cfg(not(windows))]
fn local_offset_secs() -> i64 {
    0
}

/// 公历年月日 → 天数（1970-01-01 起）。Howard Hinnant 的 days_from_civil 算法。
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 } as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// 天数（1970-01-01 起）→ 公历年月日。Howard Hinnant 的 civil_from_days 算法。
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// 追加一条带时间戳的日志。写给文件 + 镜像到 stderr（保留原有命令行可见性）。
pub fn log_line(level: &str, msg: &str) {
    eprintln!("[encode][{level}] {msg}");
    let _guard = match LOG_LOCK.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    let path = log_file_path();
    if let Some(parent) = path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
    }
    let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) else {
        return;
    };
    let _ = writeln!(f, "{} [{level}] {msg}", now_stamp());
    let _ = f.flush();
}

/// 日志文件完整路径（供"打开日志"类提示使用；不保证已存在）。
pub fn log_file() -> PathBuf {
    log_file_path()
}

/// 记一条错误级日志。
#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => { $crate::encode_log::log_line("错误", &format!($($arg)*)) };
}

/// 记一条警告级日志。
#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => { $crate::encode_log::log_line("警告", &format!($($arg)*)) };
}

/// 记一条普通信息日志。
#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => { $crate::encode_log::log_line("信息", &format!($($arg)*)) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_from_days_matches_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1)); // 2024-01-01
        assert_eq!(civil_from_days(20_000), (2024, 10, 4));
    }

    #[test]
    fn days_from_civil_round_trips_with_civil_from_days() {
        // 两个方向必须互为逆运算，否则本地时间戳会算错（曾把日志写成 UTC）。
        for days in [-1_000i64, 0, 1, 19_723, 20_000, 20_642] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days, "round trip failed at {days}");
        }
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        // 2026-10-07 距 1970-01-01 = 20733 天（用系统 date 核对过）。
        assert_eq!(days_from_civil(2026, 10, 7), 20_733);
        assert_eq!(civil_from_days(20_733), (2026, 10, 7));
    }

    #[test]
    fn local_time_offset_is_plausible() {
        // 本地秒与 UTC 秒之差必须是合理的时区偏移（-14h..+14h），
        // 防止取错值时静默算出离谱的时间戳。
        let utc = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let off = local_secs() - utc;
        assert!(
            (-14 * 3600..=14 * 3600).contains(&off),
            "时区偏移不合理: {off} 秒"
        );
    }

    #[test]
    fn stamps_have_expected_shape() {
        let d = today_stamp();
        assert_eq!(d.len(), 8, "yyyymmdd，实际 {d}");
        assert!(d.chars().all(|c| c.is_ascii_digit()));
        let t = now_stamp();
        assert_eq!(t.len(), 19, "yyyy-mm-dd hh:mm:ss，实际 {t}");
    }

    #[test]
    fn log_line_appends_and_creates_dir() {
        let dir = std::env::temp_dir().join(format!(
            "ov_log_test_{}_{}",
            std::process::id(),
            today_stamp()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        set_log_dir(&dir);
        log_line("信息", "第一行");
        log_line("错误", "第二行");
        let file = log_file_path();
        assert!(file.exists(), "日志文件未创建: {}", file.display());
        let body = std::fs::read_to_string(&file).unwrap();
        assert!(body.contains("第一行"), "缺第一行: {body}");
        assert!(body.contains("第二行"), "缺第二行: {body}");
        assert!(body.contains("[错误]"), "缺级别标记: {body}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
