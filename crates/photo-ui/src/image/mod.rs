//! 图片管线与三档缓存（对应 §7 与 §12.5）。
//!
//! 三档图源：
//! 1. thumb: 磁盘缓存的网格缩略图 JPEG（.pt/thumbs/{hash}.jpg）
//! 2. master: 预览母版（长边 2560）
//! 3. full: 1:1 全分辨率原图

use parking_lot::RwLock;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use gpui_kit::{Image, ImageFormat};
use photo_domain::{CaptureMeta, ImageFormat as DomainFormat, SourceFile};
use photo_engine::thumbnail::ThumbnailCache;

pub const THUMB_SIZE_GRID: u32 = 440;
pub const THUMB_SIZE_FILMSTRIP: u32 = 180;
pub const MASTER_SIZE: u32 = 2560;

#[derive(Clone)]
pub struct ImageManager {
    thumbnail_cache: Option<ThumbnailCache>,
    master_cache: Arc<RwLock<HashMap<(String, u64), Arc<Image>>>>,
    /// 1:1 全分辨率图源缓存（只留 1 张）
    full_cache: Arc<RwLock<HashMap<(String, u64), Arc<Image>>>>,
}

/// 从 CaptureMeta 构造引擎侧 SourceFile。
/// 网格、胶片条、预览、缩略图管线共用同一构造口径，避免各处手拼字段漂移。
pub fn source_file_of(meta: &CaptureMeta) -> Option<SourceFile> {
    let path = PathBuf::from(&meta.primary_path);
    let ext = path.extension()?.to_str()?;
    let format = DomainFormat::from_extension(ext)?;
    Some(SourceFile {
        path,
        format,
        file_size: meta.file_size,
    })
}

impl ImageManager {
    pub fn new(cache_dir: Option<PathBuf>) -> Self {
        let thumbnail_cache = cache_dir.map(ThumbnailCache::new);
        Self {
            thumbnail_cache,
            master_cache: Arc::new(RwLock::new(HashMap::new())),
            full_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn set_cache_dir(&mut self, cache_dir: Option<PathBuf>) {
        self.thumbnail_cache = cache_dir.map(ThumbnailCache::new);
        self.master_cache.write().clear();
        self.full_cache.write().clear();
    }

    pub fn thumbnail_cache(&self) -> Option<&ThumbnailCache> {
        self.thumbnail_cache.as_ref()
    }

    /// 查询缩略图在磁盘上是否已就绪。若是，直接返回其 PathBuf（可直接传给 img()）
    pub fn get_cached_thumb_path(&self, source: &SourceFile, size: u32) -> Option<PathBuf> {
        let cache = self.thumbnail_cache.as_ref()?;
        let key = cache.cache_key(source, size, "std");
        let path = cache.cache_dir().join(key);
        if path.exists() { Some(path) } else { None }
    }

    /// 便捷方法：传入字符串路径与文件大小查询缩略图
    pub fn get_thumbnail_path(
        &self,
        primary_path: &str,
        file_size: u64,
        size: u32,
    ) -> Option<PathBuf> {
        let source = SourceFile {
            path: PathBuf::from(primary_path),
            format: DomainFormat::Jpeg,
            file_size: Some(file_size),
        };
        self.get_cached_thumb_path(&source, size)
    }

    /// 确保缩略图生成并返回其文件路径
    pub fn ensure_thumb(
        &self,
        source: &SourceFile,
        size: u32,
        cancel: Option<&AtomicBool>,
    ) -> Result<PathBuf, String> {
        let cache = self
            .thumbnail_cache
            .as_ref()
            .ok_or_else(|| "缩略图缓存未初始化".to_string())?;

        let key = cache.cache_key(source, size, "std");
        let path = cache.cache_dir().join(&key);
        if path.exists() {
            return Ok(path);
        }

        cache
            .get_or_generate(source, size, cancel)
            .map_err(|e| e.to_string())?;

        Ok(path)
    }

    /// 加载预览母版（长边 2560），带内存缓存。
    ///
    /// **所有格式都走 ThumbnailCache**，绝不把原图字节交给渲染层——
    /// 实测一张 7232×5424（39MP）原图 RGBA 展开 **157MB**，而 2560 母版只要 19MB；
    /// 而且渲染层（image crate）根本不认 RW2。常规图在这里解码 + 缩放到 2560 后落盘复用。
    pub fn load_master_image(
        &self,
        source: &SourceFile,
        cancel: Option<&AtomicBool>,
    ) -> Result<Arc<Image>, String> {
        let file_size = std::fs::metadata(&source.path)
            .map(|m| m.len())
            .unwrap_or(0);
        let cache_key = (source.path.to_string_lossy().to_string(), file_size);

        if let Some(img) = self.master_cache.read().get(&cache_key).cloned() {
            return Ok(img);
        }

        let cache = self
            .thumbnail_cache
            .as_ref()
            .ok_or_else(|| "缓存未初始化".to_string())?;
        let started = std::time::Instant::now();
        let bytes = cache
            .get_or_generate(source, MASTER_SIZE, cancel)
            .map_err(|e| e.to_string())?;
        let master_bytes = bytes.len();

        let img = Arc::new(Image::from_bytes(ImageFormat::Jpeg, bytes));
        {
            let mut map = self.master_cache.write();
            // 简单上界：每张母版解码后约 19MB，8 张 ≈150MB。
            // 超了整体清空（粗暴但够用）——磁盘上已有 2560 母版，重载只是一次读盘。
            if map.len() >= 8 {
                map.clear();
            }
            map.insert(cache_key, img.clone());
        }
        tracing::debug!(
            "预览母版就绪 {} ({:?}, 母版 {} KB)",
            source.path.display(),
            started.elapsed(),
            master_bytes / 1024
        );
        Ok(img)
    }

    /// 加载 1:1 全分辨率图源（RAW 专用：AHD 全尺寸解码，落盘在 `full` 缓存变体）。
    ///
    /// 只在**显示尺寸超过母版可用像素**（含 1:1）时调用——母版长边只有 [`MASTER_SIZE`]，
    /// 继续放大等于放大母版，糊；这里换成真原图。代价是渲染层全尺寸 RGBA
    /// （44MP ≈ 177MB），所以内存缓存只留最后一张，切换后由新图替换。
    pub fn load_full_image(
        &self,
        source: &SourceFile,
        cancel: Option<&AtomicBool>,
    ) -> Result<Arc<Image>, String> {
        let file_size = std::fs::metadata(&source.path)
            .map(|m| m.len())
            .unwrap_or(0);
        let cache_key = (source.path.to_string_lossy().to_string(), file_size);

        if let Some(img) = self.full_cache.read().get(&cache_key).cloned() {
            return Ok(img);
        }

        let cache = self
            .thumbnail_cache
            .as_ref()
            .ok_or_else(|| "缓存未初始化".to_string())?;
        let started = std::time::Instant::now();
        let bytes = cache
            .get_or_generate_full(source, cancel)
            .map_err(|e| e.to_string())?;
        let full_bytes = bytes.len();

        let img = Arc::new(Image::from_bytes(ImageFormat::Jpeg, bytes));
        {
            let mut map = self.full_cache.write();
            // 全尺寸 RGBA 太大：只留最后一张
            map.clear();
            map.insert(cache_key, img.clone());
        }
        tracing::debug!(
            "1:1 全分辨率就绪 {} ({:?}, {} KB)",
            source.path.display(),
            started.elapsed(),
            full_bytes / 1024
        );
        Ok(img)
    }
}
