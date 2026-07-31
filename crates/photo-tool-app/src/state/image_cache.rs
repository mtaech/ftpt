use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use gpui::*;

use super::app::{RootView, ViewMode};
use crate::state::preview_math::{clamp_pan_axis, pan_after_cursor_zoom};

// 预览/全分辨率图缓存与缩放数学方法（自 state/app.rs 拆出，纯移动，无逻辑改动）

/// 将 JPEG/常规图字节解码为可直接绘制的 RenderImage（worker 线程执行）。
/// 预览/全分辨率必须预解码：字节源走 GPUI asset 异步解码，解码完成前 img 画空白，
/// 源切换时会闪白屏；RenderImage 走 ImageSource::Render 同步路径，到达即可绘制。
pub(crate) fn decode_render_image(bytes: &[u8], is_jpeg: bool) -> Option<RenderImage> {
    let mut rgba = if is_jpeg {
        let options = zune_core::options::DecoderOptions::new_fast()
            .jpeg_set_out_colorspace(zune_core::colorspace::ColorSpace::RGB);
        let mut decoder =
            zune_jpeg::JpegDecoder::new_with_options(std::io::Cursor::new(bytes), options);
        let pixels = decoder.decode().ok()?;
        let info = decoder.info()?;
        let rgb = image::RgbImage::from_raw(info.width as u32, info.height as u32, pixels)?;
        image::DynamicImage::ImageRgb8(rgb).into_rgba8()
    } else {
        image::load_from_memory(bytes).ok()?.into_rgba8()
    };
    // GPUI RenderImage 帧要求 BGRA 通道序（gpui assets.rs / img.rs 均为 swap(0,2)），
    // 不交换则红蓝对调，画面整体偏青
    for px in rgba.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    Some(RenderImage::new(vec![image::Frame::new(rgba)]))
}

impl RootView {
    /// 预览图缓存上限（张）
    const PREVIEW_CACHE_LIMIT: usize = 20;
    /// 全分辨率图缓存上限（张）：单张 GPU 纹理可达百 MB，只保留当前与相邻
    const FULLRES_CACHE_LIMIT: usize = 2;
    /// 预览图长边像素（磁盘缓存键的一部分）
    pub const PREVIEW_LOAD_SIZE: u32 = 1600;
    /// 预览预取半径：焦点前后各预取几张
    const PREVIEW_PREFETCH_RADIUS: usize = 2;

    fn insert_preview(&mut self, idx: usize, image: Arc<RenderImage>) {
        self.preview_order.retain(|&i| i != idx);
        self.preview_order.push_back(idx);
        self.preview_data.insert(idx, image);
        while self.preview_order.len() > Self::PREVIEW_CACHE_LIMIT {
            if let Some(oldest) = self.preview_order.pop_front() {
                self.preview_data.remove(&oldest);
            }
        }
    }

    fn insert_fullres(&mut self, idx: usize, image: Arc<RenderImage>) {
        self.fullres_order.retain(|&i| i != idx);
        self.fullres_order.push_back(idx);
        self.fullres_data.insert(idx, image);
        while self.fullres_order.len() > Self::FULLRES_CACHE_LIMIT {
            if let Some(oldest) = self.fullres_order.pop_front() {
                self.fullres_data.remove(&oldest);
            }
        }
    }

