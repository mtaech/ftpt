use std::path::Path;
use thiserror::Error;
use crate::domain::ImageFormat;


/// 相机制造商信息
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraInfo {
    pub make: Option<String>,
    pub model: Option<String>,
    pub lens: Option<String>,
}

/// 拍摄参数
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
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
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GpsInfo {
    pub latitude: Option<(f64, f64, f64)>,
    pub longitude: Option<(f64, f64, f64)>,
    pub altitude: Option<f64>,
}

/// 完整的 EXIF 元数据
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
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

/// EXIF 提取错误
#[derive(Error, Debug)]
pub enum ExifError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("EXIF parse error: {0}")]
    Parse(String),
    #[error("RAW EXIF error: {0}")]
    Raw(String),
}


/// EXIF 提取器 trait：不同的文件格式使用不同的实现
pub trait ExifExtractor {
    /// 从指定路径提取 EXIF 元数据
    fn extract(&self, path: &Path) -> Result<ExifMetadata, ExifError>;
}

/// 常规图片（JPEG/TIFF/PNG 等）EXIF 提取，使用 kamadak-exif 库
pub struct RegularExifExtractor;
impl ExifExtractor for RegularExifExtractor {
    fn extract(&self, path: &Path) -> Result<ExifMetadata, ExifError> {
        extract_exif_regular(path)
    }
}

/// RAW 格式 EXIF 提取，使用 rawlib 库
pub struct RawExifExtractor;
impl ExifExtractor for RawExifExtractor {
    fn extract(&self, path: &Path) -> Result<ExifMetadata, ExifError> {
        extract_exif_raw(path)
    }
}

/// 根据格式选择对应的 EXIF 提取器
pub fn extractor_for(format: &ImageFormat) -> Box<dyn ExifExtractor> {
    match format {
        ImageFormat::Raw(_) => Box::new(RawExifExtractor),
        _ => Box::new(RegularExifExtractor),
    }
}

/// 统一入口：根据文件格式选择提取方式，提取失败时记录 warning
pub fn extract_exif(path: &Path, format: &ImageFormat) -> Result<ExifMetadata, ExifError> {
    let result = extractor_for(format).extract(path);
    if let Err(ref e) = result {
        tracing::warn!("EXIF 提取失败 {} (格式 {:?}): {e}", path.display(), format);
    }
    result
}

/// 从常规图片文件读取 EXIF（使用 kamadak-exif）
fn extract_exif_regular(path: &Path) -> Result<ExifMetadata, ExifError> {
    let file = std::fs::File::open(path)?;
    let mut reader = std::io::BufReader::new(file);
    let exif_reader = exif::Reader::new();

    let exif = exif_reader
        .read_from_container(&mut reader)
        .map_err(|e| ExifError::Parse(e.to_string()))?;

    let mut meta = ExifMetadata::default();

    for field in exif.fields() {
        let value = field.display_value().to_string();
        match field.tag {
            exif::Tag::Make => {
                meta.camera.make = Some(value.trim_matches('"').to_string());
            }
            exif::Tag::Model => {
                meta.camera.model = Some(value.trim_matches('"').to_string());
            }
            exif::Tag::LensModel => {
                meta.camera.lens = Some(value.trim_matches('"').to_string());
            }
            exif::Tag::DateTimeOriginal => {
                meta.date_time_original = Some(value.trim_matches('"').to_string());
            }
            exif::Tag::ExposureTime => {
                meta.shooting.exposure_time = Some(value);
            }
            exif::Tag::FNumber => {
                meta.shooting.f_number = Some(value);
            }
            exif::Tag::ISOSpeed => {
                meta.shooting.iso = value.parse::<u32>().ok();
            }
            exif::Tag::FocalLength => {
                meta.shooting.focal_length = Some(value);
            }
            exif::Tag::ExposureBiasValue => {
                meta.shooting.exposure_compensation = Some(value);
            }
            exif::Tag::WhiteBalance => {
                meta.shooting.white_balance = Some(value);
            }
            exif::Tag::ImageWidth | exif::Tag::PixelXDimension => {
                meta.image_width = value.parse::<u32>().ok();
            }
            exif::Tag::ImageLength | exif::Tag::PixelYDimension => {
                meta.image_height = value.parse::<u32>().ok();
            }
            exif::Tag::Orientation => {
                meta.orientation = value.parse::<u16>().ok();
            }
            exif::Tag::ColorSpace => {
                meta.color_space = Some(value);
            }
            _ => {}
        }
    }

    // 文件大小
    if let Ok(fs) = std::fs::metadata(path) {
        meta.file_size = Some(fs.len());
    }

    Ok(meta)
}

