//! 后端与引擎同步调用封装（在后台任务中执行，结果回传到 GPUI 实体）
//!
//! 对应 §7、§8 与附录 A。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use gpui_kit::{App, Context, Entity, Window};
use photo_domain::{
    AdjustParams, BBox, CaptureMeta, ColorLabel, FilterCriteria, Flag, ImageFormat, Rating,
    SourceFile,
};

use crate::image::{ImageManager, THUMB_SIZE_GRID, source_file_of};
use crate::model::adjust::rel_path_of;
use photo_engine::folder_db::{ExifCacheRow, FileEntry, FolderDb};
use photo_engine::global_db::{GlobalDb, SpeciesRow};
use photo_engine::scanner;

use super::app_state::{AppState, SharedRecognizer, StatsPhoto, ViewMode};

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
    let (generation, cancel_token, global_db) = state_entity.update(cx, |state, cx| {
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
        (state.scan_generation, new_cancel, state.global_db.clone())
    });

    let entity_clone = state_entity.clone();
    let dir_clone = dir.clone();

    cx.spawn(async move |async_cx| {
        let result = async_cx
            .background_executor()
            .spawn(async move {
                do_background_scan(&dir_clone, recursive, &cancel_token, global_db).await
            })
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

                        // 选中：统计页跳转优先（按 pending 路径定位），否则默认第一张
                        let pending = state.pending_select_path.take();
                        let target = pending
                            .as_ref()
                            .and_then(|p| state.items.iter().position(|m| m.primary_path == *p));
                        if let Some(idx) = target {
                            state.select_single(idx);
                        } else if let Some(&first) = state.display_order.first() {
                            state.select_single(first);
                        }
                    }
                    Err(e) => {
                        // 扫描失败别留悬挂的跳转目标，否则下次扫描完成后会莫名选中
                        state.pending_select_path = None;
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
    global_db: Option<GlobalDb>,
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

    // 填充已有识别记录，并顺带按 rel_path 收集 EXIF 拍摄时间（全局索引的 date_taken 用）
    let mut date_by_rel: HashMap<String, Option<String>> = HashMap::new();
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
                    enrich_meta_recognition(meta, r);
                }
                date_by_rel.insert(rel, meta.date_taken.clone());
            }

            // 全局索引同步：扫描完成 → 当前文件夹识别行全量替换（多主体按主体展开）。
            // 派生索引的写入是尽力而为（失败只影响统计页数据源，不阻塞主流程）。
            if let Some(gdb) = &global_db {
                let folder_str = dir.to_string_lossy().to_string();
                let mut rows: Vec<SpeciesRow> = Vec::new();
                let mut stale: Vec<String> = Vec::new();
                for (rel, rec) in &recs {
                    let date = date_by_rel.get(rel.as_str()).and_then(|d| d.clone());
                    let new_rows =
                        SpeciesRow::from_recognition(&folder_str, rel, rec, date.as_deref());
                    if new_rows.is_empty() {
                        // 识别表里有记录但无任何物种结论（Unrecognized / 全部主体失败）：
                        // replace_folder 的「磁盘存在就保留」会留着旧行，必须显式清掉
                        stale.push(rel.clone());
                    }
                    rows.extend(new_rows);
                }
                let _ = gdb.replace_folder(&folder_str, &rows);
                if !stale.is_empty() {
                    let _ = gdb.delete_rows(&folder_str, &stale);
                }
            }
        }
    }

    Ok((metas, folder_db))
}

