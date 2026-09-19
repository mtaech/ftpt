//! 后端与引擎同步调用封装（在后台任务中执行，结果回传到 GPUI 实体）
//!
//! 对应 §7、§8 与附录 A。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use gpui_kit::{App, Entity};
use photo_domain::{
    AdjustParams, CaptureMeta, ColorLabel, FilterCriteria, Flag, ImageFormat, Rating, SourceFile,
};

use crate::image::{ImageManager, THUMB_SIZE_GRID, source_file_of};
use crate::model::adjust::rel_path_of;
use photo_engine::folder_db::{ExifCacheRow, FileEntry, FolderDb};
use photo_engine::scanner;

use super::app_state::AppState;

/// 在监听器里安全触发一个「会同步 update AppState」的入口（扫描 / 识别 / 删除 …）。
///
/// GPUI 的 `Context::listener` 运行时 AppState 已被租借，此时再 `entity.update` 会 panic
/// （cannot update ... while it is already being updated）。`Context::defer_in` 的回调里
/// 实体同样处于租借状态，也不能用。这里用 `cx.spawn` 把调用排到本轮 effect 之后，
/// 由 `AsyncApp::update` 执行——那时实体已经归还给 App。
pub fn defer_entity_action(
    cx: &mut gpui_kit::Context<AppState>,
    entity: Entity<AppState>,
    f: impl FnOnce(Entity<AppState>, &mut App) + 'static,
) {
    cx.spawn(
        async move |_weak: gpui_kit::WeakEntity<AppState>, async_cx: &mut gpui_kit::AsyncApp| {
            let _ = async_cx.update(|cx| f(entity, cx));
        },
    )
    .detach();
}

/// 启动目录扫描任务
pub fn start_scan(state_entity: Entity<AppState>, dir: PathBuf, recursive: bool, cx: &mut App) {
    let (generation, cancel_token) = state_entity.update(cx, |state, cx| {
        // 换目录前先把上一张未落盘的调整参数写回：flush 用的是旧的 current_dir + folder_db，
        // 一旦 current_dir 先改了，相对路径就会算到新目录上（写错库或干脆写不进去）。
        state.flush_adjustments();
        state.adjust_path = None;
        state.adjust = AdjustParams::default();
        state.adjust_preview_toned = false;
        state.scan_generation = state.scan_generation.wrapping_add(1);
        state.scan_cancel.store(true, Ordering::Relaxed);
        let new_cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        state.scan_cancel = new_cancel.clone();
        state.is_scanning = true;
        state.scan_stage = Some("正在扫描文件...".to_string());
        state.scan_done = 0;
        state.scan_total = 0;
        state.current_dir = Some(dir.clone());
        cx.notify();
        (state.scan_generation, new_cancel)
    });

    let entity_clone = state_entity.clone();
    let dir_clone = dir.clone();

    cx.spawn(async move |async_cx| {
        let result = async_cx
            .background_executor()
            .spawn(async move { do_background_scan(&dir_clone, recursive, &cancel_token).await })
            .await;

        let _ = async_cx.update(|cx| {
            state_entity.update(cx, |state, cx| {
                if state.scan_generation != generation {
                    return; // 旧代际扫描结果丢弃（§8.3 一致性机制）
                }
                state.is_scanning = false;
                state.scan_stage = None;

                match result {
                    Ok((metas, db)) => {
                        state.items = metas;
                        state.folder_db = db;
                        state
                            .image_manager
                            .set_cache_dir(Some(dir.join(".pt").join("thumbs")));

                        // 记录到最近打开
                        if !state.recent_dirs.contains(&dir) {
                            state.recent_dirs.insert(0, dir.clone());
                            if state.recent_dirs.len() > 20 {
                                state.recent_dirs.truncate(20);
                            }
                        }

                        // 重算子目录
                        state.subdirs = list_subdirectories(&dir);

                        state.recompute_pipeline();
                        state.set_status_message(format!(
                            "扫描完成，共 {} 张照片",
                            state.items.len()
                        ));

                        // 默认选中第一张
                        if let Some(&first) = state.display_order.first() {
                            state.select_single(first);
                        }
                    }
                    Err(e) => {
                        state.set_status_message(format!("扫描失败: {e}"));
                    }
                }
                cx.notify();
            });
        });

        // 启动后台缩略图生成管线（§7.1）：先补图，再提 EXIF
        let _ = async_cx.update(|cx| {
            start_thumb_pipeline(entity_clone.clone(), generation, cx);
        });

        // 启动后台 EXIF 增量提取任务
        let _ = async_cx.update(|cx| {
            start_background_enrich(entity_clone, generation, cx);
        });
    })
    .detach();
}

