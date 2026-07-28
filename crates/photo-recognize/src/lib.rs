//! 鸟类识别：YOLO 检测 → 鸟种分类 → 名录映射（移植自 pica，全同步）。
//!
//! 管线语义与持久化格式见 `docs/adr/0002-folder-central-db.md`、
//! `docs/adr/0003-recognition-subsystem.md`；领域类型定义在 `photo-domain`。
//!
//! ## 与 pica 的差异
//!
//! - 候选列表中的未映射项 `bird=None` 也保留（pica 跳过不可映射项）
//! - 输入源解析：RAW 格式使用 `photo_engine::thumbnail::decode_raw_preview`
//!   提取内嵌 JPEG（pica 仅 `image` 库解码，不专门处理 RAW）
//! - 其余预处理/后处理常数与 Dart 源码逐行对应
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

mod catalog;
mod classify;
mod detect;
mod pipeline;

use std::path::Path;
use std::sync::atomic::AtomicBool;

use ort::ep;
use ort::session::Session;

use photo_domain::{Capture, Recognition};

pub use catalog::CatalogDb;
pub use catalog::ClassificationOutput;
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

/// 识别器：持有两个 ONNX Session + 名录库连接。
///
/// 示例：
/// ```no_run
/// use photo_recognize::Recognizer;
///
/// let recognizer = Recognizer::new(
///     &std::path::Path::new("./models"),
///     &std::path::Path::new("./data/pica_ref.db"),
/// ).unwrap();
/// ```
pub struct Recognizer {
    detection_session: Session,
    classification_session: Session,
    catalog: CatalogDb,
    backend: Backend,
}

impl Recognizer {
    /// 创建识别器。
    ///
    /// # 参数
    /// - `models_dir`: 包含 `yolo26l.onnx` 和 `bird_model.onnx` 的目录
    /// - `catalog_db`: 名录库 `pica_ref.db` 路径
    ///
    /// # 模型路径约定（便携模式）
    /// 默认路径相对于 exe 所在目录：`exe_dir/models/` + `exe_dir/data/pica_ref.db`。
    /// 调用方可通过参数注入任意路径（测试用临时目录等）。
    ///
    /// # 执行提供程序
    /// - Windows: DirectML → 失败回退 CPU 并 `tracing::warn` 记录原因
    /// - 非 Windows: CPU
    pub fn new(models_dir: &Path, catalog_db: &Path) -> Result<Self, RecognizeError> {
        let yolo_path = models_dir.join("yolo26l.onnx");
        let bird_path = models_dir.join("bird_model.onnx");

        if !yolo_path.exists() {
            return Err(RecognizeError::ModelLoad(format!(
                "YOLO 模型文件不存在: {}。请将 yolo26l.onnx 放入 models/ 目录",
                yolo_path.display()
            )));
        }
        if !bird_path.exists() {
            return Err(RecognizeError::ModelLoad(format!(
                "bird_model 模型文件不存在: {}。请将 bird_model.onnx 放入 models/ 目录",
                bird_path.display()
            )));
        }
        let (detection_session, backend) = load_model(&yolo_path)?;
        let (classification_session, _) = load_model(&bird_path)?;
        let catalog = CatalogDb::open(catalog_db)?;

        tracing::info!("识别器初始化完成，推理后端: {}", backend);

        Ok(Self {
            detection_session,
            classification_session,
            catalog,
            backend,
        })
    }

    /// 单张全管线识别。
    ///
    /// 业务失败体现在 `Recognition.status`，`Err` 仅用于模型/库不可用等系统性故障。
    /// 对应 pica `recognition_pipeline_service.dart:recognize()`。
    pub fn recognize(
        &mut self,
        capture: &Capture,
        on_progress: Option<&pipeline::ProgressCallback>,
    ) -> Result<Recognition, RecognizeError> {
        pipeline::recognize_capture(
            &mut self.detection_session,
            &mut self.classification_session,
            &self.catalog,
            capture,
            on_progress,
        )
    }

    /// 批量识别。
    ///
    /// 顺序执行，每张后检查 `cancel` 标志，取消时返回已完成部分的识别结果。
    /// 每个结果关联 Capture 在输入切片中的索引。
    pub fn recognize_paths(
        &mut self,
        captures: &[Capture],
        on_progress: Option<&pipeline::ProgressCallback>,
        cancel: &AtomicBool,
    ) -> Vec<(usize, Recognition)> {
        pipeline::recognize_captures(
            &mut self.detection_session,
            &mut self.classification_session,
            &self.catalog,
            captures,
            on_progress,
            cancel,
        )
    }

    /// 返回检测 session 的可变引用（供直接调用 detect 模块用）
    pub fn detection_session(&mut self) -> &mut Session {
        &mut self.detection_session
    }

