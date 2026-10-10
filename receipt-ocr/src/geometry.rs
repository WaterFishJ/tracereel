use image::{Rgb, RgbImage};
use imageproc::geometric_transformations::{warp_into, Interpolation, Projection};

pub type Quad = [[f32; 2]; 4];
pub fn distance(a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}
pub fn warp(image: &RgbImage, quad: Quad, width: u32, height: u32) -> Result<RgbImage, String> {
    let from = quad.map(|p| (p[0], p[1]));
    let to = [
        (0., 0.),
        ((width - 1) as f32, 0.),
        ((width - 1) as f32, (height - 1) as f32),
        (0., (height - 1) as f32),
    ];
    let projection =
        Projection::from_control_points(from, to).ok_or("无法校正四角，请重新选择区域")?;
    let mut output = RgbImage::new(width, height);
    warp_into(
        image,
        &projection,
        Interpolation::Bilinear,
        Rgb([255; 3]),
        &mut output,
    );
    Ok(output)
}
pub fn rectify(image: &RgbImage, corners: Quad, rotation: u8) -> Result<RgbImage, String> {
    if corners
        .iter()
        .flatten()
        .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
    {
        return Err("裁剪点超出图片范围".into());
    }
    let quad = corners.map(|p| {
        [
            p[0] * (image.width() - 1) as f32,
            p[1] * (image.height() - 1) as f32,
        ]
    });
    let cross: Vec<f32> = (0..4)
        .map(|i| {
            let a = quad[i];
            let b = quad[(i + 1) % 4];
            let c = quad[(i + 2) % 4];
            (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0])
        })
        .collect();
    if !cross.iter().all(|v| *v > 0.) {
        return Err("四角必须按左上、右上、右下、左下构成不交叉的凸四边形".into());
    }
    let w = distance(quad[0], quad[1])
        .max(distance(quad[2], quad[3]))
        .round();
    let h = distance(quad[0], quad[3])
        .max(distance(quad[1], quad[2]))
        .round();
    if w.min(h) < 30. {
        return Err("裁剪区域过小".into());
    }
    let scale = 2.0_f32.min(2200. / w.max(h));
    let output = warp(
        image,
        quad,
        (w * scale).round() as u32,
        (h * scale).round() as u32,
    )?;
    Ok(match rotation % 4 {
        1 => image::imageops::rotate90(&output),
        2 => image::imageops::rotate180(&output),
        3 => image::imageops::rotate270(&output),
        _ => output,
    })
}
pub fn line_crop(image: &RgbImage, quad: Quad) -> Result<RgbImage, String> {
    let w = distance(quad[0], quad[1])
        .max(distance(quad[2], quad[3]))
        .round()
        .max(8.) as u32;
    let h = distance(quad[0], quad[3])
        .max(distance(quad[1], quad[2]))
        .round()
        .max(8.) as u32;
    let output = warp(image, quad, w, h)?;
    Ok(if h as f32 / w as f32 >= 1.5 {
        image::imageops::rotate270(&output)
    } else {
        output
    })
}
pub fn padded(image: &RgbImage) -> RgbImage {
    let mut result = RgbImage::from_pixel(image.width() + 6, image.height() + 6, Rgb([255; 3]));
    image::imageops::replace(&mut result, image, 3, 3);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn crop_validation_and_rotation() {
        let image = RgbImage::from_pixel(100, 60, Rgb([255; 3]));
        let quad = [[0., 0.], [1., 0.], [1., 1.], [0., 1.]];
        let out = rectify(&image, quad, 0).unwrap();
        let rotated = rectify(&image, quad, 1).unwrap();
        assert_eq!(out.dimensions(), (198, 118));
        assert_eq!(rotated.dimensions(), (118, 198));
        assert!(rectify(&image, [[0., 0.], [1., 1.], [1., 0.], [0., 1.]], 0).is_err());
        assert!(rectify(&image, [[0., 0.], [0.01, 0.], [0.01, 1.], [0., 1.]], 0).is_err());
    }
}
