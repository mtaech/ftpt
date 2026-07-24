use std::ops::Range;

use gpui::*;
use gpui_component::InteractiveElementExt;
use gpui_component::scroll::ScrollableElement;



use crate::state::app::RootView;
use crate::ui::grid_cell::render_grid_cell;
use crate::ui::theme;

/// Render the thumbnail grid using uniform_list.
/// 按视口宽度把 cell 分批成行，每行是列表的一项，实现多列网格 + 行级虚拟化。
pub fn render_grid(
    view: &RootView,
    window: &Window,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    let item_count = view.display_order.len();
    let captures = view.captures.clone();
    let display_order = view.display_order.clone();
    let selected = view.selected.clone();
    let thumbnail_size = view.config.thumbnail_size;
    let cell_size = thumbnail_size as f32 + 56.; // thumbnail + 两行信息区
    let thumbnail_data = view.thumbnail_data.clone();
    let scroll_handle = &view.grid_scroll_handle;

    // 按可用宽度精确计算列数与卡宽：行内卡片为固定像素尺寸
    let viewport_w: f32 = window.viewport_size().width.into();
    let right_w = if view.config.right_panel_visible {
        crate::ui::layout::RIGHT_PANEL_WIDTH
    } else {
        0.
    };
    let available_w =
        (viewport_w - crate::ui::layout::RAIL_WIDTH - view.config.left_panel_width as f32 - right_w).max(cell_size);
    let cols = ((available_w / cell_size) as usize).max(1);
    let cell_w = (available_w / cols as f32).floor();
    let row_count = item_count.div_ceil(cols);

    let view_handle = cx.entity().downgrade();

    let list = gpui::uniform_list(
        "photo-grid",
        row_count,
        move |range: Range<usize>, _window: &mut Window, _app: &mut App| {
            let mut missing: Vec<usize> = Vec::new();
            let rows: Vec<AnyElement> = range
                .map(|row| {
                    let start = row * cols;
                    let end = (start + cols).min(item_count);
                    let cells = (start..end)
                        .filter_map(|i| {
                            let capture_idx = display_order.get(i)?;
                            let capture = captures.get(*capture_idx)?;
                            let is_selected = selected.contains(capture_idx);
                            let thumb = thumbnail_data.get(capture_idx);
                            if thumb.is_none() {
                                missing.push(*capture_idx);
                            }

                            let vh = view_handle.clone();
                            let idx = i;

                            Some(
                                div()
                                    .id(ElementId::Name(format!("cell-{i}").into()))
                                    // 固定像素尺寸，行列严格对齐
                                    .w(px(cell_w))
                                    .h(px(cell_size))
                                    .flex_shrink_0()
                                    .p_1()
                                    .child(render_grid_cell(capture, *capture_idx, is_selected, thumb))
                                    .on_click({
                                        let vh = vh.clone();
                                        move |event: &ClickEvent, _window, cx| {
                                            if let Some(view) = vh.upgrade() {
                                                let ctrl = event.modifiers().control;
                                                let shift = event.modifiers().shift;
                                                let _ = cx.update_entity(&view, |root_view, root_cx| {
                                                    root_view.select(idx, ctrl, shift);
                                                    root_cx.notify();
                                                });
                                            }
                                        }
                                    })
                                    .on_double_click({
                                        let vh = vh;
                                        move |_event: &ClickEvent, _window, cx| {
                                            if let Some(view) = vh.upgrade() {
                                                let _ = cx.update_entity(&view, |root_view, root_cx| {
                                                    root_view.toggle_view_mode(root_cx);
                                                    root_cx.notify();
                                                });
                                            }
                                        }
                                    }),
                            )
                        })
                        .collect::<Vec<_>>();

                    div()
                        .flex()
                        .flex_row()
                        .w_full()
                        .h(px(cell_size))
                        .children(cells)
                        .into_any_element()
                })
                .collect::<Vec<_>>();

            // 触发懒加载：可见区域内缺少缩略图的 capture
            if !missing.is_empty() {
                if let Some(view) = view_handle.upgrade() {
                    let _ = _app.update_entity(&view, |root_view, cx| {
                        for ci in missing {
                            root_view.ensure_thumbnail_loaded(ci, cx);
                        }
                    });
                }
            }

            rows
        },
    )
    .track_scroll(scroll_handle);

    div()
        .relative()
        .size_full()
        .bg(theme::colors().background)
        .pr_4() // 给滚动条留出空间，避免内容被遮挡
        .vertical_scrollbar(scroll_handle)
        .child(list.size_full())
}
