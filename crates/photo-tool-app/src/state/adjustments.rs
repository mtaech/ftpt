//! 参数化调整（ADR 0007）：per-capture 参数加载/持久化、调整渲染（16-bit 显示源 + tone 重算）、
//! 单张导出。数据流：
//!
//! ```text
//! focus 变化 → refresh_adjustments_sync（同步点查参数，失效旧渲染/显示源）
//! 渲染路径 → ensure_adjust_ready（构建显示源：RAW 1600px 16-bit 派生 / JPEG 1600px 8-bit）
//! slider   → set_adjustment（持久化 + 重算 tone，只认最新参数版本）
//! 导出     → export_adjusted（rfd 选目录 → worker 烘焙，状态栏提示）
//! ```
//!
//! 性能约束（第一优先级）：tone 重算在 1600px 显示源上做（<8ms/帧，实时拖动），
//! 参数变更绝不触发重新解码——显示源只解码/派生一次，之后只重算像素变换。

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use gpui::*;
use photo_domain::{AdjustParams, ImageFormat as DomainFormat, SourceFile};
use photo_engine::adjustments::{apply_tone16, apply_tone8, Rgb16Image, ToneParams};
use photo_engine::convert::export_adjusted;

use super::app::RootView;

/// 调整显示源长边像素（与预览一致：tone 在此分辨率重算，保证实时）
const ADJUST_DISPLAY_SIZE: u32 = super::app::RootView::PREVIEW_LOAD_SIZE;

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
        let params = self
            .folder_db
            .as_ref()
            .and_then(|db| db.get_adjustments(&self.rel_path_of(&meta.primary_path)).ok())
            .flatten()
            .unwrap_or_default();
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
                    // 16-bit 母版（half_size）→ 缩到显示尺寸（16-bit 保持精度）
                    let master = photo_engine::thumbnail::decode_raw_preview16(&path, Some(&cancel))
                        .ok()?;
                    let (w, h) = master.dimensions();
                    let scale = (ADJUST_DISPLAY_SIZE as f32 / w.max(h) as f32).min(1.0);
                    if scale < 1.0 {
                        let nw = (w as f32 * scale).round().max(1.0) as u32;
                        let nh = (h as f32 * scale).round().max(1.0) as u32;
                        Some(AdjustSource::Rgb16(Arc::new(image::imageops::resize(
                            &master,
                            nw,
                            nh,
                            image::imageops::FilterType::Triangle,
                        ))))
                    } else {
                        Some(AdjustSource::Rgb16(Arc::new(master)))
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

    /// slider/重置/裁切变化：更新参数 → 持久化（异步）→ 重算 tone。
    /// 连续拖动时每帧调用，参数相等则跳过。
    pub fn set_adjustment(&mut self, params: AdjustParams, cx: &mut Context<Self>) {
        if self.current_adjust == params {
            return;
        }
        self.current_adjust = params;
        // 持久化（批量池异步，单条 UPSERT 廉价；拖动中不阻塞 UI）
        let Some(meta) = self.get_focused_capture().map(|m| m.primary_path.clone()) else {
            return;
        };
        let rel = self.rel_path_of(&meta);
        let db = self.folder_db.clone();
        let params_db = params;
        self.worker.spawn(
            cx,
            move || -> Result<(), photo_engine::folder_db::FolderDbError> {
                let Some(db) = db else { return Ok(()) };
                db.put_adjustments(&rel, &params_db)
            },
            |_, _: Result<(), _>, _| {},
        );
        self.recompute_adjust_render(cx);
    }

    /// 按当前参数重算调整渲染（tone 在显示源上，1600px <8ms；只认最新版本）。
    /// 显示源未就绪时静默返回（ensure_adjust_ready 构建完成后触发重算）。
    fn recompute_adjust_render(&mut self, cx: &mut Context<Self>) {
        if self.current_adjust.is_neutral() {
            self.adjust_render = None;
            cx.notify();
            return;
        }
        let display16 = self.adjust_display16.clone();
        let display8 = self.adjust_display8.clone();
        if display16.is_none() && display8.is_none() {
            return; // 源构建中/未构建
        }
        if self.adjust_render_loading {
            return; // 在飞任务会按版本丢弃旧结果
        }
        self.adjust_render_loading = true;
        // 新任务配新取消令牌（旧任务被取消，不占用 fast 池）
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
                // 预览显示全图（裁切框由叠加层绘制，导出时才真正裁切）
                if let Some(d16) = &display16 {
                    Some(rgb16_to_render_image(&apply_tone16(d16, &tone)))
                } else if let Some(d8) = &display8 {
                    Some(rgb8_to_render_image(&apply_tone8(d8, &tone)))
                } else {
                    None
                }
            },
            move |this, result, cx| {
                this.adjust_render_loading = false;
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

    // ── 裁切交互（ADR 0007：调整视图内框选/移动/手柄调整；拖动中只更新 draft，mouse_up 才提交）──
    // 性能约束：事件处理只更新状态（crop_draft 等），不触发 set_adjustment；
    // 提交（含持久化 + 重算 tone）统一在 mouse_up，避免拖动中反复持久化/重算。

    /// 调整视图鼠标按下：Shift → 开始框选裁切；非 Shift → 命中检测
    /// （手柄 8px 邻域 → crop_resize；框内含 6px 边缘 → crop_move；未命中 → 由调用方回退平移）。
    pub(crate) fn adjust_mouse_down(&mut self, x: f32, y: f32, shift: bool, cx: &mut Context<Self>) {
        if shift {
            // 框选新裁切（替换旧框；过小取消时保留原框）
            self.crop_draw = Some((x, y, x, y));
            self.crop_draft = None;
            cx.notify();
            return;
        }
        // 命中检测基于当前显示框（draft 优先，与叠加层一致）
        let Some(bbox) = self.crop_draft.or(self.current_adjust.crop) else {
            return; // 无框 → 平移
        };
        let (Some((nx, ny)), Some((dw, dh))) = (self.window_pos_to_image_norm(x, y), self.preview_disp_size()) else {
            return;
        };
        if dw <= 0. || dh <= 0. {
            return;
        }
        // 1) 手柄命中（8px 邻域，半宽 4px；与叠加层 8px 手柄一致，索引约定见 crop_resize 字段）
        const HANDLE_HIT: f32 = 4.0;
        let (x1, y1, x2, y2) = (bbox.x1, bbox.y1, bbox.x2, bbox.y2);
        let mid_x = (x1 + x2) / 2.;
        let mid_y = (y1 + y2) / 2.;
        let centers = [
            (x1, y1), (mid_x, y1), (x2, y1), (x2, mid_y),
            (x2, y2), (mid_x, y2), (x1, y2), (x1, mid_y),
        ];
        for (idx, (hx, hy)) in centers.iter().enumerate() {
            if (nx - hx).abs() * dw <= HANDLE_HIT && (ny - hy).abs() * dh <= HANDLE_HIT {
                self.crop_resize = Some((idx, bbox));
                cx.notify();
                return;
            }
        }
        // 2) 框内（含 6px 边缘）→ 移动
        let mx = 6. / dw;
        let my = 6. / dh;
        if nx >= x1 - mx && nx <= x2 + mx && ny >= y1 - my && ny <= y2 + my {
            self.crop_move = Some((x, y, bbox));
            cx.notify();
        }
        // 3) 未命中 → 调用方回退平移
    }

    /// 调整视图鼠标移动：框选/手柄/移动各自更新 crop_draft（不动 current_adjust）。
    pub(crate) fn adjust_mouse_move(&mut self, x: f32, y: f32, cx: &mut Context<Self>) {
        // 框选中：更新角点并实时换算归一化 draft
        if let Some((sx, sy, _, _)) = self.crop_draw {
            self.crop_draw = Some((sx, sy, x, y));
            if let (Some((ax1, ay1)), Some((ax2, ay2))) = (
                self.window_pos_to_image_norm(sx, sy),
                self.window_pos_to_image_norm(x, y),
            ) {
                // 反向拖拽归一化；出界由 BBox::new 钳制到 [0,1]（与 submit_box_draw 同模式）
                self.crop_draft = Some(photo_domain::BBox::new(
                    ax1.min(ax2), ay1.min(ay2), ax1.max(ax2), ay1.max(ay2),
                ));
            }
            cx.notify();
            return;
        }
        // 手柄调整：对边不动，最小 5% 尺寸，夹紧 0-1
        if let Some((idx, orig)) = self.crop_resize {
            if let Some((nx, ny)) = self.window_pos_to_image_norm(x, y) {
                self.crop_draft = Some(Self::resize_crop(idx, orig, nx, ny));
            }
            cx.notify();
            return;
        }
        // 平移框：按归一化位移移动，保持尺寸，夹紧 0-1
        if let Some((sx, sy, orig)) = self.crop_move {
            if let (Some((snx, sny)), Some((nx, ny))) = (
                self.window_pos_to_image_norm(sx, sy),
                self.window_pos_to_image_norm(x, y),
            ) {
                let w = orig.x2 - orig.x1;
                let h = orig.y2 - orig.y1;
                let x1 = (orig.x1 + (nx - snx)).clamp(0., 1. - w);
                let y1 = (orig.y1 + (ny - sny)).clamp(0., 1. - h);
                self.crop_draft = Some(photo_domain::BBox::new(x1, y1, x1 + w, y1 + h));
            }
            cx.notify();
            return;
        }
        // 无裁切交互 → 平移（沿用 preview_drag 语义）
        if let Some((sx, sy, spx, spy)) = self.preview_drag {
            self.preview_pan = (spx + (x - sx), spy + (y - sy));
            self.clamp_preview_pan();
            cx.notify();
        }
    }

    /// 按手柄索引更新裁切框：对边不动，最小 5% 尺寸，夹紧 0-1。
    fn resize_crop(idx: usize, orig: photo_domain::BBox, nx: f32, ny: f32) -> photo_domain::BBox {
        const MIN: f32 = 0.05;
        let (mut x1, mut y1, mut x2, mut y2) = (orig.x1, orig.y1, orig.x2, orig.y2);
        match idx {
            0 => { x1 = nx.min(x2 - MIN); y1 = ny.min(y2 - MIN); } // 左上
            1 => { y1 = ny.min(y2 - MIN); }                        // 上中
            2 => { x2 = nx.max(x1 + MIN); y1 = ny.min(y2 - MIN); } // 右上
            3 => { x2 = nx.max(x1 + MIN); }                        // 右中
            4 => { x2 = nx.max(x1 + MIN); y2 = ny.max(y1 + MIN); } // 右下
            5 => { y2 = ny.max(y1 + MIN); }                        // 下中
            6 => { x1 = nx.min(x2 - MIN); y2 = ny.max(y1 + MIN); } // 左下
            7 => { x1 = nx.min(x2 - MIN); }                        // 左中
            _ => {}
        }
        photo_domain::BBox::new(x1, y1, x2, y2)
    }

    /// 调整视图鼠标抬起：框选/移动/手柄结束 → set_adjustment 提交（框选过小取消），清空交互状态。
    pub(crate) fn adjust_mouse_up(&mut self, cx: &mut Context<Self>) {
        if self.crop_draw.is_some() {
            // 框选结束：窗口坐标 → 归一化 BBox；任一方向 < 5% 视为取消（保持原 crop）
            let committed = self.crop_draw.take().and_then(|(x1, y1, x2, y2)| {
                let (Some((ax1, ay1)), Some((ax2, ay2))) = (
                    self.window_pos_to_image_norm(x1, y1),
                    self.window_pos_to_image_norm(x2, y2),
                ) else {
                    return None;
                };
                let bbox = photo_domain::BBox::new(
                    ax1.min(ax2), ay1.min(ay2), ax1.max(ax2), ay1.max(ay2),
                );
                (bbox.x2 - bbox.x1 >= 0.05 && bbox.y2 - bbox.y1 >= 0.05).then_some(bbox)
            });
            self.crop_draft = None;
            if let Some(bbox) = committed {
                let mut params = self.current_adjust;
                params.crop = Some(bbox);
                self.set_adjustment(params, cx);
            }
            cx.notify();
            return;
        }
        if self.crop_move.is_some() || self.crop_resize.is_some() {
            let draft = self.crop_draft.take();
            self.crop_move = None;
            self.crop_resize = None;
            if let Some(bbox) = draft {
                let mut params = self.current_adjust;
                params.crop = Some(bbox);
                self.set_adjustment(params, cx);
            }
            cx.notify();
        }
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
