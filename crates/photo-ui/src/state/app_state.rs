//! 应用主状态定义与状态机（对应 §3、§5、§8）。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use photo_config::AppConfig;
use photo_domain::{CaptureMeta, SortBy, SortDirection};
use photo_engine::folder_db::FolderDb;
use photo_engine::global_db::GlobalDb;
use photo_engine::undo::OpJournal;

use gpui_kit::component::dock::{DockArea, DockEvent, DockPlacement};
use gpui_kit::component::input::InputState;
use gpui_kit::{App, Context, Entity, Subscription, Window};

use super::import::ImportState;
use crate::image::ImageManager;
use crate::model::best_frame::pick_best_frame;
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
    Compare,
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
    Correct(usize),
    BurstConfirm,
    BatchConfirm(photo_domain::BatchOpType),
}

/// 设置弹窗子 Tab（§9.10）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsTab {
    #[default]
    General,
    Shortcuts,
    About,
}

/// 应用核心权威与派生状态
pub struct AppState {
    pub settings_tab: SettingsTab,
    // ── 权威副本 ──
    pub current_dir: Option<PathBuf>,
    pub items: Vec<CaptureMeta>,
    pub folder_db: Option<FolderDb>,
    pub global_db: Option<GlobalDb>,
    pub app_config: AppConfig,
    pub image_manager: ImageManager,
    pub op_journal: OpJournal,

    // ── 派生管线 ──
    pub criteria: FilterCriteria,
    pub sort_by: SortBy,
    pub sort_direction: SortDirection,
    pub quality_scores: HashMap<String, f64>,
    pub display_order: Vec<usize>,
    pub stack_groups: Vec<StackGroup>,
    pub burst_groups: BurstGroupMap,
    pub best_frame_paths: HashSet<String>,

    // ── 选择集（items 下标） ──
    pub selected_indices: Vec<usize>,
    pub anchor_index: Option<usize>,

    // ── 视图状态机 ──
    pub view_mode: ViewMode,
    pub view_before_compare: ViewMode,
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

    // ── 对比状态（2-4 张） ──
    pub compare_indices: Vec<usize>,
    pub compare_focused_slot: usize,

    // ── 幻灯片状态 ──
    pub slideshow_pos: usize,
    pub slideshow_paused: bool,

    // ── Dock 工作区（左/右停靠区 + 中央主视图）与持久化宽度 ──
    pub dock_area: Option<Entity<DockArea>>,
    pub left_panel_width: f32,
    pub right_panel_width: f32,
    pub filter_bar_expanded: bool,
    pub grid_columns: usize,
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

    // ── 导入弹窗（SD 卡 / 目录）──
    pub import: ImportState,
    /// 目标根目录输入框（创建需要 Window）
    pub import_dest_input: Option<Entity<InputState>>,
    /// 重命名模板输入框（创建需要 Window）
    pub import_rename_input: Option<Entity<InputState>>,
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
            app_config,
            image_manager: ImageManager::new(None),
            op_journal: OpJournal::new(),

            criteria: default_filter_criteria(),
            sort_by: SortBy::FileName,
            sort_direction: SortDirection::Ascending,
            quality_scores: HashMap::new(),
            display_order: Vec::new(),
            stack_groups: Vec::new(),
            burst_groups: HashMap::new(),
            best_frame_paths: HashSet::new(),

            selected_indices: Vec::new(),
            anchor_index: None,

            view_mode: ViewMode::Grid,
            view_before_compare: ViewMode::Grid,
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

            compare_indices: Vec::new(),
            compare_focused_slot: 0,

            slideshow_pos: 0,
            slideshow_paused: false,

            dock_area: None,
            left_panel_width,
            right_panel_width,
            filter_bar_expanded: false,
            grid_columns: 4,
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
            import: ImportState::default(),
            import_dest_input: None,
            import_rename_input: None,
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

    /// 应用当前配置里的主题（seed + 明暗）并写回 config.toml。
    pub fn apply_theme(&mut self, window: Option<&mut Window>, cx: &mut App) {
        let dark = matches!(self.app_config.theme, photo_config::Theme::Dark);
        let seed = self.app_config.accent_color.clone();
        crate::theme::apply(seed.as_deref(), dark, window, cx);
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
    /// 对比态 -> 仅聚焦格那一张
    /// 幻灯片态 -> 当前显示张
    /// 其余 -> 选中集
    pub fn mark_indices(&self) -> Vec<usize> {
        match self.view_mode {
            ViewMode::Compare => {
                if let Some(&idx) = self.compare_indices.get(self.compare_focused_slot) {
                    vec![idx]
                } else {
                    Vec::new()
                }
            }
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
            Some(&self.quality_scores),
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
        // 4. 对比态 -> 退出对比
        if self.view_mode == ViewMode::Compare {
            self.view_mode = self.view_before_compare;
            return true;
        }
        // 5. 幻灯片态 -> 退出幻灯片
        if self.view_mode == ViewMode::Slideshow {
            self.view_mode = self.view_before_slideshow;
            return true;
        }
        // 6. 统计态 -> 退出统计回网格
        if self.view_mode == ViewMode::Stats {
            self.view_mode = ViewMode::Grid;
            return true;
        }
        // 7. 预览态 -> 返回网格
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
}
