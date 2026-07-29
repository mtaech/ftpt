use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 图片格式枚举
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ImageFormat {
    Jpeg,
    Png,
    Tiff,
    Heif,
    WebP,
    Bmp,
    Gif,
    Raw(String), // RAW 扩展名（如 "NEF"、"CR2"）
}

impl std::fmt::Display for ImageFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Jpeg => write!(f, "JPEG"),
            Self::Png => write!(f, "PNG"),
            Self::Tiff => write!(f, "TIFF"),
            Self::Heif => write!(f, "HEIF"),
            Self::WebP => write!(f, "WebP"),
            Self::Bmp => write!(f, "BMP"),
            Self::Gif => write!(f, "GIF"),
            Self::Raw(r) => write!(f, "{}", r),
        }
    }
}

impl ImageFormat {
    /// 从文件扩展名推断格式
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_lowercase().as_str() {
            "jpg" | "jpeg" => Some(Self::Jpeg),
            "png" => Some(Self::Png),
            "tif" | "tiff" => Some(Self::Tiff),
            "heif" | "heic" => Some(Self::Heif),
            "webp" => Some(Self::WebP),
            "bmp" => Some(Self::Bmp),
            "gif" => Some(Self::Gif),
            raw if Self::is_raw_extension(raw) => Some(Self::Raw(raw.to_uppercase())),
            _ => None,
        }
    }

    /// 判断是否是可查看图片格式（包括 RAW）
    pub fn is_viewable(ext: &str) -> bool {
        Self::from_extension(ext).is_some()
    }

    /// RAW 扩展名白名单
    pub fn is_raw_extension(ext: &str) -> bool {
        matches!(
            ext.to_lowercase().as_str(),
            "nef"
                | "nrw"
                | "cr2"
                | "cr3"
                | "arw"
                | "srf"
                | "sr2"
                | "dng"
                | "orf"
                | "raf"
                | "pef"
                | "rw2"
                | "raw"
                | "3fr"
                | "ari"
                | "bay"
                | "cap"
                | "dcr"
                | "drf"
                | "eip"
                | "erf"
                | "fff"
                | "iiq"
                | "k25"
                | "kdc"
                | "mdc"
                | "mef"
                | "mos"
                | "mrw"
                | "ndf"
                | "obm"
                | "ori"
                | "ptx"
                | "pxn"
                | "r3d"
                | "rwl"
                | "rwz"
                | "srw"
                | "x3f"
        )
    }
}

/// 组成一次拍摄的单个源文件
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceFile {
    pub path: PathBuf,
    pub format: ImageFormat,
    pub file_size: Option<u64>,
}

/// 一次快门产生的拍摄
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capture {
    pub base_name: String,
    pub source_files: Vec<SourceFile>,
    pub primary_index: usize,
}

/// 发送到前端的拍摄摘要（轻量，不含完整 SourceFile）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureMeta {
    pub index: usize,
    pub base_name: String,
    pub primary_path: String,
    pub primary_format: String,
    pub stack_count: usize,
    pub file_size: Option<u64>,
    pub date_taken: Option<String>,
    pub extensions: Vec<String>,
    // --- EXIF 摘要字段（可延迟填充） ---
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub lens: Option<String>,
    pub exposure_time: Option<String>,
    pub f_number: Option<String>,
    pub iso: Option<u32>,
    pub focal_length: Option<String>,
    pub image_width: Option<u32>,
    pub image_height: Option<u32>,
    // --- 评分/色标/旗标字段（从文件夹数据库 xmp_meta 表填充） ---
    pub rating: Rating,
    pub color_label: ColorLabel,
    pub flag: Option<Flag>,
    // --- 识别摘要字段（从文件夹数据目录的 recognition 表填充，None = 未识别） ---
    pub bird_name: Option<String>,
    pub bird_confidence: Option<f32>,
    pub recognition_status: Option<RecognitionStatus>,
    pub bird_bbox: Option<BBox>,
}

