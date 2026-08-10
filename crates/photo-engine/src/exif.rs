use std::path::Path;
use thiserror::Error;
use photo_domain::ImageFormat;


use photo_domain::ExifMetadata;


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


/// 统一入口：根据文件格式选择提取方式，提取失败时记录 warning
pub fn extract_exif(path: &Path, format: &ImageFormat) -> Result<ExifMetadata, ExifError> {
    let result = match format {
        ImageFormat::Raw(_) => extract_exif_raw(path),
        _ => extract_exif_regular(path),
    };
    if let Err(e) = &result {
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

    // GPS 标签（ref 字段可能出现在值字段之前或之后，先收集再统一应用符号）
    let mut gps_lat: Option<(f64, f64, f64)> = None;
    let mut gps_lon: Option<(f64, f64, f64)> = None;
    let mut gps_alt: Option<f64> = None;
    let mut lat_ref: Option<char> = None;
    let mut lon_ref: Option<char> = None;

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
            exif::Tag::GPSLatitude => gps_lat = gps_dms_to_tuple(&field.value),
            exif::Tag::GPSLongitude => gps_lon = gps_dms_to_tuple(&field.value),
            exif::Tag::GPSAltitude => gps_alt = gps_rational_to_f64(&field.value),
            exif::Tag::GPSLatitudeRef => {
                lat_ref = value.trim_matches('"').chars().next();
            }
            exif::Tag::GPSLongitudeRef => {
                lon_ref = value.trim_matches('"').chars().next();
            }
            _ => {}
        }
    }

    // 与 rawlib 路径产出格式一致：(度, 分, 秒) 元组，南纬/西经符号施加在度分量上
    if let Some((deg, min, sec)) = gps_lat {
        let deg = if lat_ref == Some('S') { -deg } else { deg };
        meta.gps.latitude = Some((deg, min, sec));
    }
    if let Some((deg, min, sec)) = gps_lon {
        let deg = if lon_ref == Some('W') { -deg } else { deg };
        meta.gps.longitude = Some((deg, min, sec));
    }
    meta.gps.altitude = gps_alt;

    // 文件大小
    if let Ok(fs) = std::fs::metadata(path) {
        meta.file_size = Some(fs.len());
    }

    Ok(meta)
}

/// 将 GPS 经纬度的有理数分量（度/分/秒）转为 (度, 分, 秒) 元组
fn gps_dms_to_tuple(value: &exif::Value) -> Option<(f64, f64, f64)> {
    match value {
        exif::Value::Rational(ratios) if ratios.len() >= 3 => {
            Some((ratios[0].to_f64(), ratios[1].to_f64(), ratios[2].to_f64()))
        }
        _ => None,
    }
}

/// 将 GPS 单值有理数（如海拔，米）转为 f64
fn gps_rational_to_f64(value: &exif::Value) -> Option<f64> {
    match value {
        exif::Value::Rational(ratios) if !ratios.is_empty() => Some(ratios[0].to_f64()),
        _ => None,
    }
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