/// 缩略图后台生成管线（§7.1）。
///
/// 扫描完成后调用：把「磁盘上还没有缩略图的照片」按 **display_order（可见顺序）** 排成队列，
/// 用少量并发在后台补齐，每 250ms 回主线程更新进度并 `cx.notify()`——
/// 网格每帧都重新查缓存路径，缩略图落盘后下一次绘制即命中，不需要逐张回传图像。
///
/// 取消沿用 `scan_cancel`：新一次扫描开始即中止旧管线（§8.3 生成号 + 取消令牌）。
/// 视频等非图片格式跳过——不生成缩略图，预览走「用默认软件打开」。
pub fn start_thumb_pipeline(state_entity: Entity<AppState>, generation: u64, cx: &mut App) {
    /// 并发解码线程数：解码是 CPU 密集，2–4 足够，过高会拖慢扫描与交互
    const WORKERS: usize = 3;
    const TICK_MS: u64 = 250;

    let Some((jobs, cancel, manager)) = state_entity.update(cx, |state, _cx| {
        let cancel = state.scan_cancel.clone();
        let manager = state.image_manager.clone();

        let mut jobs: Vec<SourceFile> = Vec::new();
        for &idx in &state.display_order {
            let Some(meta) = state.items.get(idx) else {
                continue;
            };
            let Some(source) = source_file_of(meta) else {
                continue;
            };
            if source.format.is_other() {
                continue; // 视频等：不生成缩略图
            }
            if manager
                .get_cached_thumb_path(&source, THUMB_SIZE_GRID)
                .is_some()
            {
                continue;
            }
            jobs.push(source);
        }

        if jobs.is_empty() {
            return None;
        }
        state.thumb_done = 0;
        state.thumb_total = jobs.len();
        Some((jobs, cancel, manager))
    }) else {
        return;
    };

    let total = jobs.len();
    let jobs = Arc::new(jobs);
    let done = Arc::new(AtomicUsize::new(0));
    let cursor = Arc::new(AtomicUsize::new(0));

    cx.spawn(async move |async_cx| {
        // ── 并发 worker：共享游标取任务，各自写磁盘缓存 ──
        for _ in 0..WORKERS {
            let jobs = jobs.clone();
            let done = done.clone();
            let cursor = cursor.clone();
            let cancel = cancel.clone();
            let manager = manager.clone();
            async_cx
                .background_executor()
                .spawn(async move {
                    loop {
                        if cancel.load(Ordering::Relaxed) {
                            break;
                        }
                        let i = cursor.fetch_add(1, Ordering::Relaxed);
                        let Some(source) = jobs.get(i) else { break };
                        if let Err(e) =
                            manager.ensure_thumb(source, THUMB_SIZE_GRID, Some(cancel.as_ref()))
                        {
                            tracing::debug!("缩略图生成失败 {}: {e}", source.path.display());
                        }
                        done.fetch_add(1, Ordering::Relaxed);
                    }
                })
                .detach();
        }

        // ── 轮询进度：回主线程刷新计数与网格 ──
        loop {
            async_cx
                .background_executor()
                .timer(Duration::from_millis(TICK_MS))
                .await;

            let finished = done.load(Ordering::Relaxed).min(total);
            let keep_running = async_cx.update(|cx| {
                let mut keep = false;
                let _ = state_entity.update(cx, |state, cx| {
                    if state.scan_generation != generation {
                        return; // 旧代际：让位给新管线
                    }
                    state.thumb_done = finished;
                    cx.notify();
                    keep = finished < total && !state.scan_cancel.load(Ordering::Relaxed);
                });
                keep
            });
            if !keep_running {
                break;
            }
        }

        // ── 收尾：清进度（同代际才清，避免清掉新一代的计数）──
        let _ = async_cx.update(|cx| {
            let _ = state_entity.update(cx, |state, cx| {
                if state.scan_generation == generation {
                    state.thumb_done = 0;
                    state.thumb_total = 0;
                    cx.notify();
                }
            });
        });
    })
    .detach();
}

/// 后台加载预览母版（§7.2）。
///
/// 预览**绝不直接渲染原图**：39MP 原图 RGBA 展开约 157MB，而且渲染层（image crate）不认 RAW。
/// 渲染层先用网格缩略图占位，这里在后台把 2560 母版备好，完成后回主线程替换——
/// 两者按同一显示尺寸排布，所以没有布局跳动。
///
/// manager 由调用方传入而不是读实体：本函数在渲染期被调用，而渲染期实体已被可变借用。
pub fn load_preview_image(
    state_entity: Entity<AppState>,
    manager: ImageManager,
    path: String,
    source: SourceFile,
    cx: &mut App,
) {
    cx.spawn(async move |async_cx| {
        let result = async_cx
            .background_executor()
            .spawn(async move { manager.load_master_image(&source, None) })
            .await;

        let _ = async_cx.update(|cx| {
            let _ = state_entity.update(cx, |state, cx| {
                match result {
                    Ok(img) => {
                        // 只有「当前请求的仍是这张」才替换显示；否则留在内存缓存里（回头再点即秒开）
                        // 且这张已有烘焙好的调整预览时不能用未调整母版覆盖（两者可能并发完成）
                        let toned_active = state.adjust_preview_toned
                            && state.adjust_path.as_deref() == Some(path.as_str());
                        if state.preview_request.as_deref() == Some(path.as_str()) && !toned_active {
                            state.preview_image = Some((path.clone(), img));
                        }
                    }
                    Err(e) => tracing::warn!("预览母版加载失败 {path}: {e}"),
                }
                cx.notify();
            });
        });
    })
    .detach();
}

