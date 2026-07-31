use gpui::*;

use gpui_component::menu::ContextMenuExt as _;
use gpui_component::{Icon, IconName, Sizable};
use photo_domain::RecognitionStatus;
use crate::state::app::RootView;
use crate::ui::theme;

/// Render the full-size preview for the selected image.
pub fn render_preview(
    view: &RootView,
    window: &Window,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    let focused = view.get_focused_capture();
    // 视频等非图片格式不进预览：聚焦时预览区视为未选择（导航/网格仍可选中，识别按钮已禁用）
    let focused = focused.filter(|m| m.primary_format.to_uppercase() != "OTHER");

    // 调整视图激活：右侧调整 tab + 非中性参数（曝光/对比/饱和度/裁切任一非默认）
    let adjust_active = view.right_panel_tab == 1 && !view.current_adjust.is_neutral();

    // 放大超过预览分辨率 / 100% 时优先全分辨率；否则 1600px 预览；未加载完成回退缩略图。
    // 预览/全分辨率已在 worker 线程预解码为 RenderImage，源切换无空白帧。
    // 调整视图锁定 1600px：优先调整渲染（含 tone/裁切效果），不加载 fullres；
    // 调整渲染未就绪（显示源/首帧构建中）回退已解码预览，避免闪「加载中」。
    let need_full = !adjust_active && view.needs_fullres();
    let image_source: Option<ImageSource> = focused.and_then(|meta| {
        let idx = meta.index;
        if adjust_active {
            if let Some(img) = view.adjust_render.as_ref() {
                return Some(ImageSource::from(img.clone()));
            }
            return view.preview_data
                .get(&idx)
                .map(|i| ImageSource::from(i.clone()))
                .or_else(|| view.thumbnail_data.get(&idx).map(|i| ImageSource::from(i.clone())));
        }
        if need_full && let Some(img) = view.fullres_data.get(&idx) {
            return Some(ImageSource::from(img.clone()));
        }
        view.preview_data
            .get(&idx)
            .map(|i| ImageSource::from(i.clone()))
            .or_else(|| view.thumbnail_data.get(&idx).map(|i| ImageSource::from(i.clone())))
    });
    // 加载状态：预览未就绪（显示模糊缩略图兜底）/ 全分辨率加载中（显示放大略软的预览）
    // 调整视图不显示加载 chip（调整渲染构建中由 ensure_adjust_ready 负责，无感）
    let loading_preview = focused.is_some_and(|m| !view.preview_data.contains_key(&m.index));
    let loading_fullres = !adjust_active
        && need_full
        && focused.is_some_and(|m| !view.fullres_data.contains_key(&m.index));

    // 图片区尺寸：优先用 canvas 实测值（border-box），首帧回退手算。
    // 手算只作首帧兜底：视口 − 左右 rail − 左右面板 − 边框。
    let measured_rc = view.preview_area_bounds.clone();
    let m = *measured_rc.borrow();
    let (area_w, area_h) = if m.2 > 0. {
        (m.2, m.3)
    } else {
        let viewport_w: f32 = window.viewport_size().width.into();
        let viewport_h: f32 = window.viewport_size().height.into();
        let left_w = (if view.sidebar_visible {
            view.config.left_panel_width as f32
        } else {
            0.
        }) + crate::ui::layout::RAIL_WIDTH;
        let right_w = (if view.config.right_panel_visible {
            view.config.right_panel_width as f32
        } else {
            0.
        }) + crate::ui::layout::RAIL_WIDTH;
        (
            (viewport_w - left_w - right_w - 2.).max(100.),
            // 可用高度 ≈ 视口 − 工具栏(~40) − 状态栏(~24) − 导航栏(~32) − 缩略图条(~77) − 缩放栏(~28)
            (viewport_h - 40. - 24. - 32. - 77. - 28.).max(100.),
        )
    };

    // 内容区 = 图片区 − p_4 内边距（16×2）
    let pad_px = 16.0;
    let container_w = (area_w - pad_px * 2.).max(1.);
    let container_h = (area_h - pad_px * 2.).max(1.);
    // 按原始比例计算适配尺寸：同时约束宽度和高度，竖图也能顶满
    let (img_w, img_h) = focused
        .and_then(|m| Some((m.image_width?, m.image_height?)))
        .map(|(w, h)| {
            let scale = (container_w / w as f32).min(container_h / h as f32).min(1.0);
            (w as f32 * scale, h as f32 * scale)
        })
        .unwrap_or((container_w, container_h * 0.75));

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
    let img_x = crate::state::preview_math::preview_center_offset(disp_w, container_w) + view.preview_pan.0;
    let img_y = crate::state::preview_math::preview_center_offset(disp_h, container_h) + view.preview_pan.1;


    let view_handle = cx.entity().downgrade();

    // 加载状态浮层（ContextMenu 包装后无 .when，提前构建 Option 元素）
    let fullres_chip: Option<AnyElement> = loading_fullres.then(|| {
        // 全分辨率加载中提示（当前显示的是 1600px 预览的放大）
        div()
            .absolute()
            .top(px(24.))
            .right(px(24.))
            .px_2()
            .py_1()
            .rounded_sm()
            .bg(theme::colors().surface_background)
            .text_color(theme::colors().text_muted)
            .child("全分辨率加载中…")
            .into_any_element()
    });
    let preview_chip: Option<AnyElement> = (!adjust_active && loading_preview && image_source.is_some()).then(|| {
        // 预览解码中（当前显示模糊缩略图兜底）
        div()
            .absolute()
            .top(px(24.))
            .right(px(24.))
            .px_2()
            .py_1()
            .rounded_sm()
            .bg(theme::colors().surface_background)
            .text_color(theme::colors().text_muted)
            .child("预览解码中…")
            .into_any_element()
    });

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
                        
                        .text_color(theme::colors().text)
                        .child(
                            focused
                                .map(|m| m.base_name.clone())
                                .unwrap_or_else(|| "无图片".into()),
                        ),
                )
                .child(
                    div()
                        
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
                        // 调整视图：整区十字光标（Shift 框选语义）；裁切框/手柄子元素各自覆盖光标
                        .cursor(if adjust_active { CursorStyle::Crosshair } else { CursorStyle::Arrow })
                        .on_scroll_wheel({
                            let vh = view_handle.clone();
                            move |event: &ScrollWheelEvent, _window, cx| {
                                let delta_y: f32 = match event.delta {
                                    ScrollDelta::Pixels(p) => p.y.into(),
                                    ScrollDelta::Lines(l) => l.y * 20.,
                                };
                                if let Some(view) = vh.upgrade() {
                                    let _ = cx.update_entity(&view, |root_view, root_cx| {
                                        // 以光标为中心缩放：窗口坐标 → 容器坐标（图片区原点 + p_4 内边距）
                                        let b = *root_view.preview_area_bounds.borrow();
                                        let cursor = if b.2 > 0. {
                                            let px: f32 = event.position.x.into();
                                            let py: f32 = event.position.y.into();
                                            Some((px - b.0 - 16., py - b.1 - 16.))
                                        } else {
                                            None
                                        };
                                        root_view.zoom_step(delta_y > 0., cursor, root_cx);
                                    });
                                }
                            }
                        })
                        .on_mouse_down(MouseButton::Left, {
                            let vh = view_handle.clone();
                            move |event: &MouseDownEvent, _window, cx| {
                                if let Some(view) = vh.upgrade() {
                                    let pos = event.position;
                                    let shift = event.modifiers.shift;
                                    let _ = cx.update_entity(&view, |root_view, root_cx| {
                                        let x: f32 = pos.x.into();
                                        let y: f32 = pos.y.into();
                                        if adjust_active {
                                            // 调整视图：Shift+拖拽 = 框选裁切；命中框/手柄 = 移动/调整；未命中 = 平移（识别框选不启动）
                                            root_view.adjust_mouse_down(x, y, shift, root_cx);
                                            if root_view.crop_draw.is_none()
                                                && root_view.crop_move.is_none()
                                                && root_view.crop_resize.is_none()
                                            {
                                                root_view.preview_drag = Some((x, y, root_view.preview_pan.0, root_view.preview_pan.1));
                                            }
                                        } else if shift && root_view.get_focused_capture().is_some() {
                                            // Shift+拖拽 = 手动框选识别；普通拖拽 = 平移
                                            root_view.box_draw = Some((x, y, x, y));
                                            root_cx.notify();
                                        } else {
                                            root_view.preview_drag = Some((x, y, root_view.preview_pan.0, root_view.preview_pan.1));
                                        }
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
                                        let cx_pos: f32 = pos.x.into();
                                        let cy_pos: f32 = pos.y.into();
                                        if adjust_active {
                                            // 调整视图：裁切交互（框选/移动/手柄）或平移，统一由状态层处理（只更新 draft）
                                            root_view.adjust_mouse_move(cx_pos, cy_pos, root_cx);
                                            return;
                                        }
                                        // 画框优先：更新当前角点
                                        if let Some((sx, sy, _, _)) = root_view.box_draw {
                                            root_view.box_draw = Some((sx, sy, cx_pos, cy_pos));
                                            root_cx.notify();
                                            return;
                                        }
                                        if let Some((sx, sy, spx, spy)) = root_view.preview_drag {
                                            root_view.preview_pan = (spx + (cx_pos - sx), spy + (cy_pos - sy));
                                            root_view.clamp_preview_pan();
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
                                    let _ = cx.update_entity(&view, |root_view, root_cx| {
                                        if adjust_active {
                                            // 调整视图：裁切交互提交（框选/移动/手柄 → set_adjustment，内部清空 draft/状态）
                                            root_view.adjust_mouse_up(root_cx);
                                        } else {
                                            // 画框结束 → 提交手动框选识别（内部清除 box_draw）
                                            if root_view.box_draw.is_some() {
                                                root_view.submit_box_draw(root_cx);
                                            }
                                            // 防御：调整视图内开始的裁切拖拽若中途切 tab 被中断，清掉残留状态
                                            root_view.crop_draw = None;
                                            root_view.crop_draft = None;
                                            root_view.crop_move = None;
                                            root_view.crop_resize = None;
                                        }
                                        root_view.preview_drag = None;
                                    });
                                }
                            }
                        })
                        .context_menu({
                            let vh = view_handle.clone();
                            move |menu, window, cx| {
                                let (meta, selected_count) = vh
                                    .upgrade()
                                    .map(|view| {
                                        let reader = view.read(cx);
                                        (reader.get_focused_capture().cloned(), reader.selected.len())
                                    })
                                    .unwrap_or_default();
                                crate::ui::context_menu::capture_menu(
                                    menu,
                                    meta.as_ref(),
                                    true,
                                    selected_count,
                                    window,
                                    cx,
                                )
                            }
                        })
                        .child(
                            // 实测图片区尺寸写入 preview_area_size，变化时 defer notify 重排
                            //（手算会漏 rail/边框，且拖拽面板期间 config 宽度是旧值）
                            canvas({
                                let vh = view_handle.clone();
                                move |bounds, _window, cx| {
                                    let x: f32 = bounds.origin.x.into();
                                    let y: f32 = bounds.origin.y.into();
                                    let w: f32 = bounds.size.width.into();
                                    let h: f32 = bounds.size.height.into();
                                    let changed = {
                                        let mut slot = measured_rc.borrow_mut();
                                        let changed = (slot.2 - w).abs() > 0.5
                                            || (slot.3 - h).abs() > 0.5;
                                        *slot = (x, y, w, h);
                                        changed
                                    };
                                    if changed {
                                        if let Some(view) = vh.upgrade() {
                                            cx.defer(move |cx| {
                                                let _ = cx.update_entity(&view, |_, cx| cx.notify());
                                            });
                                        }
                                    }
                                }
                            }, |_, _, _, _| {})
                            .absolute()
                            .size_full(),
                        )
                        .child(match &image_source {
                            Some(image) => {
                                // 绝对定位脱离文档流：拖动/缩放只改偏移，不参与 flex 布局，
                                // 否则 margin 会改变内容固有尺寸，把左右面板顶移位。
                                // 坐标原点 = 父容器左上，需补回 p_4 的 16px 内边距。
                                // 调整视图（右面板调整 tab）隐藏识别叠加（检测框/眼角/pending），只显示裁切叠加
                                let bbox_el: Option<AnyElement> = if view.bbox_visible && view.right_panel_tab != 1 {
                                    focused.and_then(|meta| {
                                        let status = meta.recognition_status?;
                                        let bbox = meta.bird_bbox?;
                                        // 只画边框+淡填充，不叠加鸟种名标签（会遮挡鸟体；名字看右侧信息面板）
                                        let border_color = match status {
                                            RecognitionStatus::Confirmed => theme::colors().success,
                                            RecognitionStatus::NeedsReview => theme::colors().warning,
                                            _ => return None,
                                        };
                                        let mut fill = border_color;
                                        fill.a = 0.08;
                                        let l = bbox.x1 * disp_w;
                                        let t = bbox.y1 * disp_h;
                                        let w = (bbox.x2 - bbox.x1) * disp_w;
                                        let h_val = (bbox.y2 - bbox.y1) * disp_h;
                                        Some(
                                            div()
                                                .absolute()
                                                .left(px(l))
                                                .top(px(t))
                                                .w(px(w))
                                                .h(px(h_val))
                                                .border_2()
                                                .border_color(border_color)
                                                .bg(fill)
                                                .into_any_element()
                                        )
                                    })
                                } else {
                                    None
                                };

                                // 鸟眼角标（info 色 L 形四角标，不遮挡眼睛本体；随 V 键 bbox_visible 开关）
                                let eye_el: Option<AnyElement> = if view.bbox_visible && view.right_panel_tab != 1 {
                                    view.focused_recognition
                                        .as_ref()
                                        .and_then(|r| r.eye_bbox)
                                        .map(|eye| {
                                            eye_corner_marks(eye, disp_w, disp_h, theme::colors().info)
                                        })
                                } else {
                                    None
                                };

                                // 手动框选已提交、识别中的 pending 框（accent 色，区别于正式检测框；调整视图隐藏）
                                let pending_el: Option<AnyElement> = if view.right_panel_tab != 1 {
                                view.pending_region.map(|bbox| {
                                    let accent = theme::colors().text_accent;
                                    let mut fill = accent;
                                    fill.a = 0.10;
                                    div()
                                        .absolute()
                                        .left(px(bbox.x1 * disp_w))
                                        .top(px(bbox.y1 * disp_h))
                                        .w(px((bbox.x2 - bbox.x1) * disp_w))
                                        .h(px((bbox.y2 - bbox.y1) * disp_h))
                                        .border_2()
                                        .border_color(accent)
                                        .bg(fill)
                                        .into_any_element()
                                })
                                } else {
                                    None
                                };

                                // 裁切叠加层（调整视图：区外遮罩 + 边框 + 8 手柄；crop_draft 优先于 current_adjust.crop 显示）
                                let crop_overlay: Vec<AnyElement> = if adjust_active {
                                    match view.crop_draft.or(view.current_adjust.crop) {
                                        Some(bbox) => {
                                            let l = bbox.x1 * disp_w;
                                            let t = bbox.y1 * disp_h;
                                            let cw = (bbox.x2 - bbox.x1) * disp_w;
                                            let ch = (bbox.y2 - bbox.y1) * disp_h;
                                            let accent = theme::colors().text_accent;
                                            // 裁切区外半透明黑遮罩（上下左右 4 条，填满图外区域）
                                            let mask_color = Hsla::from(Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.45 });
                                            let mask = |mx: f32, my: f32, mw: f32, mh: f32| {
                                                div()
                                                    .absolute()
                                                    .left(px(mx))
                                                    .top(px(my))
                                                    .w(px(mw.max(0.)))
                                                    .h(px(mh.max(0.)))
                                                    .bg(mask_color)
                                            };
                                            // 8 个手柄（四角 + 四边中点，8px，accent 填充）；索引约定与状态层 crop_resize 一致
                                            let handle_pos = [
                                                (l, t), (l + cw / 2., t), (l + cw, t), (l + cw, t + ch / 2.),
                                                (l + cw, t + ch), (l + cw / 2., t + ch), (l, t + ch), (l, t + ch / 2.),
                                            ];
                                            let handle_cursor = [
                                                CursorStyle::ResizeUpRightDownLeft, // 0 左上（nwse）
                                                CursorStyle::ResizeUpDown,          // 1 上中
                                                CursorStyle::ResizeUpLeftDownRight, // 2 右上（nesw）
                                                CursorStyle::ResizeLeftRight,       // 3 右中
                                                CursorStyle::ResizeUpRightDownLeft, // 4 右下（nwse）
                                                CursorStyle::ResizeUpDown,          // 5 下中
                                                CursorStyle::ResizeUpLeftDownRight, // 6 左下（nesw）
                                                CursorStyle::ResizeLeftRight,       // 7 左中
                                            ];
                                            let mut els: Vec<AnyElement> = vec![
                                                // 上下左右 4 条遮罩
                                                mask(0., 0., disp_w, t).into_any_element(),
                                                mask(0., t + ch, disp_w, disp_h - (t + ch)).into_any_element(),
                                                mask(0., t, l, ch).into_any_element(),
                                                mask(l + cw, t, disp_w - (l + cw), ch).into_any_element(),
                                                // 边框（accent 2px；悬停移动光标——GPUI 无 Move，用 OpenHand 表示可抓取移动）
                                                div()
                                                    .absolute()
                                                    .left(px(l))
                                                    .top(px(t))
                                                    .w(px(cw))
                                                    .h(px(ch))
                                                    .border_2()
                                                    .border_color(accent)
                                                    .cursor(CursorStyle::OpenHand)
                                                    .into_any_element(),
                                            ];
                                            for (idx, (hx, hy)) in handle_pos.iter().enumerate() {
                                                els.push(
                                                    div()
                                                        .absolute()
                                                        .left(px(hx - 4.))
                                                        .top(px(hy - 4.))
                                                        .w(px(8.))
                                                        .h(px(8.))
                                                        .bg(accent)
                                                        .cursor(handle_cursor[idx])
                                                        .into_any_element(),
                                                );
                                            }
                                            els
                                        }
                                        None => {
                                            // 无裁切：图上方提示框选操作
                                            vec![div()
                                                .absolute()
                                                .top(px(8.))
                                                .left(px(8.))
                                                .px_2()
                                                .py_1()
                                                .rounded_sm()
                                                .bg(theme::colors().surface_background)
                                                .text_color(theme::colors().text_muted)
                                                .child("Shift+拖拽 框选裁切区域")
                                                .into_any_element()]
                                        }
                                    }
                                } else {
                                    Vec::new()
                                };

                                let mut container = div()
                                    .absolute()
                                    .left(px(img_x + pad_px))
                                    .top(px(img_y + pad_px))
                                    .child(
                                        img(image.clone())
                                            .w(px(disp_w))
                                            .h(px(disp_h)),
                                    );
                                if let Some(el) = bbox_el {
                                    container = container.child(el);
                                }
                                if let Some(el) = eye_el {
                                    container = container.child(el);
                                }
                                if let Some(el) = pending_el {
                                    container = container.child(el);
                                }
                                if !crop_overlay.is_empty() {
                                    container = container.children(crop_overlay);
                                }
                                container.into_any_element()

                            }
                            None => {
                                if loading_preview {
                                    // 有焦点但预览/缩略图都未就绪：加载中指示
                                    div()
                                        .flex()
                                        .size_full()
                                        .items_center()
                                        .justify_center()
                                        .child(
                                            div()
                                                .text_color(theme::colors().text_muted)
                                                .child("加载中…"),
                                        )
                                        .into_any_element()
                                } else {
                                div()
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
                                .into_any_element()
                                }
                            }
                        })
                        .children(fullres_chip)
                        .children(preview_chip)
                        // Shift+拖拽画框中的实时框（窗口坐标 → 图片区相对坐标）
                        .children(view.box_draw.map(|(x1, y1, x2, y2)| {
                            let accent = theme::colors().text_accent;
                            let mut fill = accent;
                            fill.a = 0.08;
                            div()
                                .absolute()
                                .left(px(x1.min(x2) - m.0))
                                .top(px(y1.min(y2) - m.1))
                                .w(px((x2 - x1).abs()))
                                .h(px((y2 - y1).abs()))
                                .border_2()
                                .border_color(accent)
                                .bg(fill)
                                .into_any_element()
                        }))
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
                        
                        .text_color(theme::colors().text_muted)
                        .child(zoom_label),
                )
                .child(zoom_button(IconName::Plus, "zoom-in", crate::action::Action::ZoomIn, false, view_handle.clone()))
                .child(zoom_button(IconName::Frame, "zoom-fit", crate::action::Action::ZoomToFit, zoom == 1.0, view_handle.clone()))
                .child(zoom_text_button("1:1", "zoom-actual", crate::action::Action::ZoomActual, zoom == 0.0, view_handle.clone()))
                // 检测框显隐开关（默认不显示，V 快捷键同款）
                .child(
                    div()
                        .id("bbox-toggle")
                        .flex()
                        .items_center()
                        .justify_center()
                        .px_2()
                        .h(px(36.))
                        .rounded_md()
                        .bg(if view.bbox_visible { theme::colors().element_hover } else { theme::colors().element_background })
                        .text_color(if view.bbox_visible { theme::colors().text_accent } else { theme::colors().text_muted })
                        .cursor(CursorStyle::PointingHand)
                        .on_click({
                            let vh = view_handle.clone();
                            move |_event: &ClickEvent, _window, cx| {
                                if let Some(view) = vh.upgrade() {
                                    let _ = cx.update_entity(&view, |root_view, root_cx| {
                                        root_view.dispatch_action(crate::action::Action::ToggleBbox, root_cx);
                                    });
                                }
                            }
                        })
                        .child(if view.bbox_visible { "检测框 ✓" } else { "检测框" }),
                )
        )
}

