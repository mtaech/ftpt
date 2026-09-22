//! 主体检测（`org_det.onnx` = Ultralytics YOLOE-26s-seg，开放词表通用检测）：
//! 640×640 CatmullRom 缩放 → RGB/255 归一化 NCHW → ONNX 推理 →
//! 解析 `[x1,y1,x2,y2, conf, cls_id, 32×掩码系数] × 300` → 最高分框（conf ≥ 0.25）。
//!
//! ## 输出布局（实测确定，非推测）
//!
//! 由 `examples/debug_org_det.rs` 在真实照片上测得（2026-09-22）：
//! output0 `[1,300,38]` 是 **300 个固定槽位**，每行
//! `ch0..3 = 像素框（640 空间）`、`ch4 = 置信度`、`ch5 = 类别 id`、
//! `ch6..38 = 32 个掩码系数`（与 output1 `[1,32,160,160]` 的掩码原型配套）。
//! 空槽位整行为 0。图内已完成 NMS（导出参数 `nms=True`），本模块不做 NMS。
//! 掩码系数当前不消费——管线只需要主体框（抠图/去背景是后续可能的增强）。
//!
//! ## 为什么不是单类鸟检测器
//!
//! 19 个开放词表类别是**生物界粗词**（animal / bird / plant / flower / mushroom / …），
//! 所以花、虫、菌这类非鸟类主体也能出框。实测（`examples/detect_ab.rs`）：
//! 花卉照片上原单类鸟检测器 `detect.onnx` 一个框都不给，org_det 给出
//! `flower 0.78`；鸟类照片上两者框位基本重合（6/7 张检出；1 张漏检退化为整图识别）。

use image::DynamicImage;
use ort::session::Session;
use ort::value::Tensor;

use photo_domain::BBox;

use crate::RecognizeError;

/// 检测模型文件名（随包分发，见 AGENTS「识别资产」）
pub const MODEL_FILE: &str = "org_det.onnx";

/// 模型输入尺寸
const INPUT_SIZE: usize = 640;

/// 分数阈值
const SCORE_THRESHOLD: f32 = 0.25;

/// 单行通道数 = 4（框）+ 1（conf）+ 1（cls）+ 32（掩码系数）
const ROW_LEN: usize = 38;

/// 近满幅候选的面积占比下限：这类框几乎总是背景（树冠/天空/整片植物），
/// 当图内还有更小的主体框时优先取后者，避免「树 0.62」把鸟 0.68 之外的场景抢走
/// （实测 小黑领噪鹛：整幅 tree 0.617 vs bird 0.681，只差 0.06）。
const BACKGROUND_AREA_RATIO: f32 = 0.9;

/// 开放词表类别名（顺序即模型 `names` 元数据 0..18）
pub const CLASS_NAMES: [&str; 19] = [
    "animal", "bird", "mammal", "reptile", "amphibian", "fish", "insect", "spider",
    "crustacean", "mollusk", "worm", "plant", "flower", "tree", "leaf", "fruit", "grass",
    "mushroom", "lichen",
];

/// 检测结果
#[derive(Debug, Clone)]
pub struct DetectionResult {
    /// 归一化边界框 (0-1)
    pub bbox: BBox,
    /// 原始检测分数 (0-1，非百分制)
    pub raw_score: f32,
    /// 开放词表类别名（诊断用：能看出这段框是花还是鸟）
    pub class_name: &'static str,
}

/// 一个候选框（解析后的中间形态，纯数据便于单测）
#[derive(Debug, Clone, PartialEq)]
struct Candidate {
    bbox: BBox,
    score: f32,
    class_id: usize,
}

/// 640×640 CatmullRom 缩放（检测模型输入）。
pub(crate) fn resize_to_yolo_input(img: &DynamicImage) -> DynamicImage {
    img.resize_exact(
        INPUT_SIZE as u32,
        INPUT_SIZE as u32,
        image::imageops::FilterType::CatmullRom,
    )
}

