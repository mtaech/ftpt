//! gpui-kit Dock 工作区：左右边栏的停靠实现（可拖宽 / 可折叠）。
//!
//! 用官方 Dock 组件（DockArea + DockSkin）搭工作区：
//! - 左停靠区：文件树 / 批量操作（dock 自己的标签栏，替代原来的自绘 TabBar）
//! - 中央：当前主视图（网格 / 预览 / 对比 / 幻灯片 / 统计）
//! - 右停靠区：信息 / 调整
//!
//! 边缘拖宽、折叠显隐、宽度持久化都由 dock 负责；set_locked(true) 只锁重排，
//! 不锁拖宽——照片工具不需要把面板拖来拖去，但需要能拉宽左栏看长目录名。

use gpui_kit::component::dock::{
    BasePanel, DockArea, DockLayout, DockPlacement, DockSkin, Panel, PanelControl, PanelEvent,
    panel_handle,
};
use gpui_kit::component::v_flex;
use gpui_kit::{
    AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, Render,
    SharedString, Subscription, Window, div, prelude::*, px,
};

use crate::state::{AppState, ViewMode};
use crate::views::{
    render_adjustments_tab, render_batch_ops_tab, render_compare_view, render_file_tree_tab,
    render_filter_bar, render_info_tab, render_photo_grid, render_photo_preview, render_slideshow,
    render_stats_view,
};

/// Dock 面板种类：决定 panel_name（持久化标识）、标签名与内容。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DockPanelKind {
    FileTree,
    BatchOps,
    Info,
    Adjustments,
    Center,
}

impl DockPanelKind {
    /// 持久化标识：一旦发布不要改（dock 布局按名字找面板）。
    fn panel_name(self) -> &'static str {
        match self {
            Self::FileTree => "PhotoFileTree",
            Self::BatchOps => "PhotoBatchOps",
            Self::Info => "PhotoInfo",
            Self::Adjustments => "PhotoAdjustments",
            Self::Center => "PhotoCenter",
        }
    }

    /// 静态标签名；中央面板没有固定名字（跟当前视图走）。
    fn static_label(self) -> Option<&'static str> {
        match self {
            Self::FileTree => Some("文件树"),
            Self::BatchOps => Some("批量操作"),
            Self::Info => Some("信息"),
            Self::Adjustments => Some("调整"),
            Self::Center => None,
        }
    }
}

/// 视图模式 → 中文名（中央面板标题用）。
fn view_label(mode: ViewMode) -> &'static str {
    match mode {
        ViewMode::Grid => "网格",
        ViewMode::Preview => "预览",
        ViewMode::Compare => "对比",
        ViewMode::Slideshow => "幻灯片",
        ViewMode::Stats => "统计",
    }
}

/// 一个停靠面板：持有 AppState 实体，AppState 一 notify 就重绘。
///
/// 面板内容复用 views 里的纯渲染函数（render_*_tab / render_photo_grid …），
/// 通过 Entity::update 拿到 &mut Context<AppState>，因此原来的
/// cx.listener(...) 交互照常工作。
pub struct DockPanel {
    app: Entity<AppState>,
    kind: DockPanelKind,
    focus_handle: FocusHandle,
    /// 观察 AppState 的订阅；drop 即取消，必须持有。
    _app_subscription: Subscription,
}

impl DockPanel {
    /// 建面板实体；cx 是父级 AppState 的上下文（构造期间用）。
    pub fn new(
        app: Entity<AppState>,
        kind: DockPanelKind,
        cx: &mut Context<AppState>,
    ) -> Entity<Self> {
        cx.new(|cx| {
            let subscription = cx.observe(&app, |_, _, cx| cx.notify());
            Self {
                app,
                kind,
                focus_handle: cx.focus_handle(),
                _app_subscription: subscription,
            }
        })
    }

