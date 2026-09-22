//! 应用主状态定义与状态机（对应 §3、§5、§8）。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use photo_config::AppConfig;
use photo_domain::{AdjustParams, BBox, CaptureMeta, ImageFormat, SortBy, SortDirection};
use photo_engine::folder_db::FolderDb;
use photo_engine::global_db::GlobalDb;
use photo_engine::undo::OpJournal;

use gpui_kit::component::dock::{DockArea, DockEvent, DockPlacement};
use gpui_kit::component::input::InputState;
use gpui_kit::component::slider::{SliderEvent, SliderState};
use gpui_kit::component::searchable_list::{SearchableListItem, SearchableVec};
use gpui_kit::component::select::SelectState;
use gpui_kit::{
    App, AppContext as _, Context, Entity, SharedString, Subscription, UniformListScrollHandle, Window,
};

use super::import::ImportState;
use crate::image::{ImageManager, source_file_of};
use crate::model::adjust::{
    EXPOSURE_MAX, EXPOSURE_MIN, EXPOSURE_STEP, TONE_MAX, TONE_MIN, TONE_STEP, AdjustField,
};
use crate::model::best_frame::pick_best_frame;
use crate::model::export::ExportDraft;
use crate::model::burst::{BurstGroupMap, compute_burst_groups};
use crate::model::filter::{FilterCriteria, default_filter_criteria};
use crate::model::sort::apply_filter_and_sort;
use crate::model::stacks::{
    STACK_TIME_GAP_MS, StackGroup, group_by_time, group_singles, group_stacks,
};

/// 视图模式（§3.3）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewMode {
    #[default]
    Grid,
    Preview,
    Slideshow,
    Stats,
}

/// 活动弹窗类型（§9.10）
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActiveDialog {
    Settings,
    Import,
    Export,
    Duplicates,
    BurstConfirm,
    BatchConfirm(photo_domain::BatchOpType),
}

/// 设置弹窗子 Tab（§9.10）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsTab {
    #[default]
    General,
    Typography,
    Recognition,
    Shortcuts,
    About,
}

/// 下拉候选项模型（value = 稳定机器值 / label = 显示名，支持中英文实时搜索过滤）
#[derive(Clone, Debug, PartialEq)]
pub struct ChoiceOption {
    pub value: SharedString,
    pub label: SharedString,
}

impl SearchableListItem for ChoiceOption {
    type Value = SharedString;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.value
    }

    fn matches(&self, query: &str) -> bool {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return true;
        }
        self.label.to_lowercase().contains(&q) || self.value.to_lowercase().contains(&q)
    }
}

pub type ChoiceSelectState = SelectState<SearchableVec<ChoiceOption>>;

/// 构造静态下拉选项（value/label 成对，顺序即显示顺序）
pub fn choice_options(items: &[(&str, &str)]) -> SearchableVec<ChoiceOption> {
    SearchableVec::new(
        items
            .iter()
            .map(|(value, label)| ChoiceOption {
                value: (*value).into(),
                label: (*label).into(),
            })
            .collect::<Vec<_>>(),
    )
}

/// 构建包含常用推荐预设与本机所有已安装字体的搜索集合（跨平台全字体枚举）
pub fn build_font_options(current_font: &str, cx: &App) -> SearchableVec<ChoiceOption> {
    let presets: &[(&str, &str)] = &[
        (".SystemUIFont", "系统默认 UI 字体 (.SystemUIFont)"),
        ("Microsoft YaHei UI", "微软雅黑 (Microsoft YaHei UI)"),
        ("PingFang SC", "苹方 (PingFang SC)"),
        ("Noto Sans CJK SC", "思源黑体 (Noto Sans CJK SC)"),
        ("Segoe UI", "Segoe UI (Windows 现代无衬线)"),
        ("Arial", "Arial (标准西文无衬线)"),
        ("Inter", "Inter (现代高易读无衬线)"),
        ("Roboto", "Roboto (Google 规范无衬线)"),
        ("LXGW WenKai", "霞鹜文楷 (LXGW WenKai 开源书体)"),
        ("Fira Code", "Fira Code (等宽编程字体)"),
        ("Cascadia Code", "Cascadia Code (微软等宽字体)"),
        ("JetBrains Mono", "JetBrains Mono (开发者推荐等宽字体)"),
        ("Maple Mono", "Maple Mono (圆角等宽字体)"),
    ];

    let mut options = Vec::new();
    let mut seen = HashSet::new();

    // 1. 常用推荐预设
    for &(val, label) in presets {
        options.push(ChoiceOption {
            value: val.into(),
            label: label.into(),
        });
        seen.insert(val.to_string());
    }

    // 2. 当前已配置字体（若不在预设列表中，插在最前）
    let current_trimmed = current_font.trim();
    if !current_trimmed.is_empty() && !seen.contains(current_trimmed) {
        options.insert(
            0,
            ChoiceOption {
                value: current_trimmed.to_string().into(),
                label: format!("{current_trimmed} (当前配置)").into(),
            },
        );
        seen.insert(current_trimmed.to_string());
    }

    // 3. 从 GPUI TextSystem 枚举当前操作系统中已安装的全部字体（跨 Linux / Windows / macOS）
    let sys_fonts = cx.text_system().all_font_names();
    for font in sys_fonts {
        if !seen.contains(&font) && !font.starts_with('.') {
            options.push(ChoiceOption {
                value: font.clone().into(),
                label: font.clone().into(),
            });
            seen.insert(font);
        }
    }

    SearchableVec::new(options)
}

/// 常驻识别器（懒加载，装配一次 ~1-2s 后复用；批量识别与框选识别共用同一实例）。
/// `Arc<Mutex<Option<..>>>`：克隆后 move 进后台执行器线程；None = 尚未装配。
/// 模型/名录路径在进程内固定，故不需要失效重建。
pub type SharedRecognizer = Arc<parking_lot::Mutex<Option<photo_recognize::Recognizer>>>;

