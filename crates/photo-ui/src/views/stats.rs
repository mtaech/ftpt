//! 物种统计视图组件（对应 §10.6）。
//!
//! - 顶部三卡：物种总数 / 照片总数 / 覆盖文件夹数
//! - 左栏：物种排行列表（张数、占比条、首见~末见）
//! - 右栏：选中物种的照片网格
//! - 退出返回网格

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    tag::Tag,
    v_flex,
};
use gpui_kit::{Context, IntoElement, Window, div, prelude::*, px};

use crate::actions::Escape;
use crate::state::AppState;

pub fn render_stats_view(
    state: &AppState,
    _window: &mut Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement + use<> {
    let global_db = state.global_db.as_ref();
    let stats = global_db
        .and_then(|db| db.species_stats().ok())
        .unwrap_or_default();
    let folder_count = global_db
        .and_then(|db| db.distinct_folder_count().ok())
        .unwrap_or(0);

    let total_species = stats.len();
    let total_photos: i64 = stats.iter().map(|s| s.photo_count).sum();

    v_flex()
        .w_full()
        .h_full()
        .bg(cx.theme().background)
        .p_4()
        .gap_4()
        // ── 顶部操作栏 ──
        .child(
            h_flex()
                .w_full()
                .items_center()
                .justify_between()
                .child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .child(Icon::new(IconName::Asterisk).text_color(cx.theme().primary))
                        .child(
                            div()
                                .font_semibold()
                                .text_base()
                                .text_color(cx.theme().foreground)
                                .child("物种全局统计"),
                        ),
                )
                .child(
                    Button::new("btn-exit-stats")
                        .ghost()
                        .small()
                        .icon(IconName::Close)
                        .label("退出统计 (Esc)")
                        .on_click(|_, window, cx| {
                            window.dispatch_action(Box::new(Escape), cx);
                        }),
                ),
        )
        // ── 顶部四卡 ──
        .child(
            h_flex()
                .w_full()
                .gap_3()
                // 卡片 1: 物种总数
                .child(render_stat_card(
                    "物种总数",
                    &format!("{total_species}"),
                    "全库去重种数",
                    cx,
                ))
                // 卡片 2: 照片总数
                .child(render_stat_card(
                    "照片总数",
                    &format!("{total_photos}"),
                    "已识别照片记录",
                    cx,
                ))
                // 卡片 3: 文件夹数
                .child(render_stat_card(
                    "覆盖文件夹",
                    &format!("{folder_count}"),
                    "已扫描入库目录",
                    cx,
                ))
        )
        // ── 主体双栏：物种排行 ──
        .child(
            h_flex()
                .w_full()
                .flex_1()
                .gap_4()
                .overflow_hidden()
                .child(
                    v_flex()
                        .w(px(380.))
                        .h_full()
                        .bg(cx.theme().popover)
                        .rounded(px(14.))
                        .border_1()
                        .border_color(cx.theme().border.opacity(0.6))
                        .shadow(crate::theme::panel_card_shadow(cx))
                        .p_3()
                        .gap_2()
                        .child(
                            div()
                                .text_xs()
                                .font_medium()
                                .text_color(cx.theme().muted_foreground)
                                .child("物种排行榜"),
                        )
                        .child(
                            div()
                                .id("stats-species-scroll")
                                .w_full()
                                .flex_1()
                                .overflow_y_scroll()
                                .child(v_flex().w_full().gap_1p5().children(
                                    stats.into_iter().enumerate().map(|(idx, item)| {
                                        let pct = if total_photos > 0 {
                                            (item.photo_count as f64 / total_photos as f64) * 100.0
                                        } else {
                                            0.0
                                        };

                                        h_flex()
                                            .w_full()
                                            .p_2()
                                            .rounded(px(8.))
                                            .bg(cx.theme().background)
                                            .border_1()
                                            .border_color(cx.theme().border.opacity(0.4))
                                            .items_center()
                                            .justify_between()
                                            .text_xs()
                                            .child(
                                                h_flex()
                                                    .items_center()
                                                    .gap_2()
                                                    .child(
                                                        Tag::secondary()
                                                            .small()
                                                            .child(format!("{}.", idx + 1)),
                                                    )
                                                    .child(
                                                        div()
                                                            .font_medium()
                                                            .text_color(cx.theme().foreground)
                                                            .child(item.bird_name),
                                                    ),
                                            )
                                            .child(
                                                h_flex()
                                                    .items_center()
                                                    .gap_2()
                                                    .child(
                                                        div()
                                                            .text_color(cx.theme().muted_foreground)
                                                            .child(format!("{:.1}%", pct)),
                                                    )
                                                    .child(
                                                        Tag::secondary().small().child(format!(
                                                            "{} 张",
                                                            item.photo_count
                                                        )),
                                                    ),
                                            )
                                    }),
                                )),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .h_full()
                        .bg(cx.theme().popover)
                        .rounded(px(14.))
                        .border_1()
                        .border_color(cx.theme().border.opacity(0.6))
                        .shadow(crate::theme::panel_card_shadow(cx))
                        .p_6()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("选择左侧物种查看照片记录"),
                ),
        )
        .into_any_element()
}

fn render_stat_card(
    title: &str,
    value: &str,
    subtitle: &str,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    v_flex()
        .flex_1()
        .p_4()
        .rounded(px(14.))
        .bg(cx.theme().popover)
        .border_1()
        .border_color(cx.theme().border.opacity(0.6))
        .shadow(crate::theme::panel_card_shadow(cx))
        .gap_1()
        .child(
            div()
                .text_xs()
                .font_medium()
                .text_color(cx.theme().muted_foreground)
                .child(title.to_string()),
        )
        .child(
            div()
                .text_2xl()
                .font_semibold()
                .text_color(cx.theme().primary)
                .child(value.to_string()),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(subtitle.to_string()),
        )
}