impl From<&Capture> for CaptureMeta {
    fn from(c: &Capture) -> Self {
        let primary = &c.source_files[c.primary_index];
        let ext_list: Vec<String> = c
            .source_files
            .iter()
            .map(|f| {
                f.path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|s| s.to_uppercase())
                    .unwrap_or_default()
            })
            .collect();
        Self {
            index: 0,
            base_name: c.base_name.clone(),
            primary_path: primary.path.to_string_lossy().to_string(),
            primary_format: primary.format.to_string(),
            stack_count: c
                .source_files
                .iter()
                .enumerate()
                .filter(|(i, _f)| *i != c.primary_index)
                .count(),
            file_size: primary.file_size,
            date_taken: None,
            extensions: ext_list,
            camera_make: None,
            camera_model: None,
            lens: None,
            exposure_time: None,
            f_number: None,
            iso: None,
            focal_length: None,
            image_width: None,
            image_height: None,
            rating: Rating::None,
            color_label: ColorLabel::None,
            flag: None,
            bird_name: None,
            bird_confidence: None,
            recognition_status: None,
            bird_bbox: None,
        }
    }
}

impl CaptureMeta {
    /// 填充 EXIF 摘要字段（由调用方负责提取 ExifMetadata，本方法只做字段拷贝）
    pub fn enrich_with_exif(&mut self, exif: &ExifMetadata) {
        self.camera_make = exif.camera.make.clone();
        self.camera_model = exif.camera.model.clone();
        self.lens = exif.camera.lens.clone();
        self.exposure_time = exif.shooting.exposure_time.clone();
        self.f_number = exif.shooting.f_number.clone();
        self.iso = exif.shooting.iso;
        self.focal_length = exif.shooting.focal_length.clone();
        self.image_width = exif.image_width;
        self.image_height = exif.image_height;
        self.date_taken = exif.date_time_original.clone();
        // 如果 EXIF 没有文件大小，回退到 fs::metadata
        if self.file_size.is_none() {
            self.file_size = std::fs::metadata(std::path::Path::new(&self.primary_path))
                .ok()
                .map(|m| m.len());
        }
    }

    /// 填充评分/颜色标签/旗标（由调用方负责从文件夹数据库读取 XmpMetadata）
    pub fn enrich_with_xmp(&mut self, xmp: &XmpMetadata) {
        self.rating = xmp.rating();
        self.color_label = xmp.color_label();
        self.flag = xmp.flag();
    }

    /// 填充识别摘要字段（由调用方负责从文件夹数据目录读取 Recognition）
    pub fn enrich_with_recognition(&mut self, recognition: &Recognition) {
        self.bird_name = recognition.bird.as_ref().map(|b| b.cn_name.clone());
        self.bird_confidence = recognition.confidence;
        self.recognition_status = Some(recognition.status);
        self.bird_bbox = recognition.bbox;
    }
}

/// 评分
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Rating {
    None = 0,
    One = 1,
    Two = 2,
    Three = 3,
    Four = 4,
    Five = 5,
}

/// 颜色标签
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColorLabel {
    None,
    Red,
    Yellow,
    Green,
    Blue,
    Purple,
}

/// Pick/Reject 旗标
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Flag {
    Pick,
    Reject,
}

/// 筛选条件
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterCriteria {
    /// 按文件类型过滤
    pub format_filter: Option<ImageFormat>,
    /// 按鸟种中文名过滤（多选，空 = 不过滤）
    #[serde(default)]
    pub bird_names: Vec<String>,
    /// 按日期范围过滤
    pub date_from: Option<chrono::NaiveDate>,
    pub date_to: Option<chrono::NaiveDate>,
    /// 按评分过滤
    pub min_rating: Option<Rating>,
    /// 按颜色标签过滤
    pub color_label: Option<ColorLabel>,
    /// 按旗标过滤（Pick/Reject/None）
    pub flag_filter: Option<Flag>,
    /// 如果为 true，只显示没有标记旗标的照片
    pub unflagged_filter: bool,
    /// 按识别状态过滤（含"未识别"= 无识别记录）
    #[serde(default)]
    pub recognition_filter: RecognitionFilter,
}

/// 排序方式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortBy {
    FileName,
    DateTaken,
    FileSize,
    Rating,
    Modified,
}

/// 排序方向
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortDirection {
    Ascending,
    Descending,
}

