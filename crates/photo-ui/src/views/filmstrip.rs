//! 胶片条组件（对应 §4.9 与 §9.6）。
//!
//! 预览态底部的横向照片缩略图带：
//! - 高度 80px
//! - 缩略图项宽 96px，间距 6px
//! - 当前选中项带 2px accent 描边
//! - 单击切图，右键上下文操作

use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::{Context, IntoElement, Window, div, img, prelude::*, px};

use crate::image::THUMB_SIZE_GRID;
use crate::state::AppState;
use crate::views::scroll_area::scroll_area_h;

/// 缩略图项宽（px，手册 §9.6）
const THUMB_W: f32 = 96.0;
/// 缩略图间距（px，手册 §9.6）
const THUMB_GAP: f32 = 6.0;

pub fn render_filmstrip(
    state: &AppState,
    _window: &mut Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let active_idx = state.primary_selected_index();
    let count = state.display_order.len();
    // 横向滚动的内容必须显式撑宽（flex 项默认被拉伸到容器宽，不给宽度滚动条长度为 0）
    let content_w = count as f32 * THUMB_W
        + count.saturating_sub(1) as f32 * THUMB_GAP
        + 16.0;

    let thumbs: Vec<gpui_kit::AnyElement> = state
        .display_order
        .iter()
        .filter_map(|&item_idx| {
            let meta = state.items.get(item_idx)?;
            let is_active = Some(item_idx) == active_idx;
            let thumb_path = state.image_manager.get_thumbnail_path(
                &meta.primary_path,
                meta.file_size.unwrap_or(0),
                THUMB_SIZE_GRID,
            );

            Some(
                div()
                    .id(("filmstrip-thumb", item_idx))
                    .w(px(96.))
                    .h(px(72.))
                    .flex_shrink_0()
                    .rounded(px(10.))
                    .overflow_hidden()
                    .border_2()
                    .border_color(if is_active {
                        cx.theme().primary
                    } else {
                        cx.theme().border.opacity(0.4)
                    })
                    .hover(|s| s.border_color(cx.theme().primary.opacity(0.7)))
                    .cursor_pointer()
                    .on_click(cx.listener(move |state, _event, _window, cx| {
                        state.select_single(item_idx);
                        cx.notify();
                    }))
                    .child(div().w_full().h_full().bg(cx.theme().background).child(
                        if let Some(path) = thumb_path {
                            img(path).w_full().h_full().into_any_element()
                        } else {
                            div()
                                .w_full()
                                .h_full()
                                .bg(cx.theme().muted)
                                .into_any_element()
                        },
                    ))
                    .into_any_element(),
            )
        })
        .collect();

    scroll_area_h(
        "filmstrip-scroller",
        &state.filmstrip_scroll,
        content_w,
        h_flex()
            .h_full()
            .px_2()
            .py_1()
            .gap(px(THUMB_GAP))
            .children(thumbs),
    )
    .w_full()
    .h(px(80.))
    .bg(cx.theme().sidebar)
    .border_t_1()
    .border_color(cx.theme().border.opacity(0.6))
}
