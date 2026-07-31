use std::collections::HashSet;
use std::path::{Path, PathBuf};

use gpui::*;
use photo_domain::{ColorLabel, Flag, Rating};

use super::app::RootView;

// 选择集与元数据编辑（评分/旗标/颜色标签/删除，自 state/app.rs 拆出，纯移动）

impl RootView {
    /// Select an item in display_order by its display index.
    /// `additive`: Ctrl-click — toggle this item.
    /// `range`: Shift-click — select range from anchor to this index.
    pub fn select(&mut self, index: usize, additive: bool, range: bool) {
        if index >= self.display_order.len() {
            return;
        }
        let capture_idx = self.display_order[index];
        // 鼠标点击即移动焦点，预览/信息面板/双击进入预览都依赖 focus_index
        self.focus_index = Some(index);
        // 焦点变化 → 右侧识别卡片/调整参数同步刷新（廉价 SQLite 点查）
        self.refresh_focused_recognition();
        self.refresh_adjustments_sync();

        if range {
            if let Some(anchor) = self.anchor {
                let start = anchor.min(index);
                let end = anchor.max(index);
                if !additive {
                    self.selected.clear();
                }
                for di in start..=end {
                    if let Some(&ci) = self.display_order.get(di) {
                        self.selected.insert(ci);
                    }
                }
            }
        } else if additive {
            if self.selected.contains(&capture_idx) {
                self.selected.remove(&capture_idx);
            } else {
                self.selected.insert(capture_idx);
            }
            self.anchor = Some(index);
        } else {
            self.anchor = Some(index);
            self.selected.clear();
            self.selected.insert(capture_idx);
        }
    }

    pub fn select_all(&mut self) {
        self.selected.clear();
        for &ci in &self.display_order {
            self.selected.insert(ci);
        }
    }

    /// 右键菜单打开前调用：焦点移到被点项；该项不在多选中时独占选中，
    /// 已在多选中时保持多选（删除等操作作用于整个选择集）
    pub fn focus_for_context_menu(&mut self, display_idx: usize) {
        let Some(&ci) = self.display_order.get(display_idx) else {
            return;
        };
        self.focus_index = Some(display_idx);
        self.refresh_focused_recognition();
        self.refresh_adjustments_sync();
        if !self.selected.contains(&ci) {
            self.selected.clear();
            self.selected.insert(ci);
            self.anchor = Some(display_idx);
        }
    }

    pub fn deselect_all(&mut self) {
        self.selected.clear();
    }

