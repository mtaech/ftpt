use std::io::Cursor;
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::adjustments::Rgb16Image;
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
    /// RAW 走「派生缩略图优先命中 → 母版兜底派生」：std@size 键可能已是内嵌占位/
    /// 母版升级/历史派生写好的小文件（~40KB），命中即返回，跳过 6MB 母版 IO；
    /// 未命中才读母版（half_size 完整解码，按 `u32::MAX` 键落盘）并 DCT 派生落盘。
    /// 同一文件不再按尺寸重复完整解码。
    pub fn get_or_generate(
        &self,
        source: &SourceFile,
        size: u32,
        cancel: Option<&AtomicBool>,
    ) -> Result<Vec<u8>, ThumbnailError> {
        // RAW：派生缩略图先查（440px ~40KB vs 母版 6-20MB）——滚动网格命中即返回，
        // 不再每次读母版。内嵌占位/母版升级/历史派生都写 std@size 键（见
        // get_or_generate_embedded 注释），同键互斥覆盖，命中即最新版
        if matches!(source.format, ImageFormat::Raw(_)) {
            if size != u32::MAX {
                let thumb_key = self.cache_key(source, size, "std");
                let thumb_path = self.cache_dir.join(&thumb_key);
                if thumb_path.exists() {
                    return Ok(std::fs::read(&thumb_path)?);
                }
            }
            // 读母版前检查取消（跳过 6MB IO）
            if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                return Err(ThumbnailError::Cancelled);
            }
            let master_key = self.cache_key(source, u32::MAX, "std");
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
            // 派生前检查取消（跳过 DCT）
            if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                return Err(ThumbnailError::Cancelled);
            }
            // 派生缩略图落盘（440px JPEG ~40KB）：命中即读小文件，无需重新派生
            let thumb_key = self.cache_key(source, size, "std");
            let thumb_path = self.cache_dir.join(&thumb_key);
            let bytes = resize_jpeg(&master, size)?;
            std::fs::write(&thumb_path, &bytes)?;
            return Ok(bytes);
        }

        // 常规图：按 size 独立缓存（JPEG DCT 缩放本就快，源文件小）
        // 读缓存前检查取消（跳过 IO）
        if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            return Err(ThumbnailError::Cancelled);
        }
        let cache_key = self.cache_key(source, size, "std");
        let cache_path = self.cache_dir.join(&cache_key);

        if cache_path.exists() {
            return Ok(std::fs::read(&cache_path)?);
        }

        let thumb_bytes = self.generate_thumbnail(source, size, cancel)?;
        std::fs::write(&cache_path, &thumb_bytes)?;
        Ok(thumb_bytes)
    }

    /// RAW 缩略图 = 内嵌 JPEG（相机写入，~50ms 提取，足够缩略图使用）。
    /// 落盘到 std 缓存键：旧版本升级生成的清晰版缩略图优先命中，未命中提取内嵌。
    pub fn get_or_generate_embedded(
        &self,
        source: &SourceFile,
        size: u32,
        cancel: Option<&AtomicBool>,
    ) -> Result<Vec<u8>, ThumbnailError> {
        let start = std::time::Instant::now();
        let cache_key = self.cache_key(source, size, "std");
        let cache_path = self.cache_dir.join(&cache_key);
        if cache_path.exists() {
            let bytes = std::fs::read(&cache_path)?;
            tracing::info!("RAW 缩略图缓存命中: {} ({:?})", source.path.display(), start.elapsed());
            return Ok(bytes);
        }
        if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            return Err(ThumbnailError::Cancelled);
        }
        let bytes = decode_raw_embedded_thumb(&source.path, size)?;
        std::fs::write(&cache_path, &bytes)?;
        tracing::info!("RAW 缩略图生成(内嵌提取): {} ({:?})", source.path.display(), start.elapsed());
        Ok(bytes)
    }

    /// RAW 缩略图母版升级：内嵌占位显示后调用。half_size 母版（std@u32::MAX 键）已在
    /// 磁盘时，从母版 DCT 派生清晰版并**覆盖写回 std@size 键**（与 get_or_generate_embedded
    /// 共用键——内嵌占位与清晰版同键互斥覆盖，之后 get_or_generate/get_or_generate_embedded
    /// 命中即最新版）；母版尚未生成（该 RAW 从未预览/放大过）返回 `Ok(None)`，跳过升级。
    pub fn upgrade_embedded_from_master(
        &self,
        source: &SourceFile,
        size: u32,
    ) -> Result<Option<Vec<u8>>, ThumbnailError> {
        let master_key = self.cache_key(source, u32::MAX, "std");
        let master_path = self.cache_dir.join(&master_key);
        if !master_path.exists() {
            return Ok(None);
        }
        let master = std::fs::read(&master_path)?;
        let bytes = resize_jpeg(&master, size)?;
        let thumb_key = self.cache_key(source, size, "std");
        std::fs::write(self.cache_dir.join(&thumb_key), &bytes)?;
        Ok(Some(bytes))
    }

    /// RAW 全尺寸母版（1:1 像素级查看）：AHD 全尺寸解码，独立于 half_size 母版缓存。
    /// 惰性生成（仅在 1:1 查看过才落盘），键带 `full` 变体与 half_size 母版分离。
    pub fn get_or_generate_full(
        &self,
        source: &SourceFile,
        cancel: Option<&AtomicBool>,
    ) -> Result<Vec<u8>, ThumbnailError> {
        let cache_key = self.cache_key(source, u32::MAX, "full");
        let cache_path = self.cache_dir.join(&cache_key);

        if cache_path.exists() {
            return Ok(std::fs::read(&cache_path)?);
        }
        let bytes = decode_raw_full(&source.path, cancel)?;
        std::fs::write(&cache_path, &bytes)?;
        Ok(bytes)
    }

    fn cache_key(&self, source: &SourceFile, size: u32, variant: &str) -> String {
        use std::hash::{Hash, Hasher};
        // 缓存格式版本：解码逻辑修复（如行宽错位）时递增，旧缓存自动失效
        const CACHE_VERSION: u8 = 4;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        CACHE_VERSION.hash(&mut hasher);
        // 变体：std（half_size 母版/常规图）与 full（全尺寸母版）分开，键互不冲突
        variant.hash(&mut hasher);
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
        // 进入解码前检查取消：排队/被标记的任务立即放弃，不占线程
        //（DCT 解码不可中断，只能在此检查点省时间）
        if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
            return Err(ThumbnailError::Cancelled);
        }
        let ext = path.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase());

        if matches!(ext.as_deref(), Some("jpg" | "jpeg"))
            && let Ok(bytes) = self.decode_jpeg_scaled(path, size)
        {
            return Ok(bytes);
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

    /// JPEG DCT 降采样解码：jpeg-decoder 1/2 因子（只解需要的 block）
    fn decode_jpeg_scaled(&self, path: &Path, size: u32) -> Result<Vec<u8>, ThumbnailError> {
        let file = std::fs::File::open(path)?;
        decode_jpeg_scaled_reader(std::io::BufReader::new(file), size)
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

/// RAW 16-bit 母版解码（half_size + 16bit + 自动亮度 + sRGB + 相机白平衡，ADR 0007）。
/// 返回 RGB16 缓冲，供参数化调整：**参数变更只重算像素变换，不重复解码**——
/// 16-bit 母版是 slider 拖动实时性的前提。在 worker 线程中调用。
/// 不落磁盘（内存缓冲，随焦点图淘汰，见性能预算 ≤40MB/张）。
pub fn decode_raw_preview16(
    path: &Path,
    cancel: Option<&AtomicBool>,
) -> Result<Rgb16Image, ThumbnailError> {
    decode_raw16_with_options(path, &rawlib::DecodeOptions::preview16(), cancel)
}

/// 按指定选项解码 RAW 为 16-bit RGB 缓冲（小端 ushort RGB 交织 → Rgb16Image）。
/// 供调整母版（half_size）与导出（全尺寸 quality）共用。
pub fn decode_raw16_with_options(
    path: &Path,
    opts: &rawlib::DecodeOptions,
    cancel: Option<&AtomicBool>,
) -> Result<Rgb16Image, ThumbnailError> {
    if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
        return Err(ThumbnailError::Cancelled);
    }
    let path_str = path.to_string_lossy();
    let img = rawlib::extract_image_with_options(path_str.as_ref(), opts).map_err(|e| {
        tracing::error!("16-bit 解码失败: {} — {e}", path.display());
        ThumbnailError::Raw(e.to_string())
    })?;
    tracing::info!(
        "16-bit 解码成功: {} ({}×{} {}-bit)",
        path.display(),
        img.width,
        img.height,
        img.bits
    );
    if img.bits != 16 {
        return Err(ThumbnailError::Raw(format!(
            "解码输出位深异常: {}（期望 16，实际 {}）",
            path.display(),
            img.bits
        )));
    }
    // LibRaw 16-bit 输出为小端 ushort RGB 交织（本机字节序，x86/ARM LE）
    let mut data = Vec::with_capacity(img.data.len() / 2);
    for ch in img.data.chunks_exact(2) {
        data.push(u16::from_le_bytes([ch[0], ch[1]]));
    }
    let data_len = data.len();
    Rgb16Image::from_raw(img.width as u32, img.height as u32, data).ok_or_else(|| {
        tracing::error!(
            "16-bit 缓冲尺寸不符: {}×{} got {}",
            img.width,
            img.height,
            data_len
        );
        ThumbnailError::Image(image::ImageError::IoError(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "16-bit buffer size mismatch",
        )))
    })
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

/// 从 RAW 提取内嵌缩略图（相机写入的 JPEG，160-640px，约 50ms）。
/// 用作缩略图即时占位（放大略糊），清晰版由母版解码升级后替换。
/// 在 worker 线程中调用。
pub fn decode_raw_embedded_thumb(path: &Path, size: u32) -> Result<Vec<u8>, ThumbnailError> {
    let path_str = path.to_string_lossy();
    let thumb = rawlib::extract_thumbnail_with_info(path_str.as_ref())
        .map_err(|e| ThumbnailError::Raw(e.to_string()))?;
    if thumb.format != rawlib::ImageFormat::Jpeg {
        return Err(ThumbnailError::Raw("内嵌缩略图非 JPEG".into()));
    }
    // 内嵌小图通常小于目标尺寸：resize_jpeg 小图原样返回（不放大，保持原尺寸占位）
    resize_jpeg(&thumb.data, size)
}

/// RAW 全尺寸解码（1:1 像素级查看）：AHD 去马赛克 + 8bit + 自动亮度，不缩放。
/// 相对 half_size 预览慢 4-8x（24MP 约 3-5s），配合磁盘缓存与取消令牌。
/// 在 worker 线程中调用。
fn decode_raw_full(path: &Path, cancel: Option<&AtomicBool>) -> Result<Vec<u8>, ThumbnailError> {
    // 完整 RAW 解码（慢操作）前检查取消
    if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
        return Err(ThumbnailError::Cancelled);
    }
    let path_str = path.to_string_lossy();
    let img = rawlib::extract_image_with_options(path_str.as_ref(), &rawlib::DecodeOptions::full())
        .map_err(|e| {
            tracing::error!("全尺寸解码失败: {} — {e}", path.display());
            ThumbnailError::Raw(e.to_string())
        })?;
    tracing::info!(
        "全尺寸解码成功: {} ({}×{})",
        path.display(),
        img.width,
        img.height
    );
    encode_bitmap_to_jpeg(&img, u32::MAX)
}

