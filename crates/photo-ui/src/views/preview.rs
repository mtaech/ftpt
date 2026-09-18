//! 单图预览视图组件（对应 §4.6、§7.1–§7.4 与 §9.6）。
//!
//! - 图片区：缩放平移视口，图源 master / full
//! - 叠加层（指针事件穿透）：
//!   - 鸟体检测框（V 键）：2px accent 描边 + 20% 填充
//!   - 鸟眼角标：四角 L 形臂条（4–14px，不遮眼）
//!   - 对焦点（F 键）：十字准星（focus 颜色）
//!   - 剪切警告（O 键）：高光溢出与死黑叠加
//! - 底部胶囊工具条：
//!   - 放大 / 缩小 / 适应 / 1:1
//!   - 堆叠分段选择器
//!   - 检测框 / 对焦点 / 剪切 / 返回网格
//! - 底部胶片条（80px）
//! - 视频等非图片格式：不解码、不抽帧，只给「用默认软件打开」一条出口（§7.5）

use gpui_kit::base::ElementExt as _;
use gpui_kit::component::dock::DockPlacement;
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Selectable as _, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    separator::Separator,
    v_flex,
};
use gpui_kit::{
    Context, CursorStyle, Decorations, IntoElement, MouseButton, Window, div, img, prelude::*, px,
};
use photo_domain::FocusShape;

use crate::actions::{ToggleBbox, ToggleClipping, ToggleFocus, ToggleView, ZoomIn, ZoomOut};
use crate::image::{MASTER_SIZE, THUMB_SIZE_GRID, source_file_of};
use crate::model::preview_math::{
    clamp_pan_axis, exceeds_master_res, fit_scale, preview_center_offset,
};
use crate::state::AppState;
use crate::state::engine_ops::{load_preview_full, load_preview_image};
use crate::theme::COLOR_FOCUS;
use crate::views::filmstrip::render_filmstrip;

