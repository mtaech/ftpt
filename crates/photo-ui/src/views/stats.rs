//! 物种统计视图组件（对应 §10.6）。
//!
//! - 顶部三卡：物种总数 / 照片总数 / 覆盖文件夹数
//! - 左栏：物种排行列表（条数、占比；点击选中该物种）
//! - 右栏：选中物种的照片网格（缩略图按**各自照片目录**的 .pt/thumbs 定位；
//!   点击某张跳转到它所在文件夹并选中）
//! - 退出返回网格

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    tag::Tag,
    v_flex,
};
use gpui_kit::{AnyElement, Context, IntoElement, ScrollHandle, Window, div, img, prelude::*, px};
use photo_engine::global_db::SpeciesStat;

use crate::actions::Escape;
use crate::state::AppState;
use crate::state::app_state::StatsPhoto;
use crate::state::engine_ops::{defer_entity_action, open_stats_photo, select_stats_species};

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

    // 右栏数据先拷出来：下面构造元素时要可变借用 cx（listener），不能同时借 state
    let selected = state.stats_selected_species.clone();
    let photos = state.stats_photos.clone();
    let photo_total = state.stats_photo_total;

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
        // ── 顶部三卡 ──
        .child(
            h_flex()
                .w_full()
                .gap_3()
                .child(render_stat_card(
                    "物种总数",
                    &format!("{total_species}"),
                    "全库去重种数",
                    cx,
                ))
                .child(render_stat_card(
                    "照片总数",
                    &format!("{total_photos}"),
                    "已识别主体记录",
                    cx,
                ))
                .child(render_stat_card(
                    "覆盖文件夹",
                    &format!("{folder_count}"),
                    "已扫描入库目录",
                    cx,
                ))
        )
        // ── 主体双栏：物种排行 / 该物种照片 ──
        .child(
            h_flex()
                .w_full()
                .flex_1()
                .gap_4()
                .overflow_hidden()
                .child(render_stats_ranking(
                    &stats,
                    total_photos,
                    selected.as_deref(),
                    &state.stats_species_scroll,
                    cx,
                ))
                .child(render_stats_photos(
                    &photos,
                    photo_total,
                    selected.as_deref(),
                    &state.stats_photos_scroll,
                    cx,
                )),
        )
        .into_any_element()
}

/// 左栏：物种排行榜（点击某行 → 右栏显示该物种的照片）。
fn render_stats_ranking(
    stats: &[SpeciesStat],
    total_photos: i64,
    selected: Option<&str>,
    scroll: &ScrollHandle,
    cx: &mut Context<AppState>,
) -> impl IntoElement + use<> {
    let rows: Vec<AnyElement> = stats
        .iter()
        .enumerate()
        .map(|(idx, item)| {
            let pct = if total_photos > 0 {
                (item.photo_count as f64 / total_photos as f64) * 100.0
            } else {
                0.0
            };
            let is_selected = selected == Some(item.species_name.as_str());
            let border = if is_selected {
                cx.theme().primary
            } else {
                cx.theme().border.opacity(0.4)
            };
            let bg = if is_selected {
                cx.theme().primary.opacity(0.10)
            } else {
                cx.theme().background
            };
            let name_for_click = item.species_name.clone();

            h_flex()
                .id(("stats-species-row", idx))
                .w_full()
                .p_2()
                .rounded(px(8.))
                .bg(bg)
                .border_1()
                .border_color(border)
                .items_center()
                .justify_between()
                .text_xs()
                .cursor_pointer()
                .on_click(cx.listener(move |state, _, _, cx| {
                    // 同步查询全局索引 + 至多 240 次 metadata：前台毫秒级，无需后台
                    select_stats_species(state, &name_for_click);
                    cx.notify();
                }))
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
                                .child(item.species_name.clone()),
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
                            Tag::secondary()
                                .small()
                                .child(format!("{} 条", item.photo_count)),
                        ),
                )
                .into_any_element()
        })
        .collect();

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
                .child("物种排行榜（点击查看照片）"),
        )
        .child(
            crate::views::scroll_area::scroll_area_v(
                "stats-species-scroll",
                scroll,
                v_flex().w_full().gap_1p5().children(rows),
            )
            .w_full()
            .flex_1(),
        )
}

