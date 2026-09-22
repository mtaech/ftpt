//! 分类器后端接缝。
//!
//! 当前唯一实现是 BioCLIP（`bioclip.rs`）：名录子集余弦检索 → 标签本身就是学名 →
//! 名录只做补充。接缝保留是为了后续换/加通用分类模型时管线零改动——
//! 管线（检测、分类、进度、取消、落库）只认 `Classified`，不关心后端实现。

use image::DynamicImage;
use photo_domain::{BBox, RecognitionFailureStage, TaxonCandidate, TaxonMatch};

use crate::catalog::CatalogDb;
use crate::RecognizeError;

/// 一次分类 + 落到物种的结果。
pub struct Classified {
    /// Top-1 物种结论（None = 没落到任何物种）
    pub taxon: Option<TaxonMatch>,
    /// Top-1 识别器类别号（诊断用）
    pub class_index: Option<u32>,
    /// Top-1 置信度（0–100；两个后端语义不同，见各自实现）
    pub confidence: Option<f32>,
    /// 其余候选（不含 Top-1）
    pub candidates: Vec<TaxonCandidate>,
    /// 失败阶段：None = 有结论；Classification = 推理失败；Mapping = 推理成功但没落到物种
    pub failure: RecognitionFailureStage,
}

/// 识别后端。
///
/// `Send`：识别器要常驻缓存（`photo-ui` 的批量识别与框选识别共用同一个实例，
/// 装配一次约 1-2s），实例会跨到后台执行器线程。
pub trait Classifier: Send {
    /// 对给定区域分类并落到物种。
    ///
    /// 推理层故障返回 Err（管线映射为 NeedsReview(Classification)）；
    /// 业务失败（没映射上）体现在 `Classified::failure`。
    fn classify(
        &mut self,
        catalog: &CatalogDb,
        img: &DynamicImage,
        bbox: BBox,
    ) -> Result<Classified, RecognizeError>;

    /// 稳定标识：进日志与结果诊断（当前恒为 `"bioclip"`）
    fn backend(&self) -> &'static str;

    /// 资产版本（`class_index` 的语义依赖它；没有版本信息时为 None）
    fn asset_version(&self) -> Option<String> {
        None
    }

    /// 检测阶段一个框都没给时，是否退回**整图**识别。
    ///
    /// BioCLIP 返回 true：单类鸟检测器对花卉/昆虫/真菌/静物根本不给框，
    /// 而整图直接分类本来就是这个工具的默认用法（见 bioclip_demo README「整图识别」）。
    fn whole_image_on_no_detection(&self) -> bool {
        false
    }
}

