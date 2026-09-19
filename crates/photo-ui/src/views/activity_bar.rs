use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    v_flex,
};
use gpui_kit::{Context, IntoElement, Window, div, prelude::*, px};

use crate::actions::{
    OpenDirectory, ToggleLeftPanel, ToggleRightPanel, ToggleThemeMode, ToggleView,
};
use crate::state::{AppState, ViewMode};

pub fn render_left_activity_bar(
    state: &AppState,
    _window: &mut Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let is_preview = state.view_mode == ViewMode::Preview;

    v_flex()
        .w(px(50.))
        .h_full()
        .items_center()
        .py_3()
        .gap_2()
        .bg(cx.theme().sidebar)
        .border_r_1()
        .border_color(cx.theme().border.opacity(0.5))
        .child(render_rail_item(
            "rail-toggle-left",
            IconName::PanelLeft,
            "切换左侧栏 (Ctrl+[)",
            false,
            false,
            |_, window, cx| {
                window.dispatch_action(Box::new(ToggleLeftPanel), cx);
            },
            cx,
        ))
        .child(render_rail_item(
            "rail-open-dir",
            IconName::FolderOpen,
            "打开照片目录 (Ctrl+O)",
            false,
            false,
            |_, window, cx| {
                window.dispatch_action(Box::new(OpenDirectory), cx);
            },
            cx,
        ))
        .child(render_rail_item(
            "rail-toggle-preview",
            if is_preview {
                IconName::LayoutDashboard
            } else {
                // 🖼（Lucide image）：只有图标时 frame（#）跟"网格"分不清，换成语义直白的图片图标
                IconName::Image
            },
            if is_preview {
                "返回网格 (G / Esc)"
            } else {
                "单图预览 (G)"
            },
            is_preview,
            false,
            |_, window, cx| {
                window.dispatch_action(Box::new(ToggleView), cx);
            },
            cx,
        ))
        .child(div().flex_1())
        .child(render_rail_item(
            "rail-toggle-theme",
            if cx.theme().mode.is_dark() {
                IconName::Sun
            } else {
                IconName::Moon
            },
            if cx.theme().mode.is_dark() {
                "切换至浅色模式"
            } else {
                "切换至深色模式"
            },
            false,
            false,
            |_, window, cx| {
                window.dispatch_action(Box::new(ToggleThemeMode), cx);
            },
            cx,
        ))
}

pub fn render_right_activity_bar(
    _state: &AppState,
    _window: &mut Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    v_flex()
        .w(px(50.))
        .h_full()
        .items_center()
        .py_3()
        .gap_2()
        .bg(cx.theme().sidebar)
        .border_l_1()
        .border_color(cx.theme().border.opacity(0.5))
        .child(render_rail_item(
            "rail-toggle-right",
            IconName::PanelRight,
            "切换右侧信息栏 (Ctrl+])",
            false,
            false,
            |_, window, cx| {
                window.dispatch_action(Box::new(ToggleRightPanel), cx);
            },
            cx,
        ))
}

fn render_rail_item(
    id: &'static str,
    icon: IconName,
    tooltip: &'static str,
    is_active: bool,
    disabled: bool,
    on_click: impl Fn(&gpui_kit::ClickEvent, &mut Window, &mut gpui_kit::App) + 'static,
    cx: &Context<AppState>,
) -> impl IntoElement {
    let theme = cx.theme();
    div()
        .w(px(40.))
        .h(px(32.))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .when(is_active, |this| this.bg(theme.selection))
        .child(
            Button::new(id)
                .ghost()
                .small()
                .icon(icon)
                .selected(is_active)
                .disabled(disabled)
                .tooltip(tooltip)
                .on_click(on_click),
        )
}
