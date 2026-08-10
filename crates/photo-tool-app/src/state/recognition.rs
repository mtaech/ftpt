use std::collections::HashMap;
use std::sync::Arc;

use gpui::*;
use photo_domain::{BBox, CaptureMeta};
use photo_recognize::Recognizer;

use super::app::RootView;
use crate::state::preview_math::window_pos_to_image_norm;

// 鸟种识别（单张/框选/批量）与识别结果回填（自 state/app.rs 拆出，纯移动，无逻辑改动）

/// 批量识别单张结果：(capture_index, 相对路径, 识别结果)
type BatchResult = (usize, String, Result<photo_domain::Recognition, String>);

/// 批量识别共享进度：工作线程逐张写入，UI 侧 200ms 轮询读取（与 sync_progress 同一模式）。
/// 640 张级别的批量任务中途无任何上报时，UI 与日志会静止数十分钟，表现为“卡死”。
#[derive(Default)]
struct BatchProgress {
    done: std::sync::atomic::AtomicUsize,
    confirmed: std::sync::atomic::AtomicUsize,
    unrecognized: std::sync::atomic::AtomicUsize,
    needs_review: std::sync::atomic::AtomicUsize,
    /// 批量识别当前处理的文件名（多线程时为最后开始识别的文件）
    current: parking_lot::Mutex<String>,
    /// 已完成结果队列：工作线程逐张推入，UI 轮询 drain 后即时刻到网格，
    /// 不再等整批结束才更新
    results: parking_lot::Mutex<Vec<BatchResult>>,
}

impl RootView {
    /// 构建识别用 Recognizer（模型目录 = exe 同级 models/ + data/pica_ref.db）。
    /// DirectML 初始化 ~2-5s，属一次性成本：单张路径缓存复用，批量路径每线程建一份。
    fn build_recognizer() -> Result<photo_recognize::Recognizer, String> {
        let exe_dir = std::env::current_exe()
            .map_err(|e| format!("获取 exe 路径失败: {e}"))?
            .parent()
            .ok_or_else(|| "无法确定 exe 目录".to_string())?
            .to_path_buf();
        let models_dir = exe_dir.join("models");
        let catalog_db = exe_dir.join("data").join("pica_ref.db");
        photo_recognize::Recognizer::new(&models_dir, &catalog_db)
            .map_err(|e| format!("加载识别模型失败: {e}"))
    }

    /// 后台预热单张识别 Recognizer：应用启动即加载模型（DirectML 初始化 ~2-5s），
    /// 首次点击「识别此照片」不再等待模型加载。
    pub(crate) fn warmup_single_recognizer(&mut self, cx: &mut Context<Self>) {
        if self.single_recognizer.is_some() {
            return;
        }
        self.worker.spawn(
            cx,
            // 包成 Option：满足 worker.spawn 的 R: Default 约束（Recognizer 无 Default 实现），
            // panic 兜底值为 None，走下方「预热失败」分支，不影响正常路径语义
            || Self::build_recognizer().ok(),
            move |this, result, cx| {
                if let Some(rec) = result {
                    this.single_recognizer = Some(rec);
                    tracing::info!("单张识别 Recognizer 预热完成");
                } else {
                    tracing::warn!("单张识别 Recognizer 预热失败（首次识别时再试）");
                }
                cx.notify();
            },
        );
    }

    /// 从 folder_db 刷新焦点图片的识别记录
    pub(crate) fn refresh_focused_recognition(&mut self) {
        self.focused_recognition = None;
        let Some(meta) = self.get_focused_capture() else { return };
        let Some(ref db) = self.folder_db else { return };
        let Some(ref dir) = self.dir_path else { return };
        let primary = std::path::Path::new(&meta.primary_path);
        let rel = primary.strip_prefix(dir).ok().map(|p| p.to_string_lossy().replace('\\', "/"));
        if let Some(rel_str) = rel {
            if let Ok(Some(rec)) = db.get_recognition(&rel_str) {
                self.focused_recognition = Some(rec);
            }
        }
    }