pub fn render_photo_preview(
    state: &mut AppState,
    window: &mut Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement + use<> {
    let Some(meta) = state.primary_selected_meta().cloned() else {
        return div()
            .w_full()
            .h_full()
            .bg(cx.theme().background)
            .flex()
            .items_center()
            .justify_center()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child("未选中照片")
            .into_any_element();
    };

    // 视频等非图片格式：不接解码管线，给一条明确的出口（§7.5）
    if is_video_file(&meta.primary_path) {
        return render_video_preview(state, &meta, window, cx).into_any_element();
    }

    // 视口尺寸：优先使用 post-layout prepaint 实测尺寸；首帧回退根据窗口与边栏动态估算
    let (container_w, container_h) = if let Some((w, h)) = state.preview_viewport_size {
        (w, h)
    } else {
        let win_size = window.viewport_size();
        let win_w = f32::from(win_size.width) as f64;
        let win_h = f32::from(win_size.height) as f64;

        // 左右活动栏各 48px
        let mut avail_w = win_w - 96.0;
        if let Some(area) = &state.dock_area {
            let area_ref = area.read(cx);
            if area_ref.is_dock_open(DockPlacement::Left) {
                let lw = area_ref
                    .dock_size(DockPlacement::Left)
                    .map(|p| f32::from(p) as f64)
                    .unwrap_or(state.left_panel_width as f64);
                avail_w -= lw;
            }
            if area_ref.is_dock_open(DockPlacement::Right) {
                let rw = area_ref
                    .dock_size(DockPlacement::Right)
                    .map(|p| f32::from(p) as f64)
                    .unwrap_or(state.right_panel_width as f64);
                avail_w -= rw;
            }
        } else {
            avail_w -= (state.left_panel_width + state.right_panel_width) as f64;
        }

        let is_client_dec = matches!(window.window_decorations(), Decorations::Client { .. });
        let title_h = if is_client_dec { 36.0 } else { 0.0 };
        let header_h = 44.0;
        let status_h = 24.0;
        let filmstrip_h = 80.0;
        let dock_tab_h = 32.0;

        let avail_h = win_h - title_h - header_h - status_h - filmstrip_h - dock_tab_h;
        (avail_w.max(400.0), avail_h.max(300.0))
    };

    let natural_w = meta.image_width.unwrap_or(4000) as f64;
    let natural_h = meta.image_height.unwrap_or(3000) as f64;

    // 视口内边距（16px 呼吸空间，保证适应窗口时不贴边）
    let pad = 16.0;
    let inner_w = (container_w - pad * 2.0).max(1.0);
    let inner_h = (container_h - pad * 2.0).max(1.0);

    let fit = fit_scale(inner_w, inner_h, natural_w, natural_h);
    let scale = if state.preview_zoom == 0.0 {
        1.0 // 1:1 原图自然尺寸
    } else if (state.preview_zoom - 1.0).abs() < 1e-4 {
        fit
    } else {
        fit * state.preview_zoom
    };

    let disp_w = natural_w * scale;
    let disp_h = natural_h * scale;

    let clamped_pan_x = clamp_pan_axis(disp_w, container_w, state.preview_pan.0);
    let clamped_pan_y = clamp_pan_axis(disp_h, container_h, state.preview_pan.1);
    let offset_x = preview_center_offset(disp_w, container_w) + clamped_pan_x;
    let offset_y = preview_center_offset(disp_h, container_h) + clamped_pan_y;

    // 网格缩略图：既是列表图，也是预览的**即时占位**（§7.2）
    let thumb_path = state.image_manager.get_thumbnail_path(
        &meta.primary_path,
        meta.file_size.unwrap_or(0),
        THUMB_SIZE_GRID,
    );

    // §7.2：预览绝不直接渲染原图——39MP 原图 RGBA 展开约 157MB（2560 母版只要 19MB），
    // 而且渲染层（image crate）根本不认 RAW。命中内存母版直接画；否则本帧用缩略图占位，
    // 同时在后台备母版（同一请求路径只发起一次，失败也不会重试风暴）。
    let source = source_file_of(&meta);
    let has_master = state
        .preview_image
        .as_ref()
        .is_some_and(|(p, _)| p == &meta.primary_path);
    if !has_master && state.preview_request.as_deref() != Some(meta.primary_path.as_str()) {
        if let Some(src) = source.clone() {
            state.preview_request = Some(meta.primary_path.clone());
            load_preview_image(
                cx.entity().clone(),
                state.image_manager.clone(),
                meta.primary_path.clone(),
                src,
                cx,
            );
        }
    }

    // 只有 1:1 才用原图；RAW 退回母版（渲染层解不了 RAW）
    let is_raw = source
        .as_ref()
        .is_some_and(|s| matches!(s.format, photo_domain::ImageFormat::Raw(_)));
    let original_path = std::path::PathBuf::from(&meta.primary_path);
    let one_to_one = state.preview_zoom == 0.0 && !is_raw && original_path.exists();

    let master = state
        .preview_image
        .as_ref()
        .filter(|(p, _)| p == &meta.primary_path)
        .map(|(_, img)| img.clone());

    // 显示尺寸超过母版可用像素（母版长边 = MASTER_SIZE）时需要真原图：1:1 与高倍放大都属此列。
    // 常规图直接渲染原文件（one_to_one），RAW 渲染层解不了，改后台加载全分辨率母版
    // （get_or_generate_full：AHD 全尺寸 + 落盘缓存）。否则 1:1 就是把 2560 母版放大 3 倍多。
    let need_full = is_raw && exceeds_master_res((disp_w, disp_h), MASTER_SIZE);
    let full = if need_full {
        state
            .preview_full
            .as_ref()
            .filter(|(p, _)| p == &meta.primary_path)
            .map(|(_, img)| img.clone())
    } else {
        None
    };
    if need_full
        && full.is_none()
        && state.preview_full_request.as_deref() != Some(meta.primary_path.as_str())
        && let Some(src) = source.clone()
    {
        state.preview_full_request = Some(meta.primary_path.clone());
        load_preview_full(
            cx.entity().clone(),
            state.image_manager.clone(),
            meta.primary_path.clone(),
            src,
            cx,
        );
    }

    let zoom_pct = if state.preview_zoom == 0.0 {
        100
    } else if natural_w > 0.0 {
        ((disp_w / natural_w) * 100.0).round() as i32
    } else {
        100
    };

    let entity = cx.entity().clone();
    let is_dragging = state.preview_drag_start.is_some();
    let can_pan = disp_w > container_w || disp_h > container_h;

    v_flex()
        .w_full()
        .h_full()
        .bg(cx.theme().background)
        .relative()
        .overflow_hidden()
        // ── 1. 图片主视口 ──
        .child(
            div()
                .id("photo-preview-viewport")
                .w_full()
                .flex_1()
                .relative()
                .overflow_hidden()
                .cursor(if is_dragging {
                    CursorStyle::ClosedHand
                } else if can_pan {
                    CursorStyle::OpenHand
                } else {
                    CursorStyle::Arrow
                })
                .on_prepaint(move |bounds, _window, cx| {
                    let w = f32::from(bounds.size.width) as f64;
                    let h = f32::from(bounds.size.height) as f64;
                    if w > 10.0 && h > 10.0 {
                        entity.update(cx, |state, cx| {
                            let changed = match state.preview_viewport_size {
                                Some((cur_w, cur_h)) => {
                                    (cur_w - w).abs() >= 1.0 || (cur_h - h).abs() >= 1.0
                                }
                                None => true,
                            };
                            if changed {
                                state.preview_viewport_size = Some((w, h));
                                cx.notify();
                            }
                        });
                    }
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|state, event: &gpui_kit::MouseDownEvent, _window, cx| {
                        if event.click_count == 2 {
                            if (state.preview_zoom - 1.0).abs() < 1e-4 {
                                state.preview_zoom = 0.0;
                            } else {
                                state.preview_zoom = 1.0;
                            }
                            state.preview_pan = (0.0, 0.0);
                            cx.notify();
                        } else {
                            state.preview_drag_start = Some((event.position.x, event.position.y));
                        }
                    }),
                )
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|state, _event: &gpui_kit::MouseUpEvent, _window, _cx| {
                        state.preview_drag_start = None;
                    }),
                )
                .on_mouse_move(cx.listener(|state, event: &gpui_kit::MouseMoveEvent, _window, cx| {
                    if let Some((start_x, start_y)) = state.preview_drag_start {
                        let dx = f32::from(event.position.x - start_x) as f64;
                        let dy = f32::from(event.position.y - start_y) as f64;
                        if dx.abs() > 0.0 || dy.abs() > 0.0 {
                            state.preview_pan.0 += dx;
                            state.preview_pan.1 += dy;
                            state.preview_drag_start = Some((event.position.x, event.position.y));
                            cx.notify();
                        }
                    }
                }))
                .on_scroll_wheel(cx.listener(|state, event: &gpui_kit::ScrollWheelEvent, _window, cx| {
                    let delta_y = match event.delta {
                        gpui_kit::ScrollDelta::Lines(p) => p.y,
                        gpui_kit::ScrollDelta::Pixels(p) => f32::from(p.y),
                    };
                    if delta_y.abs() < 0.01 {
                        return;
                    }
                    let factor = if delta_y > 0.0 { 1.25 } else { 1.0 / 1.25 };
                    let cur_zoom = if state.preview_zoom == 0.0 {
                        1.0
                    } else {
                        state.preview_zoom
                    };
                    state.preview_zoom = (cur_zoom * factor).clamp(0.1, 10.0);
                    cx.notify();
                }))
                .child(
                    div()
                        .absolute()
                        .top(px(offset_y as f32))
                        .left(px(offset_x as f32))
                        .w(px(disp_w as f32))
                        .h(px(disp_h as f32))
                        .child(if one_to_one {
                            // 1:1 原图：只有用户显式放大到实际像素才付这个代价
                            img(original_path).w_full().h_full().into_any_element()
                        } else if let Some(full_img) = full {
                            // RAW 的 1:1 / 高倍放大：全分辨率图源（已超过母版像素）
                            img(full_img).w_full().h_full().into_any_element()
                        } else if let Some(master_img) = master {
                            img(master_img).w_full().h_full().into_any_element()
                        } else if let Some(ref tp) = thumb_path {
                            // 占位：网格缩略图先顶上，母版到了自动替换
                            img(tp.clone()).w_full().h_full().into_any_element()
                        } else {
                            div()
                                .w_full()
                                .h_full()
                                .bg(cx.theme().muted)
                                .into_any_element()
                        })
                        // ── 叠加层：检测框与鸟眼角标 ──
                        .when(state.show_bbox && meta.bird_bbox.is_some(), |this| {
                            let bbox = meta.bird_bbox.as_ref().unwrap();
                            let bx = bbox.x1 as f64 * disp_w;
                            let by = bbox.y1 as f64 * disp_h;
                            let bw = (bbox.x2 - bbox.x1) as f64 * disp_w;
                            let bh = (bbox.y2 - bbox.y1) as f64 * disp_h;

                            this.child(
                                div()
                                    .absolute()
                                    .top(px(by as f32))
                                    .left(px(bx as f32))
                                    .w(px(bw as f32))
                                    .h(px(bh as f32))
                                    .border_2()
                                    .border_color(cx.theme().primary)
                                    .bg(cx.theme().primary.opacity(0.15)),
                            )
                        })
                        // ── 叠加层：对焦点 ──
                        .when(state.show_focus && meta.focus_point.is_some(), |this| {
                            let pt = meta.focus_point.as_ref().unwrap();
                            let fx = pt.x as f64 * disp_w;
                            let fy = pt.y as f64 * disp_h;

                            this.child(
                                div()
                                    .absolute()
                                    .top(px((fy - 14.0) as f32))
                                    .left(px((fx - 14.0) as f32))
                                    .w(px(28.))
                                    .h(px(28.))
                                    .border_1()
                                    .border_color(COLOR_FOCUS)
                                    .rounded(if pt.shape == FocusShape::Circle {
                                        px(14.)
                                    } else {
                                        px(2.)
                                    })
                                    .child(
                                        // 十字中心准星
                                        div()
                                            .absolute()
                                            .top(px(13.))
                                            .left(px(6.))
                                            .w(px(16.))
                                            .h(px(2.))
                                            .bg(COLOR_FOCUS),
                                    )
                                    .child(
                                        div()
                                            .absolute()
                                            .top(px(6.))
                                            .left(px(13.))
                                            .w(px(2.))
                                            .h(px(16.))
                                            .bg(COLOR_FOCUS),
                                    ),
                            )
                        }),
                ),
        )
        // ── 2. 底部浮动胶囊工具条 (Material You Floating Pill Toolbar) ──
        .child(
            div()
                .absolute()
                .bottom(px(96.))
                .w_full()
                .flex()
                .justify_center()
                .child(
                    h_flex()
                        .items_center()
                        .gap_1p5()
                        .px_4()
                        .py_1p5()
                        .rounded_full()
                        .bg(cx.theme().popover.opacity(0.88))
                        .border_1()
                        .border_color(cx.theme().border.opacity(0.5))
                        .shadow(crate::theme::floating_toolbar_shadow(cx))
                        .text_xs()
                        // 缩小
                        .child(
                            Button::new("zoom-out")
                                .ghost()
                                .xsmall()
                                .icon(IconName::Minus)
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(ZoomOut), cx);
                                }),
                        )
                        // 缩放百分比
                        .child(
                            div()
                                .w(px(52.))
                                .text_center()
                                .font_semibold()
                                .text_color(cx.theme().foreground)
                                .child(format!("{zoom_pct}%")),
                        )
                        // 放大
                        .child(
                            Button::new("zoom-in")
                                .ghost()
                                .xsmall()
                                .icon(IconName::Plus)
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(ZoomIn), cx);
                                }),
                        )
                        // 适应
                        .child(
                            Button::new("zoom-fit")
                                .ghost()
                                .xsmall()
                                .label("适应")
                                .on_click(cx.listener(|state, _, _, cx| {
                                    state.preview_zoom = 1.0;
                                    state.preview_pan = (0.0, 0.0);
                                    cx.notify();
                                })),
                        )
                        // 1:1
                        .child(
                            Button::new("zoom-1-1")
                                .ghost()
                                .xsmall()
                                .label("1:1")
                                .on_click(cx.listener(|state, _, _, cx| {
                                    state.preview_zoom = 0.0;
                                    state.preview_pan = (0.0, 0.0);
                                    cx.notify();
                                })),
                        )
                        .child(Separator::vertical().h(px(16.)))
                        // 检测框开关 (V)
                        .child(
                            Button::new("toggle-bbox")
                                .ghost()
                                .xsmall()
                                .icon(IconName::Asterisk)
                                .tooltip("检测框 (V)")
                                .selected(state.show_bbox)
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(ToggleBbox), cx);
                                }),
                        )
                        // 对焦点开关 (F)
                        .child(
                            Button::new("toggle-focus")
                                .ghost()
                                .xsmall()
                                .icon(IconName::Frame)
                                .tooltip("对焦点 (F)")
                                .selected(state.show_focus)
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(ToggleFocus), cx);
                                }),
                        )
                        // 剪切警告开关 (O)
                        .child(
                            Button::new("toggle-clipping")
                                .ghost()
                                .xsmall()
                                .icon(IconName::Eye)
                                .tooltip("剪切警告 (O)")
                                .selected(state.show_clipping)
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(ToggleClipping), cx);
                                }),
                        )
                        .child(Separator::vertical().h(px(16.)))
                        // 返回网格 (G)
                        .child(
                            Button::new("back-grid")
                                .ghost()
                                .xsmall()
                                .icon(IconName::LayoutDashboard)
                                .label("网格")
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(ToggleView), cx);
                                }),
                        ),
                ),
        )
        // ── 3. 胶片条（80px） ──
        .child(render_filmstrip(state, window, cx))
        .into_any_element()
}