/// 使用已完成缩放的 640×640 图运行检测（管线共享缩放结果时调用）。
///
/// `Ok(Some(result))` 检测成功，`Ok(None)` 无有效检测，`Err(...)` 系统故障。
pub(crate) fn run_yolo_detection_resized(
    session: &mut Session,
    resized: &DynamicImage,
) -> Result<Option<DetectionResult>, RecognizeError> {
    let input_data = build_input_data(resized);
    let tensor = Tensor::<f32>::from_array((
        [1usize, 3, INPUT_SIZE, INPUT_SIZE],
        input_data.into_boxed_slice(),
    ))?;

    let outputs = session.run(ort::inputs![tensor])?;
    // 模型异常防护：无输出时返回系统错误而非裸索引 panic
    let output = if outputs.len() == 0 {
        return Err(RecognizeError::ModelLoad("检测模型推理无输出".into()));
    } else {
        &outputs[0]
    };

    let (_shape, flat) = output.try_extract_tensor::<f32>()?;
    Ok(pick_best(&parse_candidates(flat)))
}

/// 扁平输出 → 候选列表（纯函数）。
///
/// 空槽位（conf 为 0）、低于阈值、退化为零面积/反向框的槽位全部丢弃。
fn parse_candidates(flat: &[f32]) -> Vec<Candidate> {
    let mut out = Vec::new();
    for row in flat.chunks_exact(ROW_LEN) {
        let score = row[4];
        if !score.is_finite() || score < SCORE_THRESHOLD {
            continue;
        }
        // 像素坐标 → 归一化；模型在贴边处会给出 -5.25 / 640.64 这类越界值，统一夹紧
        let x1 = (row[0] / INPUT_SIZE as f32).clamp(0.0, 1.0);
        let y1 = (row[1] / INPUT_SIZE as f32).clamp(0.0, 1.0);
        let x2 = (row[2] / INPUT_SIZE as f32).clamp(0.0, 1.0);
        let y2 = (row[3] / INPUT_SIZE as f32).clamp(0.0, 1.0);
        if x2 <= x1 || y2 <= y1 {
            continue;
        }
        let class_id = row[5].max(0.0) as usize;
        out.push(Candidate {
            bbox: BBox::new(x1, y1, x2, y2),
            score,
            class_id,
        });
    }
    out
}

/// 候选 → 最终结论（纯函数）：最高分，但近满幅背景框让位于更小的主体框。
///
/// 若所有候选都是近满幅（例如只有「整片树冠」一个框），仍返回最高分者——
/// 此时等价于整图识别，比什么都不给更接近可用结果。
fn pick_best(candidates: &[Candidate]) -> Option<DetectionResult> {
    let to_result = |c: &Candidate| DetectionResult {
        bbox: c.bbox,
        raw_score: c.score,
        class_name: CLASS_NAMES.get(c.class_id).copied().unwrap_or("unknown"),
    };
    let area = |c: &Candidate| {
        (c.bbox.x2 - c.bbox.x1).max(0.0) * (c.bbox.y2 - c.bbox.y1).max(0.0)
    };
    let subjects: Vec<&Candidate> = candidates
        .iter()
        .filter(|c| area(c) < BACKGROUND_AREA_RATIO)
        .collect();
    let pool = if subjects.is_empty() {
        candidates.iter().collect::<Vec<_>>()
    } else {
        subjects
    };
    pool.into_iter()
        .max_by(|a, b| a.score.partial_cmp(&b.score).unwrap_or(std::cmp::Ordering::Equal))
        .map(to_result)
}

