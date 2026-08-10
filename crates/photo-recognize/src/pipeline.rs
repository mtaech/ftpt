//! 识别管线编排：检测 → 分类 → 名录映射，带进度回调。
//!
//! 对应 pica `recognition_pipeline_service.dart`。
//!
//! ## 失败阶段 → 状态映射（pica recognition_pipeline_service.dart）
//!
//! | 阶段 | 状态 | 说明 |
//! |------|------|------|
//! | 检测无框 | `Unrecognized`(Detection) | 未检测到鸟类目标 |
//! | 分类/解码异常 | `NeedsReview`(Classification) | 模型推理或解析异常 |
//! | 映射失败 | `NeedsReview`(Mapping) | 无映射或歧义映射 |
//! | 源文件不可用 | `NeedsReview`(Assets) | 图片文件不存在/无法解码 |
//! | 全部成功 | `Confirmed`(None) | 检测→分类→映射均成功 |

use image::{DynamicImage, GenericImageView};
use ort::session::Session;

use photo_domain::{BBox, Capture, Recognition, RecognitionFailureStage, RecognitionStatus};

use crate::catalog::{CatalogDb, ClassificationOutput};
use crate::detect;
use crate::eye;
use crate::sharpness;
use crate::RecognizeError;

/// 进度回调：进度 0.0-1.0 + 阶段文本
pub type ProgressCallback = dyn Fn(RecognitionProgress) + Send;

/// 识别进度信息
#[derive(Debug, Clone)]
pub struct RecognitionProgress {
    /// 进度值 (0.0-1.0)
    pub value: f32,
    /// 中文阶段描述
    pub stage: &'static str,
}

/// 输入源解析结果
pub(crate) enum ResolvedSource {
    /// 直接解码得到的图像
    Image(DynamicImage),
}

/// 识别源图长边上限（px）。检测/眼/分类输入为 640/640/224，2048 源图足够；
/// 超限缩放一次可显著降低两次全图缩放（20MP → 640 CatmullRom）的代价。
/// 代价：小鸟（<11% 画面）的分类裁切源分辨率下降，识别精度轻微让步于速度。
const SOURCE_MAX_SIDE: u32 = 2048;

/// 超限源图缩放到 [`SOURCE_MAX_SIDE`]，否则原样返回。
fn cap_source_size(img: DynamicImage) -> DynamicImage {
    let (w, h) = img.dimensions();
    let max_side = w.max(h);
    if max_side <= SOURCE_MAX_SIDE {
        return img;
    }
    let scale = SOURCE_MAX_SIDE as f32 / max_side as f32;
    let nw = ((w as f32 * scale).round() as u32).max(1);
    let nh = ((h as f32 * scale).round() as u32).max(1);
    img.resize_exact(nw, nh, image::imageops::FilterType::Lanczos3)
}

/// 解析输入源（带可选内存缩略图）。
///
/// `thumb_bytes` 非空时优先从内存解码——跳过全图 `image::open`
/// （24MP 全量解码 100-300ms/张；缩略图为 app 侧 `.pt/thumbs` 磁盘缓存或
/// 预览缓存派生图字节）。解码失败时回落到完整路径。
///
/// 完整路径策略（与 pica `_resolveSourcePath` 对等, recognition_pipeline_service.dart:156-173）：
/// 1. 取主显示文件路径
/// 2. JPEG/PNG 等标准格式 → `image::open` 直接解码
/// 3. RAW 格式 → `photo_engine::thumbnail::decode_raw_preview` 提取内嵌 JPEG 预览
/// 4. 均失败 → `NeedsReview(Assets)`
pub(crate) fn resolve_source_with_thumbnail(
    capture: &Capture,
    thumb_bytes: Option<&[u8]>,
) -> Result<ResolvedSource, (RecognitionFailureStage, &'static str)> {
    resolve_source_opt(capture, thumb_bytes)
}