/// 后台加载 1:1 全分辨率图源（RAW：AHD 全尺寸解码，落盘在 `full` 缓存变体）。
///
/// 与母版加载同样只在渲染期发起一次：显示尺寸超过母版像素（含 1:1）时，
/// 渲染层先用母版顶上，这里在后台把真原图备好，完成后回主线程替换。
/// 记忆点：全尺寸 RGBA 约 177MB（44MP），所以 ImageManager 只缓存最后一张。
pub fn load_preview_full(
    state_entity: Entity<AppState>,
    manager: ImageManager,
    path: String,
    source: SourceFile,
    cx: &mut App,
) {
    cx.spawn(async move |async_cx| {
        let result = async_cx
            .background_executor()
            .spawn(async move { manager.load_full_image(&source, None) })
            .await;

        let _ = async_cx.update(|cx| {
            let _ = state_entity.update(cx, |state, cx| {
                match result {
                    Ok(img) => {
                        // 只有「当前请求的仍是这张」才替换显示；否则留在内存缓存里（回头再点即秒开）
                        if state.preview_full_request.as_deref() == Some(path.as_str()) {
                            state.preview_full = Some((path.clone(), img));
                        }
                    }
                    Err(e) => tracing::warn!("1:1 全分辨率加载失败 {path}: {e}"),
                }
                cx.notify();
            });
        });
    })
    .detach();
}

// ── 调整参数（ADR 0007）：读取 / 落盘 / 预览渲染 ──

/// 读取某张图持久化的调整参数。
///
/// 无 folder_db、路径不在当前目录下、无记录三种情况都返回中性参数
/// （与「从未识别」同构：无行 = 无调整）。
pub fn load_adjustments(state: &AppState, path: &str) -> AdjustParams {
    let (Some(db), Some(dir)) = (&state.folder_db, &state.current_dir) else {
        return AdjustParams::default();
    };
    let Some(rel) = rel_path_of(dir, Path::new(path)) else {
        return AdjustParams::default();
    };
    match db.get_adjustments(&rel) {
        Ok(Some(params)) => params,
        Ok(None) => AdjustParams::default(),
        Err(e) => {
            tracing::warn!("读取调整参数失败 {rel}: {e}");
            AdjustParams::default()
        }
    }
}

/// 写入某张图的调整参数。全零参数也写（显式记录「已复位」，读取侧与无行同义）。
pub fn persist_adjustments(state: &AppState, path: &str, params: &AdjustParams) {
    let (Some(db), Some(dir)) = (&state.folder_db, &state.current_dir) else {
        return;
    };
    let Some(rel) = rel_path_of(dir, Path::new(path)) else {
        return;
    };
    if let Err(e) = db.put_adjustments(&rel, params) {
        tracing::warn!("保存调整参数失败 {rel}: {e}");
    }
}

/// 后台渲染「带调整参数」的预览母版，完成后回主线程替换 `preview_image`。
///
/// 拖动滑杆会连续发起多次渲染，只有 `seq` 仍是最新的那次允许写回（旧帧丢弃，
/// 画面不回跳）；切图后 `adjust_path` 变了，过期结果同样被丢弃。
/// 与 `load_preview_image` 同口径——manager 由调用方传入（渲染期实体已被可变借用）。
pub fn render_adjust_preview(
    state_entity: Entity<AppState>,
    manager: ImageManager,
    path: String,
    source: SourceFile,
    params: AdjustParams,
    seq: u64,
    cx: &mut App,
) {
    cx.spawn(async move |async_cx| {
        let result = async_cx
            .background_executor()
            .spawn(async move { manager.render_adjusted_master(&source, &params) })
            .await;

        let _ = async_cx.update(|cx| {
            let _ = state_entity.update(cx, |state, cx| {
                if state.adjust_render_seq != seq
                    || state.adjust_path.as_deref() != Some(path.as_str())
                {
                    return;
                }
                match result {
                    Ok(img) => {
                        state.preview_image = Some((path, img));
                        state.adjust_preview_toned = !params.is_neutral();
                    }
                    Err(e) => tracing::warn!("调整预览渲染失败: {e}"),
                }
                cx.notify();
            });
        });
    })
    .detach();
}

