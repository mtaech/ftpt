//! 鸟种纠错弹窗（对应 §9.10 与 §4.4）。
//!
//! 手动重命名或修正识别结果，记录到修正日志并写入 XMP。

use gpui_kit::component::{
    ActiveTheme as _, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use gpui_kit::{Context, IntoElement, Window, div, prelude::*, px};
use photo_domain::RecognitionStatus;

use crate::state::AppState;

pub fn render_correct_dialog(
    state: &AppState,
    item_idx: usize,
    _window: &mut Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let cur_name = state
        .items
        .get(item_idx)
        .and_then(|m| m.bird_name.clone())
        .unwrap_or_else(|| "未识别".to_string());

    div()
        .id("correct-modal-overlay")
        .occlude()
        .absolute()
        .inset_0()
        .bg(gpui_kit::rgba(0x00000088))
        .flex()
        .items_center()
        .justify_center()
        .child(
            v_flex()
                .w(px(416.))
                .bg(cx.theme().sidebar)
                .rounded(px(20.))
                .border_1()
                .border_color(cx.theme().border.opacity(0.6))
                .shadow(crate::theme::overlay_shadow())
                .p_6()
                .gap_4()
                .child(
                    h_flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .font_semibold()
                                .text_sm()
                                .text_color(cx.theme().foreground)
                                .child("人工纠错鸟种"),
                        )
                        .child(
                            Button::new("close-correct")
                                .ghost()
                                .small()
                                .icon(IconName::Close)
                                .tooltip("关闭 (Esc)")
                                .on_click(cx.listener(|state, _, _, cx| {
                                    state.active_dialog = None;
                                    cx.notify();
                                })),
                        ),
                )
                .child(
                    v_flex()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child("当前识别为"),
                        )
                        .child(
                            div()
                                .p_2()
                                .rounded(px(10.))
                                .bg(cx.theme().background)
                                .font_medium()
                                .text_sm()
                                .text_color(cx.theme().foreground)
                                .child(cur_name),
                        ),
                )
                .child(
                    v_flex()
                        .gap_2()
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child("常用修正鸟种候选"),
                        )
                        .child(
                            h_flex()
                                .flex_wrap()
                                .gap_1()
                                .child(render_candidate_chip("白鹭", item_idx, cx))
                                .child(render_candidate_chip("大白鹭", item_idx, cx))
                                .child(render_candidate_chip("苍鹭", item_idx, cx))
                                .child(render_candidate_chip("池鹭", item_idx, cx))
                                .child(render_candidate_chip("夜鹭", item_idx, cx))
                                .child(render_candidate_chip("普通翠鸟", item_idx, cx))
                                .child(render_candidate_chip("白头鹎", item_idx, cx))
                                .child(render_candidate_chip("喜鹊", item_idx, cx))
                                .child(render_candidate_chip("麻雀", item_idx, cx)),
                        ),
                ),
        )
}

fn render_candidate_chip(
    name: &'static str,
    item_idx: usize,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    Button::new(format!("chip-{name}"))
        .xsmall()
        .ghost()
        .label(name)
        .on_click(cx.listener(move |state, _, _, cx| {
            if let Some(m) = state.items.get_mut(item_idx) {
                m.bird_name = Some(name.to_string());
                m.recognition_status = Some(RecognitionStatus::Confirmed);
                m.bird_confidence = Some(1.0);
            }
            state.recompute_pipeline();
            state.active_dialog = None;
            state.set_status_message(format!("已将该照片鸟种更正为「{name}」"));
            cx.notify();
        }))
}