/// 统计页右栏的一张照片记录（跨文件夹汇总）。
/// 缩略图按**照片自己所在目录**的 .pt/thumbs 定位（缓存目录是按文件夹隔离的）。
#[derive(Debug, Clone)]
pub struct StatsPhoto {
    /// 完整路径（folder + rel_path）：tooltip 与跳转用
    pub full_path: PathBuf,
    /// 已就绪的缩略图路径；None = 尚未生成（显示占位）
    pub thumb_path: Option<PathBuf>,
    /// 归属文件夹（跳转定位用）
    pub folder: String,
    /// 相对路径（跳转定位用）
    pub rel_path: String,
}

/// 应用核心权威与派生状态
pub struct AppState {
    pub settings_tab: SettingsTab,
    // ── 权威副本 ──
    pub current_dir: Option<PathBuf>,
    pub items: Vec<CaptureMeta>,
    pub folder_db: Option<FolderDb>,
    pub global_db: Option<GlobalDb>,
    /// 常驻识别器（识别路径懒装配一次后复用，避免每次识别/框选重装 ~1-2s 模型）
    pub recognizer: SharedRecognizer,
    pub app_config: AppConfig,
    pub image_manager: ImageManager,
    pub op_journal: OpJournal,

    // ── 派生管线 ──
    pub criteria: FilterCriteria,
    pub sort_by: SortBy,
    pub sort_direction: SortDirection,
    pub display_order: Vec<usize>,
    pub stack_groups: Vec<StackGroup>,
    pub burst_groups: BurstGroupMap,
    pub best_frame_paths: HashSet<String>,

    // ── 选择集（items 下标） ──
    pub selected_indices: Vec<usize>,
    pub anchor_index: Option<usize>,

    // ── 视图状态机 ──
    pub view_mode: ViewMode,
    pub view_before_slideshow: ViewMode,
    pub map_overlay: bool,

    // ── 预览状态 ──
    /// 预览母版（后台加载，避免渲染层解原图）：(源路径, 已解码句柄)
    pub preview_image: Option<(String, Arc<gpui_kit::Image>)>,
    /// 最近一次已发起的预览母版加载路径（成功/失败都保留，用于去重、避免失败重试风暴）
    pub preview_request: Option<String>,
    /// 1:1 全分辨率图源（RAW 专用：显示尺寸超过母版像素时后台加载）
    pub preview_full: Option<(String, Arc<gpui_kit::Image>)>,
    /// 最近一次已发起的 1:1 全分辨率加载路径（去重；失败保留，避免重试风暴）
    pub preview_full_request: Option<String>,
    pub preview_zoom: f64,
    pub preview_pan: (f64, f64),
    pub show_bbox: bool,
    pub show_focus: bool,
    pub show_clipping: bool,
    /// 预览视口实际容器尺寸 (width, height)；由视口容器 prepaint 动态更新
    pub preview_viewport_size: Option<(f64, f64)>,
    /// 预览图片拖拽平移的鼠标起点 (x, y)
    pub preview_drag_start: Option<(gpui_kit::Pixels, gpui_kit::Pixels)>,
    /// 预览「框选识别」模式开关（工具条按钮 toggle；开启时左键拖拽画框而非平移）
    pub region_select: bool,
    /// 框选拖拽起点（图片视口坐标）；None = 未在拖拽
    pub region_drag_start: Option<(f64, f64)>,
    /// 进行中/最近一次的框选矩形（归一化 0-1，叠加层绘制用）
    pub region_bbox: Option<BBox>,
    /// 框选识别进行中（防重复触发 + 状态栏提示）
    pub region_recognizing: bool,

    // ── 统计页右栏（全局物种统计：选中物种 → 照片记录） ──
    /// 当前选中的物种名（None = 未选，右栏显示引导文案）
    pub stats_selected_species: Option<String>,
    /// 该物种的照片记录（已解析缩略图路径；可能被上限截断，见 stats_photo_total）
    pub stats_photos: Vec<StatsPhoto>,
    /// 该物种照片总数（> stats_photos.len() 表示列表被截断）
    pub stats_photo_total: usize,
    /// 待选中路径：跨文件夹跳转先记下，扫描完成后按此定位选中
    pub pending_select_path: Option<String>,

    // ── 调整状态（ADR 0007：右栏「调整」tab，参数随图入库） ──
    /// 焦点图的调整参数（全零 = 无调整，走未调整母版）
    pub adjust: AdjustParams,
    /// `adjust` 当前所属图片路径；None = 尚未装载
    pub adjust_path: Option<String>,
    /// 有未落盘的改动（350ms 去抖后写库）
    pub adjust_dirty: bool,
    /// 去抖世代：350ms 内又有改动则本世代作废，只保存最后一次
    pub adjust_persist_seq: u64,
    /// 预览渲染世代：只有最新一次结果允许写回 `preview_image`
    pub adjust_render_seq: u64,
    /// 当前 `preview_image` 是否为调整后的图（中性参数时避免无谓重算）
    pub adjust_preview_toned: bool,
    /// 三条滑杆实体（首次渲染调整 tab 时懒创建）
    pub adjust_sliders: Option<AdjustSliders>,

    // ── 幻灯片状态 ──
    pub slideshow_pos: usize,
    pub slideshow_paused: bool,

    // ── Dock 工作区（左/右停靠区 + 中央主视图）与持久化宽度 ──
    pub dock_area: Option<Entity<DockArea>>,
    pub left_panel_width: f32,
    pub right_panel_width: f32,
    pub filter_bar_expanded: bool,
    pub grid_columns: usize,
    /// 网格滚动句柄：每帧交给 `uniform_list.track_scroll`，并给 `Scrollbar` 当数据源
    /// （GPUI 的溢出滚动容器不会自动画滚动条，必须显式绑定）。
    pub grid_scroll: UniformListScrollHandle,
    /// 网格容器实测尺寸（post-layout prepaint 回填）；缩略图边长按它算，
    /// 首帧为空时按窗口与停靠区宽度估算。
    pub grid_viewport_size: Option<(f64, f64)>,
    /// 拖宽去抖保存的世代号（350ms 内多次变更只落盘一次）
    layout_save_generation: u64,
    /// Dock 布局事件订阅（drop 即取消，必须持有）
    _dock_subscription: Option<Subscription>,