    fn render_body(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        match self.kind {
            DockPanelKind::FileTree => self.app.update(cx, |state, cx| {
                v_flex()
                    .w_full()
                    .p_3()
                    .child(render_file_tree_tab(state, cx))
                    .into_any_element()
            }),
            DockPanelKind::BatchOps => self.app.update(cx, |state, cx| {
                v_flex()
                    .w_full()
                    .p_3()
                    .child(render_batch_ops_tab(state, cx))
                    .into_any_element()
            }),
            DockPanelKind::Info => self.app.update(cx, |state, cx| {
                let meta = state.primary_selected_meta().cloned();
                v_flex()
                    .w_full()
                    .p_3()
                    .child(render_info_tab(state, meta.as_ref(), cx))
                    .into_any_element()
            }),
            DockPanelKind::Adjustments => self.app.update(cx, |state, cx| {
                let meta = state.primary_selected_meta().cloned();
                v_flex()
                    .w_full()
                    .p_3()
                    .child(render_adjustments_tab(state, meta.as_ref(), cx))
                    .into_any_element()
            }),
            DockPanelKind::Center => self.app.update(cx, |state, cx| {
                let filter_bar = (state.view_mode == ViewMode::Grid)
                    .then(|| render_filter_bar(state, window, cx).into_any_element());
                v_flex()
                    .size_full()
                    .overflow_hidden()
                    .children(filter_bar)
                    .child(match state.view_mode {
                        ViewMode::Grid => render_photo_grid(state, window, cx).into_any_element(),
                        ViewMode::Preview => {
                            render_photo_preview(state, window, cx).into_any_element()
                        }
                        ViewMode::Compare => {
                            render_compare_view(state, window, cx).into_any_element()
                        }
                        ViewMode::Slideshow => {
                            render_slideshow(state, window, cx).into_any_element()
                        }
                        ViewMode::Stats => render_stats_view(state, window, cx).into_any_element(),
                    })
                    .into_any_element()
            }),
        }
    }
}

impl EventEmitter<PanelEvent> for DockPanel {}

impl Focusable for DockPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for DockPanel {
    fn panel_name(&self) -> &'static str {
        self.kind.panel_name()
    }

    /// 不允许关闭：左右栏用显隐（Ctrl+[ / Ctrl+]）控制，关掉面板会把内容弄丢。
    fn closable(&self, _cx: &App) -> bool {
        false
    }

    /// 不允许整组缩放：照片工具没有"某个栏占满全屏"的用法。
    fn zoomable(&self, _cx: &App) -> bool {
        false
    }
}

impl Panel for DockPanel {
    fn tab_name(&self, _cx: &App) -> Option<SharedString> {
        self.kind.static_label().map(SharedString::from)
    }

    fn title(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let text = match self.kind.static_label() {
            Some(label) => label.to_string(),
            None => {
                let state = self.app.read(cx);
                format!(
                    "{} · {} 张",
                    view_label(state.view_mode),
                    state.display_order.len()
                )
            }
        };
        div().text_xs().child(text)
    }

    /// 不要缩放按钮 / 省略号菜单：标题栏右侧只留 dock 自带的折叠按钮。
    fn zoom_control(&self, _cx: &App) -> Option<PanelControl> {
        None
    }

    /// 内边距由各面板自己的 p_3() 负责，免得和 dock 的默认 padding 叠成两层。
    fn inner_padding(&self, _cx: &App) -> bool {
        false
    }
}

impl Render for DockPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_body(window, cx)
    }
}

/// 建整个工作区的 DockArea：中央主视图 + 左右停靠区。
///
/// 初始宽度取自配置（200–480），之后由用户拖边缘改，事件里回写配置。
pub fn create_dock(
    app: Entity<AppState>,
    left_width: f32,
    right_width: f32,
    window: &mut Window,
    cx: &mut Context<AppState>,
) -> Entity<DockArea> {
    let (area, _skin) = DockSkin::dock_area("photo-ui-dock", Some(1), window, cx);

    let file_tree = DockPanel::new(app.clone(), DockPanelKind::FileTree, cx);
    let batch_ops = DockPanel::new(app.clone(), DockPanelKind::BatchOps, cx);
    let info = DockPanel::new(app.clone(), DockPanelKind::Info, cx);
    let adjustments = DockPanel::new(app.clone(), DockPanelKind::Adjustments, cx);
    let center = DockPanel::new(app, DockPanelKind::Center, cx);

    let left = DockLayout::tabs()
        .panel_view(panel_handle(file_tree), cx)
        .panel_view(panel_handle(batch_ops), cx);
    let right = DockLayout::tabs()
        .panel_view(panel_handle(info), cx)
        .panel_view(panel_handle(adjustments), cx);
    let center_layout = DockLayout::tabs().panel_view(panel_handle(center), cx);

    area.update(cx, |area, cx| {
        area.set_center(center_layout, window, cx);
        area.set_dock(DockPlacement::Left, left, window, cx);
        area.set_dock(DockPlacement::Right, right, window, cx);
        area.set_dock_size(DockPlacement::Left, px(left_width), window, cx);
        area.set_dock_size(DockPlacement::Right, px(right_width), window, cx);
        // 锁重排：只保留拖宽与折叠，避免误拖把面板挪走
        area.set_locked(true, window, cx);
    });

    area
}
