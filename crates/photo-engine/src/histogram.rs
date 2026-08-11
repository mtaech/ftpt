//! 直方图与过曝/欠曝剪切叠加（T1 批次 HistogramPanel 切片）。
//!
//! 输入统一为预览尺寸 RGB 像素（JPEG 走 `thumbnail::decode_jpeg_rgb8_scaled` 的
//! DCT 降采样，避免全尺寸解码 24MP；RAW 走 `decode_raw_preview` half_size 预览；
//! 其余栅格格式 image 全解码后 Lanczos 缩到预览尺寸），输出 256 级 luma/RGB 计数
//! 与剪切统计；剪切叠加输出 RGBA PNG（红 = 高光溢出、蓝 = 死黑，其余透明）。

use std::path::Path;
use thiserror::Error;

use photo_domain::ImageFormat;

use crate::thumbnail;

/// 直方图计算错误
#[derive(Error, Debug)]
pub enum HistogramError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("缩略图/解码错误: {0}")]
    Thumbnail(#[from] thumbnail::ThumbnailError),
    #[error("图像解码错误: {0}")]
    Image(#[from] image::ImageError),
}

/// 高光剪切阈值（含）：luma >= 此值计入高光溢出（过曝警告）
pub const CLIP_HIGH_THRESHOLD: u8 = 250;
/// 死黑剪切阈值（含）：luma <= 此值计入死黑（欠曝警告）
pub const CLIP_LOW_THRESHOLD: u8 = 5;
/// 直方图解码长边上限（预览尺寸，别全尺寸解码 24MP）
pub const HISTOGRAM_PREVIEW_LONG_EDGE: u32 = 1600;
/// 剪切叠加图长边（前端以归一化坐标叠在主图上，800 足够标识溢出区域）
pub const CLIP_MASK_LONG_EDGE: u32 = 800;

/// 直方图数据：256 级 luma（BT.601 加权）+ RGB 三通道计数 + 剪切统计。
/// 各通道 bin 总和相等（= 参与统计的像素数）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistogramData {
    /// 亮度直方图（BT.601：Y = (299R + 587G + 114B) / 1000）
    pub luma: [u32; 256],
    /// 红通道直方图
    pub r: [u32; 256],
    /// 绿通道直方图
    pub g: [u32; 256],
    /// 蓝通道直方图
    pub b: [u32; 256],
    /// 高光溢出像素数（luma >= CLIP_HIGH_THRESHOLD）
    pub clip_high_count: u32,
    /// 死黑像素数（luma <= CLIP_LOW_THRESHOLD）
    pub clip_low_count: u32,
}

impl HistogramData {
    /// 参与统计的总像素数（任一通道 bin 之和）
    pub fn total_pixels(&self) -> u64 {
        self.luma.iter().map(|&v| u64::from(v)).sum()
    }
}

/// BT.601 亮度加权（整数定点，避免浮点偏差；加权和 = 1000，结果恒在 0..=255）
fn luma_bt601(p: &image::Rgb<u8>) -> u8 {
    let y = (299 * u32::from(p[0]) + 587 * u32::from(p[1]) + 114 * u32::from(p[2])) / 1000;
    y as u8
}

/// 从 RGB8 像素图计算直方图（纯函数，不碰磁盘；剪切阈值见模块常量）
pub fn compute_histogram(img: &image::RgbImage) -> HistogramData {
    let mut data = HistogramData {
        luma: [0; 256],
        r: [0; 256],
        g: [0; 256],
        b: [0; 256],
        clip_high_count: 0,
        clip_low_count: 0,
    };
    for px in img.pixels() {
        let y = luma_bt601(px);
        data.luma[usize::from(y)] += 1;
        data.r[usize::from(px[0])] += 1;
        data.g[usize::from(px[1])] += 1;
        data.b[usize::from(px[2])] += 1;
        if y >= CLIP_HIGH_THRESHOLD {
            data.clip_high_count += 1;
        } else if y <= CLIP_LOW_THRESHOLD {
            data.clip_low_count += 1;
        }
    }
    data
}