/// 后台扫描核心
async fn do_background_scan(
    dir: &Path,
    recursive: bool,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<(Vec<CaptureMeta>, Option<FolderDb>), String> {
    let folder_db = FolderDb::open_in_dir(dir).ok();

    if cancel.load(Ordering::Relaxed) {
        return Err("已取消".to_string());
    }

    let captures = if recursive {
        scanner::scan_directory_recursive(dir, &FilterCriteria::default(), None)
    } else {
        scanner::scan_directory(dir, &FilterCriteria::default(), None)
    }
    .map_err(|e| e.to_string())?;

    let mut entries: Vec<FileEntry> = Vec::new();
    let mut fingerprints: HashMap<String, (u64, i64)> = HashMap::new();

    for f in captures.iter().flat_map(|c| c.source_files.iter()) {
        let Ok(rel) = f.path.strip_prefix(dir) else {
            continue;
        };
        let Some(m) = std::fs::metadata(&f.path).ok() else {
            continue;
        };
        let mtime_ns = m
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0);
        fingerprints.insert(f.path.to_string_lossy().to_string(), (m.len(), mtime_ns));
        entries.push(FileEntry {
            full_path: f.path.clone(),
            rel_path: rel.to_string_lossy().replace('\\', "/"),
            file_size: m.len(),
            mtime_ns,
            format: f.format.clone(),
        });
    }

    if let Some(db) = &folder_db {
        let _ = db.sync_with_scan(&entries, &|_, _| {});
    }

    let xmp_rows = folder_db
        .as_ref()
        .and_then(|db| db.all_xmp_meta().ok())
        .unwrap_or_default();
    let keyword_rows = folder_db
        .as_ref()
        .and_then(|db| db.all_keywords().ok())
        .unwrap_or_default();
    let exif_rows: HashMap<String, ExifCacheRow> = folder_db
        .as_ref()
        .and_then(|db| db.all_exif().ok())
        .unwrap_or_default();

    let mut metas: Vec<CaptureMeta> = captures
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let mut meta = CaptureMeta::from_capture(c, i);
            let primary = &c.source_files[c.primary_index];
            let key = primary.path.to_string_lossy().to_string();
            if let Some(xmp) = xmp_rows.get(&key) {
                meta.enrich_with_xmp(xmp);
            }
            if let Some(kws) = keyword_rows.get(&key) {
                meta.enrich_with_keywords(kws);
            }
            if let Some(&(size, mtime_ns)) = fingerprints.get(&key) {
                if let Some(row) = exif_rows.get(&key) {
                    if row.file_size == size as i64 && row.mtime_ns == mtime_ns {
                        meta.enrich_with_exif(&row.exif);
                    }
                }
            }
            meta
        })
        .collect();

    // 填充已有识别记录
    if let Some(db) = &folder_db {
        if let Ok(recs) = db.all_recognitions() {
            let rec_map: HashMap<&str, &photo_domain::Recognition> =
                recs.iter().map(|(p, r)| (p.as_str(), r)).collect();
            for meta in metas.iter_mut() {
                let primary_path = Path::new(&meta.primary_path);
                let rel = primary_path
                    .strip_prefix(dir)
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_default();
                if let Some(r) = rec_map.get(rel.as_str()) {
                    meta.enrich_with_recognition(r);
                }
            }
        }
    }

    Ok((metas, folder_db))
}

/// 后台增量 EXIF 提取
pub fn start_background_enrich(state_entity: Entity<AppState>, generation: u64, cx: &mut App) {
    let to_enrich: Vec<(usize, PathBuf, ImageFormat)> = state_entity.update(cx, |state, _| {
        if state.scan_generation != generation {
            return Vec::new();
        }
        state
            .items
            .iter()
            .enumerate()
            .filter(|(_, m)| m.camera_make.is_none() && m.date_taken.is_none())
            .filter_map(|(i, m)| {
                let p = Path::new(&m.primary_path);
                let ext = p.extension()?.to_str()?;
                let fmt = ImageFormat::from_extension(ext)?;
                Some((i, p.to_path_buf(), fmt))
            })
            .collect()
    });

    if to_enrich.is_empty() {
        return;
    }

    cx.spawn(async move |async_cx| {
        let enriched_results = async_cx
            .background_executor()
            .spawn(async move {
                let mut results = Vec::new();
                for (idx, path, fmt) in to_enrich {
                    if let Ok(meta) = photo_engine::exif::extract_exif(&path, &fmt) {
                        results.push((idx, path, meta));
                    }
                }
                results
            })
            .await;

        let _ = async_cx.update(|cx| {
            state_entity.update(cx, |state, cx| {
                if state.scan_generation != generation {
                    return;
                }
                for (idx, path, meta) in enriched_results {
                    if let Some(item) = state.items.get_mut(idx) {
                        item.enrich_with_exif(&meta);
                        if let Some(db) = &state.folder_db {
                            let _ = db.put_exif(&path, &meta);
                        }
                    }
                }
                state.recompute_pipeline();
                cx.notify();
            });
        });
    })
    .detach();
}