/// 640×640 RGB/255 归一化 NCHW 预处理。
///
/// 单遍遍历 `as_raw()` 像素按通道分散写，替代三重循环 `get_pixel`。
fn build_input_data(resized: &DynamicImage) -> Vec<f32> {
    let plane_size = INPUT_SIZE * INPUT_SIZE;
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

    /// 造一行 38 通道：[x1,y1,x2,y2, conf, cls, 32×系数]
    fn row(x1: f32, y1: f32, x2: f32, y2: f32, conf: f32, cls: f32) -> Vec<f32> {
        let mut r = vec![x1, y1, x2, y2, conf, cls];
        r.extend(std::iter::repeat(0.0).take(32));
        r
    }

    #[test]
    fn test_parse_skips_padding_low_score_and_invalid_box() {
        let mut flat = Vec::new();
        flat.extend(row(10.0, 20.0, 300.0, 400.0, 0.9, 1.0)); // 有效
        flat.extend(row(0.0, 0.0, 0.0, 0.0, 0.0, 0.0)); // 空槽位（全 0）
        flat.extend(row(50.0, 60.0, 100.0, 200.0, 0.1, 2.0)); // 低于阈值
        flat.extend(row(300.0, 20.0, 10.0, 400.0, 0.8, 3.0)); // x2 < x1
        let c = parse_candidates(&flat);
        assert_eq!(c.len(), 1);
        assert!((c[0].bbox.x1 - 10.0 / 640.0).abs() < 1e-6);
        assert!((c[0].bbox.y2 - 400.0 / 640.0).abs() < 1e-6);
        assert_eq!(c[0].class_id, 1);
        assert!((c[0].score - 0.9).abs() < 1e-6);
    }

    #[test]
    fn test_parse_clamps_out_of_range_pixels() {
        // 贴边检测会出现负坐标/超出 640 的坐标（实测 -5.25 / 640.64）
        let flat = row(-5.25, -0.01, 640.64, 637.44, 0.5, 13.0);
        let c = parse_candidates(&flat);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].bbox.x1, 0.0);
        assert_eq!(c[0].bbox.x2, 1.0);
        assert_eq!(c[0].bbox.y2, 637.44 / 640.0);
    }

    #[test]
    fn test_parse_short_output_is_empty() {
        assert!(parse_candidates(&[]).is_empty());
        assert!(parse_candidates(&[1.0, 2.0, 3.0]).is_empty());
    }

    #[test]
    fn test_pick_best_prefers_subject_over_full_frame_background() {
        // 实测场景：鸟 0.681（小框）vs 整幅树 0.617。按分数直觉会选树，
        // 但整幅框是背景，必须让位给真正的主体
        let cands = vec![
            Candidate { bbox: BBox::new(0.307, 0.377, 0.606, 0.886), score: 0.681, class_id: 1 },
            Candidate { bbox: BBox::new(0.0, 0.001, 0.994, 1.0), score: 0.617, class_id: 13 },
        ];
        let best = pick_best(&cands).unwrap();
        assert_eq!(best.class_name, "bird");
        assert!((best.raw_score - 0.681).abs() < 1e-6);
    }

    #[test]
    fn test_pick_best_keeps_full_frame_when_only_candidate() {
        // 只有整幅背景框时仍返回它（等价整图识别，好过什么都不给）
        let cands = vec![Candidate {
            bbox: BBox::new(0.005, 0.002, 0.999, 0.998),
            score: 0.273,
            class_id: 13,
        }];
        let best = pick_best(&cands).unwrap();
        assert_eq!(best.class_name, "tree");
    }

    #[test]
    fn test_pick_best_highest_score_among_subjects_and_unknown_class() {
        let cands = vec![
            Candidate { bbox: BBox::new(0.1, 0.1, 0.4, 0.4), score: 0.5, class_id: 99 },
            Candidate { bbox: BBox::new(0.5, 0.5, 0.8, 0.8), score: 0.7, class_id: 12 },
        ];
        let best = pick_best(&cands).unwrap();
        assert_eq!(best.class_name, "flower");
        assert!(pick_best(&[]).is_none());
    }
}
