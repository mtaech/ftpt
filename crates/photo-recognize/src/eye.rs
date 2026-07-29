//! 鸟眼检测：从裁剪的鸟体区域跑 eye.onnx → 定位眼关键点。
//!
//! 管线位置：第四阶段「鸟眼锐度」——在识别管线得到鸟框之后运行。
//!
//! ## 模型输出（YOLO26 系端到端姿态头，NMS-free）
//!
//! 输出张量 `[1, 300, 12]`，每行：
//! `[x1, y1, x2, y2(像素), conf, cls, kpt1(x, y, conf), kpt2(x, y, conf)]`
//! ——鸟框恒为整幅（训练集即整幅鸟图），实际只用两个眼关键点；
//! 取置信度最高的关键点，展开为方形小框后映射回全图坐标。

use image::{DynamicImage, GenericImageView};
use ort::session::Session;
use ort::value::Tensor;

use photo_domain::BBox;

use crate::RecognizeError;

// ---------------------------------------------------------------------------
// 常量
// ---------------------------------------------------------------------------

/// 眼关键点置信度阈值（实测：真眼 0.86–0.93，噪声 <0.1，余量充足）
const EYE_KPT_CONF_THRESHOLD: f32 = 0.5;

/// 眼框边长相对裁剪区域的比例（覆盖眼睛及周围羽毛供锐度统计）
const EYE_BOX_SIDE_RATIO: f32 = 0.15;

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
    // eye.onnx 是 YOLO26 系端到端姿态头（NMS-free），输出 [1, 300, 12]，
    // 每行 12 值：[x1,y1,x2,y2(像素), conf, cls, kpt1(x,y,conf), kpt2(x,y,conf)]。
    // 鸟框对本场景无用（训练集即整幅鸟图，框恒为整幅），只用两个眼关键点。
    let (_shape, flat) = output.try_extract_tensor::<f32>()?;
    let Some((nx, ny, _conf)) = pick_best_eye_keypoint(&flat, input_w, input_h) else {
        return Ok(None);
    };

    // 关键点 → 以眼为中心的方形小框（crop 归一化），再映射回全图
    let half = EYE_BOX_SIDE_RATIO / 2.0;
    let (ex1, ey1) = ((nx - half).clamp(0.0, 1.0), (ny - half).clamp(0.0, 1.0));
    let (ex2, ey2) = ((nx + half).clamp(0.0, 1.0), (ny + half).clamp(0.0, 1.0));

    let fw = full_w as f32;
    let fh = full_h as f32;
    let cw = crop_w as f32;
    let ch = crop_h as f32;
    let cox = px1 as f32;
    let coy = py1 as f32;

    Ok(Some(BBox::new(
        (cox + ex1 * cw) / fw,
        (coy + ey1 * ch) / fh,
        (cox + ex2 * cw) / fw,
        (coy + ey2 * ch) / fh,
    )))
}

/// 从姿态头输出中挑选置信度最高的眼关键点（ADR 0005：取最高置信度单眼）。
///
/// 输入为展平的 f32 输出（每 12 值一行），返回 (crop 归一化 x, y, conf)。
fn pick_best_eye_keypoint(flat: &[f32], input_w: usize, input_h: usize) -> Option<(f32, f32, f32)> {
    if flat.len() < 12 || flat.len() % 12 != 0 {
        return None;
    }
    let mut best: Option<(f32, f32, f32)> = None;
    for row in flat.chunks_exact(12) {
        for kpt in [&row[6..9], &row[9..12]] {
            let (kx, ky, kc) = (kpt[0], kpt[1], kpt[2]);
            if kc < EYE_KPT_CONF_THRESHOLD {
                continue;
            }
            if best.is_none_or(|b| kc > b.2) {
                best = Some((kx / input_w as f32, ky / input_h as f32, kc));
            }
        }
    }
    best
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

    /// 关键点解析：取置信度最高的单眼（跨行、跨两个关键点位）
    #[test]
    fn test_pick_best_eye_keypoint_picks_highest_conf() {
        let row = |k1: (f32, f32, f32), k2: (f32, f32, f32)| -> Vec<f32> {
            vec![
                0.0, 0.0, 640.0, 640.0, 0.9, 0.0, k1.0, k1.1, k1.2, k2.0, k2.1, k2.2,
            ]
        };
        let mut flat = row((100.0, 100.0, 0.6), (200.0, 200.0, 0.9));
        flat.extend(row((300.0, 300.0, 0.7), (50.0, 50.0, 0.8)));

        let (nx, ny, conf) = pick_best_eye_keypoint(&flat, 640, 640).unwrap();
        assert!((conf - 0.9).abs() < 1e-6);
        assert!((nx - 200.0 / 640.0).abs() < 1e-6);
        assert!((ny - 200.0 / 640.0).abs() < 1e-6);
    }

    /// 全部关键点低于阈值 → None
    #[test]
    fn test_pick_best_eye_keypoint_below_threshold() {
        let flat = vec![
            0.0, 0.0, 640.0, 640.0, 0.9, 0.0, 100.0, 100.0, 0.3, 200.0, 200.0, 0.49,
        ];
        assert!(pick_best_eye_keypoint(&flat, 640, 640).is_none());
    }

    /// 长度非法（非 12 的倍数 / 空）→ None 而非 panic
    #[test]
    fn test_pick_best_eye_keypoint_malformed_input() {
        assert!(pick_best_eye_keypoint(&[], 640, 640).is_none());
        assert!(pick_best_eye_keypoint(&[1.0; 6], 640, 640).is_none());
        assert!(pick_best_eye_keypoint(&[1.0; 13], 640, 640).is_none());
    }
}