/// 只解析 JPEG 头部取尺寸（不解码像素）。
fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut decoder = zune_jpeg::JpegDecoder::new(Cursor::new(bytes));
    decoder.decode_headers().ok()?;
    let info = decoder.info()?;
    Some((info.width as u32, info.height as u32))
}

/// 将 JPEG 字节缩放到目标尺寸（长边 ≤ size）。
/// 快路径：jpeg-decoder DCT 降采样（1/8、1/4、1/2 因子，只解需要的 block）；
/// 失败回退全解码 + Lanczos3。小于目标时原样返回不重编码。
fn resize_jpeg(jpeg_bytes: &[u8], size: u32) -> Result<Vec<u8>, ThumbnailError> {
    if let Some((w, h)) = jpeg_dimensions(jpeg_bytes) {
        if w.max(h) <= size {
            return Ok(jpeg_bytes.to_vec());
        }
        if let Ok(bytes) = decode_jpeg_scaled_reader(Cursor::new(jpeg_bytes), size) {
            return Ok(bytes);
        }
    }
    // 回退：全解码 + Lanczos3（非 JPEG 或 jpeg-decoder 失败时）
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

/// JPEG DCT 降采样解码为 RGB8 像素（长边 ≤ size，只解需要的 block，不重编码）。
/// 供直方图/剪切叠加等需要像素而非 JPEG 字节的场景复用（对齐
/// `decode_jpeg_scaled_reader` 的 DCT 因子选择；1/2 因子输出仍超 size 时 Lanczos 收尾）。
pub fn decode_jpeg_rgb8_scaled(path: &Path, size: u32) -> Result<image::RgbImage, ThumbnailError> {
    use jpeg_decoder::Decoder;
    let file = std::fs::File::open(path)?;
    let mut decoder = Decoder::new(std::io::BufReader::new(file));
    decoder
        .read_info()
        .map_err(|e| ThumbnailError::Image(image::ImageError::IoError(std::io::Error::new(
            std::io::ErrorKind::InvalidData, e.to_string(),
        ))))?;
    let info = decoder.info().ok_or_else(|| {
        ThumbnailError::Image(image::ImageError::IoError(std::io::Error::new(
            std::io::ErrorKind::InvalidData, "missing JPEG info after read_info",
        )))
    })?;
    // 原图长边 ≤ size：不缩放直接全解码（DCT 因子选 1/1 等价，显式短路更省）
    let (out_w, out_h) = if u32::from(info.width.max(info.height)) <= size {
        (info.width, info.height)
    } else {
        let req = size.min(u16::MAX as u32) as u16;
        decoder
            .scale(req, req)
            .map_err(|e| ThumbnailError::Image(image::ImageError::IoError(std::io::Error::new(
                std::io::ErrorKind::InvalidData, e.to_string(),
            ))))?
    };
    let pixels = decoder
        .decode()
        .map_err(|e| ThumbnailError::Image(image::ImageError::IoError(std::io::Error::new(
            std::io::ErrorKind::InvalidData, e.to_string(),
        ))))?;
    let expected = (out_w as u32 * out_h as u32 * 3) as usize;
    if pixels.len() != expected {
        return Err(ThumbnailError::Image(image::ImageError::IoError(
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("JPEG buffer size mismatch: {} != {}", pixels.len(), expected),
            ),
        )));
    }
    let img = image::RgbImage::from_raw(out_w as u32, out_h as u32, pixels).ok_or_else(|| {
        ThumbnailError::Image(image::ImageError::IoError(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "JPEG decode produced invalid buffer",
        )))
    })?;
    // 1/2 因子输出仍超 size（如 4000→2000 但目标 1600）→ Lanczos 收尾
    if img.width().max(img.height()) <= size {
        Ok(img)
    } else {
        let ratio = size as f64 / img.width().max(img.height()) as f64;
        let nw = ((img.width() as f64 * ratio).round().max(1.0)) as u32;
        let nh = ((img.height() as f64 * ratio).round().max(1.0)) as u32;
        Ok(image::imageops::resize(&img, nw, nh, image::imageops::FilterType::Lanczos3))
    }
}

