use std::io::Cursor;
use std::path::{Path, PathBuf};
use thiserror::Error;

use photo_domain::{ImageFormat, SourceFile};

use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Error, Debug)]
pub enum ThumbnailError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Image decode/encode error: {0}")]
    Image(#[from] image::ImageError),
    #[error("RAW extraction error: {0}")]
    Raw(String),
    #[error("Cancelled")]
    Cancelled,
}
/// 缩略图缓存管理器
#[derive(Clone)]
pub struct ThumbnailCache {
    cache_dir: PathBuf,
}

impl ThumbnailCache {
    /// 创建缓存管理器
    pub fn new(cache_dir: PathBuf) -> Self {
        std::fs::create_dir_all(&cache_dir).ok();
        Self { cache_dir }
    }

    /// 获取缓存目录路径
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// 获取或生成缩略图，返回 JPEG 字节。
    /// `cancel` 为合作式取消令牌：Some 时每次慢操作前检查，已取消则返回 `Cancelled` 错误。
    ///
    /// RAW 走母版缓存：完整解码（half_size）结果按 `u32::MAX` 键存一份，
    /// 任意 size 请求从母版 DCT 降采样派生（毫秒级，不落盘）——同一文件不再按尺寸重复解码。
    pub fn get_or_generate(
        &self,
        source: &SourceFile,
        size: u32,
        cancel: Option<&AtomicBool>,
    ) -> Result<Vec<u8>, ThumbnailError> {
        // RAW：母版缓存 + 派生
        if matches!(source.format, ImageFormat::Raw(_)) {
            let master_key = self.cache_key(source, u32::MAX);
            let master_path = self.cache_dir.join(&master_key);
            let master: Vec<u8> = if master_path.exists() {
                std::fs::read(&master_path)?
            } else {
                let bytes = decode_raw_impl(&source.path, u32::MAX, cancel)?;
                std::fs::write(&master_path, &bytes)?;
                bytes
            };
            if size == u32::MAX {
                return Ok(master);
            }
            return resize_jpeg(&master, size);
        }

        // 常规图：按 size 独立缓存（JPEG DCT 缩放本就快，源文件小）
        let cache_key = self.cache_key(source, size);
        let cache_path = self.cache_dir.join(&cache_key);

        if cache_path.exists() {
            return Ok(std::fs::read(&cache_path)?);
        }

        let thumb_bytes = self.generate_thumbnail(source, size, cancel)?;
        std::fs::write(&cache_path, &thumb_bytes)?;
        Ok(thumb_bytes)
    }

    fn cache_key(&self, source: &SourceFile, size: u32) -> String {
        use std::hash::{Hash, Hasher};
        // 缓存格式版本：解码逻辑修复（如行宽错位）时递增，旧缓存自动失效
        const CACHE_VERSION: u8 = 3;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        CACHE_VERSION.hash(&mut hasher);
        source.path.to_string_lossy().hash(&mut hasher);
        size.hash(&mut hasher);
        // 文件大小参与键：同名文件被覆盖（重拍导回）时旧缓存自动失效
        source.file_size.hash(&mut hasher);
        format!("{:016x}.jpg", hasher.finish())
    }

    fn generate_thumbnail(
        &self,
        source: &SourceFile,
        size: u32,
        cancel: Option<&AtomicBool>,
    ) -> Result<Vec<u8>, ThumbnailError> {
        self.generate_image_thumbnail(&source.path, size, cancel)
    }

    /// 常规图片格式：JPEG 走 DCT 降采样快路径，其余全解码
    fn generate_image_thumbnail(&self, path: &Path, size: u32, cancel: Option<&AtomicBool>) -> Result<Vec<u8>, ThumbnailError> {
        let ext = path.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase());

        if matches!(ext.as_deref(), Some("jpg" | "jpeg")) {
            if let Ok(bytes) = self.decode_jpeg_scaled(path, size) {
                return Ok(bytes);
            }
        }

