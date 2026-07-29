use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::path::{Path, PathBuf};

use gpui::*;
use gpui_component::IndexPath;
use gpui_component::combobox::{ComboboxEvent, ComboboxState};
use gpui_component::select::{SearchableVec, SelectEvent, SelectState};
use std::sync::LazyLock;
use photo_config::AppConfig;
use photo_domain::{
    BatchOpType, BBox, CaptureMeta, ColorLabel, FilterCriteria, Flag, Rating,
    SortBy, SortDirection,
};
use photo_recognize::Recognizer;
use photo_engine::thumbnail::ThumbnailCache;
use crate::ui::toolbar::SettingsOverlay;
use photo_engine::{scanner, folder_db::FolderDb};

use crate::worker::Worker;

/// 全局通知回调（dispatch_action 通过它弹出 toast）
/// 由 main.rs 在 RootView 创建后设置，避免 Context 类型转换。


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

// ── 预览缩放/平移纯函数（与 preview.rs 渲染公式严格一致，改动需同步）──

/// 预览居中偏移：图片 ≤ 容器时居中（≥0），> 容器时为负（图片向左上溢出）
pub(crate) fn preview_center_offset(disp: f32, container: f32) -> f32 {
    (container - disp) / 2.
}

/// 单轴平移钳制：图片 ≤ 容器时不允许平移；> 容器时边缘不可进入视口
pub(crate) fn clamp_pan_axis(disp: f32, container: f32, pan: f32) -> f32 {
    if disp <= container {
        0.
    } else {
        let center = preview_center_offset(disp, container);
        pan.clamp(container - disp - center, -center)
    }
}

/// 光标中心缩放：保持光标下的图像点不动，返回新 pan。
/// old_disp/new_disp 为缩放前后显示尺寸，container 为容器尺寸，cursor 为容器坐标。
pub(crate) fn pan_after_cursor_zoom(
    old_disp: (f32, f32),
    new_disp: (f32, f32),
    container: (f32, f32),
    pan: (f32, f32),
    cursor: (f32, f32),
) -> (f32, f32) {
    let axis = |old_d: f32, new_d: f32, c: f32, p: f32, cur: f32| {
        let old_origin = preview_center_offset(old_d, c) + p;
        let r = if old_d > 0. { new_d / old_d } else { 1. };
        let new_origin = cur - (cur - old_origin) * r;
        new_origin - preview_center_offset(new_d, c)
    };
    (
        axis(old_disp.0, new_disp.0, container.0, pan.0, cursor.0),
        axis(old_disp.1, new_disp.1, container.1, pan.1, cursor.1),
    )
}

/// 窗口坐标 → 图片归一化坐标（0-1，相对原图）。
///
/// 与 preview.rs 渲染公式严格一致（改动需同步）：
/// 图片左上角窗口坐标 = 图片区原点 + p_4 内边距 + 居中偏移 + 平移。
/// 返回值可能出界（超出 [0,1]），由调用方钳制。
pub(crate) fn window_pos_to_image_norm(
    wx: f32,
    wy: f32,
    area: (f32, f32, f32, f32),
    img: (u32, u32),
    zoom: f32,
    pan: (f32, f32),
) -> Option<(f32, f32)> {
    if area.2 <= 0. || img.0 == 0 || img.1 == 0 {
        return None;
    }
    let pad = 16.0;
    let container_w = (area.2 - pad * 2.).max(1.);
    let container_h = (area.3 - pad * 2.).max(1.);
    let scale = (container_w / img.0 as f32)
        .min(container_h / img.1 as f32)
        .min(1.0);
    let (fit_w, fit_h) = (img.0 as f32 * scale, img.1 as f32 * scale);
    let (disp_w, disp_h) = if zoom == 0.0 {
        (img.0 as f32, img.1 as f32)
    } else {
        (fit_w * zoom, fit_h * zoom)
    };
    if disp_w <= 0. || disp_h <= 0. {
        return None;
    }
    let img_left = area.0 + pad + preview_center_offset(disp_w, container_w) + pan.0;
    let img_top = area.1 + pad + preview_center_offset(disp_h, container_h) + pan.1;
    Some(((wx - img_left) / disp_w, (wy - img_top) / disp_h))
}