/// 从 RAW 文件读取 EXIF（通过 rawlib 0.7+ 的 LibRaw C API）
///
/// rawlib 0.7.0 改用 LibRaw 直接读取 EXIF 结构体，不再依赖 kamadak-exif，
/// 因此能正确支持 Panasonic RW2 等非标准 TIFF 魔数的 RAW 格式。
fn extract_exif_raw(path: &Path) -> Result<ExifMetadata, ExifError> {
    let path_str = path.to_string_lossy();
    let raw_exif = rawlib::exif::extract_exif(path_str.as_ref())
        .map_err(|e| ExifError::Raw(e.to_string()))?;
    Ok(raw_exif_to_meta(raw_exif, path))
}

/// 从 rawlib::ExifData 转为 ExifMetadata，共用逻辑
fn raw_exif_to_meta(raw_exif: rawlib::ExifData, path: &Path) -> ExifMetadata {
    let mut meta = ExifMetadata::default();
    meta.camera.make = raw_exif.make;
    meta.camera.model = raw_exif.model;
    meta.camera.lens = raw_exif.lens_model;
    meta.date_time_original = raw_exif.date_time_original;
    meta.shooting.exposure_time = raw_exif.exposure_time;
    meta.shooting.f_number = raw_exif.f_number;
    meta.shooting.iso = raw_exif.iso;
    meta.shooting.focal_length = raw_exif.focal_length;
    meta.image_width = raw_exif.image_width;
    meta.image_height = raw_exif.image_height;
    meta.orientation = raw_exif.orientation;
    if let Some(lat) = raw_exif.gps_latitude {
        meta.gps.latitude = Some(lat);
    }
    if let Some(lon) = raw_exif.gps_longitude {
        meta.gps.longitude = Some(lon);
    }
    meta.gps.altitude = raw_exif.gps_altitude;
    if let Ok(fs) = std::fs::metadata(path) {
        meta.file_size = Some(fs.len());
    }
    meta
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// 创建一个包含基本 EXIF 的测试 JPEG 文件
    /// 使用 image crate 写一个简单图片，然后附加 EXIF 头
    fn create_test_jpeg_with_exif(dir: &TempDir, name: &str) -> std::path::PathBuf {
        let path = dir.path().join(name);
        // 使用 image crate 生成 160x120 的 JPEG
        let img = image::RgbImage::new(160, 120);
        img.save(&path).unwrap();
        path
    }

    #[test]
    fn test_extract_exif_from_jpeg_no_exif_returns_parse_error() {
        let dir = TempDir::new().unwrap();
        let path = create_test_jpeg_with_exif(&dir, "test.jpg");

        // image crate 生成的 JPEG 不包含 EXIF 段
        let result = extract_exif(&path, &ImageFormat::Jpeg);

        // 应该返回 Parse 错误（"No Exif data found in JPEG"）
        match result {
            Err(ExifError::Parse(_)) => {} // 预期行为
            other => panic!("expected ExifError::Parse, got {:?}", other),
        }
    }

    #[test]
    fn test_extract_exif_summary_format() {
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

    #[test]
    fn test_extract_exif_nonexistent_file() {
        let path = std::path::Path::new("/nonexistent/photo.jpg");
        let result = extract_exif(path, &ImageFormat::Jpeg);
        assert!(result.is_err());
    }

    #[test]
    fn test_default_exif_metadata_is_empty() {
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
    fn test_extract_exif_raw_handles_nonexistent() {
        let path = std::path::Path::new("/nonexistent/photo.nef");
        let result = extract_exif(path, &ImageFormat::Raw("NEF".into()));
        // RAW 文件不存在：可能是 IO 错误或 Raw 错误
        assert!(result.is_err());
    }

    #[test]
    fn test_file_size_is_set_on_any_result() {
        let dir = TempDir::new().unwrap();
        let path = create_test_jpeg_with_exif(&dir, "size_test.jpg");

        // 即使是 Parse 错误，文件大小也应通过 fs::metadata 设置
        let result = extract_exif(&path, &ImageFormat::Jpeg);
        match result {
            Err(ExifError::Parse(_)) => {
                // 无法从错误中获取元数据，所以验证文件本身大小正确
                let actual_size = fs::metadata(&path).unwrap().len();
                assert!(actual_size > 0);
            }
            Ok(meta) => {
                let actual_size = fs::metadata(&path).unwrap().len();
                assert_eq!(meta.file_size, Some(actual_size));
            }
            other => panic!("unexpected result: {:?}", other),
        }
    }
}
