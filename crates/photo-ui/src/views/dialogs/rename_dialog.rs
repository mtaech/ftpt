//! 批量重命名弹窗（§9.10）：命名模板 + 实时预览 + 执行。
//!
//! 作用域 = 当前可见顺序（display_order，与导出/批量操作同一口径）；执行后
//! 逐文件撤销记录进 op_journal，Ctrl+Z 可改回原名。

use std::path::Path;

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::Input,
    v_flex,
};
use gpui_kit::{Context, IntoElement, Window, div, prelude::*, px};
use photo_engine::template::{NameTemplateContext, render_name_template};

use crate::state::AppState;
use crate::state::engine_ops::{defer_entity_action, start_batch_rename};

pub fn render_rename_dialog(
    state: &AppState,
    _window: &mut Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let count = state.display_order.len();
    let template = state
        .rename_template_input
        .as_ref()
        .map(|input| input.read(cx).value().to_string())
        .unwrap_or_default();

    // 实时预览第一张：与引擎同一套模板渲染（photo_engine::template），不另写一份口径
    let preview = state
        .display_order
        .first()
        .and_then(|&i| state.items.get(i))
        .map(|meta| {
            let ctx = NameTemplateContext {
                name: meta.base_name.clone(),
                species: meta.taxon_name.clone(),
                date: meta.date_taken.clone(),
                camera: meta.camera_model.clone(),
                seq: 1,
            };
            let base = render_name_template(&template, &ctx);
            let ext = Path::new(&meta.primary_path)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_string();
            if ext.is_empty() {
                base
            } else {
                format!("{base}.{ext}")
            }
        })
        .unwrap_or_default();

    div()
        .id("rename-modal-overlay")
        .occlude()
        .absolute()
        .inset_0()
        .bg(gpui_kit::rgba(0x00000088))
        .flex()
        .items_center()
        .justify_center()
        .child(
            v_flex()
                .w(px(520.))
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
                            Icon::new(IconName::FileText)
                                .size(px(22.))
                                .text_color(cx.theme().primary),
                        )
                        .child(
                            div()
                                .font_semibold()
                                .text_base()
                                .text_color(cx.theme().foreground)
                                .child("批量重命名"),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!(
                            "将对当前可见顺序的 {count} 张照片改名（含同名的 XMP/RAW 附属文件）。占位符：{{name}} 原名 / {{species}} 物种 / {{date}} 拍摄日期 / {{seq}} 序号 / {{camera}} 相机。模板留空或渲染结果与原文件名相同则跳过该文件。"
                        )),
                )
                .when_some(state.rename_template_input.clone(), |this, input| {
                    this.child(Input::new(&input).small().w_full())
                })
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("预览（第 1 张）：{preview}")),
                )
                .child(
                    h_flex()
                        .w_full()
                        .justify_end()
                        .gap_2()
                        .pt_2()
                        .child(
                            Button::new("cancel-rename")
                                .ghost()
                                .small()
                                .label("取消")
                                .on_click(cx.listener(|state, _, _, cx| {
                                    state.active_dialog = None;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("confirm-rename")
                                .small()
                                .disabled(template.trim().is_empty() || count == 0)
                                .label("开始重命名")
                                .on_click(cx.listener(|state, _, _, cx| {
                                    let template = state
                                        .rename_template_input
                                        .as_ref()
                                        .map(|input| input.read(cx).value().to_string())
                                        .unwrap_or_default();
                                    state.active_dialog = None;
                                    cx.notify();
                                    let entity = cx.entity().clone();
                                    defer_entity_action(cx, entity, move |entity, cx| {
                                        start_batch_rename(entity, template, cx);
                                    });
                                })),
                        ),
                ),
        )
}
