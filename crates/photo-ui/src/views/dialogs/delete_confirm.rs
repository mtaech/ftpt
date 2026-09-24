//! 直接删除选中项的确认弹窗（Delete / Backspace，§9.10）。
//!
//! 与左栏「批量删除」同一套口径：先确认、再进回收站、执行后 Ctrl+Z 可恢复。
//! 2026-09-24 之前 Delete 键是**没有确认直接删**的，与批量删除的二次确认不一致（§13）。

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use gpui_kit::{Context, IntoElement, Window, div, prelude::*, px};

use crate::state::AppState;
use crate::state::engine_ops::{defer_entity_action, delete_selected_to_trash};

pub fn render_delete_confirm_dialog(
    state: &AppState,
    _window: &mut Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let count = state.mark_indices().len();

    div()
        .id("delete-modal-overlay")
        .occlude()
        .absolute()
        .inset_0()
        .bg(gpui_kit::rgba(0x00000088))
        .flex()
        .items_center()
        .justify_center()
        .child(
            v_flex()
                .w(px(440.))
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
                        .child(
                            Icon::new(IconName::TriangleAlert)
                                .size(px(24.))
                                .text_color(cx.theme().danger),
                        )
                        .child(
                            div()
                                .font_semibold()
                                .text_base()
                                .text_color(cx.theme().foreground)
                                .child("删除确认"),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!(
                            "即将把选中的 {count} 张照片（及其关联的 XMP/RAW 附属文件）移至系统回收站。\n可在执行后通过 Ctrl+Z 从回收站恢复。"
                        )),
                )
                .child(
                    h_flex()
                        .w_full()
                        .justify_end()
                        .gap_2()
                        .pt_2()
                        .child(
                            Button::new("cancel-delete")
                                .ghost()
                                .small()
                                .label("取消")
                                .on_click(cx.listener(|state, _, _, cx| {
                                    state.active_dialog = None;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("confirm-delete")
                                .danger()
                                .small()
                                .label("确认删除")
                                .on_click(cx.listener(|state, _, _, cx| {
                                    state.active_dialog = None;
                                    cx.notify();
                                    // delete_selected_to_trash 会同步 update AppState，必须等实体归还
                                    let entity = cx.entity().clone();
                                    defer_entity_action(cx, entity, |entity, cx| {
                                        delete_selected_to_trash(entity, cx);
                                    });
                                })),
                        ),
                ),
        )
}
