use std::sync::Arc;

use gpui::*;

use gpui_component::menu::ContextMenuExt as _;
use gpui_component::{Icon, IconName, Sizable};
use crate::state::app::RootView;
use crate::ui::theme;

/// Render the full-size preview for the selected image.
pub fn render_preview(
    view: &RootView,
    window: &Window,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    let focused = view.get_focused_capture();
    let thumbnail_data = view.thumbnail_data.clone();

    // 优先 1600px 预览数据；未加载完成时回退缩略图
    let image_source = focused.and_then(|meta| {
        let idx = meta.index;
        view.preview_data
            .get(&idx)
            .map(|(f, b)| (*f, b.clone()))
            .or_else(|| thumbnail_data.get(&idx).map(|b| (ImageFormat::Jpeg, b.clone())))
    });

    // 计算中间区的显式宽度，替代 size_full（GPUI img 配合字节源时 size_full + object_fit 不生效）
    let viewport_w: f32 = window.viewport_size().width.into();
    let viewport_h: f32 = window.viewport_size().height.into();
    let left_w = view.config.left_panel_width as f32 + crate::ui::layout::RAIL_WIDTH;
    let right_w = if view.config.right_panel_visible {
        crate::ui::layout::RIGHT_PANEL_WIDTH
    } else {
        0.
    };
    let center_w = viewport_w - left_w - right_w - 16. * 2. - 2.;
    let center_w = center_w.max(100.);
    // 可用高度 = 视口 − 工具栏(~40) − 状态栏(~24) − 导航栏(~32) − 缩略图条(~77) − 缩放栏(~28) − 图片padding(32)
    let center_h = (viewport_h - 40. - 24. - 32. - 77. - 28. - 32.).max(100.);
    // 按原始比例计算适配尺寸：同时约束宽度和高度，竖图也能顶满
    let (img_w, img_h) = focused
        .and_then(|m| Some((m.image_width?, m.image_height?)))
        .map(|(w, h)| {
            let scale = (center_w / w as f32).min(center_h / h as f32).min(1.0);
            (w as f32 * scale, h as f32 * scale)
        })
        .unwrap_or((center_w, center_h * 0.75));

    // 应用缩放倍率
    let zoom = view.preview_zoom;
    let (disp_w, disp_h) = if zoom == 0.0 {
        focused
            .and_then(|m| Some((m.image_width?, m.image_height?)))
            .map(|(w, h)| (w as f32, h as f32))
            .unwrap_or((img_w, img_h))
    } else {
        (img_w * zoom, img_h * zoom)
    };
    let zoom_label = if zoom == 0.0 {
        "100%".to_string()
    } else {
        format!("{:.0}%", zoom * 100.)
    };

    // 手动计算居中偏移（替代 flex items_center/justify_center），缩放时从中心展开
    let pad_px = 16.0;
    let container_w = (center_w - pad_px * 2.).max(1.);
    let container_h = (center_h - pad_px * 2.).max(1.);
    let img_x = ((container_w - disp_w) / 2.).max(-disp_w).min(0.) + view.preview_pan.0;
    let img_y = ((container_h - disp_h) / 2.).max(-disp_h).min(0.) + view.preview_pan.1;


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
            // Image area（导航用方向键或底部缩略图条）
            div()
                .flex()
                .flex_row()
                .flex_grow(1.0)
                .min_h(px(0.))
                .overflow_hidden()
                .child(
                    // Center image area（淡灰背景 + 滚轮缩放 + 拖拽平移）
                    div()
                        .id("preview-image-area")
                        .flex()
                        .flex_grow(1.0)
                        .flex_shrink_1()
                        .min_w(px(0.))
                        .overflow_hidden()
                        .h_full()
                        .p_4()
                        .bg(theme::colors().element_background)
                        .on_scroll_wheel({
                            let vh = view_handle.clone();
                            move |event: &ScrollWheelEvent, _window, cx| {
                                let delta_y: f32 = match event.delta {
                                    ScrollDelta::Pixels(p) => p.y.into(),
                                    ScrollDelta::Lines(l) => l.y * 20.,
                                };
                                if let Some(view) = vh.upgrade() {
                                    let _ = cx.update_entity(&view, |root_view, root_cx| {
                                        if delta_y > 0. {
                                            root_view.dispatch_action(crate::action::Action::ZoomIn, root_cx);
                                        } else {
                                            root_view.dispatch_action(crate::action::Action::ZoomOut, root_cx);
                                        }
                                    });
                                }
                            }
                        })
                        .on_mouse_down(MouseButton::Left, {
                            let vh = view_handle.clone();
                            move |event: &MouseDownEvent, _window, cx| {
                                if let Some(view) = vh.upgrade() {
                                    let pos = event.position;
                                    let _ = cx.update_entity(&view, |root_view, _cx| {
                                        let x: f32 = pos.x.into();
                                        let y: f32 = pos.y.into();
                                        root_view.preview_drag = Some((x, y, root_view.preview_pan.0, root_view.preview_pan.1));
                                    });
                                }
                            }
                        })
                        .on_mouse_move({
                            let vh = view_handle.clone();
                            move |event: &MouseMoveEvent, _window, cx| {
                                if event.pressed_button != Some(MouseButton::Left) { return; }
                                if let Some(view) = vh.upgrade() {
                                    let pos = event.position;
                                    let _ = cx.update_entity(&view, |root_view, root_cx| {
                                        if let Some((sx, sy, spx, spy)) = root_view.preview_drag {
                                            let cx_pos: f32 = pos.x.into();
                                            let cy_pos: f32 = pos.y.into();
                                            root_view.preview_pan = (spx + (cx_pos - sx), spy + (cy_pos - sy));
                                            root_cx.notify();
                                        }
                                    });
                                }
                            }
                        })
                        .on_mouse_up(MouseButton::Left, {
                            let vh = view_handle.clone();
                            move |_event: &MouseUpEvent, _window, cx| {
                                if let Some(view) = vh.upgrade() {
                                    let _ = cx.update_entity(&view, |root_view, _cx| {
                                        root_view.preview_drag = None;
                                    });
                                }
                            }
                        })
                        .context_menu({
                            let vh = view_handle.clone();
                            move |menu, window, cx| {
                                let meta = vh
                                    .upgrade()
                                    .and_then(|view| view.read(cx).get_focused_capture().cloned());
                                crate::ui::context_menu::capture_menu(
                                    menu,
                                    meta.as_ref(),
                                    true,
                                    window,
                                    cx,
                                )
                            }
                        })
                        .child(match &image_source {
                            Some((fmt, bytes)) => {
                                let img_obj = Image::from_bytes(*fmt, bytes.clone());
                                // 绝对定位脱离文档流：拖动/缩放只改偏移，不参与 flex 布局，
                                // 否则 margin 会改变内容固有尺寸，把左右面板顶移位。
                                // 坐标原点 = 父容器左上，需补回 p_4 的 16px 内边距。
                                div()
                                    .absolute()
                                    .left(px(img_x + pad_px))
                                    .top(px(img_y + pad_px))
                                    .child(
                                        img(Arc::new(img_obj))
                                            .w(px(disp_w))
                                            .h(px(disp_h)),
                                    )
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
        )
        .child(crate::ui::filmstrip::render_filmstrip(view, cx))
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
                .child(zoom_button(IconName::Minus, "zoom-out", crate::action::Action::ZoomOut, false, view_handle.clone()))
                .child(
                    div()
                        .text_sm()
                        .text_color(theme::colors().text_muted)
                        .child(zoom_label),
                )
                .child(zoom_button(IconName::Plus, "zoom-in", crate::action::Action::ZoomIn, false, view_handle.clone()))
                .child(zoom_button(IconName::Frame, "zoom-fit", crate::action::Action::ZoomToFit, false, view_handle.clone()))
        )
}

fn zoom_button(icon: IconName, id: &str, action: crate::action::Action, active: bool, view_handle: WeakEntity<RootView>) -> impl IntoElement {
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
        .bg(if active { theme::colors().element_hover } else { theme::colors().element_background })
        .text_color(theme::colors().text)
        .text_sm()
        .cursor(CursorStyle::PointingHand)
        .child(Icon::new(icon).small().text_color(theme::colors().text))
        .on_click(move |_event: &ClickEvent, _window, cx| {
            if let Some(view) = vh.upgrade() {
                let _ = cx.update_entity(&view, |root_view, root_cx| {
                    root_view.dispatch_action(action, root_cx);
                });
            }
        })
}