/// 输入源解析共用实现（`thumb_bytes` 为可选缩略图优先路径）。
fn resolve_source_opt(
    capture: &Capture,
    thumb_bytes: Option<&[u8]>,
) -> Result<ResolvedSource, (RecognitionFailureStage, &'static str)> {
    // 缩略图优先：内存解码成功即用（仍需超限截幅），失败静默回退完整路径
    if let Some(bytes) = thumb_bytes
        && !bytes.is_empty()
    {
        match image::load_from_memory(bytes) {
            Ok(img) => return Ok(ResolvedSource::Image(cap_source_size(img))),
            Err(e) => {
                tracing::warn!("[识别] 缩略图字节解码失败，回退完整解码: {e}");
            }
        }
    }

    let primary = &capture.source_files[capture.primary_index];
    let path = &primary.path;

    // 检查文件是否存在
    if !path.exists() {
        return Err((RecognitionFailureStage::Assets, "识别源文件不存在"));
    }

    match &primary.format {
        // 标准格式直接解码（pica recognition_pipeline_service.dart:167-170）
        photo_domain::ImageFormat::Jpeg
        | photo_domain::ImageFormat::Png
        | photo_domain::ImageFormat::Tiff
        | photo_domain::ImageFormat::Heif
        | photo_domain::ImageFormat::WebP
        | photo_domain::ImageFormat::Bmp
        | photo_domain::ImageFormat::Gif => match image::open(path) {
            Ok(img) => Ok(ResolvedSource::Image(cap_source_size(img))),
            Err(e) => {
                tracing::warn!("标准图片解码失败: {} — {e}", path.display());
                Err((RecognitionFailureStage::Assets, "无法解码图片"))
            }
        },
        // RAW 格式提取内嵌预览（与 pica 不同：pica 直接用 image 解码失败后退出，这里尝试 RAW 提取）
        photo_domain::ImageFormat::Raw(_) => {
            // 先尝试用 `image` 直接解码（部分 RAW 如 DNG 可能被 image 支持）
            match image::open(path) {
                Ok(img) => Ok(ResolvedSource::Image(cap_source_size(img))),
                Err(_) => {
                    // 使用 photo-engine 的 RAW 预览提取
                    match crate::engine_raw_preview(path) {
                        Ok(jpeg_bytes) => match image::load_from_memory(&jpeg_bytes) {
                            Ok(img) => Ok(ResolvedSource::Image(cap_source_size(img))),
                            Err(e) => {
                                tracing::warn!("RAW 预览 JPEG 解码失败: {} — {e}", path.display());
                                Err((RecognitionFailureStage::Assets, "RAW 预览解码失败"))
                            }
                        },
                        Err(e) => {
                            tracing::warn!("RAW 预览提取失败: {} — {e}", path.display());
                            Err((RecognitionFailureStage::Assets, "RAW 文件无法解码"))
                        }
                    }
                }
            }
        }
        // 视频：无画面解码能力（无视频抽帧），拒绝识别
        photo_domain::ImageFormat::Other => {
            Err((RecognitionFailureStage::Assets, "视频不支持识别"))
        }
    }
}

/// 单张 Capture 全管线识别。
///
/// 对应 pica `recognition_pipeline_service.dart:recognize()` (行 30-154)。
///
/// # 参数
/// - `detection_session`: YOLO 检测 session
/// - `classification_session`: 鸟种分类 session
/// - `eye_session`: 鸟眼检测 session
/// - `catalog`: 名录映射
/// - `capture`: 待识别的 Capture
/// - `on_progress`: 可选进度回调
///
/// # 返回
/// 识别结果（业务失败体现在 Recognition.status，Err 只用于系统故障）
pub fn recognize_capture(
    detection_session: &mut Session,
    classification_session: &mut Session,
    eye_session: &mut Session,
    catalog: &CatalogDb,
    capture: &Capture,
    on_progress: Option<&ProgressCallback>,
) -> Result<Recognition, RecognizeError> {
    recognize_capture_impl(
        detection_session,
        classification_session,
        eye_session,
        catalog,
        capture,
        None,
        on_progress,
    )
}

/// 带可选内存缩略图的全管线识别。
///
/// `thumb_bytes`：app 侧已生成的较大派生图字节（如 `.pt/thumbs` 缓存或
/// 1600px 预览），识别前优先从内存解码，避免对每张全分辨率 `image::open`
/// （24MP 全量解码 100-300ms/张）；解码失败自动回落完整路径。
/// 传 `None` 时行为与 [`recognize_capture`] 完全一致。
///
/// app 接线建议：识别前调用 `ThumbnailCache::get_or_generate(&primary_source,
/// 2048, None)`（JPEG 走 DCT 降采样快路径，毫秒级）取字节后传入；
/// lib.rs 侧包装为 `Recognizer::recognize_with_thumbnail` 后移除本 allow。
pub(crate) fn recognize_capture_with_thumbnail(
    detection_session: &mut Session,
    classification_session: &mut Session,
    eye_session: &mut Session,
    catalog: &CatalogDb,
    capture: &Capture,
    thumb_bytes: Option<&[u8]>,
    on_progress: Option<&ProgressCallback>,
) -> Result<Recognition, RecognizeError> {
    recognize_capture_impl(
        detection_session,
        classification_session,
        eye_session,
        catalog,
        capture,
        thumb_bytes,
        on_progress,
    )
}

