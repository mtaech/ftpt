//! 物种识别：YOLO 检测 → BioCLIP 全物种零样本分类 → 名录补充（全同步）。
//!
//! 管线语义与持久化格式见 `docs/adr/0002-folder-central-db.md`、
//! `docs/adr/0003-recognition-subsystem.md`；领域类型定义在 `photo-domain`。
//!
//! ## 实现说明
//!
//! - 候选列表中的未映射项 `taxon=None` 也保留（不跳过）
//! - 输入源解析：RAW 格式使用 `photo_engine::thumbnail::decode_raw_preview`
//!   提取内嵌 JPEG（通用解码失败时仍尝试 RAW 提取）
//! - 预处理/后处理常数与模型元数据一致
//!
//! ## 识别库表级边界契约
//!
//! `photo-engine` 的 `data.db`（SQLite 库）内含三表：
//!
//! | 表 | 性质 | 内容 |
//! |---|---|---|
//! | `exif_cache` | **缓存** | EXIF 元数据（可从源文件重算，可清） |
//! | `recognition` | **真相** | 识别结果（不可当缓存清） |
//!
//! 本 crate **不直接读写 `data.db`**——recognition 表的写入/读取由 `photo-engine`
//! 的 `FolderDb` 模块完成，本 crate 只负责计算 `Recognition` 值对象。

mod bioclip;
mod catalog;
mod classifier;
mod detect;
mod pipeline;

use std::path::{Path, PathBuf};

#[cfg(target_os = "windows")]
use ort::ep;
use ort::session::Session;

use photo_domain::{BBox, Capture, FocusPoint, Recognition};

pub use bioclip::{
    BioClipAssets, BioClipClassifier, CHINA_PROVINCES, RegionFilter, build_region_filter,
    MODEL_FILE as BIOCLIP_MODEL_FILE,
};
pub use catalog::CatalogDb;
pub use classifier::{Classified, Classifier};
pub use detect::DetectionResult;
pub use pipeline::{ProgressCallback, RecognitionProgress};

// ---------------------------------------------------------------------------
// RecognizeError
// ---------------------------------------------------------------------------

/// 识别系统错误（仅系统级故障，非业务失败）。
///
/// 业务失败（检测无框、分类异常、映射失败）体现在 `Recognition.status`。
#[derive(Debug, thiserror::Error)]
pub enum RecognizeError {
    /// IO 操作失败
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),

    /// ONNX Runtime 错误
    #[error("ONNX Runtime 错误: {0}")]
    Ort(#[from] ort::Error),

    /// 名录库访问错误
    #[error("名录库错误: {0}")]
    Catalog(#[from] rusqlite::Error),

    /// 图片解码错误
    #[error("图片错误: {0}")]
    Image(#[from] image::ImageError),

    /// 模型文件加载失败
    #[error("模型加载失败: {0}")]
    ModelLoad(String),

    /// 分类器输出为空
    #[error("分类输出为空")]
    ClassificationOutputEmpty,

    /// RAW 预览提取失败
    #[error("RAW 预览提取失败: {0}")]
    RawPreview(String),

    /// 识别资产（标签 / 中文名表 / VERSION）JSON 解析失败
    #[error("识别资产 JSON 错误: {0}")]
    Json(#[from] serde_json::Error),
}

/// 推理后端。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    DirectML,
    Cpu,
}

impl std::fmt::Display for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Backend::DirectML => write!(f, "DirectML"),
            Backend::Cpu => write!(f, "CPU"),
        }
    }
}
// ---------------------------------------------------------------------------
// Recognizer
// ---------------------------------------------------------------------------

/// 识别器：持有主体检测 Session、BioCLIP 分类后端 + 名录库连接。
///
/// 示例：
/// ```no_run
/// use photo_recognize::Recognizer;
///
/// let recognizer = Recognizer::new(
///     &std::path::Path::new("./models"),
///     &std::path::Path::new("./data/bird_catalog.db"),
/// ).unwrap();
/// ```
pub struct Recognizer {
    detection_session: Session,
    /// 物种分类后端：分类 + 落到物种（BioCLIP，见 classifier.rs / bioclip.rs）
    classifier: Box<dyn Classifier>,
    catalog: CatalogDb,
    /// ONNX Runtime 执行提供程序（DirectML / CPU）——与识别后端是两件事
    backend: Backend,
}