/// EXIF 补提取完成后，把当前文件夹的拍摄日期回写全局索引（docs/todo.md #12 残留）。
///
/// 扫描完成时索引行的 `date_taken` 取的是**补提取之前**的元数据（首次扫描恒为 None），
/// 而 `species_stats()` 的 `first_date` / `last_date` 直接读它——不补写，一个目录的
/// 「首见/最近」日期会一直缺到下次重扫。这里只对本批补到日期的照片按 rel_path 增量
/// upsert（没有识别记录的照片本来就不在索引里，跳过；`upsert_rows` 是 REPLACE 语义，
/// 其余列原样带回）。返回写入行数，失败只记日志（派生索引是尽力而为）。
pub fn rewrite_folder_index_dates(
    folder_db: &FolderDb,
    global_db: &GlobalDb,
    folder: &Path,
    items: &[CaptureMeta],
) -> usize {
    let folder_str = folder.to_string_lossy().to_string();
    let mut rows: Vec<SpeciesRow> = Vec::new();
    for meta in items {
        let Some(date) = meta.date_taken.as_deref() else {
            continue;
        };
        let Some(rel) = rel_path_of(folder, Path::new(&meta.primary_path)) else {
            continue;
        };
        let Ok(Some(rec)) = folder_db.get_recognition(&rel) else {
            continue;
        };
        rows.extend(SpeciesRow::from_recognition(
            &folder_str,
            &rel,
            &rec,
            Some(date),
        ));
    }
    if rows.is_empty() {
        return 0;
    }
    match global_db.upsert_rows(&rows) {
        Ok(()) => rows.len(),
        Err(e) => {
            tracing::warn!("全局索引回写拍摄日期失败：{e}");
            0
        }
    }
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
                let mut enriched = 0usize;
                for (idx, path, meta) in enriched_results {
                    if let Some(item) = state.items.get_mut(idx) {
                        item.enrich_with_exif(&meta);
                        if let Some(db) = &state.folder_db {
                            let _ = db.put_exif(&path, &meta);
                        }
                        enriched += 1;
                    }
                }
                state.recompute_pipeline();
                // 索引行在扫描那一刻取的是补提取前的 date_taken（首次扫描恒为 None）→
                // 补到日期后重写一次，`species_stats()` 的首见/最近日期才是真的
                if enriched > 0
                    && let (Some(db), Some(gdb), Some(dir)) = (
                        state.folder_db.as_ref(),
                        state.global_db.as_ref(),
                        state.current_dir.clone(),
                    )
                {
                    let written = rewrite_folder_index_dates(db, gdb, &dir, &state.items);
                    if written > 0 {
                        tracing::debug!("全局索引回写拍摄日期：{written} 行");
                    }
                }
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
    let date_taken = if let Some(meta) = state.items.get_mut(idx) {
        enrich_meta_recognition(meta, recognition);
        meta.date_taken.clone()
    } else {
        None
    };
    // 识别结果写回文件夹级 data.db（连接不能跨线程，所以在前台写）
    if let (Some(db), Some(dir)) = (&state.folder_db, &state.current_dir) {
        if let Ok(rel) = path.strip_prefix(dir) {
            let rel_str = rel.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/");
            let _ = db.upsert_recognition(&rel_str, recognition);
            // 全局索引同步：单张识别完成 → 当前照片行 upsert（多主体按主体展开）；
            // 无任何结论（Unrecognized / 全部主体失败）则清掉旧索引行
            if let Some(gdb) = &state.global_db {
                let folder_str = dir.to_string_lossy().to_string();
                let rows = SpeciesRow::from_recognition(
                    &folder_str,
                    &rel_str,
                    recognition,
                    date_taken.as_deref(),
                );
                if rows.is_empty() {
                    let _ = gdb.delete_rows(&folder_str, &[rel_str.clone()]);
                } else {
                    let _ = gdb.upsert_rows(&rows);
                }
            }
        }
    }
}

/// 状态栏/日志里的识别器名（BioCLIP 是唯一后端）
const BACKEND_LABEL: &str = "BioCLIP（全物种）";

/// 确保常驻识别器已装配（**在后台线程调用**；已装配则直接返回）。
///
/// 模型装配（org_det + BioCLIP int8 + 名录子集，约 1-2s）只做一次：批量识别与框选
/// 识别共用同一个实例，避免每次识别都重装。失败返回面向用户的错误文案。
fn ensure_recognizer(
    slot: &SharedRecognizer,
    models_dir: &Path,
    catalog_db: &Path,
) -> Result<(), String> {
    let mut guard = slot.lock();
    if guard.is_none() {
        match photo_recognize::Recognizer::new(models_dir, catalog_db) {
            Ok(recognizer) => {
                tracing::info!(
                    "识别器装配完成（常驻缓存）：后端 {}（{}），资产版本 {:?}",
                    recognizer.classifier_backend(),
                    BACKEND_LABEL,
                    recognizer.asset_version()
                );
                *guard = Some(recognizer);
                Ok(())
            }
            // RecognizeError::ModelLoad 的文案已含「缺哪个文件」，直接透传
            Err(e) => {
                let msg = format!("初始化识别模型失败（{BACKEND_LABEL}）：{e}");
                tracing::warn!("{msg}");
                Err(msg)
            }
        }
    } else {
        Ok(())
    }
}