    pub fn set_rating(
        &mut self,
        indices: &[usize],
        rating: Rating,
        cx: &mut Context<Self>,
    ) {
        let paths: Vec<(PathBuf, Rating)> = indices
            .iter()
            .filter_map(|&i| {
                let meta = self.captures.get_mut(i)?;
                let old = meta.rating;
                // 乐观更新：立即显示新评分，XMP 异步写入
                meta.rating = rating;
                Some((PathBuf::from(&meta.primary_path), old))
            })
            .collect();

        if paths.is_empty() {
            return;
        }

        self.apply_filter_and_sort();
        cx.notify();

        let folder_db = self.folder_db.clone();
        self.worker.spawn(
            cx,
            move || {
                let Some(db) = &folder_db else { return vec![]; };
                let mut results = Vec::new();
                for (path, old) in &paths {
                    let result = (|| -> Result<(), photo_engine::folder_db::FolderDbError> {
                        let mut meta = db.get_xmp(path)?.unwrap_or_default();
                        meta.set_rating(rating);
                        db.put_xmp(path, &meta)?;
                        Ok(())
                    })();
                    results.push((path.clone(), *old, result));
                }
                results
            },
            move |this, results, cx| {
                let mut reverted = false;
                for (path, old, result) in &results {
                    if let Err(e) = result {
                        tracing::error!("Failed to persist rating for {}: {e}", path.display());
                        // 回滚乐观更新：按主路径找回 meta 恢复旧值，UI 与 DB 保持一致
                        if let Some(meta) = this.captures.iter_mut().find(
                            |m| Path::new(&m.primary_path) == path,
                        ) {
                            meta.rating = *old;
                            reverted = true;
                        }
                    }
                }
                if reverted {
                    this.apply_filter_and_sort();
                    this.show_toast("评分保存失败，已回滚", cx);
                    cx.notify();
                }
            },
        );
    }
    pub fn set_flag(
        &mut self,
        indices: &[usize],
        flag: Option<Flag>,
        cx: &mut Context<Self>,
    ) {
        let paths: Vec<(PathBuf, Option<Flag>)> = indices
            .iter()
            .filter_map(|&i| {
                let meta = self.captures.get_mut(i)?;
                let old = meta.flag;
                // 乐观更新：立即显示新旗标，XMP 异步写入
                meta.flag = flag;
                Some((PathBuf::from(&meta.primary_path), old))
            })
            .collect();

        if paths.is_empty() {
            return;
        }
        self.apply_filter_and_sort();
        cx.notify();

        let folder_db = self.folder_db.clone();
        self.worker.spawn(
            cx,
            move || {
                let Some(db) = &folder_db else { return vec![]; };
                let mut results = Vec::new();
                for (path, old) in &paths {
                    let result = (|| -> Result<(), photo_engine::folder_db::FolderDbError> {
                        let mut meta = db.get_xmp(path)?.unwrap_or_default();
                        meta.set_flag(flag);
                        db.put_xmp(path, &meta)?;
                        Ok(())
                    })();
                    results.push((path.clone(), *old, result));
                }
                results
            },
            move |this, results, cx| {
                let mut reverted = false;
                for (path, old, result) in &results {
                    if let Err(e) = result {
                        tracing::error!("Failed to persist flag for {}: {e}", path.display());
                        if let Some(meta) = this.captures.iter_mut().find(
                            |m| Path::new(&m.primary_path) == path,
                        ) {
                            meta.flag = *old;
                            reverted = true;
                        }
                    }
                }
                if reverted {
                    this.apply_filter_and_sort();
                    this.show_toast("旗标保存失败，已回滚", cx);
                    cx.notify();
                }
            },
        );
    }

    pub fn set_color_label(
        &mut self,
        indices: &[usize],
        label: ColorLabel,
        cx: &mut Context<Self>,
    ) {
        let paths: Vec<(PathBuf, ColorLabel)> = indices
            .iter()
            .filter_map(|&i| {
                let meta = self.captures.get_mut(i)?;
                let old = meta.color_label;
                // 乐观更新：立即显示新标签，XMP 异步写入
                meta.color_label = label;
                Some((PathBuf::from(&meta.primary_path), old))
            })
            .collect();

        if paths.is_empty() {
            return;
        }
        self.apply_filter_and_sort();
        cx.notify();

        let folder_db = self.folder_db.clone();
        self.worker.spawn(
            cx,
            move || {
                let Some(db) = &folder_db else { return vec![]; };
                let mut results = Vec::new();
                for (path, old) in &paths {
                    let result = (|| -> Result<(), photo_engine::folder_db::FolderDbError> {
                        let mut meta = db.get_xmp(path)?.unwrap_or_default();
                        meta.set_color_label(label);
                        db.put_xmp(path, &meta)?;
                        Ok(())
                    })();
                    results.push((path.clone(), *old, result));
                }
                results
            },
            move |this, results, cx| {
                let mut reverted = false;
                for (path, old, result) in &results {
                    if let Err(e) = result {
                        tracing::error!("Failed to persist color label for {}: {e}", path.display());
                        if let Some(meta) = this.captures.iter_mut().find(
                            |m| Path::new(&m.primary_path) == path,
                        ) {
                            meta.color_label = *old;
                            reverted = true;
                        }
                    }
                }
                if reverted {
                    this.apply_filter_and_sort();
                    this.show_toast("颜色标签保存失败，已回滚", cx);
                    cx.notify();
                }
            },
        );
    }


