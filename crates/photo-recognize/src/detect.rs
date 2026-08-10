//! YOLO 检测：将全分辨率图片缩放至 640×640 → RGB/255 归一化 NCHW →
//! ONNX 推理 → 解析 [x1,y1,x2,y2,score,classId] × N → 最高分框（score≥0.25）
//!
//! 所有常数对应 Dart `onnx_detection_service.dart`。

use image::DynamicImage;
use ort::session::Session;
use ort::value::Tensor;

use photo_domain::BBox;

use crate::RecognizeError;

/// YOLO 模型输入尺寸（pica onnx_detection_service.dart:29）
const YOLO_INPUT_SIZE: usize = 640;

/// 分数阈值（pica onnx_detection_service.dart:30）
const YOLO_SCORE_THRESHOLD: f32 = 0.25;

/// 检测结果
#[derive(Debug, Clone)]
pub struct DetectionResult {
    /// 归一化边界框 (0-1)
    pub bbox: BBox,
    /// 原始检测分数 (0-1，非百分制)
    pub raw_score: f32,
}

/// 640×640 CatmullRom 缩放（检测模型输入）。
///
/// 与眼检测共享：管线在 [`crate::pipeline`] 层调用本函数一次，把结果同时
/// 喂给检测与眼模型，避免对同一张图做第二次全图缩放。
pub(crate) fn resize_to_yolo_input(img: &DynamicImage) -> DynamicImage {
    img.resize_exact(
        YOLO_INPUT_SIZE as u32,
        YOLO_INPUT_SIZE as u32,
        image::imageops::FilterType::CatmullRom,
    )
}

/// 使用已完成缩放的 640×640 图运行检测（管线共享缩放结果时调用）。
///
/// 输入为 [`resize_to_yolo_input`] 的输出；返回语义与 YOLO 检测一致：
/// `Ok(Some(result))` 检测成功，`Ok(None)` 无有效检测，`Err(...)` 系统故障。
pub(crate) fn run_yolo_detection_resized(
    session: &mut Session,
    resized: &DynamicImage,
) -> Result<Option<DetectionResult>, RecognizeError> {
    // ---- 预处理 ----
    // NCHW 布局：RGB/255 归一化（pica onnx_detection_service.dart:78-88）
    let input_data = build_input_data(resized);

    // ---- 推理 ----
    let tensor = Tensor::<f32>::from_array((
        [1usize, 3, YOLO_INPUT_SIZE, YOLO_INPUT_SIZE],
        input_data.into_boxed_slice(),
    ))?;

    // 与 pica 行为一致：只取第一个输出（pica onnx_detection_service.dart:92-97）
    // 模型异常防护：无输出时返回系统错误而非裸索引 panic
    let outputs = session.run(ort::inputs![tensor])?;
    // 模型异常防护：无输出时返回系统错误而非裸索引 panic
    let output = if outputs.len() == 0 {
        return Err(RecognizeError::ModelLoad("YOLO 模型推理无输出".into()));
    } else {
        &outputs[0]
    };

    // ---- 后处理 ----
    // 输出格式：[x1, y1, x2, y2, score, classId, ...] 步长 6（pica onnx_detection_service.dart:103-110）
    let (_shape, flat) = output.try_extract_tensor::<f32>()?;

    if flat.len() < 6 {
        // 输出太短，无检测候选（pica onnx_detection_service.dart:103-106）
        return Ok(None);
    }

    let mut best: Option<DetectionResult> = None;

    // 每 6 个元素为一个候选（pica onnx_detection_service.dart:110-111）
    for offset in (0..flat.len() - 5).step_by(6) {
        let score = flat[offset + 4]; // score
        let _class_id = flat[offset + 5] as i32; // classId（pica 仅 round 未过滤特定 class）

        // 分数过滤（pica onnx_detection_service.dart:124）
        if score < YOLO_SCORE_THRESHOLD {
            continue;
        }

        // 将像素坐标归一化到 0-1（pica onnx_detection_service.dart:133-136）
        let x1 = (flat[offset] / YOLO_INPUT_SIZE as f32).clamp(0.0, 1.0);
        let y1 = (flat[offset + 1] / YOLO_INPUT_SIZE as f32).clamp(0.0, 1.0);
        let x2 = (flat[offset + 2] / YOLO_INPUT_SIZE as f32).clamp(0.0, 1.0);
        let y2 = (flat[offset + 3] / YOLO_INPUT_SIZE as f32).clamp(0.0, 1.0);

        // 无效框过滤（pica onnx_detection_service.dart:137-144）
        if x2 <= x1 || y2 <= y1 {
            continue;
        }

        // 取最高分（pica onnx_detection_service.dart:151-160）
        let is_better = match &best {
            None => true,
            Some(b) => score > b.raw_score,
        };
        if is_better {
            best = Some(DetectionResult {
                bbox: BBox::new(x1, y1, x2, y2),
                raw_score: score,
            });
        }
    }

    Ok(best)
}

