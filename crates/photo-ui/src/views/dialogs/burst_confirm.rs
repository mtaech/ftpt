//! 连拍选优确认弹窗（对应 §9.10 与 §4.5）。
//!
//! 规则说明：按 鸟眼锐度 -> 文件大小 -> 路径字典序 确定最优帧，
//! 将连拍组内其余非最优帧标记为 Reject（淘汰）。

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use gpui_kit::{Context, IntoElement, Window, div, prelude::*, px};
use photo_domain::Flag;

use crate::model::best_frame::non_best_paths;
use crate::state::AppState;
use crate::state::engine_ops::set_flag_for_paths;

pub fn render_burst_confirm_dialog(
    state: &AppState,
    _window: &mut Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let burst_count = state.burst_groups.len();

    div()
        .id("burst-modal-overlay")
        .occlude()
        .absolute()
        .inset_0()
        .bg(gpui_kit::rgba(0x00000088))
        .flex()
        .items_center()
        .justify_center()
        .child(
            v_flex()
                .w(px(448.))
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
                        .gap_3()
                        .child(Icon::new(IconName::Asterisk).size(px(24.)).text_color(cx.theme().primary))
                        .child(
                            div()
                                .font_semibold()
                                .text_base()
                                .text_color(cx.theme().foreground)
                                .child("连拍选优确认"),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!(
                            "当前筛选视图下共检测到 {} 个连拍组。选优算法将按：\n1. 鸟眼锐度得分（降序）\n2. 文件大小（降序）\n3. 路径字典序\n确定每组最优帧，并将组内其余非最优帧标记为「淘汰」(Reject)。",
                            burst_count
                        )),
                )
                .child(
                    h_flex()
                        .w_full()
                        .justify_end()
                        .gap_2()
                        .pt_2()
                        .child(
                            Button::new("cancel-burst")
                                .ghost()
                                .small()
                                .label("取消")
                                .on_click(cx.listener(|state, _, _, cx| {
                                    state.active_dialog = None;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("confirm-burst")
                                .small()
                                .danger()
                                .icon(IconName::Close)
                                .label("标记淘汰")
                                .on_click(cx.listener(|state, _, _, cx| {
                                    let display_metas: Vec<_> = state
                                        .display_order
                                        .iter()
                                        .filter_map(|&i| state.items.get(i).cloned())
                                        .collect();

                                    let mut group_items: std::collections::HashMap<String, Vec<_>> = std::collections::HashMap::new();
                                    for (&pos, entry) in &state.burst_groups {
                                        if let Some(meta) = display_metas.get(pos) {
                                            group_items.entry(entry.group_id.clone()).or_default().push(meta.clone());
                                        }
                                    }

                                    let mut paths_to_reject = Vec::new();
                                    for (_, members) in group_items {
                                        paths_to_reject.extend(non_best_paths(&members));
                                    }

                                    state.active_dialog = None;
                                    if !paths_to_reject.is_empty() {
                                        set_flag_for_paths(state, &paths_to_reject, Some(Flag::Reject));
                                    }
                                    cx.notify();
                                })),
                        ),
                ),
        )
}
