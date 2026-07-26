use std::sync::Arc;

use gpui::*;
use gpui_component::scroll::{Scrollbar, ScrollbarShow};

use crate::state::app::RootView;
use crate::ui::theme;

/// gpui-component 滚动条厚度（WIDTH = THUMB_ACTIVE_INSET*2 + THUMB_ACTIVE_WIDTH = 16）
const SCROLLBAR_H: f32 = 16.;

/// 预览模式底部的水平缩略图条：点击跳转、滚轮横滚、高亮跟随焦点。
/// 结构：条带（52 缩略图框 + py_1(8) + border_t_1(1) = 61px）+ 滚动条行(16) = 77px。
/// track_scroll 让 ScrollHandle 记录子项位置，导航后 scroll_to_item 横向跟随。
pub fn render_filmstrip(view: &RootView, cx: &mut Context<RootView>) -> impl IntoElement {
    let view_handle = cx.entity().downgrade();
    let focus_index = view.focus_index;

    let thumbs: Vec<AnyElement> = view
        .display_order
        .iter()
        .enumerate()
        .map(|(display_idx, &capture_idx)| {
            let is_focused = Some(display_idx) == focus_index;
            let vh = view_handle.clone();

            // 外框 68x52 含 2px 边框（border-box），内容区恰好 64x48
            let content: AnyElement = if let Some(bytes) = view.thumbnail_data.get(&capture_idx) {
                // 字节源 img 必须显式尺寸（size_full + object_fit 不生效，见 preview.rs 注释）
                img(Arc::new(Image::from_bytes(ImageFormat::Jpeg, bytes.clone())))
                    .w(px(64.))
                    .h(px(48.))
                    .object_fit(ObjectFit::Cover)
                    .into_any_element()
            } else {
                // 缩略图未就绪：灰占位，worker 回调 notify 后自动替换
                div()
                    .size_full()
                    .bg(theme::colors().element_background)
                    .into_any_element()
            };

            div()
                .id(ElementId::named_usize("filmstrip-thumb", display_idx))
                .w(px(68.))
                .h(px(52.))
                .flex_shrink_0()
                .rounded_sm()
                .overflow_hidden()
                .border_2()
                // 边框恒 2px：透明/accent 切换高亮，布局不抖动
                .border_color(if is_focused {
                    theme::colors().text_accent
                } else {
                    transparent_black()
                })
                .cursor(CursorStyle::PointingHand)
                .child(content)
                .on_click(move |_event: &ClickEvent, _window, cx| {
                    if let Some(view) = vh.upgrade() {
                        let _ = cx.update_entity(&view, |root_view, root_cx| {
                            root_view.focus_index = Some(display_idx);
                            root_view.preview_zoom = 1.0;
                            root_view.preview_pan = (0.0, 0.0);
                            root_view.ensure_preview_loaded(root_cx);
                            root_view.ensure_filmstrip_thumbs_loaded(root_cx);
                            root_view.scroll_filmstrip_to(display_idx);
                            root_cx.notify();
                        });
                    }
                })
                .into_any_element()
        })
        .collect();

    let strip = div()
        // 本版本 GPUI 的 overflow_x_scroll/track_scroll 仅在 Stateful<Div> 上，必须给 id
        .id("filmstrip")
        .w_full()
        .overflow_x_scroll()
        .track_scroll(&view.filmstrip_scroll)
        .flex()
        .flex_row()
        .gap_1()
        .px_3()
        .py_1()
        .bg(theme::colors().surface_background)
        .border_t_1()
        .border_color(theme::colors().border_variant)
        .children(thumbs);

    // 条带 + 常驻水平滚动条（拖拽 thumb 快速滚动）
    div()
        .w_full()
        .flex()
        .flex_col()
        .child(strip)
        .child(
            div()
                .w_full()
                .h(px(SCROLLBAR_H))
                .bg(theme::colors().surface_background)
                .child(
                    Scrollbar::horizontal(&view.filmstrip_scroll)
                        .scrollbar_show(ScrollbarShow::Always),
                ),
        )
}
