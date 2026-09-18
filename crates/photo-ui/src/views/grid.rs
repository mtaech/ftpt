//! 照片网格视图组件（对应 §4.8 与 §9.5）。
//!
//! - 空态：居中图标与「打开目录」按钮
//! - 网格虚拟化：按行渲染，每行 `grid_columns`（2–5）个 cell
//! - Cell：缩略图、格式徽标、旗标角标、连拍/选优徽标、堆叠成员带、58px 信息区、3px 色标底条
//! - 选中态：2px 墨框 + 反相页脚条（黑底白字），不依赖照片明暗（§6.8）
//! - 无缩略图（视频等非图片格式 / 缓存未命中）：主题色占位 + 胶片图标，不显示破图

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    rating::Rating as ComponentRating,
    v_flex,
};
use gpui_kit::{Context, IntoElement, MouseButton, Window, div, img, prelude::*, px, uniform_list};
use photo_domain::{ColorLabel, Flag, Rating};

use crate::actions::OpenDirectory;
use crate::image::THUMB_SIZE_GRID;
use crate::state::{AppState, ViewMode};
use crate::theme::{
    COLOR_LABEL_BLUE, COLOR_LABEL_GREEN, COLOR_LABEL_PURPLE, COLOR_LABEL_RED, COLOR_LABEL_YELLOW,
    hex_to_hsla,
};

