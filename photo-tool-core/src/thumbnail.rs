use std::io::Cursor;
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::domain::{ImageFormat, SourceFile};

#[derive(Error, Debug)]
pub enum ThumbnailError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Image decode/encode error: {0}")]
    Image(#[from] image::ImageError),
    #[error("RAW extraction error: {0}")]
    Raw(String),
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

    /// 获取或生成缩略图，返回 JPEG 字节
    pub fn get_or_generate(
        &self,
        source: &SourceFile,
        size: u32,
    ) -> Result<Vec<u8>, ThumbnailError> {
        let cache_key = self.cache_key(source, size);
        let cache_path = self.cache_dir.join(&cache_key);

        if cache_path.exists() {
            return Ok(std::fs::read(&cache_path)?);
        }

        let thumb_bytes = self.generate_thumbnail(source, size)?;
        std::fs::write(&cache_path, &thumb_bytes)?;
        Ok(thumb_bytes)
    }

    fn cache_key(&self, source: &SourceFile, size: u32) -> String {
        use std::hash::{Hash, Hasher};
        // 缓存格式版本：解码逻辑修复（如行宽错位）时递增，旧缓存自动失效
        const CACHE_VERSION: u8 = 2;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        CACHE_VERSION.hash(&mut hasher);
        source.path.to_string_lossy().hash(&mut hasher);
        size.hash(&mut hasher);
        format!("{:016x}.jpg", hasher.finish())
    }

    fn generate_thumbnail(
        &self,
        source: &SourceFile,
        size: u32,
    ) -> Result<Vec<u8>, ThumbnailError> {
        match source.format {
            ImageFormat::Raw(_) => self.generate_raw_thumbnail(&source.path, size),
            _ => self.generate_image_thumbnail(&source.path, size),
        }
    }

    /// RAW 格式：通过 rawlib 提取内嵌缩略图
    fn generate_raw_thumbnail(&self, path: &Path, size: u32) -> Result<Vec<u8>, ThumbnailError> {
        let path_str = path.to_string_lossy();
        let thumb_bytes = rawlib::extract_thumbnail(path_str.as_ref())
            .map_err(|e| ThumbnailError::Raw(e.to_string()))?;
        self.resize_jpeg(&thumb_bytes, size)
    }

    /// 常规图片格式：JPEG 走 DCT 降采样快路径，其余全解码
    fn generate_image_thumbnail(&self, path: &Path, size: u32) -> Result<Vec<u8>, ThumbnailError> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase());

        // JPEG 快路径：DCT 降采样（1/2, 1/4, 1/8），比全解码快 10-20x
        if matches!(ext.as_deref(), Some("jpg" | "jpeg")) {
            if let Ok(bytes) = self.decode_jpeg_scaled(path, size) {
                return Ok(bytes);
            }
            // 失败时回退全解码
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
        use zune_jpeg::JpegDecoder;

        let options = DecoderOptions::new_fast()
            .jpeg_set_out_colorspace(ColorSpace::RGB)
            .set_max_width(size as usize)
            .set_max_height(size as usize);

        let file = std::fs::File::open(path)?;
        let mut decoder =
            JpegDecoder::new_with_options(std::io::BufReader::new(file), options);
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

    /// 将 JPEG 字节缩放到目标尺寸
    fn resize_jpeg(&self, jpeg_bytes: &[u8], size: u32) -> Result<Vec<u8>, ThumbnailError> {
        let img = image::load_from_memory(jpeg_bytes)?;
        let resized = img.thumbnail(size, size);
        let mut buf = Cursor::new(Vec::new());
        resized.write_to(&mut buf, image::ImageFormat::Jpeg)?;
        Ok(buf.into_inner())
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

/// 通过 rawlib 完整解码 RAW 文件（half_size 去马赛克，速度约 4x），
/// 返回 JPEG 字节。在 worker 线程中调用，不会阻塞 UI。
///
/// half_size 将输出分辨率减半 —— 对高像素 RAW 预览足够清晰，远好于内嵌 JPEG。
pub fn decode_raw_preview(path: &Path, max_size: u32) -> Result<Vec<u8>, ThumbnailError> {
    use std::io::Cursor;

    let raw_img = rawlib::extract_image_half_size(path, true).map_err(|e| {
        ThumbnailError::Raw(e.to_string())
    })?;
    let orig_w = raw_img.width as u32;
    let orig_h = raw_img.height as u32;
    let colors = raw_img.colors as usize;
    let bits = raw_img.bits;

    // 16-bit -> 8-bit 降采样
    let rgb8: Vec<u8> = if bits >= 16 {
        let channel_bytes = (bits / 8) as usize;
        raw_img
            .data
            .chunks_exact(colors * channel_bytes)
            .flat_map(|pixel| {
                (0..colors.min(3)).map(|i| pixel[i * channel_bytes + (channel_bytes - 1)])
            })
            .collect()
    } else {
        raw_img
            .data
            .chunks_exact(colors)
            .flat_map(|pixel| {
                let r = pixel[0];
                let g = if colors > 1 { pixel[1] } else { r };
                let b = if colors > 2 { pixel[2] } else { r };
                [r, g, b]
            })
            .collect()
    };

    let img = image::RgbImage::from_raw(orig_w, orig_h, rgb8)
        .ok_or_else(|| {
            ThumbnailError::Image(image::ImageError::IoError(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "RAW decode produced invalid buffer",
            )))
        })?;
    let dynamic = image::DynamicImage::ImageRgb8(img);

    let resized = if orig_w.max(orig_h) > max_size {
        dynamic.thumbnail(max_size, max_size)
    } else {
        dynamic
    };

    let mut buf = Cursor::new(Vec::new());
    resized.write_to(&mut buf, image::ImageFormat::Jpeg)?;
    Ok(buf.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::SourceFile;
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
            is_sidecar: false,
            file_size: None,
        };

        let result = cache.get_or_generate(&source, 64).unwrap();
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
            is_sidecar: false,
            file_size: None,
        };

        let first = cache.get_or_generate(&source, 64).unwrap();
        let second = cache.get_or_generate(&source, 64).unwrap();
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
            is_sidecar: false,
            file_size: None,
        };

        let small = cache.get_or_generate(&source, 32).unwrap();
        let large = cache.get_or_generate(&source, 64).unwrap();
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
            is_sidecar: false,
            file_size: None,
        };

        let _ = cache.get_or_generate(&source, 64).unwrap();
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
                is_sidecar: false,
                file_size: None,
            };
            let _ = cache.get_or_generate(&source, 64).unwrap();
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
            is_sidecar: false,
            file_size: None,
        };

        let result = cache.get_or_generate(&source, 64);
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
            is_sidecar: false,
            file_size: None,
        };
        let bytes = cache.get_or_generate(&source, 128).unwrap();
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
            &["xmp".to_string()],
            &crate::domain::FilterCriteria::default(),
            None,
        )
        .unwrap();
        assert_eq!(captures.len(), 3, "扫描应找到 3 个 capture");

        let cache = ThumbnailCache::new(dir.path().join("thumbs"));
        for capture in &captures {
            let primary = &capture.source_files[capture.primary_index];
            let bytes = cache.get_or_generate(primary, 220).unwrap();
            assert!(bytes.len() > 100, "缩略图字节数异常：{}", bytes.len());
            assert_eq!(&bytes[..2], &[0xFF, 0xD8], "应为 JPEG 头");
        }
    }
}