/// 列出子目录
pub fn list_subdirectories(dir: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(read) = std::fs::read_dir(dir) {
        for entry in read.flatten() {
            if let Ok(file_type) = entry.file_type() {
                if file_type.is_dir() {
                    let name = entry.file_name();
                    let name_str = name.to_string_lossy();
                    if !name_str.starts_with('.') {
                        dirs.push(entry.path());
                    }
                }
            }
        }
    }
    dirs.sort();
    dirs
}

/// 乐观修改评分
pub fn set_rating(state: &mut AppState, rating: Rating) {
    let indices = state.mark_indices();
    for &idx in &indices {
        if let Some(item) = state.items.get_mut(idx) {
            item.rating = rating;
            if let Some(db) = &state.folder_db {
                let p = Path::new(&item.primary_path);
                if let Ok(mut xmp) = db.get_xmp(p).map(|x| x.unwrap_or_default()) {
                    xmp.set_rating(rating);
                    let _ = db.put_xmp(p, &xmp);
                }
            }
        }
    }
    state.recompute_pipeline();
}

/// 乐观修改色标
pub fn set_color_label(state: &mut AppState, label: ColorLabel) {
    let indices = state.mark_indices();
    for &idx in &indices {
        if let Some(item) = state.items.get_mut(idx) {
            item.color_label = label;
            if let Some(db) = &state.folder_db {
                let p = Path::new(&item.primary_path);
                if let Ok(mut xmp) = db.get_xmp(p).map(|x| x.unwrap_or_default()) {
                    xmp.set_color_label(label);
                    let _ = db.put_xmp(p, &xmp);
                }
            }
        }
    }
    state.recompute_pipeline();
}

/// 乐观修改旗标
pub fn set_flag(state: &mut AppState, flag: Option<Flag>) {
    let indices = state.mark_indices();
    for &idx in &indices {
        if let Some(item) = state.items.get_mut(idx) {
            item.flag = flag;
            if let Some(db) = &state.folder_db {
                let p = Path::new(&item.primary_path);
                if let Ok(mut xmp) = db.get_xmp(p).map(|x| x.unwrap_or_default()) {
                    xmp.set_flag(flag);
                    let _ = db.put_xmp(p, &xmp);
                }
            }
        }
    }
    state.recompute_pipeline();
}

/// 对指定路径列表批量设置旗标
pub fn set_flag_for_paths(state: &mut AppState, paths: &[String], flag: Option<Flag>) {
    let path_set: std::collections::HashSet<&str> = paths.iter().map(|s| s.as_str()).collect();
    for item in &mut state.items {
        if path_set.contains(item.primary_path.as_str()) {
            item.flag = flag;
            if let Some(db) = &state.folder_db {
                let p = Path::new(&item.primary_path);
                if let Ok(mut xmp) = db.get_xmp(p).map(|x| x.unwrap_or_default()) {
                    xmp.set_flag(flag);
                    let _ = db.put_xmp(p, &xmp);
                }
            }
        }
    }
    state.recompute_pipeline();
}

/// 删除指定路径列表到回收站
pub fn delete_paths(state_entity: Entity<AppState>, paths: Vec<PathBuf>, cx: &mut App) {
    if paths.is_empty() {
        return;
    }
    let current_dir = state_entity.read(cx).current_dir.clone();

    cx.spawn(async move |async_cx| {
        let count = paths.len();
        for p in &paths {
            let _ = trash::delete(p);
        }
        let _ = async_cx.update(|cx| {
            let _ = state_entity.update(cx, |state, cx| {
                state.set_status_message(format!("已删除 {count} 项到回收站"));
                cx.notify();
            });
        });
        // 重扫必须在上面的 update 之外启动：start_scan 自己会 update 同一个实体，
        // 嵌在上面的闭包里就是同一实体的双重租借（GPUI 直接 panic）。
        if let Some(dir) = current_dir {
            let _ = async_cx.update(|cx| {
                start_scan(state_entity.clone(), dir, false, cx);
            });
        }
    })
    .detach();
}

/// 删除选中项到回收站（§5.2）
pub fn delete_selected_to_trash(state_entity: Entity<AppState>, cx: &mut App) {
    let paths: Vec<PathBuf> = state_entity.update(cx, |state, _| {
        let indices = state.mark_indices();
        indices
            .iter()
            .filter_map(|&i| state.items.get(i).map(|m| PathBuf::from(&m.primary_path)))
            .collect()
    });

    delete_paths(state_entity, paths, cx);
}

