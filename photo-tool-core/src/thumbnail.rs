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
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
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

    /// 常规图片格式：优先提取 EXIF 内嵌缩略图，失败时回退完整解码
    fn generate_image_thumbnail(&self, path: &Path, size: u32) -> Result<Vec<u8>, ThumbnailError> {
        // 快路径：EXIF 内嵌缩略图（相机 JPEG 几乎都有，直接提取无需解码）
        if let Some(thumb) = extract_exif_thumbnail(path) {
            return self.resize_jpeg(&thumb, size);
        }
        // 慢路径：完整解码
        let img = image::open(path)?;
        let resized = img.thumbnail(size, size);
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

// ── 模块级辅助函数 ──

/// 从 JPEG 文件中提取 EXIF 内嵌缩略图（零解码开销）
fn extract_exif_thumbnail(path: &Path) -> Option<Vec<u8>> {
    let file = std::fs::File::open(path).ok()?;
    let mut reader = std::io::BufReader::new(file);

    let raw_tiff = exif::get_exif_attr_from_jpeg(&mut reader).ok()?;
    if raw_tiff.len() < 8 {
        return None;
    }

    let tiff = &raw_tiff;
    let little_endian = tiff[0] == b'I';
    let read_u32 = |offset: usize| -> u32 {
        let b = &tiff[offset..offset + 4];
        if little_endian {
            u32::from_le_bytes([b[0], b[1], b[2], b[3]])
        } else {
            u32::from_be_bytes([b[0], b[1], b[2], b[3]])
        }
    };
    let read_u16 = |offset: usize| -> u16 {
        let b = &tiff[offset..offset + 2];
        if little_endian {
            u16::from_le_bytes([b[0], b[1]])
        } else {
            u16::from_be_bytes([b[0], b[1]])
        }
    };

    let ifd0_offset = read_u32(4) as usize;
    let ifd1_offset = read_ifd_next(tiff, ifd0_offset, little_endian)?;

    let mut thumb_offset: Option<u32> = None;
    let mut thumb_length: Option<u32> = None;

    let entry_count = read_u16(ifd1_offset) as usize;
    for i in 0..entry_count {
        let entry_pos = ifd1_offset + 2 + i * 12;
        if entry_pos + 12 > tiff.len() {
            break;
        }
        let tag = read_u16(entry_pos);
        let value = read_u32(entry_pos + 8);
        match tag {
            0x0201 => thumb_offset = Some(value),
            0x0202 => thumb_length = Some(value),
            _ => {}
        }
    }

    let offset = thumb_offset? as usize;
    let length = thumb_length? as usize;
    if offset + length > tiff.len() {
        return None;
    }

    Some(tiff[offset..offset + length].to_vec())
}

/// 读取 IFD 的 next-IFD 偏移指针
fn read_ifd_next(tiff: &[u8], ifd_offset: usize, little_endian: bool) -> Option<usize> {
    let read_u16 = |offset: usize| -> u16 {
        let b = &tiff[offset..offset + 2];
        if little_endian {
            u16::from_le_bytes([b[0], b[1]])
        } else {
            u16::from_be_bytes([b[0], b[1]])
        }
    };
    let read_u32 = |offset: usize| -> u32 {
        let b = &tiff[offset..offset + 4];
        if little_endian {
            u32::from_le_bytes([b[0], b[1], b[2], b[3]])
        } else {
            u32::from_be_bytes([b[0], b[1], b[2], b[3]])
        }
    };

    let count = read_u16(ifd_offset) as usize;
    let next_offset_pos = ifd_offset + 2 + count * 12;
    if next_offset_pos + 4 > tiff.len() {
        return None;
    }
    let next = read_u32(next_offset_pos) as usize;
    if next == 0 {
        None
    } else {
        Some(next)
    }
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
}
