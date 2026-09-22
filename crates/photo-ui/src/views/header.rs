//! 顶栏组件（对应 §9.1）。
//!
//! 44px 高：
//! - 左：当前目录名 + 项数
//! - 中：网格 / 预览 / 幻灯片 / 统计 签名下划线 Tab
//! - 右：扫描/识别任务进度 + 快捷动作

use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    tag::Tag,
};
use gpui_kit::{ClickEvent, Context, IntoElement, Window, div, prelude::*, px};

use crate::actions::{OpenSettings, RecognizeAllUnrecognized, Rescan, Stats};
use crate::state::{AppState, ViewMode};

pub fn render_header(
    state: &AppState,
    _window: &mut Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let dir_name = state
        .current_dir
        .as_ref()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "未打开目录".to_string());

    let total_count = state.items.len();
    let filtered_count = state.display_order.len();

    // 「全部识别」按钮：作用口径与 Ctrl+B（RecognizeAllUnrecognized）完全一致——
    // 只覆盖当前目录里**从未识别**（recognition_status == None）的照片，不碰筛选、
    // 也不重跑已有结果（要连已识别的一起重跑是 Ctrl+Shift+B）。
    let unrecognized_count = state
        .items
        .iter()
        .filter(|m| m.recognition_status.is_none())
        .count();
    // 按钮直接 dispatch 既有 action，不复制识别逻辑
    let (recognize_label, recognize_disabled, recognize_tip) = if state.is_recognizing {
        ("识别中…".to_string(), true, "批量识别进行中（状态栏可取消）")
    } else if unrecognized_count > 0 {
        (
            format!("全部识别 {unrecognized_count}"),
            false,
            "识别当前目录全部未识别照片（Ctrl+B）；连已识别的一起重跑用 Ctrl+Shift+B",
        )
    } else if state.items.is_empty() {
        ("全部识别".to_string(), true, "当前目录还没有照片")
    } else {
        (
            "全部已识别".to_string(),
            true,
            "没有未识别的照片；要全部重跑用 Ctrl+Shift+B",
        )
    };

    h_flex()
        .w_full()
        .h(px(46.))
        .items_center()
        .justify_between()
        .px_4()
        .bg(cx.theme().sidebar)
        .border_b_1()
        .border_color(cx.theme().border.opacity(0.6))
        // ── 左侧：目录与项数胶囊 ──
        .child(
            h_flex()
                .items_center()
                .gap_2()
                .flex_1()
                .child(
                    div()
                        .w(px(28.))
                        .h(px(28.))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(cx.theme().selection)
                        .child(
                            Icon::new(IconName::Folder)
                                .size(px(14.))
                                .text_color(cx.theme().primary),
                        ),
                )
                .child(
                    div()
                        .font_semibold()
                        .text_sm()
                        .text_color(cx.theme().foreground)
                        .child(dir_name),
                )
                .child(
                    Tag::secondary()
                        .small()
                        .rounded_full()
                        .child(if filtered_count < total_count {
                            format!("{filtered_count} / {total_count} 项")
                        } else {
                            format!("{total_count} 项")
                        }),
                ),
        )
        // ── 中间：Material You 分段胶囊选择器 (Segmented Pill Buttons) ──
        .child(
            h_flex()
                .items_center()
                .p(px(2.))
                .gap(px(2.))
                .rounded_full()
                .bg(cx.theme().muted.opacity(0.6))
                .border_1()
                .border_color(cx.theme().border.opacity(0.5))
                .child(render_view_mode_pill(
                    "pill-view-grid",
                    "网格",
                    IconName::LayoutDashboard,
                    state.view_mode == ViewMode::Grid,
                    cx.listener(|state, _, _, cx| {
                        state.view_mode = ViewMode::Grid;
                        cx.notify();
                    }),
                    cx,
                ))
                .child(render_view_mode_pill(
                    "pill-view-preview",
                    "单张",
                    IconName::Image,
                    state.view_mode == ViewMode::Preview,
                    cx.listener(|state, _, _, cx| {
                        if state.primary_selected_meta().is_some() {
                            state.view_mode = ViewMode::Preview;
                            state.preview_zoom = 1.0;
                            state.preview_pan = (0.0, 0.0);
                            state.preview_drag_start = None;
                            cx.notify();
                        }
                    }),
                    cx,
                ))
                .child(render_view_mode_pill(
                    "pill-view-stats",
                    "统计",
                    IconName::Asterisk,
                    state.view_mode == ViewMode::Stats,
                    |_, window, cx| {
                        window.dispatch_action(Box::new(Stats), cx);
                    },
                    cx,
                )),
        )
        // ── 右侧：状态、刷新、设置 ──
        .child(
            h_flex()
                .items_center()
                .gap_1p5()
                .flex_1()
                .justify_end()
                .when(state.is_scanning, |this| {
                    this.child(
                        Tag::secondary()
                            .small()
                            .rounded_full()
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_1p5()
                                    .child(Icon::new(IconName::RotateCw).size(px(12.)))
                                    .child(
                                        state
                                            .scan_stage
                                            .clone()
                                            .unwrap_or_else(|| "扫描中...".to_string()),
                                    ),
                            ),
                    )
                })
                // ── 导出（Ctrl+E 同效）：当前筛选结果 / 已选照片 ──
                .child(
                    Button::new("header-export")
                        .ghost()
                        .small()
                        .icon(IconName::ExternalLink)
                        .disabled(state.items.is_empty() || state.is_exporting)
                        .tooltip("导出当前筛选结果（Ctrl+E）")
                        .on_click(cx.listener(|state, _, window, cx| {
                            state.open_export_dialog(window, cx);
                        })),
                )
                // ── 全部识别（Ctrl+B 同效）：常驻可见，不用先选中照片 ──
                .child(
                    Button::new("header-recognize-all")
                        .secondary()
                        .small()
                        .icon(IconName::Sparkles)
                        .disabled(recognize_disabled)
                        .tooltip(recognize_tip)
                        .label(recognize_label)
                        .on_click(|_, window, cx| {
                            window.dispatch_action(Box::new(RecognizeAllUnrecognized), cx);
                        }),
                )
                .child(
                    Button::new("header-rescan")
                        .ghost()
                        .small()
                        .icon(IconName::RotateCw)
                        .disabled(state.is_scanning || state.current_dir.is_none())
                        .tooltip("刷新当前目录 (F5)")
                        .on_click(|_, window, cx| {
                            window.dispatch_action(Box::new(Rescan), cx);
                        }),
                )
                .child(
                    Button::new("header-settings")
                        .ghost()
                        .small()
                        .icon(IconName::Settings)
                        .tooltip("设置 (Ctrl+,)")
                        .on_click(|_, window, cx| {
                            window.dispatch_action(Box::new(OpenSettings), cx);
                        }),
                ),
        )
}

fn render_view_mode_pill(
    id: &'static str,
    label: &'static str,
    icon: IconName,
    is_active: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui_kit::App) + 'static,
    cx: &Context<AppState>,
) -> impl IntoElement {
    let theme = cx.theme();
    h_flex()
        .id(id)
        .items_center()
        .gap_1p5()
        .px_3()
        .py_1()
        .rounded_full()
        .cursor_pointer()
        .text_xs()
        .when(is_active, |this| {
            this.bg(theme.primary)
                .text_color(theme.primary_foreground)
                .font_medium()
                .shadow(crate::theme::panel_card_shadow(cx))
        })
        .when(!is_active, |this| {
            this.text_color(theme.muted_foreground)
                .hover(move |s| s.bg(theme.accent).text_color(theme.foreground))
        })
        .child(Icon::new(icon).size(px(13.)))
        .child(label)
        .on_click(on_click)
}
