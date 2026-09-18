//! 对比视图组件（对应 §3.3、§4.6、§5.3 与 §5.6）。
//!
//! - 2–4 张图片并排或 2x2 网格并排显示
//! - 聚焦格（focused slot）高亮并接收 1–5 评分
//! - ← / → 切换聚焦格
//! - Esc / G 返回来源视图

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    rating::Rating as ComponentRating,
    tag::Tag,
    v_flex,
};
use gpui_kit::{Context, IntoElement, Window, div, img, prelude::*, px};
use photo_domain::Rating;

use crate::actions::Escape;
use crate::image::THUMB_SIZE_GRID;
use crate::state::AppState;

pub fn render_compare_view(
    state: &AppState,
    _window: &mut Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement + use<> {
    let slots = &state.compare_indices;
    let total_slots = slots.len();

    if total_slots < 2 {
        return div()
            .w_full()
            .h_full()
            .bg(cx.theme().background)
            .flex()
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
                    .gap_3()
                    .child(
                        Icon::new(IconName::LayoutDashboard)
                            .size(px(36.))
                            .text_color(cx.theme().muted_foreground),
                    )
                    .child(
                        div()
                            .text_sm()
                            .font_medium()
                            .text_color(cx.theme().foreground)
                            .child("请至少选择 2 张照片进行对比"),
                    ),
            )
            .into_any_element();
    }

    let focused_slot = state.compare_focused_slot.min(total_slots - 1);

    v_flex()
        .w_full()
        .h_full()
        .bg(cx.theme().background)
        .p_3()
        .gap_3()
        // ── 顶部操作栏 ──
        .child(
            h_flex()
                .w_full()
                .items_center()
                .justify_between()
                .px_2()
                .child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("对比模式")
                        .child("·")
                        .child(
                            Tag::secondary()
                                .small()
                                .child(format!("{total_slots} 张照片")),
                        )
                        .child("·")
                        .child(format!("当前聚焦: 第 {} 格", focused_slot + 1)),
                )
                .child(
                    Button::new("btn-exit-compare")
                        .ghost()
                        .small()
                        .icon(IconName::Close)
                        .label("退出对比 (Esc)")
                        .on_click(|_, window, cx| {
                            window.dispatch_action(Box::new(Escape), cx);
                        }),
                ),
        )
        // ── 窗格网格 ──
        .child(
            h_flex()
                .w_full()
                .flex_1()
                .gap_3()
                .children(slots.iter().enumerate().map(|(slot_idx, &item_idx)| {
                    let is_focused = slot_idx == focused_slot;
                    let meta = state.items.get(item_idx).cloned();
                    let thumb_path = meta.as_ref().and_then(|m| {
                        state.image_manager.get_thumbnail_path(
                            &m.primary_path,
                            m.file_size.unwrap_or(0),
                            THUMB_SIZE_GRID,
                        )
                    });

                    let stars = match meta.as_ref().map(|m| m.rating).unwrap_or(Rating::None) {
                        Rating::None => 0,
                        Rating::One => 1,
                        Rating::Two => 2,
                        Rating::Three => 3,
                        Rating::Four => 4,
                        Rating::Five => 5,
                    };

                    v_flex()
                        .id(("compare-slot", slot_idx))
                        .flex_1()
                        .h_full()
                        .bg(cx.theme().popover)
                        .rounded(px(14.))
                        .border_2()
                        .border_color(if is_focused {
                            cx.theme().primary
                        } else {
                            cx.theme().border.opacity(0.5)
                        })
                        .shadow(if is_focused {
                            crate::theme::floating_toolbar_shadow(cx)
                        } else {
                            crate::theme::panel_card_shadow(cx)
                        })
                        .hover(|s| s.border_color(cx.theme().primary.opacity(0.7)))
                        .overflow_hidden()
                        .cursor_pointer()
                        .on_click(cx.listener(move |state, _, _, cx| {
                            state.compare_focused_slot = slot_idx;
                            cx.notify();
                        }))
                        // 窗格头
                        .child(
                            h_flex()
                                .w_full()
                                .h(px(36.))
                                .items_center()
                                .justify_between()
                                .px_3()
                                .bg(if is_focused {
                                    cx.theme().selection
                                } else {
                                    cx.theme().muted
                                })
                                .border_b_1()
                                .border_color(cx.theme().border.opacity(0.6))
                                .text_xs()
                                .child(
                                    div()
                                        .font_medium()
                                        .truncate()
                                        .text_color(cx.theme().foreground)
                                        .child(
                                            meta.as_ref()
                                                .map(|m| m.base_name.clone())
                                                .unwrap_or_default(),
                                        ),
                                )
                                .child(if stars > 0 {
                                    ComponentRating::new(format!("compare-star-{slot_idx}"))
                                        .value(stars)
                                        .max(5)
                                        .disabled(true)
                                        .small()
                                        .into_any_element()
                                } else {
                                    div().into_any_element()
                                }),
                        )
                        // 窗格主图
                        .child(
                            div()
                                .w_full()
                                .flex_1()
                                .bg(cx.theme().background)
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(if let Some(path) = thumb_path {
                                    img(path).max_w_full().max_h_full().into_any_element()
                                } else {
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child("加载中...")
                                        .into_any_element()
                                }),
                        )
                })),
        )
        .into_any_element()
}