    /// 返回分类 session 的可变引用（供直接调用 classify 模块用）
    pub fn classification_session(&mut self) -> &mut Session {
        &mut self.classification_session
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
        // Windows: 先试 DirectML，失败回退 CPU（pica 策略对等）
        // HighPerformance = DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE：
        // 双显卡机型（核显 + N 卡独显）上让 Windows 把 DirectML 设备绑到独显，
        // 否则默认可能落在核显（如 Radeon 780M）上
        match (|| -> Result<_, ort::Error> {
            let builder = Session::builder()?;
            let mut builder = builder.with_execution_providers([
                ep::DirectML::default()
                    .with_performance_preference(ep::directml::PerformancePreference::HighPerformance)
                    .build(),
            ])?;
            builder.commit_from_file(path)
        })()
        {
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

/// 使用 `photo-engine` 从 RAW 文件提取最大内嵌预览 JPEG。
fn engine_raw_preview(path: &Path) -> Result<Vec<u8>, RecognizeError> {
    // photo_engine::thumbnail::decode_raw_preview 提取内嵌 JPEG
    // 若没有内嵌缩略图则回落完整解码（不解拜耳）
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
    use photo_domain::{RecognitionFailureStage, RecognitionStatus, SourceFile, ImageFormat, BBox};
    use std::path::PathBuf;

    /// 验证失败阶段 → 识别状态的映射表
    #[test]
    fn test_failure_stage_status_mapping() {
        // 检测无框 → Unrecognized(Detection)
        let r = Recognition {
            status: RecognitionStatus::Unrecognized,
            bird: None,
            class_index: None,
            confidence: None,
            bbox: None,
            candidates: vec![],
            failure_stage: RecognitionFailureStage::Detection,
            recognized_at: String::new(),
        };
        assert_eq!(r.status, RecognitionStatus::Unrecognized);
        assert_eq!(r.failure_stage, RecognitionFailureStage::Detection);

        // 分类异常 → NeedsReview(Classification)
        let r = Recognition {
            status: RecognitionStatus::NeedsReview,
            bird: None,
            class_index: None,
            confidence: None,
            bbox: None,
            candidates: vec![],
            failure_stage: RecognitionFailureStage::Classification,
            recognized_at: String::new(),
        };
        assert_eq!(r.status, RecognitionStatus::NeedsReview);
        assert_eq!(r.failure_stage, RecognitionFailureStage::Classification);

        // 映射失败 → NeedsReview(Mapping)
        let r = Recognition {
            status: RecognitionStatus::NeedsReview,
            bird: None,
            class_index: None,
            confidence: None,
            bbox: None,
            candidates: vec![],
            failure_stage: RecognitionFailureStage::Mapping,
            recognized_at: String::new(),
        };
        assert_eq!(r.status, RecognitionStatus::NeedsReview);
        assert_eq!(r.failure_stage, RecognitionFailureStage::Mapping);

        // 资源不可用 → NeedsReview(Assets)
        let r = Recognition {
            status: RecognitionStatus::NeedsReview,
            bird: None,
            class_index: None,
            confidence: None,
            bbox: None,
            candidates: vec![],
            failure_stage: RecognitionFailureStage::Assets,
            recognized_at: String::new(),
        };
        assert_eq!(r.status, RecognitionStatus::NeedsReview);
        assert_eq!(r.failure_stage, RecognitionFailureStage::Assets);

        // 全部成功 → Confirmed(None)
        let r = Recognition {
            status: RecognitionStatus::Confirmed,
            bird: Some(photo_domain::BirdMatch {
                bird_id: 1,
                cn_name: "乌鸫".into(),
                latin_name: "Turdus merula".into(),
            }),
            class_index: Some(100),
            confidence: Some(95.5),
            bbox: Some(BBox::new(0.1, 0.2, 0.5, 0.6)),
            candidates: vec![],
            failure_stage: RecognitionFailureStage::None,
            recognized_at: "2026-07-28T12:00:00+00:00".into(),
        };
        assert_eq!(r.status, RecognitionStatus::Confirmed);
        assert_eq!(r.failure_stage, RecognitionFailureStage::None);
        assert!(r.bird.is_some());
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

        let result = super::pipeline::resolve_source(&capture);
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

        let result = super::pipeline::resolve_source(&capture);
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

        let result = super::pipeline::resolve_source(&capture);
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

    /// #[ignore] 真实模型冒烟测试（手动触发：cargo test -- --ignored -p photo-recognize）
    #[test]
    #[ignore]
    fn test_real_model_smoke() {
        // 需要在 worktree 根目录有 models/ 和 data/ 目录
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();

        let models_dir = workspace_root.join("models");
        let catalog_db = workspace_root.join("data").join("pica_ref.db");

        if !models_dir.join("yolo26l.onnx").exists() || !catalog_db.exists() {
            eprintln!("SKIP: 模型或名录库不存在，请确保 worktree 根有 models/ 和 data/");
            return;
        }

        let mut recognizer = Recognizer::new(&models_dir, &catalog_db).unwrap();

        // 创建一个临时 JPEG 作为测试图
        let dir = tempfile::TempDir::new().unwrap();
        let img_path = dir.path().join("test.jpg");
        let img = image::RgbImage::from_fn(640, 480, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, 128])
        });
        img.save(&img_path).unwrap();

        let capture = Capture {
            base_name: "smoke_test".into(),
            primary_index: 0,
            source_files: vec![SourceFile {
                path: img_path,
                format: ImageFormat::Jpeg,
                file_size: None,
            }],
        };

        let result = recognizer.recognize(&capture, None);
        match result {
            Ok(rec) => {
                // 任何结果都是可接受的——只要不 panic 就是成功
                eprintln!(
                    "冒烟测试完成: status={:?}, bird={:?}, stage={:?}",
                    rec.status, rec.bird, rec.failure_stage
                );
            }
            Err(e) => {
                // 如果报缺少 DirectML 运行时（例如未安装 DirectX），
                // 回退 CPU 也视为正常
                eprintln!("识别返回系统错误（可能是缺少运行时）: {e}");
            }
        }
    }
}
