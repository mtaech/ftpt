//! 状态栏组件（对应 §9.8）。
//!
//! 24px 高，三段式：
//! - 左：目录全路径（截断，tooltip）
//! - 中：N 项 · M 已选（已选 > 0 用 pick 绿）
//! - 右：状态区（扫描中 > 识别中 > 瞬态提示 > 就绪）

use std::sync::atomic::Ordering;
use std::time::Duration;

use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
};
use gpui_kit::{Context, IntoElement, Window, div, prelude::*, px};

use crate::state::AppState;

pub fn render_status_bar(
    state: &AppState,
    _window: &mut Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let dir_path = state
        .current_dir
        .as_ref()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| "未打开目录".to_string());

    let total_count = state.items.len();
    let selected_count = state.selected_indices.len();

    StatusBar::new()
        .left(
            h_flex()
                .items_center()
                .gap_1p5()
                .child(
                    Icon::new(IconName::Folder)
                        .size(px(12.))
                        .text_color(cx.theme().muted_foreground),
                )
                .child(
                    div()
                        .truncate()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(dir_path),
                ),
        )
        .right(render_status_right(state, cx))
        .child(
            h_flex()
                .items_center()
                .gap_1p5()
                .text_xs()
                .child(
                    div()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("{total_count} 项")),
                )
                .when(selected_count > 0, |this| {
                    this.child(div().text_color(cx.theme().muted_foreground).child("·"))
                        .child(
                            div()
                                .px_2()
                                .py_0p5()
                                .rounded_full()
                                .bg(cx.theme().selection)
                                .font_medium()
                                .text_color(cx.theme().primary)
                                .child(format!("{selected_count} 已选")),
                        )
                }),
        )
}

fn render_status_right(state: &AppState, cx: &mut Context<AppState>) -> impl IntoElement {
    // 优先级 1: 扫描中
    if state.is_scanning {
        let stage = state.scan_stage.as_deref().unwrap_or("扫描中");
        return h_flex()
            .items_center()
            .gap_1()
            .text_color(cx.theme().primary)
            .child(Icon::new(IconName::RotateCw).size(px(12.)))
            .child(if state.scan_total > 0 {
                format!("{stage}: {}/{}", state.scan_done, state.scan_total)
            } else {
                stage.to_string()
            })
            .into_any_element();
    }

    // 优先级 2: 识别中
    if state.is_recognizing {
        let cancel = state.recognize_cancel.clone();
        return h_flex()
            .items_center()
            .gap_1()
            .text_color(cx.theme().warning)
            .child(Icon::new(IconName::Asterisk).size(px(12.)))
            .child(format!(
                "识别中: {}/{} {}",
                state.recognize_done, state.recognize_total, state.recognize_current
            ))
            .child(
                Button::new("cancel-recognize")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Close)
                    .on_click(move |_, _window, _cx| {
                        cancel.store(true, Ordering::Relaxed);
                    }),
            )
            .into_any_element();
    }

    // 优先级 3: 缩略图后台生成中（不阻塞交互，只提示进度）
    if state.thumb_total > 0 {
        return h_flex()
            .items_center()
            .gap_1()
            .text_color(cx.theme().muted_foreground)
            .child(Icon::new(IconName::Frame).size(px(12.)))
            .child(format!("缩略图 {}/{}", state.thumb_done, state.thumb_total))
            .into_any_element();
    }

    // 优先级 4: 瞬态提示（4s 自动消失）
    if let Some((msg, created_at)) = &state.status_message {
        if created_at.elapsed() < Duration::from_secs(4) {
            return h_flex()
                .items_center()
                .gap_1()
                .text_color(cx.theme().foreground)
                .child(
                    Icon::new(IconName::Info)
                        .size(px(12.))
                        .text_color(cx.theme().primary),
                )
                .child(msg.clone())
                .into_any_element();
        }
    }

    // 默认: 就绪
    h_flex()
        .items_center()
        .gap_1()
        .text_color(cx.theme().muted_foreground)
        .child("就绪")
        .into_any_element()
}
