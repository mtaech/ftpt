use std::ops::Range;

use gpui::*;
use gpui_component::InteractiveElementExt;
use gpui_component::menu::ContextMenuExt as _;
use gpui_component::scroll::ScrollableElement;



use crate::state::app::RootView;
use crate::ui::grid_cell::render_grid_cell;
use crate::ui::theme;

/// Render the thumbnail grid using uniform_list.
/// 按视口宽度把 cell 分批成行，每行是列表的一项，实现多列网格 + 行级虚拟化。
pub fn render_grid(
    view: &RootView,
    _window: &Window,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    let item_count = view.display_order.len();
    let thumbnail_size = view.config.thumbnail_size;
    let scroll_handle = &view.grid_scroll_handle;

    // 固定 4 列网格：行高固定（uniform_list 等高约束），列宽由 flex 均分。
    // 不手算容器宽度——此前漏算右侧 rail/边框，两边栏展开时最右列被遮挡。
    const COLS: usize = 4;
    let cell_size = thumbnail_size as f32 + 56.;
    let row_count = item_count.div_ceil(COLS);
    let cols = COLS;

    let view_handle = cx.entity().downgrade();

    let list = gpui::uniform_list(
        "photo-grid",
        row_count,
        move |range: Range<usize>, _window: &mut Window, app: &mut App| {
            let mut missing: Vec<usize> = Vec::new();
            let Some(view) = view_handle.upgrade() else {
                return Vec::new();
            };
            tracing::debug!("渲染闭包: rows={}..{} thumbnails={}", range.start, range.end, view.read(app).thumbnail_data.len());
            // range 被下方 map 消费，先保存边界供预取区使用
            let (range_start, range_end) = (range.start, range.end);
            // 闭包在 prepaint 阶段执行（render 借用已释放，下方 update_entity 可证），
            // 直接读实体，避免每帧克隆 captures/display_order/缩略图表。
            let rows: Vec<AnyElement> = {
                let state = view.read(app);
                range
                    .map(|row| {
                        let start = row * cols;
                        let end = (start + cols).min(item_count);
                        let cells = (start..end)
                            .filter_map(|i| {
                                let ci = *state.display_order.get(i)?;
                                let capture = state.captures.get(ci)?;
                                let is_selected = state.selected.contains(&ci);
                                let thumb = state.thumbnail_data.get(&ci).cloned();
                                if thumb.is_none() {
                                    missing.push(ci);
                                }

                                let vh = view_handle.clone();
                                let idx = i;

                                Some(
                                    div()
                                        .id(ElementId::Name(format!("cell-{i}").into()))
                                        // 行高固定、列宽 flex 均分（行列严格对齐）
                                        .flex_1()
                                        .min_w_0()
                                        .h(px(cell_size))
                                        .p_1()
                                        .child(render_grid_cell(capture, ci, is_selected, thumb))
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
                                                    // 用 capture_idx 在 display_order 中的位置设焦点，比 display_idx 可靠
                                                    if let Some(di) = root_view.display_order
                                                        .iter()
                                                        .position(|&c| c == ci)
                                                    {
                                                        root_view.focus_index = Some(di);
                                                        root_view.anchor = Some(di);
                                                        root_view.selected.clear();
                                                        root_view.selected.insert(ci);
                                                    }
                                                    root_view.toggle_view_mode(root_cx);
                                                    root_cx.notify();
                                                });
                                            }
                                        }
                                    })
                                    .context_menu({
                                        let vh = view_handle.clone();
                                        move |menu, window, cx| {
                                            let (meta, selected_count) = vh.upgrade().map(|view| {
                                                let _ = cx.update_entity(&view, |root_view, root_cx| {
                                                    // 右键先移动焦点/选中到被点项，菜单命令作用于该项
                                                    root_view.focus_for_context_menu(idx);
                                                    root_cx.notify();
                                                });
                                                let reader = view.read(cx);
                                                (reader.get_focused_capture().cloned(), reader.selected.len())
                                            }).unwrap_or_default();
                                            crate::ui::context_menu::capture_menu(
                                                menu,
                                                meta.as_ref(),
                                                false,
                                                selected_count,
                                                window,
                                                cx,
                                            )
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
                    .collect::<Vec<_>>()
            };

            // 保留区 = 可见区 ± 2 行：拖动滚动条时即将进入视口的行提前就绪，
            // 离开保留区的在途任务被取消（执行前快速放弃，不堵队列）
            let prefetch_start = range_start.saturating_sub(2 * cols);
            let prefetch_end = (range_end + 2 * cols).min(row_count);
            let mut keep: std::collections::HashSet<usize> = std::collections::HashSet::new();
            {
                let state = view.read(app);
                for row in prefetch_start..prefetch_end {
                    let p_start = row * cols;
                    let p_end = (p_start + cols).min(item_count);
                    for i in p_start..p_end {
                        if let Some(&ci) = state.display_order.get(i) {
                            keep.insert(ci);
                            if !state.thumbnail_data.contains_key(&ci) {
                                missing.push(ci);
                            }
                        }
                    }
                }
            }

            // 触发懒加载：保留区（可见 ± 2 行）内缺少缩略图的 capture
            if !keep.is_empty() {
                let _ = app.update_entity(&view, |root_view, cx| {
                    for ci in missing {
                        root_view.ensure_thumbnail_loaded(ci, cx);
                    }
                });
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