    /// 确保当前焦点图片的预览数据已加载：全部格式经 worker 线程缩放到 1600px。
    /// 预取焦点前后各 PREVIEW_PREFETCH_RADIUS 张；离开邻域的未完成慢解码（RAW）被取消。
    pub fn ensure_preview_loaded(&mut self, cx: &mut Context<Self>) {
        let Some(focus_idx) = self.focus_index else { return };
        // 取消离开预取邻域的未完成加载，释放 worker 线程给当前邻域
        let keep_start = focus_idx.saturating_sub(Self::PREVIEW_PREFETCH_RADIUS);
        let keep_end = (focus_idx + Self::PREVIEW_PREFETCH_RADIUS).min(self.display_order.len().saturating_sub(1));
        let keep: HashSet<usize> = (keep_start..=keep_end)
            .filter_map(|di| self.display_order.get(di).copied())
            .collect();
        self.preview_cancel.retain(|&ci, token| {
            if keep.contains(&ci) {
                true
            } else {
                token.store(true, std::sync::atomic::Ordering::Relaxed);
                false
            }
        });
        // 焦点图优先入队，再向左右扩展：解码是用户等待的结果，焦点图必须先到
        self.spawn_preview_load(focus_idx, cx);
        for step in 1..=Self::PREVIEW_PREFETCH_RADIUS {
            if focus_idx >= step {
                self.spawn_preview_load(focus_idx - step, cx);
            }
            if focus_idx + step < self.display_order.len() {
                self.spawn_preview_load(focus_idx + step, cx);
            }
        }
    }

    /// 为指定 display_order 索引 spawn 预览图加载任务（已缓存或已在加载则跳过）
    fn spawn_preview_load(&mut self, display_idx: usize, cx: &mut Context<Self>) {
        use photo_domain::{ImageFormat as DomainFormat, SourceFile};

        let Some(&capture_idx) = self.display_order.get(display_idx) else { return };
        if self.preview_data.contains_key(&capture_idx) { return; }
        if !self.preview_loading.insert(capture_idx) { return; } // 已在加载
        let Some(meta) = self.captures.get(capture_idx) else { return };
        let generation = self.scan_generation;

        let path = PathBuf::from(&meta.primary_path);
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        // RAW 与常规图统一走缩略图缓存：RAW 提取内嵌 JPEG（快，已处理降噪/锐化）
        let Some(cache) = self.thumbnail_cache.clone() else { return };
        let source = SourceFile {
            path,
            format: DomainFormat::from_extension(&ext).unwrap_or(DomainFormat::Jpeg),

            file_size: meta.file_size,
        };
        let cancel = Arc::new(AtomicBool::new(false));
        self.preview_cancel.insert(capture_idx, cancel.clone());
        // 交互优先池：预览是用户等待的结果，不被批量预加载任务排队阻塞
        self.worker.spawn_fast(
            cx,
            move || {
                // 预览字节恒为 JPEG（缩略图磁盘缓存统一存 JPEG）
                match cache.get_or_generate(&source, Self::PREVIEW_LOAD_SIZE, Some(&cancel)) {
                    Ok(bytes) => decode_render_image(&bytes, true).map(Arc::new),
                    Err(photo_engine::thumbnail::ThumbnailError::Cancelled) => None,
                    Err(e) => {
                        tracing::warn!("预览图生成失败: {e}");
                        None
                    }
                }
            },
            move |this, result, cx| {
                // 过期目录：结果作废，不写缓存（加载哨兵已在换代时清空）
                if generation != this.scan_generation {
                    return;
                }
                this.preview_loading.remove(&capture_idx);
                this.preview_cancel.remove(&capture_idx);
                if let Some(image) = result {
                    this.insert_preview(capture_idx, image);
                    cx.notify();
                }
            },
        );
    }

    /// 当前预览显示尺寸是否需要全分辨率源：100%（zoom==0）或显示尺寸超过预览分辨率。
    pub fn needs_fullres(&self) -> bool {
        if self.view_mode != ViewMode::Preview { return false; }
        if self.preview_zoom == 0.0 { return true; }
        match self.preview_disp_size() {
            Some((w, h)) => w > Self::PREVIEW_LOAD_SIZE as f32 || h > Self::PREVIEW_LOAD_SIZE as f32,
            None => false,
        }
    }