/// jpeg-decoder DCT 降采样解码：从任意 reader 解码并缩放到长边 ≤ size，输出 JPEG 字节。
/// 请求尺寸直接传给 scale：choose_idct_size 取输出 ≥ 请求的最小因子（1/8~1/1）——
/// 小目标走降采样（快），大目标走 1/1 全量（保精度）；输出仍超 size 时 Lanczos 收尾。
fn decode_jpeg_scaled_reader<R: std::io::BufRead + std::io::Seek>(
    reader: R,
    size: u32,
) -> Result<Vec<u8>, ThumbnailError> {
    use jpeg_decoder::Decoder;

    let mut decoder = Decoder::new(reader);
    decoder
        .read_info()
        .map_err(|e| ThumbnailError::Image(image::ImageError::IoError(std::io::Error::new(
            std::io::ErrorKind::InvalidData, e.to_string(),
        ))))?;
    let _info = decoder.info().ok_or_else(|| {
        ThumbnailError::Image(image::ImageError::IoError(std::io::Error::new(
            std::io::ErrorKind::InvalidData, "missing JPEG info after read_info",
        )))
    })?;

    // 请求 = 目标尺寸：DCT 因子自动权衡（≤ size 原样返回由调用方 resize_jpeg 保证）
    let req = size.min(u16::MAX as u32) as u16;
    let (out_w, out_h) = decoder
        .scale(req, req)
        .map_err(|e| ThumbnailError::Image(image::ImageError::IoError(std::io::Error::new(
            std::io::ErrorKind::InvalidData, e.to_string(),
        ))))?;

    let pixels = decoder
        .decode()
        .map_err(|e| ThumbnailError::Image(image::ImageError::IoError(std::io::Error::new(
            std::io::ErrorKind::InvalidData, e.to_string(),
        ))))?;
    let expected = (out_w as u32 * out_h as u32 * 3) as usize;
    if pixels.len() != expected {
        return Err(ThumbnailError::Image(image::ImageError::IoError(
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("JPEG buffer size mismatch: {} != {}", pixels.len(), expected),
            ),
        )));
    }

    let img = image::RgbImage::from_raw(out_w as u32, out_h as u32, pixels).ok_or_else(|| {
        ThumbnailError::Image(image::ImageError::IoError(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "JPEG decode produced invalid buffer",
        )))
    })?;
    let dynamic = image::DynamicImage::ImageRgb8(img);
    // 1/2 因子输出仍超 size（如 4000→2000 但目标 1600）→ Lanczos 收尾
    let resized = if dynamic.width().max(dynamic.height()) <= size {
        dynamic
    } else {
        let ratio = size as f64 / dynamic.width().max(dynamic.height()) as f64;
        let nw = (dynamic.width() as f64 * ratio) as u32;
        let nh = (dynamic.height() as f64 * ratio) as u32;
        image::DynamicImage::ImageRgba8(image::imageops::resize(
            &dynamic, nw, nh, image::imageops::FilterType::Lanczos3,
        ))
    };
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
        // 大图：两个目标尺寸都需缩放（32×32 会走「原样返回」早退，字节相同）
        let img_path = img_dir.path().join("test.jpg");
        let img = image::RgbImage::from_fn(320, 240, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, 128])
        });
        img.save(&img_path).unwrap();

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
        let k1 = cache.cache_key(&mk(Some(100)), 1600, "std");
        let k2 = cache.cache_key(&mk(Some(200)), 1600, "std");
        let k3 = cache.cache_key(&mk(Some(100)), 1600, "std");
        let k4 = cache.cache_key(&mk(None), 1600, "std");
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