    // ── 任务状态机与生成号 ──
    pub scan_generation: u64,
    pub is_scanning: bool,
    pub scan_stage: Option<String>,
    pub scan_done: u32,
    pub scan_total: u32,
    pub scan_cancel: Arc<AtomicBool>,

    // ── 缩略图后台生成管线（§7.1）──
    pub thumb_done: usize,
    pub thumb_total: usize,

    pub is_recognizing: bool,
    pub recognize_done: u32,
    pub recognize_total: u32,
    pub recognize_current: String,
    pub recognize_cancel: Arc<AtomicBool>,

    pub status_message: Option<(String, Instant)>,
    pub active_dialog: Option<ActiveDialog>,

    // ── 历史/收藏目录 ──
    pub favorite_dirs: Vec<PathBuf>,
    pub recent_dirs: Vec<PathBuf>,
    pub subdirs: Vec<PathBuf>,
    pub focus_handle: Option<gpui_kit::FocusHandle>,
    /// 设置页「自定义主题色」输入框（创建需要 Window，故在 AppState::build 里初始化）。
    pub accent_input: Option<Entity<InputState>>,
    /// 设置页「自定义全局字体」输入框（创建需要 Window，故在 AppState::build 里初始化）。
    pub font_input: Option<Entity<InputState>>,
    /// 设置页「全局界面字体」可搜索选择器（创建需要 Window，故在 AppState::build 里初始化）。
    pub font_select: Option<Entity<ChoiceSelectState>>,
    /// 字体选择器确认事件订阅（持有 Subscription 保证事件持续接收）
    pub _font_select_sub: Option<Subscription>,
    /// 筛选栏「排序方式」下拉（创建需要 Window）
    pub sort_select: Option<Entity<ChoiceSelectState>>,
    /// 筛选栏「每行列数」下拉（创建需要 Window）
    pub grid_cols_select: Option<Entity<ChoiceSelectState>>,
    /// 两个下拉的确认事件订阅（持有保证持续接收）
    pub _sort_select_sub: Option<Subscription>,
    pub _grid_cols_select_sub: Option<Subscription>,

    // ── 导入弹窗（SD 卡 / 目录）──
    pub import: ImportState,
    /// 目标根目录输入框（创建需要 Window）
    pub import_dest_input: Option<Entity<InputState>>,
    /// 重命名模板输入框（创建需要 Window）
    pub import_rename_input: Option<Entity<InputState>>,

    // ── 导出（Batch 1.1：engine + UI 接线）──
    /// 导出草稿：目标目录 / 长边 / 质量 / 命名模板
    pub export: ExportDraft,
    /// 目标目录输入框（创建需要 Window）
    pub export_dest_input: Option<Entity<InputState>>,
    /// 命名模板输入框（创建需要 Window）
    pub export_template_input: Option<Entity<InputState>>,
    /// 质量滑杆（懒创建，需要 Window；1-100 步进 1）
    pub export_quality_slider: Option<Entity<SliderState>>,
    /// 质量滑杆 Change 订阅（持有保证持续接收）
    pub _export_quality_sub: Option<Subscription>,
    pub is_exporting: bool,
    pub export_done: u32,
    pub export_total: u32,
    pub export_current: String,
    pub export_cancel: Arc<AtomicBool>,
    /// 逐文件结果（显示名 → 成功路径 / 失败真实错误）；导出完成后对话框展示
    pub export_results: Vec<(String, Result<String, String>)>,
}

/// 调整 tab 的三条滑杆实体 + 事件订阅。
///
/// 懒创建：`SliderState` 需要 `Context` 才建得出来，而 `AppState::new` 里没有；
/// 首次渲染调整 tab 时构造，之后随 AppState 生命周期持有（drop 即自动退订）。
pub struct AdjustSliders {
    pub exposure: Entity<SliderState>,
    pub contrast: Entity<SliderState>,
    pub saturation: Entity<SliderState>,
    /// 三条滑杆的 Change 事件订阅；drop 即取消，必须持有
    _subscriptions: Vec<Subscription>,
}

impl AdjustSliders {
    fn entity(&self, field: AdjustField) -> &Entity<SliderState> {
        match field {
            AdjustField::Exposure => &self.exposure,
            AdjustField::Contrast => &self.contrast,
            AdjustField::Saturation => &self.saturation,
        }
    }
}

impl AppState {
    // ── 调整 tab（ADR 0007）：装载 / 修改 / 去抖落盘 / 预览重算 ──

    /// 确保焦点图的调整参数已装载。面板与预览两处渲染都调用（幂等），
    /// 所以无论从哪条路径切图都不会漏；切图时先把上一张未落盘的一笔写回。
    pub fn ensure_adjustments_loaded(
        &mut self,
        primary_path: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.adjust_path.as_deref() == Some(primary_path) {
            return;
        }
        self.flush_adjustments();
        self.adjust = super::engine_ops::load_adjustments(self, primary_path);
        self.adjust_path = Some(primary_path.to_string());
        self.adjust_dirty = false;
        self.sync_sliders(window, cx);
        self.request_adjust_preview(cx);
    }