/// 是否为视频等非图片格式（网格与预览统一走占位 + 外部打开）
fn is_video_file(path: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .and_then(photo_domain::ImageFormat::from_extension)
        .is_some_and(|f| f.is_other())
}

/// 视频预览（§7.5）：不接解码/抽帧，只提供一条出口——用系统默认播放器打开。
/// 缩放、检测框、对焦点这些叠加语义对视频不成立，所以工具条只留「返回网格」。
fn render_video_preview(
    state: &AppState,
    meta: &photo_domain::CaptureMeta,
    window: &mut Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let path = std::path::PathBuf::from(&meta.primary_path);
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_uppercase())
        .unwrap_or_default();
    let size_str = meta
        .file_size
        .map(|s| format!("{:.1} MB", s as f64 / 1_048_576.0))
        .unwrap_or_default();

    v_flex()
        .w_full()
        .h_full()
        .bg(cx.theme().background)
        .relative()
        .overflow_hidden()
        // ── 1. 中央：占位 + 唯一动作 ──
        .child(
            v_flex()
                .w_full()
                .flex_1()
                .items_center()
                .justify_center()
                .gap_2()
                .child(
                    Icon::new(IconName::Play)
                        .size(px(56.))
                        .text_color(cx.theme().muted_foreground),
                )
                .child(
                    div()
                        .text_sm()
                        .font_medium()
                        .text_color(cx.theme().foreground)
                        .child(file_name),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("{ext} 视频 · {size_str}")),
                )
                .child(
                    div().pt_2().child(
                        Button::new("open-with-default")
                            .icon(IconName::ExternalLink)
                            .label("用默认软件打开")
                            .on_click(cx.listener(move |_state, _, _, cx| {
                                let target = path.clone();
                                cx.background_executor()
                                    .spawn(async move {
                                        if let Err(e) = open::that(&target) {
                                            tracing::warn!(
                                                "外部打开失败 {}: {e}",
                                                target.display()
                                            );
                                        }
                                    })
                                    .detach();
                            })),
                    ),
                ),
        )
        // ── 2. 底部工具条：只留返回网格 ──
        .child(
            div()
                .absolute()
                .bottom(px(92.))
                .w_full()
                .flex()
                .justify_center()
                .child(
                    h_flex()
                        .items_center()
                        .gap_1()
                        .px_3()
                        .py_1()
                        .rounded(cx.theme().radius)
                        .bg(cx.theme().sidebar.opacity(0.92))
                        .border_1()
                        .border_color(cx.theme().border)
                        .shadow(crate::theme::overlay_shadow())
                        .text_xs()
                        .child(
                            Button::new("back-grid-video")
                                .ghost()
                                .xsmall()
                                .icon(IconName::LayoutDashboard)
                                .label("网格")
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(ToggleView), cx);
                                }),
                        ),
                ),
        )
        // ── 3. 胶片条 ──
        .child(render_filmstrip(state, window, cx))
        .into_any_element()
}