    /// 确保焦点图的全分辨率版本在加载（needs_fullres 为真时由缩放/渲染路径触发）。
    /// RAW 提取内嵌全尺寸 JPEG（走磁盘缓存），常规图直接读原文件字节（重编码只会损质量）。
    pub fn ensure_fullres_loaded(&mut self, cx: &mut Context<Self>) {
        use photo_domain::{ImageFormat as DomainFormat, SourceFile};

        let Some(focus_idx) = self.focus_index else { return };
        let Some(&capture_idx) = self.display_order.get(focus_idx) else { return };
        if self.fullres_data.contains_key(&capture_idx) { return; }
        if !self.fullres_loading.insert(capture_idx) { return; } // 已在加载
        let Some(meta) = self.captures.get(capture_idx) else { return };
        let generation = self.scan_generation;

        let path = PathBuf::from(&meta.primary_path);
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let is_raw = matches!(DomainFormat::from_extension(&ext), Some(DomainFormat::Raw(_)));
        let cache = self.thumbnail_cache.clone();
        let source = SourceFile {
            path: path.clone(),
            format: DomainFormat::from_extension(&ext).unwrap_or(DomainFormat::Jpeg),
            file_size: meta.file_size,
        };
        self.worker.spawn_fast(
            cx,
            move || {
                if is_raw {
                    // u32::MAX = 不缩放：完整解码（half_size）母版入磁盘缓存，下次秒开
                    let cache = cache?;
                    cache.get_or_generate(&source, u32::MAX, None)
                        .map_err(|e| tracing::warn!("全分辨率 RAW 预览生成失败: {e}"))
                        .ok()
                        .and_then(|b| decode_render_image(&b, true))
                        .map(Arc::new)
                } else {
                    let is_jpeg = matches!(ext.as_str(), "jpg" | "jpeg");
                    std::fs::read(&path)
                        .map_err(|e| tracing::warn!("全分辨率图读取失败: {e}"))
                        .ok()
                        .and_then(|b| decode_render_image(&b, is_jpeg))
                        .map(Arc::new)
                }
            },
            move |this, result, cx| {
                if generation != this.scan_generation {
                    return;
                }
                this.fullres_loading.remove(&capture_idx);
                if let Some(image) = result {
                    this.insert_fullres(capture_idx, image);
                    cx.notify();
                }
            },
        );
    }



    // ── 预览缩放/平移数学（与 preview.rs 渲染公式严格一致）──

    /// 预览容器尺寸（图片区实测边界 − p_4 内边距 16×2）；未测量返回 None
    fn preview_container_size(&self) -> Option<(f32, f32)> {
        let (_, _, w, h) = *self.preview_area_bounds.borrow();
        if w <= 0. {
            return None;
        }
        Some(((w - 32.).max(1.), (h - 32.).max(1.)))
    }

    /// 焦点图原生像素尺寸
    fn focused_native_size(&self) -> Option<(f32, f32)> {
        let meta = self.get_focused_capture()?;
        Some((meta.image_width? as f32, meta.image_height? as f32))
    }

    /// fit 尺寸（适配容器，不放大）
    pub fn preview_fit_size(&self) -> Option<(f32, f32)> {
        let (cw, ch) = self.preview_container_size()?;
        let (nw, nh) = self.focused_native_size()?;
        if nw <= 0. || nh <= 0. {
            return None;
        }
        let scale = (cw / nw).min(ch / nh).min(1.0);
        Some((nw * scale, nh * scale))
    }

    /// 当前显示尺寸：zoom==0 → 原生像素（100%）；否则 fit × zoom
    pub fn preview_disp_size(&self) -> Option<(f32, f32)> {
        if self.preview_zoom == 0.0 {
            self.focused_native_size().or_else(|| self.preview_fit_size())
        } else {
            self.preview_fit_size()
                .map(|(w, h)| (w * self.preview_zoom, h * self.preview_zoom))
        }
    }

    /// 有效缩放倍率（zoom==0 的 100% 换算为相对 fit 的倍率）
    fn effective_zoom(&self) -> f32 {
        if self.preview_zoom != 0.0 {
            return self.preview_zoom;
        }
        match (self.preview_fit_size(), self.focused_native_size()) {
            (Some((fw, _)), Some((nw, _))) if fw > 0. => nw / fw,
            _ => 1.0,
        }
    }