    /// 懒创建三条滑杆并接上 Change 事件（首次渲染调整 tab 时）。
    pub fn ensure_adjust_sliders(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.adjust_sliders.is_some() {
            return;
        }
        let exposure = cx.new(|_| {
            SliderState::new()
                .min(EXPOSURE_MIN)
                .max(EXPOSURE_MAX)
                .step(EXPOSURE_STEP)
        });
        let contrast = cx.new(|_| {
            SliderState::new()
                .min(TONE_MIN)
                .max(TONE_MAX)
                .step(TONE_STEP)
        });
        let saturation = cx.new(|_| {
            SliderState::new()
                .min(TONE_MIN)
                .max(TONE_MAX)
                .step(TONE_STEP)
        });

        let subscriptions = vec![
            cx.subscribe(&exposure, |state, _, event, cx| {
                if let SliderEvent::Change(value) = event {
                    state.set_adjust_field(AdjustField::Exposure, value.start(), cx);
                }
            }),
            cx.subscribe(&contrast, |state, _, event, cx| {
                if let SliderEvent::Change(value) = event {
                    state.set_adjust_field(AdjustField::Contrast, value.start(), cx);
                }
            }),
            cx.subscribe(&saturation, |state, _, event, cx| {
                if let SliderEvent::Change(value) = event {
                    state.set_adjust_field(AdjustField::Saturation, value.start(), cx);
                }
            }),
        ];

        self.adjust_sliders = Some(AdjustSliders {
            exposure,
            contrast,
            saturation,
            _subscriptions: subscriptions,
        });
        self.sync_sliders(window, cx);
    }

    /// 滑杆改动 → 更新参数（值没变则不做任何事）并触发落盘 + 预览重算
    pub fn set_adjust_field(&mut self, field: AdjustField, raw: f32, cx: &mut Context<Self>) {
        let mut next = self.adjust;
        field.apply(&mut next, raw);
        if next == self.adjust {
            return;
        }
        self.adjust = next;
        self.on_adjust_changed(cx);
    }

    /// 单项重置（数值 chip 旁的「重置」）
    pub fn reset_adjust_field(
        &mut self,
        field: AdjustField,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if field.is_neutral(&self.adjust) {
            return;
        }
        field.reset(&mut self.adjust);
        self.sync_one_slider(field, window, cx);
        self.on_adjust_changed(cx);
    }

    /// 全部重置
    pub fn reset_all_adjustments(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.adjust.is_neutral() {
            return;
        }
        self.adjust = AdjustParams::default();
        self.sync_sliders(window, cx);
        self.on_adjust_changed(cx);
    }

    /// 参数变化后的统一收尾：立即重绘（数值 chip 跟手）+ 标脏去抖落盘 + 预览重算
    fn on_adjust_changed(&mut self, cx: &mut Context<Self>) {
        self.adjust_dirty = true;
        cx.notify();
        self.schedule_adjust_persist(cx);
        self.request_adjust_preview(cx);
    }

    /// 350ms 去抖后写库（拖动期间每帧都改内存，但不每帧写盘）
    fn schedule_adjust_persist(&mut self, cx: &mut Context<Self>) {
        self.adjust_persist_seq = self.adjust_persist_seq.wrapping_add(1);
        let seq = self.adjust_persist_seq;
        let entity = cx.entity();
        cx.spawn(
            async move |_weak: gpui_kit::WeakEntity<AppState>,
                        async_cx: &mut gpui_kit::AsyncApp| {
                async_cx
                    .background_executor()
                    .timer(Duration::from_millis(350))
                    .await;
                async_cx.update(|cx| {
                    entity.update(cx, |state, _cx| {
                        if state.adjust_persist_seq == seq {
                            state.flush_adjustments();
                        }
                    });
                });
            },
        )
        .detach();
    }

    /// 立即把未落盘的调整参数写进 folder_db（切图 / 去抖到期时调用）
    pub fn flush_adjustments(&mut self) {
        if !self.adjust_dirty {
            return;
        }
        let Some(path) = self.adjust_path.clone() else {
            return;
        };
        let params = self.adjust;
        super::engine_ops::persist_adjustments(self, &path, &params);
        self.adjust_dirty = false;
    }

    /// 重新渲染当前焦点图的调整预览。
    /// 中性参数 + 当前预览本来就是原图 → 无谓的重算直接跳过。
    fn request_adjust_preview(&mut self, cx: &mut Context<Self>) {
        if self.adjust.is_neutral() && !self.adjust_preview_toned {
            return;
        }
        let Some(path) = self.adjust_path.clone() else {
            return;
        };
        let Some(meta) = self.items.iter().find(|m| m.primary_path == path) else {
            return;
        };
        let Some(source) = source_file_of(meta) else {
            return;
        };
        self.adjust_render_seq = self.adjust_render_seq.wrapping_add(1);
        let seq = self.adjust_render_seq;
        let params = self.adjust;
        super::engine_ops::render_adjust_preview(
            cx.entity(),
            self.image_manager.clone(),
            path,
            source,
            params,
            seq,
            cx,
        );
    }

    /// 把内存参数写回滑杆（装载/重置后用，避免滑杆显示与参数不一致）
    fn sync_sliders(&self, window: &mut Window, cx: &mut Context<Self>) {
        for field in AdjustField::ALL {
            self.sync_one_slider(field, window, cx);
        }
    }

    fn sync_one_slider(&self, field: AdjustField, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sliders) = &self.adjust_sliders else {
            return;
        };
        let entity = sliders.entity(field).clone();
        let value = field.slider_value(&self.adjust);
        entity.update(cx, |slider, cx| slider.set_value(value, window, cx));
    }
}

impl Drop for AppState {
    /// 退出前兜底：350ms 去抖还没到期就关窗时，未落盘的一笔在这里写回。
    fn drop(&mut self) {
        self.flush_adjustments();
    }
}

/// 读取便携配置（缺失/损坏回退默认）。启动与 AppState 共用同一份加载逻辑。
pub fn load_app_config() -> AppConfig {
    photo_config::determine_config_path()
        .ok()
        .as_deref()
        .and_then(|path| photo_config::load_config(path).ok())
        .unwrap_or_default()
}

/// 定位模型/名录库/全局索引库的数据根目录
pub fn data_root() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("PHOTO_DATA_DIR").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
    {
        if exe_dir.join("models").exists() {
            return Some(exe_dir);
        }
    }
    let mut starts: Vec<PathBuf> = Vec::new();
    if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
        starts.push(PathBuf::from(manifest));
    }
    if let Ok(cwd) = std::env::current_dir() {
        starts.push(cwd);
    }
    for start in starts {
        let mut dir = Some(start);
        for _ in 0..6 {
            let Some(d) = dir else { break };
            if d.join("models").exists() && d.join("data").join("bird_catalog.db").exists() {
                return Some(d);
            }
            dir = d.parent().map(|p| p.to_path_buf());
        }
    }
    None
}