impl Recognizer {
    /// 创建识别器。
    ///
    /// # 参数
    /// - `models_dir`: 模型目录（`org_det.onnx`、`bioclip2_model_int8.onnx`）
    /// - `catalog_db`: 名录库 `bird_catalog.db` 路径
    ///
    /// # 模型路径约定（便携模式）
    /// 默认路径相对于 exe 所在目录：`exe_dir/models/` + `exe_dir/data/bird_catalog.db`；
    /// BioCLIP 的名录子集资产在名录库同级的 `taxon/`（`exe_dir/data/taxon/`）。
    /// 调用方可通过参数注入任意路径（测试用临时目录等）。
    ///
    /// # 执行提供程序
    /// - Windows: DirectML → 失败回退 CPU 并 `tracing::warn` 记录原因
    /// - 非 Windows: CPU
    pub fn new(models_dir: &Path, catalog_db: &Path) -> Result<Self, RecognizeError> {
        Self::with_region(models_dir, catalog_db, "")
    }

    /// 创建识别器并启用地区过滤。
    ///
    /// `region` 为空串 = 全国（不过滤，与 [`Recognizer::new`] 一致）；否则为省级
    /// 行政区名称（见 [`crate::CHINA_PROVINCES`]）。地区过滤在装配时把 top-k 候选
    /// 裁剪到「该省有 GBIF 出现记录」的鸟种（非鸟标签与无地区数据的鸟种恒不过滤），
    /// 只影响检索，不改变资产版本（class_index 仍引用中国包原始列）。
    /// 改动地区后需重新装配识别器（photo-ui 侧在设置变更时释放常驻识别器）。
    pub fn with_region(
        models_dir: &Path,
        catalog_db: &Path,
        region: &str,
    ) -> Result<Self, RecognizeError> {
        let detector_path = models_dir.join(detect::MODEL_FILE);

        if !detector_path.exists() {
            return Err(RecognizeError::ModelLoad(format!(
                "检测模型文件不存在: {}。请将 org_det.onnx 放入 models/ 目录",
                detector_path.display()
            )));
        }
        let (detection_session, backend_ep) = load_model(&detector_path)?;
        let catalog = CatalogDb::open(catalog_db)?;

        let model = models_dir.join(bioclip::MODEL_FILE);
        if !model.exists() {
            return Err(RecognizeError::ModelLoad(format!(
                "BioCLIP 模型文件不存在: {}。请把 bioclip2_model_int8.onnx 放入 models/ 目录",
                model.display()
            )));
        }
        // 名录子集资产与名录库同级：<data_root>/data/taxon/
        let taxon_dir = catalog_db
            .parent()
            .map(|d| d.join("taxon"))
            .unwrap_or_else(|| PathBuf::from("taxon"));
        let assets = BioClipAssets::load(&taxon_dir)?;
        tracing::info!("BioCLIP 名录子集: {} 类（{}）", assets.label_count(), taxon_dir.display());
        let session = Session::builder()?.commit_from_file(&model).map_err(|e| {
            RecognizeError::ModelLoad(format!(
                "BioCLIP session 创建失败 ({}): {e}",
                model.display()
            ))
        })?;
        let classifier: Box<dyn Classifier> =
            Box::new(BioClipClassifier::with_region(session, assets, region));

        tracing::info!(
            "识别器初始化完成，识别后端: {}，推理后端: {}",
            classifier.backend(),
            backend_ep
        );

        Ok(Self {
            detection_session,
            classifier,
            catalog,
            backend: backend_ep,
        })
    }