/// 后台识别线程回传给前台的进度事件。
///
/// 识别是同步 CPU 推理，只能放后台线程跑；但结果必须回到前台才能写 `AppState` 与
/// `folder_db`（SQLite 连接随实体走），所以用 std channel 传事件、前台按节拍收。
enum RecognizeOutcome {
    /// 单张完成（Err 表示这张系统性失败；业务失败体现在 Recognition.status 里）
    Done {
        idx: usize,
        path: PathBuf,
        filename: String,
        result: Result<photo_domain::Recognition, photo_recognize::RecognizeError>,
    },
    /// 模型/名录库不可用：整批中止
    Fatal(String),
    /// 后台 worker 退出（跑完或被取消）——前台据此收尾并停掉轮询
    Finished,
}

/// 单张识别结果落内存 + 落 folder_db（**只在前台线程调用**）。
fn apply_recognition(
    state: &mut AppState,
    idx: usize,
    path: &Path,
    result: &Result<photo_domain::Recognition, photo_recognize::RecognizeError>,
) {
    let Ok(recognition) = result else {
        return;
    };
    if let Some(meta) = state.items.get_mut(idx) {
        meta.enrich_with_recognition(recognition);
    }
    // 识别结果写回文件夹级 data.db（连接不能跨线程，所以在前台写）
    if let (Some(db), Some(dir)) = (&state.folder_db, &state.current_dir) {
        if let Ok(rel) = path.strip_prefix(dir) {
            let rel_str = rel.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/");
            let _ = db.upsert_recognition(&rel_str, recognition);
        }
    }
}

/// 启动鸟类识别管线（§5.6）
///
/// **推理必须留在后台线程**：`Recognizer::recognize` 是 CPU 密集的同步调用（单张
/// 几百 ms）。此前它直接写在 `cx.spawn` 的循环里，而该循环没有任何 `.await`——GPUI 的
/// 前台执行器（渲染 + 事件循环）被整批识别占住：状态栏的「识别中 n/m」要等全部跑完
/// 才闪一下（用户报的「全部识别没有进度」），「取消识别」也点不动。
/// 现在与缩略图管线同构：后台 worker 推理 → channel 回传 → 前台每 TICK_MS 收一次结果，
/// 更新 items / folder_db / 进度字段并 `cx.notify()`，所以进度条真的会走。
pub fn start_recognition(
    state_entity: Entity<AppState>,
    only_unrecognized: bool,
    force_all: bool,
    cx: &mut App,
) {
    /// 前台收结果的节拍：状态栏进度条按这个频率刷新（与缩略图管线同量级）
    const TICK_MS: u64 = 250;

    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_flag = cancel.clone();

    let items_to_recognize: Vec<(usize, PathBuf)> = state_entity.update(cx, |state, cx| {
        if state.is_recognizing {
            return Vec::new();
        }
        state.is_recognizing = true;
        state.recognize_cancel = cancel.clone();
        state.recognize_done = 0;
        // 进度条上别残留上一批的文件名
        state.recognize_current.clear();

        let targets: Vec<usize> = if force_all {
            (0..state.items.len()).collect()
        } else if only_unrecognized {
            state
                .items
                .iter()
                .enumerate()
                .filter(|(_, m)| m.recognition_status.is_none())
                .map(|(i, _)| i)
                .collect()
        } else {
            state.mark_indices()
        };

        state.recognize_total = targets.len() as u32;
        cx.notify();

        targets
            .into_iter()
            .filter_map(|i| {
                state
                    .items
                    .get(i)
                    .map(|m| (i, PathBuf::from(&m.primary_path)))
            })
            .collect()
    });

    if items_to_recognize.is_empty() {
        state_entity.update(cx, |state, cx| {
            state.is_recognizing = false;
            cx.notify();
        });
        return;
    }

    let Some(data_root) = crate::state::app_state::data_root() else {
        state_entity.update(cx, |state, cx| {
            state.is_recognizing = false;
            state.set_status_message("无法定位模型与名录库数据目录 (PHOTO_DATA_DIR)");
            cx.notify();
        });
        return;
    };

    let models_dir = data_root.join("models");
    let catalog_db = data_root.join("data/bird_catalog.db");

    // 后台 worker -> 前台的结果队列（前台 try_recv 非阻塞，节拍到了就收）
    let (tx, rx) = std::sync::mpsc::channel::<RecognizeOutcome>();

    cx.spawn(async move |async_cx: &mut gpui_kit::AsyncApp| {
        // ── 1. 后台线程：模型加载 + 逐张推理（CPU 密集，绝不放前台执行器） ──
        async_cx
            .background_executor()
            .spawn(async move {
                let recognizer = photo_recognize::Recognizer::new(&models_dir, &catalog_db);
                let mut recognizer = match recognizer {
                    Ok(recognizer) => recognizer,
                    Err(e) => {
                        tracing::warn!("初始化识别模型失败: {e}");
                        let _ = tx.send(RecognizeOutcome::Fatal("初始化识别模型失败".to_string()));
                        let _ = tx.send(RecognizeOutcome::Finished);
                        return;
                    }
                };

                for (idx, path) in items_to_recognize {
                    if cancel_flag.load(Ordering::Relaxed) {
                        break;
                    }

                    let format = photo_domain::ImageFormat::from_extension(
                        path.extension()
                            .unwrap_or_default()
                            .to_str()
                            .unwrap_or_default(),
                    )
                    .unwrap_or(photo_domain::ImageFormat::Jpeg);
                    let capture = photo_domain::Capture {
                        base_name: path
                            .file_stem()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string(),
                        source_files: vec![photo_domain::SourceFile {
                            path: path.clone(),
                            format,
                            file_size: None,
                        }],
                        primary_index: 0,
                    };
                    let result = recognizer.recognize(&capture, None, None);
                    let filename = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();

                    // 前台已经走了（实体回收）→ 直接收工
                    if tx
                        .send(RecognizeOutcome::Done {
                            idx,
                            path,
                            filename,
                            result,
                        })
                        .is_err()
                    {
                        return;
                    }
                }

                // 跑完/被取消都要送结束哨兵，否则前台轮询等不到收尾条件
                let _ = tx.send(RecognizeOutcome::Finished);
            })
            .detach();

        // ── 2. 前台：按节拍收结果、更新进度并重绘（状态栏「识别中 n/m」靠这里刷新） ──
        loop {
            async_cx
                .background_executor()
                .timer(Duration::from_millis(TICK_MS))
                .await;

            let keep_running = async_cx.update(|cx| {
                state_entity.update(cx, |state, cx| {
                    use RecognizeOutcome as Out;

                    let mut applied = 0usize;
                    let mut finished = false;
                    let mut fatal = None;

                    while let Ok(outcome) = rx.try_recv() {
                        match outcome {
                            Out::Done {
                                idx,
                                path,
                                filename,
                                result,
                            } => {
                                apply_recognition(state, idx, &path, &result);
                                state.recognize_done += 1;
                                state.recognize_current = filename;
                                applied += 1;
                            }
                            Out::Fatal(msg) => fatal = Some(msg),
                            Out::Finished => finished = true,
                        }
                    }

                    if let Some(msg) = fatal {
                        state.is_recognizing = false;
                        state.set_status_message(msg);
                        cx.notify();
                        return false;
                    }

                    // 跑完、被取消（状态栏 ✕ 置的 cancel 标志让 worker 提前退出）都走这里收尾
                    if finished || !state.is_recognizing {
                        let (done, total) = (state.recognize_done, state.recognize_total);
                        state.is_recognizing = false;
                        state.recompute_pipeline();
                        state.set_status_message(if done < total {
                            format!("识别已取消：完成 {done}/{total}")
                        } else {
                            format!("识别完成：{done} 张")
                        });
                        cx.notify();
                        return false;
                    }

                    if applied > 0 {
                        cx.notify();
                    }
                    true
                })
            });

            if !keep_running {
                break;
            }
        }
    })
    .detach();
}

