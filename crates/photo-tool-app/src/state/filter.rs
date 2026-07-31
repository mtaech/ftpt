use std::collections::{HashMap, HashSet};

use gpui::*;
use gpui_component::IndexPath;
use gpui_component::combobox::{ComboboxEvent, ComboboxState};
use gpui_component::select::SearchableVec;
use photo_domain::{FilterCriteria, RecognitionFilter, RecognitionStatus, SortBy, SortDirection};

use super::app::RootView;

// 筛选/排序与鸟种下拉（自 state/app.rs 拆出，纯移动，无逻辑改动）

impl RootView {
    /// 是否有任一筛选条件生效（筛选栏折叠态摘要/「清除全部」显隐依据）
    pub fn has_active_filters(&self) -> bool {
        let f = &self.filter;
        !f.bird_names.is_empty()
            || f.min_rating.is_some()
            || f.flag_filter.is_some()
            || f.unflagged_filter
            || f.recognition_filter != photo_domain::RecognitionFilter::All
            || f.format_filter.is_some()
            || f.date_from.is_some()
            || f.date_to.is_some()
    }

    /// 清空全部筛选条件并重算展示顺序
    pub fn clear_filters(&mut self) {
        self.filter = FilterCriteria::default();
        // 鸟种下拉的选中集独立于 filter，标记重建以同步清空
        self.bird_options_dirty = true;
        self.apply_filter_and_sort();
    }

    pub fn apply_filter_and_sort(&mut self) {
        let filter = &self.filter;
        let sort_by = self.sort_by;
        let sort_dir = self.sort_dir;

        // Filter: collect indices of captures that pass all criteria
        let mut indices: Vec<usize> = self
            .captures
            .iter()
            .enumerate()
            .filter(|(_i, meta)| {
                // format_filter
                if let Some(ref fmt_filter) = filter.format_filter
                    && meta.primary_format != fmt_filter.to_string()
                {
                    return false;
                }
                // bird_names（鸟种多选：bird_name 命中任一选中项即保留）
                if !filter.bird_names.is_empty() {
                    match &meta.bird_name {
                        Some(bn) if filter.bird_names.contains(bn) => {}
                        _ => return false,
                    }
                }
                // date_from / date_to
                if filter.date_from.is_some() || filter.date_to.is_some() {
                    if let Some(date_str) = &meta.date_taken {
                        if let Ok(dt) =
                            chrono::NaiveDateTime::parse_from_str(date_str, "%Y-%m-%d %H:%M:%S")
                                .or_else(|_| {
                                    chrono::NaiveDateTime::parse_from_str(
                                        date_str,
                                        "%Y-%m-%dT%H:%M:%S",
                                    )
                                })
                        {
                            let d = dt.date();
                            if let Some(from) = filter.date_from
                                && d < from
                            {
                                return false;
                            }
                            if let Some(to) = filter.date_to
                                && d > to
                            {
                                return false;
                            }
                        }
                    } else if filter.date_from.is_some() || filter.date_to.is_some() {
                        return false;
                    }
                }
                // unflagged_filter：只显示没有标记旗标的照片（XMP 无记录 = 未标记）
                if filter.unflagged_filter && meta.flag.is_some() {
                    return false;
                }
                // min_rating：评分 ≥ N（无评分照片不满足 ≥1）
                if let Some(min) = filter.min_rating
                    && (meta.rating as u8) < (min as u8)
                {
                    return false;
                }
                // color_label：颜色标签精确匹配
                if let Some(label) = filter.color_label
                    && meta.color_label != label
                {
                    return false;
                }
                // flag_filter：旗标精确匹配（Pick/Reject）
                if let Some(flag) = filter.flag_filter
                    && meta.flag != Some(flag)
                {
                    return false;
                }
                // recognition_filter（CaptureMeta 层已由 folder_db 识别记录 enrich）
                match filter.recognition_filter {
                    RecognitionFilter::All => {}
                    RecognitionFilter::Confirmed => {
                        if meta.recognition_status != Some(RecognitionStatus::Confirmed) {
                            return false;
                        }
                    }
                    RecognitionFilter::NeedsReview => {
                        if meta.recognition_status != Some(RecognitionStatus::NeedsReview) {
                            return false;
                        }
                    }
                    RecognitionFilter::Unrecognized => {
                        if meta.recognition_status != Some(RecognitionStatus::Unrecognized) {
                            return false;
                        }
                    }
                    RecognitionFilter::NotRecognized => {
                        if meta.recognition_status.is_some() {
                            return false;
                        }
                    }
                }
                true
            })
            .map(|(i, _)| i)
            .collect();

        // Modified 排序：预先一次性取 mtime，避免每对比较两次 fs::metadata（UI 线程 O(n log n) 系统调用）
        let mtimes: Option<HashMap<usize, Option<std::time::SystemTime>>> =
            if sort_by == SortBy::Modified {
                Some(
                    self.captures
                        .iter()
                        .enumerate()
                        .map(|(i, m)| {
                            let mt = std::fs::metadata(&m.primary_path)
                                .and_then(|md| md.modified())
                                .ok();
                            (i, mt)
                        })
                        .collect(),
                )
            } else {
                None
            };

        // Sort
        indices.sort_by(|&a, &b| {
            let ma = &self.captures[a];
            let mb = &self.captures[b];
            let cmp = match sort_by {
                SortBy::FileName => ma
                    .base_name
                    .to_lowercase()
                    .cmp(&mb.base_name.to_lowercase()),
                SortBy::DateTaken => {
                    let da = ma.date_taken.as_deref().unwrap_or("");
                    let db = mb.date_taken.as_deref().unwrap_or("");
                    da.cmp(db)
                }
                SortBy::FileSize => {
                    let sa = ma.file_size.unwrap_or(0);
                    let sb = mb.file_size.unwrap_or(0);
                    sa.cmp(&sb)
                }
                SortBy::Rating => {
                    let ra = ma.rating as u8;
                    let rb = mb.rating as u8;
                    ra.cmp(&rb)
                }
                SortBy::Modified => {
                    let ta = mtimes.as_ref().and_then(|m| m.get(&a)).copied().flatten();
                    let tb = mtimes.as_ref().and_then(|m| m.get(&b)).copied().flatten();
                    ta.cmp(&tb)
                }
            };
            match sort_dir {
                SortDirection::Ascending => cmp,
                SortDirection::Descending => cmp.reverse(),
            }
        });

        self.display_order = indices;
        // 筛选集变化 → 同步预估失效重算（UI 显示「+M」）
        self.refresh_batch_sync_extra();
    }

