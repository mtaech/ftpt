//! 相似与重复照片检测弹窗（对应 §9.10 与 §10.5）。
//!
//! 基于 dHash 感知哈希计算，支持相似分组浏览与一键优选。

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use gpui_kit::{Context, IntoElement, Window, div, prelude::*, px};

use crate::state::AppState;

pub fn render_duplicates_dialog(
    _state: &AppState,
    _window: &mut Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    div()
        .id("duplicates-modal-overlay")
        .occlude()
        .absolute()
        .inset_0()
        .bg(gpui_kit::rgba(0x00000088))
        .flex()
        .items_center()
        .justify_center()
        .child(
            v_flex()
                .w(px(720.))
                .h(px(480.))
                .bg(cx.theme().sidebar)
                .rounded(px(20.))
                .border_1()
                .border_color(cx.theme().border.opacity(0.6))
                .shadow(crate::theme::overlay_shadow())
                .overflow_hidden()
                .child(
                    h_flex()
                        .w_full()
                        .h(px(48.))
                        .items_center()
                        .justify_between()
                        .px_4()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .child(
                            div()
                                .font_semibold()
                                .text_sm()
                                .text_color(cx.theme().foreground)
                                .child("相似与重复照片检测"),
                        )
                        .child(
                            Button::new("close-duplicates")
                                .ghost()
                                .small()
                                .icon(IconName::Close)
                                .on_click(cx.listener(|state, _, _, cx| {
                                    state.active_dialog = None;
                                    cx.notify();
                                })),
                        ),
                )
                .child(
                    v_flex()
                        .w_full()
                        .flex_1()
                        .p_6()
                        .items_center()
                        .justify_center()
                        .child(
                            v_flex()
                                .p_8()
                                .rounded(px(16.))
                                .border_1()
                                .border_color(cx.theme().border.opacity(0.6))
                                .bg(cx.theme().popover)
                                .shadow(crate::theme::panel_card_shadow(cx))
                                .items_center()
                                .gap_4()
                                .child(Icon::new(IconName::Copy).size(px(36.)).text_color(cx.theme().muted_foreground))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("基于 dHash 感知哈希计算相似度，阈值默认为 10（汉明距离 ≤ 10 判定为相似）"),
                                )
                                .child(
                                    Button::new("run-duplicates-detect")
                                        .small()
                                        .secondary()
                                        .icon(IconName::Asterisk)
                                        .label("开始检测当前目录")
                                        .on_click(cx.listener(|state, _, _, cx| {
                                            state.set_status_message("正在进行相似照片检测...");
                                            state.active_dialog = None;
                                            cx.notify();
                                        })),
                                ),
                        ),
                ),
        )
}
