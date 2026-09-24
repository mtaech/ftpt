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
    // 原尺寸 + 质量 95 即预设导出的特例（保持既有行为与命名）
    export_with_preset(source, params, None, 95, output_path)
}

/// 预设导出（T1 批次）：源图 → 可选长边缩放 → 色调调整 → JPEG 写出。
///
/// - `long_edge`：输出长边像素上限（None = 原尺寸）。RAW 走全尺寸 16-bit quality
///   解码后缩放（三角形滤波，保持导出高质量语义）；常规图原图解码后缩放。
/// - `quality`：JPEG 质量（1-100，内部钳制；0/越界值不产生非法编码器状态）
/// - `output_path` 的命名/防覆盖由调用方保证（模板渲染 + 序号去重）
/// - 全同步，调用方负责 worker 线程
pub fn export_with_preset(
    source: &SourceFile,
    params: &AdjustParams,
    long_edge: Option<u32>,
    quality: u8,
    output_path: &Path,
) -> Result<PathBuf, ConvertError> {
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let rgb8 = bake_rgb8(source, params, long_edge)?;
    let mut buf = Cursor::new(Vec::new());
    let mut encoder =
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, quality.clamp(1, 100));
    encoder.encode(
        rgb8.as_raw(),
        rgb8.width(),
        rgb8.height(),
        image::ExtendedColorType::Rgb8,
    )?;
    std::fs::write(output_path, buf.into_inner())?;
    Ok(output_path.to_path_buf())
}

/// 解码 + 色调调整 → 8-bit RGB（导出用）：RAW 全尺寸 quality 解码，长边缩放。
/// 与 `render_adjusted`（预览 half_size 语义）分离，导出始终走全尺寸解码。
fn bake_rgb8(
    source: &SourceFile,
    params: &AdjustParams,
    long_edge: Option<u32>,
) -> Result<image::RgbImage, ConvertError> {
    let tone: crate::adjustments::ToneParams = params.into();
    match &source.format {
        ImageFormat::Raw(_) => {
            let img16 = decode_raw16_with_options(
                &source.path,
                &rawlib::DecodeOptions::quality(),
                None,
            )
            .map_err(|e| ConvertError::Raw(e.to_string()))?;
            let (w, h) = img16.dimensions();
            let scale = scale_for_long_edge(w, h, long_edge);
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
            Ok(rgb16_to_rgb8(&toned))
        }
        _ => {
            let img = image::open(&source.path)?;
            let (w, h) = img.dimensions();
            let scale = scale_for_long_edge(w, h, long_edge);
            let rgb8 = if scale < 1.0 {
                let nw = ((w as f32 * scale).round().max(1.0)) as u32;
                let nh = ((h as f32 * scale).round().max(1.0)) as u32;
                img.resize_exact(nw, nh, image::imageops::FilterType::Triangle)
                    .to_rgb8()
            } else {
                img.to_rgb8()
            };
            Ok(apply_tone8(&rgb8, &tone))
        }
    }
}

/// 长边缩放系数：`long_edge` 为 None 或大于原长边时返回 1.0（不放大）。
fn scale_for_long_edge(w: u32, h: u32, long_edge: Option<u32>) -> f32 {
    match long_edge {
        Some(m) if m > 0 => (m as f32 / w.max(h) as f32).min(1.0),
        _ => 1.0,
    }
}

/// 解码 RAW 的 **16-bit 预览母版**（ADR 0007）：half_size + 16bit + 自动亮度 + sRGB + 相机白平衡，
/// 长边缩到 max_size 内（**保持 16-bit 精度**，缩放用三角形滤波）。
///
/// 这是调整预览的像素源：8-bit 母版在大范围拉曝光时会条带，16-bit 母版把这段动态范围留住。
/// 非 RAW 源没有更高位宽可用，调用方应走 8-bit 链路（本函数不校验格式，解码失败会报 Raw 错误）。
pub fn decode_raw16_master(source: &SourceFile, max_size: u32) -> Result<Rgb16Image, ConvertError> {
    let img16 = decode_raw16_with_options(
        &source.path,
        &rawlib::DecodeOptions::preview16(),
        None,
    )
    .map_err(|e| ConvertError::Raw(e.to_string()))?;
    Ok(fit_long_edge16(&img16, max_size))
}

/// 16-bit 图按长边上限缩小（只缩不放）。抽出来单独测：解码要真实 RAW 文件，缩放不用。
pub fn fit_long_edge16(img: &Rgb16Image, max_size: u32) -> Rgb16Image {
    let (w, h) = img.dimensions();
    if max_size == 0 || w.max(h) <= max_size {
        return img.clone();
    }
    let scale = max_size as f32 / w.max(h) as f32;
    let nw = ((w as f32 * scale).round().max(1.0)) as u32;
    let nh = ((h as f32 * scale).round().max(1.0)) as u32;
    image::imageops::resize(img, nw, nh, image::imageops::FilterType::Triangle)
}

/// 16-bit RGB 缓冲降为 8-bit（sRGB 编码值直接截高 8 位，语义一致）。
///
/// 公开给 UI 侧：16-bit 调整帧算完色调后，直方图 / 剪切掩码 / JPEG 显示都要先降到 8-bit。
pub fn rgb16_to_rgb8(img: &Rgb16Image) -> image::RgbImage {
    image::RgbImage::from_fn(img.width(), img.height(), |x, y| {
        let p = img.get_pixel(x, y);
        image::Rgb([(p[0] >> 8) as u8, (p[1] >> 8) as u8, (p[2] >> 8) as u8])
    })
}