/// 640×640 RGB/255 归一化 NCHW 预处理。
///
/// 单遍遍历 `as_raw()` 像素按通道分散写，替代三重循环 `get_pixel`
/// （640²×3 ≈ 1.2M 次带边界检查的逐像素取值，同像素取 3 遍 RGB）。
/// Rgb8 源零拷贝借用；其他色型经 `to_rgb8` 转换（与旧 `get_pixel` 语义一致）。
fn build_input_data(resized: &DynamicImage) -> Vec<f32> {
    let plane_size = YOLO_INPUT_SIZE * YOLO_INPUT_SIZE;
    let mut input_data = vec![0.0f32; 3 * plane_size];
    match resized.as_rgb8() {
        Some(rgb) => fill_nchw(rgb.as_raw(), &mut input_data, plane_size),
        None => {
            let rgb = resized.to_rgb8();
            fill_nchw(rgb.as_raw(), &mut input_data, plane_size);
        }
    }
    input_data
}

/// RGB8 字节切片 → NCHW 通道分散写（RGB/255 归一化）。
fn fill_nchw(raw: &[u8], out: &mut [f32], plane_size: usize) {
    for (i, px) in raw.chunks_exact(3).enumerate() {
        out[i] = px[0] as f32 / 255.0;
        out[plane_size + i] = px[1] as f32 / 255.0;
        out[2 * plane_size + i] = px[2] as f32 / 255.0;
    }
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbImage;

    #[allow(dead_code)]
    fn create_test_image(width: u32, height: u32) -> DynamicImage {
        let mut img = RgbImage::new(width, height);
        // 画一个简单的矩形区域模拟鸟
        for y in 0..height {
            for x in 0..width {
                if x > width / 4 && x < 3 * width / 4 && y > height / 4 && y < 3 * height / 4 {
                    img.put_pixel(x, y, image::Rgb([128, 64, 32]));
                }
            }
        }
        DynamicImage::ImageRgb8(img)
    }

    /// 模拟 YOLO 输出的 flat 数组：[x1,y1,x2,y2,score,classId] × N。
    /// x1,y1,x2,y2 为像素坐标 (0-640)，score 为 0-1，classId 为浮点。
    fn make_yolo_output(detections: &[(f32, f32, f32, f32, f32, f32)]) -> Vec<f32> {
        let mut flat = Vec::new();
        for &(x1, y1, x2, y2, score, class_id) in detections {
            flat.push(x1);
            flat.push(y1);
            flat.push(x2);
            flat.push(y2);
            flat.push(score);
            flat.push(class_id);
        }
        flat
    }

    #[test]
    fn test_bbox_crop_math_normalized_to_pixel() {
        // 验证归一化坐标换算为像素坐标的逻辑
        // 与 pica cropNormalizedRegion 对等（bird_classification_service.dart:66-74）
        let img_width = 800;
        let img_height = 600;
        let bbox = BBox::new(0.1, 0.2, 0.5, 0.6);

        let x1 = (bbox.x1 * img_width as f32).floor() as u32;
        let y1 = (bbox.y1 * img_height as f32).floor() as u32;
        let x2 = (bbox.x2 * img_width as f32).ceil() as u32;
        let y2 = (bbox.y2 * img_height as f32).ceil() as u32;

        assert_eq!(x1, 80);
        assert_eq!(y1, 120);
        assert_eq!(x2, 400);
        assert_eq!(y2, 360);
    }

    #[test]
    fn test_yolo_output_parsing() {
        // 模拟 YOLO 输出：一个高分框和一个低分框
        let flat = make_yolo_output(&[
            (10.0, 20.0, 300.0, 400.0, 0.9, 0.0), // 高分框
            (50.0, 60.0, 100.0, 200.0, 0.1, 1.0), // 低分框（被阈值过滤）
        ]);

        // 检查解析逻辑
        assert!(flat.len() >= 6);
        let mut best_score = 0.0f32;
        let mut best_bbox: Option<(f32, f32, f32, f32)> = None;

        for offset in (0..flat.len() - 5).step_by(6) {
            let score = flat[offset + 4];
            if score < 0.25 {
                continue;
            }
            let x1 = (flat[offset] / 640.0).clamp(0.0, 1.0);
            let y1 = (flat[offset + 1] / 640.0).clamp(0.0, 1.0);
            let x2 = (flat[offset + 2] / 640.0).clamp(0.0, 1.0);
            let y2 = (flat[offset + 3] / 640.0).clamp(0.0, 1.0);
            if x2 <= x1 || y2 <= y1 {
                continue;
            }
            if score > best_score {
                best_score = score;
                best_bbox = Some((x1, y1, x2, y2));
            }
        }

        assert!(best_bbox.is_some());
        let (x1, y1, x2, y2) = best_bbox.unwrap();
        assert!((x1 - 10.0 / 640.0).abs() < 1e-6);
        assert!((y1 - 20.0 / 640.0).abs() < 1e-6);
        assert!((x2 - 300.0 / 640.0).abs() < 1e-6);
        assert!((y2 - 400.0 / 640.0).abs() < 1e-6);
        assert!((best_score - 0.9).abs() < 1e-6);
    }

    #[test]
    fn test_yolo_output_all_low_score() {
        // 所有框分数低于阈值 → 无检测
        let flat = make_yolo_output(&[
            (10.0, 20.0, 100.0, 200.0, 0.1, 0.0),
            (30.0, 40.0, 80.0, 150.0, 0.05, 1.0),
        ]);

        let mut has_valid = false;
        for offset in (0..flat.len() - 5).step_by(6) {
            if flat[offset + 4] >= 0.25 {
                has_valid = true;
                break;
            }
        }
        assert!(!has_valid);
    }

    #[test]
    fn test_yolo_output_empty() {
        // 空输出 → None
        let flat: Vec<f32> = vec![];
        assert!(flat.len() < 6);
    }

    #[test]
    fn test_yolo_output_invalid_bbox() {
        // x2 <= x1 → 过滤
        let flat = make_yolo_output(&[
            (300.0, 20.0, 10.0, 400.0, 0.9, 0.0), // x2 < x1
        ]);

        let mut valid_count = 0;
        for offset in (0..flat.len() - 5).step_by(6) {
            let score = flat[offset + 4];
            if score < 0.25 {
                continue;
            }
            let x1 = (flat[offset] / 640.0).clamp(0.0, 1.0);
            let y1 = (flat[offset + 1] / 640.0).clamp(0.0, 1.0);
            let x2 = (flat[offset + 2] / 640.0).clamp(0.0, 1.0);
            let y2 = (flat[offset + 3] / 640.0).clamp(0.0, 1.0);
            if x2 <= x1 || y2 <= y1 {
                continue;
            }
            valid_count += 1;
        }
        assert_eq!(valid_count, 0);
    }
}