// ── 统计页（全局物种统计 → 照片记录） ──

/// 统计页右栏照片列表的展示上限：避免一次解析上千张的 fs metadata。
/// 超过时 stats_photo_total 仍记真实总数，UI 提示「仅显示前 N 张」。
const STATS_PHOTO_LIMIT: usize = 240;

/// 选中统计页左栏某物种：从全局索引查它的照片记录，并解析各自文件夹的缩略图缓存路径。
///
/// 同步执行（前台）：一次全局索引查询 + 至多 STATS_PHOTO_LIMIT 次 metadata/哈希——
/// 目录已在扫描时生成缩略图，这里只做「是否已就绪」的定位，不解码、不生成。
pub fn select_stats_species(state: &mut AppState, species: &str) {
    let Some(gdb) = state.global_db.clone() else {
        state.stats_selected_species = Some(species.to_string());
        state.stats_photos.clear();
        state.stats_photo_total = 0;
        return;
    };
    let all = gdb.photos_of_species(species).unwrap_or_default();
    let total = all.len();
    // 缩略图缓存目录按文件夹隔离：按文件夹复用一个只读 ImageManager
    let mut per_folder: HashMap<String, ImageManager> = HashMap::new();
    let photos: Vec<StatsPhoto> = all
        .into_iter()
        .take(STATS_PHOTO_LIMIT)
        .map(|(folder, rel_path)| {
            let thumb_path =
                crate::image::cached_thumb_path_in_folder(&mut per_folder, &folder, &rel_path);
            let full_path = Path::new(&folder).join(&rel_path);
            StatsPhoto {
                full_path,
                thumb_path,
                folder,
                rel_path,
            }
        })
        .collect();
    state.stats_selected_species = Some(species.to_string());
    state.stats_photos = photos;
    state.stats_photo_total = total;
}

/// 统计页右栏点击某张照片：跳到它所在文件夹并选中（跨文件夹时先扫描）。
///
/// 跨文件夹不能直接改 items：先记 pending_select_path，扫完由 `start_scan` 完成后按路径定位。
pub fn open_stats_photo(
    state_entity: Entity<AppState>,
    folder: String,
    rel_path: String,
    cx: &mut App,
) {
    let full = Path::new(&folder).join(&rel_path);
    let same_dir = state_entity
        .read(cx)
        .current_dir
        .as_deref()
        .is_some_and(|d| d == Path::new(&folder));

    if same_dir {
        state_entity.update(cx, |state, cx| {
            if let Some(idx) = state
                .items
                .iter()
                .position(|m| Path::new(&m.primary_path) == full)
            {
                state.select_single(idx);
            } else {
                state.set_status_message(format!("当前目录里找不到该照片：{rel_path}"));
            }
            state.view_mode = ViewMode::Grid;
            cx.notify();
        });
    } else {
        state_entity.update(cx, |state, cx| {
            state.pending_select_path = Some(full.to_string_lossy().to_string());
            state.view_mode = ViewMode::Grid;
            cx.notify();
        });
        start_scan(state_entity, PathBuf::from(&folder), false, cx);
    }
}