/// 复制图片到系统剪贴板（全尺寸 RGBA）。
///
/// 对齐已删除的 Tauri 版 `copy_image_to_clipboard`：
/// - 常规格式（JPEG/PNG/WebP/BMP/GIF/TIFF）：直接读原文件全尺寸解码，不缩放、不重编码；
/// - RAW：走 `ThumbnailCache::get_or_generate_full`（AHD 全尺寸 JPEG）再解码；
/// - 原文件解不开（DNG/TIFF/HEIF 等）同样回退到 full 母版。
///
/// 用 `arboard` 而不是 GPUI 自带的剪贴板：gpui-pre 的 Linux 后端（X11/Wayland）
/// 只写文本，图片项会被静默丢弃。解码放后台 executor（39MP RGBA ≈157MB，不能冻 UI），
/// 写剪贴板回主线程。
pub fn copy_image_to_clipboard(
    state_entity: Entity<AppState>,
    manager: ImageManager,
    source: SourceFile,
    cx: &mut App,
) {
    // 注意：不能在这里 state_entity.read(cx)——本函数是从 AppState 的 listener 里
    // 调进来的，实体正被租借；manager 由调用方传入（与 load_preview_image 同口径）。
    let name = source
        .path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| source.path.to_string_lossy().to_string());

    cx.spawn(async move |async_cx| {
        let decoded = async_cx
            .background_executor()
            .spawn(async move { decode_full_rgba(&manager, &source) })
            .await;

        let _ = async_cx.update(|cx| {
            let _ = state_entity.update(cx, |state, cx| {
                match decoded {
                    Ok((w, h, rgba)) => {
                        let mb = rgba.len() as f64 / (1024.0 * 1024.0);
                        match write_image_to_system_clipboard(w, h, rgba) {
                            Ok(()) => state.set_status_message(format!(
                                "已复制图片到剪贴板：{name}（{w}×{h}，RGBA {mb:.1} MB）"
                            )),
                            Err(e) => state.set_status_message(format!("复制图片失败：{e}")),
                        }
                    }
                    Err(e) => state.set_status_message(format!("复制图片失败：{e}")),
                }
                cx.notify();
            });
        });
    })
    .detach();
}

