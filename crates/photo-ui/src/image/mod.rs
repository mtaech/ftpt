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
use photo_domain::{AdjustParams, CaptureMeta, ImageFormat as DomainFormat, SourceFile};
use photo_engine::adjustments::{ToneParams, apply_tone8};
use photo_engine::thumbnail::ThumbnailCache;

pub const THUMB_SIZE_GRID: u32 = 440;
pub const THUMB_SIZE_FILMSTRIP: u32 = 180;
pub const MASTER_SIZE: u32 = 2560;

/// 调整预览重编码的 JPEG 质量：母版本身已是 JPEG，二次编码留足质量余量（ADR 0007 §预览）。
const ADJUST_JPEG_QUALITY: u8 = 90;

#[derive(Clone)]
pub struct ImageManager {
    thumbnail_cache: Option<ThumbnailCache>,
    master_cache: Arc<RwLock<HashMap<(String, u64), Arc<Image>>>>,
    /// 1:1 全分辨率图源缓存（只留 1 张）
    full_cache: Arc<RwLock<HashMap<(String, u64), Arc<Image>>>>,
    /// 调整预览用的母版 8-bit 像素缓存（键 = 路径 + 文件大小，只留最近 2 张）。
    /// 拖动滑杆时只需"重算色调 + 重编码"，不必重新解码母版——这是实时预览的前提。
    base_cache: Arc<RwLock<HashMap<(String, u64), Arc<image::RgbImage>>>>,
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

/// 跨文件夹缩略图定位（统计页的跨文件夹照片列表用）。
///
/// 缩略图缓存**按照片目录隔离**（「目录/.pt/thumbs/{hash}.jpg」），且缓存键含
/// path/size/file_size（见 ThumbnailCache::cache_key）——所以不能拿当前目录的
/// ImageManager 去查别的文件夹。这里按文件夹各建一个只读查询用的实例并缓存复用。
///
/// cache 由调用方持有（一次列表解析内复用，避免逐张重建）；
/// 返回 None = 该照片的缩略图尚未落盘（统计页显示占位）。
pub fn cached_thumb_path_in_folder(
    cache: &mut HashMap<String, ImageManager>,
    folder: &str,
    rel_path: &str,
) -> Option<PathBuf> {
    let full = std::path::Path::new(folder).join(rel_path);
    let ext = full.extension().and_then(|e| e.to_str())?;
    let format = DomainFormat::from_extension(ext)?;
    // 缓存键含 file_size：必须取真实大小，否则键对不上（同名覆盖时也靠它失效）
    let file_size = std::fs::metadata(&full).ok().map(|m| m.len());
    let source = SourceFile {
        path: full,
        format,
        file_size,
    };
    let manager = cache.entry(folder.to_string()).or_insert_with(|| {
        ImageManager::new(Some(
            std::path::Path::new(folder).join(".pt").join("thumbs"),
        ))
    });
    manager.get_cached_thumb_path(&source, THUMB_SIZE_GRID)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photo_engine::thumbnail::ThumbnailCache;

    /// 跨文件夹缩略图定位：按**各照片目录自己的** .pt/thumbs 算键，
    /// 且键含 file_size（对不上就找不到）。
    #[test]
    fn test_cached_thumb_path_in_folder_uses_per_folder_cache_and_size() {
        let root = std::env::temp_dir().join(format!("pt_thumb_resolve_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let folder_a = root.join("A");
        let folder_b = root.join("B");
        std::fs::create_dir_all(folder_a.join(".pt").join("thumbs")).unwrap();
        std::fs::create_dir_all(&folder_b).unwrap();
        std::fs::write(folder_a.join("pic.jpg"), b"fake-jpeg-bytes").unwrap();

        let folder_a_s = folder_a.to_string_lossy().to_string();
        let folder_b_s = folder_b.to_string_lossy().to_string();

        // 按管线的口径自己算一次键，写入「已就绪的缩略图」
        let size = std::fs::metadata(folder_a.join("pic.jpg")).unwrap().len();
        let source = SourceFile {
            path: folder_a.join("pic.jpg"),
            format: DomainFormat::Jpeg,
            file_size: Some(size),
        };
        let cache = ThumbnailCache::new(folder_a.join(".pt").join("thumbs"));
        let key = cache.cache_key(&source, THUMB_SIZE_GRID, "std");
        let thumb = cache.cache_dir().join(&key);
        std::fs::write(&thumb, b"thumb-bytes").unwrap();

        let mut managers: HashMap<String, ImageManager> = HashMap::new();
        assert_eq!(
            cached_thumb_path_in_folder(&mut managers, &folder_a_s, "pic.jpg"),
            Some(thumb.clone())
        );
        // 另一个文件夹（无缓存）→ None，且不会误用 A 的目录
        assert_eq!(
            cached_thumb_path_in_folder(&mut managers, &folder_b_s, "pic.jpg"),
            None
        );
        // 同一文件夹内不存在的文件 → None
        assert_eq!(
            cached_thumb_path_in_folder(&mut managers, &folder_a_s, "missing.jpg"),
            None
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}

impl ImageManager {
    pub fn new(cache_dir: Option<PathBuf>) -> Self {
        let thumbnail_cache = cache_dir.map(ThumbnailCache::new);
        Self {
            thumbnail_cache,
            master_cache: Arc::new(RwLock::new(HashMap::new())),
            full_cache: Arc::new(RwLock::new(HashMap::new())),
            base_cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn set_cache_dir(&mut self, cache_dir: Option<PathBuf>) {
        self.thumbnail_cache = cache_dir.map(ThumbnailCache::new);
        self.master_cache.write().clear();
        self.full_cache.write().clear();
        self.base_cache.write().clear();
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

    /// 渲染「带调整参数」的预览母版（ADR 0007）：母版像素 → 色调变换 → JPEG 字节 → Image。
    ///
    /// - 参数中性 → 直接返回未调整母版（短路到旧路径，零回归）
    /// - 像素源是**已裁到 2560 的母版**（8-bit 语义，RAW/JPEG 同口径）：拖动滑杆只重算色调
    ///   + 重编码，不重新解码原图（RAW half_size 16-bit 解码 ≤2s，做不了实时交互）
    /// - 主线程零像素工作：调用方负责放进后台 executor
    pub fn render_adjusted_master(
        &self,
        source: &SourceFile,
        params: &AdjustParams,
    ) -> Result<Arc<Image>, String> {
        if params.is_neutral() {
            return self.load_master_image(source, None);
        }
        let base = self.master_rgb8(source)?;
        let toned = apply_tone8(&base, &ToneParams::from(params));
        let mut buf = std::io::Cursor::new(Vec::new());
        let mut encoder =
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, ADJUST_JPEG_QUALITY);
        encoder
            .encode(
                toned.as_raw(),
                toned.width(),
                toned.height(),
                image::ExtendedColorType::Rgb8,
            )
            .map_err(|e| format!("调整预览编码失败: {e}"))?;
        Ok(Arc::new(Image::from_bytes(
            ImageFormat::Jpeg,
            buf.into_inner(),
        )))
    }

    /// 取母版的 8-bit RGB 像素（带内存缓存）：同一张图连续拖滑杆只解码一次母版。
    fn master_rgb8(&self, source: &SourceFile) -> Result<Arc<image::RgbImage>, String> {
        let file_size = std::fs::metadata(&source.path)
            .map(|m| m.len())
            .unwrap_or(0);
        let cache_key = (source.path.to_string_lossy().to_string(), file_size);

        if let Some(img) = self.base_cache.read().get(&cache_key).cloned() {
            return Ok(img);
        }

        let cache = self
            .thumbnail_cache
            .as_ref()
            .ok_or_else(|| "缓存未初始化".to_string())?;
        let bytes = cache
            .get_or_generate(source, MASTER_SIZE, None)
            .map_err(|e| e.to_string())?;
        let decoded = image::load_from_memory(&bytes)
            .map_err(|e| format!("母版解码失败: {e}"))?
            .to_rgb8();
        let img = Arc::new(decoded);
        {
            let mut map = self.base_cache.write();
            // 2560 母版 RGB 约 13MB/张；两张足够覆盖"上一张来回切"。
            if map.len() >= 2 {
                map.clear();
            }
            map.insert(cache_key, img.clone());
        }
        Ok(img)
    }
}