/// 按预览尺寸（长边 ≤ size）加载任意格式图片为 RGB8：
/// - JPEG：jpeg-decoder DCT 降采样（不解全尺寸）
/// - RAW：rawlib half_size 预览（长边 ≤ size），再解其 JPEG 输出
/// - 其他栅格（PNG/TIFF/WebP/BMP/GIF）：image 全解码后 Lanczos 缩到预览尺寸
fn load_preview_rgb(path: &Path, size: u32) -> Result<image::RgbImage, HistogramError> {
    // JPEG 优先：DCT 降采样只解需要的 block，最快
    if let Ok(img) = thumbnail::decode_jpeg_rgb8_scaled(path, size) {
        return Ok(img);
    }
    // RAW：走 rawlib half_size 预览（≈4x 加速，长边 ≤ size），与缩略图/预览同源
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();
    if ImageFormat::from_extension(ext)
        .is_some_and(|f| matches!(f, ImageFormat::Raw(_)))
    {
        let jpeg = thumbnail::decode_raw_preview(path, size)?;
        let img = image::load_from_memory(&jpeg)?;
        return Ok(img.to_rgb8());
    }
    // 其他栅格格式：image 全解码后缩到预览尺寸（JPEG 已在上方短路，这里不会再碰 24MP JPEG）
    let img = image::open(path)?;
    let rgb = img.to_rgb8();
    if rgb.width().max(rgb.height()) <= size {
        return Ok(rgb);
    }
    let ratio = f64::from(size) / f64::from(rgb.width().max(rgb.height()));
    let nw = ((f64::from(rgb.width()) * ratio).round().max(1.0)) as u32;
    let nh = ((f64::from(rgb.height()) * ratio).round().max(1.0)) as u32;
    Ok(image::imageops::resize(&rgb, nw, nh, image::imageops::FilterType::Lanczos3))
}

/// 计算文件的直方图（预览尺寸解码，见 [`load_preview_rgb`]）
pub fn compute_histogram_from_file(path: &Path) -> Result<HistogramData, HistogramError> {
    let img = load_preview_rgb(path, HISTOGRAM_PREVIEW_LONG_EDGE)?;
    Ok(compute_histogram(&img))
}

