//! 批量操作二次确认弹窗（对应 §8.4、§9.10 与 §10.4）。
//!
//! 筛选驱动：仅对当前筛选结果生效。
//! 显示操作类型、影响照片数、确认与取消。

use std::path::PathBuf;

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use gpui_kit::{Context, IntoElement, Window, div, prelude::*, px};
use photo_domain::BatchOpType;

use crate::state::AppState;
use crate::state::engine_ops::{defer_entity_action, delete_paths};

pub fn render_batch_confirm_dialog(
    state: &AppState,
    op_type: BatchOpType,
    _window: &mut Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let affected_count = state.display_order.len();

    let title = match op_type {
        BatchOpType::Delete => "批量删除确认",
        BatchOpType::Move => "批量移动确认",
        BatchOpType::Copy => "批量复制确认",
    };

    let desc = match op_type {
        BatchOpType::Delete => format!(
            "即将把当前筛选出的 {} 张照片（及其关联的 XMP/RAW 附属文件）移至系统回收站。\n此操作不可撤销，请确认。",
            affected_count
        ),
        BatchOpType::Move => format!(
            "即将把当前筛选出的 {} 张照片移动至目标目录。\n可在执行后通过 Ctrl+Z 撤销。",
            affected_count
        ),
        BatchOpType::Copy => format!(
            "即将把当前筛选出的 {} 张照片复制至目标目录。",
            affected_count
        ),
    };

    div()
        .id("batch-modal-overlay")
        .occlude()
        .absolute()
        .inset_0()
        .bg(gpui_kit::rgba(0x00000088))
        .flex()
        .items_center()
        .justify_center()
        .child(
            v_flex()
                .w(px(460.))
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
                        .child(Icon::new(IconName::TriangleAlert).size(px(24.)).text_color(
                            if op_type == BatchOpType::Delete {
                                cx.theme().danger
                            } else {
                                cx.theme().warning
                            },
                        ))
                        .child(
                            div()
                                .font_semibold()
                                .text_base()
                                .text_color(cx.theme().foreground)
                                .child(title),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(desc),
                )
                .child(
                    h_flex()
                        .w_full()
                        .justify_end()
                        .gap_2()
                        .pt_2()
                        .child(
                            Button::new("cancel-batch-op")
                                .ghost()
                                .small()
                                .label("取消")
                                .on_click(cx.listener(|state, _, _, cx| {
                                    state.active_dialog = None;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("confirm-batch-op")
                                .small()
                                .when(op_type == BatchOpType::Delete, |btn| btn.danger())
                                .label(if op_type == BatchOpType::Delete {
                                    "确认删除"
                                } else {
                                    "确认执行"
                                })
                                .on_click(cx.listener(move |state, _, _window, cx| {
                                    let paths_to_op: Vec<PathBuf> = state
                                        .display_order
                                        .iter()
                                        .filter_map(|&i| {
                                            state
                                                .items
                                                .get(i)
                                                .map(|m| PathBuf::from(&m.primary_path))
                                        })
                                        .collect();
                                    state.active_dialog = None;
                                    cx.notify();

                                    if op_type == BatchOpType::Delete && !paths_to_op.is_empty() {
                                        // delete_paths 会同步 read/update AppState，必须等实体归还
                                        let entity = cx.entity().clone();
                                        defer_entity_action(cx, entity, move |entity, cx| {
                                            delete_paths(entity, paths_to_op, cx);
                                        });
                                    }
                                })),
                        ),
                ),
        )
}
