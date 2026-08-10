//! 鸟眼检测：整图跑 eye.onnx → 定位眼关键点。
//!
//! 管线位置：第四阶段「鸟眼锐度」——在识别管线得到鸟框之后运行。
//!
//! ## 模型输出（YOLO26 系端到端姿态头，NMS-free）
//!
//! 输出张量 `[1, 300, 12]`，每行：
//! `[x1, y1, x2, y2(像素), conf, cls, kpt1(x, y, conf), kpt2(x, y, conf)]`
//! ——鸟框恒为整幅（训练集即整幅鸟图），实际只用两个眼关键点。
//!
//! ## 输入与选点策略（2026-07-30 标定集实证，34 张真实鸟片回放）
//!
//! - **输入为整图**，不做鸟框裁剪：模型训练集是整幅鸟图，裁剪会破坏其空间先验——
//!   实测裁剪输入下被遮挡眼会被幻觉到头顶/脸颊且置信度虚高（0.95+），压过真眼。
//! - 两个关键点槽位 = 左/右眼。正脸双眼分立均有效；侧脸时被遮眼为幻觉点。
//!   没有任何单一槽位恒真，取全局最高置信必选错（3.05 案例：头顶 0.957 > 真眼 0.848）。
//! - **采信判据 = 双槽一致性**：两槽各自最优关键点均 ≥ 阈值且归一化间距 ≤ 0.15
//!   时取高置信点；否则返回 None（无眼/不确定 → 锐度 NULL 兑底，不误标错框）。

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

/// 双槽关键点一致性阈值：归一化间距上限（输入边长占比）。
/// 实证：正脸双眼间距 ≈0.14（5.14_P1080702），幻觉点对间距 ≥0.19。
const EYE_KPT_AGREEMENT_MAX_DIST: f32 = 0.15;

/// 眼框边长相对鸟框短边的比例（覆盖眼睛及周围羽毛供锐度统计）
const EYE_BOX_SIDE_RATIO: f32 = 0.15;

// ---------------------------------------------------------------------------
// 公共接口
// ---------------------------------------------------------------------------

/// 在整图上检测眼睛，返回全图归一化眼框。
///
/// # 参数
/// - `session`: eye.onnx session
/// - `img`: 全分辨率原始图像
/// - `bird_bbox`: 鸟体检测框（归一化 0-1，全图坐标），仅用于确定眼框边长
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
    detect_eye_impl(session, img, None, bird_bbox)
}

/// 复用检测阶段已完成的 640×640 CatmullRom 缩放（输入为
/// [`crate::detect::resize_to_yolo_input`] 的结果）。
///
/// 当 eye 模型输入恰为 640×640（或动态维度按 640 兜底）时直接复用，
/// 避免对同一张图做第二次全图缩放；模型输入为其他尺寸时自动回退内部
/// 按模型尺寸缩放（行为与 [`detect_eye`] 完全一致）。
pub(crate) fn detect_eye_shared(
    session: &mut Session,
    img: &DynamicImage,
    shared_640: &DynamicImage,
    bird_bbox: BBox,
) -> Result<Option<BBox>, RecognizeError> {
    detect_eye_impl(session, img, Some(shared_640), bird_bbox)
}