/// 预览手动框选识别（漏检补充）：对用户框选的区域跑识别（跳过 YOLO 检测），
/// 结果作为**新主体**追加进该照片已有的 subjects，落 folder_db + 全局索引。
///
/// 路径：预览工具条「框选」toggle 开启后，拖拽画框、松开即触发（见 preview.rs）。
/// 重型推理放后台 executor（与批量识别同一口径），前台只做合并与落库。
pub fn start_region_recognition(
    state_entity: Entity<AppState>,
    path: PathBuf,
    bbox: BBox,
    cx: &mut App,
) {
    // 防重入：同一时刻只跑一次框选识别；顺带取出常驻识别器槽位
    let Some(shared) = state_entity.update(cx, |state, cx| {
        if state.region_recognizing {
            return None;
        }
        state.region_recognizing = true;
        state.set_status_message("框选识别中…".to_string());
        cx.notify();
        Some(state.recognizer.clone())
    }) else {
        return;
    };

    let Some(data_root) = crate::state::app_state::data_root() else {
        state_entity.update(cx, |state, cx| {
            state.region_recognizing = false;
            state.set_status_message("无法定位模型与名录库数据目录 (PHOTO_DATA_DIR)");
            cx.notify();
        });
        return;
    };
    let models_dir = data_root.join("models");
    let catalog_db = data_root.join("data/bird_catalog.db");

    cx.spawn(async move |async_cx: &mut gpui_kit::AsyncApp| {
        // 后台：模型装配 + 区域识别（CPU 密集，绝不放前台执行器）
        let path_bg = path.clone();
        let result: Result<photo_domain::Recognition, String> = async_cx
            .background_executor()
            .spawn(async move {
                // 常驻识别器：首次调用装配（~1-2s），之后复用（批量/框选共用）
                ensure_recognizer(&shared, &models_dir, &catalog_db)?;
                let format = photo_domain::ImageFormat::from_extension(
                    path_bg
                        .extension()
                        .unwrap_or_default()
                        .to_str()
                        .unwrap_or_default(),
                )
                .unwrap_or(photo_domain::ImageFormat::Jpeg);
                let capture = photo_domain::Capture {
                    base_name: path_bg
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string(),
                    source_files: vec![photo_domain::SourceFile {
                        path: path_bg.clone(),
                        format,
                        file_size: None,
                    }],
                    primary_index: 0,
                };
                // 推理期间持锁：与批量识别互斥（同一个 ONNX session 不做并发推理）
                let mut guard = shared.lock();
                let Some(recognizer) = guard.as_mut() else {
                    return Err("识别器未就绪".to_string());
                };
                recognizer
                    .recognize_region(&capture, bbox, None)
                    .map_err(|e| e.to_string())
            })
            .await;

        // 前台：合并主体 + 落库 + 摘要 + 全局索引同步
        let _ = async_cx.update(|cx| {
            state_entity.update(cx, |state, cx| {
                state.region_recognizing = false;
                match result {
                    Err(e) => state.set_status_message(format!("框选识别失败：{e}")),
                    Ok(rec) => apply_region_recognition(state, &path, rec),
                }
                cx.notify();
            });
        });
    })
    .detach();
}

/// 把框选识别结果合并进该照片已有识别（**只在前台线程调用**）。
///
/// - 框选主体无物种结论（分类/名录失败）→ 不追加，仅状态栏提示
/// - 已有识别记录 → append_subject（旧数据先把顶层物化为「主主体」再追加）
/// - 无识别记录 → 框选结果即该照片的识别结论
fn apply_region_recognition(
    state: &mut AppState,
    path: &Path,
    region_rec: photo_domain::Recognition,
) {
    let subject = match region_rec.subjects.first() {
        Some(s) if s.taxon.is_some() => s.clone(),
        Some(s) => {
            let msg = s.failure.user_message();
            state.set_status_message(format!(
                "框选识别未得到物种结论{}",
                if msg.is_empty() {
                    String::new()
                } else {
                    format!("（{msg}）")
                }
            ));
            return;
        }
        None => {
            state.set_status_message("框选识别未得到物种结论".to_string());
            return;
        }
    };
    let display = subject
        .taxon
        .as_ref()
        .map(|t| t.display_name().to_string())
        .unwrap_or_default();

    // 先 clone 出数据库句柄与目录，避免与后面对 state.items 的可变借用冲突
    let dir = state.current_dir.clone();
    let db = state.folder_db.clone();
    let gdb = state.global_db.clone();
    let (Some(dir), Some(db)) = (dir, db) else {
        state.set_status_message("框选识别完成，但当前没有打开的文件夹数据库".to_string());
        return;
    };
    let Ok(rel) = path.strip_prefix(&dir) else {
        state.set_status_message("框选识别完成，但无法定位文件的相对路径".to_string());
        return;
    };
    let rel_str = rel.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/");

    // 合并：已有记录 → 追加主体；无记录 → 框选结果直接用
    let merged = match db.get_recognition(&rel_str).ok().flatten() {
        Some(existing) => existing.append_subject(subject),
        None => region_rec,
    };
    let _ = db.upsert_recognition(&rel_str, &merged);

    // 内存摘要回填（网格/信息栏立即可见），并取拍摄时间供全局索引
    let idx = state
        .items
        .iter()
        .position(|m| Path::new(&m.primary_path) == path);
    let date_taken = if let Some(i) = idx {
        if let Some(meta) = state.items.get_mut(i) {
            enrich_meta_recognition(meta, &merged);
            meta.date_taken.clone()
        } else {
            None
        }
    } else {
        None
    };

    // 全局索引同步（多主体按主体展开）
    if let Some(gdb) = &gdb {
        let folder_str = dir.to_string_lossy().to_string();
        let rows =
            SpeciesRow::from_recognition(&folder_str, &rel_str, &merged, date_taken.as_deref());
        if rows.is_empty() {
            let _ = gdb.delete_rows(&folder_str, &[rel_str.clone()]);
        } else {
            let _ = gdb.upsert_rows(&rows);
        }
    }

    state.set_status_message(format!(
        "框选识别：{display}（已作为新主体加入，共 {} 个主体）",
        merged.subjects.len()
    ));
}

