use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::path::PathBuf;

use gpui::*;
use std::sync::LazyLock;
use photo_config::AppConfig;
use photo_domain::{
    BBox, CaptureMeta, ColorLabel, FilterCriteria, Flag, Rating,
    SortBy, SortDirection,
};
use crate::state::batch_ops::BatchDeletePreview;
use gpui_component::combobox::ComboboxState;
use gpui_component::select::SearchableVec;
use photo_engine::thumbnail::ThumbnailCache;
use crate::ui::toolbar::SettingsOverlay;
use photo_engine::folder_db::FolderDb;

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
    /// 网格缩略图按 capture 索引缓存：worker 预解码为 RenderImage（字节源走 GPUI
    /// 异步解码，快速拖动滚动条时解码排队导致渐显慢；RenderImage 到达即可绘制）
    pub thumbnail_data: HashMap<usize, Arc<RenderImage>>,
    /// 预览图按 capture 索引缓存：worker 线程预解码为 RenderImage，
    /// 到达即可绘制（字节源走 GPUI asset 异步解码，首帧会画空白——切换白屏根因）
    pub preview_data: HashMap<usize, Arc<RenderImage>>,
    /// preview_data 的 FIFO 淘汰顺序
    pub(crate) preview_order: VecDeque<usize>,
    /// 全分辨率图按 capture 索引缓存：放大超过预览分辨率或 100% 时按需加载（同样预解码）
    pub fullres_data: HashMap<usize, Arc<RenderImage>>,
    /// fullres_data 的 FIFO 淘汰顺序（单张 GPU 纹理可达百 MB，上限极小）
    pub(crate) fullres_order: VecDeque<usize>,
    /// 网格虚拟列表滚动句柄（uniform_list 滚动条用）
    pub grid_scroll_handle: UniformListScrollHandle,
    /// 预览图片区实测边界（窗口坐标 x/y/w/h；canvas prepaint 写入，变化时 defer notify 重排；w=0 = 未测量）
    pub preview_area_bounds: Rc<RefCell<(f32, f32, f32, f32)>>,
    /// 预览缩略图条滚动句柄（track_scroll 记录子项位置，scroll_to_item 跟随焦点）
    pub filmstrip_scroll: ScrollHandle,
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
    /// 扫描/删除换代计数：加载与回填任务在 spawn 时捕获代数，
    /// on_done 时若代数已过期则丢弃结果，防旧任务按新索引写入（张冠李戴）
    pub(crate) scan_generation: u64,
    /// 扫描进行中（状态栏指示）
    pub(crate) scan_in_progress: bool,
    /// 左侧边栏当前显示的侧栏 tab：0=文件树，1=文件操作
    pub sidebar_section: usize,
    /// 左侧边栏显隐（由 Activity Rail 或快捷键切换）
    pub sidebar_visible: bool,
    /// 正在后台加载中的预览图 capture 索引（防重复 spawn）
    pub(crate) preview_loading: HashSet<usize>,
    /// 预览图加载的合作式取消令牌：焦点离开后取消未完成的慢解码（RAW 完整解码）
    pub(crate) preview_cancel: HashMap<usize, Arc<AtomicBool>>,
    /// 正在后台加载中的全分辨率图 capture 索引（防重复 spawn）
    pub(crate) fullres_loading: HashSet<usize>,
    /// 正在后台加载中的网格缩略图 capture 索引（防重复 spawn）
    pub(crate) grid_loading: HashSet<usize>,
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
    // ── 批量文件操作（ADR 0006：筛选驱动 + 画面粒度）──
    /// 「同步同名文件」开关（默认关）：开启后按 stem 将兄弟文件纳入操作集
    pub batch_sync_enabled: bool,
    /// 同步的格式集合（UI 多选栏，默认全选目录实际格式）
    pub batch_sync_formats: HashSet<String>,
    /// 同步预估额外文件数（UI 显示「+M」，开关/格式/筛选变化时重算）
    pub batch_sync_extra: usize,
    /// 删除确认弹窗数据（None = 未打开）
    pub batch_delete_confirm: Option<BatchDeletePreview>,
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
        // 缩略图缓存跟随扫描目录创建（每文件夹 .pt/thumbs），未扫描时为 None
        let thumbnail_cache = None;

        let worker = Worker::new();

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
            scan_generation: 0,
            scan_in_progress: false,
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
            batch_sync_enabled: false,
            batch_sync_formats: HashSet::new(),
            batch_sync_extra: 0,
            batch_delete_confirm: None,
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
            bbox_visible: false,
            show_recognize_all_confirm: false,
            focused_recognition: None,
            folder_menu_dir: None,
        };

        if let Some(last_dir) = &auto_dir {
            this.scan_directory(PathBuf::from(last_dir), cx);
        }

        this
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
                // 导航时清掉未完成的框选与「识别中」overlay：
                // 窗口坐标对新手焦点图无意义，避免识别错绑到新图片
                self.box_draw = None;
                self.pending_region = None;
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
                // 忙/空守卫：与其他批量识别入口一致，空目标会让 chunks(0) 在
                // rayon 线程 panic 且 on_done 永不执行，batch_recognizing 卡死
                if self.recognizing_single.is_some() || self.batch_recognizing {
                    self.show_toast("识别进行中，稍后再试", cx);
                    cx.notify();
                    return;
                }
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
                if targets.is_empty() {
                    self.show_toast("当前目录没有可识别的图片", cx);
                    cx.notify();
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
    pub(crate) fn show_toast(&self, msg: impl std::fmt::Display, cx: &mut Context<Self>) {
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
    use crate::state::preview_math::{
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
        let img = crate::state::image_cache::decode_render_image(&jpeg.into_inner(), true).expect("JPEG 应解码成功");
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