/// 单张全管线识别共用实现（`thumb_bytes` 为可选缩略图优先输入）。
fn recognize_capture_impl(
    detection_session: &mut Session,
    classification_session: &mut Session,
    eye_session: &mut Session,
    catalog: &CatalogDb,
    capture: &Capture,
    thumb_bytes: Option<&[u8]>,
    on_progress: Option<&ProgressCallback>,
) -> Result<Recognition, RecognizeError> {
    let recognized_at = chrono::Utc::now().to_rfc3339();

    // ---- 1. 输入源解析（pica recognition_pipeline_service.dart:41-49） ----
    let source = match resolve_source_with_thumbnail(capture, thumb_bytes) {
        Ok(s) => s,
        Err((stage, msg)) => {
            tracing::warn!("[识别] 输入源解析失败: {} — {}", capture.base_name, msg);
            // 状态推断与测试共用 stage_to_status（顶部映射表：源文件不可用 → NeedsReview(Assets)）
            let (status, failure_stage) = stage_to_status(stage, false);
            return Ok(Recognition {
                status,
                bird: None,
                class_index: None,
                confidence: None,
                bbox: None,
                eye_sharpness: None,
                eye_bbox: None,
                candidates: vec![],
                failure_stage,
                recognized_at,
            });
        }
    };
    report_progress(on_progress, 0.1, "图片加载完成");

    let ResolvedSource::Image(img) = source;

    // ---- 共享 640×640 缩放：检测与眼模型同尺寸同插值（CatmullRom），
    // 缩放一次同时喂给两者，避免对同一张图做第二次全图缩放 ----
    let shared_640 = detect::resize_to_yolo_input(&img);

    // ---- 2. 检测（pica recognition_pipeline_service.dart:65-77） ----
    report_progress(on_progress, 0.35, "检测中");
    let detection = match detect::run_yolo_detection_resized(detection_session, &shared_640) {
        Ok(Some(d)) => d,
        Ok(None) => {
            // 检测无框 → Unrecognized (Detection)（pica recognition_pipeline_service.dart:68-75）
            let (status, failure_stage) = stage_to_status(RecognitionFailureStage::Detection, true);
            return Ok(Recognition {
                status,
                bird: None,
                class_index: None,
                confidence: None,
                bbox: None,
                eye_sharpness: None,
                eye_bbox: None,
                candidates: vec![],
                failure_stage,
                recognized_at,
            });
        }
        Err(e) => {
            // 检测系统故障 → NeedsReview(Classification) 而不是 Err
            // （pica 类似：catch → needs_review + classification stage）
            tracing::error!("[识别] 检测系统错误: {e}");
            let (status, failure_stage) =
                stage_to_status(RecognitionFailureStage::Classification, false);
            return Ok(Recognition {
                status,
                bird: None,
                class_index: None,
                confidence: None,
                bbox: None,
                eye_sharpness: None,
                eye_bbox: None,
                candidates: vec![],
                failure_stage,
                recognized_at,
            });
        }
    };
    let bbox = detection.bbox;
    report_progress(on_progress, 0.5, "检测完成");

    // ---- 3. 鸟眼锐度（第四阶段） ----
    report_progress(on_progress, 0.6, "鸟眼锐度");
    let (eye_sharpness, eye_bbox) = run_eye_stage(eye_session, &img, Some(&shared_640), bbox);

    // ---- 4-5. 分类 + 名录映射（与 recognize_region 共用尾部管线） ----
    report_progress(on_progress, 0.7, "分类中");
    Ok(classify_and_map(
        classification_session,
        catalog,
        &img,
        bbox,
        eye_sharpness,
        eye_bbox,
        on_progress,
        recognized_at,
    ))
}

/// 用户手动框选区域识别：跳过 YOLO 检测，直接对用户给的 bbox 分类 + 名录映射。
///
/// 用于预览界面「重新框选」：用户画的框即鸟体框，人工定位比检测更可信，
/// 因此映射成功时状态同样为 `Confirmed`。
///
/// # 参数
/// - `bbox`: 用户框选区域（归一化 0-1 坐标，相对原图）
/// - 其余同 `recognize_capture`
pub fn recognize_region(
    classification_session: &mut Session,
    eye_session: &mut Session,
    catalog: &CatalogDb,
    capture: &Capture,
    bbox: BBox,
    on_progress: Option<&ProgressCallback>,
) -> Result<Recognition, RecognizeError> {
    recognize_region_impl(
        classification_session,
        eye_session,
        catalog,
        capture,
        bbox,
        None,
        on_progress,
    )
}