/// 删除模式（仅支持移到回收站）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeleteMode {
    Trash,
}

/// 批量文件操作类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BatchOpType {
    /// 复制同名文件到目标目录
    CopySame,
    /// 复制非同名文件到目标目录
    CopyNotSame,
    /// 删除同名文件
    DeleteSame,
    /// 删除非同名文件
    DeleteNotSame,
    /// 移动同名文件到目标目录
    MoveSame,
    /// 移动非同名文件到目标目录
    MoveNotSame,
}

impl BatchOpType {
    /// 全部操作类型（UI 下拉用）
    pub fn all() -> &'static [BatchOpType] {
        &[
            Self::CopySame,
            Self::CopyNotSame,
            Self::DeleteSame,
            Self::DeleteNotSame,
            Self::MoveSame,
            Self::MoveNotSame,
        ]
    }

    /// 是否为"同名"匹配
    pub fn is_same_match(&self) -> bool {
        matches!(self, Self::CopySame | Self::DeleteSame | Self::MoveSame)
    }

    /// 是否需要目标目录（删除操作不需要）
    pub fn needs_target_dir(&self) -> bool {
        matches!(self, Self::CopySame | Self::CopyNotSame | Self::MoveSame | Self::MoveNotSame)
    }

    /// 执行的动作标签
    pub fn action_label(&self) -> &'static str {
        match self {
            Self::CopySame | Self::CopyNotSame => "复制",
            Self::DeleteSame | Self::DeleteNotSame => "删除",
            Self::MoveSame | Self::MoveNotSame => "移动",
        }
    }
}

impl std::fmt::Display for BatchOpType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CopySame => write!(f, "复制同名文件"),
            Self::CopyNotSame => write!(f, "复制非同名文件"),
            Self::DeleteSame => write!(f, "删除同名文件"),
            Self::DeleteNotSame => write!(f, "删除非同名文件"),
            Self::MoveSame => write!(f, "移动同名文件"),
            Self::MoveNotSame => write!(f, "移动非同名文件"),
        }
    }
}

// ============================================================================
// Exif 数据类型（纯结构体，提取/读写机械在 photo-engine 中）
// ============================================================================

/// 相机制造商信息
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraInfo {
    pub make: Option<String>,
    pub model: Option<String>,
    pub lens: Option<String>,
}

/// 拍摄参数
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShootingParams {
    pub exposure_time: Option<String>,
    pub f_number: Option<String>,
    pub iso: Option<u32>,
    pub focal_length: Option<String>,
    pub exposure_compensation: Option<String>,
    pub white_balance: Option<String>,
}

/// GPS 信息
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GpsInfo {
    pub latitude: Option<(f64, f64, f64)>,
    pub longitude: Option<(f64, f64, f64)>,
    pub altitude: Option<f64>,
}

/// 完整的 EXIF 元数据
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExifMetadata {
    pub camera: CameraInfo,
    pub shooting: ShootingParams,
    pub gps: GpsInfo,
    pub date_time_original: Option<String>,
    pub image_width: Option<u32>,
    pub image_height: Option<u32>,
    pub file_size: Option<u64>,
    pub color_space: Option<String>,
    pub orientation: Option<u16>,
}

impl ExifMetadata {
    /// 生成格式化摘要文本
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        if let Some(ref make) = self.camera.make {
            parts.push(make.clone());
        }
        if let Some(ref model) = self.camera.model {
            parts.push(model.clone());
        }
        if let Some(ref date) = self.date_time_original {
            parts.push(format!("拍摄: {}", date));
        }
        if let Some(ref exp) = self.shooting.exposure_time {
            parts.push(format!("快门: {}", exp));
        }
        if let Some(ref fnum) = self.shooting.f_number {
            parts.push(format!("光圈: {}", fnum));
        }
        if let Some(iso) = self.shooting.iso {
            parts.push(format!("ISO: {}", iso));
        }
        if let Some(ref focal) = self.shooting.focal_length {
            parts.push(format!("焦距: {}", focal));
        }
        if let (Some(w), Some(h)) = (self.image_width, self.image_height) {
            parts.push(format!("{}×{}", w, h));
        }
        parts.join(" | ")
    }
}

