//! 参数化调整（ADR 0007）：per-capture 参数加载/持久化、调整渲染（16-bit 显示源 + tone 重算）、
//! 单张导出。数据流：
//!
//! ```text
//! focus 变化 → refresh_adjustments_sync（同步点查参数，失效旧渲染/显示源）
//! 渲染路径 → ensure_adjust_ready（构建显示源：RAW 1600px 16-bit 派生 / JPEG 1600px 8-bit）
//! slider   → set_adjustment_live（内存更新 + 重算，持久化 350ms 去抖一次）
//! 重置     → set_adjustment（立即持久化 + 重算 tone，只认最新参数版本）
//! 导出     → export_adjusted（rfd 选目录 → worker 烘焙，状态栏提示）
//! ```
//!
//! 性能约束（第一优先级）：tone 重算在 1600px 显示源上做（<8ms/帧，实时拖动），
//! 参数变更绝不触发重新解码——显示源只解码/派生一次，之后只重算像素变换。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use gpui::*;
use photo_domain::{AdjustParams, ImageFormat as DomainFormat, SourceFile};
use photo_engine::adjustments::{apply_tone16, apply_tone8, Rgb16Image, ToneParams};
use photo_engine::convert::export_adjusted;

use super::app::RootView;

/// 调整显示源长边像素（与预览一致：tone 在此分辨率重算，保证实时）
const ADJUST_DISPLAY_SIZE: u32 = super::app::RootView::PREVIEW_LOAD_SIZE;

/// RAW 16-bit 调整母版 LRU 缓存（容量 2，键 = primary_path）。
/// 每次切图构建显示源都需解码母版（200-500ms/张），切走再切回应命中缓存跳过解码；
/// 显示源（adjust_display16）随焦点淘汰，母版保留在此缓存。应用仅一个 RootView 实例，
/// 用全局静态承载（构建任务在 worker 线程执行，缓存需跨任务共享）。
static RAW16_MASTER_CACHE: Mutex<Vec<(PathBuf, Arc<Rgb16Image>)>> = Mutex::new(Vec::new());
/// 母版缓存上限（张）：16-bit half_size 母版 ≈ 40MB/张，保留最近 2 张
const RAW16_CACHE_LIMIT: usize = 2;

/// 取 RAW 16-bit 母版：LRU 命中直接返回，未命中解码后入缓存并淘汰最旧。
/// 解码被取消（切图令牌）时不入缓存。返回的 Arc 与缓存共享同一份不可变数据。
fn raw16_master_cached(path: &Path, cancel: Option<&AtomicBool>) -> Option<Arc<Rgb16Image>> {
    {
        let mut cache = RAW16_MASTER_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(pos) = cache.iter().position(|(p, _)| p == path) {
            let (_, img) = cache.remove(pos);
            cache.insert(0, (path.to_path_buf(), img.clone()));
            return Some(img);
        }
    }
    let master = Arc::new(photo_engine::thumbnail::decode_raw_preview16(path, cancel).ok()?);
    let mut cache = RAW16_MASTER_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache.insert(0, (path.to_path_buf(), master.clone()));
    cache.truncate(RAW16_CACHE_LIMIT);
    Some(master)
}

/// 调整显示源（构建任务返回类型：RAW 为 16-bit 派生，JPEG 为 8-bit）
enum AdjustSource {
    Rgb16(Arc<Rgb16Image>),
    Rgb8(Arc<image::RgbImage>),
}

impl RootView {
    /// 焦点变化时同步刷新调整状态（3 个焦点入口调用，无 cx）：
    /// - 焦点 capture 变化 → folder_db 点查参数、失效旧调整渲染与显示源
    /// - 焦点 capture 未变 → 保留内存参数（可能是 slider 未持久化值），不覆盖
    pub fn refresh_adjustments_sync(&mut self) {
        let Some(meta) = self.get_focused_capture() else {
            self.current_adjust = AdjustParams::default();
            self.adjust_params_capture = None;
            self.adjust_render = None;
            self.adjust_source_capture = None;
            self.adjust_display16 = None;
            self.adjust_display8 = None;
            return;
        };
        let Some(&ci) = self
            .focus_index
            .and_then(|di| self.display_order.get(di))
        else {
            return;
        };
        if self.adjust_params_capture == Some(ci) {
            // 参数已在内存（可能含未持久化的 slider 值）
            return;
        }
        let mut params = self
            .folder_db
            .as_ref()
            .and_then(|db| db.get_adjustments(&self.rel_path_of(&meta.primary_path)).ok())
            .flatten()
            .unwrap_or_default();
        // 防御 DB 坏值：Q15 定点饱和要求 saturation∈[-100,100]，钳制后再入内存
        // （UI 显示与后续持久化均用钳制值，引擎侧 From 转换另有兜底钳制）
        params.exposure = params.exposure.clamp(-2.0, 2.0);
        params.contrast = params.contrast.clamp(-100, 100);
        params.saturation = params.saturation.clamp(-100, 100);
        self.current_adjust = params;
        self.adjust_params_capture = Some(ci);
        // 焦点图变化：调整渲染与显示源全部失效（由渲染路径 ensure_adjust_ready 重建）
        // 旧在飞任务由换代/取消令牌丢弃
        self.adjust_render_cancel.store(true, Ordering::Relaxed);
        self.adjust_render = None;
        self.adjust_source_capture = None;
        self.adjust_display16 = None;
        self.adjust_display8 = None;
    }

