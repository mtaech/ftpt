use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use gpui::*;
use gpui_component::IndexPath;
use gpui_component::select::{SearchableVec, SelectEvent, SelectState};
use std::sync::LazyLock;
use photo_tool_core::config::AppConfig;
use photo_tool_core::domain::{
    CaptureMeta, ColorLabel, DeleteMode, FilterCriteria, Flag, Rating, SortBy, SortDirection,
};
use photo_tool_core::thumbnail::ThumbnailCache;
use photo_tool_core::{scanner, xmp};

use crate::worker::Worker;

/// 系统已安装字体家族（排序去重，LazyLock 只枚举一次）
pub static SYSTEM_FONTS: LazyLock<Vec<String>> = LazyLock::new(|| {
    let mut families = font_kit::source::SystemSource::new()
        .all_families()
        .unwrap_or_default();
    families.sort();
    families.dedup();
    families
});

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Grid,
    Preview,
}

pub struct RootView {
    pub config: AppConfig,
    pub config_path: PathBuf,
    pub worker: Worker,
    pub thumbnail_cache: Option<ThumbnailCache>,
    pub dir_path: Option<PathBuf>,
    pub captures: Vec<CaptureMeta>,
    /// Stores indices into `self.captures` of selected items.
    pub selected: HashSet<usize>,
    /// Stores a display-order index for range selection anchor.
    pub anchor: Option<usize>,
    pub view_mode: ViewMode,
    pub sort_by: SortBy,
    pub sort_dir: SortDirection,
    pub filter: FilterCriteria,
    /// Sorted, filtered indices into `self.captures`.
    pub focus_index: Option<usize>,
    /// Sorted, filtered indices into `self.captures`.
    pub display_order: Vec<usize>,
    /// Thumbnail JPEG bytes keyed by capture index.
    pub thumbnail_data: HashMap<usize, Vec<u8>>,
    /// 预览图字节（含格式）按 capture 索引缓存：非 RAW 为原图，RAW 为大尺寸内嵌预览
    pub preview_data: HashMap<usize, (ImageFormat, Vec<u8>)>,
    /// preview_data 的 FIFO 淘汰顺序
    preview_order: VecDeque<usize>,
    /// 网格虚拟列表滚动句柄（uniform_list 滚动条用）
    pub grid_scroll_handle: UniformListScrollHandle,
    /// 字体选择下拉状态（设置弹窗用）
    pub font_select: Entity<SelectState<SearchableVec<SharedString>>>,
    pub show_settings: bool,
    pub scan_task: Option<Task<()>>,
    /// 左侧边栏当前显示的侧栏 tab：0=文件树，1=收藏夹，2=筛选
    pub sidebar_section: usize,
    /// 左侧边栏显隐（由 Activity Rail 或快捷键切换）
    pub sidebar_visible: bool,
    /// 正在后台加载中的预览图 capture 索引（防重复 spawn）
    preview_loading: HashSet<usize>,
    /// 正在后台加载中的网格缩略图 capture 索引（防重复 spawn）
    grid_loading: HashSet<usize>,
    /// 预览缩放倍率（1.0 = 适配窗口）
    pub preview_zoom: f32,
    /// 预览平移偏移（像素），缩放后拖拽移动图片
    pub preview_pan: (f32, f32),
    /// 拖拽起始状态：(鼠标x, 鼠标y, 起始pan_x, 起始pan_y)
    pub preview_drag: Option<(f32, f32, f32, f32)>,
}

impl RootView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>, config_path: PathBuf, config: AppConfig) -> Self {
        let cache_dir = config_path
            .parent()
            .unwrap_or(Path::new("."))
            .join("thumbnails");
        let thumbnail_cache = Some(ThumbnailCache::new(cache_dir));

        let worker = Worker::new();