        if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            return Err(ThumbnailError::Cancelled);
        }
        let img = image::open(path)?;
        let resized = img.thumbnail(size, size);
        let mut buf = Cursor::new(Vec::new());
        resized.write_to(&mut buf, image::ImageFormat::Jpeg)?;
        Ok(buf.into_inner())
    }

    /// JPEG DCT 降采样解码：zune-jpeg SIMD 加速 + max_width/max_height 自动 DCT 缩放
    fn decode_jpeg_scaled(&self, path: &Path, size: u32) -> Result<Vec<u8>, ThumbnailError> {
        use zune_core::colorspace::ColorSpace;
        use zune_core::options::DecoderOptions;

        let options = DecoderOptions::new_fast()
            .jpeg_set_out_colorspace(ColorSpace::RGB)
            .set_max_width(size as usize)
            .set_max_height(size as usize);

        let file = std::fs::File::open(path)?;
        decode_jpeg_scaled_reader(std::io::BufReader::new(file), options, size)
    }

    /// 获取缓存总大小（字节）
    pub fn cache_size_bytes(&self) -> Result<u64, std::io::Error> {
        let mut total = 0u64;
        for entry in std::fs::read_dir(&self.cache_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                total += entry.metadata()?.len();
            }
        }
        Ok(total)
    }

    /// 清理过期缓存：按 mtime 删除最旧文件直到满足 max_size_bytes
    pub fn prune(&self, max_size_bytes: u64) -> Result<(), std::io::Error> {
        let mut files: Vec<_> = std::fs::read_dir(&self.cache_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
            .collect();

        files.sort_by_key(|e| {
            e.metadata()
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
        });

        let mut total_size: u64 = files
            .iter()
            .filter_map(|f| f.metadata().ok())
            .map(|m| m.len())
            .sum();

        for file in &files {
            if total_size <= max_size_bytes {
                break;
            }
            if let Ok(meta) = file.metadata() {
                let size = meta.len();
                if std::fs::remove_file(file.path()).is_ok() {
                    total_size = total_size.saturating_sub(size);
                }
            }
        }

        Ok(())
    }

    /// 获取缓存统计信息：(文件数, 总字节数)
    pub fn stats(&self) -> Result<(usize, u64), std::io::Error> {
        let files: Vec<_> = std::fs::read_dir(&self.cache_dir)?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
            .collect();

        let count = files.len();
        let total_size = files
            .iter()
            .filter_map(|f| f.metadata().ok())
            .map(|m| m.len())
            .sum();
        Ok((count, total_size))
    }

    /// 清除所有缓存
    pub fn clear(&self) -> Result<(), std::io::Error> {
        for entry in std::fs::read_dir(&self.cache_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                std::fs::remove_file(entry.path())?;
            }
        }
        Ok(())
}
}



/// 从 RAW 解码用于预览的图像，返回 JPEG 字节，长边缩放到 `max_size` 以内（`u32::MAX` 表示不缩放）。
/// 在 worker 线程中调用。
///
/// 策略：内嵌 JPEG 足够大（长边 ≥ 2048，如 Panasonic RW2 / 部分 DNG）时直接使用（快）；
/// 小内嵌（多数相机 160-640px）不再使用——放大会糊，直接完整解码（half_size 预览选项，约 4x 加速）。
pub fn decode_raw_preview(path: &Path, max_size: u32) -> Result<Vec<u8>, ThumbnailError> {
    decode_raw_impl(path, max_size, None)
}

/// RAW 解码共用实现。`cancel` 为合作式取消令牌：完整解码（慢操作）前检查一次。
fn decode_raw_impl(path: &Path, size: u32, cancel: Option<&AtomicBool>) -> Result<Vec<u8>, ThumbnailError> {
    let path_str = path.to_string_lossy();
    // 快路径：内嵌 JPEG 长边 ≥ 2048（大内嵌，如 RW2/DNG 全尺寸预览）直接用，省完整解码。
    // 小内嵌不再返回：160×120 放大到预览/全分辨率会糊，统一走完整解码。
    if let Ok(thumb) = rawlib::extract_thumbnail_with_info(path_str.as_ref()) {
        if thumb.format == rawlib::ImageFormat::Jpeg && thumb.width.max(thumb.height) >= 2048 {
            return resize_jpeg(&thumb.data, size);
        }
        tracing::debug!(
            "内嵌缩略图过小（{}×{}），走完整解码: {}",
            thumb.width, thumb.height, path.display()
        );
    } else {
        tracing::debug!("无内嵌缩略图，走完整解码: {}", path.display());
    }
    // 完整 RAW 解码（half_size 预览选项：输出分辨率减半，约 4x 加速）前检查取消
    if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
        return Err(ThumbnailError::Cancelled);
    }
    let img = rawlib::extract_image_with_options(
        path_str.as_ref(),
        &rawlib::DecodeOptions::preview(),
    )
    .map_err(|e| {
        tracing::error!("完整解码失败: {} — {e}", path.display());
        ThumbnailError::Raw(e.to_string())
    })?;
    tracing::info!("完整解码成功: {} ({}×{})", path.display(), img.width, img.height);
    encode_bitmap_to_jpeg(&img, size)
}