    /// 创建鸟种多选下拉实体并订阅 Change 事件（选中变化即写回 filter.bird_names）
    pub(crate) fn new_bird_select(
        options: &[String],
        selected_names: &[String],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<ComboboxState<SearchableVec<String>>> {
        let selected: Vec<IndexPath> = options
            .iter()
            .enumerate()
            .filter(|(_, n)| selected_names.contains(n))
            .map(|(i, _)| IndexPath::default().row(i))
            .collect();
        let entity = cx.new(|cx| {
            ComboboxState::new(SearchableVec::new(options.to_vec()), selected, window, cx)
                .multiple(true)
                .searchable(true)
        });
        cx.subscribe(&entity, |view, _state, event, cx| {
            if let ComboboxEvent::Change(values) = event {
                view.filter.bird_names = values.clone();
                view.apply_filter_and_sort();
                cx.notify();
            }
        })
        .detach();
        entity
    }

    /// 收集当前 captures 中去重排序后的鸟种中文名；变化时标记鸟种下拉待重建
    pub(crate) fn refresh_bird_options(&mut self) {
        let mut names: Vec<String> = self
            .captures
            .iter()
            .filter_map(|m| m.bird_name.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        names.sort();
        if names != self.bird_options {
            self.bird_options = names;
            self.bird_options_dirty = true;
        }
    }

    /// 重建鸟种下拉（鸟名列表或选中集外部变化后调用；创建需要 Window，故由筛选栏 render 驱动）
    pub fn rebuild_bird_select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 已选鸟名中已不存在的条目剔除
        let pruned: Vec<String> = self
            .filter
            .bird_names
            .iter()
            .filter(|n| self.bird_options.contains(n))
            .cloned()
            .collect();
        if pruned.len() != self.filter.bird_names.len() {
            self.filter.bird_names = pruned;
            self.apply_filter_and_sort();
        }
        let options = self.bird_options.clone();
        let selected = self.filter.bird_names.clone();
        self.bird_select = Self::new_bird_select(&options, &selected, window, cx);
        self.bird_options_dirty = false;
    }
}