impl AppState {
    pub fn new() -> Self {
        let app_config = load_app_config();
        let favorite_dirs = app_config.favorite_dirs.iter().map(PathBuf::from).collect();
        let recent_dirs = app_config
            .recent_directories
            .iter()
            .map(PathBuf::from)
            .collect();
        let left_panel_width = app_config.left_panel_width.clamp(200, 480) as f32;
        let right_panel_width = app_config.right_panel_width.clamp(200, 480) as f32;

        let global_db = data_root().and_then(|r| GlobalDb::open(&r.join("data")).ok());

        Self {
            settings_tab: SettingsTab::General,
            current_dir: None,
            items: Vec::new(),
            folder_db: None,
            global_db,
            recognizer: Arc::new(parking_lot::Mutex::new(None)),
            app_config,
            image_manager: ImageManager::new(None),
            op_journal: OpJournal::new(),

            criteria: default_filter_criteria(),
            sort_by: SortBy::FileName,
            sort_direction: SortDirection::Ascending,
            display_order: Vec::new(),
            stack_groups: Vec::new(),
            burst_groups: HashMap::new(),
            best_frame_paths: HashSet::new(),

            selected_indices: Vec::new(),
            anchor_index: None,

            view_mode: ViewMode::Grid,
            view_before_slideshow: ViewMode::Grid,
            map_overlay: false,

            preview_image: None,
            preview_request: None,
            preview_full: None,
            preview_full_request: None,
            preview_zoom: 1.0,
            preview_pan: (0.0, 0.0),
            show_bbox: true,
            show_focus: false,
            show_clipping: false,
            preview_viewport_size: None,
            preview_drag_start: None,
            region_select: false,
            region_drag_start: None,
            region_bbox: None,
            region_recognizing: false,
            stats_selected_species: None,
            stats_photos: Vec::new(),
            stats_photo_total: 0,
            pending_select_path: None,

            adjust: AdjustParams::default(),
            adjust_path: None,
            adjust_dirty: false,
            adjust_persist_seq: 0,
            adjust_render_seq: 0,
            adjust_preview_toned: false,
            adjust_sliders: None,

            slideshow_pos: 0,
            slideshow_paused: false,

            dock_area: None,
            left_panel_width,
            right_panel_width,
            filter_bar_expanded: false,
            grid_columns: 4,
            grid_scroll: UniformListScrollHandle::new(),
            grid_viewport_size: None,
            layout_save_generation: 0,
            _dock_subscription: None,

            scan_generation: 0,
            is_scanning: false,
            scan_stage: None,
            scan_done: 0,
            scan_total: 0,
            scan_cancel: Arc::new(AtomicBool::new(false)),

            thumb_done: 0,
            thumb_total: 0,

            is_recognizing: false,
            recognize_done: 0,
            recognize_total: 0,
            recognize_current: String::new(),
            recognize_cancel: Arc::new(AtomicBool::new(false)),

            status_message: None,
            active_dialog: None,

            favorite_dirs,
            recent_dirs,
            subdirs: Vec::new(),
            focus_handle: None,
            accent_input: None,
            font_input: None,
            font_select: None,
            _font_select_sub: None,
            sort_select: None,
            grid_cols_select: None,
            _sort_select_sub: None,
            _grid_cols_select_sub: None,
            import: ImportState::default(),
            import_dest_input: None,
            import_rename_input: None,
            export: ExportDraft::default(),
            export_dest_input: None,
            export_template_input: None,
            export_quality_slider: None,
            _export_quality_sub: None,
            is_exporting: false,
            export_done: 0,
            export_total: 0,
            export_current: String::new(),
            export_cancel: Arc::new(AtomicBool::new(false)),
            export_results: Vec::new(),
        }
    }