    /// 渲染路径调用（preview.rs render 时）：确保焦点图调整显示源就绪。
    /// 参数中性 → 清理调整状态（回退现有预览路径，零回归）；
    /// 非中性 → 显示源未构建则异步构建（RAW 16-bit 母版→1600px 派生 / JPEG 缓存字节→8-bit）。
    pub fn ensure_adjust_ready(&mut self, cx: &mut Context<Self>) {
        if self.current_adjust.is_neutral() {
            self.adjust_render = None;
            self.adjust_source_capture = None;
            return;
        }
        let Some(meta) = self.get_focused_capture() else { return };
        let Some(&ci) = self
            .focus_index
            .and_then(|di| self.display_order.get(di))
        else {
            return;
        };
        // 显示源已就绪且归属当前图
        let source_ready = self.adjust_source_capture == Some(ci)
            && (self.adjust_display16.is_some() || self.adjust_display8.is_some());
        if source_ready {
            return;
        }
        if self.adjust_source_loading {
            return; // 构建中（on_done 会触发重算）
        }
        let generation = self.scan_generation;
        let path = PathBuf::from(&meta.primary_path);
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let is_raw = matches!(DomainFormat::from_extension(&ext), Some(DomainFormat::Raw(_)));
        let source = SourceFile {
            path: path.clone(),
            format: DomainFormat::from_extension(&ext).unwrap_or(DomainFormat::Jpeg),
            file_size: meta.file_size,
        };
        let cache = self.thumbnail_cache.clone();
        // 新构建任务配新令牌：切图时旧令牌被置 true（取消旧任务），新构建不受影响
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.adjust_render_cancel = cancel.clone();
        self.adjust_source_loading = true;

        self.worker.spawn_fast(
            cx,
            move || -> Option<AdjustSource> {
                if cancel.load(Ordering::Relaxed) {
                    return None;
                }
                if is_raw {
                    // 16-bit 母版（half_size，LRU 缓存命中免重解码）→ 缩到显示尺寸（16-bit 保持精度）
                    let master = raw16_master_cached(&path, Some(&cancel))?;
                    let (w, h) = master.dimensions();
                    let scale = (ADJUST_DISPLAY_SIZE as f32 / w.max(h) as f32).min(1.0);
                    if scale < 1.0 {
                        let nw = (w as f32 * scale).round().max(1.0) as u32;
                        let nh = (h as f32 * scale).round().max(1.0) as u32;
                        Some(AdjustSource::Rgb16(Arc::new(image::imageops::resize(
                            master.as_ref(),
                            nw,
                            nh,
                            image::imageops::FilterType::Triangle,
                        ))))
                    } else {
                        Some(AdjustSource::Rgb16(master))
                    }
                } else {
                    // JPEG/常规图：磁盘缓存字节（与预览同源）→ 8-bit RGB
                    let bytes = cache
                        .as_ref()?
                        .get_or_generate(&source, ADJUST_DISPLAY_SIZE, Some(&cancel))
                        .ok()?;
                    let img = image::load_from_memory(&bytes).ok()?;
                    Some(AdjustSource::Rgb8(Arc::new(img.to_rgb8())))
                }
            },
            move |this, result, cx| {
                this.adjust_source_loading = false;
                if generation != this.scan_generation {
                    return;
                }
                // 已切走（焦点图变化）：丢弃，避免错绑显示源
                let focused = this
                    .focus_index
                    .and_then(|di| this.display_order.get(di))
                    .copied();
                if focused != Some(ci) {
                    return;
                }
                if let Some(display) = result {
                    this.adjust_source_capture = Some(ci);
                    match display {
                        AdjustSource::Rgb16(img) => this.adjust_display16 = Some(img),
                        AdjustSource::Rgb8(img) => this.adjust_display8 = Some(img),
                    }
                    // 源就绪 → 立即重算首帧
                    this.recompute_adjust_render(cx);
                }
            },
        );
    }

