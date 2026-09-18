//! 导出对话框（对应 §9.10 与 §10.6）。
//!
//! 预设、目标目录、长边尺寸、质量滑杆与命名模板。

use gpui_kit::component::{
    ActiveTheme as _, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use gpui_kit::{Context, IntoElement, Window, div, prelude::*, px};

use crate::state::AppState;

pub fn render_export_dialog(
    state: &AppState,
    _window: &mut Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let selected_count = if state.selected_indices.is_empty() {
        state.display_order.len()
    } else {
        state.selected_indices.len()
    };

    div()
        .id("export-modal-overlay")
        .occlude()
        .absolute()
        .inset_0()
        .bg(gpui_kit::rgba(0x00000088))
        .flex()
        .items_center()
        .justify_center()
        .child(
            v_flex()
                .w(px(512.))
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
                                .child(format!("照片导出 (共 {} 张)", selected_count)),
                        )
                        .child(
                            Button::new("close-export")
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
                        .p_6()
                        .gap_3()
                        .child(
                            v_flex()
                                .p_3()
                                .rounded(px(12.))
                                .border_1()
                                .border_color(cx.theme().border.opacity(0.6))
                                .bg(cx.theme().popover)
                                .shadow(crate::theme::panel_card_shadow(cx))
                                .gap_1()
                                .child(
                                    div()
                                        .font_medium()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("长边最大像素"),
                                )
                                .child(
                                    div()
                                        .p_2()
                                        .rounded(cx.theme().radius)
                                        .bg(cx.theme().background)
                                        .border_1()
                                        .border_color(cx.theme().border.opacity(0.6))
                                        .text_xs()
                                        .text_color(cx.theme().foreground)
                                        .child("2560 px (高清网图/社交分享)"),
                                )
                        )
                        .child(
                            v_flex()
                                .p_3()
                                .rounded(px(12.))
                                .border_1()
                                .border_color(cx.theme().border.opacity(0.6))
                                .bg(cx.theme().popover)
                                .shadow(crate::theme::panel_card_shadow(cx))
                                .gap_1()
                                .child(
                                    div()
                                        .font_medium()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("JPEG 质量"),
                                )
                                .child(
                                    div()
                                        .p_2()
                                        .rounded(cx.theme().radius)
                                        .bg(cx.theme().background)
                                        .border_1()
                                        .border_color(cx.theme().border.opacity(0.6))
                                        .text_xs()
                                        .text_color(cx.theme().foreground)
                                        .child("90%"),
                                )
                        )
                        .child(
                            v_flex()
                                .p_3()
                                .rounded(px(12.))
                                .border_1()
                                .border_color(cx.theme().border.opacity(0.6))
                                .bg(cx.theme().popover)
                                .shadow(crate::theme::panel_card_shadow(cx))
                                .gap_1()
                                .child(
                                    div()
                                        .font_medium()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("重命名模板"),
                                )
                                .child(
                                    div()
                                        .p_2()
                                        .rounded(cx.theme().radius)
                                        .bg(cx.theme().background)
                                        .border_1()
                                        .border_color(cx.theme().border.opacity(0.6))
                                        .text_xs()
                                        .text_color(cx.theme().foreground)
                                        .child("{name}_{species}_{seq}"),
                                )
                        ),
                )
                .child(
                    h_flex()
                        .w_full()
                        .h(px(48.))
                        .items_center()
                        .justify_end()
                        .px_4()
                        .border_t_1()
                        .border_color(cx.theme().border)
                        .gap_2()
                        .child(
                            Button::new("cancel-export-btn")
                                .ghost()
                                .small()
                                .label("取消")
                                .on_click(cx.listener(|state, _, _, cx| {
                                    state.active_dialog = None;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("run-export-btn")
                                .small()
                                .secondary()
                                .icon(IconName::ExternalLink)
                                .label("开始导出")
                                .on_click(cx.listener(|state, _, _, cx| {
                                    state.set_status_message("已开始导出照片");
                                    state.active_dialog = None;
                                    cx.notify();
                                })),
                        ),
                ),
        )
}
