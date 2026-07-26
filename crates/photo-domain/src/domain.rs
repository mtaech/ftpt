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

    /// 主显示图片优先级（数字越小越优先）
    pub fn display_priority(&self) -> u8 {
        match self {
            Self::Jpeg => 0,
            Self::Png => 1,
            Self::Tiff => 2,
            Self::Heif => 3,
            Self::WebP => 4,
            Self::Bmp => 5,
            Self::Gif => 6,
            Self::Raw(_) => 7,
        }
    }
}

/// 组成一次拍摄的单个源文件
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceFile {
    pub path: PathBuf,
    pub format: ImageFormat,
    pub file_size: Option<u64>,
    pub is_sidecar: bool,
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
    pub has_xmp: bool,
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
    // --- XMP 元数据字段（从旁车文件填充） ---
    pub rating: Rating,
    pub color_label: ColorLabel,
    pub flag: Option<Flag>,
}

impl From<&Capture> for CaptureMeta {
    fn from(c: &Capture) -> Self {
        let primary = &c.source_files[c.primary_index];
        let ext_list: Vec<String> = c
            .source_files
            .iter()
            .filter(|f| !f.is_sidecar)
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
                .filter(|(i, f)| *i != c.primary_index && !f.is_sidecar)
                .count(),
            file_size: primary.file_size,
            date_taken: None,
            has_xmp: c.source_files.iter().any(|f| f.is_sidecar),
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

    /// 从 XMP 元数据填充评分/颜色标签/旗标（由调用方负责读取 XMP 文件）
    pub fn enrich_with_xmp(&mut self, xmp: &XmpMetadata) {
        if !self.has_xmp {
            return;
        }
        self.rating = xmp.rating();
        self.color_label = xmp.color_label();
        self.flag = xmp.flag();
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
    /// 仅显示有配对的（JPEG+RAW）
    pub paired_only: Option<bool>,
    /// 按文件类型过滤
    pub format_filter: Option<ImageFormat>,
    /// 文件名文本搜索
    pub text_search: Option<String>,
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

/// 删除模式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeleteMode {
    Trash,
    Permanent,
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
    use tempfile::TempDir;

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
    fn test_display_priority_jpeg_higher_than_raw() {
        assert!(
            ImageFormat::Jpeg.display_priority()
                < ImageFormat::Raw("NEF".into()).display_priority()
        );
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
    fn test_enrich_with_xmp_reads_sidecar() {
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
            has_xmp: true,
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
        };
        cm.enrich_with_xmp(&xmp);
        assert_eq!(cm.rating, Rating::Three);
        assert_eq!(cm.color_label, ColorLabel::Green);
        assert_eq!(cm.flag, Some(Flag::Pick));
    }

    #[test]
    fn test_enrich_with_xmp_no_sidecar_keeps_defaults() {
        let xmp = XmpMetadata::default();
        let mut cm = CaptureMeta {
            index: 0,
            base_name: "NO_XMP".into(),
            primary_path: "/nonexistent/NO_XMP.jpg".into(),
            primary_format: "JPEG".into(),
            stack_count: 0,
            file_size: None,
            date_taken: None,
            has_xmp: false,
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
        };
        cm.enrich_with_xmp(&xmp);
        assert_eq!(cm.rating, Rating::None);
        assert_eq!(cm.color_label, ColorLabel::None);
        assert_eq!(cm.flag, None);
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
