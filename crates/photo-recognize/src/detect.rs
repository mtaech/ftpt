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

use std::cmp::Ordering;

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

/// 多主体上限：一张照片最多保留几个主体（控推理成本；org_det 每框一次 BioCLIP）
const MAX_SUBJECTS: usize = 8;

/// 去重 IoU 阈值：与已保留框重叠超过该值的候选视为同一主体（org_det 已 NMS，跨类重复少见）
const DUP_IOU_THRESHOLD: f32 = 0.5;

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
/// 返回**多个**候选主体（已去背景 / 去重 / 限数）；`Ok(vec![])` = 无有效检测，
/// `Err(...)` 系统故障。
pub(crate) fn run_yolo_detection_resized(
    session: &mut Session,
    resized: &DynamicImage,
) -> Result<Vec<DetectionResult>, RecognizeError> {
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
    Ok(pick_subjects(&parse_candidates(flat)))
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

/// 候选 → 多主体结论（纯函数）。
///
/// 规则：
/// 1. 近满幅背景框（树冠/天空/整片植物）在存在更小主体框时剔除；
///    只有背景时退化为单主体（整幅 ≈ 整图识别）。
/// 2. 按置信度降序贪心选取，与已保留框 IoU ≥ 阈值视为同一主体跳过。
/// 3. 最多保留 [`MAX_SUBJECTS`] 个（每框一次 BioCLIP，控推理成本）。
fn pick_subjects(candidates: &[Candidate]) -> Vec<DetectionResult> {
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
    let mut pool = pool;
    pool.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal));

    let mut kept: Vec<&Candidate> = Vec::new();
    for c in pool {
        if kept.len() >= MAX_SUBJECTS {
            break;
        }
        if kept.iter().any(|k| iou(&k.bbox, &c.bbox) >= DUP_IOU_THRESHOLD) {
            continue;
        }
        kept.push(c);
    }
    kept.into_iter().map(to_result).collect()
}

/// 交叉验证（docs/todo.md #16，规则搬自 bioclip_demo/src/detect.rs）：
/// 检测器声称的**粗类群**与 BioCLIP 认出的**细类群**是否自洽。
///
/// 不自洽说明这个框是误检——例如花朵被误检成鸟，裁出来 BioCLIP 说是植物。
/// 检测器只是我们的前处理，拿不准的框宁可丢掉：丢掉后会退回整图识别，
/// 不会把整张图的结果一起丢（见 pipeline 的兜底）。
///
/// ranks 是七级分类 [界, 门, 纲, 目, 科, 属, 种]，空串 = 该级缺。
/// **认不出的粗类名一律放行**（不拿不确定当证据丢框）。
/// 与 bioclip_demo 的一处**有意差别**：那边标签路径永远来自标签空间，而本仓的
/// 名录兜底路径会产出空 ranks（catalog.rs 的 TaxonMatch.ranks = vec![]）。
/// 没有分类证据时一律放行——否则凡是走名录兜底的主体都会被误判成误检丢掉。
pub fn class_is_consistent(det_class: &str, ranks: &[String]) -> bool {
    if ranks.iter().all(|s| s.is_empty()) {
        return true;
    }
    let get = |i: usize| ranks.get(i).map_or("", String::as_str);
    let (kingdom, phylum, class) = (get(0), get(1), get(2));
    match det_class {
        "animal" => kingdom == "Animalia",
        "bird" => class == "Aves",
        "mammal" => class == "Mammalia",
        "reptile" => matches!(
            class,
            "Reptilia" | "Squamata" | "Testudines" | "Crocodylia" | "Rhynchocephalia"
        ),
        "amphibian" => class == "Amphibia",
        "fish" => matches!(
            class,
            "Actinopterygii"
                | "Chondrichthyes"
                | "Sarcopterygii"
                | "Myxini"
                | "Petromyzontida"
                | "Cephalaspidomorphi"
        ),
        "insect" => class == "Insecta",
        "spider" => class == "Arachnida",
        "crustacean" => matches!(
            class,
            "Malacostraca" | "Branchiopoda" | "Maxillopoda" | "Ostracoda" | "Remipedia"
        ),
        "mollusk" => phylum == "Mollusca",
        // 环节/扁形/线虫分散在好几个门，没法用一条规则收；认不出具体门时放行
        "worm" => matches!(
            phylum,
            "" | "Annelida" | "Platyhelminthes" | "Nematoda" | "Sipuncula" | "Echiura"
        ),
        "plant" | "flower" | "tree" | "leaf" | "fruit" | "grass" => {
            matches!(kingdom, "Plantae" | "Archaeplastida")
        }
        // 地衣是真菌与藻类的共生体，名录里归在真菌界
        "mushroom" | "lichen" => kingdom == "Fungi",
        _ => true,
    }
}