    /// 当前识别后端标识（恒为 `"bioclip"`），诊断与状态栏用。
    pub fn classifier_backend(&self) -> &'static str {
        self.classifier.backend()
    }

    /// 当前识别是否启用了地区过滤（诊断/状态栏/冒烟用）。
    pub fn region_filter_active(&self) -> bool {
        self.classifier.region_filter_active()
    }

    /// 当前识别后端的资产版本（class_index 的语义依赖它），无版本信息时为 None。
    pub fn asset_version(&self) -> Option<String> {
        self.classifier.asset_version()
    }

    /// 单张全管线识别。
    ///
    /// 业务失败体现在 `Recognition.status`，`Err` 仅用于模型/库不可用等系统性故障。
    ///
    /// `focus_override`：相机对焦点（`Some` = 跳过 YOLO 检测，用对焦点 ROI 直接
    /// 分类，对齐 [`Recognizer::recognize_region`] 的框选语义；`None` = 全图 YOLO）。
    pub fn recognize(
        &mut self,
        capture: &Capture,
        focus_override: Option<FocusPoint>,
        on_progress: Option<&pipeline::ProgressCallback>,
    ) -> Result<Recognition, RecognizeError> {
        pipeline::recognize_capture(
            &mut self.detection_session,
            &mut *self.classifier,
            &self.catalog,
            capture,
            focus_override,
            on_progress,
        )
    }

    /// 单张全管线识别（缩略图优先输入）。
    ///
    /// `thumb_bytes` 非空时优先从内存解码——跳过全图 `image::open`
    /// （24MP 全量解码 100-300ms/张；缩略图为 app 侧 `.pt/thumbs` 派生图字节，
    /// JPEG DCT 快路径毫秒级），解码失败自动回落完整路径；
    /// `None` 时与 [`Recognizer::recognize`] 行为完全一致。
    pub fn recognize_with_thumbnail(
        &mut self,
        capture: &Capture,
        thumb_bytes: Option<&[u8]>,
        focus_override: Option<FocusPoint>,
        on_progress: Option<&pipeline::ProgressCallback>,
    ) -> Result<Recognition, RecognizeError> {
        pipeline::recognize_capture_with_thumbnail(
            &mut self.detection_session,
            &mut *self.classifier,
            &self.catalog,
            capture,
            thumb_bytes,
            focus_override,
            on_progress,
        )
    }

    /// 手动框选区域识别。
    ///
    /// 跳过 YOLO 检测，直接对用户给的 `bbox`（归一化 0-1 坐标）分类 + 名录映射。
    /// 用于预览界面「重新框选」。业务失败体现在 `Recognition.status`，
    /// `Err` 仅用于模型/库不可用等系统性故障。
    pub fn recognize_region(
        &mut self,
        capture: &Capture,
        bbox: BBox,
        on_progress: Option<&pipeline::ProgressCallback>,
    ) -> Result<Recognition, RecognizeError> {
        pipeline::recognize_region(
            &mut *self.classifier,
            &self.catalog,
            capture,
            bbox,
            on_progress,
        )
    }

    /// 手动框选区域识别（缩略图优先输入）。
    ///
    /// `thumb_bytes` 语义同 [`Recognizer::recognize_with_thumbnail`]：
    /// 非空优先内存解码省全图 `image::open`，失败回落完整路径；
    /// `None` 时与 [`Recognizer::recognize_region`] 行为完全一致。
    pub fn recognize_region_with_thumbnail(
        &mut self,
        capture: &Capture,
        bbox: BBox,
        thumb_bytes: Option<&[u8]>,
        on_progress: Option<&pipeline::ProgressCallback>,
    ) -> Result<Recognition, RecognizeError> {
        pipeline::recognize_region_with_thumbnail(
            &mut *self.classifier,
            &self.catalog,
            capture,
            bbox,
            thumb_bytes,
            on_progress,
        )
    }

    /// 返回当前使用的推理后端。
    pub fn backend(&self) -> Backend {
        self.backend
    }
}

// ---------------------------------------------------------------------------
// 模型加载
// ---------------------------------------------------------------------------

