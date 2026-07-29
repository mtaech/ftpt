//! 鸟眼检测：从裁剪的鸟体区域跑 eye.onnx → 检测眼框。
//!
//! 管线位置：第四阶段「鸟眼锐度」——在识别管线得到鸟框之后运行。
//!
//! ## 坐标映射
//!
//! - eye.onnx 输入：鸟框裁剪图经 resize 至模型输入尺寸
//! - 模型输出：归一化坐标（相对模型输入尺寸）
//! - 最终眼框：归一化坐标映射回全图，与检测框同坐标系

use image::{DynamicImage, GenericImageView};
use ort::session::Session;
use ort::value::Tensor;

use photo_domain::BBox;

use crate::RecognizeError;

// ---------------------------------------------------------------------------
// 公共接口
// ---------------------------------------------------------------------------

/// 在鸟框裁剪区域上检测眼睛，返回全图归一化眼框。
///
/// # 参数
/// - `session`: eye.onnx session
/// - `img`: 全分辨率原始图像
/// - `bird_bbox`: 鸟体检测框（归一化 0-1，全图坐标）
///
/// # 返回
/// - `Ok(Some(bbox))` 检测到眼框
/// - `Ok(None)` 无有效眼框（不视为错误）
/// - `Err(...)` 模型/推理系统故障
pub fn detect_eye(
    session: &mut Session,
    img: &DynamicImage,
    bird_bbox: BBox,
) -> Result<Option<BBox>, RecognizeError> {
    // ---- 从 session 元数据读取模型输入尺寸（不硬编码） ----
    let input_shape = session.inputs()[0]
        .dtype()
        .tensor_shape()
        .ok_or_else(|| RecognizeError::ModelLoad("eye.onnx 输入非张量类型".into()))?;

    // 兼容 NCHW ([1, 3, H, W]) 与 NHWC ([1, H, W, 3])
    let (input_h, input_w) = if input_shape.len() >= 4 {
        let d0 = input_shape[1];
        let d3 = input_shape[3];
        if d3 == 3 || d3 == -1 {
            (input_shape[1], input_shape[2])
        } else if d0 == 3 || d0 == -1 {
            (input_shape[2], input_shape[3])
        } else if d0 == 1 {
            (input_shape[2], input_shape[3])
        } else if d3 == 1 {
            (input_shape[1], input_shape[2])
        } else {
            (input_shape[2], input_shape[3])
        }
    } else {
        return Err(RecognizeError::ModelLoad("eye.onnx 输入至少需要 4 维".into()));
    };

    let input_h = if input_h <= 0 { 640usize } else { input_h as usize };
    let input_w = if input_w <= 0 { 640usize } else { input_w as usize };

    // ---- 从 bird_bbox 裁剪鸟体区域（外扩 ~10% 并夹紧） ----
    let (full_w, full_h) = img.dimensions();

    let margin = 0.10;
    let expand_w = (bird_bbox.x2 - bird_bbox.x1) * margin;
    let expand_h = (bird_bbox.y2 - bird_bbox.y1) * margin;

    let crop_x1 = (bird_bbox.x1 - expand_w).clamp(0.0, 1.0);
    let crop_y1 = (bird_bbox.y1 - expand_h).clamp(0.0, 1.0);
    let crop_x2 = (bird_bbox.x2 + expand_w).clamp(0.0, 1.0);
    let crop_y2 = (bird_bbox.y2 + expand_h).clamp(0.0, 1.0);

    let px1 = (crop_x1 * full_w as f32).floor() as u32;
    let py1 = (crop_y1 * full_h as f32).floor() as u32;
    let px2 = (crop_x2 * full_w as f32).ceil() as u32;
    let py2 = (crop_y2 * full_h as f32).ceil() as u32;

    let crop_w = (px2 - px1).max(1);
    let crop_h = (py2 - py1).max(1);

    let crop = DynamicImage::ImageRgba8(
        image::imageops::crop_imm(img, px1, py1, crop_w, crop_h).to_image(),
    );

    // ---- 预处理：resize 至模型输入尺寸 + RGB/255 归一化 ----
    let resized = crop.resize_exact(
        input_w as u32, input_h as u32,
        image::imageops::FilterType::CatmullRom,
    );

    let plane_size = input_w * input_h;
    let total = 3 * plane_size;
    let mut input_data = Vec::with_capacity(total);

    for c in 0..3 {
        for y in 0..input_h {
            for x in 0..input_w {
                let pixel = resized.get_pixel(x as u32, y as u32);
                let val = pixel[c] as f32 / 255.0;
                input_data.push(val);
            }
        }
    }

    // ---- 推理 ----
    let tensor = Tensor::<f32>::from_array(
        ([1usize, 3, input_h, input_w], input_data.into_boxed_slice()),
    )?;

    let outputs = session.run(ort::inputs![tensor])?;
    let output = &outputs[0];

    // ---- 解析输出 ----
    let (shape, flat) = output.try_extract_tensor::<f32>()?;

    if flat.is_empty() {
        return Ok(None);
    }

    let total_len = flat.len();

    let stride = if shape.len() >= 3 && shape[shape.len() - 1] == 6 {
        6
    } else if shape.len() >= 2 && shape[shape.len() - 1] == 6 {
        6
    } else if total_len % 6 == 0 && total_len > 0 {
        6
    } else if total_len % 4 == 0 {
        4
    } else {
        return Err(RecognizeError::ModelLoad(format!(
            "eye.onnx 输出形状无法解析: len={}", total_len
        )));
    };

    // ---- 后处理：筛选最高分眼框 ----
    let score_threshold = 0.25;
    let mut best: Option<(f32, f32, f32, f32, f32)> = None;

    for offset in (0..total_len - (stride - 1)).step_by(stride) {
        let score = if stride >= 5 { flat[offset + 4] } else { 1.0 };

        if score < score_threshold {
            continue;
        }

        let x1_raw = flat[offset];
        let y1_raw = flat[offset + 1];
        let x2_raw = flat[offset + 2];
        let y2_raw = flat[offset + 3];

        let x1 = (x1_raw / input_w as f32).clamp(0.0, 1.0);
        let y1 = (y1_raw / input_h as f32).clamp(0.0, 1.0);
        let x2 = (x2_raw / input_w as f32).clamp(0.0, 1.0);
        let y2 = (y2_raw / input_h as f32).clamp(0.0, 1.0);

        if x2 <= x1 || y2 <= y1 {
            continue;
        }

        let is_better = match &best {
            None => true,
            Some(b) => score > b.4,
        };
        if is_better {
            best = Some((x1, y1, x2, y2, score));
        }
    }

    // 将 crop 内的归一化坐标映射回全图
    let eye_bbox = best.map(|(x1, y1, x2, y2, _score)| {
        let fw = full_w as f32;
        let fh = full_h as f32;
        let cw = crop_w as f32;
        let ch = crop_h as f32;
        let cox = px1 as f32;
        let coy = py1 as f32;

        BBox::new(
            (cox + x1 * cw) / fw,
            (coy + y1 * ch) / fh,
            (cox + x2 * cw) / fw,
            (coy + y2 * ch) / fh,
        )
    });

    Ok(eye_bbox)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证 crop 内坐标 → 全图归一化坐标的正逆变换一致性。
    #[test]
    fn test_eye_coord_mapping_crop_to_full() {
        let bird_bbox = BBox::new(0.2, 0.3, 0.6, 0.7);
        let full_w: f32 = 1600.0;
        let full_h: f32 = 1200.0;

        let margin = 0.10;
        let expand_w = (bird_bbox.x2 - bird_bbox.x1) * margin;
        let expand_h = (bird_bbox.y2 - bird_bbox.y1) * margin;

        let crop_x1 = (bird_bbox.x1 - expand_w).clamp(0.0, 1.0);
        let crop_y1 = (bird_bbox.y1 - expand_h).clamp(0.0, 1.0);
        let crop_x2 = (bird_bbox.x2 + expand_w).clamp(0.0, 1.0);
        let crop_y2 = (bird_bbox.y2 + expand_h).clamp(0.0, 1.0);

        let px1 = (crop_x1 * full_w).floor() as u32;
        let py1 = (crop_y1 * full_h).floor() as u32;
        let px2 = (crop_x2 * full_w).ceil() as u32;
        let py2 = (crop_y2 * full_h).ceil() as u32;

        let crop_w = (px2 - px1).max(1) as f32;
        let crop_h = (py2 - py1).max(1) as f32;
        let cox = px1 as f32;
        let coy = py1 as f32;

        let eye_in_crop = BBox::new(0.4, 0.4, 0.6, 0.6);

        // 正向：crop 内归一化 → 全图归一化
        let eye_full = BBox::new(
            (cox + eye_in_crop.x1 * crop_w) / full_w,
            (coy + eye_in_crop.y1 * crop_h) / full_h,
            (cox + eye_in_crop.x2 * crop_w) / full_w,
            (coy + eye_in_crop.y2 * crop_h) / full_h,
        );

        // 逆向：全图归一化 → crop 内归一化
        let roundtrip = BBox::new(
            (eye_full.x1 * full_w - cox) / crop_w,
            (eye_full.y1 * full_h - coy) / crop_h,
            (eye_full.x2 * full_w - cox) / crop_w,
            (eye_full.y2 * full_h - coy) / crop_h,
        );

        // 正逆变换应在 f32 精度内一致
        assert!((roundtrip.x1 - eye_in_crop.x1).abs() < 1e-4, "x1");
        assert!((roundtrip.y1 - eye_in_crop.y1).abs() < 1e-4, "y1");
        assert!((roundtrip.x2 - eye_in_crop.x2).abs() < 1e-4, "x2");
        assert!((roundtrip.y2 - eye_in_crop.y2).abs() < 1e-4, "y2");

        // 全图范围检查
        assert!(eye_full.x1 >= 0.0 && eye_full.x1 <= 1.0);
        assert!(eye_full.y1 >= 0.0 && eye_full.y1 <= 1.0);
        assert!(eye_full.x2 >= 0.0 && eye_full.x2 <= 1.0);
        assert!(eye_full.y2 >= 0.0 && eye_full.y2 <= 1.0);
    }

    /// 验证边界处的裁剪计算
    #[test]
    fn test_eye_crop_edge_cases() {
        let bird_bbox = BBox::new(0.0, 0.0, 0.1, 0.1);
        let full_w = 100u32;
        let full_h = 100u32;

        let margin = 0.10;
        let expand_w = (bird_bbox.x2 - bird_bbox.x1) * margin;
        let expand_h = (bird_bbox.y2 - bird_bbox.y1) * margin;

        let crop_x1 = (bird_bbox.x1 - expand_w).clamp(0.0, 1.0);
        let crop_y1 = (bird_bbox.y1 - expand_h).clamp(0.0, 1.0);
        let crop_x2 = (bird_bbox.x2 + expand_w).clamp(0.0, 1.0);
        let crop_y2 = (bird_bbox.y2 + expand_h).clamp(0.0, 1.0);

        let px1 = (crop_x1 * full_w as f32).floor() as u32;
        let py1 = (crop_y1 * full_h as f32).floor() as u32;
        let px2 = (crop_x2 * full_w as f32).ceil() as u32;
        let py2 = (crop_y2 * full_h as f32).ceil() as u32;

        assert_eq!(px1, 0);
        assert_eq!(py1, 0);
        assert!(px2 >= px1 + 1);
        assert!(py2 >= py1 + 1);
    }
}