/// 将 JPEG/常规图字节解码为可直接绘制的 RenderImage（worker 线程执行）。
/// 预览/全分辨率必须预解码：字节源走 GPUI asset 异步解码，解码完成前 img 画空白，
/// 源切换时会闪白屏；RenderImage 走 ImageSource::Render 同步路径，到达即可绘制。
fn decode_render_image(bytes: &[u8], is_jpeg: bool) -> Option<RenderImage> {
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

pub struct RootView {
    pub config: AppConfig,
    pub config_path: PathBuf,
    pub worker: Worker,
    pub folder_db: Option<FolderDb>,
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
    /// 已解码缩略图按 capture 索引缓存。存 Arc<Image>（加载时构建一次、哈希一次），
    /// 渲染路径只做指针拷贝——此前存 Vec<u8>，grid/preview 每帧克隆整表字节，是卡顿主因。
    pub thumbnail_data: HashMap<usize, Arc<Image>>,
    /// 预览图按 capture 索引缓存：worker 线程预解码为 RenderImage，
    /// 到达即可绘制（字节源走 GPUI asset 异步解码，首帧会画空白——切换白屏根因）
    pub preview_data: HashMap<usize, Arc<RenderImage>>,
    /// preview_data 的 FIFO 淘汰顺序
    preview_order: VecDeque<usize>,
    /// 全分辨率图按 capture 索引缓存：放大超过预览分辨率或 100% 时按需加载（同样预解码）
    pub fullres_data: HashMap<usize, Arc<RenderImage>>,
    /// fullres_data 的 FIFO 淘汰顺序（单张 GPU 纹理可达百 MB，上限极小）
    fullres_order: VecDeque<usize>,
    /// 网格虚拟列表滚动句柄（uniform_list 滚动条用）
    pub grid_scroll_handle: UniformListScrollHandle,
    /// 预览图片区实测边界（窗口坐标 x/y/w/h；canvas prepaint 写入，变化时 defer notify 重排；w=0 = 未测量）
    pub preview_area_bounds: Rc<RefCell<(f32, f32, f32, f32)>>,
    /// 预览缩略图条滚动句柄（track_scroll 记录子项位置，scroll_to_item 跟随焦点）
    pub filmstrip_scroll: ScrollHandle,
    /// 字体选择下拉状态（设置弹窗用）
    pub font_select: Entity<SelectState<SearchableVec<SharedString>>>,
    /// 鸟种多选下拉状态（筛选栏用，ComboboxState 创建需要 Window，
    /// 鸟名列表变化时置 dirty 标记，由筛选栏 render 时重建）
    pub bird_select: Entity<ComboboxState<SearchableVec<String>>>,
    /// 当前 captures 中去重排序后的鸟种中文名（鸟种下拉的数据源）
    pub bird_options: Vec<String>,
    /// 鸟种下拉待重建标记（扫描/识别完成或外部清除筛选时置位）
    pub bird_options_dirty: bool,
    pub show_settings: bool,
    /// 设置弹窗独立 View（拥有自身 render 生命周期，避免每帧重建）
    pub settings_overlay: Option<gpui::Entity<SettingsOverlay>>,
    /// 网格视图筛选栏展开状态（默认折叠，仅折叠态一行摘要）
    pub filter_bar_expanded: bool,
    pub scan_task: Option<Task<()>>,
    /// 左侧边栏当前显示的侧栏 tab：0=文件树，1=文件操作
    pub sidebar_section: usize,
    /// 左侧边栏显隐（由 Activity Rail 或快捷键切换）
    pub sidebar_visible: bool,
    /// 正在后台加载中的预览图 capture 索引（防重复 spawn）
    preview_loading: HashSet<usize>,
    /// 预览图加载的合作式取消令牌：焦点离开后取消未完成的慢解码（RAW 完整解码）
    preview_cancel: HashMap<usize, Arc<AtomicBool>>,
    /// 正在后台加载中的全分辨率图 capture 索引（防重复 spawn）
    fullres_loading: HashSet<usize>,
    /// 正在后台加载中的网格缩略图 capture 索引（防重复 spawn）
    grid_loading: HashSet<usize>,
    /// 预览缩放倍率（1.0 = 适配窗口）
    pub preview_zoom: f32,
    /// 预览平移偏移（像素），缩放后拖拽移动图片
    pub preview_pan: (f32, f32),
    /// 拖拽起始状态：(鼠标x, 鼠标y, 起始pan_x, 起始pan_y)
    pub preview_drag: Option<(f32, f32, f32, f32)>,
    /// Shift+拖拽手动框选中：(起始x, 起始y, 当前x, 当前y)（窗口坐标）
    pub box_draw: Option<(f32, f32, f32, f32)>,
    /// 已提交、等待识别结果的手动框（归一化坐标，渲染「识别中」overlay）
    pub pending_region: Option<BBox>,
    // ── 批量文件操作 ──
    pub batch_compare_dir: String,
    pub batch_source_format: String,
    pub batch_compare_format: String,
    pub batch_op_type: BatchOpType,
    pub batch_op_dropdown_open: bool,
    pub batch_source_fmt_open: bool,
    pub batch_compare_fmt_open: bool,
    pub batch_results: Vec<String>,
    pub batch_in_progress: bool,
    pub batch_progress: Option<(u32, u32)>,
    pub batch_progress_msg: String,
    pub batch_show_progress_popup: bool,
    // ── 识别 ──
    /// 单张识别中的 capture index（None=空闲）
    pub recognizing_single: Option<usize>,
    /// 当前识别阶段文本（检测中/分类中/名录映射中）
    pub recognize_stage: Option<String>,
    /// 批量识别进行中
    pub batch_recognizing: bool,
    /// 批量识别进度 (已处理, 总数)
    pub batch_progress_rc: (usize, usize),
    /// 批量识别当前处理的文件名
    pub batch_current_file: String,
    /// 侧栏文件夹卡片右键菜单的目标目录（由卡片 context_menu 闭包设置）
    pub folder_menu_dir: Option<String>,
    /// 识别统计数据 (已识别, 无鸟, 待复核)
    pub batch_counts: (usize, usize, usize),
    /// 批量取消标志
    pub batch_cancel: Arc<AtomicBool>,
    /// DB 同步进度 (done, total)；None = 未在同步
    pub sync_progress: Option<(usize, usize)>,
    /// 检测框显隐
    pub bbox_visible: bool,
    /// 显示全量识别确认对话框
    pub show_recognize_all_confirm: bool,
    /// 焦点图片的完整识别记录
    pub focused_recognition: Option<photo_domain::Recognition>,
}

impl RootView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>, config_path: PathBuf, mut config: AppConfig) -> Self {
        let cache_dir = config_path
            .parent()
            .unwrap_or(Path::new("."))
            .join("thumbnails");
        let thumbnail_cache = Some(ThumbnailCache::new(cache_dir));