/// 常驻的剪贴板持有者。
///
/// **X11 的剪贴板内容由进程持有**：`arboard::Clipboard` 一 drop，选择所有权就没了
/// （那份数据是应答 X 请求时才提供的），于是「复制」会静默变成什么都没复制。
/// 所以这里把实例留在静态里让它的服务线程活到进程退出——与多数 X11 应用的行为一致。
/// Wayland 的 data-control 由合成器接管，不受影响（但同一个静态复用即可）。
static CLIPBOARD_OWNER: std::sync::OnceLock<std::sync::Mutex<Option<arboard::Clipboard>>> =
    std::sync::OnceLock::new();

/// 把 RGBA8 像素写进系统剪贴板（独立成函数，便于单测打桩 / 将来换后端）。
fn write_image_to_system_clipboard(width: u32, height: u32, rgba: Vec<u8>) -> Result<(), String> {
    let slot = CLIPBOARD_OWNER.get_or_init(|| std::sync::Mutex::new(None));
    let mut guard = slot.lock().map_err(|_| "剪贴板状态锁失效".to_string())?;
    if guard.is_none() {
        *guard = Some(arboard::Clipboard::new().map_err(|e| e.to_string())?);
    }
    let clipboard = guard.as_mut().ok_or_else(|| "剪贴板初始化失败".to_string())?;
    clipboard
        .set_image(arboard::ImageData {
            width: width as usize,
            height: height as usize,
            bytes: std::borrow::Cow::Owned(rgba),
        })
        .map_err(|e| e.to_string())
}

/// 取全尺寸 RGBA：返回（宽、高、RGBA8 字节）。
fn decode_full_rgba(
    manager: &ImageManager,
    source: &SourceFile,
) -> Result<(u32, u32, Vec<u8>), String> {
    let is_raw = matches!(source.format, ImageFormat::Raw(_));
    let mut errors: Vec<String> = Vec::new();

    if !is_raw {
        match std::fs::read(&source.path) {
            Ok(bytes) => match decode_rgba(&bytes) {
                Ok(v) => return Ok(v),
                Err(e) => errors.push(format!("原文件解码失败：{e}")),
            },
            Err(e) => errors.push(format!("读取原文件失败：{e}")),
        }
    }

    // RAW，或原文件解不开（DNG/TIFF/HEIF 等）：用 full 变体（AHD 全尺寸 JPEG）
    match manager.load_full_image(source, None) {
        Ok(img) => match decode_rgba(&img.bytes) {
            Ok(v) => return Ok(v),
            Err(e) => errors.push(format!("全尺寸母版解码失败：{e}")),
        },
        Err(e) => errors.push(format!("全尺寸母版生成失败：{e}")),
    }

    Err(errors.join("；"))
}

fn decode_rgba(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let img = image::load_from_memory(bytes)
        .map_err(|e| e.to_string())?
        .to_rgba8();
    Ok((img.width(), img.height(), img.into_raw()))
}

#[cfg(test)]
mod clipboard_tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ftpt-clip-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        dir
    }

    /// 常规格式：全尺寸解码走「读原文件」这条路，且完全不需要缩略图缓存。
    #[test]
    fn decode_full_rgba_reads_original_file_for_regular_formats() {
        let dir = temp_dir("png");
        let path = dir.join("clip.png");
        image::RgbaImage::from_fn(3, 2, |x, y| image::Rgba([x as u8, y as u8, 0, 255]))
            .save(&path)
            .expect("写测试 PNG");

        let manager = ImageManager::new(None);
        let source = SourceFile {
            path: path.clone(),
            format: ImageFormat::Png,
            file_size: std::fs::metadata(&path).ok().map(|m| m.len()),
        };

        let (w, h, rgba) = decode_full_rgba(&manager, &source).expect("应从原文件解码");
        assert_eq!((w, h), (3, 2));
        assert_eq!(rgba.len(), 3 * 2 * 4);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 原文件坏 + 无缓存目录：两条路都失败时必须报错，而不是 panic 或返回空图。
    #[test]
    fn decode_full_rgba_reports_error_when_nothing_decodes() {
        let dir = temp_dir("broken");
        let path = dir.join("broken.jpg");
        std::fs::write(&path, b"not an image").expect("写坏文件");

        let manager = ImageManager::new(None);
        let source = SourceFile {
            path: path.clone(),
            format: ImageFormat::Jpeg,
            file_size: Some(12),
        };

        let err = decode_full_rgba(&manager, &source).expect_err("应报错");
        assert!(
            err.contains("解码失败") || err.contains("读取原文件失败"),
            "错误信息应说明失败原因：{err}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}

