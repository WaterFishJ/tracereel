//! Rust adaptation of PP-OCRv4 preprocessing and DB/CTC postprocessing.
//! Copyright (c) 2020 PaddlePaddle Authors. All Rights Reserved.
//! Copyright (c) 2021 RapidOCR Authors. All rights reserved.
//! Original algorithms: Apache-2.0, see ocr/licenses. Modified for Rust by TraceReel.
use crate::geometry::Quad;
use image::{GrayImage, Luma, RgbImage};
use ort::{
    session::{builder::GraphOptimizationLevel, Session},
    value::Tensor,
};
use std::path::Path;

pub struct Models {
    det: Session,
    rec: Session,
    cls: Session,
    chars: Vec<String>,
}
#[derive(Clone, Debug)]
pub struct TextBox {
    pub quad: Quad,
    pub text: String,
    pub score: f32,
}
pub fn session(path: &Path) -> Result<Session, String> {
    Session::builder()
        .and_then(|b| b.with_optimization_level(GraphOptimizationLevel::Level3))
        .and_then(|b| b.with_intra_threads(2))
        .and_then(|b| b.commit_from_file(path))
        .map_err(|e| e.to_string())
}
impl Models {
    pub fn load(root: &Path) -> Result<Self, String> {
        let det = session(&root.join("models/ch_PP-OCRv4_det_infer.onnx"))?;
        let rec = session(&root.join("models/ch_PP-OCRv4_rec_infer.onnx"))?;
        let cls = session(&root.join("models/ch_ppocr_mobile_v2.0_cls_infer.onnx"))?;
        let dictionary = rec
            .metadata()
            .and_then(|m| m.custom("character"))
            .map_err(|e| e.to_string())?
            .ok_or("识别模型缺少字符字典")?;
        let mut chars = vec![String::new()];
        chars.extend(dictionary.lines().map(str::to_owned));
        chars.push(" ".into());
        Ok(Self {
            det,
            rec,
            cls,
            chars,
        })
    }
    pub fn detect(&mut self, image: &RgbImage) -> Result<Vec<Quad>, String> {
        let max = image.width().max(image.height());
        let limit = if max < 960 {
            960
        } else if max < 1500 {
            1500
        } else {
            2000
        };
        let scale = (limit as f32 / max as f32).min(1.);
        let w = ((image.width() as f32 * scale / 32.).round() as u32 * 32).max(32);
        let h = ((image.height() as f32 * scale / 32.).round() as u32 * 32).max(32);
        let data = normalized(image, w, h, w);
        let tensor = Tensor::from_array(([1, 3, h as usize, w as usize], data))
            .map_err(|e| e.to_string())?;
        let output = self
            .det
            .run(ort::inputs![tensor])
            .map_err(|e| e.to_string())?;
        let (shape, prob) = output[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| e.to_string())?;
        let mw = shape[3] as u32;
        let mh = shape[2] as u32;
        let mut bitmap = GrayImage::new(mw, mh);
        // Paddle's 2x2 dilation (anchor at 1,1).
        for y in 0..mh {
            for x in 0..mw {
                let ink = (y.saturating_sub(1)..=y).any(|yy| {
                    (x.saturating_sub(1)..=x).any(|xx| prob[(yy * mw + xx) as usize] > 0.3)
                });
                if ink {
                    bitmap.put_pixel(x, y, Luma([255]));
                }
            }
        }
        let mut boxes = Vec::new();
        for contour in imageproc::contours::find_contours::<i32>(&bitmap)
            .into_iter()
            .take(1000)
        {
            if contour.border_type != imageproc::contours::BorderType::Outer
                || contour.points.len() < 4
            {
                continue;
            }
            let points = imageproc::geometry::min_area_rect(&contour.points);
            let mut quad = points.map(|p| [p.x as f32, p.y as f32]);
            order(&mut quad);
            let rw = crate::geometry::distance(quad[0], quad[1]);
            let rh = crate::geometry::distance(quad[0], quad[3]);
            if rw.min(rh) < 3. {
                continue;
            }
            let xmin = quad
                .iter()
                .map(|p| p[0])
                .fold(f32::INFINITY, f32::min)
                .floor()
                .max(0.) as u32;
            let xmax = quad
                .iter()
                .map(|p| p[0])
                .fold(0., f32::max)
                .ceil()
                .min((mw - 1) as f32) as u32;
            let ymin = quad
                .iter()
                .map(|p| p[1])
                .fold(f32::INFINITY, f32::min)
                .floor()
                .max(0.) as u32;
            let ymax = quad
                .iter()
                .map(|p| p[1])
                .fold(0., f32::max)
                .ceil()
                .min((mh - 1) as f32) as u32;
            let mut sum = 0.;
            let mut count = 0;
            for y in ymin..=ymax {
                for x in xmin..=xmax {
                    if inside(quad, [x as f32, y as f32]) {
                        sum += prob[(y * mw + x) as usize];
                        count += 1;
                    }
                }
            }
            if count == 0 || sum / (count as f32) < 0.5 {
                continue;
            }
            let expand = rw * rh * 1.6 / (2. * (rw + rh));
            let ux = [
                (quad[1][0] - quad[0][0]) / rw,
                (quad[1][1] - quad[0][1]) / rw,
            ];
            let uy = [
                (quad[3][0] - quad[0][0]) / rh,
                (quad[3][1] - quad[0][1]) / rh,
            ];
            for (i, sign) in [[-1., -1.], [1., -1.], [1., 1.], [-1., 1.]]
                .iter()
                .enumerate()
            {
                quad[i][0] = ((quad[i][0] + expand * (sign[0] * ux[0] + sign[1] * uy[0]))
                    / mw as f32
                    * image.width() as f32)
                    .round()
                    .clamp(0., (image.width() - 1) as f32);
                quad[i][1] = ((quad[i][1] + expand * (sign[0] * ux[1] + sign[1] * uy[1]))
                    / mh as f32
                    * image.height() as f32)
                    .round()
                    .clamp(0., (image.height() - 1) as f32);
            }
            boxes.push(quad);
        }
        boxes.sort_by(|a, b| center_y(a).total_cmp(&center_y(b)));
        Ok(boxes)
    }
    pub fn classify(&mut self, images: &mut [RgbImage]) -> Result<(), String> {
        for batch in images.chunks_mut(6) {
            let mut data = Vec::new();
            for image in batch.iter() {
                let w = ((48. * image.width() as f32 / image.height() as f32).ceil() as u32)
                    .clamp(1, 192);
                data.extend(normalized(image, w, 48, 192));
            }
            let tensor =
                Tensor::from_array(([batch.len(), 3, 48, 192], data)).map_err(|e| e.to_string())?;
            let output = self
                .cls
                .run(ort::inputs![tensor])
                .map_err(|e| e.to_string())?;
            let (_, p) = output[0]
                .try_extract_tensor::<f32>()
                .map_err(|e| e.to_string())?;
            for (i, image) in batch.iter_mut().enumerate() {
                if p[i * 2 + 1] > 0.9 && p[i * 2 + 1] > p[i * 2] {
                    *image = image::imageops::rotate180(image);
                }
            }
        }
        Ok(())
    }
    pub fn recognize(&mut self, images: &[RgbImage]) -> Result<Vec<(String, f32)>, String> {
        let mut indices: Vec<usize> = (0..images.len()).collect();
        indices.sort_by(|a, b| ratio(&images[*a]).total_cmp(&ratio(&images[*b])));
        let mut results = vec![(String::new(), 0.); images.len()];
        for batch in indices.chunks(6) {
            let w = (48.
                * batch
                    .iter()
                    .map(|i| ratio(&images[*i]))
                    .fold(320. / 48., f32::max)) as u32;
            let mut data = Vec::new();
            for i in batch {
                let resized = (48. * ratio(&images[*i])).ceil() as u32;
                data.extend(normalized(&images[*i], resized.min(w).max(1), 48, w));
            }
            let tensor = Tensor::from_array(([batch.len(), 3, 48, w as usize], data))
                .map_err(|e| e.to_string())?;
            let output = self
                .rec
                .run(ort::inputs![tensor])
                .map_err(|e| e.to_string())?;
            let (shape, p) = output[0]
                .try_extract_tensor::<f32>()
                .map_err(|e| e.to_string())?;
            let steps = shape[1] as usize;
            let classes = shape[2] as usize;
            if classes != self.chars.len() {
                return Err(format!("模型字典不匹配：{classes}/{}", self.chars.len()));
            }
            for (n, i) in batch.iter().enumerate() {
                results[*i] = decode(
                    &p[n * steps * classes..(n + 1) * steps * classes],
                    classes,
                    &self.chars,
                );
            }
        }
        Ok(results)
    }
    pub fn lines(&mut self, image: &RgbImage) -> Result<Vec<TextBox>, String> {
        let boxes = self.detect(image)?;
        let mut crops = boxes
            .iter()
            .map(|q| crate::geometry::line_crop(image, *q))
            .collect::<Result<Vec<_>, _>>()?;
        self.classify(&mut crops)?;
        let texts = self.recognize(&crops)?;
        Ok(boxes
            .into_iter()
            .zip(texts)
            .filter(|(_, (_, s))| *s >= 0.5)
            .map(|(quad, (text, score))| TextBox { quad, text, score })
            .collect())
    }
}
fn ratio(image: &RgbImage) -> f32 {
    image.width() as f32 / image.height() as f32
}
pub fn center_y(quad: &Quad) -> f32 {
    quad.iter().map(|p| p[1]).sum::<f32>() / 4.
}
pub fn order(quad: &mut Quad) {
    quad.sort_by(|a, b| a[0].total_cmp(&b[0]));
    if quad[0][1] > quad[1][1] {
        quad.swap(0, 1);
    }
    if quad[2][1] > quad[3][1] {
        quad.swap(2, 3);
    }
    quad.swap(2, 3);
    quad.swap(1, 3);
}
fn inside(q: Quad, p: [f32; 2]) -> bool {
    (0..4).all(|i| {
        let a = q[i];
        let b = q[(i + 1) % 4];
        (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]) >= -0.01
    })
}
// Paddle consumes OpenCV BGR, normalized to [-1,1], with zero padding.
fn normalized(image: &RgbImage, w: u32, h: u32, padded_w: u32) -> Vec<f32> {
    let resized = image::imageops::resize(image, w, h, image::imageops::FilterType::Triangle);
    let mut data = vec![0.; 3 * (h * padded_w) as usize];
    for (x, y, p) in resized.enumerate_pixels() {
        for c in 0..3 {
            data[c * (h * padded_w) as usize + (y * padded_w + x) as usize] =
                p[2 - c] as f32 / 127.5 - 1.;
        }
    }
    data
}
fn decode(prob: &[f32], classes: usize, chars: &[String]) -> (String, f32) {
    let mut last = usize::MAX;
    let mut text = String::new();
    let mut scores = Vec::new();
    for step in prob.chunks_exact(classes) {
        let (index, score) = step
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .unwrap();
        if index != 0 && index != last {
            text.push_str(&chars[index]);
            scores.push(*score);
        }
        last = index;
    }
    let score = if scores.is_empty() {
        0.
    } else {
        scores.iter().sum::<f32>() / scores.len() as f32
    };
    (text, score)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ctc_keeps_repeated_digits_separated_by_blank() {
        let chars = vec!["".into(), "1".into(), "2".into()];
        assert_eq!(
            decode(
                &[0., 1., 0., 0., 1., 0., 1., 0., 0., 0., 1., 0., 0., 0., 1.],
                3,
                &chars
            )
            .0,
            "112"
        );
    }
}