/// 带可选内存缩略图的手动框选识别（跳过检测）。
///
/// `thumb_bytes` 语义同 [`recognize_capture_with_thumbnail`]：优先从内存解码
/// 派生图避免全图 `image::open`，解码失败回落完整路径；`None` 时行为与
/// [`recognize_region`] 完全一致。
pub(crate) fn recognize_region_with_thumbnail(
    classification_session: &mut Session,
    eye_session: &mut Session,
    catalog: &CatalogDb,
    capture: &Capture,
    bbox: BBox,
    thumb_bytes: Option<&[u8]>,
    on_progress: Option<&ProgressCallback>,
) -> Result<Recognition, RecognizeError> {
    recognize_region_impl(
        classification_session,
        eye_session,
        catalog,
        capture,
        bbox,
        thumb_bytes,
        on_progress,
    )
}

/// 手动框选识别共用实现（`thumb_bytes` 为可选缩略图优先输入）。
fn recognize_region_impl(
    classification_session: &mut Session,
    eye_session: &mut Session,
    catalog: &CatalogDb,
    capture: &Capture,
    bbox: BBox,
    thumb_bytes: Option<&[u8]>,
    on_progress: Option<&ProgressCallback>,
) -> Result<Recognition, RecognizeError> {
    let recognized_at = chrono::Utc::now().to_rfc3339();

    // ---- 1. 输入源解析（与自动识别同源） ----
    let source = match resolve_source_with_thumbnail(capture, thumb_bytes) {
        Ok(s) => s,
        Err((stage, msg)) => {
            tracing::warn!("[识别] 手动框选输入源解析失败: {} — {}", capture.base_name, msg);
            let (status, failure_stage) = stage_to_status(stage, false);
            return Ok(Recognition {
                status,
                bird: None,
                class_index: None,
                confidence: None,
                bbox: Some(bbox),
                eye_sharpness: None,
                eye_bbox: None,
                candidates: vec![],
                failure_stage,
                recognized_at,
            });
        }
    };
    report_progress(on_progress, 0.2, "图片加载完成");

    let ResolvedSource::Image(img) = source;

    // ---- 2. 鸟眼锐度（用户框视为鸟框；无检测阶段，眼自行缩放） ----
    report_progress(on_progress, 0.4, "鸟眼锐度");
    let (eye_sharpness, eye_bbox) = run_eye_stage(eye_session, &img, None, bbox);

    // ---- 3. 分类 + 名录映射（跳过检测） ----
    report_progress(on_progress, 0.5, "分类中");
    Ok(classify_and_map(
        classification_session,
        catalog,
        &img,
        bbox,
        eye_sharpness,
        eye_bbox,
        on_progress,
        recognized_at,
    ))
}

/// 分类 → 名录映射 → 候选解析，构建最终 Recognition。
///
/// `recognize_capture`（YOLO 检测框）与 `recognize_region`（用户手动画框）共用的尾部管线。
fn classify_and_map(
    classification_session: &mut Session,
    catalog: &CatalogDb,
    img: &DynamicImage,
    bbox: BBox,
    eye_sharpness: Option<f32>,
    eye_bbox: Option<BBox>,
    on_progress: Option<&ProgressCallback>,
    recognized_at: String,
) -> Recognition {
    // ---- 分类（pica recognition_pipeline_service.dart:80-85） ----
    let classified: ClassificationOutput =
        match crate::classify::run_classification(classification_session, img, bbox) {
            Ok(c) => c,
            Err(e) => {
                // 分类失败 → NeedsReview(Classification)（pica recognition_pipeline_service.dart:88-117）
                tracing::error!("[识别] 分类错误: {e}");
                return Recognition {
                    status: RecognitionStatus::NeedsReview,
                    bird: None,
                    class_index: None,
                    confidence: None,
                    bbox: Some(bbox),
                    eye_sharpness,
                    eye_bbox,
                    candidates: vec![],
                    failure_stage: RecognitionFailureStage::Classification,
                    recognized_at,
                };
            }
        };
    report_progress(on_progress, 0.85, "名录映射中");

    // ---- 名录映射（pica bird_label_resolver.dart:13-59） ----
    let (bird, map_stage) = catalog.resolve_class(classified.class_index);

    // 候选列表：Top-5 跳过 Top-1 自身，至多 4 条（未映射项 bird=None 也保留）
    let candidates =
        catalog.resolve_top_candidates(&classified.top_candidates, classified.class_index);

    // 状态推断（pica recognition_pipeline_service.dart:88-145）
    // 唯一匹配 → Confirmed(None)；映射失败 0/多 → NeedsReview(Mapping)
    // （与测试共用 stage_to_status，避免测试复制一份映射逻辑）
    let (status, failure_stage) = stage_to_status(map_stage, false);

    Recognition {
        status,
        bird,
        class_index: Some(classified.class_index),
        confidence: Some(classified.confidence),
        bbox: Some(bbox),
        eye_sharpness,
        eye_bbox,
        candidates,
        failure_stage,
        recognized_at,
    }
}

