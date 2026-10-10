//! Split-time parsing (ported from SplitsManager.cpp / splits_manager.py).
//!
//! Split times are a string like `1:23|2:55|1:13:22` where each time is
//! `hours:minutes:seconds` (hours/minutes optional) and separators are `|` or `;`.
//! Times may be absolute or relative.

/// Error produced while parsing split times.
#[derive(Debug, Clone, PartialEq)]
pub struct SplitsError(pub String);

impl std::fmt::Display for SplitsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for SplitsError {}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Split {
    pub position: usize,
    /// Seconds from the start of the route.
    pub absolute_time: f64,
}

#[derive(Debug, Clone, Default)]
pub struct RunnerInfo {
    pub final_position: usize,
    pub name: String,
    pub splits: Vec<Split>,
}

/// Parse `h:mm:ss`, `mm:ss` or `s(.ss)` (also `.`, `-` or `:` separators).
pub fn parse_time_string(text: &str) -> Result<f64, SplitsError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(SplitsError("Invalid time (empty)".to_string()));
    }
    // A dot in the seconds field is fractional when colons are present.
    // Keep historical dot/dash-only notation ("1.30" = 1 minute 30 seconds).
    let parts: Vec<&str> = if text.contains(':') {
        text.split(':').collect()
    } else {
        text.split(|c| c == '.' || c == '-').collect()
    };
    if parts.iter().any(|p| p.is_empty()) {
        return Err(SplitsError(format!("Invalid time: {text}")));
    }
    let nums: Vec<f64> = parts
        .iter()
        .map(|p| p.parse().unwrap_or(f64::NAN))
        .collect();
    if nums.iter().any(|n| !n.is_finite()) {
        return Err(SplitsError(format!("Invalid time: {text}")));
    }
    let seconds = match nums.len() {
        1 => nums[0],
        2 => nums[0] * 60.0 + nums[1],
        3 => nums[0] * 3600.0 + nums[1] * 60.0 + nums[2],
        _ => return Err(SplitsError(format!("Invalid time: {text}"))),
    };
    Ok(seconds)
}

/// Build a default `RunnerInfo` from a split-time string.
///
/// * An implicit first split is always added at t=0.
/// * `is_relative`: when true each listed time accumulates onto the previous.
/// * `final_position` = number of splits.
pub fn build_runner_info(split_times: &str, is_relative: bool) -> RunnerInfo {
    let mut runner = RunnerInfo::default();
    if split_times.trim().is_empty() {
        return runner;
    }

    let raw_times: Vec<&str> = split_times.split(|c| c == '|' || c == ';').collect();
    let mut times = Vec::new();
    for raw in raw_times {
        let raw = raw.trim();
        if !raw.is_empty() {
            if let Ok(t) = parse_time_string(raw) {
                times.push(t);
            }
        }
    }

    runner.splits.push(Split {
        position: 0,
        absolute_time: 0.0,
    });
    if is_relative {
        let mut absolute = 0.0;
        for (i, t) in times.into_iter().enumerate() {
            absolute += t;
            runner.splits.push(Split {
                position: i + 1,
                absolute_time: absolute,
            });
        }
    } else {
        for (i, t) in times.into_iter().enumerate() {
            runner.splits.push(Split {
                position: i + 1,
                absolute_time: t,
            });
        }
    }
    runner.final_position = runner.splits.len();
    runner
}

/// The start time (seconds from the route start) of every segment, given a
/// split-time string.
///
/// A split list of `n` entries describes `n` legs: leg *k* runs from split
/// *k-1* to split *k*, with split 0 pinned at t=0. So the returned list has `n`
/// entries — the first is always `0.0` — and the *finish* time (the last split)
/// is not one of them, because no leg starts there.
///
/// This is what 「按分段设置换图时间」 offers: switching the map at the start of
/// leg *k*.
pub fn segment_start_times(split_times: &str, is_relative: bool) -> Vec<f64> {
    let info = build_runner_info(split_times, is_relative);
    if info.splits.len() < 2 {
        return Vec::new();
    }
    info.splits[..info.splits.len() - 1]
        .iter()
        .map(|s| s.absolute_time)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_starts_are_the_previous_split() {
        // 3 个分段点 ⇒ 3 条腿：起点 0 / 83 / 175（4402 是终点，不是任何一段的起点）。
        let starts = segment_start_times("1:23|2:55|1:13:22", false);
        assert_eq!(starts, vec![0.0, 83.0, 175.0]);
        // 相对时间按累加。
        let starts = segment_start_times("1:23|1:00", true);
        assert_eq!(starts, vec![0.0, 83.0]);
        // 1 个分段点 = 1 条腿，起点就是 0。
        assert_eq!(segment_start_times("12", false), vec![0.0]);
        // 没有分段就没有可选起点。
        assert!(segment_start_times("", false).is_empty());
    }

    #[test]
    fn parses_time_strings() {
        assert!((parse_time_string("1:23").unwrap() - 83.0).abs() < 1e-9);
        assert!((parse_time_string("2:55").unwrap() - 175.0).abs() < 1e-9);
        assert!((parse_time_string("1:13:22").unwrap() - 4402.0).abs() < 1e-9);
        assert!((parse_time_string("90").unwrap() - 90.0).abs() < 1e-9);
        assert!((parse_time_string("1.30").unwrap() - 90.0).abs() < 1e-9);
        assert!((parse_time_string("22:03.1").unwrap() - 1323.1).abs() < 1e-9);
        assert!((parse_time_string("00:22:03.1").unwrap() - 1323.1).abs() < 1e-9);
    }

    #[test]
    fn builds_absolute_splits() {
        let r = build_runner_info("1:23|2:55|1:13:22", false);
        assert_eq!(r.splits.len(), 4);
        assert_eq!(r.splits[0].absolute_time, 0.0);
        assert_eq!(r.splits[1].absolute_time, 83.0);
        assert_eq!(r.splits[2].absolute_time, 175.0);
        assert_eq!(r.splits[3].absolute_time, 4402.0);
        assert_eq!(r.final_position, 4);
    }

    #[test]
    fn builds_relative_splits() {
        let r = build_runner_info("1:23|1:00", true);
        assert_eq!(r.splits.len(), 3);
        assert_eq!(r.splits[1].absolute_time, 83.0);
        assert_eq!(r.splits[2].absolute_time, 83.0 + 60.0);
    }

    #[test]
    fn empty_string_no_splits() {
        let r = build_runner_info("", false);
        assert_eq!(r.splits.len(), 0);
        assert_eq!(r.final_position, 0);
    }
}
