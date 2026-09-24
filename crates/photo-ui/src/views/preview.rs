//! 单图预览视图组件（对应 §4.6、§7.1–§7.4 与 §9.6）。
//!
//! - 图片区：缩放平移视口，图源 master / full
//! - 叠加层（指针事件穿透）：
//!   - 主体检测框（V 键）：2px accent 描边 + 20% 填充
//!   - 对焦点（F 键）：十字准星（focus 颜色）
//!   - 剪切警告（O 键）：高光溢出与死黑叠加
//!   - 手动框选矩形：工具条「框选」toggle 或**按住 Shift + 左键拖拽**（快捷键），warning 橙
//! - 视口左上角常驻框选文字提示：把 Shift 快捷键摆出来，文案/颜色随 Shift 与框选模式切换
//! - 底部胶囊工具条：
//!   - 放大 / 缩小 / 适应 / 1:1
//!   - 堆叠分段选择器
//!   - 检测框 / 对焦点 / 剪切 / 返回网格
//! - 底部胶片条（80px）
//! - 视频等非图片格式：不解码、不抽帧，只给「用默认软件打开」一条出口（§7.5）

use std::path::PathBuf;

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

use crate::actions::{
    CopyImage, ToggleBbox, ToggleClipping, ToggleFocus, ToggleRegionSelect, ToggleView, ZoomIn,
    ZoomOut,
};
use crate::image::{MASTER_SIZE, THUMB_SIZE_GRID, source_file_of};
use crate::model::preview_math::{
    clamp_pan_axis, exceeds_master_res, fit_scale, preview_center_offset, region_bbox_from_drag,
};
use crate::model::region::{region_hint_text, starts_region_drag};
use crate::state::AppState;
use crate::state::engine_ops::{
    defer_entity_action, load_preview_full, load_preview_image, start_region_recognition,
};
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

    // 调整参数（ADR 0007）：装载焦点图的参数；非中性时预览走「调整母版」，不再请求原图母版
    state.ensure_adjustments_loaded(&meta.primary_path, window, cx);
    let adjust_active = state.adjust_path.as_deref() == Some(meta.primary_path.as_str())
        && !state.adjust.is_neutral();
    // before/after：按住反斜杠时改用未调整母版（没有调整时预览本来就是原图，无需切换）
    let holding_original = state.before_after_held && adjust_active;

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
    if !has_master
        && !adjust_active
        && state.preview_request.as_deref() != Some(meta.primary_path.as_str())
    {
        if let Some(src) = source.clone() {
            state.preview_request = Some(meta.primary_path.clone());
            load_preview_image(
                cx.entity().clone(),
                state.image_manager.clone(),
                meta.primary_path.clone(),
                src,
                state.show_clipping,
                cx,
            );
        }
    }

    // 只有 1:1 才用原图；RAW 退回母版（渲染层解不了 RAW）
    let is_raw = source
        .as_ref()
        .is_some_and(|s| matches!(s.format, photo_domain::ImageFormat::Raw(_)));
    let original_path = std::path::PathBuf::from(&meta.primary_path);
    // 有调整时不切原图：原文件没有烘焙参数，1:1 会静默丢掉调整效果（全尺寸重算见 ADR 遗留项）
    let one_to_one =
        state.preview_zoom == 0.0 && !is_raw && original_path.exists() && !adjust_active;

    let master = state
        .preview_image
        .as_ref()
        .filter(|(p, _)| p == &meta.primary_path)
        .map(|(_, img)| img.clone());

    // before/after 的图源：未调整母版（可能还在后台加载；未就绪时保持调整图，不闪白）
    let base_image = if holding_original {
        state
            .preview_base_image
            .as_ref()
            .filter(|(p, _)| p == &meta.primary_path)
            .map(|(_, img)| img.clone())
    } else {
        None
    };

    // 显示尺寸超过母版可用像素（母版长边 = MASTER_SIZE）时需要真原图：1:1 与高倍放大都属此列。
    // 常规图直接渲染原文件（one_to_one），RAW 渲染层解不了，改后台加载全分辨率母版
    // （get_or_generate_full：AHD 全尺寸 + 落盘缓存）。否则 1:1 就是把 2560 母版放大 3 倍多。
    let need_full =
        is_raw && exceeds_master_res((disp_w, disp_h), MASTER_SIZE) && !adjust_active;
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

    // 有调整时，显示尺寸一旦超过调整链路（1600）的像素，就请求全分辨率调整帧；
    // 未就绪前先沿用放大后的母版（不闪、不空白），就绪后替换。
    let want_full_adjusted = adjust_active
        && !holding_original
        && exceeds_master_res((disp_w, disp_h), crate::image::ADJUST_SOURCE_SIZE);
    let toned_full_ready = state
        .preview_full_toned_params
        .as_ref()
        .is_some_and(|(p, _)| p == &meta.primary_path);
    let full_toned = if want_full_adjusted && toned_full_ready {
        state
            .preview_full
            .as_ref()
            .filter(|(p, _)| p == &meta.primary_path)
            .map(|(_, img)| img.clone())
    } else {
        None
    };
    if want_full_adjusted && full_toned.is_none() {
        // 停手闸门 + 同 (path, 参数) 去重都在 request_adjust_full 里
        state.request_adjust_full(cx);
    }

    let zoom_pct = if state.preview_zoom == 0.0 {
        100
    } else if natural_w > 0.0 {
        ((disp_w / natural_w) * 100.0).round() as i32
    } else {
        100
    };

    let entity = cx.entity().clone();
    // 图片显示 div 的 prepaint 需要第二个句柄（`entity` 已被视口 prepaint 的闭包吃掉）
    let image_entity = cx.entity().clone();
    let is_dragging = state.preview_drag_start.is_some();
    let can_pan = disp_w > container_w || disp_h > container_h;
    // 剪切警告叠加（§7.4）：掩码按图片路径归属，只画属于当前这张图的那份
    // 掩码算在调整后的像素上：按住看原图时它不对应当前画面，必须一起收起来
    let clip_mask = if state.show_clipping && !holding_original {
        state
            .preview_clip_mask
            .as_ref()
            .filter(|(p, _)| p == &meta.primary_path)
            .map(|(_, img)| img.clone())
    } else {
        None
    };
    // 框选：叠加层用值 + 触发识别要的文件路径（BBox 是 Copy）
    let region_overlay = state.region_bbox;
    let region_path = meta.primary_path.clone();

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
                } else if state.region_select
                    || state.region_shift_held
                    || state.region_drag_start.is_some()
                {
                    // 画框态（toggle 开启 / 按住 Shift / 拖拽中）：十字光标
                    CursorStyle::Crosshair
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
                // 跟踪 Shift（光标 + 左上角提示文字实时切到「已按住 Shift」）；
                // 只在状态真变化时 notify，修饰符事件来得很密
                .on_modifiers_changed(cx.listener(
                    |state, event: &gpui_kit::ModifiersChangedEvent, _window, cx| {
                        if state.region_shift_held != event.modifiers.shift {
                            state.region_shift_held = event.modifiers.shift;
                            cx.notify();
                        }
                    },
                ))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|state, event: &gpui_kit::MouseDownEvent, _window, cx| {
                        if event.click_count == 2 {
                            // 双击缩放（框选模式下也保留）
                            if (state.preview_zoom - 1.0).abs() < 1e-4 {
                                state.preview_zoom = 0.0;
                            } else {
                                state.preview_zoom = 1.0;
                            }
                            state.preview_pan = (0.0, 0.0);
                            cx.notify();
                        } else if starts_region_drag(state.region_select, event.modifiers.shift) {
                            // 框选：工具条 toggle 开启，或按住 Shift（快捷键）——左键拖拽画框，
                            // 都不进入平移
                            state.region_drag_start = Some((
                                f32::from(event.position.x) as f64,
                                f32::from(event.position.y) as f64,
                            ));
                            state.region_bbox = None;
                            cx.notify();
                        } else {
                            state.preview_drag_start = Some((event.position.x, event.position.y));
                        }
                    }),
                )
                // 右键菜单（§13.4）：目标是当前这张预览图
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(|state, event: &gpui_kit::MouseDownEvent, _window, cx| {
                        // 框选拖拽中右键 = 取消这次框（比弹菜单更符合直觉）
                        if state.region_drag_start.take().is_some() {
                            state.region_bbox = None;
                            cx.notify();
                            return;
                        }
                        let path = state
                            .primary_selected_meta()
                            .map(|m| m.primary_path.clone())
                            .unwrap_or_default();
                        if path.is_empty() {
                            return;
                        }
                        state.open_photo_context_menu(
                            f32::from(event.position.x),
                            f32::from(event.position.y),
                            path,
                            cx,
                        );
                    }),
                )
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(move |state, event: &gpui_kit::MouseUpEvent, _window, cx| {
                        // 拖拽模式在 mouse_down 时就定了（工具条 toggle 或按住 Shift）：这里只看
                        // 有没有框选起点。若改回看 region_select，「Shift 画框、松手前先放掉 Shift」
                        // 就会掉进平移分支——框画了却不识别。
                        if let Some(start) = state.region_drag_start.take() {
                            let end = (
                                f32::from(event.position.x) as f64,
                                f32::from(event.position.y) as f64,
                            );
                            // 面积过小的框视为误点击（不触发识别）
                            // 鼠标事件是窗口坐标，锚在图片显示 div 的窗口 bounds 上换算
                            let bbox = state
                                .preview_image_rect
                                .and_then(|rect| region_bbox_from_drag(start, end, rect))
                                .filter(|b| (b.x2 - b.x1) * (b.y2 - b.y1) >= 0.0004);
                            state.region_bbox = bbox;
                            if let Some(b) = bbox {
                                // listener 内 AppState 已租借，直接 update 会 panic：
                                // 用 defer_entity_action 排到本轮 effect 之后（同识别/扫描入口）
                                let entity = cx.entity().clone();
                                let path = PathBuf::from(region_path.clone());
                                defer_entity_action(cx, entity, move |entity, cx| {
                                    start_region_recognition(entity, path, b, cx);
                                });
                            }
                            cx.notify();
                        } else {
                            state.preview_drag_start = None;
                        }
                    }),
                )
                .on_mouse_move(cx.listener(move |state, event: &gpui_kit::MouseMoveEvent, _window, cx| {
                    // 鼠标事件本身带着修饰符：平台没投递 ModifiersChanged（无 WM / 无键盘焦点）
                    // 时，指针一动也能把提示文字与光标切到「已按住 Shift」
                    let shift = event.modifiers.shift;
                    let shift_changed = state.region_shift_held != shift;
                    state.region_shift_held = shift;

                    if let Some(start) = state.region_drag_start {
                        // 框选进行中：实时更新叠加框
                        let end = (
                            f32::from(event.position.x) as f64,
                            f32::from(event.position.y) as f64,
                        );
                        state.region_bbox = state
                            .preview_image_rect
                            .and_then(|rect| region_bbox_from_drag(start, end, rect));
                        cx.notify();
                    } else if let Some((start_x, start_y)) = state.preview_drag_start {
                        let dx = f32::from(event.position.x - start_x) as f64;
                        let dy = f32::from(event.position.y - start_y) as f64;
                        if dx.abs() > 0.0 || dy.abs() > 0.0 {
                            state.preview_pan.0 += dx;
                            state.preview_pan.1 += dy;
                            state.preview_drag_start = Some((event.position.x, event.position.y));
                            cx.notify();
                        }
                    } else if shift_changed {
                        // 只有 Shift 状态变了、又没在拖拽：重绘提示与光标
                        cx.notify();
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
                        // 图片显示 div 在**窗口**里的 bounds：框选拖拽（鼠标事件=窗口坐标）
                        // 与叠加框（画在这个 div 里）共用这一个坐标系 → 由构造保证对齐。
                        // 只记「变化 ≥0.5px」的更新，拖动/缩放/侧栏变化都会自然刷新。
                        .on_prepaint(move |bounds, _window, cx| {
                            let rect = (
                                f32::from(bounds.origin.x) as f64,
                                f32::from(bounds.origin.y) as f64,
                                f32::from(bounds.size.width) as f64,
                                f32::from(bounds.size.height) as f64,
                            );
                            image_entity.update(cx, |state, cx| {
                                let changed = match state.preview_image_rect {
                                    Some((x, y, w, h)) => {
                                        (x - rect.0).abs() >= 0.5
                                            || (y - rect.1).abs() >= 0.5
                                            || (w - rect.2).abs() >= 0.5
                                            || (h - rect.3).abs() >= 0.5
                                    }
                                    None => true,
                                };
                                if changed {
                                    state.preview_image_rect = Some(rect);
                                    cx.notify();
                                }
                            });
                        })
                        .child(if one_to_one {
                            // 1:1 原图：只有用户显式放大到实际像素才付这个代价
                            img(original_path).w_full().h_full().into_any_element()
                        } else if let Some(full_img) = full {
                            // RAW 的 1:1 / 高倍放大：全分辨率图源（已超过母版像素）
                            img(full_img).w_full().h_full().into_any_element()
                        } else if let Some(toned_full) = full_toned {
                            // 1:1 / 高倍放大：全分辨率调整帧（滑杆停手后异步重算）
                            img(toned_full).w_full().h_full().into_any_element()
                        } else if let Some(base_img) = base_image {
                            // before/after：按住反斜杠看未调整母版
                            img(base_img).w_full().h_full().into_any_element()
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
                        // ── 叠加层：剪切警告（红 = 高光溢出、蓝 = 死黑；图与主图同尺寸） ──
                        .children(clip_mask.map(|mask| {
                            div()
                                .absolute()
                                .top(px(0.))
                                .left(px(0.))
                                .w_full()
                                .h_full()
                                .opacity(0.7)
                                .child(img(mask).w_full().h_full())
                        }))
                        // ── 叠加层：主体检测框 ──
                        .when(state.show_bbox && meta.taxon_bbox.is_some(), |this| {
                            let bbox = meta.taxon_bbox.as_ref().unwrap();
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
                        // ── 叠加层：手动框选矩形（warning 橙，区分识别检测框的 primary）──
                        .when(region_overlay.is_some(), |this| {
                            let bbox = region_overlay.unwrap();
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
                                    .border_color(cx.theme().warning)
                                    .bg(cx.theme().warning.opacity(0.12)),
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
                )
                // ── 叠加层：框选操作文字提示（Shift 快捷键；纯展示，不接收指针事件）──
                // 置于图片视口左上角；三态文案见 model::region::region_hint_text
                .child(
                    div()
                        .absolute()
                        .top(px(12.))
                        .left(px(12.))
                        .px_2()
                        .py(px(3.))
                        .rounded_full()
                        .bg(cx.theme().popover.opacity(0.85))
                        .border_1()
                        .border_color(cx.theme().border.opacity(0.45))
                        .text_xs()
                        .text_color(if state.region_select || state.region_shift_held {
                            cx.theme().warning
                        } else {
                            cx.theme().muted_foreground
                        })
                        .child(region_hint_text(
                    state.region_select,
                    state.region_shift_held,
                    state.region_drag_start.is_some(),
                )),
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
                                .tooltip("缩小 (-)")
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
                                .tooltip("放大 (=)")
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
                        // 复制到系统剪贴板（全尺寸）
                        .child(
                            Button::new("copy-image")
                                .ghost()
                                .xsmall()
                                .label("复制")
                                .tooltip("复制图片到系统剪贴板（全尺寸，Ctrl+C）")
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(CopyImage), cx);
                                }),
                        )
                        .child(Separator::vertical().h(px(16.)))
                        // 框选识别（漏检补充）：开启后拖拽画框、松开识别
                        .child(
                            Button::new("toggle-region")
                                .ghost()
                                .xsmall()
                                .icon(gpui_kit::assets::IconName::Scan)
                                .tooltip("框选识别：检测漏检时手动框选补充主体（也可按住 Shift + 左键直接拖框）")
                                .selected(state.region_select)
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(ToggleRegionSelect), cx);
                                }),
                        )
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