fn report_progress(on_progress: Option<&ProgressCallback>, value: f32, stage: &'static str) {
    if let Some(cb) = on_progress {
        cb(RecognitionProgress { value, stage });
    }
}

/// 失败阶段 → (状态, 失败阶段) 推断（pica recognition_pipeline_service.dart:88-145）。
///
/// 与文件顶部「失败阶段 → 状态映射」表保持一致：
/// - 检测无框 → `Unrecognized(Detection)`
/// - 全部成功 → `Confirmed(None)`
/// - 其余（分类/映射/资源异常）→ `NeedsReview(原阶段)`
///
/// 生产路径（recognize_capture / recognize_region / classify_and_map）
/// 与单元测试共用本实现，测试不再复制一份映射逻辑。
pub(crate) fn stage_to_status(
    failure_stage: RecognitionFailureStage,
    is_detection_failure: bool,
) -> (RecognitionStatus, RecognitionFailureStage) {
    match failure_stage {
        RecognitionFailureStage::Detection if is_detection_failure => {
            (RecognitionStatus::Unrecognized, RecognitionFailureStage::Detection)
        }
        RecognitionFailureStage::None => {
            (RecognitionStatus::Confirmed, RecognitionFailureStage::None)
        }
        stage => {
            // 分类/映射/资源异常 → NeedsReview(原阶段)
            (RecognitionStatus::NeedsReview, stage)
        }
    }
}

/// 运行鸟眼锐度阶段：眼检测 → 锐度计算。
///
/// 任何一步失败均返回 (None, None)，不影响管线主状态。
/// `shared_640` 为检测阶段的共享 640×640 缩放（可选，检测路径传 `Some`，
/// 框选路径无检测阶段传 `None` 由眼自行缩放）。
fn run_eye_stage(
    eye_session: &mut Session,
    img: &DynamicImage,
    shared_640: Option<&DynamicImage>,
    bird_bbox: BBox,
) -> (Option<f32>, Option<BBox>) {
    // 眼检测（复用共享缩放时避免第二次全图缩放）
    let eye_bbox = match shared_640 {
        Some(shared) => eye::detect_eye_shared(eye_session, img, shared, bird_bbox),
        None => eye::detect_eye(eye_session, img, bird_bbox),
    };
    let eye_bbox = match eye_bbox {
        Ok(Some(bbox)) => bbox,
        _ => return (None, None),
    };

    // 锐度计算
    let sharpness = sharpness::eye_sharpness(img, &eye_bbox);

    (sharpness, Some(eye_bbox))
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::stage_to_status;
    use photo_domain::{RecognitionFailureStage, RecognitionStatus};

    /// 测试失败阶段 → 状态映射的完备性（断言生产函数 stage_to_status，对应
    /// pica recognition_pipeline_service.dart；生产路径与测试共用同一实现）
    #[test]
    fn test_failure_stage_to_status_mapping() {
        // 检测无框 → Unrecognized(Detection)
        assert_eq!(
            stage_to_status(RecognitionFailureStage::Detection, true),
            (
                RecognitionStatus::Unrecognized,
                RecognitionFailureStage::Detection
            )
        );

        // 分类异常 → NeedsReview(Classification)
        assert_eq!(
            stage_to_status(RecognitionFailureStage::Classification, false),
            (
                RecognitionStatus::NeedsReview,
                RecognitionFailureStage::Classification
            )
        );

        // 映射失败 → NeedsReview(Mapping)
        assert_eq!(
            stage_to_status(RecognitionFailureStage::Mapping, false),
            (
                RecognitionStatus::NeedsReview,
                RecognitionFailureStage::Mapping
            )
        );

        // 资源不可用 → NeedsReview(Assets)
        assert_eq!(
            stage_to_status(RecognitionFailureStage::Assets, false),
            (
                RecognitionStatus::NeedsReview,
                RecognitionFailureStage::Assets
            )
        );

        // 全部成功 → Confirmed(None)
        assert_eq!(
            stage_to_status(RecognitionFailureStage::None, false),
            (RecognitionStatus::Confirmed, RecognitionFailureStage::None)
        );
    }
}
