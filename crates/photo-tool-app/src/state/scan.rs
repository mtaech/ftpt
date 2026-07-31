use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use gpui::*;

use photo_domain::CaptureMeta;
use photo_engine::{folder_db::FolderDb, scanner};

use super::app::RootView;

// 目录扫描、EXIF 提取（自 state/app.rs 拆出，纯移动，无逻辑改动）

impl RootView {
    /// 弹出目录选择对话框并扫描选中目录。
    /// rfd 对话框是阻塞式模态窗口，直接放在事件处理器里会因嵌套消息循环
    /// 触发 GPUI 的 RefCell 重入借用，所以放到 worker 线程执行。
    pub fn pick_and_scan_directory(&mut self, cx: &mut Context<Self>) {
        self.worker.spawn(
            cx,
            move || rfd::FileDialog::new().pick_folder(),
            move |this, result, cx| {
                if let Some(path) = result {
                    this.scan_directory(path, cx);
                }
            },
        );
    }

    pub fn scan_directory(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        // 换代：使在途的扫描/加载/回填结果全部作废（旧的 on_done 按代数丢弃），
        // 并清空加载哨兵与取消令牌——旧索引在换目录后对新 captures 无意义。
        self.scan_generation += 1;
        let generation = self.scan_generation;
        self.scan_in_progress = true;
        self.grid_loading.clear();
        self.preview_loading.clear();
        self.preview_cancel.clear();
        self.fullres_loading.clear();
        // 在照片目录中打开中心数据库（.pt/data.db）
        let folder_db = FolderDb::open_in_dir(&path)
            .ok();
        let filter = self.filter.clone();

        self.worker.spawn(
            cx,
            move || {
                scanner::scan_directory(&path, &filter, None)
                    .map(|captures| {
                        // 供扫描完成后同步 folder_db 的文件清单（全部非旁车源文件）
                        let entries: Vec<photo_engine::folder_db::FileEntry> = captures
                            .iter()
                            .flat_map(|c| c.source_files.iter())
                            .filter_map(|f| {
                                let rel = f.path.strip_prefix(&path).ok()?;
                                let m = std::fs::metadata(&f.path).ok()?;
                                let mtime_ns = m
                                    .modified()
                                    .ok()
                                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                                    .map(|d| d.as_nanos() as i64)
                                    .unwrap_or(0);
                                Some(photo_engine::folder_db::FileEntry {
                                    full_path: f.path.clone(),
                                    rel_path: rel.to_string_lossy().replace('\\', "/"),
                                    file_size: m.len(),
                                    mtime_ns,
                                    format: f.format.clone(),
                                })
                            })
                            .collect();
                        let metas: Vec<CaptureMeta> = captures
                            .iter()
                            .enumerate()
                            .map(|(i, c)| {
                                let mut meta = CaptureMeta::from_capture(c, i);
                                if let Some(cache) = &folder_db {
                                    let primary = &c.source_files[c.primary_index];
                                    let xmp = cache
                                        .get_xmp(&primary.path)
                                        .unwrap_or(None)
                                        .unwrap_or_default();
                                    meta.rating = xmp.rating();
                                    meta.color_label = xmp.color_label();
                                    meta.flag = xmp.flag();
                                }
                                if let Some(cache) = &folder_db {
                                    let primary = &c.source_files[c.primary_index];
                                    // 只查缓存：未命中的 EXIF 由扫描完成后的 spawn_enrich_tasks 并发提取，
                                    // 不再在扫描闭包内串行 LibRaw open（首次打开大目录会拖慢扫描完成）
                                    let exif = cache
                                        .get_exif(&primary.path)
                                        .unwrap_or(None)
                                        .unwrap_or_default();
                                    meta.enrich_with_exif(&exif);
                                }
                                meta
                            })
                            .collect();
                        (path, metas, folder_db, entries)
                    })
            },
            move |this, result, cx| {
                // 过期扫描：直接丢弃，避免旧结果覆盖新状态
                if generation != this.scan_generation {
                    return;
                }
                this.scan_in_progress = false;
                match result {
                    Ok((dir, metas, cache, entries)) => {
                        // 缓存跟随文件夹：每目录独立 .pt/thumbs（与 .pt/data.db 同级），
                        // 删除文件夹即清空缓存，无需全局上限/淘汰
                        this.thumbnail_cache = Some(photo_engine::thumbnail::ThumbnailCache::new(
                            dir.join(".pt").join("thumbs"),
                        ));
                        // 增量保留内存缓存：按 primary_path 映射到新 capture 索引，
                        // 仅丢弃消失文件的项（批量操作后重扫不重载未变化文件的缩略图/预览/全分辨率）
                        let old_captures = std::mem::take(&mut this.captures);
                        let old_thumbs = std::mem::take(&mut this.thumbnail_data);
                        let old_previews = std::mem::take(&mut this.preview_data);
                        let old_fullres = std::mem::take(&mut this.fullres_data);
                        let old_preview_order = std::mem::take(&mut this.preview_order);
                        let old_fullres_order = std::mem::take(&mut this.fullres_order);
                        let mut path_to_new: HashMap<PathBuf, usize> = HashMap::new();
                        for m in &metas {
                            path_to_new.insert(PathBuf::from(&m.primary_path), m.index);
                        }
                        let path_of = |old_idx: usize| {
                            old_captures
                                .get(old_idx)
                                .map(|m| PathBuf::from(&m.primary_path))
                        };
                        for (old_idx, img) in old_thumbs {
                            if let Some(path) = path_of(old_idx)
                                && let Some(&new_idx) = path_to_new.get(&path)
                            {
                                this.thumbnail_data.insert(new_idx, img);
                            }
                        }
                        for (old_idx, img) in old_previews {
                            if let Some(path) = path_of(old_idx)
                                && let Some(&new_idx) = path_to_new.get(&path)
                            {
                                this.preview_data.insert(new_idx, img);
                            }
                        }
                        for (old_idx, img) in old_fullres {
                            if let Some(path) = path_of(old_idx)
                                && let Some(&new_idx) = path_to_new.get(&path)
                            {
                                this.fullres_data.insert(new_idx, img);
                            }
                        }
                        // FIFO 顺序按保留项过滤（近似 LRU，保序）
                        for idx in old_preview_order {
                            if this.preview_data.contains_key(&idx) {
                                this.preview_order.push_back(idx);
                            }
                        }
                        for idx in old_fullres_order {
                            if this.fullres_data.contains_key(&idx) {
                                this.fullres_order.push_back(idx);
                            }
                        }
                        this.captures = metas;
                        this.folder_db = cache;
                        this.dir_path = Some(dir.clone());
                        // 记住最后打开的目录，下次启动自动恢复
                        let dir_str = dir.to_string_lossy().to_string();
                        this.config.last_directory = Some(dir_str.clone());
                        // 记录最近打开的目录（去重、最新在前、最多 10 个）
                        this.config.recent_directories.retain(|d| d != &dir_str);
                        this.config.recent_directories.insert(0, dir_str);
                        this.config.recent_directories.truncate(10);
                        this.save_config();
                        this.apply_filter_and_sort();
                        // 扫描完成后，用 folder_db 中已有的识别记录 enrich CaptureMeta
                        if let Some(ref db) = this.folder_db {
                            if let Ok(recs) = db.all_recognitions() {
                                for meta in this.captures.iter_mut() {
                                    // rel_path = primary_path 相对 dir 的路径，正斜杠
                                    let primary_path = std::path::Path::new(&meta.primary_path);
                                    if let Ok(rel) = primary_path.strip_prefix(&dir) {
                                        let rel_str = rel.to_string_lossy().replace('\\', "/");
                                        if let Some(rec) = recs.iter().find(|(p, _)| *p == rel_str) {
                                            meta.enrich_with_recognition(&rec.1);
                                        }
                                    }
                                }
                            }
                        }
                        this.apply_filter_and_sort();
                        // 鸟种列表供筛选栏多选下拉使用
                        this.refresh_bird_options();
                        tracing::info!(
                            "扫描完成：{} 找到 {} 个 capture，过滤后 {} 个",
                            dir.display(),
                            this.captures.len(),
                            this.display_order.len()
                        );
                        // 后台逐步提取 EXIF（RAW 文件的 LibRaw unpack 较慢）
                        this.spawn_enrich_tasks(cx);
                        this.preload_thumbnails(cx);
                        // 同步 folder_db 三表：删多余行、新/改文件入库、清识别孤儿行
                        if let Some(db) = this.folder_db.clone() {
                            if !entries.is_empty() {
                                let total = entries.len();
                                this.sync_progress = Some((0, total));
                                let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));
                                let counter_work = counter.clone();
                                this.worker.spawn(
                                    cx,
                                    move || {
                                        db.sync_with_scan(&entries, &|done, _| {
                                            counter_work
                                                .store(done, std::sync::atomic::Ordering::Relaxed);
                                        })
                                    },
                                    |this, result, cx| {
                                        this.sync_progress = None;
                                        match result {
                                            Ok(stats) => {
                                                tracing::info!(
                                                    "DB 同步完成：清理缓存 {} / 识别 {}，更新 {}，失败 {}",
                                                    stats.cache_deleted,
                                                    stats.recognition_deleted,
                                                    stats.cache_updated,
                                                    stats.cache_failed
                                                );
                                                if stats.cache_deleted
                                                    + stats.recognition_deleted
                                                    + stats.cache_updated
                                                    > 0
                                                {
                                                    this.show_toast(
                                                        format!(
                                                            "同步完成：清理 {} 条缓存、{} 条识别记录，更新 {} 条",
                                                            stats.cache_deleted,
                                                            stats.recognition_deleted,
                                                            stats.cache_updated
                                                        ),
                                                        cx,
                                                    );
                                                }
                                            }
                                            Err(e) => tracing::error!("DB 同步失败: {e}"),
                                        }
                                        cx.notify();
                                    },
                                );
                                // 轮询进度计数器，刷新状态栏 done/total
                                let counter_poll = counter.clone();
                                cx.spawn(|weak: WeakEntity<RootView>, cx: &mut AsyncApp| {
                                    let mut cx = cx.clone();
                                    async move {
                                        loop {
                                            cx.background_executor()
                                                .timer(std::time::Duration::from_millis(200))
                                                .await;
                                            let done = counter_poll
                                                .load(std::sync::atomic::Ordering::Relaxed);
                                            let Some(view) = weak.upgrade() else {
                                                break;
                                            };
                                            let running = cx
                                                .update_entity(&view, |this, cx| {
                                                    match this.sync_progress {
                                                        Some((_, total)) => {
                                                            this.sync_progress = Some((done, total));
                                                            cx.notify();
                                                            true
                                                        }
                                                        None => false,
                                                    }
                                                });
                                            if !running {
                                                break;
                                            }
                                        }
                                    }
                                })
                                .detach();
                            }
                        }
                        cx.notify();
                    }
                    Err(e) => {
                        tracing::error!("Scan failed: {e}");
                    }
                }
            },
        );
    }

    /// 后台逐个提取 EXIF（并发），完成后更新 CaptureMeta 并通知重绘。
    /// RAW 文件通过 rawlib 0.7+ 的 LibRaw 读取，可能较慢，不阻塞主线程。
    /// 提取结果写回 SQLite exif_cache（下次扫描不再提取）；全部完成后重排一次
    /// （EXIF 日期/尺寸影响 DateTaken 排序与预览 fit 尺寸）。
    pub(crate) fn spawn_enrich_tasks(&mut self, cx: &mut Context<Self>) {
        use std::sync::atomic::Ordering;
        let generation = self.scan_generation;
        let folder_db = self.folder_db.clone();
        // 收集所有需要提取 EXIF 的 capture 路径（扫描闭包只查缓存，未命中的字段为空）
        let paths: Vec<(usize, PathBuf)> = self
            .captures
            .iter()
            .filter_map(|meta| {
                if meta.camera_make.is_some() || meta.iso.is_some() || meta.image_width.is_some() {
                    return None;
                }
                Some((meta.index, PathBuf::from(&meta.primary_path)))
            })
            .collect();
        if paths.is_empty() {
            return;
        }
        let total = paths.len();
        let done = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        for (idx, path) in paths {
            let path_for_worker = path.clone();
            let db = folder_db.clone();
            let done_work = done.clone();
            self.worker.spawn(
                cx,
                move || {
                    let ext = path_for_worker
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_lowercase();
                    let fmt = photo_domain::ImageFormat::from_extension(&ext);
                    let format = match fmt {
                        Some(f) => f,
                        None => return None,
                    };
                    // 经 SQLite 缓存提取并写回：下次扫描命中缓存，不再重复 LibRaw open
                    if let Some(db) = &db {
                        db.get_or_extract_exif(&path_for_worker, &format).ok()
                    } else {
                        photo_engine::exif::extract_exif(&path_for_worker, &format).ok()
                    }
                },
                move |this, exif, cx| {
                    // 过期目录/列表：丢弃，防按新索引错绑 EXIF
                    if generation != this.scan_generation {
                        return;
                    }
                    done_work.fetch_add(1, Ordering::Relaxed);
                    if let Some(ref exif) = exif {
                        if let Some(meta) = this.captures.iter_mut().find(|m| m.index == idx) {
                            meta.camera_make = exif.camera.make.clone();
                            meta.camera_model = exif.camera.model.clone();
                            meta.lens = exif.camera.lens.clone();
                            meta.exposure_time = exif.shooting.exposure_time.clone();
                            meta.f_number = exif.shooting.f_number.clone();
                            meta.iso = exif.shooting.iso;
                            meta.focal_length = exif.shooting.focal_length.clone();
                            meta.image_width = exif.image_width;
                            meta.image_height = exif.image_height;
                            meta.date_taken = exif.date_time_original.clone();
                            if meta.file_size.is_none() {
                                meta.file_size = std::fs::metadata(&path).ok().map(|m| m.len());
                            }
                        }
                    }
                    // 全部提取完成后重排一次（EXIF 渐显期间排序保持原序，避免每张全量重排）
                    if done_work.load(Ordering::Relaxed) >= total {
                        this.apply_filter_and_sort();
                    }
                    cx.notify();
                },
            );
        }
    }
}