/// 加载 ONNX 模型，返回 session 与实际使用的推理后端。
fn load_model(path: &Path) -> Result<(Session, Backend), RecognizeError> {
    #[cfg(target_os = "windows")]
    {
        // Windows: 先试 DirectML，失败回退 CPU
        // HighPerformance = DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE：
        // 双显卡机型（核显 + N 卡独显）上让 Windows 把 DirectML 设备绑到独显，
        // 否则默认可能落在核显（如 Radeon 780M）上
        match (|| -> Result<_, ort::Error> {
            let builder = Session::builder()?;
            let mut builder = builder.with_execution_providers([ep::DirectML::default()
                .with_performance_preference(ep::directml::PerformancePreference::HighPerformance)
                .build()])?;
            builder.commit_from_file(path)
        })() {
            Ok(session) => {
                tracing::info!("模型加载成功 (DirectML): {}", path.display());
                return Ok((session, Backend::DirectML));
            }
            Err(e) => {
                tracing::warn!(
                    "DirectML 创建 session 失败，回退 CPU: {} — {e}",
                    path.display()
                );
            }
        }
    }

    // CPU 加载（Windows 回退 + 非 Windows 默认）
    let session = Session::builder()?.commit_from_file(path).map_err(|e| {
        RecognizeError::ModelLoad(format!("CPU session 创建失败 ({}): {e}", path.display()))
    })?;
    tracing::info!("模型加载成功 (CPU): {}", path.display());
    Ok((session, Backend::Cpu))
}

// ---------------------------------------------------------------------------
// RAW 预览提取（供 pipeline 模块调用）
// ---------------------------------------------------------------------------

/// 识别用 RAW 预览提取：优先相机内嵌 JPEG（~50ms）。
///
/// 识别输入只需 640/224 分辨率（检测/分类），内嵌预览（相机写入的
/// 1600×1200 级 JPEG）足够——避免 20MP RAW 完整解码（LibRaw half_size
/// 约 5-8s，是识别慢的主因）。内嵌缺失/非 JPEG 时回退完整解码。
fn engine_raw_preview(path: &Path) -> Result<Vec<u8>, RecognizeError> {
    // 快路径：内嵌 JPEG（任意尺寸都收——识别会再缩到 2048/640）
    if let Ok(bytes) = photo_engine::thumbnail::decode_raw_embedded_thumb(path, u32::MAX) {
        tracing::info!("识别源: 内嵌 JPEG (快): {}", path.display());
        return Ok(bytes);
    }
    // 回退：完整解码（无内嵌或内嵌非 JPEG 的 RAW）
    let jpeg = photo_engine::thumbnail::decode_raw_preview(path, u32::MAX)
        .map_err(|e| RecognizeError::RawPreview(e.to_string()))?;
    Ok(jpeg)
}