/// 只解析 JPEG 头部取尺寸（不解码像素）。
fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut decoder = zune_jpeg::JpegDecoder::new(Cursor::new(bytes));
    decoder.decode_headers().ok()?;
    let info = decoder.info()?;
    Some((info.width as u32, info.height as u32))
}

/// 将 JPEG 字节缩放到目标尺寸（长边 ≤ size）。
/// 快路径：zune DCT 降采样；失败回退全解码 + Lanczos3。小于目标时原样返回不重编码。
fn resize_jpeg(jpeg_bytes: &[u8], size: u32) -> Result<Vec<u8>, ThumbnailError> {
    if let Some((w, h)) = jpeg_dimensions(jpeg_bytes) {
        if w.max(h) <= size {
            return Ok(jpeg_bytes.to_vec());
        }
        let options = zune_core::options::DecoderOptions::new_fast()
            .jpeg_set_out_colorspace(zune_core::colorspace::ColorSpace::RGB)
            .set_max_width(size as usize)
            .set_max_height(size as usize);
        if let Ok(bytes) = decode_jpeg_scaled_reader(Cursor::new(jpeg_bytes), options, size) {
            return Ok(bytes);
        }
    }
    // 回退：全解码 + Lanczos3（非 JPEG 或 zune 失败时）
    let img = image::load_from_memory(jpeg_bytes)?;
    if img.width().max(img.height()) <= size {
        return Ok(jpeg_bytes.to_vec());
    }
    let ratio = size as f64 / img.width().max(img.height()) as f64;
    let nw = (img.width() as f64 * ratio) as u32;
    let nh = (img.height() as f64 * ratio) as u32;
    let buf = image::imageops::resize(&img, nw, nh, image::imageops::FilterType::Lanczos3);
    let out = image::DynamicImage::ImageRgba8(buf);
    let mut cursor = Cursor::new(Vec::new());
    out.write_to(&mut cursor, image::ImageFormat::Jpeg)?;
    Ok(cursor.into_inner())
}

/// zune-jpeg DCT 降采样解码核心：从任意 reader 解码并缩放到 size 内，输出 JPEG 字节。
fn decode_jpeg_scaled_reader<R: std::io::BufRead + std::io::Seek>(
    reader: R,
    options: zune_core::options::DecoderOptions,
    size: u32,
) -> Result<Vec<u8>, ThumbnailError> {
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(reader, options);
    let pixels = decoder.decode().map_err(|e| {
        ThumbnailError::Image(image::ImageError::IoError(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            e.to_string(),
        )))
    })?;

    let info = decoder.info().ok_or_else(|| {
        ThumbnailError::Image(image::ImageError::IoError(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "missing JPEG info after decode",
        )))
    })?;
    let (out_w, out_h) = (info.width as u32, info.height as u32);
    let expected = (out_w * out_h * 3) as usize;
    if pixels.len() != expected {
        return Err(ThumbnailError::Image(image::ImageError::IoError(
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("JPEG buffer size mismatch: {} != {}", pixels.len(), expected),
            ),
        )));
    }

    let img = image::RgbImage::from_raw(out_w, out_h, pixels).ok_or_else(|| {
        ThumbnailError::Image(image::ImageError::IoError(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "JPEG decode produced invalid buffer",
        )))
    })?;

    let dynamic = image::DynamicImage::ImageRgb8(img);
    let resized = dynamic.thumbnail(size, size);
    let mut buf = Cursor::new(Vec::new());
    resized.write_to(&mut buf, image::ImageFormat::Jpeg)?;
    Ok(buf.into_inner())
}

/// 将 rawlib 解码出的 RGB bitmap 编码为 JPEG，缩放到目标尺寸（`u32::MAX` = 不缩放，母版原尺寸）。
fn encode_bitmap_to_jpeg(img: &rawlib::ThumbnailData, size: u32) -> Result<Vec<u8>, ThumbnailError> {
    let rgb = image::RgbImage::from_raw(img.width as u32, img.height as u32, img.data.clone())
        .ok_or_else(|| {
            tracing::error!(
                "bitmap buffer size mismatch: expected {} bytes, got {}",
                img.width as usize * img.height as usize * 3,
                img.data.len()
            );
            ThumbnailError::Image(image::ImageError::IoError(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "bitmap buffer size mismatch",
            )))
        })?;
    let dynamic = image::DynamicImage::ImageRgb8(rgb);
    // u32::MAX 不缩放：thumbnail(u32::MAX, u32::MAX) 会因 ratio 下溢产生错误结果
    let out = if size == u32::MAX {
        dynamic
    } else {
        dynamic.thumbnail(size, size)
    };
    let mut buf = Cursor::new(Vec::new());
    out.write_to(&mut buf, image::ImageFormat::Jpeg)?;
    Ok(buf.into_inner())
}