    /// 懒加载全量鸟种名录（手动修正下拉数据源，首次展开修正时调用）。
    /// 名录库在 exe 同级 data/pica_ref.db（与 Recognizer 同路径约定）。
    pub(crate) fn ensure_correction_species(&mut self, cx: &mut Context<Self>) {
        if !self.correction_birds.is_empty() {
            return;
        }
        let Ok(exe_dir) = std::env::current_exe() else { return };
        let Some(exe_dir) = exe_dir.parent() else { return };
        let catalog_db = exe_dir.join("data").join("pica_ref.db");
        match photo_recognize::list_all_species(&catalog_db) {
            Ok(list) => {
                let mut names: Vec<String> = list.iter().map(|b| b.cn_name.clone()).collect();
                names.sort();
                self.correction_birds = list;
                self.correction_options = names;
                self.correction_select_dirty = true;
            }
            Err(e) => {
                tracing::warn!("名录加载失败（修正功能不可用）: {e}");
                self.show_toast(format!("加载名录失败: {e}"), cx);
            }
        }
    }

    /// 重建手动修正鸟种下拉（创建需要 Window，由 info_panel render 按 dirty 标记驱动）。
    pub fn rebuild_correction_select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.correction_options.is_empty() {
            return;
        }
        let options = self.correction_options.clone();
        let entity = cx.new(|cx| {
            gpui_component::combobox::ComboboxState::new(
                gpui_component::select::SearchableVec::new(options),
                vec![],
                window,
                cx,
            )
            .multiple(false)
            .searchable(true)
        });
        cx.subscribe(&entity, |view, _state, event, cx| {
            if let gpui_component::combobox::ComboboxEvent::Change(values) = event {
                if let Some(name) = values.first().cloned() {
                    view.correct_bird_by_name(&name, cx);
                }
            }
        })
        .detach();
        self.correction_select = Some(entity);
        self.correction_select_dirty = false;
    }

    /// 手动修正鸟种：按中文名查名录 → 构造 Confirmed Recognition（保留原框/眼数据）→ 持久化。
    /// 人工指定即为权威结论：状态 Confirmed、置信度 100%（区别于模型置信度）。
    pub(crate) fn correct_bird_by_name(&mut self, name: &str, cx: &mut Context<Self>) {
        let Some(bird) = self
            .correction_birds
            .iter()
            .find(|b| b.cn_name == name)
            .cloned()
        else {
            return;
        };
        let Some(meta) = self.get_focused_capture() else { return };
        let capture_index = meta.index;
        let (Some(db), Some(dir)) = (self.folder_db.as_ref(), self.dir_path.as_ref()) else {
            return;
        };
        let primary = std::path::Path::new(&meta.primary_path);
        let rel = match primary.strip_prefix(dir) {
            Ok(p) => p.to_string_lossy().replace('\\', "/"),
            Err(_) => return,
        };

        // 保留原识别框/眼锐度，只改鸟种与状态
        let prev = self.focused_recognition.clone();
        let rec = photo_domain::Recognition {
            status: photo_domain::RecognitionStatus::Confirmed,
            bird: Some(bird.clone()),
            class_index: prev.as_ref().and_then(|r| r.class_index),
            confidence: Some(100.0),
            bbox: prev.as_ref().and_then(|r| r.bbox),
            eye_sharpness: prev.as_ref().and_then(|r| r.eye_sharpness),
            eye_bbox: prev.as_ref().and_then(|r| r.eye_bbox),
            candidates: vec![],
            failure_stage: photo_domain::RecognitionFailureStage::None,
            recognized_at: chrono::Utc::now().to_rfc3339(),
        };
        if let Err(e) = db.upsert_recognition(&rel, &rec) {
            tracing::error!("手动修正写入失败 {}: {e}", rel);
            self.show_toast(format!("修正写入失败: {e}"), cx);
            return;
        }
        if let Some(m) = self.captures.iter_mut().find(|m| m.index == capture_index) {
            m.enrich_with_recognition(&rec);
        }
        self.correction_open = false;
        self.refresh_focused_recognition();
        self.refresh_bird_options();
        self.apply_filter_and_sort();
        cx.notify();
        self.show_toast(format!("已手动标记为 {}", bird.cn_name), cx);
    }

    /// 从 CaptureMeta 构建 Capture（供识别管线使用）
    pub(crate) fn build_capture_from_meta(&self, meta: &CaptureMeta) -> Option<photo_domain::Capture> {
        let primary_path = std::path::Path::new(&meta.primary_path);
        let ext = primary_path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let format = photo_domain::ImageFormat::from_extension(&ext)
            .unwrap_or(photo_domain::ImageFormat::Jpeg);
        let source = photo_domain::SourceFile {
            path: primary_path.to_path_buf(),
            format,

            file_size: meta.file_size,
        };
        Some(photo_domain::Capture {
            base_name: meta.base_name.clone(),
            source_files: vec![source],
            primary_index: 0,
        })
    }

    /// 单张识别（Recognize action）
    pub(crate) fn recognize_single(&mut self, cx: &mut Context<Self>) {
        if self.recognizing_single.is_some() || self.batch_recognizing {
            tracing::info!("识别忽略：已有识别任务进行中");
            return;
        }
        let Some(focus_di) = self.focus_index else {
            tracing::info!("识别忽略：无焦点图片");
            return;
        };
        let Some(&capture_idx) = self.display_order.get(focus_di) else { return };
        let Some(meta) = self.captures.get(capture_idx) else { return };
        let dir = match self.dir_path.clone() { Some(d) => d, None => return };
        if self.folder_db.is_none() { return };

        let primary_path = std::path::PathBuf::from(&meta.primary_path);
        let rel_path = primary_path.strip_prefix(&dir)
            .ok()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        let rel_path_clone = rel_path.clone();

        let Some(capture) = self.build_capture_from_meta(meta) else {
            tracing::error!("构建 Capture 失败: {}", meta.base_name);
            return;
        };

        // 缩略图优先输入：识别前取 .pt/thumbs 派生图字节（JPEG DCT 毫秒级 / RAW
        // 母版派生），worker 闭包内解码，省全图 image::open（24MP 100-300ms/张）
        let thumb_source = capture.source_files[capture.primary_index].clone();
        let cache = self.thumbnail_cache.clone();

        self.recognizing_single = Some(capture_idx);
        self.recognize_stage = Some("检测中".into());
        cx.notify();

        // 复用缓存 Recognizer（预热或上次归还），用完归还；None 则现场构建（构建结果也归还）
        let cached = self.single_recognizer.take();

        self.worker.spawn(
            cx,
            move || {
                // 工作线程侧：优先复用缓存 Recognizer，否则现场构建；构建结果随结果归还
                let mut recognizer = match cached {
                    Some(r) => r,
                    None => match Self::build_recognizer() {
                        Ok(r) => r,
                        Err(e) => {
                            return (Some(Err(e)), rel_path_clone, capture_idx, dir, None);
                        }
                    },
                };
                tracing::info!("单张识别开始，推理后端: {}", recognizer.backend());
                let thumb_bytes = cache
                    .as_ref()
                    .and_then(|c| c.get_or_generate(&thumb_source, 2048, None).ok());
                let res = recognizer
                    .recognize_with_thumbnail(&capture, thumb_bytes.as_deref(), None)
                    .map_err(|e| format!("识别失败: {e}"));
                // 结果包成 Option<Result<Recognition, String>>：满足 worker.spawn 的 R: Default
                // 约束（Result 无 Default，panic 时整个元组兜底为 None），on_done 只复位哨兵、不写库
                (Some(res), rel_path_clone, capture_idx, dir, Some(recognizer))
            },
            move |this, (result, rel, idx, _scan_dir, recognizer), cx| {
                // 归还缓存 Recognizer（下次单张识别直接复用，省 DirectML 初始化）
                if let Some(rec) = recognizer {
                    this.single_recognizer = Some(rec);
                }
                match result {
                    Some(Ok(rec)) => {
                        // upsert 到 FolderDb
                        if let Some(ref db) = this.folder_db {
                            if let Err(e) = db.upsert_recognition(&rel, &rec) {
                                tracing::error!("写入识别结果失败 {}: {e}", rel);
                            }
                        }
                        // enrich CaptureMeta
                        if let Some(meta) = this.captures.iter_mut().find(|m| m.index == idx) {
                            meta.enrich_with_recognition(&rec);
                        }
                        // 刷新 focused_recognition
                        this.refresh_focused_recognition();
                        this.recognizing_single = None;
                        this.recognize_stage = None;
                        this.apply_filter_and_sort();
                        this.refresh_bird_options();
                        cx.notify();

                        // toast
                        match rec.status {
                            photo_domain::RecognitionStatus::Confirmed => {
                                let bird_name = rec.bird.as_ref().map(|b| b.cn_name.as_str()).unwrap_or("未知");
                                let conf = rec.confidence.unwrap_or(0.0);
                                this.show_toast(format!("{} · 置信度 {:.1}%", bird_name, conf), cx);
                            }
                            photo_domain::RecognitionStatus::Unrecognized => {
                                this.show_toast("未检测到鸟类", cx);
                            }
                            photo_domain::RecognitionStatus::NeedsReview => {
                                let reason = rec.failure_stage.user_message();
                                this.show_toast(format!("待复核·{}", reason), cx);
                            }
                        }
                        tracing::info!("单张识别完成: {} 状态={:?}", rel, rec.status);
                    }
                    None => {
                        // worker 闭包 panic 兜底（R::default()）：只复位哨兵，不写库
                        tracing::error!("单张识别任务异常中断（worker panic）");
                        this.recognizing_single = None;
                        this.recognize_stage = None;
                        this.show_toast("识别任务异常中断", cx);
                        cx.notify();
                    }
                    Some(Err(e)) => {
                        tracing::error!("识别错误 {}: {e}", rel);
                        this.recognizing_single = None;
                        this.recognize_stage = None;
                        this.show_toast(format!("识别失败: {e}"), cx);
                        cx.notify();
                    }
                }
            },
        );
    }

    /// 窗口坐标 → 图片归一化坐标（0-1，相对原图），委托给同名纯函数。
    pub(crate) fn window_pos_to_image_norm(&self, wx: f32, wy: f32) -> Option<(f32, f32)> {
        let area = *self.preview_area_bounds.borrow();
        let meta = self.get_focused_capture()?;
        let img = (meta.image_width?, meta.image_height?);
        window_pos_to_image_norm(wx, wy, area, img, self.preview_zoom, self.preview_pan)
    }

    /// Shift+拖拽画框结束：换算为归一化 bbox 并触发手动框选识别。
    ///
    /// 过小的框（任一方向 <8 显示像素，多为误触）直接忽略；识别进行中拒绝并提示。
    pub(crate) fn submit_box_draw(&mut self, cx: &mut Context<Self>) {
        let Some((x1, y1, x2, y2)) = self.box_draw.take() else {
            return;
        };
        cx.notify();
        if (x2 - x1).abs() < 8. || (y2 - y1).abs() < 8. {
            return;
        }
        if self.recognizing_single.is_some() || self.batch_recognizing {
            self.show_toast("识别进行中，稍后再试", cx);
            return;
        }
        let (Some((ax1, ay1)), Some((ax2, ay2))) = (
            self.window_pos_to_image_norm(x1, y1),
            self.window_pos_to_image_norm(x2, y2),
        ) else {
            return;
        };
        // 反向拖拽归一化；出界由 BBox::new 钳制到 [0,1]
        let bbox = BBox::new(ax1.min(ax2), ay1.min(ay2), ax1.max(ax2), ay1.max(ay2));
        self.pending_region = Some(bbox);
        self.recognize_region_single(bbox, cx);
    }

    /// 手动框选区域识别（预览界面 Shift+拖拽画框）。
    ///
    /// 跳过 YOLO 检测，直接对用户框分类；结果覆盖该文件旧识别行。
    fn recognize_region_single(&mut self, bbox: BBox, cx: &mut Context<Self>) {
        if self.recognizing_single.is_some() || self.batch_recognizing {
            self.pending_region = None;
            self.show_toast("识别进行中，稍后再试", cx);
            return;
        }
        let Some(focus_di) = self.focus_index else {
            self.pending_region = None;
            return;
        };
        let Some(&capture_idx) = self.display_order.get(focus_di) else {
            self.pending_region = None;
            return;
        };
        let Some(meta) = self.captures.get(capture_idx) else {
            self.pending_region = None;
            return;
        };
        let dir = match self.dir_path.clone() {
            Some(d) => d,
            None => {
                self.pending_region = None;
                return;
            }
        };
        if self.folder_db.is_none() {
            self.pending_region = None;
            return;
        }

        let primary_path = std::path::PathBuf::from(&meta.primary_path);
        let rel_path = primary_path
            .strip_prefix(&dir)
            .ok()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        let rel_path_clone = rel_path.clone();

        let Some(capture) = self.build_capture_from_meta(meta) else {
            tracing::error!("构建 Capture 失败: {}", meta.base_name);
            self.pending_region = None;
            return;
        };

        // 缩略图优先输入（同 recognize_single）
        let thumb_source = capture.source_files[capture.primary_index].clone();
        let cache = self.thumbnail_cache.clone();

        self.recognizing_single = Some(capture_idx);
        self.recognize_stage = Some("分类中".into());
        cx.notify();

        // 复用缓存 Recognizer（与单张识别共用），用完归还
        let cached = self.single_recognizer.take();

        self.worker.spawn(
            cx,
            move || {
                // 工作线程侧：优先复用缓存 Recognizer，否则现场构建；构建结果随结果归还
                let mut recognizer = match cached {
                    Some(r) => r,
                    None => match Self::build_recognizer() {
                        Ok(r) => r,
                        Err(e) => {
                            return (Some(Err(e)), rel_path_clone, capture_idx, dir, None);
                        }
                    },
                };
                tracing::info!("手动框选识别开始，推理后端: {}", recognizer.backend());
                let thumb_bytes = cache
                    .as_ref()
                    .and_then(|c| c.get_or_generate(&thumb_source, 2048, None).ok());
                let res = recognizer
                    .recognize_region_with_thumbnail(&capture, bbox, thumb_bytes.as_deref(), None)
                    .map_err(|e| format!("识别失败: {e}"));
                // 结果包成 Option<Result<Recognition, String>>：满足 worker.spawn 的 R: Default
                // 约束（Result 无 Default，panic 时整个元组兜底为 None），on_done 只复位哨兵、不写库
                (Some(res), rel_path_clone, capture_idx, dir, Some(recognizer))
            },
            move |this, (result, rel, idx, _scan_dir, recognizer), cx| {
                // 归还缓存 Recognizer（下次识别直接复用，省 DirectML 初始化）
                if let Some(rec) = recognizer {
                    this.single_recognizer = Some(rec);
                }
                this.pending_region = None;
                match result {
                    Some(Ok(rec)) => {
                        // upsert 到 FolderDb（覆盖旧识别行）
                        if let Some(db) = &this.folder_db {
                            if let Err(e) = db.upsert_recognition(&rel, &rec) {
                                tracing::error!("写入识别结果失败 {}: {e}", rel);
                            }
                        }
                        // enrich CaptureMeta
                        if let Some(meta) = this.captures.iter_mut().find(|m| m.index == idx) {
                            meta.enrich_with_recognition(&rec);
                        }
                        // 刷新 focused_recognition；强制显示新检测框
                        this.refresh_focused_recognition();
                        this.recognizing_single = None;
                        this.recognize_stage = None;
                        this.bbox_visible = true;
                        this.apply_filter_and_sort();
                        this.refresh_bird_options();
                        cx.notify();

                        // toast（与单张识别一致）
                        match rec.status {
                            photo_domain::RecognitionStatus::Confirmed => {
                                let bird_name = rec
                                    .bird
                                    .as_ref()
                                    .map(|b| b.cn_name.as_str())
                                    .unwrap_or("未知");
                                let conf = rec.confidence.unwrap_or(0.0);
                                this.show_toast(
                                    format!("{} · 置信度 {:.1}%", bird_name, conf),
                                    cx,
                                );
                            }
                            photo_domain::RecognitionStatus::Unrecognized => {
                                this.show_toast("未检测到鸟类", cx);
                            }
                            photo_domain::RecognitionStatus::NeedsReview => {
                                let reason = rec.failure_stage.user_message();
                                this.show_toast(format!("待复核·{}", reason), cx);
                            }
                        }
                        tracing::info!("手动框选识别完成: {} 状态={:?}", rel, rec.status);
                    }
                    None => {
                        // worker 闭包 panic 兜底（R::default()）：只复位哨兵，不写库
                        tracing::error!("手动框选识别任务异常中断（worker panic）");
                        this.recognizing_single = None;
                        this.recognize_stage = None;
                        this.show_toast("识别任务异常中断", cx);
                        cx.notify();
                    }
                    Some(Err(e)) => {
                        tracing::error!("手动框选识别错误 {}: {e}", rel);
                        this.recognizing_single = None;
                        this.recognize_stage = None;
                        this.show_toast(format!("识别失败: {e}"), cx);
                        cx.notify();
                    }
                }
            },
        );
    }

    /// 多选识别：选中超过一张时批量识别所有选中照片
    pub(crate) fn recognize_selected(&mut self, cx: &mut Context<Self>) {
        if self.recognizing_single.is_some() || self.batch_recognizing {
            tracing::info!("识别忽略：已有识别任务进行中");
            return;
        }
        let dir = match self.dir_path.clone() { Some(d) => d, None => return };
        if self.folder_db.is_none() { return };

        let mut targets: Vec<(usize, photo_domain::Capture)> = Vec::new();
        for meta in &self.captures {
            if self.selected.contains(&meta.index) {
                if let Some(cap) = self.build_capture_from_meta(meta) {
                    targets.push((meta.index, cap));
                }
            }
        }
        if targets.is_empty() {
            return;
        }

        let total = targets.len();
        tracing::info!("多选识别：选中 {} 张，开始批量识别", total);
        self.batch_recognizing = true;
        self.batch_progress_rc = (0, total);
        self.batch_current_file = String::new();
        self.batch_counts = (0, 0, 0);
        self.batch_cancel.store(false, std::sync::atomic::Ordering::Relaxed);
        cx.notify();

        self.spawn_batch_recognize(targets, dir, cx);
    }

    /// 启动批量识别（RecognizeUnrecognized / ConfirmRecognizeAll 共用）
    pub(crate) fn spawn_batch_recognize(
        &mut self,
        targets: Vec<(usize, photo_domain::Capture)>,
        dir: std::path::PathBuf,
        cx: &mut Context<Self>,
    ) {
        use std::sync::atomic::Ordering;

        let cancel = self.batch_cancel.clone();
        let total = targets.len();
        let progress = Arc::new(BatchProgress::default());
        let progress_work = progress.clone();
        let progress_done = progress.clone();

        // B2：批量开始时的代际快照。重扫（scan_generation 变化）后旧索引对新 captures
        // 无意义，所有回填入口先校验，不匹配直接丢弃防张冠李戴
        let generation = self.scan_generation;
        // B3①：folder_db 句柄（Arc<Mutex<Connection>>，Send）移入 worker，
        // 识别完成即写库，UI 线程不再逐张做 SQLite 写
        let folder_db = self.folder_db.clone();
        // 缩略图优先输入：批量识别前取 .pt/thumbs 派生图字节，省每张全图解码
        let cache = self.thumbnail_cache.clone();

        let n_threads = (self.config.recognition_thread_count as usize).min(total).max(1);

        self.worker.spawn(
            cx,
            move || {
                // 每个识别线程懒加载自己独占的 Recognizer（Session 需 &mut，不可跨线程共享）
                let create_recognizer = || -> Result<Recognizer, String> {
                    let exe_dir = std::env::current_exe()
                        .map_err(|e| format!("获取 exe 路径失败: {e}"))?
                        .parent()
                        .ok_or_else(|| "无法确定 exe 目录".to_string())?
                        .to_path_buf();
                    let models_dir = exe_dir.join("models");
                    let catalog_db = exe_dir.join("data").join("pica_ref.db");
                    Recognizer::new(&models_dir, &catalog_db)
                        .map_err(|e| format!("加载识别模型失败: {e}"))
                };

                tracing::info!("批量识别开始，共 {} 张，{} 个识别线程", total, n_threads);

                // 全局“开始序号”，仅用于日志显示 [N/total]
                let started_counter = std::sync::atomic::AtomicUsize::new(0);

                // 分块 + std::thread::scope：每线程恰好创建一份 Recognizer，顺序处理本块
                std::thread::scope(|s| {
                    // 防御：空目标已由调用方守卫拦截，这里再钳制一次防止 chunks(0) panic
                    let chunk_size = total.div_ceil(n_threads).max(1);
                    // 以引用形式供 move 闭包捕获，避免 FnMut 中移出外层变量
                    let cancel = &cancel;
                    let dir = &dir;
                    let cache = &cache;
                    let progress_work = &progress_work;
                    let started_counter = &started_counter;
                    let db_work = &folder_db;
                    let handles: Vec<_> = targets
                        .chunks(chunk_size)
                        .map(|chunk| {
                            s.spawn(move || {
                                let mut recognizer = create_recognizer();

                                for (capture_idx, cap) in chunk {
                                    if cancel.load(Ordering::Relaxed) {
                                        break;
                                    }

                                    let primary = &cap.source_files[cap.primary_index];
                                    let rel = primary.path.strip_prefix(&dir)
                                        .map(|p| p.to_string_lossy().replace('\\', "/"))
                                        .unwrap_or_default();

                                    // 开始前记录当前文件：若中途真卡死，最后一行日志即嫌疑人
                                    let n = started_counter.fetch_add(1, Ordering::Relaxed) + 1;
                                    tracing::info!("批量识别 [{}/{}] 识别中: {}", n, total, rel);
                                    *progress_work.current.lock() = rel.clone();
                                    let started = std::time::Instant::now();

                                    // 缩略图优先输入：命中 .pt/thumbs 派生图（JPEG DCT
                                    // 毫秒级 / RAW 母版派生），省每张全图 image::open；
                                    // 取消令牌顺带跳过慢 IO
                                    let thumb_bytes = cache.as_ref().and_then(|c| {
                                        c.get_or_generate(primary, 2048, Some(cancel)).ok()
                                    });
                                    let rec_result = match &mut recognizer {
                                        Ok(rec) => rec
                                            .recognize_with_thumbnail(cap, thumb_bytes.as_deref(), None)
                                            .map_err(|e| format!("识别失败: {e}")),
                                        Err(e) => Err(e.clone()),
                                    };

                                    let elapsed = started.elapsed();
                                    progress_work.done.fetch_add(1, Ordering::Relaxed);
                                    match &rec_result {
                                        Ok(rec) => {
                                            match rec.status {
                                                photo_domain::RecognitionStatus::Confirmed => &progress_work.confirmed,
                                                photo_domain::RecognitionStatus::Unrecognized => &progress_work.unrecognized,
                                                photo_domain::RecognitionStatus::NeedsReview => &progress_work.needs_review,
                                            }
                                            .fetch_add(1, Ordering::Relaxed);
                                        }
                                        Err(_) => {
                                            progress_work.needs_review.fetch_add(1, Ordering::Relaxed);
                                        }
                                    }
                                    // 慢照片告警：正常 2-4s/张，超 10s 说明该文件解码或推理异常
                                    if elapsed.as_secs() >= 10 {
                                        tracing::warn!("批量识别 [{}/{}] {} 耗时 {:.1?}，异常缓慢", n, total, rel, elapsed);
                                    }

                                    // B3①：识别完成即写库（工作线程侧）。DB 按 rel 路径键控，
                                    // 与 UI 代际无关，重扫后仍有效；UI 线程不再做 SQLite 写
                                    if let Ok(rec) = &rec_result {
                                        if let Some(db) = db_work.as_ref() {
                                            if let Err(e) = db.upsert_recognition(&rel, rec) {
                                                tracing::error!("写入识别结果失败 {}: {e}", rel);
                                            }
                                        }
                                    }

                                    // 逐张推入共享队列：UI 轮询 drain 后网格即时刻入，
                                    // 不再等整批结束
                                    progress_work.results.lock().push((*capture_idx, rel, rec_result));
                                }
                            })
                        })
                        .collect();

                    if cancel.load(Ordering::Relaxed) {
                        let done = progress_work.done.load(Ordering::Relaxed);
                        tracing::info!("批量识别被取消，已完成 {}/{} 张", done, total);
                    }

                    // 单线程 panic 不应吞掉其他线程：join 只为检出 panic
                    for h in handles {
                        if h.join().is_err() {
                            tracing::error!("批量识别某工作线程 panic，该线程未完成的结果丢失");
                        }
                    }
                })
            },
            move |this, (), cx| {
                // 轮询可能尚未 drain 最后几张，on_done 先兜底清队列（锁内 take，不会重复应用）
                let remaining = std::mem::take(&mut *progress_done.results.lock());
                this.apply_recognition_results(remaining, generation);

                let done = progress_done.done.load(Ordering::Relaxed);
                let confirmed = progress_done.confirmed.load(Ordering::Relaxed);
                let unrecognized = progress_done.unrecognized.load(Ordering::Relaxed);
                let needs_review = progress_done.needs_review.load(Ordering::Relaxed);

                this.batch_progress_rc = (done, total);
                this.batch_counts = (confirmed, unrecognized, needs_review);
                this.batch_recognizing = false;
                this.apply_filter_and_sort();
                this.refresh_focused_recognition();
                this.refresh_bird_options();

                this.show_toast(format!(
                    "识别完成：确认 {} · 无鸟 {} · 待复核 {}",
                    confirmed, unrecognized, needs_review
                ), cx);

                tracing::info!(
                    "批量识别结束：完成 {} / {} 张，确认 {} 无鸟 {} 待复核 {}",
                    done, total, confirmed, unrecognized, needs_review
                );
                cx.notify();
            },
        );

        // 轮询共享进度，实时刷新状态栏与网格（与 sync_progress 同一模式）；
        // on_done 将 batch_recognizing 置 false 后轮询退出
        cx.spawn(move |weak: WeakEntity<RootView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                loop {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(200))
                        .await;
                    let Some(view) = weak.upgrade() else {
                        break;
                    };
                    let running = cx.update_entity(&view, |this, cx| {
                        if !this.batch_recognizing {
                            return false;
                        }
                        this.batch_progress_rc = (progress.done.load(Ordering::Relaxed), total);
                        this.batch_current_file = progress.current.lock().clone();
                        this.batch_counts = (
                            progress.confirmed.load(Ordering::Relaxed),
                            progress.unrecognized.load(Ordering::Relaxed),
                            progress.needs_review.load(Ordering::Relaxed),
                        );
                        // drain 已完成结果：逐张刻到网格（DB 写入已在工作线程完成，此处仅 enrich 内存）
                        let drained = std::mem::take(&mut *progress.results.lock());
                        if !drained.is_empty() {
                            this.apply_recognition_results(drained, generation);
                            this.refresh_focused_recognition();
                            this.refresh_bird_options();
                            // B3③ 重排去抖：轮询期只回填内存，批末 on_done 统一一次
                            // apply_filter_and_sort，避免 640 张批中每 200ms 全量重排
                        }
                        cx.notify();
                        true
                    });
                    if !running {
                        break;
                    }
                }
            }
        })
        .detach();
    }

    /// 应用一批批量识别结果：仅 enrich CaptureMeta（DB 写入已在工作线程完成）。
    /// 轮询 drain 与 on_done 兜底共用；状态计数由工作线程侧原子量维护，此处不累加。
    ///
    /// `generation` 为批量开始时的代际快照：重扫（scan_generation 变化）后旧索引
    /// 对新 captures 无意义，不匹配直接丢弃，防结果张冠李戴。
    pub(crate) fn apply_recognition_results(&mut self, results: Vec<BatchResult>, generation: u64) {
        if generation != self.scan_generation {
            tracing::warn!(
                "批量识别回填代际不匹配（{} != {}），丢弃 {} 条结果",
                generation,
                self.scan_generation,
                results.len()
            );
            return;
        }
        // B3②：预建 capture index → 位置 映射，替代逐条 iter_mut().find() 的 O(N) 查找
        let pos_by_index: HashMap<usize, usize> = self
            .captures
            .iter()
            .enumerate()
            .map(|(pos, m)| (m.index, pos))
            .collect();
        for (capture_idx, rel, rec_result) in &results {
            match rec_result {
                Ok(rec) => {
                    if let Some(&pos) = pos_by_index.get(capture_idx) {
                        if let Some(meta) = self.captures.get_mut(pos) {
                            meta.enrich_with_recognition(rec);
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("批量识别错误 {}: {e}", rel);
                }
            }
        }
    }
}