// ============================================================================
// 识别数据类型（纯结构体，推理机械在 photo-recognize 中，持久化机械在 photo-engine 中）
// ============================================================================

/// 检测框：归一化坐标 [x1, y1, x2, y2]（0–1，相对图像宽高）
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BBox {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
}

impl BBox {
    /// 构造并夹紧到 0–1 范围
    pub fn new(x1: f32, y1: f32, x2: f32, y2: f32) -> Self {
        let clamp01 = |v: f32| v.clamp(0.0, 1.0);
        Self {
            x1: clamp01(x1),
            y1: clamp01(y1),
            x2: clamp01(x2),
            y2: clamp01(y2),
        }
    }

    /// 解析数据库文本格式 "x1,y1,x2,y2"
    pub fn parse(s: &str) -> Option<Self> {
        let parts: Vec<f32> = s.split(',').filter_map(|p| p.trim().parse().ok()).collect();
        if parts.len() == 4 {
            Some(Self::new(parts[0], parts[1], parts[2], parts[3]))
        } else {
            None
        }
    }

    /// 序列化为数据库文本格式 "x1,y1,x2,y2"
    pub fn to_db_string(&self) -> String {
        format!("{},{},{},{}", self.x1, self.y1, self.x2, self.y2)
    }
}

/// 识别状态（三态；无识别记录 = 未识别，不占枚举值）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecognitionStatus {
    /// 检测、分类、名录映射全部成功
    Confirmed,
    /// 管线中途失败（分类异常 / 名录映射失败 / 源图不可用），需人工复核
    NeedsReview,
    /// 检测阶段未发现鸟
    Unrecognized,
}

impl RecognitionStatus {
    /// 数据库存储文本
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Confirmed => "confirmed",
            Self::NeedsReview => "needs_review",
            Self::Unrecognized => "unrecognized",
        }
    }

    /// 从数据库文本解析
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "confirmed" => Some(Self::Confirmed),
            "needs_review" => Some(Self::NeedsReview),
            "unrecognized" => Some(Self::Unrecognized),
            _ => None,
        }
    }
}

/// 识别失败阶段
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecognitionFailureStage {
    None,
    Detection,
    Classification,
    Mapping,
    Assets,
}

impl RecognitionFailureStage {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Detection => "detection",
            Self::Classification => "classification",
            Self::Mapping => "mapping",
            Self::Assets => "assets",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "none" => Some(Self::None),
            "detection" => Some(Self::Detection),
            "classification" => Some(Self::Classification),
            "mapping" => Some(Self::Mapping),
            "assets" => Some(Self::Assets),
            _ => None,
        }
    }

    /// 面向用户的失败原因说明
    pub fn user_message(&self) -> &'static str {
        match self {
            Self::None => "",
            Self::Detection => "检测异常",
            Self::Classification => "分类异常",
            Self::Mapping => "名录映射失败",
            Self::Assets => "源图不可用",
        }
    }
}

/// 鸟种匹配：分类器类别号经名录库映射到的具体鸟种
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BirdMatch {
    /// 名录库 animal_info 主键
    pub bird_id: i64,
    /// 中文名
    pub cn_name: String,
    /// 学名（拉丁名）
    pub latin_name: String,
}

/// Top-N 候选（含未映射项：bird 为 None 表示该类别号未映射到名录）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BirdCandidate {
    /// bird_model 原始类别号
    pub class_index: u32,
    /// 置信度（0–100）
    pub confidence: f32,
    /// 映射到的鸟种（未映射为 None）
    pub bird: Option<BirdMatch>,
}