/// 把识别结果写进 CaptureMeta 摘要，并把物种名归一为「显示名」。
///
/// `CaptureMeta::enrich_with_recognition` 取的是 taxon.cn_name，而 BioCLIP 的标签
/// 是学名、大多数预测根本不在本地名录里——此时 taxon_id = None、cn_name 为空串，
/// 直接落摘要就会在网格/信息栏/幻灯片里显示成空白。摘要里没有学名字段，所以这里
/// 统一改写成 TaxonMatch::display_name()（有中文名用中文名，没有用学名）。
fn enrich_meta_recognition(meta: &mut CaptureMeta, recognition: &photo_domain::Recognition) {
    meta.enrich_with_recognition(recognition);
    if let Some(taxon) = &recognition.taxon {
        let display = taxon.display_name();
        meta.taxon_name = (!display.is_empty()).then(|| display.to_string());
    }
}

/// 启动物种识别管线（§5.6）
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

        let items = targets
            .into_iter()
            .filter_map(|i| {
                state
                    .items
                    .get(i)
                    .map(|m| (i, PathBuf::from(&m.primary_path)))
            })
            .collect();
        items
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
    // 常驻识别器槽位：克隆进后台线程（批量与框选共用同一实例，装配一次）
    let shared_recognizer = state_entity.read(cx).recognizer.clone();

    // 后台 worker -> 前台的结果队列（前台 try_recv 非阻塞，节拍到了就收）
    let (tx, rx) = std::sync::mpsc::channel::<RecognizeOutcome>();

    cx.spawn(async move |async_cx: &mut gpui_kit::AsyncApp| {
        // ── 1. 后台线程：模型加载 + 逐张推理（CPU 密集，绝不放前台执行器） ──
        async_cx
            .background_executor()
            .spawn(async move {
                // 常驻识别器：首次调用装配（~1-2s），之后复用（批量/框选共用）
                if let Err(msg) = ensure_recognizer(&shared_recognizer, &models_dir, &catalog_db) {
                    let _ = tx.send(RecognizeOutcome::Fatal(msg));
                    let _ = tx.send(RecognizeOutcome::Finished);
                    return;
                }

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
                    // 锁只在单张推理期间持有（不整批持有）：框选识别可在两张之间插进来
                    let result = {
                        let mut guard = shared_recognizer.lock();
                        match guard.as_mut() {
                            Some(recognizer) => recognizer.recognize(&capture, None, None),
                            None => Err(photo_recognize::RecognizeError::ModelLoad(
                                "识别器未就绪".to_string(),
                            )),
                        }
                    };
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

/// 一个导出作业：源文件 + 已解析的调整参数 + 已排定的输出路径。
struct ExportJob {
    filename: String,
    source: SourceFile,
    params: AdjustParams,
    output: PathBuf,
}

/// 导出结果（后台 worker → 前台）。
enum ExportOutcome {
    /// 单张完成：成功给出输出路径，失败给出真实错误文案
    Done {
        filename: String,
        result: Result<String, String>,
    },
    /// 跑完 / 被取消都要送的收尾哨兵
    Finished,
}

/// 批量导出（§9.10 / §10.6）：把当前目标集按草稿参数导出为 JPEG。
///
/// 与识别管线同构（CPU 密集不进前台执行器，否则进度条与「取消」都点不动）：
/// 1. **前台**取快照——钳制草稿、建目标目录、算目标集（选中 ∩ 筛选结果 / 无选中 = 全量）、
///    按模板渲染基名并逐批去重、读每张的调整参数（导出烘焙调整，ADR 0007）；
/// 2. **后台线程**逐张 `convert::export_with_preset`（RAW 走全尺寸 16-bit 解码，单张 3–5s）；
/// 3. **前台**每 250ms 收结果，推进 `export_done` 并 `cx.notify()`，收尾写状态栏文案。
///
/// 每个文件的失败原因记进 `state.export_results`（对话框展示真实错误）——
/// 这条链路以前只报「已开始导出照片」就关窗，**不再谎报成功**。
pub fn start_export(state_entity: Entity<AppState>, cx: &mut App) {
    /// 前台收结果的节拍（与识别 / 缩略图管线同量级）
    const TICK_MS: u64 = 250;

    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_flag = cancel.clone();
    let draft = state_entity.read(cx).export.clone().clamped();

    let jobs: Vec<ExportJob> = state_entity.update(cx, |state, cx| {
        if state.is_exporting {
            return Vec::new();
        }
        // 钳制后的草稿写回（越界质量 / 空模板不带到引擎）
        state.export = draft.clone();
        if !draft.is_ready() {
            state.set_status_message("导出失败：未选择目标目录");
            cx.notify();
            return Vec::new();
        }
        let dest = PathBuf::from(draft.dest_dir.trim());
        if let Err(e) = std::fs::create_dir_all(&dest) {
            state.set_status_message(format!(
                "导出失败：无法创建目标目录 {}（{e}）",
                dest.display()
            ));
            cx.notify();
            return Vec::new();
        }

        let targets =
            crate::model::export::export_targets(&state.selected_indices, &state.display_order);
        if targets.is_empty() {
            state.set_status_message("导出失败：没有可导出的照片");
            cx.notify();
            return Vec::new();
        }

        // 目标集快照：模板渲染（{seq} 从 1 起）+ 输出路径去重 + 调整参数
        let mut used = std::collections::HashSet::new();
        let mut jobs: Vec<ExportJob> = Vec::with_capacity(targets.len());
        for (n, idx) in targets.iter().enumerate() {
            let Some(meta) = state.items.get(*idx) else {
                continue;
            };
            let Some(source) = source_file_of(meta) else {
                continue;
            };
            let ctx = photo_engine::template::NameTemplateContext {
                name: meta.base_name.clone(),
                species: meta.taxon_name.clone(),
                date: meta.date_taken.clone(),
                camera: meta.camera_model.clone(),
                seq: (n + 1) as u32,
            };
            let base = photo_engine::template::render_name_template(&draft.template, &ctx);
            let output = crate::model::export::unique_output_path(&dest, &base, "jpg", &mut used);
            jobs.push(ExportJob {
                filename: meta.display_name(),
                source,
                params: load_adjustments(state, &meta.primary_path),
                output,
            });
        }

        state.is_exporting = true;
        state.export_cancel = cancel.clone();
        state.export_done = 0;
        state.export_total = jobs.len() as u32;
        state.export_current.clear();
        state.export_results.clear();
        // 目标目录记忆（#14 导出侧）：与配置同一份，落盘失败不影响导出
        state.app_config.export_dir = Some(draft.dest_dir.trim().to_string());
        state.save_config();
        cx.notify();
        jobs
    });

    if jobs.is_empty() {
        state_entity.update(cx, |state, cx| {
            state.is_exporting = false;
            cx.notify();
        });
        return;
    }

    // 后台 worker → 前台的结果队列（前台 try_recv 非阻塞，节拍到了就收）
    let (tx, rx) = std::sync::mpsc::channel::<ExportOutcome>();

    cx.spawn(async move |async_cx: &mut gpui_kit::AsyncApp| {
        // ── 1. 后台线程：逐张烘焙 + 写盘（RAW 全尺寸解码，绝不放前台执行器） ──
        async_cx
            .background_executor()
            .spawn(async move {
                for job in jobs {
                    if cancel_flag.load(Ordering::Relaxed) {
                        break;
                    }
                    let result = photo_engine::convert::export_with_preset(
                        &job.source,
                        &job.params,
                        draft.long_edge,
                        draft.quality,
                        &job.output,
                    )
                    .map(|p| p.to_string_lossy().to_string())
                    .map_err(|e| e.to_string());

                    if tx
                        .send(ExportOutcome::Done {
                            filename: job.filename,
                            result,
                        })
                        .is_err()
                    {
                        // 前台已经走了（实体回收）→ 直接收工
                        return;
                    }
                }
                // 跑完 / 被取消都要送结束哨兵，否则前台轮询等不到收尾条件
                let _ = tx.send(ExportOutcome::Finished);
            })
            .detach();

        // ── 2. 前台：按节拍收结果、更新进度并重绘（状态栏「导出中 n/m」靠这里刷新） ──
        loop {
            async_cx
                .background_executor()
                .timer(Duration::from_millis(TICK_MS))
                .await;

            let keep_running = async_cx.update(|cx| {
                state_entity.update(cx, |state, cx| {
                    let mut applied = 0usize;
                    let mut finished = false;

                    while let Ok(outcome) = rx.try_recv() {
                        match outcome {
                            ExportOutcome::Done { filename, result } => {
                                state.export_done += 1;
                                state.export_current = filename.clone();
                                state.export_results.push((filename, result));
                                applied += 1;
                            }
                            ExportOutcome::Finished => finished = true,
                        }
                    }

                    // 跑完、被取消（对话框「取消」置的 cancel 标志让 worker 提前退出）都走这里收尾
                    if finished || !state.is_exporting {
                        let done = state.export_done as usize;
                        let total = state.export_total as usize;
                        let failed = state
                            .export_results
                            .iter()
                            .filter(|(_, r)| r.is_err())
                            .count();
                        state.is_exporting = false;
                        state.set_status_message(if done < total {
                            format!("导出已取消：完成 {done}/{total}（失败 {failed}）")
                        } else if failed > 0 {
                            format!(
                                "导出完成：成功 {} 张，失败 {failed} 张（详见导出面板）",
                                done - failed
                            )
                        } else {
                            format!("导出完成：{done} 张")
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

/// 选择导出目标目录（系统目录对话框）→ 回填草稿与输入框。
pub fn pick_export_dest(window: &mut Window, cx: &mut Context<AppState>) {
    cx.spawn_in(window, async move |weak, async_cx| {
        let Some(folder) = rfd::AsyncFileDialog::new().pick_folder().await else {
            return;
        };
        let text = folder.path().to_string_lossy().to_string();
        let _ = async_cx.update(|window, cx| {
            let Some(entity) = weak.upgrade() else {
                return;
            };
            entity.update(cx, |state, cx| {
                state.export.dest_dir = text.clone();
                if let Some(input) = state.export_dest_input.clone() {
                    input.update(cx, |input_state, cx| {
                        input_state.set_value(text.clone(), window, cx);
                    });
                }
                cx.notify();
            });
        });
    })
    .detach();
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

#[cfg(test)]
mod recognition_tests {
    use super::*;
    use photo_domain::{CnLevel, RecognitionFailureStage, RecognitionStatus, TaxonMatch};

    /// 造一张最小识别结果（字段全部与 domain::Recognition 对齐）
    fn recognition(taxon: Option<TaxonMatch>) -> photo_domain::Recognition {
        photo_domain::Recognition {
            status: RecognitionStatus::Confirmed,
            taxon,
            class_index: Some(7),
            confidence: Some(88.5),
            bbox: None,
            candidates: Vec::new(),
            failure_stage: RecognitionFailureStage::None,
            recognized_at: "2026-01-01T00:00:00Z".to_string(),
            subjects: Vec::new(),
        }
    }

    fn empty_meta() -> CaptureMeta {
        let capture = photo_domain::Capture {
            base_name: "IMG_0001".to_string(),
            source_files: Vec::new(),
            primary_index: 0,
        };
        CaptureMeta::from_capture(&capture, 0)
    }

    /// 造一个临时工作区：照片目录（folder_db）+ 数据目录（global_db），互不干扰
    fn temp_dbs(tag: &str) -> (PathBuf, FolderDb, GlobalDb) {
        let base = std::env::temp_dir().join(format!("ftpt-index-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let photo_dir = base.join("photos");
        std::fs::create_dir_all(&photo_dir).expect("建照片目录");
        let db = FolderDb::open_in_dir(&photo_dir).expect("开 folder_db");
        let gdb = GlobalDb::open(&base.join("data")).expect("开 global_db");
        (photo_dir, db, gdb)
    }

    fn crow() -> photo_domain::Recognition {
        recognition(Some(TaxonMatch {
            taxon_id: Some(42),
            cn_name: "大嘴乌鸦".to_string(),
            latin_name: "Corvus macrorhynchos".to_string(),
            cn_level: CnLevel::Species,
            ranks: vec![],
        }))
    }

    #[test]
    fn test_rewrite_folder_index_dates_backfills_first_date() {
        let (photo_dir, db, gdb) = temp_dbs("dates");
        let rec = crow();
        db.upsert_recognition("a.jpg", &rec).expect("写识别行");
        let folder = photo_dir.to_string_lossy().to_string();
        // 扫描那一刻：EXIF 还没提取，date_taken = None
        gdb.upsert_rows(&SpeciesRow::from_recognition(&folder, "a.jpg", &rec, None))
            .expect("写索引行");
        assert_eq!(
            gdb.species_stats().expect("统计")[0].first_date,
            None,
            "扫描时索引没有日期"
        );

        // 补提取完成后 items 带回日期 → 回写
        let mut meta = empty_meta();
        meta.primary_path = photo_dir.join("a.jpg").to_string_lossy().to_string();
        meta.date_taken = Some("2024:05:12 10:30:00".to_string());
        let written = rewrite_folder_index_dates(&db, &gdb, &photo_dir, &[meta]);
        assert_eq!(written, 1, "命中一条有识别记录的照片");

        let stats = gdb.species_stats().expect("统计");
        let stat = stats
            .iter()
            .find(|s| s.species_name == "大嘴乌鸦")
            .expect("物种行");
        assert_eq!(
            stat.first_date.as_deref(),
            Some("2024:05:12 10:30:00"),
            "首见日期被回写"
        );
        assert_eq!(stat.first_date, stat.last_date, "只有一条记录");
    }

    #[test]
    fn test_rewrite_folder_index_dates_skips_without_recognition() {
        let (photo_dir, db, gdb) = temp_dbs("nodate");
        let mut meta = empty_meta();
        meta.primary_path = photo_dir.join("b.jpg").to_string_lossy().to_string();
        meta.date_taken = Some("2024-01-01".to_string());
        // 没有识别记录 → 不新建索引行（回写是「更新」而不是「补录」）
        assert_eq!(rewrite_folder_index_dates(&db, &gdb, &photo_dir, &[meta]), 0);
        assert!(gdb.species_stats().expect("统计").is_empty());
    }

    #[test]
    fn test_enrich_meta_recognition_falls_back_to_latin_name() {
        // BioCLIP 常见情形：预测不在名录（taxon_id None、cn_name 空），摘要要落学名
        let mut meta = empty_meta();
        let r = recognition(Some(TaxonMatch {
            taxon_id: None,
            cn_name: String::new(),
            latin_name: "Corvus corax".to_string(),
            cn_level: CnLevel::Missing,
            ranks: vec![],
        }));
        enrich_meta_recognition(&mut meta, &r);
        assert_eq!(meta.taxon_name.as_deref(), Some("Corvus corax"));
        assert_eq!(meta.taxon_confidence, Some(88.5));
        assert_eq!(meta.recognition_status, Some(RecognitionStatus::Confirmed));
    }

    #[test]
    fn test_enrich_meta_recognition_prefers_cn_name() {
        let mut meta = empty_meta();
        let r = recognition(Some(TaxonMatch {
            taxon_id: Some(42),
            cn_name: "大嘴乌鸦".to_string(),
            latin_name: "Corvus macrorhynchos".to_string(),
            cn_level: CnLevel::Species,
            ranks: vec![],
        }));
        enrich_meta_recognition(&mut meta, &r);
        assert_eq!(meta.taxon_name.as_deref(), Some("大嘴乌鸦"));
    }

    #[test]
    fn test_enrich_meta_recognition_without_taxon_keeps_name_empty() {
        let mut meta = empty_meta();
        let r = recognition(None);
        enrich_meta_recognition(&mut meta, &r);
        assert_eq!(meta.taxon_name, None);
    }

}