        // 字体选择下拉：系统字体列表 + 当前配置项 + 选中后写回配置
        let font_items: Vec<SharedString> = SYSTEM_FONTS
            .iter()
            .map(|f| SharedString::from(f.clone()))
            .collect();
        let selected_ix = font_items
            .iter()
            .position(|f| f.as_str() == config.font_family.as_str())
            .map(|row| IndexPath::default().row(row));
        let font_select = cx.new(|cx| {
            SelectState::new(SearchableVec::new(font_items), selected_ix, window, cx).searchable(true)
        });
        cx.subscribe(&font_select, |view, _state, event, cx| {
            if let SelectEvent::Confirm(Some(name)) = event {
                view.config.font_family = name.to_string();
                view.save_config();
                // gpui-component 组件不从根元素继承字体，必须更新全局 Theme
                gpui_component::theme::Theme::global_mut(cx).font_family = name.clone();
                cx.notify();
            }
        })
        .detach();

        // 启动后自动扫描上次打开的目录
        let auto_dir = config.last_directory.clone();
        let mut this = Self {
            config,
            config_path,
            worker,
            thumbnail_cache,
            dir_path: None,
            captures: Vec::new(),
            selected: HashSet::new(),
            anchor: None,
            view_mode: ViewMode::Grid,
            sort_by: SortBy::FileName,
            sort_dir: SortDirection::Ascending,
            filter: FilterCriteria::default(),
            focus_index: None,
            display_order: Vec::new(),
            scan_task: None,
            preview_loading: HashSet::new(),
            grid_loading: HashSet::new(),
            preview_zoom: 1.0,
            preview_pan: (0.0, 0.0),
            preview_drag: None,
            thumbnail_data: HashMap::new(),
            preview_data: HashMap::new(),
            preview_order: VecDeque::new(),
            font_select,
            show_settings: false,
            sidebar_section: 0,
            sidebar_visible: true,
            grid_scroll_handle: UniformListScrollHandle::new(),
        };

        if let Some(last_dir) = &auto_dir {
            this.scan_directory(PathBuf::from(last_dir), cx);
        }

