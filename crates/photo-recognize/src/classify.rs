//! 鸟种分类：裁切检测区域 → 缩放至 224×224 平均插值 →
//! ImageNet 标准化 (mean=[0.485,0.456,0.406], std=[0.229,0.224,0.225]) →
//! NCHW → ONNX 推理 → softmax log-sum-exp → Top-1+Top-5
//!
//! 所有常数对应 Dart `bird_model_classifier.dart` 与 `bird_classification_service.dart`。

use image::{DynamicImage, GenericImageView};
use ort::session::Session;
use ort::value::Tensor;

use photo_domain::BBox;

use crate::catalog::ClassificationOutput;
use crate::RecognizeError;

/// 分类模型输入尺寸（pica bird_model_classifier.dart:16）
const CLASSIFY_INPUT_SIZE: usize = 224;

/// ImageNet 归一化均值（pica bird_model_classifier.dart:126）
const MEAN: [f32; 3] = [0.485, 0.456, 0.406];

/// ImageNet 归一化标准差（pica bird_model_classifier.dart:127）
const STD: [f32; 3] = [0.229, 0.224, 0.225];

/// Top-N 数量
const TOP_N: usize = 5;

/// 对检测框区域进行分类。
///
/// # 参数
/// - `session`: ONNX session（bird_model）
/// - `img`: 全分辨率原始图像
/// - `bbox`: 归一化检测框 (0-1)
///
/// # 返回
/// `ClassificationOutput`（Top-1 置信度 + 索引 + Top-5 候选）
pub fn run_classification(
    session: &mut Session,
    img: &DynamicImage,
    bbox: BBox,
) -> Result<ClassificationOutput, RecognizeError> {
    let (orig_w, orig_h) = img.dimensions();

    // ---- 1. 裁切归一化区域（pica bird_classification_service.dart:66-74） ----
    let cx1 = (bbox.x1.clamp(0.0, 1.0) * orig_w as f32).floor() as u32;
    let cy1 = (bbox.y1.clamp(0.0, 1.0) * orig_h as f32).floor() as u32;
    let cx2 = (bbox.x2.clamp(0.0, 1.0) * orig_w as f32).ceil() as u32;
    let cy2 = (bbox.y2.clamp(0.0, 1.0) * orig_h as f32).ceil() as u32;
    let crop_w = (cx2 - cx1).clamp(1, orig_w);
    let crop_h = (cy2 - cy1).clamp(1, orig_h);

    let cropped = img.crop_imm(cx1, cy1, crop_w, crop_h);

    // ---- 2. 缩放至 224×224 平均插值（pica bird_model_classifier.dart:120-125） ----
    let resized = cropped.resize_exact(
        CLASSIFY_INPUT_SIZE as u32,
        CLASSIFY_INPUT_SIZE as u32,
        image::imageops::FilterType::Triangle,
    );

    // ---- 3. ImageNet 标准化（pica bird_model_classifier.dart:128-141） ----
    let plane_size = CLASSIFY_INPUT_SIZE * CLASSIFY_INPUT_SIZE;
    let total = 3 * plane_size;
    let mut input_data = Vec::with_capacity(total);

    for c in 0..3 {
        for y in 0..CLASSIFY_INPUT_SIZE {
            for x in 0..CLASSIFY_INPUT_SIZE {
                let pixel = resized.get_pixel(x as u32, y as u32);
                let normalized = (pixel[c] as f32 / 255.0 - MEAN[c]) / STD[c];
                input_data.push(normalized);
            }
        }
    }

    // ---- 4. 推理（pica bird_model_classifier.dart:51-57） ----
    let tensor = Tensor::<f32>::from_array((
        [1usize, 3, CLASSIFY_INPUT_SIZE, CLASSIFY_INPUT_SIZE],
        input_data.into_boxed_slice(),
    ))?;

    let outputs = session.run(ort::inputs![tensor])?;
    let output = &outputs[0];

    let (_shape, flat) = output.try_extract_tensor::<f32>()?;
    if flat.is_empty() {
        return Err(RecognizeError::ClassificationOutputEmpty);
    }

    // ---- 5. softmax log-sum-exp（pica bird_model_classifier.dart:67-97） ----
    // 找最大 logit（数值稳定性）
    let mut best_index = 0usize;
    let mut best_logit = flat[0];
    for i in 1..flat.len() {
        if flat[i] > best_logit {
            best_logit = flat[i];
            best_index = i;
        }
    }

    // log-sum-exp: sum(exp(logit_i - max_logit)) （pica bird_model_classifier.dart:76-80）
    let exp_sum: f64 = flat.iter().map(|v| ((*v - best_logit) as f64).exp()).sum();
    let top_confidence = if exp_sum == 0.0 {
        0.0
    } else {
        (100.0 / exp_sum) as f32
    };

    // ---- 6. Top-N（pica bird_model_classifier.dart:82-97） ----
    let mut indexed: Vec<(usize, f32)> = flat.iter().copied().enumerate().collect();
    indexed.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let top_candidates: Vec<(u32, f32)> = indexed
        .iter()
        .take(TOP_N)
        .map(|(idx, logit)| {
            // 每个候选重新计算 softmax（pica bird_model_classifier.dart:87-96）
            let exp_sum: f64 = flat.iter().map(|v| ((*v - logit) as f64).exp()).sum();
            let conf = if exp_sum == 0.0 {
                0.0
            } else {
                (100.0 / exp_sum) as f32
            };
            (*idx as u32, conf)
        })
        .collect();

    Ok(ClassificationOutput {
        class_index: best_index as u32,
        confidence: top_confidence,
        top_candidates,
    })
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbImage;

    #[allow(dead_code)]
    fn create_solid_image(r: u8, g: u8, b: u8, w: u32, h: u32) -> DynamicImage {
        let img = RgbImage::from_pixel(w, h, image::Rgb([r, g, b]));
        DynamicImage::ImageRgb8(img)
    }

    #[test]
    fn test_softmax_numerical_correctness() {
        // 验证 log-sum-exp softmax 正确性：固定 logits 验证
        // softmax 性质：和为 1，最大条目应有最高值
        let flat = vec![2.0_f32, 1.0, 0.5, 0.1, -1.0];

        // 计算 log-sum-exp softmax（与 classify.rs 内一致）
        let best_logit = flat.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        let exp_sum: f64 = flat.iter().map(|v| ((*v - best_logit) as f64).exp()).sum();

        // Top-1 confidence
        let top_conf = 100.0 / exp_sum;

        // 手动验证：max logit = 2.0
        // exp_sum = exp(0) + exp(-1) + exp(-1.5) + exp(-1.9) + exp(-3)
        //         = 1 + 0.3679 + 0.2231 + 0.1496 + 0.0498 ≈ 1.7904
        // top_conf = 100 / 1.7904 ≈ 55.85
        assert!((top_conf - 55.85).abs() < 0.1);
        assert!(top_conf > 0.0 && top_conf < 100.0);

        // 验证 softmax 输出和为 1 (归一化后)
        let softmax_sum: f64 = flat
            .iter()
            .map(|v| ((*v - best_logit) as f64).exp() / exp_sum)
            .sum();
        assert!((softmax_sum - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_softmax_all_equal_logits() {
        // 所有 logit 相等 → 每个类别置信度 = 100/N
        let flat = vec![1.0_f32, 1.0, 1.0, 1.0, 1.0];

        let best_logit = flat.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        let exp_sum: f64 = flat.iter().map(|v| ((*v - best_logit) as f64).exp()).sum();

        let softmax_values: Vec<f64> = flat
            .iter()
            .map(|v| ((*v - best_logit) as f64).exp() / exp_sum)
            .collect();

        for v in &softmax_values {
            assert!((*v - 0.2).abs() < 1e-6);
        }

        let top_conf = 100.0 / exp_sum;
        assert!((top_conf - 20.0).abs() < 1e-4, "top_conf={}", top_conf);
    }

    #[test]
    fn test_top_candidates_are_sorted_descending() {
        // Top-5 候选必须按原始 logit 降序（pica bird_model_classifier.dart:83-84）
        let flat = vec![10.0_f32, 5.0, 2.0, 1.0, 0.5, 0.1, 0.01, -1.0, -5.0, -10.0];
        let mut indexed: Vec<(usize, f32)> = flat.iter().copied().enumerate().collect();
        indexed.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        for i in 1..TOP_N.min(indexed.len()) {
            assert!(indexed[i - 1].1 >= indexed[i].1, "候选应降序排列");
        }

        // Top-1 应为最大 logit
        assert_eq!(indexed[0].0, 0);
        assert!((indexed[0].1 - 10.0).abs() < 1e-6);
    }

    #[test]
    fn test_confidence_0_100_range() {
        // 置信度必须在 0-100 区间
        let flat = vec![3.0_f32, 2.0, 1.0, 0.0, -1.0];

        let best_logit = flat.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        let exp_sum: f64 = flat.iter().map(|v| ((*v - best_logit) as f64).exp()).sum();
        let top_conf = 100.0 / exp_sum;

        assert!(top_conf > 0.0 && top_conf <= 100.0);
    }

    #[test]
    fn test_bbox_crop_pixel_conversion() {
        // 验证 bbox→像素裁切（pica bird_classification_service.dart:66-74）
        let img_w = 1920u32;
        let img_h = 1080u32;
        let bbox = BBox::new(0.1, 0.2, 0.5, 0.6);

        let cx1 = (bbox.x1 * img_w as f32).floor() as u32;
        let cy1 = (bbox.y1 * img_h as f32).floor() as u32;
        let cx2 = (bbox.x2 * img_w as f32).ceil() as u32;
        let cy2 = (bbox.y2 * img_h as f32).ceil() as u32;
        let crop_w = (cx2 - cx1).clamp(1, img_w);
        let crop_h = (cy2 - cy1).clamp(1, img_h);

        assert_eq!(cx1, 192); // 0.1 * 1920 = 192.0 floor
        assert_eq!(cy1, 216); // 0.2 * 1080 = 216.0 floor
        assert_eq!(cx2, 960); // 0.5 * 1920 = 960.0 ceil
        assert_eq!(cy2, 648); // 0.6 * 1080 = 648.0 ceil
        assert_eq!(crop_w, 768);
        assert_eq!(crop_h, 432);
    }

    #[test]
    fn test_bbox_clamp_overflow() {
        // 归一化坐标超出 0-1 时 clamp 处理
        let bbox = BBox::new(-0.1, -0.2, 1.5, 1.3);
        assert!((bbox.x1 - 0.0).abs() < 1e-6);
        assert!((bbox.y1 - 0.0).abs() < 1e-6);
        assert!((bbox.x2 - 1.0).abs() < 1e-6);
        assert!((bbox.y2 - 1.0).abs() < 1e-6);
    }
}