    /// 按固定步进缩放（×1.25 / ÷1.25）；cursor（容器坐标）给定时以光标为中心
    pub fn zoom_step(&mut self, zoom_in: bool, cursor: Option<(f32, f32)>, cx: &mut Context<Self>) {
        let eff = self.effective_zoom();
        let z = if zoom_in { eff * 1.25 } else { eff / 1.25 };
        self.zoom_to(z, cursor, cx);
    }

    /// 设置缩放倍率（0.1–10 钳制）；cursor（容器坐标）给定时以光标为中心缩放。
    /// 缩放后钳制平移并按需触发全分辨率加载。
    pub fn zoom_to(&mut self, new_zoom: f32, cursor: Option<(f32, f32)>, cx: &mut Context<Self>) {
        let new_zoom = new_zoom.clamp(0.1, 10.0);
        if let (Some(container), Some(fit)) = (self.preview_container_size(), self.preview_fit_size()) {
            let old_eff = self.effective_zoom();
            let old_disp = (fit.0 * old_eff, fit.1 * old_eff);
            let new_disp = (fit.0 * new_zoom, fit.1 * new_zoom);
            if let Some(cur) = cursor {
                self.preview_pan = pan_after_cursor_zoom(old_disp, new_disp, container, self.preview_pan, cur);
            }
        }
        self.preview_zoom = new_zoom;
        self.clamp_preview_pan();
        if self.needs_fullres() {
            self.ensure_fullres_loaded(cx);
        }
        cx.notify();
    }

    /// 切到 100% 实际像素（zoom==0），触发全分辨率加载
    pub fn zoom_to_actual(&mut self, cx: &mut Context<Self>) {
        self.preview_zoom = 0.0;
        self.clamp_preview_pan();
        self.ensure_fullres_loaded(cx);
        cx.notify();
    }

    /// 平移钳制：图片 ≤ 容器时回中；> 容器时边缘不可进入视口
    pub fn clamp_preview_pan(&mut self) {
        let (Some(container), Some(disp)) = (self.preview_container_size(), self.preview_disp_size()) else {
            return;
        };
        self.preview_pan = (
            clamp_pan_axis(disp.0, container.0, self.preview_pan.0),
            clamp_pan_axis(disp.1, container.1, self.preview_pan.1),
        );
    }

    /// 预载焦点前后 ±20 张网格缩略图，供预览缩略图条使用。    /// ensure_thumbnail_loaded 内部有双重早退（已缓存/加载中），重复调用零成本。
    pub fn ensure_filmstrip_thumbs_loaded(&mut self, cx: &mut Context<Self>) {
        if self.display_order.is_empty() { return; }
        let center = self.focus_index.unwrap_or(0);
        let start = center.saturating_sub(20);
        let end = (center + 20).min(self.display_order.len() - 1);
        for di in start..=end {
            let ci = self.display_order[di];
            self.ensure_thumbnail_loaded(ci, cx);
        }
    }

    /// 缩略图条滚动到指定项并尽量居中（切换图片时跟随焦点）。
    /// 布局 bounds 未就绪（如刚进预览的首帧）时由 scroll_to_item 兜底，
    /// active_item 会保留到子项 bounds 出现后做最小滚动，后续导航再居中。
    pub fn scroll_filmstrip_to(&self, display_idx: usize) {
        self.filmstrip_scroll.scroll_to_item(display_idx);
        let Some(item) = self.filmstrip_scroll.bounds_for_item(display_idx) else { return };
        let container = self.filmstrip_scroll.bounds();
        let target_x = container.left() + (container.size.width - item.size.width) / 2. - item.left();
        let y = self.filmstrip_scroll.offset().y;
        // set_offset 在 prepaint 中会被 clamp 到合法范围，首/末项自动贴边
        self.filmstrip_scroll.set_offset(point(target_x, y));
    }

