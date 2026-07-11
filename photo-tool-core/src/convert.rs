use std::io::Cursor;
use std::path::{Path, PathBuf};
use image::GenericImageView;
use thiserror::Error;

use crate::domain::ImageFormat;

#[derive(Error, Debug)]
pub enum ConvertError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Image error: {0}")]
    Image(#[from] image::ImageError),
    #[error("RAW extraction error: {0}")]
    Raw(String),
    #[error("Unsupported format")]
    UnsupportedFormat,
}

/// 转换选项
#[derive(Debug, Clone)]
pub struct ConvertOptions {
    /// 输出目录
    pub output_dir: PathBuf,
    /// 输出格式: "jpg" | "png"
    pub output_format: String,
    /// JPEG 质量 (1-100)
    pub jpeg_quality: u8,
    /// 最大长边像素（0 表示不缩放）
    pub max_dimension: u32,
    /// 覆盖已存在文件
    pub overwrite: bool,
}

impl Default for ConvertOptions {
    fn default() -> Self {
        Self {
            output_dir: PathBuf::from("."),
            output_format: "jpg".to_string(),
            jpeg_quality: 90,
            max_dimension: 0,
            overwrite: false,
        }
    }
}

/// 将 JPEG 编码为字节
fn write_jpeg_bytes(img: &image::DynamicImage, options: &ConvertOptions) -> Result<Vec<u8>, ConvertError> {
    let mut buf = Cursor::new(Vec::new());
    let mut jpeg_encoder =
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, options.jpeg_quality);
    jpeg_encoder.encode(img.as_bytes(), img.width(), img.height(), img.color().into())?;
    Ok(buf.into_inner())
}

/// 将图片编码为目标格式的字节
fn encode_image(img: &image::DynamicImage, options: &ConvertOptions) -> Result<Vec<u8>, ConvertError> {
    match options.output_format.as_str() {
        "png" => {
            let mut buf = Cursor::new(Vec::new());
            img.write_to(&mut buf, image::ImageFormat::Png)?;
            Ok(buf.into_inner())
        }
        _ => write_jpeg_bytes(img, options),
    }
}

/// 如果图片尺寸超过 max_dimension，按比例缩小
fn scale_if_needed(img: image::DynamicImage, max_dimension: u32) -> image::DynamicImage {
    if max_dimension == 0 {
        return img;
    }
    let (w, h) = img.dimensions();
    let max_side = w.max(h);
    if max_side <= max_dimension {
        return img;
    }
    let ratio = max_dimension as f64 / max_side as f64;
    let new_w = (w as f64 * ratio).round() as u32;
    let new_h = (h as f64 * ratio).round() as u32;
    img.resize_exact(new_w, new_h, image::imageops::FilterType::Lanczos3)
}

/// 构造输出文件路径
fn output_path(input: &Path, options: &ConvertOptions) -> PathBuf {
    let stem = input.file_stem().unwrap_or_default();
    let name = format!("{}.{}", stem.to_string_lossy(), options.output_format);
    options.output_dir.join(&name)
}

/// RAW 转 JPEG：提取 RAW 内嵌预览（不解拜耳），可选缩放
pub fn convert_raw_preview(raw_path: &Path, options: &ConvertOptions) -> Result<PathBuf, ConvertError> {
    std::fs::create_dir_all(&options.output_dir)?;

    let path_str = raw_path.to_string_lossy();
    let thumb_bytes = rawlib::extract_thumbnail(path_str.as_ref())
        .map_err(|e| ConvertError::Raw(e.to_string()))?;

    let out_path = output_path(raw_path, options);
    if out_path.exists() && !options.overwrite {
        return Ok(out_path);
    }

    let img = image::load_from_memory(&thumb_bytes)?;
    let final_img = scale_if_needed(img, options.max_dimension);
    let bytes = encode_image(&final_img, options)?;

    std::fs::write(&out_path, bytes)?;
    Ok(out_path)
}

/// JPEG 或其他常规格式调整大小
pub fn resize_image(input_path: &Path, options: &ConvertOptions) -> Result<PathBuf, ConvertError> {
    std::fs::create_dir_all(&options.output_dir)?;

    let out_path = output_path(input_path, options);
    if out_path.exists() && !options.overwrite {
        return Ok(out_path);
    }

    let img = image::open(input_path)?;
    let final_img = scale_if_needed(img, options.max_dimension);
    let bytes = encode_image(&final_img, options)?;

    std::fs::write(&out_path, bytes)?;
    Ok(out_path)
}

