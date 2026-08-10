use std::io::Cursor;
use std::path::{Path, PathBuf};
use thiserror::Error;

use image::GenericImageView;
use photo_domain::{AdjustParams, ImageFormat, SourceFile};
use crate::adjustments::{apply_tone16, apply_tone8, Rgb16Image};
use crate::thumbnail::decode_raw16_with_options;

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

/// 导出调整结果（ADR 0007）：源图 → 烘焙色调调整 → JPEG（质量 95）。
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
            let toned = apply_tone16(&img16, &tone);
            rgb16_to_rgb8(&toned)
        }
        _ => {
            let img = image::open(&source.path)?;
            let rgb8 = img.to_rgb8();
            let tone: crate::adjustments::ToneParams = params.into();
            apply_tone8(&rgb8, &tone)
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

/// 调整渲染（内存版，ADR 0007）：源图 → 色调调整 → JPEG 字节（质量 85，预览用）。
///
/// 供 ptimg:// 协议 master 预览带调整参数时输出；与 `export_adjusted` 的差异是
/// 不写文件、长边缩放到 `max_size` 内（避免全尺寸逐像素遍历拖慢预览）。
/// - RAW：half_size 16-bit 解码（`preview16` 选项）→ 16-bit tone → 降 8-bit
/// - 常规图：原图解码（8-bit 语义，直出即 8-bit 无高位信息）→ 8-bit tone
pub fn render_adjusted(
    source: &SourceFile,
    params: &AdjustParams,
    max_size: u32,
) -> Result<Vec<u8>, ConvertError> {
    let tone: crate::adjustments::ToneParams = params.into();
    let rgb8 = match &source.format {
        ImageFormat::Raw(_) => {
            let img16 = decode_raw16_with_options(
                &source.path,
                &rawlib::DecodeOptions::preview16(),
                None,
            )
            .map_err(|e| ConvertError::Raw(e.to_string()))?;
            let (w, h) = img16.dimensions();
            let scale = (max_size as f32 / w.max(h) as f32).min(1.0);
            let toned = if scale < 1.0 {
                let nw = ((w as f32 * scale).round().max(1.0)) as u32;
                let nh = ((h as f32 * scale).round().max(1.0)) as u32;
                let small = image::imageops::resize(
                    &img16,
                    nw,
                    nh,
                    image::imageops::FilterType::Triangle,
                );
                apply_tone16(&small, &tone)
            } else {
                apply_tone16(&img16, &tone)
            };
            rgb16_to_rgb8(&toned)
        }
        _ => {
            let img = image::open(&source.path)?;
            let (w, h) = img.dimensions();
            let scale = (max_size as f32 / w.max(h) as f32).min(1.0);
            let rgb8 = if scale < 1.0 {
                let nw = ((w as f32 * scale).round().max(1.0)) as u32;
                let nh = ((h as f32 * scale).round().max(1.0)) as u32;
                img.resize_exact(nw, nh, image::imageops::FilterType::Triangle)
                    .to_rgb8()
            } else {
                img.to_rgb8()
            };
            apply_tone8(&rgb8, &tone)
        }
    };
    let mut buf = Cursor::new(Vec::new());
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 85);
    encoder.encode(
        rgb8.as_raw(),
        rgb8.width(),
        rgb8.height(),
        image::ExtendedColorType::Rgb8,
    )?;
    Ok(buf.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::GenericImageView;
    use tempfile::TempDir;

    fn create_test_jpeg(dir: &TempDir, name: &str, width: u32, height: u32) -> PathBuf {
        let path = dir.path().join(name);
        let img = image::RgbImage::new(width, height);
        img.save(&path).unwrap();
        path
    }

    #[test]
    fn test_export_adjusted_jpeg_tone_applied() {
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
        };
        let result = export_adjusted(&source, &params, &out).unwrap();
        assert_eq!(result, out);
        assert!(out.exists());
        let loaded = image::open(&out).unwrap();
        // 无裁切：全尺寸输出
        assert_eq!(loaded.dimensions(), (400, 300));
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
