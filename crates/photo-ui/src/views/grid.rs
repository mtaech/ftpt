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
use gpui_kit::base::ElementExt as _;
use gpui_kit::component::dock::DockPlacement;
use gpui_kit::component::scroll::{Scrollbar, ScrollbarHandle as _};
use gpui_kit::{
    Context, IntoElement, MouseButton, Role, TestSupportExt as _, Window, div, img, prelude::*,
    px, uniform_list,
};
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
    window: &mut Window,
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

    // 缩略图区边长 = cell 的实际宽度（正方形，§9.5「object-cover 正方裁切」）。
    // 必须给确定像素高度——uniform_list 假定每行等高，让高度反过来跟宽度联动
    // （aspect-ratio）会被它测量歪，行内容直接被裁掉。
    // 宽度优先用**列表自己的视口宽度**（与滚动条同源、就是 cell 摊开的那个宽度）；
    // 拿不到时退回「容器实测宽 − 内边距」，再退到按窗口估算。
    let list_vw = f32::from(state.grid_scroll.viewport_bounds().size.width);
    let container_w = state
        .grid_viewport_size
        .map(|(w, _)| w as f32)
        .unwrap_or_else(|| estimate_grid_width(state, window, cx));
    let content_w = if list_vw > 10.0 {
        list_vw
    } else {
        (container_w - 16.0).max(0.0)
    };
    let thumb_edge = thumb_edge_for(content_w, cols);

    // 滚动条：GPUI 的溢出滚动容器只负责滚、不负责画——必须把 uniform_list 绑到
    // AppState 的句柄上（跨帧保留滚动位置），再把 Scrollbar 叠在同一个视口里。
    let scroll_handle = state.grid_scroll.clone();

    div()
        .id("photo-grid-viewport")
        .w_full()
        .h_full()
        .bg(cx.theme().background)
        .p(px(8.))
        // 实测容器宽度回填 AppState（post-layout）；只有变化才 notify，避免自激重绘
        .on_prepaint({
            let entity = cx.entity().clone();
            move |bounds, _window, cx| {
                let w = f32::from(bounds.size.width) as f64;
                let h = f32::from(bounds.size.height) as f64;
                if w > 10.0 && h > 10.0 {
                    entity.update(cx, |state, cx| {
                        let changed = match state.grid_viewport_size {
                            Some((cur_w, cur_h)) => {
                                (cur_w - w).abs() >= 1.0 || (cur_h - h).abs() >= 1.0
                            }
                            None => true,
                        };
                        if changed {
                            state.grid_viewport_size = Some((w, h));
                            cx.notify();
                        }
                    });
                }
            }
        })
        .child(
            // 无障碍（§13.4「网格无表格语义」）：网格 = Grid 容器（带行列数），
            // 行 = Row，格子 = GridCell（带行列下标 + 名称 + 选中态）。
            // 读屏软件靠 role + aria_label 才能念出「第几行第几列、哪张、选没选中」。
            // `.test_support()` 是 gpui-base 的观察点（未启用 test-support 时原样返回），
            // `a11y_smoke` 用它把 role / label / selected 读回来做断言。
            div()
                .id("photo-grid-a11y")
                .role(Role::Grid)
                .aria_label(crate::model::grid_container_label(
                    state.display_order.len(),
                    cols,
                    state.selected_indices.len(),
                ))
                .aria_row_count(row_count)
                .aria_column_count(cols)
                .test_support()
                .relative()
                .size_full()
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
                                    div()
                                        .id(("grid-row", row_idx as usize))
                                        .role(Role::Row)
                                        .aria_row_index(row_idx as usize)
                                        .test_support()
                                        .w_full()
                                        .pb(px(8.))
                                        .child(
                                        h_flex()
                                            .w_full()
                                            .gap(px(8.))
                                            .children(groups_in_row.iter().enumerate().map(
                                                |(c_idx, group)| {
                                                    let global_group_idx = start + c_idx;
                                                    render_grid_cell(
                                                        state,
                                                        group,
                                                        global_group_idx,
                                                        thumb_edge,
                                                        cols,
                                                        row_count,
                                                        cx,
                                                    )
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
                    .h_full()
                    .track_scroll(&scroll_handle),
                )
                .child(Scrollbar::vertical(&scroll_handle)),
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

/// cell 内图片区边长（正方形）=（内容宽 − 列间距）/ 列数。
///
/// 不给「最小边长」兜底：内容宽不够时 cell 会继续收缩（cell 也不再设 min_w），
/// 若在这里兜一个下限，图片会比 cell 宽、行还会溢出视口。
fn thumb_edge_for(content_w: f32, cols: usize) -> f32 {
    let gaps = 8.0 * (cols.saturating_sub(1)) as f32; // 行内 h_flex gap(px(8.))
    ((content_w - gaps) / cols as f32).max(1.0)
}

/// 首帧兜底：按「窗口宽 − 左右活动栏 − 停靠区」估算网格容器宽度
fn estimate_grid_width(state: &AppState, window: &mut Window, cx: &mut Context<AppState>) -> f32 {
    let mut avail = f32::from(window.viewport_size().width) - 96.0; // 左右活动栏各 48px
    if let Some(area) = &state.dock_area {
        let area = area.read(cx);
        if area.is_dock_open(DockPlacement::Left) {
            avail -= area
                .dock_size(DockPlacement::Left)
                .map(f32::from)
                .unwrap_or(state.left_panel_width);
        }
        if area.is_dock_open(DockPlacement::Right) {
            avail -= area
                .dock_size(DockPlacement::Right)
                .map(f32::from)
                .unwrap_or(state.right_panel_width);
        }
    } else {
        avail -= state.left_panel_width + state.right_panel_width;
    }
    avail.max(200.0)
}

fn render_grid_cell(
    state: &AppState,
    group: &crate::model::stacks::StackGroup,
    group_idx: usize,
    thumb_edge: f32,
    cols: usize,
    row_count: usize,
    cx: &Context<AppState>,
) -> impl IntoElement {
    let active_item_idx = group.active;
    // 表格语义里的行列下标（0 基）：uniform_list 按行喂数据，格子位置由组序号推出
    let cell_row = group_idx / cols.max(1);
    let cell_col = group_idx % cols.max(1);
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

    // 显示用文件名：base_name（无扩展名）+ 主路径真实扩展名（domain::display_name）
    let file_name = meta
        .map(|m| m.display_name())
        .unwrap_or_else(|| "IMG".to_string());
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
    let taxon_name = meta.and_then(|m| m.taxon_name.clone());
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
        .id(("grid-cell", active_item_idx))
        .flex_1()
        // 不设 min_w：5 列在窄面板里必须能继续收缩，否则整行溢出、最后一列被裁
        // （uniform_list 横向不滚动，行宽必须小于等于视口宽）
        .min_w(px(0.))
        .cursor_pointer()
        // 列窄时文件名会截断，悬停整格看完整文件名（含扩展名）。
        // 挂在 cell 上而不是名字元素上：cell 本来就有 on_mouse_down，不额外加子 hitbox
        .tooltip({
            let full = file_name.clone();
            move |window, cx| {
                gpui_kit::component::tooltip::Tooltip::new(full.clone()).build(window, cx)
            }
        })
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
        // 右键菜单（§13.4）：命中的照片若已在选中集里就保留多选，否则单选到它，
        // 然后在该点开菜单。位置是**窗口坐标**（与 MouseDownEvent.position 同口径），
        // 菜单浮层直接按它做绝对定位。
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(
                move |state: &mut AppState, event: &gpui_kit::MouseDownEvent, _window, cx| {
                    let target = active_item_idx;
                    // 命中项已在选中集里 → 保留整批多选，只把主选中移到它身上
                    state.focus_selection_on(target);
                    let path = state
                        .items
                        .get(target)
                        .map(|m| m.primary_path.clone())
                        .unwrap_or_default();
                    state.open_photo_context_menu(
                        f32::from(event.position.x),
                        f32::from(event.position.y),
                        path,
                        cx,
                    );
                },
            ),
        )
        // 表格语义（§13.4）：角色 + 名称 + 行列下标 + 选中态。
        // 名称由 model::a11y 的纯函数拼（可单测）：文件名 + 已选中 + N 星 + Pick/Reject + 物种。
        .role(Role::GridCell)
        .aria_label(crate::model::grid_cell_label(
            &file_name,
            is_selected,
            rating,
            flag,
            taxon_name.as_deref(),
            meta.map_or(false, |m| m.has_adjustments),
        ))
        .aria_selected(is_selected)
        .aria_row_index(cell_row)
        .aria_column_index(cell_col)
        .aria_row_count(row_count)
        .aria_column_count(cols)
        .test_support()
        // ── 1. 图片区 ──
        // 正方形裁切（§9.5）：边长由网格容器实测宽度推出。以前写死 170px，侧栏一收
        // cell 变宽，ObjectFit::Cover 就把照片裁成 1.9:1 的扁横条，观感很差。
        .child(
            div()
                .w_full()
                .h(px(thumb_edge))
                // 固定高度不参与 flex 压缩：uniform_list 用 MinContent 高度探测行高，
                // 允许压缩会把行高探小（首行图片被压扁、后续行互相叠）
                .flex_shrink_0()
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
                        // 左上角：格式胶囊徽标 + 「已调整」标记（同一行，避免互相压住）
                        .child(
                            h_flex()
                                .absolute()
                                .top_2()
                                .left_2()
                                .gap_1()
                                .child(
                                    div()
                                        .px_2()
                                        .py_0p5()
                                        .rounded_full()
                                        .bg(gpui_kit::rgba(0x0000_00a0))
                                        .text_size(px(10.))
                                        .font_semibold()
                                        .text_color(gpui_kit::white())
                                        .child(badge_str.clone()),
                                )
                                .when(meta.map_or(false, |m| m.has_adjustments), |this| {
                                    this.child(
                                        div()
                                            .px_2()
                                            .py_0p5()
                                            .rounded_full()
                                            .bg(cx.theme().primary)
                                            .text_size(px(10.))
                                            .font_semibold()
                                            .text_color(cx.theme().primary_foreground)
                                            .child("已调整"),
                                    )
                                })
                                .when(meta.is_some_and(|m| m.is_empty_source()), |this| {
                                    this.child(
                                        div()
                                            .px_2()
                                            .py_0p5()
                                            .rounded_full()
                                            .bg(crate::theme::hex_to_hsla(crate::theme::COLOR_REJECT))
                                            .text_size(px(10.))
                                            .font_semibold()
                                            .text_color(gpui_kit::white())
                                            .child("空文件"),
                                    )
                                }),
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
                        // 右下角：多主体数量徽标（>1 主体时显示 ×N）
                        .when(meta.map_or(false, |m| m.subjects.len() > 1), |this| {
                            this.child(
                                div()
                                    .absolute()
                                    .bottom_2()
                                    .right_2()
                                    .px_2()
                                    .py_0p5()
                                    .rounded_full()
                                    .bg(gpui_kit::rgba(0x0000_00a0))
                                    .text_size(px(10.))
                                    .font_semibold()
                                    .text_color(gpui_kit::white())
                                    .child(format!("×{}", meta.map_or(0, |m| m.subjects.len()))),
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
                .flex_shrink_0()
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
                        .gap_1()
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .text_xs()
                                .font_medium()
                                .truncate()
                                .text_color(footer_fg)
                                .child(file_name.clone()),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_xs()
                                .text_color(footer_muted)
                                .child(file_size_str),
                        ),
                )
                // 第 2 行：星级评分 + 物种名称
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
                                .child(taxon_name.unwrap_or_default()),
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