    /// 离散变更（重置按钮/重置全部）：更新参数 → 立即持久化（异步）→ 重算 tone。
    /// slider 拖动请用 set_adjustment_live（去抖持久化，见下）。
    /// **必须无条件 notify**：显示源未就绪时 recompute 提前返回（不重算），
    /// 但参数已更新——不 notify 则画面/数值文本不刷新，拖动观感为"无反应"。
    pub fn set_adjustment(&mut self, params: AdjustParams, cx: &mut Context<Self>) {
        if self.current_adjust == params {
            return;
        }
        self.current_adjust = params;
        cx.notify();
        self.persist_adjust_now(cx);
        self.recompute_adjust_render(cx);
    }

    /// slider 拖动中的实时更新（每帧调用）：只更新内存参数 + 重算显示，
    /// **不逐帧 UPSERT**——持久化去抖：每次变化重排 350ms 定时器，停止拖动后无新变化才写一次最终值。
    /// 过期定时器触发时校验参数/焦点未变才写（避免旧值覆盖新值、防切图串行）。
    pub fn set_adjustment_live(&mut self, params: AdjustParams, cx: &mut Context<Self>) {
        if self.current_adjust == params {
            return;
        }
        self.current_adjust = params;
        cx.notify();
        // 去抖持久化：捕获参数快照与参数归属 capture（防切图串写）；
        // 触发时快照仍为当前值/当前图才写入（写入目标 rel 由 persist_adjust_now 现算）
        if self.get_focused_capture().is_some() {
            let params_snapshot = params;
            let params_ci = self.adjust_params_capture;
            let vh = cx.entity().downgrade();
            cx.spawn(move |_: WeakEntity<RootView>, cx: &mut AsyncApp| {
                let mut cx = cx.clone();
                async move {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(350))
                        .await;
                    let Some(view) = vh.upgrade() else { return };
                    let _ = cx.update_entity(&view, move |this, cx| {
                        // 期间参数又变（更新的定时器负责写最终值）或焦点已切走 → 跳过
                        if this.current_adjust != params_snapshot
                            || this.adjust_params_capture != params_ci
                        {
                            return;
                        }
                        this.persist_adjust_now(cx);
                    });
                }
            })
            .detach();
        }
        self.recompute_adjust_render(cx);
    }

    /// 将当前参数持久化到当前焦点图（批量池异步执行；失败仅记日志，不打断交互）。
    fn persist_adjust_now(&mut self, cx: &mut Context<Self>) {
        let Some(meta) = self.get_focused_capture().map(|m| m.primary_path.clone()) else {
            return;
        };
        let rel = self.rel_path_of(&meta);
        let rel_log = rel.clone();
        let db = self.folder_db.clone();
        let params = self.current_adjust;
        self.worker.spawn(
            cx,
            move || -> bool {
                let Some(db) = db else { return true };
                match db.put_adjustments(&rel, &params) {
                    Ok(()) => true,
                    Err(e) => {
                        tracing::error!("调整参数持久化失败 {rel_log}: {e}");
                        false
                    }
                }
            },
            move |_, _persisted: bool, _| {},
        );
    }

    /// 按当前参数重算调整渲染（tone 在显示源上，1600px <8ms；只认最新版本）。
    /// 显示源未就绪时静默返回（ensure_adjust_ready 构建完成后触发重算）。
    /// **每次调用都取消旧任务 + 新令牌重算**：slider 快速拖动时旧任务在 worker 闭包
    /// 开头检查令牌快速退出（不堆积），版本号保证只认最新——不设 loading 闸门，
    /// 否则拖动中/Release 的最终值会因旧任务未完成被跳过。
    /// pub(crate)：拖动结束的最终值由最后一次 set_adjustment_live 的重算渲染（无独立 Release 事件）。
    pub(crate) fn recompute_adjust_render(&mut self, cx: &mut Context<Self>) {
        if self.current_adjust.is_neutral() {
            self.adjust_render = None;
            cx.notify();
            return;
        }
        let display16 = self.adjust_display16.clone();
        let display8 = self.adjust_display8.clone();
        if display16.is_none() && display8.is_none() {
            return;
        }
        // 取消旧任务 + 新令牌（旧任务在 worker 闭包开头检查，快速退出）
        self.adjust_render_cancel.store(true, Ordering::Relaxed);
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.adjust_render_cancel = cancel.clone();
        self.adjust_render_version += 1;
        let version = self.adjust_render_version;
        let tone: ToneParams = (&self.current_adjust).into();
        let generation = self.scan_generation;
        self.worker.spawn_fast(
            cx,
            move || -> Option<RenderImage> {
                if cancel.load(Ordering::Relaxed) {
                    return None;
                }
                if let Some(d16) = &display16 {
                    Some(rgb16_to_render_image(&apply_tone16(d16, &tone)))
                } else if let Some(d8) = &display8 {
                    Some(rgb8_to_render_image(&apply_tone8(d8, &tone)))
                } else {
                    None
                }
            },
            move |this, result, cx| {
                if generation != this.scan_generation {
                    return;
                }
                if version != this.adjust_render_version {
                    return; // 过期任务（参数已再次变化）
                }
                this.adjust_render = result.map(Arc::new);
                cx.notify();
            },
        );
    }

    /// 导出当前图调整结果（rfd 选目录 → worker 烘焙全尺寸 → 状态栏消息）。
    /// 命名 `{stem}_adjusted.jpg`，已存在自动追加序号（不覆盖原文件）。
    pub fn export_adjusted(&mut self, cx: &mut Context<Self>) {
        if self.adjust_exporting {
            return;
        }
        let Some(meta) = self.get_focused_capture() else { return };
        let params = self.current_adjust;
        let source = SourceFile {
            path: PathBuf::from(&meta.primary_path),
            format: DomainFormat::from_extension(
                std::path::Path::new(&meta.primary_path)
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or(""),
            )
            .unwrap_or(DomainFormat::Jpeg),
            file_size: meta.file_size,
        };
        let stem = std::path::Path::new(&meta.primary_path)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "photo".to_string());
        self.adjust_exporting = true;
        self.adjust_export_msg = None;
        self.worker.spawn(
            cx,
            move || -> Option<Result<String, String>> {
                // rfd 目录选择（与扫描 pick_folder 同模式：worker 线程打开对话框）
                let dir = rfd::FileDialog::new()
                    .set_title("导出调整结果")
                    .pick_folder()?;
                let mut out = dir.join(format!("{stem}_adjusted.jpg"));
                let mut n = 1;
                while out.exists() {
                    out = dir.join(format!("{stem}_adjusted_{n}.jpg"));
                    n += 1;
                }
                Some(
                    export_adjusted(&source, &params, &out)
                        .map(|p| p.to_string_lossy().to_string())
                        .map_err(|e| e.to_string()),
                )
            },
            move |this, result, cx| {
                this.adjust_exporting = false;
                this.adjust_export_msg = match result {
                    Some(Ok(path)) => Some(format!("已导出: {path}")),
                    Some(Err(e)) => Some(format!("导出失败: {e}")),
                    None => None, // 用户取消目录选择
                };
                cx.notify();
            },
        );
    }

    /// 主显示文件相对文件夹根的正斜杠路径（folder_db 调整表键，与识别表同规则）
    fn rel_path_of(&self, path: &str) -> String {
        let Some(ref dir) = self.dir_path else {
            return path.replace('\\', "/");
        };
        std::path::Path::new(path)
            .strip_prefix(dir)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| path.replace('\\', "/"))
    }
}

/// 16-bit RGB → BGRA RenderImage（GPUI 帧要求 BGRA 通道序）
fn rgb16_to_render_image(img: &photo_engine::adjustments::Rgb16Image) -> RenderImage {
    let mut rgba = image::RgbaImage::new(img.width(), img.height());
    for (p16, p8) in img.pixels().zip(rgba.pixels_mut()) {
        p8[0] = (p16[2] >> 8) as u8; // B
        p8[1] = (p16[1] >> 8) as u8; // G
        p8[2] = (p16[0] >> 8) as u8; // R
        p8[3] = 255;
    }
    RenderImage::new(vec![image::Frame::new(rgba)])
}

/// 8-bit RGB → BGRA RenderImage
fn rgb8_to_render_image(img: &image::RgbImage) -> RenderImage {
    let mut rgba = image::RgbaImage::new(img.width(), img.height());
    for (p8, p4) in img.pixels().zip(rgba.pixels_mut()) {
        p4[0] = p8[2]; // B
        p4[1] = p8[1]; // G
        p4[2] = p8[0]; // R
        p4[3] = 255;
    }
    RenderImage::new(vec![image::Frame::new(rgba)])
}