#[cfg(test)]
mod tests {
    use super::*;
    use photo_domain::SourceFile;
    use tempfile::TempDir;

    fn create_test_jpeg(path: &Path) -> Result<(), image::ImageError> {
        let img = image::RgbImage::from_fn(32, 32, |x, y| {
            if (x + y) % 2 == 0 {
                image::Rgb([255, 0, 0])
            } else {
                image::Rgb([0, 255, 0])
            }
        });
        img.save(path)?;
        Ok(())
    }

    #[test]
    fn test_thumbnail_returns_non_empty_bytes() {
        let cache_dir = TempDir::new().unwrap();
        let img_dir = TempDir::new().unwrap();
        let img_path = img_dir.path().join("test.jpg");
        create_test_jpeg(&img_path).unwrap();

        let cache = ThumbnailCache::new(cache_dir.path().to_path_buf());
        let source = SourceFile {
            path: img_path,
            format: ImageFormat::Jpeg,
            file_size: None,
        };

        let result = cache.get_or_generate(&source, 64, None).unwrap();
        assert!(!result.is_empty(), "thumbnail bytes should not be empty");
    }

    #[test]
    fn test_cache_hit_returns_same_bytes() {
        let cache_dir = TempDir::new().unwrap();
        let img_dir = TempDir::new().unwrap();
        let img_path = img_dir.path().join("test.jpg");
        create_test_jpeg(&img_path).unwrap();

        let cache = ThumbnailCache::new(cache_dir.path().to_path_buf());
        let source = SourceFile {
            path: img_path.clone(),
            format: ImageFormat::Jpeg,
            file_size: None,
        };

        let first = cache.get_or_generate(&source, 64, None).unwrap();
        let second = cache.get_or_generate(&source, 64, None).unwrap();
        assert_eq!(first, second, "cache hit should return identical bytes");
    }

    #[test]
    fn test_different_sizes_produce_different_cache_keys() {
        let cache_dir = TempDir::new().unwrap();
        let img_dir = TempDir::new().unwrap();
        let img_path = img_dir.path().join("test.jpg");
        create_test_jpeg(&img_path).unwrap();

        let cache = ThumbnailCache::new(cache_dir.path().to_path_buf());
        let source = SourceFile {
            path: img_path,
            format: ImageFormat::Jpeg,
            file_size: None,
        };

        let small = cache.get_or_generate(&source, 32, None).unwrap();
        let large = cache.get_or_generate(&source, 64, None).unwrap();
        assert_ne!(
            small.len(),
            large.len(),
            "different sizes should differ in byte count"
        );
    }

    #[test]
    fn test_cache_stats_and_clear() {
        let cache_dir = TempDir::new().unwrap();
        let img_dir = TempDir::new().unwrap();
        let img_path = img_dir.path().join("test.jpg");
        create_test_jpeg(&img_path).unwrap();

        let cache = ThumbnailCache::new(cache_dir.path().to_path_buf());
        let source = SourceFile {
            path: img_path,
            format: ImageFormat::Jpeg,
            file_size: None,
        };

        let _ = cache.get_or_generate(&source, 64, None).unwrap();
        let (count, _size) = cache.stats().unwrap();
        assert_eq!(count, 1, "should have 1 cached file");

        cache.clear().unwrap();
        let (count_after, _) = cache.stats().unwrap();
        assert_eq!(count_after, 0, "clear should remove all cached files");
    }

    #[test]
    fn test_prune_removes_oldest() {
        let cache_dir = TempDir::new().unwrap();
        let img_dir = TempDir::new().unwrap();

        let cache = ThumbnailCache::new(cache_dir.path().to_path_buf());

        for i in 0..2 {
            let img_path = img_dir.path().join(format!("test{}.jpg", i));
            create_test_jpeg(&img_path).unwrap();
            let source = SourceFile {
                path: img_path,
                format: ImageFormat::Jpeg,
                    file_size: None,
            };
            let _ = cache.get_or_generate(&source, 64, None).unwrap();
        }

        let (count_before, total_size) = cache.stats().unwrap();
        assert_eq!(count_before, 2);

        cache.prune(total_size / 2).unwrap();
        let (count_after, _) = cache.stats().unwrap();
        assert!(count_after < count_before, "prune should reduce file count");
    }

    #[test]
    fn test_nonexistent_file_returns_error() {
        let cache_dir = TempDir::new().unwrap();
        let cache = ThumbnailCache::new(cache_dir.path().to_path_buf());
        let source = SourceFile {
            path: PathBuf::from("/nonexistent/path.jpg"),
            format: ImageFormat::Jpeg,
            file_size: None,
        };

        let result = cache.get_or_generate(&source, 64, None);
        assert!(result.is_err(), "should error on nonexistent file");
    }