pub fn render_photo_grid(
    state: &AppState,
    _window: &mut Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement + use<> {
    // ── 1. 空态：尚未打开目录 ──
    if state.current_dir.is_none() {
        return render_empty_state(cx).into_any_element();
    }

    // ── 2. 筛选后无内容 ──
    if state.stack_groups.is_empty() {
        return render_no_match_state(cx).into_any_element();
    }

    // ── 3. 网格按行渲染 ──
    let cols = state.grid_columns.clamp(2, 5) as usize;
    let total_groups = state.stack_groups.len();
    let row_count = (total_groups + cols - 1) / cols;

    div()
        .w_full()
        .h_full()
        .bg(cx.theme().background)
        .p(px(8.))
        .child(
            uniform_list(
                "grid-rows",
                row_count,
                cx.processor(move |state, range, _window, cx| {
                    let mut rows = Vec::new();
                    for row_idx in range {
                        let start: usize = (row_idx as usize) * cols;
                        let end: usize = (start + cols).min(total_groups);
                        let groups_in_row = &state.stack_groups[start..end];

                        rows.push(
                            div().w_full().pb(px(8.)).child(
                                h_flex()
                                    .w_full()
                                    .gap(px(8.))
                                    .children(groups_in_row.iter().enumerate().map(
                                        |(c_idx, group)| {
                                            let global_group_idx = start + c_idx;
                                            render_grid_cell(state, group, global_group_idx, cx)
                                        },
                                    ))
                                    .when(groups_in_row.len() < cols, |this| {
                                        let filler_count = cols - groups_in_row.len();
                                        this.children((0..filler_count).map(|_| div().flex_1()))
                                    }),
                            ),
                        );
                    }
                    rows
                }),
            )
            .h_full(),
        )
        .into_any_element()
}

fn render_empty_state(cx: &Context<AppState>) -> impl IntoElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .bg(cx.theme().background)
        .child(
            v_flex()
                .items_center()
                .justify_center()
                .p_10()
                .gap_4()
                .rounded(px(16.))
                .border_1()
                .border_color(cx.theme().border.opacity(0.6))
                .bg(cx.theme().popover)
                .shadow(crate::theme::panel_card_shadow(cx))
                .child(
                    div()
                        .w(px(64.))
                        .h(px(64.))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(cx.theme().selection)
                        .child(
                            Icon::new(IconName::FolderOpen)
                                .size(px(32.))
                                .text_color(cx.theme().primary),
                        ),
                )
                .child(
                    div()
                        .font_semibold()
                        .text_base()
                        .text_color(cx.theme().foreground)
                        .child("尚未打开照片目录"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("选择包含照片的文件夹以开始筛选与智能整理"),
                )
                .child(
                    Button::new("btn-open-folder")
                        .primary()
                        .small()
                        .icon(IconName::FolderOpen)
                        .label("打开目录...")
                        .on_click(|_, window, cx| {
                            window.dispatch_action(Box::new(OpenDirectory), cx);
                        }),
                ),
        )
}

fn render_no_match_state(cx: &Context<AppState>) -> impl IntoElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .bg(cx.theme().background)
        .child(
            v_flex()
                .items_center()
                .justify_center()
                .p_10()
                .gap_3()
                .rounded(px(16.))
                .border_1()
                .border_color(cx.theme().border.opacity(0.6))
                .bg(cx.theme().popover)
                .shadow(crate::theme::panel_card_shadow(cx))
                .child(
                    div()
                        .w(px(56.))
                        .h(px(56.))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(cx.theme().muted)
                        .child(
                            Icon::new(IconName::ChevronsUpDown)
                                .size(px(28.))
                                .text_color(cx.theme().muted_foreground),
                        ),
                )
                .child(
                    div()
                        .font_semibold()
                        .text_sm()
                        .text_color(cx.theme().foreground)
                        .child("没有找到匹配筛选条件的照片"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("请在上方筛选栏调整或重置筛选条件"),
                ),
        )
}

fn render_grid_cell(
    state: &AppState,
    group: &crate::model::stacks::StackGroup,
    _group_idx: usize,
    cx: &Context<AppState>,
) -> impl IntoElement {
    let active_item_idx = group.active;
    let meta = state.items.get(active_item_idx);

    let is_selected = state.selected_indices.contains(&active_item_idx);

    // Material You 现代化选中态：温润的 Primary Container 底衬 + Primary 强调色文字与边框
    let footer_bg = if is_selected {
        cx.theme().selection
    } else {
        cx.theme().popover
    };
    let footer_fg = if is_selected {
        cx.theme().foreground
    } else {
        cx.theme().foreground
    };
    let footer_muted = if is_selected {
        cx.theme().muted_foreground
    } else {
        cx.theme().muted_foreground
    };

    let base_name = meta.map(|m| m.base_name.as_str()).unwrap_or("IMG");
    let file_size_str = meta
        .and_then(|m| m.file_size)
        .map(|s| format!("{:.1} MB", s as f64 / 1_048_576.0))
        .unwrap_or_default();
    let format_str = meta.map(|m| m.primary_format.clone()).unwrap_or_default();
    let badge_str = if format_str == "OTHER" {
        meta.and_then(|m| {
            std::path::Path::new(&m.primary_path)
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_uppercase())
        })
        .unwrap_or_else(|| "OTHER".to_string())
    } else {
        format_str.clone()
    };
    let rating = meta.map(|m| m.rating).unwrap_or(Rating::None);
    let color_label = meta.map(|m| m.color_label).unwrap_or(ColorLabel::None);
    let flag = meta.and_then(|m| m.flag);
    let bird_name = meta.and_then(|m| m.bird_name.clone());
    let is_best = meta.is_some_and(|m| state.best_frame_paths.contains(&m.primary_path));

    let thumb_path = meta.and_then(|m| {
        let source = crate::image::source_file_of(m)?;
        state
            .image_manager
            .get_cached_thumb_path(&source, THUMB_SIZE_GRID)
    });

    let cell_border = if is_selected {
        cx.theme().primary
    } else {
        cx.theme().border.opacity(0.5)
    };

    v_flex()
        .flex_1()
        .min_w(px(160.))
        .cursor_pointer()
        .rounded(px(12.))
        .border_2()
        .border_color(cell_border)
        .bg(cx.theme().popover)
        .shadow(crate::theme::panel_card_shadow(cx))
        .overflow_hidden()
        .hover(move |s| s.border_color(cx.theme().primary.opacity(0.6)))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(
                move |state: &mut AppState, event: &gpui_kit::MouseDownEvent, _window, cx| {
                    let target = active_item_idx;
                    if event.click_count == 2 {
                        state.select_single(target);
                        state.view_mode = ViewMode::Preview;
                        cx.notify();
                        return;
                    }

                    let is_ctrl = event.modifiers.control || event.modifiers.platform;
                    let is_shift = event.modifiers.shift;

                    if is_ctrl {
                        state.toggle_select(target);
                    } else if is_shift {
                        state.select_range(target);
                    } else {
                        state.select_single(target);
                    }
                    cx.notify();
                },
            ),
        )
        // ── 1. 图片区 ──
        .child(
            div()
                .w_full()
                .h(px(170.))
                .bg(cx.theme().muted)
                .relative()
                .overflow_hidden()
                .border_b_1()
                .border_color(cx.theme().border.opacity(0.4))
                .when_some(thumb_path, |this, path| {
                    this.child(img(path).size_full().object_fit(gpui_kit::ObjectFit::Cover))
                })
                .when(meta.is_some(), |this| {
                    this
                        // 左上角：格式胶囊徽标 (全圆角磨砂深底)
                        .child(
                            div()
                                .absolute()
                                .top_2()
                                .left_2()
                                .px_2()
                                .py_0p5()
                                .rounded_full()
                                .bg(gpui_kit::rgba(0x0000_00a0))
                                .text_size(px(10.))
                                .font_semibold()
                                .text_color(gpui_kit::white())
                                .child(badge_str.clone()),
                        )
                        // 居中：非图片格式（视频）
                        .when(format_str == "OTHER", |this| {
                            this.child(
                                v_flex()
                                    .absolute()
                                    .inset_0()
                                    .items_center()
                                    .justify_center()
                                    .gap_1()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(Icon::new(IconName::Play).size(px(28.)))
                                    .child(div().text_size(px(11.)).font_medium().child("视频")),
                            )
                        })
                        // 右上角：Material 3 选中对勾胶囊 (Filled Check Badge)
                        .when(is_selected, |this| {
                            this.child(
                                div()
                                    .absolute()
                                    .top_2()
                                    .right_2()
                                    .w(px(22.))
                                    .h(px(22.))
                                    .rounded_full()
                                    .bg(cx.theme().primary)
                                    .shadow(crate::theme::panel_card_shadow(cx))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(
                                        Icon::new(IconName::Check)
                                            .size(px(13.))
                                            .text_color(cx.theme().primary_foreground),
                                    ),
                            )
                        })
                        // 右上角：旗标角标（若已选中则靠左并列展示）
                        .when_some(flag, |this, f| {
                            this.child(
                                div()
                                    .absolute()
                                    .top_2()
                                    .when(is_selected, |s| s.right(px(30.)))
                                    .when(!is_selected, |s| s.right_2())
                                    .w(px(20.))
                                    .h(px(20.))
                                    .rounded_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .bg(crate::theme::hex_to_hsla(match f {
                                        Flag::Pick => crate::theme::COLOR_PICK,
                                        Flag::Reject => crate::theme::COLOR_REJECT,
                                    }))
                                    .child(
                                        Icon::new(match f {
                                            Flag::Pick => IconName::Check,
                                            Flag::Reject => IconName::Close,
                                        })
                                        .size(px(11.))
                                        .text_color(gpui_kit::white()),
                                    ),
                            )
                        })
                        // 左下角：连拍选优徽标（琥珀星胶囊）
                        .when(is_best, |this| {
                            this.child(
                                div()
                                    .absolute()
                                    .bottom_2()
                                    .left_2()
                                    .px_2()
                                    .py_0p5()
                                    .rounded_full()
                                    .bg(gpui_kit::rgba(0x0000_00a0))
                                    .text_size(px(10.))
                                    .font_semibold()
                                    .child(
                                        h_flex()
                                            .items_center()
                                            .gap_1()
                                            .child(
                                                Icon::new(IconName::Star).size(px(10.)).text_color(
                                                    crate::theme::hex_to_hsla(
                                                        crate::theme::COLOR_RATING,
                                                    ),
                                                ),
                                            )
                                            .child(
                                                div().text_color(gpui_kit::white()).child("最优"),
                                            ),
                                    ),
                            )
                        })
                }),
        )
        // ── 2. 信息区（58px） ──
        .child(
            v_flex()
                .w_full()
                .h(px(58.))
                .p_2()
                .bg(footer_bg)
                .justify_between()
                .gap_1()
                // 第 1 行：文件名 + 文件大小
                .child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_xs()
                                .font_medium()
                                .truncate()
                                .text_color(footer_fg)
                                .child(base_name.to_string()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(footer_muted)
                                .child(file_size_str),
                        ),
                )
                // 第 2 行：星级评分 + 鸟种名称
                .child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .justify_between()
                        .child(render_rating_stars(active_item_idx, rating))
                        .child(
                            div()
                                .text_xs()
                                .font_medium()
                                .truncate()
                                .text_color(footer_fg)
                                .child(bird_name.unwrap_or_default()),
                        ),
                ),
        )
        // ── 3. 底缘：3px 色标药丸条 ──
        .child(
            div()
                .w_full()
                .bg(footer_bg)
                .px_2()
                .pb_1()
                .child(
                    div()
                        .w_full()
                        .h(px(3.))
                        .rounded_full()
                        .bg(match color_label {
                            ColorLabel::Red => hex_to_hsla(COLOR_LABEL_RED),
                            ColorLabel::Yellow => hex_to_hsla(COLOR_LABEL_YELLOW),
                            ColorLabel::Green => hex_to_hsla(COLOR_LABEL_GREEN),
                            ColorLabel::Blue => hex_to_hsla(COLOR_LABEL_BLUE),
                            ColorLabel::Purple => hex_to_hsla(COLOR_LABEL_PURPLE),
                            ColorLabel::None => gpui_kit::rgba(0x00000000).into(),
                        }),
                ),
        )
}

fn render_rating_stars(idx: usize, rating: Rating) -> impl IntoElement {
    let stars = match rating {
        Rating::None => 0,
        Rating::One => 1,
        Rating::Two => 2,
        Rating::Three => 3,
        Rating::Four => 4,
        Rating::Five => 5,
    };

    if stars == 0 {
        return div().into_any_element();
    }

    ComponentRating::new(format!("grid-star-{idx}"))
        .value(stars)
        .max(5)
        .disabled(true)
        .small()
        .into_any_element()
}
