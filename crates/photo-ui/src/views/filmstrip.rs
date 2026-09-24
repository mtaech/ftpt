//! 胶片条组件（对应 §4.9 与 §9.6）。
//!
//! 预览态底部的横向照片缩略图带：
//! - 高度 80px
//! - 缩略图项宽 96px，间距 6px
//! - 当前选中项带 2px accent 描边
//! - 单击切图，右键上下文操作

use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::{
    Context, IntoElement, MouseButton, Role, TestSupportExt as _, Window, div, img, prelude::*,
    px,
};

use crate::image::THUMB_SIZE_GRID;
use crate::model::filmstrip::{THUMB_GAP, THUMB_W, content_width};
use crate::state::AppState;
use crate::views::scroll_area::scroll_area_h;

pub fn render_filmstrip(
    state: &AppState,
    window: &mut Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let active_idx = state.primary_selected_index();
    let count = state.display_order.len();
    // 横向滚动的内容必须显式撑宽（flex 项默认被拉伸到容器宽，不给宽度滚动条长度为 0）。
    // 公式与「切图自动定位」共用 `model::filmstrip`，两边各写一份必然漂移。
    let content_w = content_width(count);

    // 切图（键盘 / 网格点击 / 统计跳转 / 导入…）后把当前那张滚进可视区：用户报「切换到单张的
    // 时候底下的滚动条没有同步定位」。视口尺寸要等本帧布局完才有，所以真正的偏移放 defer 里做
    // （与筛选栏同步候选同一手法）；只在「扫描世代 / 主选中项」变化后做一次，手动拖动不会被打断。
    let sync_key = (state.scan_generation, active_idx);
    if state.filmstrip_synced.get() != Some(sync_key) {
        cx.defer_in(window, move |state, _window, cx| {
            if state.filmstrip_synced.get() == Some(sync_key) {
                return;
            }
            state.sync_filmstrip_to_active(sync_key);
            cx.notify();
        });
    }

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
                    .w(px(THUMB_W))
                    .h(px(72.))
                    .flex_shrink_0()
                    .rounded(px(10.))
                    .overflow_hidden()
                    .relative()
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
                    // 右键菜单（§13.4）：与网格同一套
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(
                            move |state, event: &gpui_kit::MouseDownEvent, _window, cx| {
                                // 命中项已在选中集里 → 保留整批多选，只把主选中移到它身上
                                state.focus_selection_on(item_idx);
                                let path = state
                                    .items
                                    .get(item_idx)
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
                    // 无障碍（§13.4）：胶片条是 List，每一项是 ListItem（带文件名与当前态）。
                    // 名字由 model::a11y 的纯函数拼，视图里不拼字符串。
                    .role(Role::ListItem)
                    .aria_label(crate::model::filmstrip_item_label(
                        &meta.display_name(),
                        is_active,
                        meta.has_adjustments,
                    ))
                    .aria_selected(is_active)
                    .test_support()
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
                    // 右上角：「已调整」小徽标（96×72 里只放一个词，避免压住缩略图主体）
                    .when(meta.has_adjustments, |this| {
                        this.child(
                            div()
                                .absolute()
                                .top_1()
                                .right_1()
                                .px_1p5()
                                .py_0p5()
                                .rounded_full()
                                .bg(cx.theme().primary)
                                .text_size(px(9.))
                                .text_color(cx.theme().primary_foreground)
                                .child("调整"),
                        )
                    })
                    .into_any_element(),
            )
        })
        .collect();

    scroll_area_h(
        "filmstrip-scroller",
        &state.filmstrip_scroll,
        content_w,
        h_flex()
            .id("filmstrip-list")
            .role(Role::List)
            .aria_label(crate::model::filmstrip_label(thumbs.len()))
            .test_support()
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