    /// 为单个 capture 生成网格缩略图（懒加载：滚动时按需触发）
    pub fn ensure_thumbnail_loaded(&mut self, capture_idx: usize, cx: &mut Context<Self>) {
        if self.thumbnail_data.contains_key(&capture_idx) { return; }
        if !self.grid_loading.insert(capture_idx) { return; } // 已在加载
        let generation = self.scan_generation;

        let thumbnail_size = self.config.thumbnail_size;
        let Some(cache) = self.thumbnail_cache.clone() else { return };
        let Some(meta) = self.captures.get(capture_idx) else { return };

        let path = PathBuf::from(&meta.primary_path);
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let format = photo_domain::ImageFormat::from_extension(&ext)
            .unwrap_or(photo_domain::ImageFormat::Jpeg);

        let source = photo_domain::SourceFile {
            path,
            format,

            file_size: meta.file_size,
        };

        self.worker.spawn_fast(
            cx,
            move || {
                cache
                    .get_or_generate(&source, thumbnail_size * 2, None)
                    .map_err(|e| {
                        tracing::warn!("懒加载缩略图失败 {}: {e}", source.path.display());
                    })
                    .ok()
            },
            move |this, result, cx| {
                if generation != this.scan_generation {
                    return;
                }
                this.grid_loading.remove(&capture_idx);
                if let Some(bytes) = result {
                    this.thumbnail_data
                        .insert(capture_idx, Arc::new(Image::from_bytes(ImageFormat::Jpeg, bytes)));
                    cx.notify();
                }
            },
        );
    }

    /// Spawn background tasks to preload thumbnails for the first N visible items.
    pub fn preload_thumbnails(&mut self, cx: &mut Context<Self>) {
        use photo_domain::{ImageFormat, SourceFile};

        let thumbnail_size = self.config.thumbnail_size;
        let cache = match &self.thumbnail_cache {
            Some(c) => c.clone(),
            None => {
                tracing::warn!("缩略图缓存未初始化，跳过预加载");
                return;
            }
        };

        // 限制预加载数量：只加载可见区域 + 缓冲行（50 个），
        // 避免为整个目录（可能上千）同时生成缩略图导致线程池饱和。
        const PRELOAD_LIMIT: usize = 50;
        let generation = self.scan_generation;
        let count = self.display_order.len().min(PRELOAD_LIMIT);

        for di in 0..count {
            let capture_idx = match self.display_order.get(di) {
                Some(&ci) => ci,
                None => continue,
            };
            // 防重：与网格懒加载共享哨兵（preload 在普通池、懒加载在 fast 池，
            // 无哨兵时同一文件可能被两个池重复生成）
            if self.thumbnail_data.contains_key(&capture_idx) { continue; }
            if !self.grid_loading.insert(capture_idx) { continue; }

            let primary_path = std::path::PathBuf::from(&self.captures[capture_idx].primary_path);
            let ext = primary_path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_string();
            let format = ImageFormat::from_extension(&ext)
                .unwrap_or(ImageFormat::Jpeg);

            let source = SourceFile {
                path: primary_path,
                format,
                file_size: self.captures[capture_idx].file_size,
            };

            let cache_clone = cache.clone();
            let ci = capture_idx;
            let path_display = source.path.clone();
            self.worker.spawn(cx, move || {
                // 2x 生成：高 DPI 下 1x 缩略图拉伸会模糊
                cache_clone
                    .get_or_generate(&source, thumbnail_size * 2, None)
                    .map_err(|e| {
                        tracing::warn!("缩略图生成失败 {}: {e}", path_display.display());
                    })
                    .ok()
            }, move |this, result, _cx| {
                if generation != this.scan_generation {
                    return;
                }
                this.grid_loading.remove(&ci);
                if let Some(bytes) = result {
                    this.thumbnail_data
                        .insert(ci, Arc::new(Image::from_bytes(gpui::ImageFormat::Jpeg, bytes)));
                    _cx.notify();
                }
            });
        }
}
}