/// 一次识别的完整结果（不含路径；路径是持久化层的键）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Recognition {
    pub status: RecognitionStatus,
    /// Top-1 鸟种匹配（mapping 失败 / 未检出时为 None）
    pub bird: Option<BirdMatch>,
    /// Top-1 原始类别号（诊断用）
    pub class_index: Option<u32>,
    /// Top-1 置信度（0–100）
    pub confidence: Option<f32>,
    /// 检测框（检测失败为 None）
    pub bbox: Option<BBox>,
    /// 鸟眼锐度分（连续，仅保证单调性，阈值后置；无鸟/有鸟无眼/评分失败为 None）
    pub eye_sharpness: Option<f32>,
    /// 评分所用眼框（归一化坐标，相对全图；无锐度分为 None）
    pub eye_bbox: Option<BBox>,
    /// Top-5 候选（含 Top-1 自身除外的备选；分类失败为空）
    pub candidates: Vec<BirdCandidate>,
    pub failure_stage: RecognitionFailureStage,
    /// ISO8601 时间戳
    pub recognized_at: String,
}

/// 识别状态筛选条件
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecognitionFilter {
    /// 全部（不筛选）
    #[default]
    All,
    Confirmed,
    NeedsReview,
    Unrecognized,
    /// 未识别（无识别记录）
    NotRecognized,
}

// ============================================================================
// XMP 数据类型（纯结构体 + 枚举转换，读写机械在 photo-engine 中）
// ============================================================================

/// XMP 中存储的 PT 元数据
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct XmpMetadata {
    /// 评分 (0-5)
    pub rating: u8,
    /// 颜色标签 (空字符串表示无)
    pub color_label: String,
    /// 旗标: "" | "pick" | "reject"
    pub flag: String,
}

impl XmpMetadata {
    pub fn rating(&self) -> Rating {
        match self.rating {
            1 => Rating::One,
            2 => Rating::Two,
            3 => Rating::Three,
            4 => Rating::Four,
            5 => Rating::Five,
            _ => Rating::None,
        }
    }

    pub fn set_rating(&mut self, rating: Rating) {
        self.rating = rating as u8;
    }

    pub fn color_label(&self) -> ColorLabel {
        match self.color_label.as_str() {
            "red" => ColorLabel::Red,
            "yellow" => ColorLabel::Yellow,
            "green" => ColorLabel::Green,
            "blue" => ColorLabel::Blue,
            "purple" => ColorLabel::Purple,
            _ => ColorLabel::None,
        }
    }

    pub fn set_color_label(&mut self, label: ColorLabel) {
        self.color_label = match label {
            ColorLabel::Red => "red".into(),
            ColorLabel::Yellow => "yellow".into(),
            ColorLabel::Green => "green".into(),
            ColorLabel::Blue => "blue".into(),
            ColorLabel::Purple => "purple".into(),
            ColorLabel::None => "".into(),
        };
    }

    pub fn flag(&self) -> Option<Flag> {
        match self.flag.as_str() {
            "pick" => Some(Flag::Pick),
            "reject" => Some(Flag::Reject),
            _ => None,
        }
    }