/// 右栏：选中物种的照片网格。
///
/// 缩略图取各自照片目录里**已生成**的缓存（.pt/thumbs）；尚未生成的显示占位，
/// 不在这里解码/生成（统计页不该为浏览付解码成本）。
fn render_stats_photos(
    photos: &[StatsPhoto],
    total: usize,
    selected: Option<&str>,
    scroll: &ScrollHandle,
    cx: &mut Context<AppState>,
) -> impl IntoElement + use<> {
    let panel_base = || {
        div()
            .flex_1()
            .h_full()
            .bg(cx.theme().popover)
            .rounded(px(14.))
            .border_1()
            .border_color(cx.theme().border.opacity(0.6))
            .shadow(crate::theme::panel_card_shadow(cx))
    };

    let Some(species) = selected else {
        return panel_base()
            .p_6()
            .flex()
            .items_center()
            .justify_center()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child("选择左侧物种查看照片记录")
            .into_any_element();
    };

    // 跨文件夹分布（同一文件夹只算一次）
    let mut folders: Vec<&str> = photos.iter().map(|p| p.folder.as_str()).collect();
    folders.sort_unstable();
    folders.dedup();
    let folder_kinds = folders.len();
    let truncated = photos.len() < total;

    let mut tiles: Vec<AnyElement> = Vec::new();
    for (i, p) in photos.iter().enumerate() {
        let folder = p.folder.clone();
        let rel_path = p.rel_path.clone();
        let full_label = p.full_path.to_string_lossy().to_string();
        let tip = full_label.clone();
        let file_label = p
            .full_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| p.rel_path.clone());
        let hover_border = cx.theme().primary.opacity(0.6);
        let muted = cx.theme().muted;

        let mut tile = div()
            .id(("stats-photo", i))
            .w(px(104.))
            .h(px(104.))
            .flex_shrink_0()
            .rounded(px(8.))
            .border_1()
            .border_color(cx.theme().border.opacity(0.4))
            .bg(cx.theme().background)
            .overflow_hidden()
            .relative()
            .cursor_pointer()
            .hover(move |s| s.border_color(hover_border))
            .tooltip(move |window, cx| {
                gpui_kit::component::tooltip::Tooltip::new(tip.clone()).build(window, cx)
            })
            .on_click(cx.listener(move |_state, _, _, cx| {
                // listener 内 AppState 已租借：跳转要 start_scan/update，必须 defer
                let entity = cx.entity().clone();
                let folder = folder.clone();
                let rel_path = rel_path.clone();
                defer_entity_action(cx, entity, move |entity, cx| {
                    open_stats_photo(entity, folder.clone(), rel_path.clone(), cx);
                });
            }));

        tile = match &p.thumb_path {
            Some(thumb) => tile.child(
                img(thumb.clone())
                    .size_full()
                    .object_fit(gpui_kit::ObjectFit::Cover),
            ),
            None => tile.child(
                v_flex()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .p_1()
                    .bg(muted)
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(file_label),
                    ),
            ),
        };
        tiles.push(tile.into_any_element());
    }

    let mut panel = panel_base().p_3().flex().flex_col().gap_2();
    panel = panel.child(
        h_flex()
            .w_full()
            .items_center()
            .justify_between()
            .gap_2()
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .font_medium()
                            .text_sm()
                            .text_color(cx.theme().foreground)
                            .child(species.to_string()),
                    )
                    .child(Tag::secondary().small().child(format!("{total} 条"))),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(if truncated {
                        format!("跨 {folder_kinds} 个文件夹 · 仅显示前 {} 条", photos.len())
                    } else {
                        format!("跨 {folder_kinds} 个文件夹")
                    }),
            ),
    );

    if photos.is_empty() {
        return panel
            .child(
                div()
                    .flex_1()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("该物种暂无照片记录"),
            )
            .into_any_element();
    }

    panel
        .child(
            crate::views::scroll_area::scroll_area_v(
                "stats-photos-scroll",
                scroll,
                h_flex().w_full().flex_wrap().gap_2().children(tiles),
            )
            .w_full()
            .flex_1(),
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