/// 两个归一化框的 IoU（0..1）。
fn iou(a: &BBox, b: &BBox) -> f32 {
    let ix = (a.x2.min(b.x2) - a.x1.max(b.x1)).max(0.0);
    let iy = (a.y2.min(b.y2) - a.y1.max(b.y1)).max(0.0);
    let inter = ix * iy;
    let area_a = ((a.x2 - a.x1) * (a.y2 - a.y1)).max(0.0);
    let area_b = ((b.x2 - b.x1) * (b.y2 - b.y1)).max(0.0);
    inter / (area_a + area_b - inter).max(f32::MIN_POSITIVE)
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

    fn ranks(kingdom: &str, phylum: &str, class: &str) -> Vec<String> {
        [kingdom, phylum, class, "", "", "", ""]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    #[test]
    fn test_class_is_consistent_accepts_matching_kinds() {
        let bird = ranks("Animalia", "Chordata", "Aves");
        assert!(class_is_consistent("bird", &bird));
        assert!(class_is_consistent("animal", &bird));
        let plant = ranks("Plantae", "Tracheophyta", "Magnoliopsida");
        assert!(class_is_consistent("flower", &plant));
        assert!(class_is_consistent("tree", &plant));
        let mushroom = ranks("Fungi", "Basidiomycota", "Agaricomycetes");
        assert!(class_is_consistent("mushroom", &mushroom));
        assert!(class_is_consistent("lichen", &mushroom));
    }

    #[test]
    fn test_class_is_consistent_rejects_mismatched_kinds() {
        // 花朵被误检成鸟：检测器说 bird、分类器说是植物 → 误检，丢掉
        let plant = ranks("Plantae", "Tracheophyta", "Magnoliopsida");
        assert!(!class_is_consistent("bird", &plant));
        assert!(!class_is_consistent("mammal", &plant));
        // 反过来同样成立：鸟被误检成花
        let bird = ranks("Animalia", "Chordata", "Aves");
        assert!(!class_is_consistent("flower", &bird));
        assert!(!class_is_consistent("insect", &bird));
    }

    #[test]
    fn test_class_is_consistent_passes_unknown_and_missing_ranks() {
        // 防御：认不出的粗类名、空 ranks 都不该被当成「不自洽」丢框
        assert!(class_is_consistent("unknown", &ranks("Plantae", "", "")));
        assert!(class_is_consistent("worm", &ranks("", "", "")));
        assert!(class_is_consistent("animal", &[]));
        assert!(class_is_consistent("bird", &[]));
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
    fn test_pick_subjects_drops_full_frame_background() {
        // 实测场景：鸟 0.681（小框）vs 整幅树 0.617 → 只留鸟
        let cands = vec![
            Candidate { bbox: BBox::new(0.307, 0.377, 0.606, 0.886), score: 0.681, class_id: 1 },
            Candidate { bbox: BBox::new(0.0, 0.001, 0.994, 1.0), score: 0.617, class_id: 13 },
        ];
        let out = pick_subjects(&cands);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].class_name, "bird");
        assert!((out[0].raw_score - 0.681).abs() < 1e-6);
    }

    #[test]
    fn test_pick_subjects_keeps_full_frame_when_only_candidate() {
        // 只有整幅背景框时仍返回它（等价整图识别，好过什么都不给）
        let cands = vec![Candidate {
            bbox: BBox::new(0.005, 0.002, 0.999, 0.998),
            score: 0.273,
            class_id: 13,
        }];
        let out = pick_subjects(&cands);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].class_name, "tree");
    }

    #[test]
    fn test_pick_subjects_multi_sorted_by_score() {
        // 多个主体框：按置信度降序返回
        let cands = vec![
            Candidate { bbox: BBox::new(0.1, 0.1, 0.3, 0.3), score: 0.5, class_id: 1 },
            Candidate { bbox: BBox::new(0.6, 0.6, 0.9, 0.9), score: 0.7, class_id: 12 },
            Candidate { bbox: BBox::new(0.05, 0.6, 0.2, 0.85), score: 0.4, class_id: 6 },
        ];
        let out = pick_subjects(&cands);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].class_name, "flower");
        assert_eq!(out[1].class_name, "bird");
        assert_eq!(out[2].class_name, "insect");
    }

    #[test]
    fn test_pick_subjects_dedup_overlapping_boxes() {
        // 高度重叠的两个框（同一主体被出两次）→ 去重只留高分者
        let cands = vec![
            Candidate { bbox: BBox::new(0.1, 0.1, 0.4, 0.4), score: 0.9, class_id: 1 },
            Candidate { bbox: BBox::new(0.12, 0.12, 0.42, 0.42), score: 0.8, class_id: 0 },
        ];
        let out = pick_subjects(&cands);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].raw_score, 0.9);
    }

    #[test]
    fn test_pick_subjects_caps_at_max_and_empty() {
        // 12 个互不重叠的框 → 截断到 MAX_SUBJECTS
        let mut cands = Vec::new();
        for i in 0..12u32 {
            let x = (i % 4) as f32 * 0.2 + 0.05;
            let y = (i / 4) as f32 * 0.2 + 0.05;
            cands.push(Candidate {
                bbox: BBox::new(x, y, x + 0.1, y + 0.1),
                score: 0.9 - i as f32 * 0.01,
                class_id: 1,
            });
        }
        let out = pick_subjects(&cands);
        assert_eq!(out.len(), MAX_SUBJECTS);
        assert!(pick_subjects(&[]).is_empty());
    }
}
