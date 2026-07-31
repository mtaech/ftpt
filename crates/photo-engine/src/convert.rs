use std::io::Cursor;
use std::path::{Path, PathBuf};
use image::GenericImageView;
use thiserror::Error;

use photo_domain::{AdjustParams, ImageFormat, SourceFile};
use crate::adjustments::{apply_crop16, apply_crop8, apply_tone16, apply_tone8, Rgb16Image};
use crate::thumbnail::{decode_raw_preview, decode_raw16_with_options};

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
    // JPEG 不支持 alpha：先展平为 RGB8，否则带透明度的 PNG/TIFF/WebP
    // 以 ExtendedColorType::Rgba8 编码会被 JpegEncoder 拒绝（Unsupported）
    let rgb = img.to_rgb8();
    jpeg_encoder.encode(rgb.as_raw(), rgb.width(), rgb.height(), image::ExtendedColorType::Rgb8)?;
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

/// RAW 转 JPEG：完整解码（half_size 预览选项），可选缩放。
/// 不再使用内嵌小图（多数相机 160-640px，输出会糊）。
pub fn convert_raw_preview(raw_path: &Path, options: &ConvertOptions) -> Result<PathBuf, ConvertError> {
    std::fs::create_dir_all(&options.output_dir)?;

    let out_path = output_path(raw_path, options);
    if out_path.exists() && !options.overwrite {
        return Ok(out_path);
    }

    // 0 = 不缩放（映射 u32::MAX，与母版缓存语义一致；0 会令 DCT 尺寸为 0）
    let size = if options.max_dimension == 0 { u32::MAX } else { options.max_dimension };
    let jpeg = decode_raw_preview(raw_path, size)
        .map_err(|e| ConvertError::Raw(e.to_string()))?;

    // JPEG 输出直接用解码字节（已按 max_dimension 缩放，免一次重编码）；
    // PNG 等其他格式需重编码
    if matches!(options.output_format.as_str(), "jpg" | "jpeg") {
        std::fs::write(&out_path, jpeg)?;
    } else {
        let img = image::load_from_memory(&jpeg)?;
        let bytes = encode_image(&img, options)?;
        std::fs::write(&out_path, bytes)?;
    }
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

/// 导出调整结果（ADR 0007）：源图 → 烘焙（先裁切 → 再色调）→ JPEG（质量 95）。
///
/// - RAW：全尺寸 16-bit 解码（`DecodeOptions::quality()`：AHD + 16bit）——导出是唯一一次
///   全尺寸高质量渲染，3-5s 可接受（低频异步任务）
/// - 常规图：原文件解码（8-bit 语义，直出即 8-bit 无高位信息）
/// - `output_path` 的命名/防覆盖由调用方保证（`{stem}_adjusted.jpg` + 序号）
/// - 全同步，调用方负责 worker 线程
pub fn export_adjusted(
    source: &SourceFile,
    params: &AdjustParams,
    output_path: &Path,
) -> Result<PathBuf, ConvertError> {
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let rgb8 = match &source.format {
        ImageFormat::Raw(_) => {
            let img16 = decode_raw16_with_options(
                &source.path,
                &rawlib::DecodeOptions::quality(),
                None,
            )
            .map_err(|e| ConvertError::Raw(e.to_string()))?;
            let tone: crate::adjustments::ToneParams = params.into();
            let cropped = apply_crop16(&img16, params.crop);
            let toned = apply_tone16(&cropped, &tone);
            rgb16_to_rgb8(&toned)
        }
        _ => {
            let img = image::open(&source.path)?;
            let rgb8 = img.to_rgb8();
            let tone: crate::adjustments::ToneParams = params.into();
            let cropped = apply_crop8(&rgb8, params.crop);
            apply_tone8(&cropped, &tone)
        }
    };
    let mut buf = Cursor::new(Vec::new());
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 95);
    encoder.encode(
        rgb8.as_raw(),
        rgb8.width(),
        rgb8.height(),
        image::ExtendedColorType::Rgb8,
    )?;
    std::fs::write(output_path, buf.into_inner())?;
    Ok(output_path.to_path_buf())
}

/// 16-bit RGB 缓冲降为 8-bit（sRGB 编码值直接截高 8 位，语义一致）
fn rgb16_to_rgb8(img: &Rgb16Image) -> image::RgbImage {
    image::RgbImage::from_fn(img.width(), img.height(), |x, y| {
        let p = img.get_pixel(x, y);
        image::Rgb([(p[0] >> 8) as u8, (p[1] >> 8) as u8, (p[2] >> 8) as u8])
    })
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
    use photo_domain::BBox;
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

    #[test]
    fn test_export_adjusted_jpeg_tone_and_crop() {
        let dir = TempDir::new().unwrap();
        let input = create_test_jpeg(&dir, "shot.jpg", 400, 300);
        let out_dir = TempDir::new().unwrap();
        let out = out_dir.path().join("shot_adjusted.jpg");
        let source = SourceFile {
            path: input,
            format: ImageFormat::Jpeg,
            file_size: Some(0),
        };
        let params = photo_domain::AdjustParams {
            exposure: 1.0,
            contrast: 20,
            saturation: -30,
            crop: Some(BBox::new(0.25, 0.25, 0.75, 0.75)),
        };
        let result = export_adjusted(&source, &params, &out).unwrap();
        assert_eq!(result, out);
        assert!(out.exists());
        let loaded = image::open(&out).unwrap();
        // 裁切 400×300 的 (0.25..0.75) → 200×150
        assert_eq!(loaded.dimensions(), (200, 150));
    }

    #[test]
    fn test_export_adjusted_neutral_jpeg_full_size() {
        let dir = TempDir::new().unwrap();
        let input = create_test_jpeg(&dir, "shot.jpg", 400, 300);
        let out_dir = TempDir::new().unwrap();
        let out = out_dir.path().join("out.jpg");
        let source = SourceFile {
            path: input,
            format: ImageFormat::Jpeg,
            file_size: Some(0),
        };
        let result = export_adjusted(&source, &AdjustParams::default(), &out).unwrap();
        let loaded = image::open(result).unwrap();
        assert_eq!(loaded.dimensions(), (400, 300));
    }
}
