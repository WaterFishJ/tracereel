//! Conservative receipt-time cleanup. A colon anchors the numeric fields;
//! only an extra boundary `1` is a possible misread separator, never a digit
//! inside an otherwise valid time. Keep ambiguous candidates until another
//! time column supplies independent evidence.
use crate::Recognition;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Candidate {
    text: String,
    millis: u64,
}

fn parse(text: &str) -> Option<u64> {
    let fields: Vec<_> = text.split(':').collect();
    if !(2..=3).contains(&fields.len()) {
        return None;
    }
    let (seconds, fraction) = fields
        .last()?
        .split_once(['.', ','])
        .unwrap_or((fields.last()?, ""));
    if seconds.is_empty() || seconds.len() > 2 || !seconds.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // A present decimal separator must have 1..=3 fractional digits.
    let has_fraction = fields.last()?.contains(['.', ',']);
    if has_fraction
        && (fraction.is_empty()
            || fraction.len() > 3
            || !fraction.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    let mut total = 0_u64;
    for (i, field) in fields[..fields.len() - 1].iter().enumerate() {
        if field.is_empty() || !field.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        // The first field is unbounded: mm:ss may exceed 99 minutes, and
        // long-duration hh:mm:ss must not be silently truncated either.
        if i > 0 && field.len() > 2 {
            return None;
        }
        let n: u64 = field.parse().ok()?;
        if i > 0 && n >= 60 {
            return None;
        }
        total = total.checked_mul(60)?.checked_add(n)?;
    }
    let sec: u64 = seconds.parse().ok()?;
    if sec >= 60 {
        return None;
    }
    let decimal = if fraction.is_empty() {
        0
    } else {
        fraction.parse::<u64>().ok()? * 10_u64.pow(3 - fraction.len() as u32)
    };
    total
        .checked_mul(60)?
        .checked_add(sec)?
        .checked_mul(1000)?
        .checked_add(decimal)
}

fn candidates(text: &str) -> Vec<Candidate> {
    if let Some(millis) = parse(text) {
        return vec![Candidate {
            text: text.into(),
            millis,
        }];
    }
    let Some((head, seconds)) = text.rsplit_once(':') else {
        return Vec::new();
    };
    // Do not invent a missing decimal point or trim fractional digits. Only
    // three integer digits at a time-field boundary can contain an extra 1.
    if seconds.len() != 3 || !seconds.bytes().all(|b| b.is_ascii_digit()) {
        return Vec::new();
    }
    let mut result = Vec::new();
    for trimmed in [seconds.strip_prefix('1'), seconds.strip_suffix('1')]
        .into_iter()
        .flatten()
    {
        let value = format!("{head}:{trimmed}");
        if let Some(millis) = parse(&value) {
            let candidate = Candidate {
                text: value,
                millis,
            };
            if !result.contains(&candidate) {
                result.push(candidate);
            }
        }
    }
    result
}

fn agrees(previous: &Candidate, current: &Candidate, split: &Candidate) -> bool {
    current.millis.checked_sub(previous.millis) == Some(split.millis)
}

pub fn repair(result: &mut Recognition) {
    let mut choices: Vec<Vec<Vec<Candidate>>> = result
        .rows
        .iter()
        .map(|row| row.cells.iter().map(|text| candidates(text)).collect())
        .collect();
    // Infer cumulative/split column roles only from repeated, independently
    // unambiguous equations, not from column order or a single coincidence.
    let mut pairs = Vec::new();
    for cumulative in 0..result.columns {
        for split in 0..result.columns {
            if cumulative == split {
                continue;
            }
            let mut checked = 0;
            let mut matched = 0;
            for rows in choices.windows(2) {
                let (Some(prev), Some(curr), Some(duration)) = (
                    rows[0].get(cumulative),
                    rows[1].get(cumulative),
                    rows[1].get(split),
                ) else {
                    continue;
                };
                if prev.len() == 1 && curr.len() == 1 && duration.len() == 1 {
                    checked += 1;
                    matched += usize::from(agrees(&prev[0], &curr[0], &duration[0]));
                }
            }
            if matched >= 2 && matched * 4 >= checked * 3 {
                pairs.push((cumulative, split));
            }
        }
    }
    // Propagate uniquely supported choices in both directions. Ambiguous
    // combinations and conflicting column-role hypotheses remain untouched.
    loop {
        let mut proposals = vec![vec![Vec::<String>::new(); result.columns]; choices.len()];
        for &(cumulative, split) in &pairs {
            for i in 1..choices.len() {
                let (Some(previous), Some(current), Some(duration)) = (
                    choices[i - 1].get(cumulative),
                    choices[i].get(cumulative),
                    choices[i].get(split),
                ) else {
                    continue;
                };
                let matches: Vec<_> = previous
                    .iter()
                    .flat_map(|p| {
                        current.iter().flat_map(move |c| {
                            duration
                                .iter()
                                .filter_map(move |s| agrees(p, c, s).then_some((p, c, s)))
                        })
                    })
                    .collect();
                for (row, col, field) in [(i - 1, cumulative, 0), (i, cumulative, 1), (i, split, 2)]
                {
                    if choices[row][col].len() <= 1 {
                        continue;
                    }
                    let mut supported = Vec::new();
                    for &(p, c, s) in &matches {
                        let value = [p, c, s][field].text.clone();
                        if !supported.contains(&value) {
                            supported.push(value);
                        }
                    }
                    // An equation that still permits both candidates is not
                    // evidence against a unique choice from another equation.
                    if supported.len() == 1 && !proposals[row][col].contains(&supported[0]) {
                        proposals[row][col].push(supported[0].clone());
                    }
                }
            }
        }
        let mut changed = false;
        for (row, cells) in proposals.iter().enumerate() {
            for (col, texts) in cells.iter().enumerate() {
                if texts.len() == 1 {
                    choices[row][col].retain(|c| c.text == texts[0]);
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    for (row, cells) in result.rows.iter_mut().zip(choices) {
        for (col, options) in cells.into_iter().enumerate() {
            let corrected = options.len() == 1 && options[0].text != row.cells[col];
            if corrected {
                row.cells[col] = options[0].text.clone();
            }
            if corrected || options.len() > 1 {
                // Reuse the existing orange review indication in the GUI.
                if let Some(score) = row.scores.get_mut(col) {
                    *score = score.min(0.84);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Row;

    fn receipt(rows: &[&[&str]]) -> Recognition {
        Recognition {
            columns: rows.first().map_or(0, |row| row.len()),
            rows: rows
                .iter()
                .map(|cells| Row {
                    cells: cells.iter().map(|s| s.to_string()).collect(),
                    scores: vec![0.99; cells.len()],
                })
                .collect(),
            preview: String::new(),
        }
    }

    #[test]
    fn repairs_only_unique_extra_boundary_ones() {
        for (input, expected) in [
            ("00:00:001", "00:00:00"),
            ("00:03:021", "00:03:02"),
            ("00:09:221", "00:09:22"),
            ("00:57:171", "00:57:17"),
            ("01:05:361", "01:05:36"),
            ("3:102", "3:02"),
            ("00:03:111", "00:03:11"),
        ] {
            let options = candidates(input);
            assert_eq!(options.len(), 1, "{input}");
            assert_eq!(options[0].text, expected);
        }
    }

    #[test]
    fn preserves_valid_times_fractions_codes_and_invalid_fields() {
        for input in [
            "00:12:01",
            "00:03:02.1",
            "00:03:02,123",
            "125:30",
            "100:03:02",
            "00:03:02.111",
            "1:1",
            "00:03:02.000",
        ] {
            assert_eq!(candidates(input)[0].text, input);
        }
        for input in [
            "0031",
            "1",
            "00:03:022",
            "00:03:121.1",
            "00:03:0211",
            "00:60:021",
            "00:03:601",
            "00:03:02.",
            "00:03:02.1234",
            "00:003:02",
        ] {
            assert!(candidates(input).is_empty(), "{input}");
        }
        let mut result = receipt(&[&["0031", "00:03:121"]]);
        repair(&mut result);
        assert_eq!(result.rows[0].cells, ["0031", "00:03:121"]);
        assert!(result.rows[0].scores[1] < 0.85);
    }

    #[test]
    fn disambiguates_with_independently_established_time_columns() {
        let mut result = receipt(&[
            &["001", "00:00:00", "00:00:00"],
            &["002", "00:01:00", "00:01:00"],
            &["003", "00:02:00", "00:01:00"],
            &["004", "00:03:121", "00:01:21"],
            &["005", "00:04:121", "00:01:00"],
        ]);
        repair(&mut result);
        assert_eq!(result.rows[3].cells[1], "00:03:21");
        assert_eq!(result.rows[4].cells[1], "00:04:21");
        let once = result
            .rows
            .iter()
            .map(|r| r.cells.clone())
            .collect::<Vec<_>>();
        repair(&mut result);
        assert_eq!(
            once,
            result
                .rows
                .iter()
                .map(|r| r.cells.clone())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn single_coincidence_and_conflicting_evidence_do_not_guess() {
        let mut single = receipt(&[
            &["00:00:00", "00:00:00"],
            &["00:01:00", "00:01:00"],
            &["00:02:121", "00:01:21"],
        ]);
        repair(&mut single);
        assert_eq!(single.rows[2].cells[0], "00:02:121");
        let mut conflicting = receipt(&[
            &["00:00:00", "00:00:00", "00:00:00"],
            &["00:01:00", "00:01:00", "00:01:00"],
            &["00:02:00", "00:01:00", "00:01:00"],
            &["00:03:121", "00:01:21", "00:01:12"],
        ]);
        repair(&mut conflicting);
        assert_eq!(conflicting.rows[3].cells[0], "00:03:121");
    }
}