/// 眼检测共用实现。`shared_640` 为检测阶段的 640×640 缩放（可选复用）。
fn detect_eye_impl(
    session: &mut Session,
    img: &DynamicImage,
    shared_640: Option<&DynamicImage>,
    bird_bbox: BBox,
) -> Result<Option<BBox>, RecognizeError> {
    // ---- 从 session 元数据读取模型输入尺寸（不硬编码） ----
    // 模型异常防护：无输入时返回系统错误而非裸索引 panic
    let input = session
        .inputs()
        .first()
        .ok_or_else(|| RecognizeError::ModelLoad("eye.onnx 模型无输入".into()))?;
    let input_shape = input
        .dtype()
        .tensor_shape()
        .ok_or_else(|| RecognizeError::ModelLoad("eye.onnx 输入非张量类型".into()))?;

    // 输入恒为 NCHW [1, 3, H, W]（模型由 python 导出，通道在前），
    // 因此 [2]/[3] 维即 H/W；不存在 NHWC 布局的喂入路径。
    let (input_h, input_w) = if input_shape.len() >= 4 {
        (input_shape[2], input_shape[3])
    } else {
        return Err(RecognizeError::ModelLoad("eye.onnx 输入至少需要 4 维".into()));
    };

    // 动态维度（shape 值为 -1）按 640 兜底
    let input_h = if input_h <= 0 { 640usize } else { input_h as usize };
    let input_w = if input_w <= 0 { 640usize } else { input_w as usize };

    // ---- 预处理：整图 resize 至模型输入尺寸 + RGB/255 归一化 ----
    // 不裁剪鸟框：模型训练集为整幅鸟图，裁剪会破坏空间先验并放大幻觉点置信度。
    let (full_w, full_h) = img.dimensions();
    // 模型输入恰为 640×640 时复用检测阶段的共享缩放（同一 CatmullRom 插值，结果一致）
    let input_data = if let Some(shared) = shared_640
        && input_w == 640
        && input_h == 640
    {
        build_input_data(shared, input_w, input_h)
    } else {
        let resized = img.resize_exact(
            input_w as u32,
            input_h as u32,
            image::imageops::FilterType::CatmullRom,
        );
        build_input_data(&resized, input_w, input_h)
    };

    // ---- 推理 ----
    let tensor = Tensor::<f32>::from_array(
        ([1usize, 3, input_h, input_w], input_data.into_boxed_slice()),
    )?;

    // 模型异常防护：无输出时返回系统错误而非裸索引 panic
    let outputs = session.run(ort::inputs![tensor])?;
    // 模型异常防护：无输出时返回系统错误而非裸索引 panic
    let output = if outputs.len() == 0 {
        return Err(RecognizeError::ModelLoad("eye.onnx 模型推理无输出".into()));
    } else {
        &outputs[0]
    };

    // ---- 解析输出 ----
    // eye.onnx 是 YOLO26 系端到端姿态头（NMS-free），输出 [1, 300, 12]，
    // 每行 12 值：[x1,y1,x2,y2(像素), conf, cls, kpt1(x,y,conf), kpt2(x,y,conf)]。
    // 鸟框对本场景无用（训练集即整幅鸟图，框恒为整幅），只用两个眼关键点。
    let (_shape, flat) = output.try_extract_tensor::<f32>()?;
    let Some((nx, ny, _conf)) = pick_eye_keypoint(flat, input_w, input_h) else {
        return Ok(None);
    };

    // 整图输入：模型归一化坐标即全图归一化坐标，无需裁剪反映射。
    // 眼框边长 = 鸟框短边 × 比例（像素域构造再归一化，保证正方形）。
    let fw = full_w as f32;
    let fh = full_h as f32;
    let bird_short = ((bird_bbox.x2 - bird_bbox.x1) * fw).min((bird_bbox.y2 - bird_bbox.y1) * fh);
    let half_px = (bird_short * EYE_BOX_SIDE_RATIO / 2.0).max(1.0);

    let kx = nx * fw;
    let ky = ny * fh;

    Ok(Some(BBox::new(
        ((kx - half_px) / fw).clamp(0.0, 1.0),
        ((ky - half_px) / fh).clamp(0.0, 1.0),
        ((kx + half_px) / fw).clamp(0.0, 1.0),
        ((ky + half_px) / fh).clamp(0.0, 1.0),
    )))
}