    /// 打开导入弹窗：复位状态、刷新可移动盘、把目标目录预填进输入框。
    pub fn open_import_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.current_dir.clone();
        self.import.reset(current.as_deref());
        let dest = self.import.dest_root.clone();
        if let Some(input) = self.import_dest_input.clone() {
            input.update(cx, |input_state, cx| {
                input_state.set_value(dest.clone(), window, cx);
            });
        }
        if let Some(input) = self.import_rename_input.clone() {
            input.update(cx, |input_state, cx| {
                input_state.set_value("", window, cx);
            });
        }
        self.active_dialog = Some(ActiveDialog::Import);
        cx.notify();
    }

    // ── 导出弹窗（§9.10：预设 / 长边 / 质量 / 命名模板 / 目标目录）──

    /// 打开导出弹窗：预填目标目录（配置记忆 → 否则 `<当前目录>/exports`）与草稿参数。
    /// 目录里一张照片都没有时只提示，不开空弹窗。
    pub fn open_export_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.items.is_empty() {
            self.set_status_message("导出失败：当前目录没有照片");
            cx.notify();
            return;
        }
        // 目标目录：上次用过的（配置）→ 否则当前目录下 exports/
        if self.export.dest_dir.trim().is_empty() {
            self.export.dest_dir = self
                .app_config
                .export_dir
                .clone()
                .or_else(|| {
                    self.current_dir
                        .as_ref()
                        .map(|d| d.join("exports").to_string_lossy().to_string())
                })
                .unwrap_or_default();
        }
        // 上一批的逐文件结果不带到新一次导出里
        self.export_results.clear();
        self.ensure_export_quality_slider(window, cx);
        self.sync_export_inputs(window, cx);
        self.active_dialog = Some(ActiveDialog::Export);
        cx.notify();
    }

    /// 懒创建质量滑杆（1-100 步进 1）并接 Change 事件——滑杆是真实控件，
    /// 值直接落草稿（`{quality}` 只在执行时才钳制）。
    pub fn ensure_export_quality_slider(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.export_quality_slider.is_some() {
            return;
        }
        let slider = cx.new(|_| SliderState::new().min(1.0).max(100.0).step(1.0));
        let sub = cx.subscribe(&slider, |state, _, event, cx| {
            if let SliderEvent::Change(value) = event {
                let quality = value.start().round().clamp(1.0, 100.0) as u8;
                if quality != state.export.quality {
                    state.export.quality = quality;
                    cx.notify();
                }
            }
        });
        self.export_quality_slider = Some(slider);
        self._export_quality_sub = Some(sub);
        self.sync_export_quality_slider(window, cx);
    }

    /// 把草稿的四个参数写进两个输入框与质量滑杆
    ///（打开弹窗、点预设、或从配置重载后调用，避免控件显示与草稿不一致）。
    pub fn sync_export_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dest = self.export.dest_dir.clone();
        if let Some(input) = self.export_dest_input.clone() {
            input.update(cx, |input_state, cx| {
                input_state.set_value(dest, window, cx);
            });
        }
        let template = self.export.template.clone();
        if let Some(input) = self.export_template_input.clone() {
            input.update(cx, |input_state, cx| {
                input_state.set_value(template, window, cx);
            });
        }
        self.sync_export_quality_slider(window, cx);
    }

    /// 把草稿质量写回滑杆。
    pub fn sync_export_quality_slider(&self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(slider) = self.export_quality_slider.clone() else {
            return;
        };
        let value = self.export.quality.clamp(1, 100) as f32;
        slider.update(cx, |slider, cx| slider.set_value(value, window, cx));
    }

    /// 把两个输入框的当前文本同步进草稿（点「开始导出」前调用；滑杆走事件订阅）。
    pub fn read_export_inputs(&mut self, cx: &mut Context<Self>) {
        if let Some(input) = self.export_dest_input.clone() {
            self.export.dest_dir = input.read(cx).value().to_string();
        }
        if let Some(input) = self.export_template_input.clone() {
            self.export.template = input.read(cx).value().to_string();
        }
    }

    // ── Dock 工作区（左右边栏可拖宽 / 可折叠）──

    /// 建 Dock 工作区：左/右停靠区 + 中央主视图；锁定布局（只可拖宽/折叠，不可重排）。
    pub fn init_dock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let area = crate::views::dock_panels::create_dock(
            cx.entity(),
            self.left_panel_width,
            self.right_panel_width,
            window,
            cx,
        );
        // 拖宽/折叠 → 宽度回写配置（350ms 去抖落盘）
        let subscription = cx.subscribe(&area, |this, area, event, cx| {
            if matches!(event, DockEvent::LayoutChanged) {
                this.sync_dock_sizes(&area, cx);
            }
        });
        self._dock_subscription = Some(subscription);
        self.dock_area = Some(area);
    }

    /// 打开/关闭一侧停靠区（Ctrl+[ / Ctrl+] 与活动栏按钮共用）。
    pub fn toggle_dock(
        &mut self,
        placement: DockPlacement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(area) = self.dock_area.clone() else {
            return;
        };
        area.update(cx, |area, cx| area.toggle_dock(placement, window, cx));
    }

    /// 把 Dock 当前宽度同步进配置：内存即时，落盘去抖（拖拽期间每帧都会触发事件）。
    fn sync_dock_sizes(&mut self, area: &Entity<DockArea>, cx: &mut Context<Self>) {
        let (left, right) = {
            let area = area.read(cx);
            (
                area.dock_size(DockPlacement::Left),
                area.dock_size(DockPlacement::Right),
            )
        };
        if let Some(size) = left {
            let value = f32::from(size).round().clamp(200.0, 480.0);
            self.left_panel_width = value;
            self.app_config.left_panel_width = value as u32;
        }
        if let Some(size) = right {
            let value = f32::from(size).round().clamp(200.0, 480.0);
            self.right_panel_width = value;
            self.app_config.right_panel_width = value as u32;
        }
        self.schedule_layout_save(cx);
    }

    /// 350ms 去抖后落盘（期间又有拖宽则本世代作废，只保存最后一次）。
    fn schedule_layout_save(&mut self, cx: &mut Context<Self>) {
        self.layout_save_generation = self.layout_save_generation.wrapping_add(1);
        let generation = self.layout_save_generation;
        let entity = cx.entity();
        cx.spawn(
            async move |_weak: gpui_kit::WeakEntity<AppState>,
                        async_cx: &mut gpui_kit::AsyncApp| {
                async_cx
                    .background_executor()
                    .timer(Duration::from_millis(350))
                    .await;
                async_cx.update(|cx| {
                    entity.update(cx, |state, _cx| {
                        if state.layout_save_generation == generation {
                            state.save_config();
                        }
                    });
                });
            },
        )
        .detach();
    }

    // ── 主题（手册 §6.6）：应用 + 落盘 ──

    /// 应用当前配置里的主题（seed + 明暗 + 字体）并写回 config.toml。
    pub fn apply_theme(&mut self, window: Option<&mut Window>, cx: &mut App) {
        let dark = matches!(self.app_config.theme, photo_config::Theme::Dark);
        let seed = self.app_config.accent_color.clone();
        let font = self.app_config.font_family.clone();
        crate::theme::apply(seed.as_deref(), dark, Some(&font), window, cx);
        self.save_config();
    }

    /// 切换明暗模式（活动栏按钮与设置页共用同一入口）。
    pub fn toggle_theme(&mut self, window: Option<&mut Window>, cx: &mut App) {
        self.app_config.theme = match self.app_config.theme {
            photo_config::Theme::Light => photo_config::Theme::Dark,
            photo_config::Theme::Dark => photo_config::Theme::Light,
        };
        self.apply_theme(window, cx);
    }

    /// 设置全局字体并即刻生效与落盘。
    pub fn set_font_family(&mut self, font: &str, window: Option<&mut Window>, cx: &mut App) {
        let trimmed = font.trim();
        if !trimmed.is_empty() {
            self.app_config.font_family = trimmed.to_string();
            self.apply_theme(window, cx);
        }
    }

    /// 把当前字体家族名回写到自定义字体输入框。
    pub fn sync_font_input(&self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(input) = self.font_input.clone() else {
            return;
        };
        let font = self.app_config.font_family.clone();
        input.update(cx, |state, cx| state.set_value(font, window, cx));
    }

    /// 同步字体下拉选择框的选中项。
    pub fn sync_font_select(&self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(select) = self.font_select.clone() else {
            return;
        };
        let font: SharedString = self.app_config.font_family.clone().into();
        select.update(cx, |state, cx| {
            state.set_selected_value(&font, window, cx);
            if state.selected_value() != Some(&font) && !font.is_empty() {
                // 如果是新输入的自定义字体，刷新选项列表并选中
                let items = build_font_options(&font, cx);
                state.set_items(items, window, cx);
                state.set_selected_value(&font, window, cx);
            }
        });
    }

    /// 设置排序方式（筛选栏下拉的唯一入口；排序变化要重算管线）
    pub fn set_sort_by(&mut self, sort_by: SortBy, cx: &mut Context<Self>) {
        if self.sort_by == sort_by {
            return;
        }
        self.sort_by = sort_by;
        self.recompute_pipeline();
        cx.notify();
    }

    /// 设置网格每行列数（钳制 2–5，写回配置）
    pub fn set_grid_columns(&mut self, cols: usize, cx: &mut Context<Self>) {
        let cols = cols.clamp(2, 5);
        if self.grid_columns == cols {
            return;
        }
        self.grid_columns = cols;
        self.app_config.grid_columns = cols as u32;
        self.save_config();
        cx.notify();
    }

    /// 同步「排序方式」下拉的选中项（排序从别处改动时调用）
    pub fn sync_sort_select(&self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(select) = self.sort_select.clone() else {
            return;
        };
        let value: SharedString = crate::model::sort_by_value(self.sort_by).into();
        select.update(cx, |state, cx| state.set_selected_value(&value, window, cx));
    }

    /// 同步「每行列数」下拉的选中项（设置页改列数时调用）
    pub fn sync_grid_cols_select(&self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(select) = self.grid_cols_select.clone() else {
            return;
        };
        let value: SharedString = self.grid_columns.to_string().into();
        select.update(cx, |state, cx| state.set_selected_value(&value, window, cx));
    }

    /// 设置主题色 seed；格式非法则不改动并返回 false（调用方给用户提示）。
    pub fn set_accent(&mut self, hex: &str, window: Option<&mut Window>, cx: &mut App) -> bool {
        let Some(normalized) = photo_config::normalize_accent_hex(hex) else {
            return false;
        };
        self.app_config.accent_color = Some(normalized);
        self.apply_theme(window, cx);
        true
    }

    /// 把当前 seed 回写到自定义输入框（点预设色块后保持一致）。
    pub fn sync_accent_input(&self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(input) = self.accent_input.clone() else {
            return;
        };
        let hex = crate::theme::resolve_seed(self.app_config.accent_color.as_deref());
        input.update(cx, |state, cx| state.set_value(hex, window, cx));
    }

    /// 把内存里的 AppConfig 落盘。app_config 就是权威运行时副本
    /// （收藏 / 上次目录 / 最近打开都由前端维护），所以整体保存即可。
    pub fn save_config(&self) {
        let Ok(path) = photo_config::determine_config_path() else {
            tracing::warn!("无法定位配置路径，主题设置未落盘");
            return;
        };
        if let Err(err) = photo_config::save_config(&path, &self.app_config.clone().clamped()) {
            tracing::warn!("保存配置失败: {err}");
        }
    }

    /// 主选中项（§5.1）：anchor_index ?? 选中集末尾
    pub fn primary_selected_index(&self) -> Option<usize> {
        self.anchor_index
            .or_else(|| self.selected_indices.last().copied())
    }

    /// 获取主选中项元数据
    pub fn primary_selected_meta(&self) -> Option<&CaptureMeta> {
        self.primary_selected_index()
            .and_then(|i| self.items.get(i))
    }

    /// 标记键作用域（§5.3 markPaths）：
    /// 幻灯片态 -> 当前显示张
    /// 其余 -> 选中集
    pub fn mark_indices(&self) -> Vec<usize> {
        match self.view_mode {
            ViewMode::Slideshow => {
                if let Some(&idx) = self.display_order.get(self.slideshow_pos) {
                    vec![idx]
                } else {
                    Vec::new()
                }
            }
            _ => self.selected_indices.clone(),
        }
    }

    /// 重算派生管线（§3.2 顺序不可换）
    pub fn recompute_pipeline(&mut self) {
        // 1. 筛选与排序 -> display_order
        self.display_order = apply_filter_and_sort(
            &self.items,
            &self.criteria,
            self.sort_by,
            self.sort_direction,
        );

        // 2. 堆叠分组
        self.stack_groups = match self.app_config.stack_mode {
            photo_config::StackMode::None => group_singles(&self.display_order),
            photo_config::StackMode::ByFileName => group_stacks(&self.display_order, &self.items),
            photo_config::StackMode::ByTime => {
                group_by_time(&self.display_order, &self.items, STACK_TIME_GAP_MS)
            }
        };

        // 3. 连拍分组与选优
        let display_metas: Vec<CaptureMeta> = self
            .display_order
            .iter()
            .filter_map(|&i| self.items.get(i).cloned())
            .collect();
        self.burst_groups = compute_burst_groups(&display_metas, STACK_TIME_GAP_MS);

        // 最优帧集合
        self.best_frame_paths.clear();
        let mut group_items: HashMap<String, Vec<CaptureMeta>> = HashMap::new();
        for (&pos, entry) in &self.burst_groups {
            if let Some(meta) = display_metas.get(pos) {
                group_items
                    .entry(entry.group_id.clone())
                    .or_default()
                    .push(meta.clone());
            }
        }
        for (_, members) in group_items {
            if let Some(best) = pick_best_frame(&members) {
                self.best_frame_paths.insert(best);
            }
        }

        // 4. 清理不在筛选结果内的选中项
        let visible_set: HashSet<usize> = self.display_order.iter().copied().collect();
        self.selected_indices.retain(|i| visible_set.contains(i));
        if let Some(anchor) = self.anchor_index {
            if !visible_set.contains(&anchor) {
                self.anchor_index = self.selected_indices.last().copied();
            }
        }
    }

    /// 单击选择
    pub fn select_single(&mut self, item_idx: usize) {
        self.selected_indices = vec![item_idx];
        self.anchor_index = Some(item_idx);
        self.preview_zoom = 1.0;
        self.preview_pan = (0.0, 0.0);
        self.preview_drag_start = None;
    }

    /// Ctrl+单击切换选择
    pub fn toggle_select(&mut self, item_idx: usize) {
        if let Some(pos) = self.selected_indices.iter().position(|&x| x == item_idx) {
            self.selected_indices.remove(pos);
            if self.anchor_index == Some(item_idx) {
                self.anchor_index = self.selected_indices.last().copied();
            }
        } else {
            self.selected_indices.push(item_idx);
            self.anchor_index = Some(item_idx);
        }
    }

    /// Shift+单击范围选择（基于 display_order）
    pub fn select_range(&mut self, target_idx: usize) {
        let Some(anchor) = self.anchor_index else {
            self.select_single(target_idx);
            return;
        };

        let pos_anchor = self.display_order.iter().position(|&x| x == anchor);
        let pos_target = self.display_order.iter().position(|&x| x == target_idx);

        if let (Some(pa), Some(pt)) = (pos_anchor, pos_target) {
            let start = pa.min(pt);
            let end = pa.max(pt);
            self.selected_indices = self.display_order[start..=end].to_vec();
        } else {
            self.select_single(target_idx);
        }
    }

    /// 全选（当前筛选结果）
    pub fn select_all(&mut self) {
        self.selected_indices = self.display_order.clone();
        if self.anchor_index.is_none() {
            self.anchor_index = self.selected_indices.first().copied();
        }
    }

    /// 取消全选
    pub fn deselect_all(&mut self) {
        self.selected_indices.clear();
        self.anchor_index = None;
    }

    /// 在堆叠组间移动导航（<- / ->）
    pub fn navigate_stack_group(&mut self, delta: isize) {
        if self.stack_groups.is_empty() {
            return;
        }

        let cur_active = self.primary_selected_index();
        let cur_group_pos = cur_active
            .and_then(|idx| {
                self.stack_groups
                    .iter()
                    .position(|g| g.members.contains(&idx))
            })
            .unwrap_or(0);

        let new_group_pos = (cur_group_pos as isize + delta)
            .clamp(0, self.stack_groups.len() as isize - 1) as usize;

        let target_group = &self.stack_groups[new_group_pos];
        self.select_single(target_group.active);
    }

    /// 堆叠内切换成员（Q / E）
    pub fn navigate_intra_stack(&mut self, delta: isize) {
        let Some(cur_idx) = self.primary_selected_index() else {
            return;
        };
        let Some(group) = self
            .stack_groups
            .iter_mut()
            .find(|g| g.members.contains(&cur_idx))
        else {
            return;
        };

        if group.members.len() <= 1 {
            return;
        }

        let cur_pos = group
            .members
            .iter()
            .position(|&x| x == cur_idx)
            .unwrap_or(0);
        let new_pos = (cur_pos as isize + delta).rem_euclid(group.members.len() as isize) as usize;

        let new_member = group.members[new_pos];
        group.active = new_member;
        self.select_single(new_member);
    }

    /// 处理 Esc 按键（§5.4 严格按优先级链分发）
    pub fn handle_escape(&mut self) -> bool {
        // 1. 地图浮层打开 -> 关地图
        if self.map_overlay {
            self.map_overlay = false;
            return true;
        }
        // 2. 弹窗打开 -> 关弹窗（除导入有状态外）
        if let Some(dialog) = &self.active_dialog {
            if *dialog != ActiveDialog::Import {
                self.active_dialog = None;
                return true;
            }
        }
        // 3. 识别进行中 -> 取消批量识别
        if self.is_recognizing {
            self.recognize_cancel.store(true, Ordering::Relaxed);
            return true;
        }
        // 4. 幻灯片态 -> 退出幻灯片
        if self.view_mode == ViewMode::Slideshow {
            self.view_mode = self.view_before_slideshow;
            return true;
        }
        // 5. 统计态 -> 退出统计回网格
        if self.view_mode == ViewMode::Stats {
            self.view_mode = ViewMode::Grid;
            return true;
        }
        // 6. 预览态 -> 返回网格
        if self.view_mode == ViewMode::Preview {
            self.view_mode = ViewMode::Grid;
            return true;
        }

        false
    }

    /// 设置状态提示消息（4秒后自动消失）
    pub fn set_status_message(&mut self, msg: impl Into<String>) {
        self.status_message = Some((msg.into(), Instant::now()));
    }

    /// 复制当前照片到系统剪贴板（全尺寸 RGBA）。
    ///
    /// 作用对象与标记键一致：预览/幻灯片取当前那张，网格取主选中项。
    /// 对应已删除的 Tauri 版 `copy_image_to_clipboard` command。
    pub fn copy_current_image_to_clipboard(&mut self, cx: &mut Context<Self>) {
        if self.primary_selected_meta().is_none() {
            self.set_status_message("未选中照片");
            cx.notify();
            return;
        }
        let Some(source) = self.primary_selected_meta().and_then(source_file_of) else {
            self.set_status_message("无法识别该文件的图片格式，未复制");
            cx.notify();
            return;
        };
        if matches!(source.format, ImageFormat::Other) {
            self.set_status_message("视频等非图片格式不支持复制到剪贴板");
            cx.notify();
            return;
        }
        // image_manager 在这里取出后传进去：本函数运行在 listener 里，AppState 正被租借，
        // 引擎侧再 read 同一实体会 panic（与 load_preview_image 的调用口径一致）。
        let manager = self.image_manager.clone();
        self.set_status_message("正在解码全尺寸图片…");
        super::engine_ops::copy_image_to_clipboard(cx.entity().clone(), manager, source, cx);
        cx.notify();
    }
}