    pub fn set_flag(&mut self, flag: Option<Flag>) {
        self.flag = match flag {
            Some(Flag::Pick) => "pick".into(),
            Some(Flag::Reject) => "reject".into(),
            None => "".into(),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_from_jpg() {
        assert_eq!(ImageFormat::from_extension("jpg"), Some(ImageFormat::Jpeg));
        assert_eq!(ImageFormat::from_extension("JPEG"), Some(ImageFormat::Jpeg));
    }

    #[test]
    fn test_format_from_raw() {
        assert_eq!(
            ImageFormat::from_extension("NEF"),
            Some(ImageFormat::Raw("NEF".to_string()))
        );
    }

    #[test]
    fn test_raw_extension_whitelist() {
        assert!(ImageFormat::is_raw_extension("NEF"));
        assert!(ImageFormat::is_raw_extension("cr2"));
        assert!(ImageFormat::is_raw_extension("DNG"));
        assert!(!ImageFormat::is_raw_extension("jpg"));
        assert!(!ImageFormat::is_raw_extension("mp4"));
    }


    #[test]
    fn test_format_from_png() {
        assert_eq!(ImageFormat::from_extension("png"), Some(ImageFormat::Png));
    }

    #[test]
    fn test_format_from_tiff() {
        assert_eq!(ImageFormat::from_extension("tif"), Some(ImageFormat::Tiff));
        assert_eq!(ImageFormat::from_extension("TIFF"), Some(ImageFormat::Tiff));
    }

    #[test]
    fn test_format_gif() {
        assert_eq!(ImageFormat::from_extension("gif"), Some(ImageFormat::Gif));
    }

    #[test]
    fn test_invalid_extension() {
        assert_eq!(ImageFormat::from_extension("txt"), None);
        assert_eq!(ImageFormat::from_extension("mp4"), None);
    }

    #[test]
    fn test_is_viewable() {
        assert!(ImageFormat::is_viewable("jpg"));
        assert!(ImageFormat::is_viewable("nef"));
        assert!(!ImageFormat::is_viewable("mp4"));
        assert!(!ImageFormat::is_viewable("txt"));
    }

    #[test]
    fn test_enrich_with_xmp_fills_fields() {
        let mut xmp = XmpMetadata::default();
        xmp.set_rating(Rating::Three);
        xmp.set_color_label(ColorLabel::Green);
        xmp.set_flag(Some(Flag::Pick));

        let mut cm = CaptureMeta {
            index: 0,
            base_name: "IMG_0001".into(),
            primary_path: "/tmp/IMG_0001.jpg".into(),
            primary_format: "JPEG".into(),
            stack_count: 0,
            file_size: None,
            date_taken: None,
            extensions: vec![],
            camera_make: None,
            camera_model: None,
            lens: None,
            exposure_time: None,
            f_number: None,
            iso: None,
            focal_length: None,
            image_width: None,
            image_height: None,
            rating: Rating::None,
            color_label: ColorLabel::None,
            flag: None,
            bird_name: None,
            bird_confidence: None,
            recognition_status: None,
            bird_bbox: None,
        };
        cm.enrich_with_xmp(&xmp);
        assert_eq!(cm.rating, Rating::Three);
        assert_eq!(cm.color_label, ColorLabel::Green);
        assert_eq!(cm.flag, Some(Flag::Pick));
    }

    #[test]
    fn test_xmp_metadata_default() {
        let m = XmpMetadata::default();
        assert_eq!(m.rating, 0);
        assert_eq!(m.color_label, "");
        assert_eq!(m.flag, "");
    }

    #[test]
    fn test_xmp_rating_conversion() {
        let mut m = XmpMetadata::default();
        m.set_rating(Rating::Four);
        assert_eq!(m.rating, 4);
        assert_eq!(m.rating(), Rating::Four);
    }

    #[test]
    fn test_xmp_color_label_conversion() {
        let mut m = XmpMetadata::default();
        m.set_color_label(ColorLabel::Red);
        assert_eq!(m.color_label, "red");
        assert_eq!(m.color_label(), ColorLabel::Red);
    }

    #[test]
    fn test_xmp_flag_conversion() {
        let mut m = XmpMetadata::default();
        m.set_flag(Some(Flag::Pick));
        assert_eq!(m.flag, "pick");
        assert_eq!(m.flag(), Some(Flag::Pick));

        m.set_flag(None);
        assert_eq!(m.flag, "");
        assert_eq!(m.flag(), None);
    }

    #[test]
    fn test_exif_default_metadata_is_empty() {
        let meta = ExifMetadata::default();
        assert!(meta.camera.make.is_none());
        assert!(meta.camera.model.is_none());
        assert!(meta.shooting.iso.is_none());
        assert!(meta.image_width.is_none());
        assert!(meta.image_height.is_none());
        assert!(meta.file_size.is_none());
    }

    #[test]
    fn test_exif_summary_empty_returns_empty_string() {
        let meta = ExifMetadata::default();
        let summary = meta.summary();
        assert_eq!(summary, "");
    }

    #[test]
    fn test_bbox_db_string_roundtrip() {
        let b = BBox::new(0.1, 0.2, 0.8, 0.9);
        let parsed = BBox::parse(&b.to_db_string()).unwrap();
        assert_eq!(parsed, b);
    }

    #[test]
    fn test_bbox_new_clamps_to_unit_range() {
        let b = BBox::new(-0.5, 0.2, 1.3, 0.9);
        assert_eq!(b, BBox { x1: 0.0, y1: 0.2, x2: 1.0, y2: 0.9 });
    }

    #[test]
    fn test_bbox_parse_rejects_malformed() {
        assert!(BBox::parse("0.1,0.2,0.3").is_none());
        assert!(BBox::parse("not,a,box,here").is_none());
        assert!(BBox::parse("").is_none());
    }

    #[test]
    fn test_recognition_status_str_roundtrip() {
        for s in [
            RecognitionStatus::Confirmed,
            RecognitionStatus::NeedsReview,
            RecognitionStatus::Unrecognized,
        ] {
            assert_eq!(RecognitionStatus::from_str(s.as_str()), Some(s));
        }
        assert_eq!(RecognitionStatus::from_str("pending"), None);
    }

    #[test]
    fn test_failure_stage_str_roundtrip_and_messages() {
        for s in [
            RecognitionFailureStage::None,
            RecognitionFailureStage::Detection,
            RecognitionFailureStage::Classification,
            RecognitionFailureStage::Mapping,
            RecognitionFailureStage::Assets,
        ] {
            assert_eq!(RecognitionFailureStage::from_str(s.as_str()), Some(s));
        }
        assert_eq!(RecognitionFailureStage::Mapping.user_message(), "名录映射失败");
    }

    #[test]
    fn test_enrich_with_recognition_confirmed() {
        let rec = Recognition {
            status: RecognitionStatus::Confirmed,
            bird: Some(BirdMatch {
                bird_id: 42,
                cn_name: "大山雀".into(),
                latin_name: "Parus major".into(),
            }),
            class_index: Some(1066),
            confidence: Some(85.3),
            bbox: Some(BBox::new(0.1, 0.2, 0.8, 0.9)),
            eye_sharpness: Some(2.35),
            eye_bbox: Some(BBox::new(0.3, 0.3, 0.4, 0.4)),
            candidates: vec![],
            failure_stage: RecognitionFailureStage::None,
            recognized_at: "2026-07-28T12:00:00Z".into(),
        };
        let mut cm = CaptureMeta::from(&Capture {
            base_name: "DSC_0001".into(),
            source_files: vec![SourceFile {
                path: std::path::PathBuf::from("/photos/DSC_0001.jpg"),
                format: ImageFormat::Jpeg,
                file_size: Some(1024),
            }],
            primary_index: 0,
        });
        cm.enrich_with_recognition(&rec);
        assert_eq!(cm.bird_name.as_deref(), Some("大山雀"));
        assert_eq!(cm.bird_confidence, Some(85.3));
        assert_eq!(cm.recognition_status, Some(RecognitionStatus::Confirmed));
        assert!(cm.bird_bbox.is_some());
    }

    #[test]
    fn test_enrich_with_recognition_unrecognized_has_no_bird_fields() {
        let rec = Recognition {
            status: RecognitionStatus::Unrecognized,
            bird: None,
            class_index: None,
            confidence: None,
            bbox: None,
            eye_sharpness: None,
            eye_bbox: None,
            candidates: vec![],
            failure_stage: RecognitionFailureStage::Detection,
            recognized_at: "2026-07-28T12:00:00Z".into(),
        };
        let mut cm = CaptureMeta::from(&Capture {
            base_name: "DSC_0002".into(),
            source_files: vec![SourceFile {
                path: std::path::PathBuf::from("/photos/DSC_0002.jpg"),
                format: ImageFormat::Jpeg,
                file_size: Some(1024),
            }],
            primary_index: 0,
        });
        cm.enrich_with_recognition(&rec);
        assert_eq!(cm.bird_name, None);
        assert_eq!(cm.recognition_status, Some(RecognitionStatus::Unrecognized));
        assert_eq!(cm.bird_bbox, None);
    }

    #[test]
    fn test_exif_summary_format() {
        let mut meta = ExifMetadata::default();
        meta.camera.make = Some("NIKON".to_string());
        meta.camera.model = Some("Z6III".to_string());
        meta.shooting.iso = Some(400);
        meta.image_width = Some(6000);
        meta.image_height = Some(4000);

        let summary = meta.summary();
        assert!(summary.contains("NIKON"));
        assert!(summary.contains("Z6III"));
        assert!(summary.contains("ISO: 400"));
        assert!(summary.contains("6000×4000"));
    }
}