// ---------------------------------------------------------------------------
// 单元测试
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use photo_domain::{BBox, ImageFormat, RecognitionFailureStage, RecognitionStatus, SourceFile};
    use std::path::PathBuf;

    /// 常驻缓存的前提：识别器要能跨到后台执行器线程（photo-ui 的
    /// SharedRecognizer 把它放进 Arc<Mutex<..>> 后 move 进 background_executor）。
    #[test]
    fn test_recognizer_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<Recognizer>();
    }

    /// 验证失败阶段 → 识别状态的映射表
    #[test]
    fn test_failure_stage_status_mapping() {
        // 检测无框 → Unrecognized(Detection)
        let r = Recognition {
            status: RecognitionStatus::Unrecognized,
            taxon: None,
            class_index: None,
            confidence: None,
            bbox: None,
            candidates: vec![],
            failure_stage: RecognitionFailureStage::Detection,
            recognized_at: String::new(),
            subjects: vec![],
        };
        assert_eq!(r.status, RecognitionStatus::Unrecognized);
        assert_eq!(r.failure_stage, RecognitionFailureStage::Detection);

        // 分类异常 → NeedsReview(Classification)
        let r = Recognition {
            status: RecognitionStatus::NeedsReview,
            taxon: None,
            class_index: None,
            confidence: None,
            bbox: None,
            candidates: vec![],
            failure_stage: RecognitionFailureStage::Classification,
            recognized_at: String::new(),
            subjects: vec![],
        };
        assert_eq!(r.status, RecognitionStatus::NeedsReview);
        assert_eq!(r.failure_stage, RecognitionFailureStage::Classification);

        // 映射失败 → NeedsReview(Mapping)
        let r = Recognition {
            status: RecognitionStatus::NeedsReview,
            taxon: None,
            class_index: None,
            confidence: None,
            bbox: None,
            candidates: vec![],
            failure_stage: RecognitionFailureStage::Mapping,
            recognized_at: String::new(),
            subjects: vec![],
        };
        assert_eq!(r.status, RecognitionStatus::NeedsReview);
        assert_eq!(r.failure_stage, RecognitionFailureStage::Mapping);

        // 资源不可用 → NeedsReview(Assets)
        let r = Recognition {
            status: RecognitionStatus::NeedsReview,
            taxon: None,
            class_index: None,
            confidence: None,
            bbox: None,
            candidates: vec![],
            failure_stage: RecognitionFailureStage::Assets,
            recognized_at: String::new(),
            subjects: vec![],
        };
        assert_eq!(r.status, RecognitionStatus::NeedsReview);
        assert_eq!(r.failure_stage, RecognitionFailureStage::Assets);

        // 全部成功 → Confirmed(None)
        let r = Recognition {
            status: RecognitionStatus::Confirmed,
            taxon: Some(photo_domain::TaxonMatch {
                taxon_id: Some(1),
                cn_name: "乌鸫".into(),
                latin_name: "Turdus merula".into(),
                cn_level: photo_domain::CnLevel::Species,
                ranks: vec![],
            }),
            class_index: Some(100),
            confidence: Some(95.5),
            bbox: Some(BBox::new(0.1, 0.2, 0.5, 0.6)),
            candidates: vec![],
            failure_stage: RecognitionFailureStage::None,
            recognized_at: "2026-07-28T12:00:00+00:00".into(),
            subjects: vec![],
        };
        assert_eq!(r.status, RecognitionStatus::Confirmed);
        assert_eq!(r.failure_stage, RecognitionFailureStage::None);
        assert!(r.taxon.is_some());
    }

    /// 输入源解析：标准 JPEG 直接解码路径
    #[test]
    fn test_resolve_source_jpeg() {
        // 创建临时 JPEG 文件
        let dir = tempfile::TempDir::new().unwrap();
        let img_path = dir.path().join("test.jpg");
        let img = image::RgbImage::from_pixel(100, 100, image::Rgb([128, 64, 32]));
        img.save(&img_path).unwrap();

        let capture = Capture {
            base_name: "test".into(),
            primary_index: 0,
            source_files: vec![SourceFile {
                path: img_path,
                format: ImageFormat::Jpeg,
                file_size: None,
            }],
        };

        let result = super::pipeline::resolve_source_with_thumbnail(&capture, None);
        assert!(result.is_ok(), "JPEG 应直接解码成功");
    }

    /// 输入源解析：不存在的文件 → Assets 失败
    #[test]
    fn test_resolve_source_nonexistent() {
        let capture = Capture {
            base_name: "missing".into(),
            primary_index: 0,
            source_files: vec![SourceFile {
                path: PathBuf::from("/nonexistent/photo.jpg"),
                format: ImageFormat::Jpeg,
                file_size: None,
            }],
        };

        let result = super::pipeline::resolve_source_with_thumbnail(&capture, None);
        assert!(result.is_err());
        let (stage, _msg) = result.err().unwrap();
        assert_eq!(stage, RecognitionFailureStage::Assets);
    }

    /// 输入源解析：无法解码的文件 → Assets 失败
    #[test]
    fn test_resolve_source_unreadable() {
        let dir = tempfile::TempDir::new().unwrap();
        let bad_path = dir.path().join("bad.jpg");
        // 写入非图片数据
        std::fs::write(&bad_path, b"not an image").unwrap();

        let capture = Capture {
            base_name: "bad".into(),
            primary_index: 0,
            source_files: vec![SourceFile {
                path: bad_path,
                format: ImageFormat::Jpeg,
                file_size: None,
            }],
        };

        let result = super::pipeline::resolve_source_with_thumbnail(&capture, None);
        assert!(result.is_err());
        let (stage, _msg) = result.err().unwrap();
        assert_eq!(stage, RecognitionFailureStage::Assets);
    }

    /// 纯单元测试：各 RecognizeError 变体可创建
    #[test]
    fn test_recognize_error_variants() {
        let _ = RecognizeError::ModelLoad("test".into());
        let _ = RecognizeError::ClassificationOutputEmpty;
        let _ = RecognizeError::RawPreview("test".into());
    }

    /// 检测模型缺失时应报 ModelLoad 错误（模型目录里什么都没有）
    #[test]
    fn test_detector_model_missing_returns_modelload_error() {
        let dir = tempfile::TempDir::new().unwrap();
        let models_dir = dir.path().join("models");
        std::fs::create_dir(&models_dir).unwrap();

        // 空模型目录：第一道检查就是 org_det.onnx
        let db_path = dir.path().join("bird_catalog.db");
        let result = Recognizer::new(&models_dir, &db_path);

        match result {
            Err(RecognizeError::ModelLoad(msg)) => {
                assert!(
                    msg.contains("org_det.onnx"),
                    "错误消息应提及 org_det.onnx, got: {msg}"
                );
            }
            Err(e) => panic!("期望 ModelLoad 错误，但得到: {e:?}"),
            Ok(_) => panic!("应返回错误但成功创建了 Recognizer"),
        }
    }

    /// 资产缺失时报可读的 ModelLoad（而不是 panic 或静默空标签）
    #[test]
    fn test_bioclip_assets_missing_dir_reports_modelload() {
        let dir = tempfile::TempDir::new().unwrap();
        match BioClipAssets::load(dir.path()) {
            Err(RecognizeError::ModelLoad(msg)) => {
                assert!(msg.contains("BioCLIP 资产缺失"), "错误消息应指明资产缺失, got: {msg}");
            }
            Err(other) => panic!("期望 ModelLoad 错误，实际其它错误: {other}"),
            Ok(_) => panic!("空目录不该加载成功"),
        }
    }

    /// #[ignore] 真实 BioCLIP 资产 + 模型端到端冒烟（手动触发）。
    ///
    /// 需要仓库根有 models/bioclip2_model_int8.onnx 与 data/taxon/（名录子集包），
    /// 缺任一个就跳过；跑法：cargo test -- --ignored -p photo-recognize -- real_bioclip
    #[test]
    #[ignore]
    fn test_real_bioclip_end_to_end_smoke() {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();
        let models_dir = workspace_root.join("models");
        let catalog_db = workspace_root.join("data").join("bird_catalog.db");
        let taxon_dir = workspace_root.join("data").join("taxon");
        if !models_dir.join(BIOCLIP_MODEL_FILE).exists()
            || !taxon_dir.join("txt_emb_bioclip-2.npy").exists()
        {
            eprintln!("SKIP: 缺少 BioCLIP 资产（models/bioclip2_model_int8.onnx 或 data/taxon/）");
            return;
        }

        // 资产本身：标签数与列数必须一致（不一致说明 npy/json 配错对）
        let assets = BioClipAssets::load(&taxon_dir).unwrap();
        assert!(assets.label_count() > 70_000, "中国包应有 9 万+ 类，实际 {}", assets.label_count());
        assert!(assets.label_count() < 851_968, "不该是全量包（全量与子集用同一套代码）");

        // 端到端：识别一个自建图（内容无意义，只验证链路不 panic 且给出物种结论）
        let mut recognizer =
            Recognizer::new(&models_dir, &catalog_db).unwrap();
        assert_eq!(recognizer.classifier_backend(), "bioclip");
        assert!(recognizer.asset_version().is_some(), "VERSION 应被读到");

        let dir = tempfile::TempDir::new().unwrap();
        let img_path = dir.path().join("smoke.jpg");
        let img = image::RgbImage::from_fn(640, 480, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, 128])
        });
        img.save(&img_path).unwrap();
        let capture = Capture {
            base_name: "bioclip_smoke".into(),
            primary_index: 0,
            source_files: vec![SourceFile {
                path: img_path,
                format: ImageFormat::Jpeg,
                file_size: None,
            }],
        };
        let rec = recognizer.recognize(&capture, None, None);
        match rec {
            Ok(r) => {
                eprintln!(
                    "BioCLIP 冒烟: status={:?} taxon={:?} conf={:?}",
                    r.status,
                    r.taxon.as_ref().map(|t| t.latin_name.clone()),
                    r.confidence
                );
                // 标签空间就是物种清单，所以有结论是必然的；无结论说明检索/映射断了
                let t = r.taxon.expect("BioCLIP 后端应给出物种结论");
                assert!(!t.latin_name.is_empty());
                assert!(!t.ranks.is_empty(), "七级分类应来自标签路径");
            }
            Err(e) => panic!("BioCLIP 端到端应返回 Ok（业务失败体现在 status）: {e}"),
        }
    }

}
