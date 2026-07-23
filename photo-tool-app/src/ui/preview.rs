use std::sync::Arc;

use gpui::*;

use gpui_component::{Icon, IconName, Sizable};
use crate::state::app::RootView;
use crate::ui::theme;

/// Render the full-size preview for the selected image.
pub fn render_preview(view: &RootView, cx: &mut Context<RootView>) -> impl IntoElement {
    let focused = view.get_focused_capture();
    let thumbnail_data = view.thumbnail_data.clone();
    let has_prev = view.focus_index.map_or(false, |i| i > 0);
    let has_next = view
        .focus_index
        .map_or(false, |i| i + 1 < view.display_order.len());

    let thumbnail_bytes = focused.and_then(|meta| {
        let idx = meta.index;
        thumbnail_data.get(&idx).cloned()
    });

    let view_handle = cx.entity().downgrade();

    div()
        .flex()
        .flex_col()
        .size_full()
        .bg(theme::colors().background)
        .child(
            // Navigation bar
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .px_4()
                .py_1()
                .bg(theme::colors().surface_background)
                .border_b_1()
                .border_color(theme::colors().border_variant)
                .child(
                    div()
                        .text_sm()
                        .text_color(theme::colors().text)
                        .child(
                            focused
                                .map(|m| m.base_name.clone())
                                .unwrap_or_else(|| "无图片".into()),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme::colors().text_muted)
                        .child(format!(
                            "{} / {}",
                            view.focus_index.map_or(0, |i| i + 1),
                            view.display_order.len()
                        )),
                ),
        )
        .child(
            // Image area with navigation arrows
            div()
                .flex()
                .flex_row()
                .flex_grow(1.0)
                .items_center()
                .justify_center()
                .child(
                    // Left arrow
                    div()
                        .id("prev-arrow")
                        .flex()
                        .items_center()
                        .justify_center()
                        .w(px(56.))
                        .h(px(56.))
                        .rounded_full()
                        .bg(if has_prev { theme::colors().element_background } else { theme::colors().border_variant })
                        .text_color(if has_prev { theme::colors().text } else { theme::colors().text_muted })
                        .cursor(if has_prev {
                            CursorStyle::PointingHand
                        } else {
                            CursorStyle::default()
                        })
                        .hover(|style| {
                            if has_prev { style.bg(theme::colors().element_hover) } else { style }
                        })
                        .child(Icon::new(IconName::ChevronLeft).with_size(px(20.)).text_color(if has_prev { theme::colors().text } else { theme::colors().text_muted }))
                        .on_click({
                            let vh = view_handle.clone();
                            move |_event: &ClickEvent, _window, cx| {
                                if let Some(view) = vh.upgrade() {
                                    let _ = cx.update_entity(&view, |root_view, root_cx| {
                                        root_view
                                            .dispatch_action(
                                                crate::action::Action::Prev,
                                                root_cx,
                                            );
                                    });
                                }
                            }
                        }),
                )
                .child(
                    // Center image area
                    div()
                        .flex()
                        .flex_grow(1.0)
                        .items_center()
                        .justify_center()
                        .h_full()
                        .p_4()
                        .child(match &thumbnail_bytes {
                            Some(bytes) => {
                                let img_obj = Image::from_bytes(ImageFormat::Jpeg, bytes.clone());
                                img(Arc::new(img_obj))
                                    .object_fit(ObjectFit::Contain)
                                    .size_full()
                                    .into_any_element()
                            }
                            None => div()
                                .flex()
                                .flex_col()
                                .items_center()
                                .gap_4()
                                .child(
                                    div()
                                        .text_color(theme::colors().text)
                                        .child("未选择图片"),
                                )
                                .child(
                                    div()
                                        .text_color(theme::colors().text_muted)
                                        .child("从网格中选择图片进行预览"),
                                )
                                .into_any_element(),
                        }),
                )
                .child(
                    // Right arrow
                    div()
                        .id("next-arrow")
                        .flex()
                        .items_center()
                        .justify_center()
                        .w(px(56.))
                        .h(px(56.))
                        .rounded_full()
                        .bg(if has_next { theme::colors().element_background } else { theme::colors().border_variant })
                        .text_color(if has_next { theme::colors().text } else { theme::colors().text_muted })
                        .cursor(if has_next {
                            CursorStyle::PointingHand
                        } else {
                            CursorStyle::default()
                        })
                        .hover(|style| {
                            if has_next { style.bg(theme::colors().element_hover) } else { style }
                        })
                        .child(Icon::new(IconName::ChevronRight).with_size(px(20.)).text_color(if has_next { theme::colors().text } else { theme::colors().text_muted }))
                        .on_click({
                            let vh = view_handle.clone();
                            move |_event: &ClickEvent, _window, cx| {
                                if let Some(view) = vh.upgrade() {
                                    let _ = cx.update_entity(&view, |root_view, root_cx| {
                                        root_view
                                            .dispatch_action(
                                                crate::action::Action::Next,
                                                root_cx,
                                            );
                                    });
                                }
                            }
                        }),
                ),
        )
        .child(
            // Zoom controls
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_center()
                .gap_4()
                .px_4()
                .bg(theme::colors().surface_background)
                .border_t_1()
                .border_color(theme::colors().border_variant)
                .child(zoom_button(IconName::Minus, "zoom-out", view_handle.clone()))
                .child(
                    div()
                        .text_sm()
                        .text_color(theme::colors().text_muted)
                        .child("100%"),
                )
                .child(zoom_button(IconName::Plus, "zoom-in", view_handle.clone()))
                .child(zoom_button(IconName::Frame, "zoom-fit", view_handle))
        )
}

fn zoom_button(icon: IconName, id: &str, view_handle: WeakEntity<RootView>) -> impl IntoElement {
    let vh = view_handle.clone();
    let owned_id = id.to_string();
    div()
        .id(ElementId::Name(format!("zoom-{owned_id}").into()))
        .flex()
        .items_center()
        .justify_center()
        .w(px(36.))
        .h(px(36.))
        .rounded_md()
        .bg(theme::colors().element_background)
        .text_color(theme::colors().text)
        .text_sm()
        .cursor(CursorStyle::PointingHand)
        .child(Icon::new(icon.clone()).small().text_color(theme::colors().text))
        .on_click(move |_event: &ClickEvent, _window, cx| {
            if let Some(view) = vh.upgrade() {
                let _ = cx.update_entity(&view, |root_view, root_cx| {
                    match icon {
                        IconName::Plus => root_view.dispatch_action(
                            crate::action::Action::ZoomIn,
                            root_cx,
                        ),
                        IconName::Minus => root_view.dispatch_action(
                            crate::action::Action::ZoomOut,
                            root_cx,
                        ),
                        _ => root_view.dispatch_action(
                            crate::action::Action::ZoomToFit,
                            root_cx,
                        ),
                    }
                });
            }
        })
}
