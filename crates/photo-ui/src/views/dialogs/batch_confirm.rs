//! 批量操作二次确认弹窗（对应 §8.4、§9.10 与 §10.4）。
//!
//! 筛选驱动：作用于当前可见顺序（display_order）；无筛选时按钮在左栏被锁住，
//! 只有用户「显式确认」后才解锁（§13 安全逃生门，见 left_panel）。
//! 确认后走 engine_ops::start_batch_op：后台执行 + 逐文件撤销记录 + 重扫。

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use gpui_kit::{Context, IntoElement, Window, div, prelude::*, px};
use photo_domain::BatchOpType;

use crate::state::AppState;
use crate::state::engine_ops::{defer_entity_action, start_batch_op};

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

    // 目标目录文本：确认框里显示的必须是真正要写进去的目录（选目录在左栏点按钮时完成）
    let dest_label = state
        .batch_target_dir
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "（未选择目标目录）".to_string());
    let dest_ready = !op_type.needs_target_dir() || state.batch_target_dir.is_some();

    let desc = match op_type {
        BatchOpType::Delete => format!(
            "即将把当前筛选出的 {} 张照片（及其关联的 XMP/RAW 附属文件）移至系统回收站。\n可在执行后通过 Ctrl+Z 从回收站恢复。",
            affected_count
        ),
        BatchOpType::Move => format!(
            "即将把当前筛选出的 {} 张照片移动至：\n{}\n可在执行后通过 Ctrl+Z 撤销。",
            affected_count, dest_label
        ),
        BatchOpType::Copy => format!(
            "即将把当前筛选出的 {} 张照片复制至：\n{}",
            affected_count, dest_label
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
                .when(!dest_ready, |this| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().danger)
                            .child("没有目标目录，无法执行；请重新点击左栏按钮并选择一个目录。"),
                    )
                })
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
                                    state.batch_target_dir = None;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("confirm-batch-op")
                                .small()
                                .disabled(!dest_ready)
                                .when(op_type == BatchOpType::Delete, |btn| btn.danger())
                                .label(if op_type == BatchOpType::Delete {
                                    "确认删除"
                                } else {
                                    "确认执行"
                                })
                                .on_click(cx.listener(move |state, _, _window, cx| {
                                    if !dest_ready {
                                        return;
                                    }
                                    let target = state.batch_target_dir.clone();
                                    state.active_dialog = None;
                                    state.batch_target_dir = None;
                                    cx.notify();

                                    // start_batch_op 会同步 update AppState，必须等实体归还
                                    let entity = cx.entity().clone();
                                    defer_entity_action(cx, entity, move |entity, cx| {
                                        start_batch_op(entity, op_type, target, cx);
                                    });
                                })),
                        ),
                ),
        )
}