/// 生成剪切叠加图（RGBA PNG 字节）：红 = 高光溢出（luma >= 250）、
/// 蓝 = 死黑（luma <= 5），其余透明。与直方图同一阈值/luma 公式，
/// 前端把该图以主图同尺寸叠放即可看到过曝/欠曝区域。
pub fn clipping_mask_png(path: &Path, long_edge: u32) -> Result<Vec<u8>, HistogramError> {
    let img = load_preview_rgb(path, long_edge)?;
    let (w, h) = img.dimensions();
    let mut rgba = image::RgbaImage::new(w, h);
    for (x, y, px) in img.enumerate_pixels() {
        let yv = luma_bt601(px);
        let color = if yv >= CLIP_HIGH_THRESHOLD {
            // 高光溢出：红
            image::Rgba([255, 0, 0, 255])
        } else if yv <= CLIP_LOW_THRESHOLD {
            // 死黑：蓝
            image::Rgba([0, 0, 255, 255])
        } else {
            image::Rgba([0, 0, 0, 0])
        };
        rgba.put_pixel(x, y, color);
    }
    let mut out = Vec::new();
    image::DynamicImage::ImageRgba8(rgba).write_to(
        &mut std::io::Cursor::new(&mut out),
        image::ImageFormat::Png,
    )?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造纯色 RGB 图（w×h，三通道同值）——中灰/纯黑/纯白/灰阶测试共用
    fn solid_image(w: u32, h: u32, v: u8) -> image::RgbImage {
        image::RgbImage::from_pixel(w, h, image::Rgb([v, v, v]))
    }

    /// 构造灰阶渐变图（w=256 时逐列 0..=255，用于 bin 总数/单像素映射校验）
    fn gradient_image(w: u32, h: u32) -> image::RgbImage {
        let mut img = image::RgbImage::new(w, h);
        for (x, _y, px) in img.enumerate_pixels_mut() {
            // w=256 时 v = x 恰好逐列覆盖 0..=255（避免除法取整让低值多占列）
            let v = (x % 256) as u8;
            *px = image::Rgb([v, v, v]);
        }
        img
    }

    #[test]
    fn test_histogram_pure_black_all_clip_low() {
        // 纯黑：luma = 0 <= 5，死黑计数 = 全像素；高光 = 0
        let img = solid_image(320, 240, 0);
        let h = compute_histogram(&img);
        assert_eq!(h.clip_low_count, 320 * 240);
        assert_eq!(h.clip_high_count, 0);
        assert_eq!(h.luma[0], 320 * 240);
        assert_eq!(h.luma[255], 0);
        // 剪切计数互斥：全黑像素只计死黑不计高光
        assert_eq!(h.clip_low_count + h.clip_high_count, 320 * 240);
    }

    #[test]
    fn test_histogram_pure_white_all_clip_high() {
        // 纯白：luma = 255 >= 250，高光计数 = 全像素；死黑 = 0
        let img = solid_image(64, 48, 255);
        let h = compute_histogram(&img);
        assert_eq!(h.clip_high_count, 64 * 48);
        assert_eq!(h.clip_low_count, 0);
        assert_eq!(h.luma[255], 64 * 48);
        assert_eq!(h.luma[0], 0);
        assert_eq!(h.clip_low_count + h.clip_high_count, 64 * 48);
    }

    #[test]
    fn test_histogram_mid_gray_peaks_center() {
        // 中灰 128：luma = (299+587+114)*128/1000 = 128，bin[128] 为全像素；无剪切
        let img = solid_image(100, 100, 128);
        let h = compute_histogram(&img);
        assert_eq!(h.clip_high_count, 0);
        assert_eq!(h.clip_low_count, 0);
        assert_eq!(h.luma[128], 100 * 100);
        // 分布居中：128 是唯一非空 bin（两侧都为 0）
        assert_eq!(h.luma[127], 0);
        assert_eq!(h.luma[129], 0);
        assert_eq!(h.luma[0], 0);
        assert_eq!(h.luma[255], 0);
    }

    #[test]
    fn test_histogram_bin_total_equals_pixel_count() {
        // 灰阶渐变（含全 0/全 255 端点）：bin 总数 = 像素数，剪切两态之和 = 端点列像素
        let (w, h) = (256u32, 3u32);
        let img = gradient_image(w, h);
        let hist = compute_histogram(&img);
        assert_eq!(hist.total_pixels(), u64::from(w) * u64::from(h));
        let sum: u32 = hist.luma.iter().sum();
        assert_eq!(sum, w * h);
        // 各通道 bin 总数一致
        for ch in [&hist.r, &hist.g, &hist.b] {
            assert_eq!(ch.iter().sum::<u32>(), w * h);
        }
        // 渐变端点：v=0..5 六列全死黑（6×3=18 像素），v=250..255 六列全高光（同样 18）
        assert_eq!(hist.clip_low_count, 18);
        assert_eq!(hist.clip_high_count, 18);
        // 剪切互斥：中间值不触任何剪切
        assert_eq!(hist.clip_low_count + hist.clip_high_count, 36);
        // BT.601 对灰阶图退化为像素值本身（三通道相等，加权后不变）
        assert_eq!(hist.luma[0], 3);
        assert_eq!(hist.luma[255], 3);
        assert_eq!(hist.luma[128], 3);
    }

    #[test]
    fn test_histogram_channel_bins_match_color() {
        // 纯红图：luma = 299/1000*255 ≈ 76（BT.601 红亮度），红 bin 全量，绿/蓝为 0
        let img = image::RgbImage::from_pixel(50, 50, image::Rgb([255, 0, 0]));
        let h = compute_histogram(&img);
        assert_eq!(h.r[255], 2500);
        assert_eq!(h.g[0], 2500);
        assert_eq!(h.b[0], 2500);
        assert_eq!(h.luma[76], 2500);
        assert_eq!(h.clip_high_count, 0, "纯红 luma≈76 不触剪切");
    }

    #[test]
    fn test_clipping_mask_png_transparent_middle() {
        // 构造 3×1 图：左死黑(0)、中中灰(128)、右高光(255) → PNG 红蓝对半 + 中间透明
        let mut img = image::RgbImage::new(3, 1);
        img.put_pixel(0, 0, image::Rgb([0, 0, 0]));
        img.put_pixel(1, 0, image::Rgb([128, 128, 128]));
        img.put_pixel(2, 0, image::Rgb([255, 255, 255]));
        // 经文件路径走 mask 全链路（PNG 编码解码往返）
        let dir = tempfile::TempDir::new().expect("临时目录创建失败");
        let path = dir.path().join("probe.png");
        img.save(&path).expect("测试图保存失败");

        let png = clipping_mask_png(&path, 800).expect("mask 生成失败");
        let decoded = image::load_from_memory(&png)
            .expect("mask PNG 解码失败")
            .to_rgba8();
        assert_eq!(decoded.dimensions(), (3, 1));
        let px = |x: u32| decoded.get_pixel(x, 0).0;
        assert_eq!(px(0), [0, 0, 255, 255], "死黑应为蓝");
        assert_eq!(px(1), [0, 0, 0, 0], "中灰应透明");
        assert_eq!(px(2), [255, 0, 0, 255], "高光应为红");
    }
}