        this
    }

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
        // Cancel any existing scan
        if let Some(task) = self.scan_task.take() {
            drop(task);
        }
        // 在照片目录中打开 EXIF 缓存（.pt-cache.db）
        let exif_cache = photo_tool_core::cache::ExifCache::open_in_dir(&path)
            .ok();
        let sidecar_exts = self.config.sidecar_extensions.clone();
        let filter = self.filter.clone();

        self.worker.spawn(
            cx,
            move || {
                scanner::scan_directory(&path, &sidecar_exts, &filter, None)
                    .map(|captures| {
                        let metas: Vec<CaptureMeta> = captures
                            .iter()
                            .enumerate()
                            .map(|(i, c)| {
                                let mut meta = CaptureMeta::from(c);
                                meta.index = i;
                                meta.enrich_with_xmp(); // 快：只读 sidecar
                                // 从缓存获取 EXIF（未命中则提取并写入缓存）
                                if let Some(ref cache) = exif_cache {
                                    let primary = &c.source_files[c.primary_index];
                                    let exif = cache
                                        .get_or_extract(&primary.path, &primary.format)
                                        .unwrap_or_default();
                                    meta.enrich_with_exif(&exif);
                                }
                                meta
                            })
                            .collect();
                        (path, metas)
                    })
            },
            |this, result, cx| {
                match result {
                    Ok((dir, metas)) => {
                        this.captures = metas;
                        this.dir_path = Some(dir.clone());
                        // 记住最后打开的目录，下次启动自动恢复
                        this.config.last_directory = Some(dir.to_string_lossy().to_string());
                        this.save_config();
                        this.thumbnail_data.clear();
                        this.preview_data.clear();
                        this.preview_order.clear();
                        this.apply_filter_and_sort();
                        tracing::info!(
                            "扫描完成：{} 找到 {} 个 capture，过滤后 {} 个",
                            dir.display(),
                            this.captures.len(),
                            this.display_order.len()
                        );
                        // 后台逐步提取 EXIF（RAW 文件的 LibRaw unpack 较慢）
                        this.spawn_enrich_tasks(cx);
                        this.preload_thumbnails(cx);
                        cx.notify();
                    }
                    Err(e) => {
                        tracing::error!("Scan failed: {e}");
                    }
                }
            },
        );
    }

    /// 后台逐个提取 EXIF，完成后更新 CaptureMeta 并通知重绘。
    /// RAW 文件通过 rawlib 0.7+ 的 LibRaw 读取，可能较慢，不阻塞主线程。
    fn spawn_enrich_tasks(&mut self, cx: &mut Context<Self>) {
        // 收集所有需要提取 EXIF 的 capture 路径
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

        for (idx, path) in paths {
            let path_for_worker = path.clone();
            self.worker.spawn(
                cx,
                move || {
                    let ext = path_for_worker
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_lowercase();
                    let fmt = photo_tool_core::domain::ImageFormat::from_extension(&ext);
                    let format = match fmt {
                        Some(f) => f,
                        None => return None,
                    };
                    photo_tool_core::exif::extract_exif(&path_for_worker, &format).ok()
                },
                move |this, exif, cx| {
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
                    cx.notify();
                },
            );
        }
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
                // paired_only
                if let Some(paired) = filter.paired_only {
                    let is_paired = meta.stack_count > 0;
                    if is_paired != paired {
                        return false;
                    }
                }
                // format_filter
                if let Some(ref fmt_filter) = filter.format_filter {
                    if meta.primary_format != fmt_filter.to_string() {
                        return false;
                    }
                }
                // text_search
                if let Some(ref text) = filter.text_search {
                    if !meta
                        .base_name
                        .to_lowercase()
                        .contains(&text.to_lowercase())
                    {
                        return false;
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
                            if let Some(from) = filter.date_from {
                                if d < from {
                                    return false;
                                }
                            }
                            if let Some(to) = filter.date_to {
                                if d > to {
                                    return false;
                                }
                            }
                        }
                    } else if filter.date_from.is_some() || filter.date_to.is_some() {
                        return false;
                    }
                }
                // unflagged_filter
                if filter.unflagged_filter {
                    // Proxy: no XMP sidecar likely means no flag set
                    if meta.has_xmp {
                        return false;
                    }
                }
                // min_rating, color_label, flag_filter require XMP data
                // not available in CaptureMeta — pass through for now.
                true
            })
            .map(|(i, _)| i)
            .collect();

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
                    let ta = std::fs::metadata(&ma.primary_path)
                        .and_then(|m| m.modified())
                        .ok();
                    let tb = std::fs::metadata(&mb.primary_path)
                        .and_then(|m| m.modified())
                        .ok();
                    ta.cmp(&tb)
                }
            };
            match sort_dir {
                SortDirection::Ascending => cmp,
                SortDirection::Descending => cmp.reverse(),
            }
        });

        self.display_order = indices;
    }

    /// 预览图缓存上限（张）
    const PREVIEW_CACHE_LIMIT: usize = 20;

    fn insert_preview(&mut self, idx: usize, format: ImageFormat, bytes: Vec<u8>) {
        self.preview_order.retain(|&i| i != idx);
        self.preview_order.push_back(idx);
        self.preview_data.insert(idx, (format, bytes));
        while self.preview_order.len() > Self::PREVIEW_CACHE_LIMIT {
            if let Some(oldest) = self.preview_order.pop_front() {
                self.preview_data.remove(&oldest);
            }
        }
    }

    /// 确保当前焦点图片的预览数据已加载：全部格式经 worker 线程缩放到 1600px。
    /// 同时预取前后各一张，快速切换时无感。
    pub fn ensure_preview_loaded(&mut self, cx: &mut Context<Self>) {
        let Some(focus_idx) = self.focus_index else { return };
        self.spawn_preview_load(focus_idx, cx);
        // 相邻预取：前后各一张
        if focus_idx > 0 {
            self.spawn_preview_load(focus_idx - 1, cx);
        }
        if focus_idx + 1 < self.display_order.len() {
            self.spawn_preview_load(focus_idx + 1, cx);
        }
    }

    /// 为指定 display_order 索引 spawn 预览图加载任务（已缓存或已在加载则跳过）
    fn spawn_preview_load(&mut self, display_idx: usize, cx: &mut Context<Self>) {
        use photo_tool_core::domain::{ImageFormat as DomainFormat, SourceFile};

        let Some(&capture_idx) = self.display_order.get(display_idx) else { return };
        if self.preview_data.contains_key(&capture_idx) { return; }
        if !self.preview_loading.insert(capture_idx) { return; } // 已在加载
        let Some(meta) = self.captures.get(capture_idx) else { return };

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
            is_sidecar: false,
            file_size: meta.file_size,
        };
        self.worker.spawn(
            cx,
            move || {
                cache
                    .get_or_generate(&source, 1600)
                    .map_err(|e| tracing::warn!("预览图生成失败: {e}"))
                    .ok()
            },
            move |this, result, cx| {
                this.preview_loading.remove(&capture_idx);
                if let Some(bytes) = result {
                    this.insert_preview(capture_idx, ImageFormat::Jpeg, bytes);
                    cx.notify();
                }
            },
        );
    }



    /// 为单个 capture 生成网格缩略图（懒加载：滚动时按需触发）
    pub fn ensure_thumbnail_loaded(&mut self, capture_idx: usize, cx: &mut Context<Self>) {
        if self.thumbnail_data.contains_key(&capture_idx) { return; }
        if !self.grid_loading.insert(capture_idx) { return; } // 已在加载

        let thumbnail_size = self.config.thumbnail_size;
        let Some(cache) = self.thumbnail_cache.clone() else { return };
        let Some(meta) = self.captures.get(capture_idx) else { return };

        let path = PathBuf::from(&meta.primary_path);
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let format = photo_tool_core::domain::ImageFormat::from_extension(&ext)
            .unwrap_or(photo_tool_core::domain::ImageFormat::Jpeg);

        let source = photo_tool_core::domain::SourceFile {
            path,
            format,
            is_sidecar: false,
            file_size: meta.file_size,
        };

        self.worker.spawn(
            cx,
            move || {
                cache
                    .get_or_generate(&source, thumbnail_size * 2)
                    .map_err(|e| {
                        tracing::warn!("懒加载缩略图失败 {}: {e}", source.path.display());
                    })
                    .ok()
            },
            move |this, result, cx| {
                this.grid_loading.remove(&capture_idx);
                if let Some(bytes) = result {
                    this.thumbnail_data.insert(capture_idx, bytes);
                    cx.notify();
                }
            },
        );
    }

    /// Spawn background tasks to preload thumbnails for the first N visible items.
    pub fn preload_thumbnails(&mut self, cx: &mut Context<Self>) {
        use photo_tool_core::domain::{ImageFormat, SourceFile};

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
        let count = self.display_order.len().min(PRELOAD_LIMIT);

        for di in 0..count {
            let capture_idx = match self.display_order.get(di) {
                Some(&ci) => ci,
                None => continue,
            };
            let capture = match self.captures.get(capture_idx) {
                Some(c) => c,
                None => continue,
            };

            let primary_path = std::path::PathBuf::from(&capture.primary_path);
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
                is_sidecar: false,
                file_size: capture.file_size,
            };

            let cache_clone = cache.clone();
            let ci = capture_idx;
            let path_display = source.path.clone();
            self.worker.spawn(cx, move || {
                // 2x 生成：高 DPI 下 1x 缩略图拉伸会模糊
                cache_clone
                    .get_or_generate(&source, thumbnail_size * 2)
                    .map_err(|e| {
                        tracing::warn!("缩略图生成失败 {}: {e}", path_display.display());
                    })
                    .ok()
            }, move |this, result, _cx| {
                if let Some(bytes) = result {
                    this.thumbnail_data.insert(ci, bytes);
                    _cx.notify();
                }
            });
        }
}

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

        self.worker.spawn(
            cx,
            move || {
                let mut results = Vec::new();
                for (path, _old) in &paths {
                    let xp = xmp::xmp_path(path);
                    let result = (|| -> Result<(), xmp::XmpError> {
                        let mut meta = if xp.exists() {
                            xmp::read_xmp(&xp)?
                        } else {
                            xmp::XmpMetadata::default()
                        };
                        meta.set_rating(rating);
                        xmp::write_xmp(&xp, &meta)?;
                        Ok(())
                    })();
                    results.push((path.clone(), result));
                }
                results
            },
            move |_this, results, _cx| {
                for (path, result) in &results {
                    if let Err(e) = result {
                        tracing::error!("Failed to persist rating for {}: {e}", path.display());
                    }
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

        self.worker.spawn(
            cx,
            move || {
                let mut results = Vec::new();
                for (path, _old) in &paths {
                    let xp = xmp::xmp_path(path);
                    let result = (|| -> Result<(), xmp::XmpError> {
                        let mut meta = if xp.exists() {
                            xmp::read_xmp(&xp)?
                        } else {
                            xmp::XmpMetadata::default()
                        };
                        meta.set_flag(flag);
                        xmp::write_xmp(&xp, &meta)?;
                        Ok(())
                    })();
                    results.push((path.clone(), result));
                }
                results
            },
            move |_this, results, _cx| {
                for (path, result) in &results {
                    if let Err(e) = result {
                        tracing::error!("Failed to persist flag for {}: {e}", path.display());
                    }
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

        self.worker.spawn(
            cx,
            move || {
                let mut results = Vec::new();
                for (path, _old) in &paths {
                    let xp = xmp::xmp_path(path);
                    let result = (|| -> Result<(), xmp::XmpError> {
                        let mut meta = if xp.exists() {
                            xmp::read_xmp(&xp)?
                        } else {
                            xmp::XmpMetadata::default()
                        };
                        meta.set_color_label(label);
                        xmp::write_xmp(&xp, &meta)?;
                        Ok(())
                    })();
                    results.push((path.clone(), result));
                }
                results
            },
            move |_this, results, _cx| {
                for (path, result) in &results {
                    if let Err(e) = result {
                        tracing::error!("Failed to persist color label for {}: {e}", path.display());
                    }
                }
            },
        );
    }


    pub fn delete_selected(&mut self, mode: DeleteMode, cx: &mut Context<Self>) {
        let capture_indices: Vec<usize> = self.selected.drain().collect();
        if capture_indices.is_empty() {
            return;
        }

        let paths: Vec<PathBuf> = capture_indices
            .iter()
            .filter_map(|&i| self.captures.get(i))
            .flat_map(|meta| {
                let primary = PathBuf::from(&meta.primary_path);
                let xp = xmp::xmp_path(&primary);
                vec![primary, xp]
            })
            .collect();

        self.worker.spawn(
            cx,
            move || {
                use photo_tool_core::ops;
                let mut results = Vec::new();
                for path in &paths {
                    let result = ops::delete_file(path, mode);
                    results.push((path.clone(), result));
                }
                results
            },
            |this, results, cx| {
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
                // Remove deleted captures from the list
                this.captures.retain(|meta| {
                    let primary = PathBuf::from(&meta.primary_path);
                    !deleted.contains(&primary)
                });
                // Re-index
                for (i, meta) in this.captures.iter_mut().enumerate() {
                    meta.index = i;
                }
                this.apply_filter_and_sort();
                cx.notify();
            },
        );
    }

    pub fn toggle_view_mode(&mut self, cx: &mut Context<Self>) {
        self.view_mode = match self.view_mode {
            ViewMode::Grid => ViewMode::Preview,
            ViewMode::Preview => ViewMode::Grid,
        };
        if self.view_mode == ViewMode::Preview {
            self.ensure_preview_loaded(cx);
        }
    }

    /// Get the first selected capture (for info panel / preview).
    pub fn get_focused_capture(&self) -> Option<&CaptureMeta> {
        self.focus_index
            .and_then(|di| self.display_order.get(di))
            .and_then(|&ci| self.captures.get(ci))
    }

    /// Dispatch an action. Returns true if the view should be re-rendered.
    pub fn dispatch_action(&mut self, action: crate::action::Action, cx: &mut Context<Self>) {
        use crate::action::Action;
        match action {
            // Navigation（切换图片时重置缩放和平移）
            Action::Next => {
                if let Some(idx) = self.focus_index {
                    if idx + 1 < self.display_order.len() {
                        self.focus_index = Some(idx + 1);
                    }
                } else if !self.display_order.is_empty() {
                    self.focus_index = Some(0);
                }
                self.preview_zoom = 1.0;
                self.preview_pan = (0.0, 0.0);
                cx.notify();
            }
            Action::Prev => {
                if let Some(idx) = self.focus_index {
                    if idx > 0 {
                        self.focus_index = Some(idx - 1);
                    }
                } else if !self.display_order.is_empty() {
                    self.focus_index = Some(0);
                }
                self.preview_zoom = 1.0;
                self.preview_pan = (0.0, 0.0);
                cx.notify();
            }
            // View
            Action::ToggleGridPreview => {
                self.toggle_view_mode(cx);
                cx.notify();
            }
            Action::ZoomIn => {
                self.preview_zoom = (self.preview_zoom * 1.25).min(10.0);
                cx.notify();
            }
            Action::ZoomOut => {
                self.preview_zoom = (self.preview_zoom / 1.25).max(0.1);
                if self.preview_zoom <= 1.0 {
                    self.preview_pan = (0.0, 0.0);
                }
                cx.notify();
            }
            Action::ZoomToFit => {
                self.preview_zoom = 1.0;
                self.preview_pan = (0.0, 0.0);
                cx.notify();
            }
            // Rating
            Action::Rate0 => self.apply_rating(Rating::None, cx),
            Action::Rate1 => self.apply_rating(Rating::One, cx),
            Action::Rate2 => self.apply_rating(Rating::Two, cx),
            Action::Rate3 => self.apply_rating(Rating::Three, cx),
            Action::Rate4 => self.apply_rating(Rating::Four, cx),
            Action::Rate5 => self.apply_rating(Rating::Five, cx),
            // Color Label
            Action::LabelRed => self.apply_label(ColorLabel::Red, cx),
            Action::LabelYellow => self.apply_label(ColorLabel::Yellow, cx),
            Action::LabelGreen => self.apply_label(ColorLabel::Green, cx),
            Action::LabelBlue => self.apply_label(ColorLabel::Blue, cx),
            Action::LabelPurple => self.apply_label(ColorLabel::Purple, cx),
            Action::LabelNone => self.apply_label(ColorLabel::None, cx),
            // Flag
            Action::FlagPick => self.apply_flag(Some(Flag::Pick), cx),
            Action::FlagReject => self.apply_flag(Some(Flag::Reject), cx),
            Action::FlagNone => self.apply_flag(None, cx),
            // Selection
            Action::SelectAll => {
                self.select_all();
                cx.notify();
            }
            Action::DeselectAll => {
                self.deselect_all();
                cx.notify();
            }
            // File ops
            Action::Delete => {
                self.delete_selected(DeleteMode::Trash, cx);
            }
            Action::PermanentDelete => {
                self.delete_selected(DeleteMode::Permanent, cx);
            }
            // Other
            Action::Refresh => {
                if let Some(ref dir) = self.dir_path.clone() {
                    self.scan_directory(dir.clone(), cx);
                }
            }
            Action::ToggleLeftPanel => {
                // Toggle left panel width (show/hide)
                if self.config.left_panel_width > 0 {
                    self.config.left_panel_width = 0;
                } else {
                    self.config.left_panel_width = 260;
                }
                cx.notify();
            }
            Action::ToggleRightPanel => {
                self.config.right_panel_visible = !self.config.right_panel_visible;
                cx.notify();
            }
            Action::ToggleSettings => {
                self.show_settings = !self.show_settings;
                cx.notify();
            }
        }

        // 导航/切换后确保焦点图的预览数据在加载（已加载则直接返回）
        self.ensure_preview_loaded(cx);
    }

    fn apply_rating(&mut self, rating: Rating, cx: &mut Context<Self>) {
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

    fn apply_label(&mut self, label: ColorLabel, cx: &mut Context<Self>) {
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

    fn apply_flag(&mut self, flag: Option<Flag>, cx: &mut Context<Self>) {
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



    /// Save config to disk (public, for use by UI components).
    pub fn save_config(&self) {
        self.save_config_to_disk();
    }

    fn save_config_to_disk(&self) {
        if let Err(e) = photo_tool_core::config::save_config(&self.config_path, &self.config) {
            tracing::error!("Failed to save config: {e}");
        }
    }
}


impl Render for RootView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::ui::layout::render_layout(self, window, cx)
    }
}
