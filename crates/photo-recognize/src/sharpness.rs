//! 鸟眼锐度计算：基于拉普拉斯方差 + 梯度幅值均值 + 边缘密度的融合分数。
//!
//! 纯图像处理函数，不涉 ONNX 推理。分数仅保证单调性（锐利图 > 模糊图），
//! 阈值划定留待产品层决定。

use image::DynamicImage;
use photo_domain::BBox;

// ===========================================================================
// 权重常量（初值，待人工标注样片集标定）
// ===========================================================================

/// 拉普拉斯方差权重
const WEIGHT_LAPLACIAN: f32 = 0.50;
/// 梯度幅值均值权重
const WEIGHT_GRADIENT: f32 = 0.30;
/// 边缘密度（梯度超阈值像素占比）权重
const WEIGHT_EDGE_DENSITY: f32 = 0.20;

/// 边缘检测梯度阈值（超出此值的像素计为边缘像素）
const EDGE_GRADIENT_THRESHOLD: u8 = 30;

// ===========================================================================
// 公共接口
// ===========================================================================

/// 计算眼框区域的锐度融合分数。
///
/// # 参数
/// - `img`: 全分辨率原始图像
/// - `eye`: 眼框（归一化 0-1 坐标，相对全图）
///
/// # 返回
/// - `Some(score)` 锐度分数，越锐利越高
/// - `None` 眼框退化（宽或高 < 2 像素）
pub fn eye_sharpness(img: &DynamicImage, eye: &BBox) -> Option<f32> {
    let (full_w, full_h) = (img.width(), img.height());

    let x1 = (eye.x1 * full_w as f32).floor() as u32;
    let y1 = (eye.y1 * full_h as f32).floor() as u32;
    let x2 = (eye.x2 * full_w as f32).ceil() as u32;
    let y2 = (eye.y2 * full_h as f32).ceil() as u32;

    let ew = x2.saturating_sub(x1);
    let eh = y2.saturating_sub(y1);

    if ew < 2 || eh < 2 {
        return None;
    }

    let region_buf = image::imageops::crop_imm(img, x1, y1, ew, eh).to_image();
    let gray = DynamicImage::ImageRgba8(region_buf).to_luma8();

    let lap_var = laplacian_variance(&gray);
    let grad_mean = gradient_mean_magnitude(&gray);
    let edge_dens = edge_density(&gray);

    let score = WEIGHT_LAPLACIAN * (1.0 + lap_var).ln()
        + WEIGHT_GRADIENT * (1.0 + grad_mean).ln()
        + WEIGHT_EDGE_DENSITY * (1.0 + edge_dens).ln();

    Some(score)
}

// ===========================================================================
// 内部计算函数
// ===========================================================================

fn laplacian_variance(gray: &image::GrayImage) -> f32 {
    let (w, h) = gray.dimensions();
    let w = w as i32;
    let h = h as i32;

    let mut responses = Vec::with_capacity((w * h) as usize);

    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let p = |dx: i32, dy: i32| -> f32 {
                let px = (x + dx).clamp(0, w - 1) as u32;
                let py = (y + dy).clamp(0, h - 1) as u32;
                gray.get_pixel(px, py).0[0] as f32
            };

            let lap = p(0, -1) + p(-1, 0) + p(1, 0) + p(0, 1) - 4.0 * p(0, 0);
            responses.push(lap);
        }
    }

    if responses.is_empty() {
        return 0.0;
    }

    let n = responses.len() as f32;
    let mean = responses.iter().sum::<f32>() / n;
    responses.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / n
}

fn gradient_mean_magnitude(gray: &image::GrayImage) -> f32 {
    let (w, h) = gray.dimensions();
    let w = w as i32;
    let h = h as i32;

    let mut magnitudes = Vec::with_capacity((w * h) as usize);

    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let p = |dx: i32, dy: i32| -> f32 {
                let px = (x + dx).clamp(0, w - 1) as u32;
                let py = (y + dy).clamp(0, h - 1) as u32;
                gray.get_pixel(px, py).0[0] as f32
            };

            let gx = p(1, -1) + 2.0 * p(1, 0) + p(1, 1)
                - p(-1, -1) - 2.0 * p(-1, 0) - p(-1, 1);
            let gy = p(-1, 1) + 2.0 * p(0, 1) + p(1, 1)
                - p(-1, -1) - 2.0 * p(0, -1) - p(1, -1);

            let mag = (gx * gx + gy * gy).sqrt();
            magnitudes.push(mag);
        }
    }

    if magnitudes.is_empty() {
        return 0.0;
    }

    magnitudes.iter().sum::<f32>() / magnitudes.len() as f32
}

fn edge_density(gray: &image::GrayImage) -> f32 {
    let (w, h) = gray.dimensions();
    let w = w as i32;
    let h = h as i32;

    let mut edge_count = 0u32;
    let mut total = 0u32;

    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let p = |dx: i32, dy: i32| -> u8 {
                let px = (x + dx).clamp(0, w - 1) as u32;
                let py = (y + dy).clamp(0, h - 1) as u32;
                gray.get_pixel(px, py).0[0]
            };

            let dx = (p(1, 0) as i16 - p(-1, 0) as i16).unsigned_abs();
            let dy = (p(0, 1) as i16 - p(0, -1) as i16).unsigned_abs();

            let t = EDGE_GRADIENT_THRESHOLD as u16;
            if dx > t || dy > t {
                edge_count += 1;
            }
            total += 1;
        }
    }

    if total == 0 { 0.0 } else { edge_count as f32 / total as f32 }
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbImage;

    fn create_sharp_image(w: u32, h: u32) -> DynamicImage {
        let mut img = RgbImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let val = if (x / 4 + y / 4) % 2 == 0 { 255u8 } else { 0u8 };
                img.put_pixel(x, y, image::Rgb([val, val, val]));
            }
        }
        DynamicImage::ImageRgb8(img)
    }

    fn create_blurred_image(w: u32, h: u32) -> DynamicImage {
        let mut img = RgbImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                img.put_pixel(x, y, image::Rgb([128, 128, 128]));
            }
        }
        DynamicImage::ImageRgb8(img)
    }

    #[test]
    fn test_sharpness_sharp_gt_blurred() {
        let eye = BBox::new(0.0, 0.0, 1.0, 1.0);

        let sharp_img = create_sharp_image(100, 80);
        let blurred_img = create_blurred_image(100, 80);

        let sharp_score = eye_sharpness(&sharp_img, &eye).expect("sharp should return score");
        let blurred_score = eye_sharpness(&blurred_img, &eye).expect("blurred should return score");

        assert!(
            sharp_score > blurred_score,
            "sharp ({}) should be > blurred ({})",
            sharp_score, blurred_score
        );
    }

    #[test]
    fn test_sharpness_degenerate_bbox_returns_none() {
        let img = create_sharp_image(100, 100);

        let tiny_eye = BBox::new(0.0, 0.0, 0.005, 0.5);
        assert!(eye_sharpness(&img, &tiny_eye).is_none());

        let tiny_eye2 = BBox::new(0.0, 0.0, 0.5, 0.005);
        assert!(eye_sharpness(&img, &tiny_eye2).is_none());
    }

    #[test]
    fn test_sharpness_valid_bbox_returns_some() {
        let img = create_sharp_image(100, 100);
        let eye = BBox::new(0.1, 0.1, 0.5, 0.5);

        let score = eye_sharpness(&img, &eye);
        assert!(score.is_some());
        assert!(score.unwrap() > 0.0);
    }
}