    #[test]
    fn test_jpeg_scaled_decode_odd_dimensions_no_shear() {
        // 回归：原 decode_jpeg_scaled 用请求尺寸而非解码器实际尺寸建图，
        // 尺寸不整除时行宽错位导致画面沿对角线剪切。用行均匀图检测：
        // 每行颜色仅由 y 决定，若行宽错位则行内出现巨大色差。
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("odd.jpg");
        let img = image::RgbImage::from_fn(1001, 667, |_x, y| {
            let v = (y % 256) as u8;
            image::Rgb([v, 255 - v, 128])
        });
        img.save(&path).unwrap();

        let cache = ThumbnailCache::new(dir.path().join("thumbs"));
        let source = SourceFile {
            path,
            format: ImageFormat::Jpeg,
            file_size: None,
        };
        let bytes = cache.get_or_generate(&source, 128, None).unwrap();
        let out = image::load_from_memory(&bytes).unwrap().to_rgb8();
        for row in out.rows() {
            let mut row = row;
            let first = *row.next().unwrap();
            for p in row {
                for c in 0..3 {
                    let diff = (p[c] as i16 - first[c] as i16).abs();
                    assert!(diff <= 24, "行内色差 {diff} 过大，疑似解码错位");
                }
            }
        }
    }

    #[test]
    fn test_cache_key_includes_file_size() {
        // 同名文件被覆盖（大小变化）时必须生成不同缓存键，否则命中陈旧缓存
        let cache = ThumbnailCache::new(PathBuf::from("/unused"));
        let mk = |file_size: Option<u64>| SourceFile {
            path: PathBuf::from("/photos/a.NEF"),
            format: ImageFormat::Raw("NEF".into()),
            file_size,
        };
        let k1 = cache.cache_key(&mk(Some(100)), 1600);
        let k2 = cache.cache_key(&mk(Some(200)), 1600);
        let k3 = cache.cache_key(&mk(Some(100)), 1600);
        let k4 = cache.cache_key(&mk(None), 1600);
        assert_ne!(k1, k2, "文件大小不同应产生不同键");
        assert_eq!(k1, k3, "相同输入应产生相同键");
        assert_ne!(k1, k4, "有无文件大小应产生不同键");
    }

    #[test]
    fn test_resize_jpeg_memory_fast_path() {
        // 内存中的大图 JPEG：走 zune DCT 降采样，输出长边 ≤ 目标且仍是 JPEG
        let img = image::RgbImage::from_fn(3200, 2400, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, 128])
        });
        let mut buf = Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Jpeg).unwrap();
        let src = buf.into_inner();

        let out = resize_jpeg(&src, 1600).unwrap();
        assert_eq!(&out[..2], &[0xFF, 0xD8], "应为 JPEG 头");
        let decoded = image::load_from_memory(&out).unwrap();
        assert!(
            decoded.width().max(decoded.height()) <= 1600,
            "长边应 ≤ 1600，实际 {}×{}",
            decoded.width(),
            decoded.height()
        );

        // 小图不重复编码，原样返回
        let small = resize_jpeg(&out, 1600).unwrap();
        assert_eq!(small, out, "小于目标尺寸应原样返回字节");
    }

    #[test]
    fn test_scan_then_generate_thumbnails_pipeline() {
        // 复现应用主链路：扫描目录 → 取 primary → 生成缩略图
        let dir = TempDir::new().unwrap();
        for i in 0..3 {
            let img = image::RgbImage::from_fn(800, 600, |x, y| {
                image::Rgb([(x % 256) as u8, (y % 256) as u8, (i * 60) as u8])
            });
            img.save(dir.path().join(format!("photo_{i}.jpg"))).unwrap();
        }

        let captures = crate::scanner::scan_directory(
            dir.path(),
            &photo_domain::FilterCriteria::default(),
            None,
        )
        .unwrap();
        assert_eq!(captures.len(), 3, "扫描应找到 3 个 capture");

        let cache = ThumbnailCache::new(dir.path().join("thumbs"));
        for capture in &captures {
            let primary = &capture.source_files[capture.primary_index];
            let bytes = cache.get_or_generate(primary, 220, None).unwrap();
            assert!(bytes.len() > 100, "缩略图字节数异常：{}", bytes.len());
            assert_eq!(&bytes[..2], &[0xFF, 0xD8], "应为 JPEG 头");
        }
    }
}
