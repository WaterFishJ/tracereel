use crate::{
    geometry::{self, Quad},
    inference::{center_y, Models, TextBox},
    Recognition, Row,
};
use image::{Rgb, RgbImage};

pub fn grid(
    image: &RgbImage,
    items: &[TextBox],
    models: &mut Models,
) -> Result<Recognition, String> {
    if items.is_empty() {
        return Ok(Recognition::default());
    }
    if let Some(boundaries) = gutters(image, items) {
        let mut bands: Vec<Vec<&TextBox>> = Vec::new();
        for item in items {
            let (top, bottom) = yrange(&item.quad);
            if bands.last().is_none_or(|band| {
                (center_y(&item.quad) - center_y(&band[0].quad)).abs() > (bottom - top) * 0.55
            }) {
                bands.push(Vec::new());
            }
            bands.last_mut().unwrap().push(item);
        }
        let mut crops = Vec::new();
        for band in &bands {
            for pair in boundaries.windows(2) {
                let a = pair[0];
                let b = pair[1];
                let (overlap, best) = band
                    .iter()
                    .map(|item| {
                        let (left, right) = xrange(&item.quad);
                        ((b.min(right) - a.max(left)).max(0.), *item)
                    })
                    .max_by(|a, b| a.0.total_cmp(&b.0))
                    .unwrap();
                let crop = if overlap < 5. {
                    RgbImage::from_pixel(32, 32, Rgb([255; 3]))
                } else {
                    let q = best.quad;
                    let (left, right) = xrange(&q);
                    let x1 = a.max(left);
                    let x2 = b.min(right);
                    let edge = |p: [f32; 2], r: [f32; 2], x: f32| {
                        p[1] + (r[1] - p[1]) * (x - p[0]) / (r[0] - p[0]).max(1.)
                    };
                    let quad = [
                        [x1, edge(q[0], q[1], x1) - 2.],
                        [x2, edge(q[0], q[1], x2) - 2.],
                        [x2, edge(q[3], q[2], x2) + 2.],
                        [x1, edge(q[3], q[2], x1) + 2.],
                    ];
                    let w = (x2 - x1).round().max(8.) as u32;
                    let h = (quad[3][1] - quad[0][1])
                        .max(quad[2][1] - quad[1][1])
                        .round()
                        .max(8.) as u32;
                    geometry::warp(image, quad, w, h)?
                };
                crops.push(geometry::padded(&crop));
            }
        }
        let texts = models.recognize(&crops)?;
        let columns = boundaries.len() - 1;
        let rows = texts
            .chunks(columns)
            .map(|cells| Row {
                cells: cells
                    .iter()
                    .map(|v| normalize_text(&v.0).trim().to_owned())
                    .collect(),
                scores: cells.iter().map(|v| v.1).collect(),
            })
            .collect();
        return Ok(finish(Recognition {
            columns,
            rows,
            preview: String::new(),
        }));
    }
    Ok(finish(fallback(items)))
}
fn finish(mut result: Recognition) -> Recognition {
    crate::time::repair(&mut result);
    result
}
fn xrange(q: &Quad) -> (f32, f32) {
    (
        q.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min),
        q.iter().map(|p| p[0]).fold(0., f32::max),
    )
}
fn yrange(q: &Quad) -> (f32, f32) {
    (
        q.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min),
        q.iter().map(|p| p[1]).fold(0., f32::max),
    )
}
fn gutters(image: &RgbImage, items: &[TextBox]) -> Option<Vec<f32>> {
    let gray = image::DynamicImage::ImageRgb8(image.clone()).into_luma8();
    let threshold = imageproc::contrast::otsu_level(&gray);
    let y1 = (image.height() as f32 * 0.13).round() as u32;
    let y2 = (image.height() as f32 * 0.88).round() as u32;
    let blank: Vec<bool> = (0..image.width())
        .map(|x| {
            let count = (y1..y2)
                .filter(|y| gray.get_pixel(x, *y)[0] <= threshold)
                .count();
            count as f32 / ((y2 - y1) as f32) < 0.015
        })
        .collect();
    let mut heights: Vec<f32> = items
        .iter()
        .map(|item| {
            let (a, b) = yrange(&item.quad);
            b - a
        })
        .collect();
    heights.sort_by(f32::total_cmp);
    let median = if heights.len().is_multiple_of(2) {
        (heights[heights.len() / 2 - 1] + heights[heights.len() / 2]) / 2.
    } else {
        heights[heights.len() / 2]
    };
    let gap = (median * 0.3).max(10.);
    let left = items
        .iter()
        .map(|i| xrange(&i.quad).0)
        .fold(f32::INFINITY, f32::min);
    let right = items.iter().map(|i| xrange(&i.quad).1).fold(0., f32::max);
    let mut cuts = vec![0.];
    let mut x = 0;
    while x < blank.len() {
        if !blank[x] {
            x += 1;
            continue;
        }
        let start = x;
        while x < blank.len() && blank[x] {
            x += 1;
        }
        let end = x - 1;
        if (x - start) as f32 >= gap && start as f32 > left + gap && (end as f32) < right - gap {
            cuts.push(((start + end) / 2) as f32);
        }
    }
    if !(2..=8).contains(&cuts.len()) {
        return None;
    }
    cuts.push(image.width() as f32);
    Some(cuts)
}
#[derive(Clone)]
struct Token {
    text: String,
    x: f32,
    y: f32,
    h: f32,
    score: f32,
}
fn normalize_text(text: &str) -> String {
    static TIME: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = TIME.get_or_init(|| regex::Regex::new(r"(\d)\s*([:.,])\s*(\d)").unwrap());
    // Actual pipes are column separators, not part of the time. Keep a
    // space in their place so fallback token coordinates remain aligned.
    let mut text = text.replace('：', ":").replace(['|', '｜'], " ");
    loop {
        let next = re.replace_all(&text, "$1$2$3").into_owned();
        if next == text {
            return text;
        }
        text = next;
    }
}
fn fallback(items: &[TextBox]) -> Recognition {
    let mut tokens = Vec::new();
    for item in items {
        let (left, right) = xrange(&item.quad);
        let (top, bottom) = yrange(&item.quad);
        let text = normalize_text(&item.text);
        let chars: Vec<char> = text.chars().collect();
        let mut start = 0;
        while start < chars.len() {
            if chars[start].is_whitespace() {
                start += 1;
                continue;
            }
            let mut end = start + 1;
            while end < chars.len() && !chars[end].is_whitespace() {
                end += 1;
            }
            tokens.push(Token {
                text: chars[start..end].iter().collect(),
                x: left + (right - left) * start as f32 / chars.len() as f32,
                y: (top + bottom) / 2.,
                h: bottom - top,
                score: item.score,
            });
            start = end;
        }
    }
    if tokens.is_empty() {
        return Recognition::default();
    }
    tokens.sort_by(|a, b| a.y.total_cmp(&b.y));
    let mut rows: Vec<Vec<Token>> = Vec::new();
    for token in tokens {
        if rows
            .last()
            .is_none_or(|r| (r[0].y - token.y).abs() > (token.h * 0.55).max(5.))
        {
            rows.push(Vec::new());
        }
        rows.last_mut().unwrap().push(token);
    }
    let max = rows.iter().map(Vec::len).max().unwrap();
    let mut anchors: Vec<(f32, usize)> = Vec::new();
    for row in &rows {
        if row.len() >= 2.max(max.saturating_sub(1)) {
            for t in row {
                let nearest = anchors
                    .iter()
                    .enumerate()
                    .min_by(|a, b| (a.1 .0 - t.x).abs().total_cmp(&(b.1 .0 - t.x).abs()))
                    .map(|(i, _)| i);
                if let Some(i) =
                    nearest.filter(|i| (anchors[*i].0 - t.x).abs() < 12_f32.max(t.h * 1.1))
                {
                    let (x, n) = anchors[i];
                    anchors[i] = ((x * n as f32 + t.x) / (n + 1) as f32, n + 1);
                } else {
                    anchors.push((t.x, 1));
                }
            }
        }
    }
    if anchors.is_empty() {
        anchors = rows
            .iter()
            .max_by_key(|r| r.len())
            .unwrap()
            .iter()
            .map(|t| (t.x, 1))
            .collect();
    }
    anchors.sort_by(|a, b| a.0.total_cmp(&b.0));
    let columns = anchors.len();
    let rows = rows
        .into_iter()
        .map(|mut row| {
            row.sort_by(|a, b| a.x.total_cmp(&b.x));
            let mut cells = vec![String::new(); columns];
            let mut scores = vec![1_f32; columns];
            for t in row {
                let col = anchors
                    .iter()
                    .enumerate()
                    .min_by(|a, b| (a.1 .0 - t.x).abs().total_cmp(&(b.1 .0 - t.x).abs()))
                    .unwrap()
                    .0;
                if !cells[col].is_empty() {
                    cells[col].push(' ');
                }
                cells[col].push_str(&t.text);
                scores[col] = scores[col].min(t.score);
            }
            Row { cells, scores }
        })
        .collect();
    Recognition {
        columns,
        rows,
        preview: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn time_spacing_does_not_create_extra_columns() {
        assert_eq!(
            normalize_text("12 120 00 : 17 : 21 . 1"),
            "12 120 00:17:21.1"
        );
        assert_eq!(normalize_text("00：00：07,1"), "00:00:07,1");
    }
    #[test]
    fn time_cleanup_runs_after_either_column_path() {
        let result = finish(Recognition {
            columns: 4,
            rows: vec![Row {
                cells: ["002", "0031", "00:03:021", "00:03:02.1"]
                    .map(str::to_owned)
                    .to_vec(),
                scores: vec![0.99; 4],
            }],
            preview: String::new(),
        });
        assert_eq!(
            result.rows[0].cells,
            ["002", "0031", "00:03:02", "00:03:02.1"]
        );
        let item = |x, text: &str| TextBox {
            quad: [[x, 0.], [x + 50., 0.], [x + 50., 10.], [x, 10.]],
            text: text.into(),
            score: 0.99,
        };
        let fallback = finish(fallback(&[
            item(0., "002"),
            item(100., "0031"),
            item(200., "00:03:021"),
            item(300., "00:03:02.1"),
        ]));
        assert_eq!(fallback.columns, 4);
        assert_eq!(fallback.rows[0].cells, result.rows[0].cells);
        assert_eq!(normalize_text("00:03:02|00:03:02"), "00:03:02 00:03:02");
        assert_eq!(
            normalize_text("00：03：02｜00：03：02"),
            "00:03:02 00:03:02"
        );
    }
    #[test]
    fn preserves_blank_cells_and_nonadjacent_repeat_codes() {
        let item = |y, x, text: &str| TextBox {
            quad: [[x, y], [x + 20., y], [x + 20., y + 10.], [x, y + 10.]],
            text: text.into(),
            score: 0.9,
        };
        let result = fallback(&[
            item(0., 0., "1"),
            item(0., 100., "120"),
            item(0., 200., "1:00"),
            item(30., 0., "2"),
            item(30., 200., "2:00"),
            item(60., 0., "3"),
            item(60., 100., "120"),
            item(60., 200., "3:00"),
        ]);
        assert_eq!(result.columns, 3);
        assert_eq!(result.rows[1].cells, vec!["2", "", "2:00"]);
        assert_eq!(result.rows[2].cells[1], "120");
    }
}