        let worker = Worker::new();
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

        // 鸟种多选下拉：初始为空（尚未扫描），鸟名列表变化后由筛选栏 render 重建
        let bird_select = Self::new_bird_select(&[], &[], window, cx);

        // 旧配置 left_panel_width==0 是旧的「收起」语义：恢复默认宽度，启动时隐藏侧栏
        let sidebar_visible = config.left_panel_width > 0;
        if config.left_panel_width == 0 {
            config.left_panel_width = 180;
        }
        if config.right_panel_width == 0 {
            config.right_panel_width = 200;
        }
        // 启动后自动扫描上次打开的目录
        let auto_dir = config.last_directory.clone();
        let mut this = Self {
            config,
            config_path,
            folder_db: None,
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
            preview_cancel: HashMap::new(),
            fullres_loading: HashSet::new(),
            grid_loading: HashSet::new(),
            preview_zoom: 1.0,
            preview_pan: (0.0, 0.0),
            preview_drag: None,
            box_draw: None,
            pending_region: None,
            thumbnail_data: HashMap::new(),
            preview_data: HashMap::new(),
            preview_order: VecDeque::new(),
            fullres_data: HashMap::new(),
            fullres_order: VecDeque::new(),
            font_select,
            bird_select,
            bird_options: Vec::new(),
            bird_options_dirty: false,
            show_settings: false,
            settings_overlay: None,
            filter_bar_expanded: false,
            sidebar_section: 0,
            sidebar_visible,
            grid_scroll_handle: UniformListScrollHandle::new(),
            preview_area_bounds: Rc::new(RefCell::new((0., 0., 0., 0.))),
            filmstrip_scroll: ScrollHandle::default(),
            batch_compare_dir: String::new(),
            batch_source_format: String::new(),
            batch_compare_format: String::new(),
            batch_op_type: BatchOpType::CopySame,
            batch_op_dropdown_open: false,
            batch_source_fmt_open: false,
            batch_compare_fmt_open: false,
            batch_results: Vec::new(),
            batch_in_progress: false,
            batch_progress: None,
            batch_progress_msg: String::new(),
            batch_show_progress_popup: false,
            recognizing_single: None,
            recognize_stage: None,
            batch_recognizing: false,
            batch_progress_rc: (0, 0),
            batch_current_file: String::new(),
            batch_counts: (0, 0, 0),
            batch_cancel: Arc::new(AtomicBool::new(false)),
            sync_progress: None,
            bbox_visible: true,
            show_recognize_all_confirm: false,
            focused_recognition: None,
            folder_menu_dir: None,
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

    /// 弹出目录选择对话框，选择对比目录用于批量文件操作
    pub fn pick_batch_compare_dir(&mut self, cx: &mut Context<Self>) {
        self.worker.spawn(
            cx,
            move || rfd::FileDialog::new().pick_folder(),
            move |this, result, cx| {
                if let Some(path) = result {
                    this.batch_compare_dir = path.display().to_string();
                    cx.notify();
                }
            },
        );
    }

    pub fn scan_directory(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        // Cancel any existing scan
        if let Some(task) = self.scan_task.take() {
            drop(task);
        }
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
                                let mut meta = CaptureMeta::from(c);
                                // CaptureMeta::from 占位 index=0，必须用 captures 中的实际位置，
                                // 预览图/EXIF 回填/缩略图缓存都以它为键
                                meta.index = i;
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
                                    let exif = cache
                                        .get_or_extract_exif(&primary.path, &primary.format)
                                        .unwrap_or_default();
                                    meta.enrich_with_exif(&exif);
                                }
                                meta
                            })
                            .collect();
                        (path, metas, folder_db, entries)
                    })
            },
            |this, result, cx| {
                match result {
                    Ok((dir, metas, cache, entries)) => {
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
                        this.thumbnail_data.clear();
                        this.preview_data.clear();
                        this.preview_order.clear();
                        this.fullres_data.clear();
                        this.fullres_order.clear();
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
                    let fmt = photo_domain::ImageFormat::from_extension(&ext);
                    let format = match fmt {
                        Some(f) => f,
                        None => return None,
                    };
                    photo_engine::exif::extract_exif(&path_for_worker, &format).ok()
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

    /// 执行批量文件操作（在工作线程中运行，不阻塞 UI）
    pub fn execute_batch_ops(&mut self, cx: &mut Context<Self>) {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;

        let compare_dir = self.batch_compare_dir.clone();
        let source_format = self.batch_source_format.clone();
        let compare_format = self.batch_compare_format.clone();
        let op_type = self.batch_op_type;
        let source_dir = self.dir_path.clone();

        if compare_dir.is_empty() {
            self.batch_results = vec!["请先选择对比目录".into()];
            cx.notify();
            return;
        }
        let Some(ref src_dir) = source_dir else {
            self.batch_results = vec!["请先打开一个目录".into()];
            cx.notify();
            return;
        };

        self.batch_in_progress = true;
        self.batch_results.clear();
        self.batch_progress = Some((0, 1));
        self.batch_progress_msg = "正在扫描...".into();
        cx.notify();

        let src_dir = src_dir.clone();
        let progress = Arc::new(AtomicU32::new(0));
        let total = Arc::new(AtomicU32::new(0));
        let progress_poll = progress.clone();
        let total_poll = total.clone();

        self.worker.spawn(
            cx,
            move || {
                // 1. 扫描源目录
                let source_captures = match photo_engine::scanner::scan_directory(
                    &src_dir,
                    &Default::default(),
                    None,
                ) {
                    Ok(caps) => caps,
                    Err(e) => return (vec![format!("扫描源目录失败: {e}")], progress, total),
                };

                if source_captures.is_empty() {
                    return (vec!["源目录无匹配文件".into()], progress, total);
                }

                // 2. 查找匹配
                let (matched, unmatched) = match photo_engine::batch_ops::find_matching(
                    &source_captures,
                    Path::new(&compare_dir),
                    if source_format.is_empty() { None } else { Some(source_format.as_str()) },
                    if compare_format.is_empty() { None } else { Some(compare_format.as_str()) },
                ) {
                    Ok(result) => result,
                    Err(e) => return (vec![format!("匹配失败: {e}")], progress, total),
                };

                let indices = if op_type.is_same_match() { matched } else { unmatched };
                if indices.is_empty() {
                    let hint = if op_type.is_same_match() {
                        "对比目录中没有同名文件"
                    } else {
                        "对比目录中没有非同名的文件"
                    };
                    return (vec![hint.into()], progress, total);
                }

                total.store(indices.len() as u32, Ordering::Relaxed);

                let target_dir = if op_type.needs_target_dir() {
                    Some(Path::new(&compare_dir).parent().unwrap_or(Path::new(".")).join("batch_output"))
                } else {
                    None
                };

                let p = progress.clone();
                let results = photo_engine::batch_ops::execute(
                    &source_captures,
                    &indices,
                    op_type,
                    target_dir.as_deref(),
                    move |done, _tot| {
                        p.store(done, Ordering::Relaxed);
                    },
                );
                (results, progress, total)
            },
            move |this, (results, _progress, total), cx| {
                this.batch_in_progress = false;
                this.batch_progress = Some((total.load(Ordering::Relaxed), total.load(Ordering::Relaxed)));
                this.batch_results = results;
                this.batch_progress_msg.clear();
                cx.notify();
            },
        );

        // 轮询进度：每 300ms 检查 atomic 计数器并更新 UI
        let vh = cx.entity().downgrade();
        cx.spawn(|_: WeakEntity<RootView>, cx: &mut AsyncApp| {
            let mut cx = cx.clone();
            async move {
                loop {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(300))
                        .await;
                    let done = progress_poll.load(Ordering::Relaxed);
                    let tot = total_poll.load(Ordering::Relaxed);
                    if let Some(view) = vh.upgrade() {
                        let _ = cx.update_entity(&view, |view: &mut RootView, cx: &mut Context<RootView>| {
                            if view.batch_in_progress {
                                view.batch_progress = Some((done, tot));
                                view.batch_progress_msg = format!("处理中: {done}/{tot}");
                                cx.notify();
                            }
                        });
                    } else {
                        break;
                    }
                }
            }
        })
        .detach();
    }

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
                if let Some(ref fmt_filter) = filter.format_filter {
                    if meta.primary_format != fmt_filter.to_string() {
                        return false;
                    }
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
                    // no flag in DB means no flag set
                    if meta.flag.is_none() {
                        return false;
                    }
                }
                // min_rating, color_label, flag_filter available via xmp_meta DB now
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
        for di in keep_start..=keep_end {
            self.spawn_preview_load(di, cx);
        }
    }

    /// 为指定 display_order 索引 spawn 预览图加载任务（已缓存或已在加载则跳过）
    fn spawn_preview_load(&mut self, display_idx: usize, cx: &mut Context<Self>) {
        use photo_domain::{ImageFormat as DomainFormat, SourceFile};

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

            file_size: meta.file_size,
        };
        let cancel = Arc::new(AtomicBool::new(false));
        self.preview_cancel.insert(capture_idx, cancel.clone());
        self.worker.spawn(
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
        self.worker.spawn(
            cx,
            move || {
                if is_raw {
                    // u32::MAX = 不缩放：内嵌全尺寸 JPEG 原样入磁盘缓存，下次秒开
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

        self.worker.spawn(
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

                file_size: capture.file_size,
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
                if let Some(bytes) = result {
                    this.thumbnail_data
                        .insert(ci, Arc::new(Image::from_bytes(gpui::ImageFormat::Jpeg, bytes)));
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
        // 焦点变化 → 右侧识别卡片同步刷新（廉价 SQLite 点查）
        self.refresh_focused_recognition();

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
                for (path, _old) in &paths {
                    let result = (|| -> Result<(), photo_engine::folder_db::FolderDbError> {
                        let mut meta = db.get_xmp(path)?.unwrap_or_default();
                        meta.set_rating(rating);
                        db.put_xmp(path, &meta)?;
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

        let folder_db = self.folder_db.clone();
        self.worker.spawn(
            cx,
            move || {
                let Some(db) = &folder_db else { return vec![]; };
                let mut results = Vec::new();
                for (path, _old) in &paths {
                    let result = (|| -> Result<(), photo_engine::folder_db::FolderDbError> {
                        let mut meta = db.get_xmp(path)?.unwrap_or_default();
                        meta.set_flag(flag);
                        db.put_xmp(path, &meta)?;
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

        let folder_db = self.folder_db.clone();
        self.worker.spawn(
            cx,
            move || {
                let Some(db) = &folder_db else { return vec![]; };
                let mut results = Vec::new();
                for (path, _old) in &paths {
                    let result = (|| -> Result<(), photo_engine::folder_db::FolderDbError> {
                        let mut meta = db.get_xmp(path)?.unwrap_or_default();
                        meta.set_color_label(label);
                        db.put_xmp(path, &meta)?;
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
                this.refresh_focused_recognition();
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
        tracing::info!("toggle_view_mode: focus_index={:?}, view_mode={:?}", self.focus_index, self.view_mode);
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
            Action::Next | Action::Prev | Action::First | Action::Last => {
                let old_idx = self.focus_index;
                match action {
                    Action::Next => {
                        if let Some(idx) = self.focus_index {
                            if idx + 1 < self.display_order.len() {
                                self.focus_index = Some(idx + 1);
                            }
                        } else if !self.display_order.is_empty() {
                            self.focus_index = Some(0);
                        }
                    }
                    Action::Prev => {
                        if let Some(idx) = self.focus_index {
                            if idx > 0 {
                                self.focus_index = Some(idx - 1);
                            }
                        } else if !self.display_order.is_empty() {
                            self.focus_index = Some(0);
                        }
                    }
                    Action::First => {
                        if !self.display_order.is_empty() {
                            self.focus_index = Some(0);
                        }
                    }
                    Action::Last => {
                        if !self.display_order.is_empty() {
                            self.focus_index = Some(self.display_order.len() - 1);
                        }
                    }
                    _ => unreachable!(),
                }
                // 焦点变化时刷新 focused_recognition
                if self.focus_index != old_idx {
                    self.refresh_focused_recognition();
                }
                self.preview_zoom = 1.0;
                self.preview_pan = (0.0, 0.0);
                // 预览模式下预加载新图片 + 滚动缩略图条
                if self.view_mode == crate::state::app::ViewMode::Preview
                    && self.focus_index != old_idx
                {
                    self.ensure_preview_loaded(cx);
                    if let Some(i) = self.focus_index {
                        self.scroll_filmstrip_to(i);
                    }
                    self.ensure_filmstrip_thumbs_loaded(cx);
                }
                cx.notify();
            }
            // View
            Action::ToggleGridPreview => {
                self.toggle_view_mode(cx);
                cx.notify();
            }
            Action::ZoomIn => {
                self.zoom_step(true, None, cx);
            }
            Action::ZoomOut => {
                self.zoom_step(false, None, cx);
            }
            Action::ZoomToFit => {
                self.preview_zoom = 1.0;
                self.preview_pan = (0.0, 0.0);
                cx.notify();
            }
            Action::ZoomActual => {
                self.zoom_to_actual(cx);
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
                self.delete_selected(cx);
            }
            // Recognition
            Action::Recognize => {
                if self.selected.len() > 1 {
                    self.recognize_selected(cx);
                } else {
                    self.recognize_single(cx);
                }
            }
            Action::ToggleBbox => {
                self.bbox_visible = !self.bbox_visible;
                tracing::info!("检测框显隐切换: {}", self.bbox_visible);
                cx.notify();
            }
            // 文件夹卡片右键：加入/取消收藏（作用于 folder_menu_dir）
            Action::ToggleContextDirFavorite => {
                if let Some(dir) = self.folder_menu_dir.clone() {
                    if self.config.favorite_dirs.iter().any(|d| d == &dir) {
                        self.config.favorite_dirs.retain(|d| d != &dir);
                    } else {
                        self.config.favorite_dirs.push(dir);
                    }
                    self.save_config();
                    cx.notify();
                }
            }
            // 文件夹卡片右键：从列表移除（同时清出最近打开与收藏夹）
            Action::RemoveContextDir => {
                if let Some(dir) = self.folder_menu_dir.clone() {
                    self.config.recent_directories.retain(|d| d != &dir);
                    self.config.favorite_dirs.retain(|d| d != &dir);
                    self.save_config();
                    cx.notify();
                }
            }
            Action::RecognizeUnrecognized => {
                let dir = self.dir_path.clone();
                let folder_db = self.folder_db.clone();
                let Some(dir) = dir else { return };
                let Some(ref _db) = folder_db else { return };

                // 收集无识别记录的 capture
                let mut targets: Vec<(usize, photo_domain::Capture)> = Vec::new();
                for meta in &self.captures {
                    if meta.recognition_status.is_some() {
                        continue;
                    }
                    if let Some(cap) = self.build_capture_from_meta(meta) {
                        targets.push((meta.index, cap));
                    }
                }
                if targets.is_empty() {
                    self.show_toast("所有图片已有识别结果", cx);
                    return;
                }

                let total = targets.len();
                self.batch_recognizing = true;
                self.batch_progress_rc = (0, total);
                self.batch_current_file = String::new();
                self.batch_counts = (0, 0, 0);
                self.batch_cancel.store(false, std::sync::atomic::Ordering::Relaxed);
                cx.notify();

                self.spawn_batch_recognize(targets, dir, cx);
            }
            Action::RecognizeAll => {
                self.show_recognize_all_confirm = true;
                tracing::info!("显示全量识别确认对话框");
                cx.notify();
            }
            Action::ConfirmRecognizeAll => {
                self.show_recognize_all_confirm = false;
                let dir = self.dir_path.clone();
                let folder_db = self.folder_db.clone();
                let Some(dir) = dir else { return };

                // 先清空目标范围的所有旧识别行
                if let Some(ref db) = folder_db {
                    let rel_paths: Vec<String> = self.captures.iter().filter_map(|meta| {
                        let primary = std::path::Path::new(&meta.primary_path);
                        primary.strip_prefix(&dir).ok().map(|rel| {
                            rel.to_string_lossy().replace('\\', "/")
                        })
                    }).collect();
                    let _ = photo_engine::ops::sync_delete_recognitions(db, &rel_paths);
                }

                let mut targets: Vec<(usize, photo_domain::Capture)> = Vec::new();
                for meta in &self.captures {
                    if let Some(cap) = self.build_capture_from_meta(meta) {
                        targets.push((meta.index, cap));
                    }
                }
                let total = targets.len();
                self.batch_recognizing = true;
                self.batch_progress_rc = (0, total);
                self.batch_current_file = String::new();
                self.batch_counts = (0, 0, 0);
                self.batch_cancel.store(false, std::sync::atomic::Ordering::Relaxed);
                cx.notify();

                self.spawn_batch_recognize(targets, dir, cx);
            }
            Action::CancelBatchRecognize => {
                self.batch_cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                tracing::info!("批量识别取消已请求");
            }
            Action::SetRecognitionFilter(filter) => {
                self.filter.recognition_filter = filter;
                self.apply_filter_and_sort();
                tracing::info!("设置识别筛选: {:?}", filter);
                cx.notify();
            }
            Action::SetSortBy(sort_by) => {
                self.sort_by = sort_by;
                self.apply_filter_and_sort();
                cx.notify();
            }
            Action::ToggleSortDir => {
                self.sort_dir = match self.sort_dir {
                    SortDirection::Ascending => SortDirection::Descending,
                    SortDirection::Descending => SortDirection::Ascending,
                };
                self.apply_filter_and_sort();
                cx.notify();
            }
            // Other
            Action::Refresh => {
                if let Some(ref dir) = self.dir_path.clone() {
                    self.scan_directory(dir.clone(), cx);
                }
            }
            Action::ToggleLeftPanel => {
                // 切显隐（不动宽度，展开时恢复拖拽后的宽度）
                self.sidebar_visible = !self.sidebar_visible;
                self.save_config();
                cx.notify();
            }
            Action::ToggleRightPanel => {
                self.config.right_panel_visible = !self.config.right_panel_visible;
                self.save_config();
                cx.notify();
            }
            Action::ToggleSettings => {
                self.show_settings = !self.show_settings;
                if self.show_settings {
                    let vh = cx.entity().downgrade();
                    self.settings_overlay = Some(cx.new(|_| SettingsOverlay { vh }));
                } else {
                    self.settings_overlay = None;
                }
                cx.notify();
            }
        }

        // 导航/切换后确保焦点图的预览数据在加载（已加载则直接返回）
        self.ensure_preview_loaded(cx);
        // 预览模式下：缩略图条滚动跟随焦点（居中） + 预载焦点邻域缩略图
        if self.view_mode == ViewMode::Preview {
            if let Some(i) = self.focus_index {
                self.scroll_filmstrip_to(i);
            }
            self.ensure_filmstrip_thumbs_loaded(cx);
        }
    }

    // ========================================================================
    // 识别相关方法
    // ========================================================================

    /// 弹出 toast 通知（右下角浮层，自动消失；Root 已注册，直接走 gpui-component 通知层）
    fn show_toast(&self, msg: impl std::fmt::Display, cx: &mut Context<Self>) {
        use gpui_component::{notification::Notification, WindowExt as _};
        let msg = msg.to_string();
        tracing::info!("Toast: {}", msg);
        // App 级拿窗口句柄：dispatch_action 链路不传 Window，这里从 App 取当前窗口
        if let Some(handle) = cx.windows().first() {
            let _ = handle.update(cx, |_, window, cx| {
                window.push_notification(Notification::new().message(msg), cx);
            });
        }
    }

    /// 从 folder_db 刷新焦点图片的识别记录
    fn refresh_focused_recognition(&mut self) {
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

    /// 创建鸟种多选下拉实体并订阅 Change 事件（选中变化即写回 filter.bird_names）
    fn new_bird_select(
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
    fn refresh_bird_options(&mut self) {
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

    /// 从 CaptureMeta 构建 Capture（供识别管线使用）
    fn build_capture_from_meta(&self, meta: &CaptureMeta) -> Option<photo_domain::Capture> {
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
    fn recognize_single(&mut self, cx: &mut Context<Self>) {
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

        self.recognizing_single = Some(capture_idx);
        self.recognize_stage = Some("检测中".into());
        cx.notify();

        self.worker.spawn(
            cx,
            move || {
                // 工作线程侧懒加载 Recognizer
                let rec_result = (|| -> Result<photo_domain::Recognition, String> {
                    let exe_dir = std::env::current_exe()
                        .map_err(|e| format!("获取 exe 路径失败: {e}"))?
                        .parent()
                        .ok_or_else(|| "无法确定 exe 目录".to_string())?
                        .to_path_buf();
                    let models_dir = exe_dir.join("models");
                    let catalog_db = exe_dir.join("data").join("pica_ref.db");
                    let mut recognizer = photo_recognize::Recognizer::new(&models_dir, &catalog_db)
                        .map_err(|e| format!("加载识别模型失败: {e}"))?;
                    tracing::info!("单张识别开始，推理后端: {}", recognizer.backend());
                    recognizer.recognize(&capture, None)
                        .map_err(|e| format!("识别失败: {e}"))
                })();
                (rec_result, rel_path_clone, capture_idx, dir)
            },
            move |this, (result, rel, idx, _scan_dir), cx| {
                match result {
                    Ok(rec) => {
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
                    Err(e) => {
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

        self.recognizing_single = Some(capture_idx);
        self.recognize_stage = Some("分类中".into());
        cx.notify();

        self.worker.spawn(
            cx,
            move || {
                // 工作线程侧懒加载 Recognizer（与单张识别同路径）
                let rec_result = (|| -> Result<photo_domain::Recognition, String> {
                    let exe_dir = std::env::current_exe()
                        .map_err(|e| format!("获取 exe 路径失败: {e}"))?
                        .parent()
                        .ok_or_else(|| "无法确定 exe 目录".to_string())?
                        .to_path_buf();
                    let models_dir = exe_dir.join("models");
                    let catalog_db = exe_dir.join("data").join("pica_ref.db");
                    let mut recognizer =
                        photo_recognize::Recognizer::new(&models_dir, &catalog_db)
                            .map_err(|e| format!("加载识别模型失败: {e}"))?;
                    tracing::info!("手动框选识别开始，推理后端: {}", recognizer.backend());
                    recognizer
                        .recognize_region(&capture, bbox, None)
                        .map_err(|e| format!("识别失败: {e}"))
                })();
                (rec_result, rel_path_clone, capture_idx, dir)
            },
            move |this, (result, rel, idx, _scan_dir), cx| {
                this.pending_region = None;
                match result {
                    Ok(rec) => {
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
                    Err(e) => {
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
    fn recognize_selected(&mut self, cx: &mut Context<Self>) {
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
    fn spawn_batch_recognize(
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
                    let chunk_size = total.div_ceil(n_threads);
                    // 以引用形式供 move 闭包捕获，避免 FnMut 中移出外层变量
                    let cancel = &cancel;
                    let dir = &dir;
                    let progress_work = &progress_work;
                    let started_counter = &started_counter;
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

                                    let rec_result = match &mut recognizer {
                                        Ok(rec) => rec.recognize(cap, None).map_err(|e| format!("识别失败: {e}")),
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
                this.apply_recognition_results(remaining);

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
                        // drain 已完成结果：逐张刻到网格（db upsert + CaptureMeta enrich）
                        let drained = std::mem::take(&mut *progress.results.lock());
                        if !drained.is_empty() {
                            this.apply_recognition_results(drained);
                            this.refresh_focused_recognition();
                            this.refresh_bird_options();
                            this.apply_filter_and_sort();
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

    /// 应用一批批量识别结果：upsert folder_db + enrich CaptureMeta。
    /// 轮询 drain 与 on_done 兜底共用；状态计数由工作线程侧原子量维护，此处不累加。
    fn apply_recognition_results(&mut self, results: Vec<BatchResult>) {
        for (capture_idx, rel, rec_result) in &results {
            match rec_result {
                Ok(rec) => {
                    if let Some(db) = &self.folder_db {
                        if let Err(e) = db.upsert_recognition(rel, rec) {
                            tracing::error!("写入识别结果失败 {}: {e}", rel);
                        }
                    }
                    if let Some(meta) = self.captures.iter_mut().find(|m| m.index == *capture_idx) {
                        meta.enrich_with_recognition(rec);
                    }
                }
                Err(e) => {
                    tracing::error!("批量识别错误 {}: {e}", rel);
                }
            }
        }
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
        if let Err(e) = photo_config::save_config(&self.config_path, &self.config) {
            tracing::error!("Failed to save config: {e}");
        }
    }
}


impl Render for RootView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        crate::ui::layout::render_layout(self, window, cx)
    }
}

#[cfg(test)]
mod tests {
    // 不用 super::*：gpui 根导出的 test 属性宏会遮蔽内建 #[test] 导致宏展开递归
    use super::{
        clamp_pan_axis, pan_after_cursor_zoom, preview_center_offset, window_pos_to_image_norm,
    };

    #[test]
    fn test_decode_render_image_bgra_channel_order() {
        // 回归：RenderImage 帧必须 BGRA（gpui 约定）。用纯色 JPEG 验证通道序，
        // 防止红蓝对调导致画面整体偏青（曾作为“白平衡不对”出现）。
        let mut jpeg = std::io::Cursor::new(Vec::new());
        image::RgbImage::from_fn(8, 8, |_, _| image::Rgb([250, 10, 20]))
            .write_to(&mut jpeg, image::ImageFormat::Jpeg)
            .unwrap();
        let img = super::decode_render_image(&jpeg.into_inner(), true).expect("JPEG 应解码成功");
        let bytes = img.as_bytes(0).expect("应有首帧");
        let (b, g, r, a) = (bytes[0] as i32, bytes[1] as i32, bytes[2] as i32, bytes[3] as i32);
        assert!(r > 200, "BGRA[2]=R 应 ≈250，实际 {r}");
        assert!(b < 60, "BGRA[0]=B 应 ≈20，实际 {b}");
        assert!(g < 60, "BGRA[1]=G 应 ≈10，实际 {g}");
        assert_eq!(a, 255, "alpha 应为 255");
    }

    #[test]
    fn test_center_offset_small_image_centers() {
        // 图片小于容器：居中偏移为正（(1000-400)/2 = 300）
        assert_eq!(preview_center_offset(400., 1000.), 300.);
    }

    #[test]
    fn test_center_offset_large_image_overflows() {
        // 图片大于容器：偏移为负（左上溢出）
        assert_eq!(preview_center_offset(2000., 1000.), -500.);
        assert_eq!(preview_center_offset(4000., 100.), -1950.);
    }

    #[test]
    fn test_clamp_pan_small_image_forces_zero() {
        // 图片 ≤ 容器：任何平移都被拉回 0（保持居中）
        assert_eq!(clamp_pan_axis(400., 1000., 123.), 0.);
        assert_eq!(clamp_pan_axis(1000., 1000., -50.), 0.);
    }

    #[test]
    fn test_clamp_pan_large_image_keeps_edges_out() {
        // 图片 2000 容器 1000：center=-500，pan ∈ [1000-2000-(-500), 500] = [-500, 500]
        assert_eq!(clamp_pan_axis(2000., 1000., 999.), 500.);
        assert_eq!(clamp_pan_axis(2000., 1000., -999.), -500.);
        assert_eq!(clamp_pan_axis(2000., 1000., 100.), 100.);
    }

    #[test]
    fn test_cursor_zoom_keeps_point_fixed() {
        // 容器中心光标缩放：pan 不变（中心点不动）
        let container = (1000., 800.);
        let old_disp = (500., 400.);
        let new_disp = (1000., 800.);
        let cursor = (500., 400.);
        let pan = pan_after_cursor_zoom(old_disp, new_disp, container, (0., 0.), cursor);
        assert!(pan.0.abs() < 1e-4 && pan.1.abs() < 1e-4, "中心缩放不应移动 pan: {pan:?}");

        // 非中心光标：缩放前后光标下的图像点（分数坐标）不变
        let cursor = (200., 200.);
        let pan = pan_after_cursor_zoom(old_disp, new_disp, container, (10., -20.), cursor);
        let frac_before = (
            (cursor.0 - (preview_center_offset(old_disp.0, container.0) + 10.)) / old_disp.0,
            (cursor.1 - (preview_center_offset(old_disp.1, container.1) - 20.)) / old_disp.1,
        );
        let frac_after = (
            (cursor.0 - (preview_center_offset(new_disp.0, container.0) + pan.0)) / new_disp.0,
            (cursor.1 - (preview_center_offset(new_disp.1, container.1) + pan.1)) / new_disp.1,
        );
        assert!((frac_before.0 - frac_after.0).abs() < 1e-4, "x 分数坐标应不变");
        assert!((frac_before.1 - frac_after.1).abs() < 1e-4, "y 分数坐标应不变");
    }

    #[test]
    fn test_window_pos_to_image_norm_fit_mode() {
        // 适配模式（zoom=1.0）：500×300 图放入 1000×600 容器（area 含 16px 内边距）
        // 图片左上角窗口坐标 = (0+16+250, 0+16+150) = (266, 166)
        let area = (0., 0., 1032., 632.);
        let img = (500, 300);
        let top_left = window_pos_to_image_norm(266., 166., area, img, 1.0, (0., 0.)).unwrap();
        assert!(top_left.0.abs() < 1e-4 && top_left.1.abs() < 1e-4, "左上角应映射 (0,0): {top_left:?}");
        let center = window_pos_to_image_norm(516., 316., area, img, 1.0, (0., 0.)).unwrap();
        assert!((center.0 - 0.5).abs() < 1e-4 && (center.1 - 0.5).abs() < 1e-4, "图中心应映射 (0.5,0.5): {center:?}");
    }

    #[test]
    fn test_window_pos_to_image_norm_outside_gives_out_of_range() {
        // 图片外点击不归一化钳制（由调用方 BBox::new 钳制）：左侧点击 x < 0
        let area = (0., 0., 1032., 632.);
        let (nx, _) = window_pos_to_image_norm(16., 316., area, (500, 300), 1.0, (0., 0.)).unwrap();
        assert!(nx < 0., "图片左侧点击 x 应为负: {nx}");
    }

    #[test]
    fn test_window_pos_to_image_norm_zoomed_with_pan() {
        // 2x 缩放 + 平移：disp = 1000×600 恰好填满容器，居中偏移为 0
        // 图片左上角 = (16 + pan.0, 16 + pan.1) = (-84, -34)
        let area = (0., 0., 1032., 632.);
        let img = (500, 300);
        let (nx, ny) = window_pos_to_image_norm(16., 16., area, img, 2.0, (-100., -50.)).unwrap();
        assert!((nx - 0.1).abs() < 1e-4, "x 应为 0.1: {nx}");
        assert!((ny - 0.0833).abs() < 1e-3, "y 应约为 0.083: {ny}");
    }

    #[test]
    fn test_window_pos_to_image_norm_no_measurement_returns_none() {
        // 图片区尚未实测（首帧前）→ None，调用方不得触发识别
        assert!(window_pos_to_image_norm(100., 100., (0., 0., 0., 0.), (500, 300), 1.0, (0., 0.)).is_none());
    }
}
