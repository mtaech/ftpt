//! 识别管线编排：检测 → 分类 → 名录映射，带进度回调。
//!
//! ## 失败阶段 → 状态映射
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

use photo_domain::{
    BBox, Capture, FocusPoint, FocusShape, Recognition, RecognitionFailureStage, RecognitionStatus,
    SubjectRecognition,
};

use crate::catalog::CatalogDb;
use crate::classifier::Classifier;
use crate::detect;
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
/// 完整路径策略：
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

    let primary = capture
        .source_files
        .get(capture.primary_index)
        .or_else(|| capture.source_files.first())
        .ok_or((RecognitionFailureStage::Assets, "Capture 无源文件"))?;
    let path = &primary.path;

    // 检查文件是否存在
    if !path.exists() {
        return Err((RecognitionFailureStage::Assets, "识别源文件不存在"));
    }

    match &primary.format {
        // 标准格式直接解码
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
        // RAW 格式提取内嵌预览
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
///
/// # 参数
/// - `detection_session`: 主体检测 session（org_det）
/// - `classifier`: 识别后端（分类 + 落到物种，见 classifier.rs）
/// - `catalog`: 名录映射
/// - `capture`: 待识别的 Capture
/// - `focus_override`: 相机对焦点（`Some` = 跳过 YOLO 检测，用对焦点 ROI 直接
///   分类——「优先相机对焦点」设置路径；`None` = 全图 YOLO 检测，默认行为）
/// - `on_progress`: 可选进度回调
///
/// # 返回
/// 识别结果（业务失败体现在 Recognition.status，Err 只用于系统故障）
pub fn recognize_capture(
    detection_session: &mut Session,
    classifier: &mut dyn Classifier,
    catalog: &CatalogDb,
    capture: &Capture,
    focus_override: Option<FocusPoint>,
    on_progress: Option<&ProgressCallback>,
) -> Result<Recognition, RecognizeError> {
    recognize_capture_impl(
        detection_session,
        classifier,
        catalog,
        capture,
        None,
        focus_override,
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
    classifier: &mut dyn Classifier,
    catalog: &CatalogDb,
    capture: &Capture,
    thumb_bytes: Option<&[u8]>,
    focus_override: Option<FocusPoint>,
    on_progress: Option<&ProgressCallback>,
) -> Result<Recognition, RecognizeError> {
    recognize_capture_impl(
        detection_session,
        classifier,
        catalog,
        capture,
        thumb_bytes,
        focus_override,
        on_progress,
    )
}

/// 对焦点 ROI 的边长下限（归一化比例）：AF 区域/点没有鸟体范围信息，
/// 外扩后仍需保底尺寸，防止极小框（远处小鸟的 AF 区域可能很小）。
const FOCUS_MIN_SIDE_RATIO: f32 = 0.18;

/// 相机对焦点 → 分类用归一化 bbox（焦点优先路径的 ROI 构造，跳过 YOLO 检测）。
///
/// FocusPoint 的坐标/尺寸已全部归一化（相对 orientation 修正后的显示方向，
/// exif.rs `normalize_focus`），与 BBox 同一坐标系，直接在此空间构造：
/// Point 以 30% 图幅比例正方形；Circle 以圆心为中心按直径外扩 1.6×；
/// Rectangle 以中心外扩 1.4× 保持纵横比。统一夹紧 0-1（`BBox::new` 自带）。
/// 注：归一化空间的「正方形」在像素空间随图宽高比伸缩，ROI 本就是近似。
fn focus_to_bbox(fp: &FocusPoint) -> BBox {
    match fp.shape {
        FocusShape::Point => {
            // 无尺寸信息：以点为中心、30% 图幅比例的正方形（猜测鸟体范围）
            let side = 0.3f32.max(FOCUS_MIN_SIDE_RATIO);
            BBox::new(fp.x - side / 2.0, fp.y - side / 2.0, fp.x + side / 2.0, fp.y + side / 2.0)
        }
        FocusShape::Circle => {
            // 圆心 + 直径（归一化）：外扩 1.6× 并保底下限（AF 区域通常小于鸟体）
            let side = (fp.width.max(0.0) * 1.6).max(FOCUS_MIN_SIDE_RATIO);
            BBox::new(fp.x - side / 2.0, fp.y - side / 2.0, fp.x + side / 2.0, fp.y + side / 2.0)
        }
        FocusShape::Rectangle => {
            // 左上角 + 宽高（归一化）：以中心外扩 1.4×，保持纵横比并保底下限
            let rw = (fp.width.max(0.0) * 1.4).max(FOCUS_MIN_SIDE_RATIO);
            let rh = (fp.height.max(0.0) * 1.4).max(FOCUS_MIN_SIDE_RATIO);
            let cx = fp.x + fp.width / 2.0;
            let cy = fp.y + fp.height / 2.0;
            BBox::new(cx - rw / 2.0, cy - rh / 2.0, cx + rw / 2.0, cy + rh / 2.0)
        }
    }
}

/// 单张全管线识别共用实现（`thumb_bytes` 为可选缩略图优先输入）。
fn recognize_capture_impl(
    detection_session: &mut Session,
    classifier: &mut dyn Classifier,
    catalog: &CatalogDb,
    capture: &Capture,
    thumb_bytes: Option<&[u8]>,
    focus_override: Option<FocusPoint>,
    on_progress: Option<&ProgressCallback>,
) -> Result<Recognition, RecognizeError> {
    let recognized_at = chrono::Utc::now().to_rfc3339();
    tracing::debug!(
        "[识别] 进入识别: {}（缩略图优先={}，焦点优先={}）",
        capture.base_name,
        thumb_bytes.is_some_and(|b| !b.is_empty()),
        focus_override.is_some()
    );

    // ---- 1. 输入源解析 ----
    let source = match resolve_source_with_thumbnail(capture, thumb_bytes) {
        Ok(s) => s,
        Err((stage, msg)) => {
            tracing::warn!("[识别] 输入源解析失败: {} — {}", capture.base_name, msg);
            // 状态推断与测试共用 stage_to_status（顶部映射表：源文件不可用 → NeedsReview(Assets)）
            let (status, failure_stage) = stage_to_status(stage, false);
            return Ok(Recognition {
                status,
                taxon: None,
                class_index: None,
                confidence: None,
                bbox: None,
                candidates: vec![],
                failure_stage,
                recognized_at,
                subjects: vec![],
            });
        }
    };
    report_progress(on_progress, 0.1, "图片加载完成");

    let ResolvedSource::Image(img) = source;

    // 焦点优先路径：有相机对焦点 → 跳过 YOLO 检测，用对焦点 ROI 直接分类。
    // 状态语义对齐 recognize_region 人工框选（映射成功即 Confirmed）——相机对焦
    // 位置先验在拍鸟场景可靠（摄影师对鸟对焦），且跳过检测阶段不存在 Detection 失败。
    if let Some(fp) = focus_override {
        report_progress(on_progress, 0.35, "对焦点定位");
        let bbox = focus_to_bbox(&fp);
        report_progress(on_progress, 0.5, "检测完成");
        report_progress(on_progress, 0.7, "分类中");
        let subject = classify_subject(classifier, catalog, &img, bbox, 0);
        report_progress(on_progress, 0.85, "结果整理中");
        return Ok(build_recognition(vec![subject], recognized_at));
    }

    // ---- 共享 640×640 缩放：检测与眼模型同尺寸同插值（CatmullRom），
    // 缩放一次同时喂给两者，避免对同一张图做第二次全图缩放 ----
    let shared_640 = detect::resize_to_yolo_input(&img);

    // ---- 2. 检测（org_det 多框） ----
    report_progress(on_progress, 0.35, "检测中");
    let detections = match detect::run_yolo_detection_resized(detection_session, &shared_640) {
        Ok(d) => d,
        Err(e) => {
            // 检测系统故障 → NeedsReview(Classification) 而不是 Err
            tracing::error!("[识别] 检测系统错误: {e}");
            let (status, failure_stage) =
                stage_to_status(RecognitionFailureStage::Classification, false);
            return Ok(Recognition {
                status,
                taxon: None,
                class_index: None,
                confidence: None,
                bbox: None,
                candidates: vec![],
                failure_stage,
                recognized_at,
                subjects: vec![],
            });
        }
    };
    report_progress(on_progress, 0.5, "检测完成");

    if detections.is_empty() {
        // 检测无框 → 退回整图识别（org_det 的 19 个粗类偶尔也会漏掉主体，
        // 此时整图直接分类仍是可用的兜底；整幅图算一个主体）。
        if classifier.whole_image_on_no_detection() {
            tracing::debug!("[识别] {} 未检出主体，BioCLIP 退回整图识别", capture.base_name);
            report_progress(on_progress, 0.5, "整图识别");
            let subject = classify_subject(classifier, catalog, &img, BBox::new(0.0, 0.0, 1.0, 1.0), 0);
            report_progress(on_progress, 0.85, "结果整理中");
            return Ok(build_recognition(vec![subject], recognized_at));
        }
        tracing::debug!("[识别] {} 未检出主体（Unrecognized）", capture.base_name);
        let (status, failure_stage) = stage_to_status(RecognitionFailureStage::Detection, true);
        return Ok(Recognition {
            status,
            taxon: None,
            class_index: None,
            confidence: None,
            bbox: None,
            candidates: vec![],
            failure_stage,
            recognized_at,
            subjects: vec![],
        });
    }

    // ---- 3. 每个主体分类 + 名录映射（多主体：逐个分类） ----
    report_progress(on_progress, 0.7, "分类中");
    let classified: Vec<SubjectRecognition> = detections
        .iter()
        .enumerate()
        .map(|(i, d)| classify_subject(classifier, catalog, &img, d.bbox, i as u32))
        .collect();

    // ---- 4. 交叉验证（docs/todo.md #16）：检测器粗类 ↔ 分类器细类不自洽的框判为误检 ----
    //
    // 与 bioclip_demo 的 prune 同一顺序：交叉验证必须在去重之前（类名错的框会先把
    // 类名对的框顶掉，再去重就把对的也一起丢了）。本模块的「去背景 + IoU 去重 + 上限 8」
    // 已在 detect::run_yolo_detection_resized 里完成，所以这里只剩一致性过滤。
    let det_classes: Vec<&str> = detections.iter().map(|d| d.class_name).collect();
    let subjects = cross_validate_subjects(classified, &det_classes);

    if subjects.is_empty() {
        // 所有框都被判为误检 → 退回整图识别（与「检测无框」同一条兜底，
        // 绝不把整张图的结果一起丢掉）。
        tracing::debug!(
            "[识别] {} 的 {} 个检测框全部未通过交叉验证，退回整图识别",
            capture.base_name,
            detections.len()
        );
        if classifier.whole_image_on_no_detection() {
            report_progress(on_progress, 0.8, "整图识别");
            let subject =
                classify_subject(classifier, catalog, &img, BBox::new(0.0, 0.0, 1.0, 1.0), 0);
            report_progress(on_progress, 0.85, "结果整理中");
            return Ok(build_recognition(vec![subject], recognized_at));
        }
        let (status, failure_stage) = stage_to_status(RecognitionFailureStage::Detection, true);
        return Ok(Recognition {
            status,
            taxon: None,
            class_index: None,
            confidence: None,
            bbox: None,
            candidates: vec![],
            failure_stage,
            recognized_at,
            subjects: vec![],
        });
    }

    report_progress(on_progress, 0.85, "结果整理中");
    Ok(build_recognition(subjects, recognized_at))
}

/// 候选框交叉验证：丢掉「检测器粗类与分类器细类不自洽」的误检框，并把 index 重排成连续。
///
/// 认不出粗类名（unknown / 未来新增类）、以及**没有物种结论**的主体一律放行——
/// 不拿不确定当证据丢框。丢掉后若一个都不剩，调用方会退回整图识别。
fn cross_validate_subjects(
    subjects: Vec<SubjectRecognition>,
    det_classes: &[&str],
) -> Vec<SubjectRecognition> {
    let mut kept: Vec<SubjectRecognition> = Vec::with_capacity(subjects.len());
    for (subject, det_class) in subjects.into_iter().zip(det_classes.iter()) {
        let consistent = subject
            .taxon
            .as_ref()
            .map_or(true, |t| detect::class_is_consistent(det_class, &t.ranks));
        if consistent {
            kept.push(subject);
        } else {
            let latin = subject
                .taxon
                .as_ref()
                .map(|t| t.latin_name.clone())
                .unwrap_or_default();
            tracing::debug!(
                "[识别] 交叉验证丢弃误检框: 检测器={det_class} 分类器={latin} bbox={:?}",
                subject.bbox
            );
        }
    }
    // 丢框会留下空号，序号重排成连续的（0 = 主主体，下游/存储都依赖这个口径）
    for (i, subject) in kept.iter_mut().enumerate() {
        subject.index = i as u32;
    }
    kept
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
    classifier: &mut dyn Classifier,
    catalog: &CatalogDb,
    capture: &Capture,
    bbox: BBox,
    on_progress: Option<&ProgressCallback>,
) -> Result<Recognition, RecognizeError> {
    recognize_region_impl(classifier, catalog, capture, bbox, None, on_progress)
}

/// 带可选内存缩略图的手动框选识别（跳过检测）。
///
/// `thumb_bytes` 语义同 [`recognize_capture_with_thumbnail`]：优先从内存解码
/// 派生图避免全图 `image::open`，解码失败回落完整路径；`None` 时行为与
/// [`recognize_region`] 完全一致。
pub(crate) fn recognize_region_with_thumbnail(
    classifier: &mut dyn Classifier,
    catalog: &CatalogDb,
    capture: &Capture,
    bbox: BBox,
    thumb_bytes: Option<&[u8]>,
    on_progress: Option<&ProgressCallback>,
) -> Result<Recognition, RecognizeError> {
    recognize_region_impl(classifier, catalog, capture, bbox, thumb_bytes, on_progress)
}

/// 手动框选识别共用实现（`thumb_bytes` 为可选缩略图优先输入）。
fn recognize_region_impl(
    classifier: &mut dyn Classifier,
    catalog: &CatalogDb,
    capture: &Capture,
    bbox: BBox,
    thumb_bytes: Option<&[u8]>,
    on_progress: Option<&ProgressCallback>,
) -> Result<Recognition, RecognizeError> {
    let recognized_at = chrono::Utc::now().to_rfc3339();
    tracing::debug!(
        "[识别] 进入框选识别: {}（缩略图优先={}）",
        capture.base_name,
        thumb_bytes.is_some_and(|b| !b.is_empty())
    );

    // ---- 1. 输入源解析（与自动识别同源） ----
    let source = match resolve_source_with_thumbnail(capture, thumb_bytes) {
        Ok(s) => s,
        Err((stage, msg)) => {
            tracing::warn!("[识别] 手动框选输入源解析失败: {} — {}", capture.base_name, msg);
            let (status, failure_stage) = stage_to_status(stage, false);
            return Ok(Recognition {
                status,
                taxon: None,
                class_index: None,
                confidence: None,
                bbox: Some(bbox),
                candidates: vec![],
                failure_stage,
                recognized_at,
                subjects: vec![],
            });
        }
    };
    report_progress(on_progress, 0.2, "图片加载完成");

    let ResolvedSource::Image(img) = source;

    // ---- 2. 分类 + 名录映射（跳过检测，单主体） ----
    report_progress(on_progress, 0.5, "分类中");
    let subject = classify_subject(classifier, catalog, &img, bbox, 0);
    report_progress(on_progress, 0.85, "结果整理中");
    Ok(build_recognition(vec![subject], recognized_at))
}

/// 单个主体：分类 → 名录映射 → 主体结论（多主体照片的数组元素）。
///
/// 单个主体的分类失败（系统级）只让该主体变成 NeedsReview(Classification)，
/// 不中断同一张照片里其他主体的识别。
fn classify_subject(
    classifier: &mut dyn Classifier,
    catalog: &CatalogDb,
    img: &DynamicImage,
    bbox: BBox,
    index: u32,
) -> SubjectRecognition {
    match classifier.classify(catalog, img, bbox) {
        Ok(c) => SubjectRecognition {
            index,
            bbox,
            taxon: c.taxon,
            class_index: c.class_index,
            confidence: c.confidence,
            candidates: c.candidates,
            failure: c.failure,
        },
        Err(e) => {
            tracing::error!("[识别] 主体 {index} 分类错误: {e}");
            SubjectRecognition {
                index,
                bbox,
                taxon: None,
                class_index: None,
                confidence: None,
                candidates: vec![],
                failure: RecognitionFailureStage::Classification,
            }
        }
    }
}

/// 各主体结论 → 最终 Recognition。
///
/// - 顶层字段恒 = 主主体（subjects[0]），兼容既有展示 / 筛选 / 持久化
/// - 照片状态：任一主体有结论（failure == None）→ Confirmed；
///   所有主体都失败 → NeedsReview(首个失败阶段)；无主体 → Unrecognized(Detection)
fn build_recognition(subjects: Vec<SubjectRecognition>, recognized_at: String) -> Recognition {
    let primary = subjects.first();
    let (status, failure_stage) =
        match subjects.iter().find(|s| s.failure == RecognitionFailureStage::None) {
            Some(_) => (RecognitionStatus::Confirmed, RecognitionFailureStage::None),
            None => match subjects.first() {
                Some(s) => (RecognitionStatus::NeedsReview, s.failure),
                None => (RecognitionStatus::Unrecognized, RecognitionFailureStage::Detection),
            },
        };
    Recognition {
        status,
        taxon: primary.and_then(|s| s.taxon.clone()),
        class_index: primary.and_then(|s| s.class_index),
        confidence: primary.and_then(|s| s.confidence),
        bbox: primary.map(|s| s.bbox),
        candidates: primary.map(|s| s.candidates.clone()).unwrap_or_default(),
        failure_stage,
        recognized_at,
        subjects,
    }
}

fn report_progress(on_progress: Option<&ProgressCallback>, value: f32, stage: &'static str) {
    if let Some(cb) = on_progress {
        cb(RecognitionProgress { value, stage });
    }
}

/// 失败阶段 → (状态, 失败阶段) 推断。
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

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::{build_recognition, cross_validate_subjects, focus_to_bbox, stage_to_status};
    use photo_domain::{
        BBox, CnLevel, FocusPoint, RecognitionFailureStage, RecognitionStatus, SubjectRecognition,
        TaxonMatch,
    };

    fn subject(index: u32, kind: &str) -> SubjectRecognition {
        let ranks = match kind {
            "bird" => vec!["Animalia", "Chordata", "Aves"],
            "plant" => vec!["Plantae", "Tracheophyta", "Magnoliopsida"],
            _ => vec!["", "", ""],
        };
        SubjectRecognition {
            index,
            bbox: BBox::new(0.1, 0.1, 0.4, 0.4),
            taxon: (kind != "none").then(|| TaxonMatch {
                taxon_id: None,
                cn_name: String::new(),
                latin_name: format!("{kind}us testus"),
                cn_level: CnLevel::Missing,
                ranks: ranks.iter().map(|s| s.to_string()).collect(),
            }),
            class_index: Some(1),
            confidence: Some(50.0),
            candidates: vec![],
            failure: RecognitionFailureStage::None,
        }
    }

    #[test]
    fn test_cross_validate_drops_mismatched_and_renumbers() {
        // 三个框：鸟（检测器说 bird，自洽）、植物（检测器说 flower，自洽）、
        // 一朵被误检成鸟的花（检测器说 bird、分类器说是植物 → 丢）
        let subjects = vec![subject(0, "bird"), subject(1, "plant"), subject(2, "plant")];
        let classes = ["bird", "flower", "bird"];
        let kept = cross_validate_subjects(subjects, &classes);
        assert_eq!(kept.len(), 2, "误检框应被丢掉");
        assert_eq!(kept[0].index, 0);
        assert_eq!(kept[1].index, 1, "丢框后序号要重排成连续");
        assert_eq!(kept[0].taxon.as_ref().unwrap().latin_name, "birdus testus");
        assert_eq!(kept[1].taxon.as_ref().unwrap().latin_name, "plantus testus");
    }

    #[test]
    fn test_cross_validate_keeps_unmapped_and_unknown_classes() {
        // 没有物种结论的主体、以及未知粗类名都放行（不拿不确定当证据丢框）
        let subjects = vec![subject(0, "none"), subject(1, "bird")];
        let kept = cross_validate_subjects(subjects, &["plant", "unknown"]);
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0].index, 0);
        assert_eq!(kept[1].index, 1);
    }

    #[test]
    fn test_cross_validate_can_empty_out() {
        // 全部不自洽 → 空（调用方据此退回整图识别，不丢整张图）
        let subjects = vec![subject(0, "plant"), subject(1, "bird")];
        let kept = cross_validate_subjects(subjects, &["bird", "flower"]);
        assert!(kept.is_empty());
    }

    /// 对焦点 → ROI 构造：三种形状的尺寸/位置语义与保底下限
    #[test]
    fn test_focus_to_bbox_shapes() {
        let approx = |a: f32, b: f32| (a - b).abs() < 1e-5;
        // Point：以点为中心、30% 图幅比例的正方形
        let b = focus_to_bbox(&FocusPoint::point(0.5, 0.5));
        assert!(approx(b.x1, 0.35) && approx(b.y1, 0.35) && approx(b.x2, 0.65) && approx(b.y2, 0.65));

        // Circle：圆心 + 直径外扩 1.6×（0.2 → 0.32）
        let b = focus_to_bbox(&FocusPoint::circle(0.5, 0.5, 0.2));
        assert!(approx(b.x1, 0.34) && approx(b.y1, 0.34) && approx(b.x2, 0.66) && approx(b.y2, 0.66));

        // Rectangle：中心外扩 1.4×，保持纵横比（0.2×0.4 → 0.28×0.56）
        let b = focus_to_bbox(&FocusPoint::rectangle(0.2, 0.3, 0.2, 0.4));
        assert!(approx(b.x1, 0.16) && approx(b.y1, 0.22) && approx(b.x2, 0.44) && approx(b.y2, 0.78));
    }

    /// 极小/越界对焦点：保底下限与 0-1 夹紧
    #[test]
    fn test_focus_to_bbox_clamp_and_floor() {
        let approx = |a: f32, b: f32| (a - b).abs() < 1e-5;
        // 极小圆直径 → 保底 18% 图幅比例
        let b = focus_to_bbox(&FocusPoint::circle(0.5, 0.5, 0.001));
        assert!(approx(b.x1, 0.41) && approx(b.y1, 0.41) && approx(b.x2, 0.59) && approx(b.y2, 0.59));

        // 点靠近边缘：负坐标夹到 0，框贴边（BBox::new 逐坐标夹紧）
        let b = focus_to_bbox(&FocusPoint::point(0.05, 0.05));
        assert_eq!((b.x1, b.y1), (0.0, 0.0));
        assert!(approx(b.x2, 0.2));

        // 零尺寸矩形 → 保底下限
        let b = focus_to_bbox(&FocusPoint::rectangle(0.1, 0.1, 0.0, 0.0));
        assert!(approx(b.x2 - b.x1, 0.18));
        assert!(approx(b.y2 - b.y1, 0.18));
    }

    /// 测试失败阶段 → 状态映射的完备性（断言生产函数 stage_to_status；
    /// 生产路径与测试共用同一实现）
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

    /// 造一个主体（taxon 全空，仅指定失败阶段）
    fn subject_with(failure: RecognitionFailureStage, conf: Option<f32>) -> SubjectRecognition {
        SubjectRecognition {
            index: 0,
            bbox: BBox::new(0.1, 0.1, 0.5, 0.5),
            taxon: None,
            class_index: None,
            confidence: conf,
            candidates: vec![],
            failure,
        }
    }

    #[test]
    fn test_build_recognition_status_aggregation() {
        // 任一主体有结论 → Confirmed；顶层字段 = 主主体（subjects[0]）
        let r = build_recognition(
            vec![subject_with(RecognitionFailureStage::None, Some(88.0))],
            "t".into(),
        );
        assert_eq!(r.status, RecognitionStatus::Confirmed);
        assert_eq!(r.failure_stage, RecognitionFailureStage::None);
        assert_eq!(r.subjects.len(), 1);
        assert_eq!(r.confidence, Some(88.0));

        // 全部失败 → NeedsReview(首个失败阶段)
        let r = build_recognition(
            vec![subject_with(RecognitionFailureStage::Classification, None)],
            "t".into(),
        );
        assert_eq!(r.status, RecognitionStatus::NeedsReview);
        assert_eq!(r.failure_stage, RecognitionFailureStage::Classification);

        // 多主体混合：一个有结论 → Confirmed，主主体仍是 subjects[0]
        let r = build_recognition(
            vec![
                subject_with(RecognitionFailureStage::None, Some(70.0)),
                subject_with(RecognitionFailureStage::None, Some(80.0)),
            ],
            "t".into(),
        );
        assert_eq!(r.status, RecognitionStatus::Confirmed);
        assert_eq!(r.subjects.len(), 2);
        assert_eq!(r.confidence, Some(70.0), "主主体 = subjects[0] 的置信度");

        // 无主体 → Unrecognized(Detection)
        let r = build_recognition(vec![], "t".into());
        assert_eq!(r.status, RecognitionStatus::Unrecognized);
        assert_eq!(r.failure_stage, RecognitionFailureStage::Detection);
    }
}