/// 统一转换入口：按 ImageFormat 分发
pub fn convert_image(
    path: &Path,
    format: &ImageFormat,
    options: &ConvertOptions,
) -> Result<PathBuf, ConvertError> {
    match format {
        ImageFormat::Raw(_) => convert_raw_preview(path, options),
        _ => resize_image(path, options),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_jpeg(dir: &TempDir, name: &str, width: u32, height: u32) -> PathBuf {
        let path = dir.path().join(name);
        let img = image::RgbImage::new(width, height);
        img.save(&path).unwrap();
        path
    }

    #[test]
    fn test_resize_image_smaller() {
        let dir = TempDir::new().unwrap();
        let input = create_test_jpeg(&dir, "test.jpg", 400, 300);

        let out_dir = TempDir::new().unwrap();
        let options = ConvertOptions {
            output_dir: out_dir.path().to_path_buf(),
            max_dimension: 100,
            ..Default::default()
        };

        let result = resize_image(&input, &options);
        assert!(result.is_ok(), "resize failed: {:?}", result.err());
        let out_path = result.unwrap();

        assert!(out_path.exists(), "output file not created");
        assert_eq!(out_path.file_name().unwrap(), "test.jpg");

        let loaded = image::open(&out_path).unwrap();
        let (w, h) = loaded.dimensions();
        assert!(w <= 100, "width {} should be <= 100", w);
        assert!(h <= 100, "height {} should be <= 100", h);
        let ratio = w as f64 / h as f64;
        let expected_ratio = 4.0 / 3.0;
        assert!(
            (ratio - expected_ratio).abs() < 0.01,
            "aspect ratio broken: {}x{} (ratio {})",
            w, h, ratio
        );
    }

    #[test]
    fn test_resize_image_no_resize() {
        let dir = TempDir::new().unwrap();
        let input = create_test_jpeg(&dir, "small.jpg", 50, 50);

        let out_dir = TempDir::new().unwrap();
        let options = ConvertOptions {
            output_dir: out_dir.path().to_path_buf(),
            max_dimension: 100,
            ..Default::default()
        };

        let result = resize_image(&input, &options);
        assert!(result.is_ok());

        let loaded = image::open(result.unwrap()).unwrap();
        assert_eq!(loaded.dimensions(), (50, 50));
    }

    #[test]
    fn test_resize_image_max_dim_zero_no_change() {
        let dir = TempDir::new().unwrap();
        let input = create_test_jpeg(&dir, "large.jpg", 800, 600);

        let out_dir = TempDir::new().unwrap();
        let options = ConvertOptions {
            output_dir: out_dir.path().to_path_buf(),
            max_dimension: 0,
            ..Default::default()
        };

        let result = resize_image(&input, &options);
        assert!(result.is_ok());

        let loaded = image::open(result.unwrap()).unwrap();
        assert_eq!(loaded.dimensions(), (800, 600));
    }

    #[test]
    fn test_convert_image_dispatch_jpeg() {
        let dir = TempDir::new().unwrap();
        let input = create_test_jpeg(&dir, "test.jpg", 100, 100);

        let out_dir = TempDir::new().unwrap();
        let options = ConvertOptions {
            output_dir: out_dir.path().to_path_buf(),
            ..Default::default()
        };

        let result = convert_image(&input, &ImageFormat::Jpeg, &options);
        assert!(result.is_ok());
    }

    #[test]
    fn test_convert_image_raw_nonexistent() {
        let path = Path::new("/nonexistent/test.nef");
        let options = ConvertOptions::default();
        let result = convert_image(path, &ImageFormat::Raw("NEF".into()), &options);
        assert!(result.is_err());
    }

    #[test]
    fn test_output_path_preserves_stem() {
        let path = Path::new("/photos/test.NEF");
        let options = ConvertOptions::default();
        let out = output_path(path, &options);
        assert_eq!(out.file_name().unwrap(), "test.jpg");
    }

    #[test]
    fn test_output_path_with_different_format() {
        let path = Path::new("/photos/test.NEF");
        let options = ConvertOptions {
            output_format: "png".to_string(),
            ..Default::default()
        };
        let out = output_path(path, &options);
        assert_eq!(out.file_name().unwrap(), "test.png");
    }
}