    pub fn delete_selected(&mut self, cx: &mut Context<Self>) {
        let capture_indices: Vec<usize> = self.selected.drain().collect();
        if capture_indices.is_empty() {
            return;
        }

        // 收集被删除文件的唯一主路径（去重）
        let path_set: Vec<PathBuf> = capture_indices
            .iter()
            .filter_map(|&i| self.captures.get(i))
            .map(|meta| PathBuf::from(&meta.primary_path))
            .collect();

        // 计算 rel_paths 用于删除后的识别行同步
        let dir_clone = self.dir_path.clone();
        let rel_paths: Vec<String> = path_set
            .iter()
            .filter_map(|primary| {
                dir_clone.as_ref().and_then(|d| {
                    primary.strip_prefix(d).ok().map(|rel| {
                        rel.to_string_lossy().replace('\\', "/")
                    })
                })
            })
            .collect();

        let paths: Vec<PathBuf> = capture_indices
            .iter()
            .filter_map(|&i| self.captures.get(i))
            .map(|meta| {
                PathBuf::from(&meta.primary_path)
            })
            .collect();

        self.worker.spawn(
            cx,
            move || {
                use photo_engine::ops;
                let mut results = Vec::new();
                for path in &paths {
                    let result = ops::delete_file(path);
                    results.push((path.clone(), result));
                }
                results
            },
            move |this, results, cx| {
                let mut deleted = HashSet::new();
                for (path, result) in &results {
                    match result {
                        Ok(()) => {
                            deleted.insert(path.clone());
                            tracing::info!("Deleted: {}", path.display());
                        }
                        Err(e) => {
                            tracing::error!("Delete failed for {}: {e}", path.display());
                        }
                    }
                }
                // 同步删除识别行
                if let Some(ref db) = this.folder_db {
                    if !rel_paths.is_empty() {
                        let _ = photo_engine::ops::sync_delete_recognitions(db, &rel_paths);
                    }
                }
                // Remove deleted captures from the list
                this.captures.retain(|meta| {
                    let primary = PathBuf::from(&meta.primary_path);
                    !deleted.contains(&primary)
                });
                // Re-index
                for (i, meta) in this.captures.iter_mut().enumerate() {
                    meta.index = i;
                }
                // 换代 + 清空按旧索引缓存与加载哨兵：在途任务按代数丢弃，
                // 避免旧结果按新索引写入（张冠李戴）；可见网格下次渲染自动重载
                this.scan_generation += 1;
                this.thumbnail_data.clear();
                this.preview_data.clear();
                this.preview_order.clear();
                this.fullres_data.clear();
                this.fullres_order.clear();
                this.grid_loading.clear();
                this.preview_loading.clear();
                this.preview_cancel.clear();
                this.fullres_loading.clear();
                this.refresh_focused_recognition();
                this.apply_filter_and_sort();
                cx.notify();
            },
        );
    }

    pub(crate) fn apply_rating(&mut self, rating: Rating, cx: &mut Context<Self>) {
        let indices: Vec<usize> = if self.selected.is_empty() {
            self.get_focused_capture()
                .and_then(|m| self.captures.iter().position(|c| c.base_name == m.base_name))
                .into_iter()
                .collect()
        } else {
            self.selected.iter().copied().collect()
        };
        if !indices.is_empty() {
            self.set_rating(&indices, rating, cx);
        }
    }

    pub(crate) fn apply_label(&mut self, label: ColorLabel, cx: &mut Context<Self>) {
        let indices: Vec<usize> = if self.selected.is_empty() {
            self.get_focused_capture()
                .and_then(|m| self.captures.iter().position(|c| c.base_name == m.base_name))
                .into_iter()
                .collect()
        } else {
            self.selected.iter().copied().collect()
        };
        if !indices.is_empty() {
            self.set_color_label(&indices, label, cx);
        }
    }

    pub(crate) fn apply_flag(&mut self, flag: Option<Flag>, cx: &mut Context<Self>) {
        let indices: Vec<usize> = if self.selected.is_empty() {
            self.get_focused_capture()
                .and_then(|m| self.captures.iter().position(|c| c.base_name == m.base_name))
                .into_iter()
                .collect()
        } else {
            self.selected.iter().copied().collect()
        };
        if !indices.is_empty() {
            self.set_flag(&indices, flag, cx);
        }
    }
}