/// RGB/255 归一化 NCHW 预处理（输入尺寸由模型元数据决定）。
///
/// 单遍遍历 `as_raw()` 像素按通道分散写，替代三重循环 `get_pixel`。
/// Rgb8 源零拷贝借用；其他色型经 `to_rgb8` 转换（与旧 `get_pixel` 语义一致）。
fn build_input_data(resized: &DynamicImage, input_w: usize, input_h: usize) -> Vec<f32> {
    let plane_size = input_w * input_h;
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

/// 双槽一致性选点：左/右眼关键点各自取全图最优（置信度最高），
/// 两者均过阈值且归一化间距 ≤ [`EYE_KPT_AGREEMENT_MAX_DIST`] 时取高置信点。
///
/// 侧脸照片中被遮挡的眼会被模型幻觉到头顶/脸颊且置信度虚高，
/// 单取全局最高置信必选错；双眼间距过大（含正脸以外的分散幻觉）时判不可信。
///
/// 输入为展平的 f32 输出（每 12 值一行），返回 (归一化 x, y, conf)。
fn pick_eye_keypoint(flat: &[f32], input_w: usize, input_h: usize) -> Option<(f32, f32, f32)> {
    if flat.len() < 12 || !flat.len().is_multiple_of(12) {
        return None;
    }
    let mut best: [Option<(f32, f32, f32)>; 2] = [None, None];
    for row in flat.chunks_exact(12) {
        for (slot, kpt) in [&row[6..9], &row[9..12]].into_iter().enumerate() {
            let (kx, ky, kc) = (kpt[0], kpt[1], kpt[2]);
            if kc < EYE_KPT_CONF_THRESHOLD {
                continue;
            }
            if best[slot].is_none_or(|b| kc > b.2) {
                best[slot] = Some((kx / input_w as f32, ky / input_h as f32, kc));
            }
        }
    }
    let (a, b) = (best[0]?, best[1]?);
    let dist = ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
    if dist > EYE_KPT_AGREEMENT_MAX_DIST {
        return None;
    }
    Some(if a.2 >= b.2 { a } else { b })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 双槽一致且均过阈值 → 取高置信点
    #[test]
    fn test_pick_eye_keypoint_agreement_picks_higher_conf() {
        let row = |k1: (f32, f32, f32), k2: (f32, f32, f32)| -> Vec<f32> {
            vec![
                0.0, 0.0, 640.0, 640.0, 0.9, 0.0, k1.0, k1.1, k1.2, k2.0, k2.1, k2.2,
            ]
        };
        // 两槽最优来自相近位置（间距 ~6px），k2 置信更高
        let mut flat = row((100.0, 100.0, 0.6), (104.0, 103.0, 0.9));
        flat.extend(row((98.0, 102.0, 0.7), (50.0, 50.0, 0.55)));

        let (nx, ny, conf) = pick_eye_keypoint(&flat, 640, 640).unwrap();
        assert!((conf - 0.9).abs() < 1e-6);
        assert!((nx - 104.0 / 640.0).abs() < 1e-6);
        assert!((ny - 103.0 / 640.0).abs() < 1e-6);
    }

    /// 幻觉点压过真眼（3.05 案例）：单槽高置信但双槽分散 → None
    #[test]
    fn test_pick_eye_keypoint_rejects_hallucinated_slot() {
        let row = |k1: (f32, f32, f32), k2: (f32, f32, f32)| -> Vec<f32> {
            vec![
                0.0, 0.0, 640.0, 640.0, 0.9, 0.0, k1.0, k1.1, k1.2, k2.0, k2.1, k2.2,
            ]
        };
        // k1 真眼 (348,190)@0.85；k2 幻觉头顶 (229,106)@0.96，间距 ~150px
        let flat = row((348.0, 190.0, 0.85), (229.0, 106.0, 0.96));
        assert!(pick_eye_keypoint(&flat, 640, 640).is_none());
    }

    /// 正脸双眼分立（间距 ≈0.14）仍采信
    #[test]
    fn test_pick_eye_keypoint_frontal_two_eyes_accepted() {
        let flat = vec![
            0.0, 0.0, 640.0, 640.0, 0.9, 0.0, 357.0, 336.0, 0.89, 267.0, 336.0, 0.91,
        ];
        let (nx, _ny, conf) = pick_eye_keypoint(&flat, 640, 640).unwrap();
        assert!((conf - 0.91).abs() < 1e-6);
        assert!((nx - 267.0 / 640.0).abs() < 1e-6);
    }

    /// 只有一个槽位过阈值 → None（实证：单槽通过样本全为误检）
    #[test]
    fn test_pick_eye_keypoint_single_slot_rejected() {
        let flat = vec![
            0.0, 0.0, 640.0, 640.0, 0.9, 0.0, 100.0, 100.0, 0.8, 200.0, 200.0, 0.49,
        ];
        assert!(pick_eye_keypoint(&flat, 640, 640).is_none());
    }

    /// 全部关键点低于阈值 → None
    #[test]
    fn test_pick_eye_keypoint_below_threshold() {
        let flat = vec![
            0.0, 0.0, 640.0, 640.0, 0.9, 0.0, 100.0, 100.0, 0.3, 200.0, 200.0, 0.49,
        ];
        assert!(pick_eye_keypoint(&flat, 640, 640).is_none());
    }

    /// 长度非法（非 12 的倍数 / 空）→ None 而非 panic
    #[test]
    fn test_pick_eye_keypoint_malformed_input() {
        assert!(pick_eye_keypoint(&[], 640, 640).is_none());
        assert!(pick_eye_keypoint(&[1.0; 6], 640, 640).is_none());
        assert!(pick_eye_keypoint(&[1.0; 13], 640, 640).is_none());
    }
}