/// 缩放栏文本按钮（如 "1:1"），样式与 zoom_button 一致
fn zoom_text_button(label: &'static str, id: &str, action: crate::action::Action, active: bool, view_handle: WeakEntity<RootView>) -> impl IntoElement {
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
        .cursor(CursorStyle::PointingHand)
        .child(label)
        .on_click(move |_event: &ClickEvent, _window, cx| {
            if let Some(view) = vh.upgrade() {
                let _ = cx.update_entity(&view, |root_view, root_cx| {
                    root_view.dispatch_action(action, root_cx);
                });
            }
        })
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

/// 鸟眼角标：眼框四角的 L 形标记（不遮挡眼睛本体）。
///
/// 输入为归一化全图坐标，输出元素在显示图容器内绝对定位（与检测框同坐标系）。
fn eye_corner_marks(eye: photo_domain::BBox, disp_w: f32, disp_h: f32, color: Hsla) -> AnyElement {
    let l = eye.x1 * disp_w;
    let t = eye.y1 * disp_h;
    let w = (eye.x2 - eye.x1) * disp_w;
    let h = (eye.y2 - eye.y1) * disp_h;
    let thick = 2.0;
    // 臂长随框大小缩放，夹紧 [4, 14] px
    let arm = (w.min(h) * 0.35).clamp(4.0, 14.0);

    let h_arm = |x: f32, y: f32| {
        div()
            .absolute()
            .left(px(x))
            .top(px(y))
            .w(px(arm))
            .h(px(thick))
            .bg(color)
    };
    let v_arm = |x: f32, y: f32| {
        div()
            .absolute()
            .left(px(x))
            .top(px(y))
            .w(px(thick))
            .h(px(arm))
            .bg(color)
    };

    // 绝对定位必须显式 left/top：缺省时 taffy 会把它放到「静态位置」
    // （即假想的文档流位置 = 前一个 img 兄弟之后 → 整组角标渲染到图片下方）
    div()
        .absolute()
        .left(px(0.))
        .top(px(0.))
        .size_full()
        .child(h_arm(l, t))
        .child(v_arm(l, t))
        .child(h_arm(l + w - arm, t))
        .child(v_arm(l + w - thick, t))
        .child(h_arm(l, t + h - thick))
        .child(v_arm(l, t + h - arm))
        .child(h_arm(l + w - arm, t + h - thick))
        .child(v_arm(l + w - thick, t + h - arm))
        .into_any_element()
}