/// 全分辨率调整渲染（ADR 0007 的 1:1 闭环）：源图 → 全尺寸解码 → 色调调整 → JPEG 字节。
///
/// RAW 走全尺寸 quality 预设（AHD + 16bit，3-5s）；常规图直接解码原文件。
/// 与 render_adjusted 的差别是**不缩尺寸**——只给 1:1 / 高倍放大用，且由调用方在
/// 滑杆停手后异步触发（拖动中仍走 1600px 即时链路）。
pub fn render_adjusted_full(
    source: &SourceFile,
    params: &AdjustParams,
    quality: u8,
) -> Result<Vec<u8>, ConvertError> {
    let rgb8 = bake_rgb8(source, params, None)?;
    let mut buf = Cursor::new(Vec::new());
    let mut encoder =
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, quality.clamp(1, 100));
    encoder.encode(
        rgb8.as_raw(),
        rgb8.width(),
        rgb8.height(),
        image::ExtendedColorType::Rgb8,
    )?;
    Ok(buf.into_inner())
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

    /// 全分辨率调整渲染：输出保持原尺寸（不缩放）
    #[test]
    fn test_render_adjusted_full_keeps_source_size() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("full.png");
        let img = image::RgbImage::from_fn(320, 200, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, 128])
        });
        img.save(&path).unwrap();
        let source = SourceFile {
            path,
            format: ImageFormat::Png,
            file_size: None,
        };
        let bytes = render_adjusted_full(
            &source,
            &AdjustParams {

                exposure: 1.0,
                contrast: 0,
                saturation: 0,
            ..AdjustParams::default()
        },
            92,
        )
        .unwrap();
        let out = image::load_from_memory(&bytes).unwrap();
        assert_eq!((out.width(), out.height()), (320, 200), "全分辨率不该被缩放");
    }

    /// 16-bit 母版的长边缩放：只缩不放，且保持 16-bit（值不被截断）。
    #[test]
    fn test_fit_long_edge16_scales_down_only() {
        let img = Rgb16Image::from_fn(200, 100, |x, y| {
            image::Rgb([(x as u16) * 300 + (y as u16), 4000, 60000])
        });
        // 长边在限内：原样返回（不放大）
        let same = fit_long_edge16(&img, 2560);
        assert_eq!(same.dimensions(), (200, 100));
        assert_eq!(same.get_pixel(199, 99)[0], img.get_pixel(199, 99)[0]);
        // 长边超限：按长边缩，宽高比保持
        let small = fit_long_edge16(&img, 100);
        assert_eq!(small.dimensions(), (100, 50));
        // 缩的是 16-bit 数据本身：最大值仍是 16-bit 量级（不是被截成 8-bit）
        assert!(small.pixels().map(|p| p[2]).max().unwrap() > 255);
        // 0 视为"不限制"
        assert_eq!(fit_long_edge16(&img, 0).dimensions(), (200, 100));
    }
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
            ..AdjustParams::default()
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

    #[test]
    fn test_export_with_preset_long_edge_scales_down() {
        let dir = TempDir::new().unwrap();
        let input = create_test_jpeg(&dir, "big.jpg", 1200, 800);
        let out_dir = TempDir::new().unwrap();
        let out = out_dir.path().join("out.jpg");
        let source = SourceFile {
            path: input,
            format: ImageFormat::Jpeg,
            file_size: Some(0),
        };
        // 长边 600 → 等比缩到 600×400
        let result =
            export_with_preset(&source, &AdjustParams::default(), Some(600), 90, &out).unwrap();
        let loaded = image::open(result).unwrap();
        assert_eq!(loaded.dimensions(), (600, 400));
    }

    #[test]
    fn test_export_with_preset_no_upscale() {
        let dir = TempDir::new().unwrap();
        let input = create_test_jpeg(&dir, "small.jpg", 200, 100);
        let out_dir = TempDir::new().unwrap();
        let out = out_dir.path().join("out.jpg");
        let source = SourceFile {
            path: input,
            format: ImageFormat::Jpeg,
            file_size: Some(0),
        };
        // 长边限制大于原尺寸 → 不放大
        let result =
            export_with_preset(&source, &AdjustParams::default(), Some(1000), 90, &out).unwrap();
        let loaded = image::open(result).unwrap();
        assert_eq!(loaded.dimensions(), (200, 100));
    }

    #[test]
    fn test_export_with_preset_none_long_edge_original_size() {
        let dir = TempDir::new().unwrap();
        let input = create_test_jpeg(&dir, "orig.jpg", 400, 300);
        let out_dir = TempDir::new().unwrap();
        let out = out_dir.path().join("out.jpg");
        let source = SourceFile {
            path: input,
            format: ImageFormat::Jpeg,
            file_size: Some(0),
        };
        let result =
            export_with_preset(&source, &AdjustParams::default(), None, 80, &out).unwrap();
        let loaded = image::open(result).unwrap();
        assert_eq!(loaded.dimensions(), (400, 300));
    }
}
