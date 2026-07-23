use std::ops::Range;

use gpui::*;
use gpui_component::InteractiveElementExt;


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
    let cell_size = thumbnail_size as f32 + 40.; // thumbnail + info bar
    let thumbnail_data = view.thumbnail_data.clone();

    // 按视口宽度计算列数（至少 1 列）
    let viewport_w: f32 = window.viewport_size().width.into();
    let cols = ((viewport_w / cell_size) as usize).max(1);
    let row_count = item_count.div_ceil(cols);

    let view_handle = cx.entity().downgrade();

    let list = gpui::uniform_list(
        "photo-grid",
        row_count,
        move |range: Range<usize>, _window: &mut Window, _app: &mut App| {
            range
                .map(|row| {
                    let start = row * cols;
                    let end = (start + cols).min(item_count);
                    let cells = (start..end)
                        .filter_map(|i| {
                            let capture_idx = display_order.get(i)?;
                            let capture = captures.get(*capture_idx)?;
                            let is_selected = selected.contains(capture_idx);
                            let thumb = thumbnail_data.get(capture_idx);

                            let vh = view_handle.clone();
                            let idx = i;

                            Some(
                                div()
                                    .id(ElementId::Name(format!("cell-{i}").into()))
                                    .w(px(cell_size))
                                    .h(px(cell_size))
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
                        .children(cells)
                        .into_any_element()
                })
                .collect::<Vec<_>>()
        },
    );

    div()
        .size_full()
        .bg(theme::colors().background)
        .child(list.size_full())
}
