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
    /// 文件完整路径
    pub path: PathBuf,
    /// 图片格式
    pub format: ImageFormat,
    /// 是否为旁车文件（.xmp 等）
    pub is_sidecar: bool,
    /// 文件大小（字节），扫描时从目录项获取（几乎无开销）
    pub file_size: Option<u64>,
}

/// 一次快门产生的拍摄
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capture {
    /// 基本文件名（不含扩展名）
    pub base_name: String,
    /// 目录路径
    pub directory: PathBuf,
    /// 所有源文件（JPEG + RAW + 旁车）
    pub source_files: Vec<SourceFile>,
    /// 主显示文件的索引（指向 source_files）
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
        }
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
}

/// 排序方式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortBy {
    FileName,
    DateTaken,
    FileSize,
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
    Trash,     // 移到回收站
    Permanent, // 永久删除
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
}
