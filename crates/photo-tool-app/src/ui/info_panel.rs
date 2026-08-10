use gpui::*;
use gpui::prelude::FluentBuilder;
use photo_domain::{CaptureMeta, ColorLabel, Flag};
use photo_domain::{Rating as DomainRating, Recognition, RecognitionStatus};

use crate::action::Action;
use crate::state::app::RootView;
use gpui_component::rating::Rating;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::Disableable;
use gpui_component::tab::{Tab, TabBar};
use gpui_component::scroll::ScrollableElement;
use gpui_component::combobox::Combobox;
use gpui_component::ElementExt;
use gpui_component::{Sizable, IconName};

use crate::ui::controls::{clear_link, segmented_button};
use crate::ui::theme;
use crate::ui::format_file_size;
use gpui_component::{h_flex, v_flex};

/// Render the right info panel: 顶部双 tab（信息/调整）+ 卡片化滚动内容。
pub fn render_info_panel(
    view: &mut RootView,
    window: &mut Window,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    let vh = cx.entity().downgrade();

    // 手动修正鸟种下拉按 dirty 重建（创建需要 Window）
    if view.correction_open
        && (view.correction_select.is_none() || view.correction_select_dirty)
    {
        view.rebuild_correction_select(window, cx);
    }

    div()
        .flex()
        .flex_col()
        .size_full()
        // ── 面板标题栏：信息/调整 tab + 关闭按钮 ──
        .child(
            h_flex()
                .justify_between()
                .items_center()
                .px_3()
                .py_2()
                .border_b_1()
                .border_color(theme::colors().border_variant)
                .child(
                    TabBar::new("right-panel-tabs")
                        .small()
                        .selected_index(view.right_panel_tab)
                        .on_click({
                            let vh = vh.clone();
                            move |ix, _window, cx| {
                                if let Some(e) = vh.upgrade() {
                                    let _ = cx.update_entity(&e, |view, cx| {
                                        if view.right_panel_tab != *ix {
                                            view.right_panel_tab = *ix;
                                            cx.notify();
                                        }
                                    });
                                }
                            }
                        })
                        .child(Tab::from("信息"))
                        .child(Tab::from("调整")),
                )
                .child(
                    Button::new("close-right-panel")
                        .icon(IconName::PanelRightClose)
                        .ghost()
                        .small()
                        .tooltip("关闭右侧面板  Ctrl+]")
                        .on_click(move |_, _window, cx| {
                            if let Some(e) = vh.upgrade() {
                                let _ = cx.update_entity(&e, |view, cx| {
                                    view.dispatch_action(Action::ToggleRightPanel, cx);
                                });
                            }
                        }),
                ),
        )
        // ── tab 内容（卡片流，可滚动）──
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .p_3()
                .gap_3()
                .child(if view.right_panel_tab == 0 {
                    // 信息 tab：hero/识别/EXIF/评分/色标/旗标
                    div()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .child(render_hero(view))
                        .child(render_recognition_section(view, cx))
                        .child(render_exif_section(view.get_focused_capture()))
                        .child(render_rating_section(view.get_focused_capture(), cx))
                        .child(render_color_label_section(view.get_focused_capture(), cx))
                        .child(render_flag_section(view.get_focused_capture(), cx))
                        .into_any_element()
                } else {
                    // 调整 tab：曝光/对比度/饱和度 slider + 重置/导出
                    render_adjust_panel(view, cx).into_any_element()
                }),
        )
}

// ── 卡片容器与标题 ────────────────────────────────────────────────────────

/// 面板卡片：element_background 底 + 细边框 + 圆角 + 统一内边距
fn panel_card() -> Div {
    theme::card(div()).p_3().flex().flex_col().gap_2()
}

/// 卡片标题行：左侧小号 muted 标题，右侧可选操作（清除链接等）
fn card_title_row(label: &str, trailing: Option<AnyElement>) -> Div {
    let row = h_flex()
        .justify_between()
        .items_center()
        .child(
            div()
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::colors().text_muted)
                .child(label.to_string()),
        );
    match trailing {
        Some(el) => row.child(el),
        None => row,
    }
}

/// 无焦点图时的卡片占位提示
fn empty_hint(text: &str) -> Div {
    div()
        .text_size(px(12.))
        .text_color(theme::colors().text_placeholder)
        .child(text.to_string())
}

// ── 调整 Tab ─────────────────────────────────────────────────────────────

/// 调整 tab：曝光/对比度/饱和度 slider + 重置全部/导出（ADR 0007 参数化非破坏）。
/// slider 状态实体常驻 RootView（不随渲染重建）；拖动回调走 set_adjustment_live（内存更新 + 重算，
/// 持久化去抖），重置走 set_adjustment（立即持久化）；渲染路径不产生任何像素级工作。
fn render_adjust_panel(view: &RootView, cx: &mut Context<RootView>) -> impl IntoElement {
    let colors = theme::colors();
    let vh = cx.entity().downgrade();
    let params = view.current_adjust;

    // 无焦点图：仅提示
    if view.get_focused_capture().is_none() {
        return panel_card()
            .child(card_title_row("基础调整", None))
            .child(empty_hint("未选择图片"));
    }

    let neutral = params.is_neutral();

    panel_card()
        .child(card_title_row(
            "基础调整",
            (!neutral).then(|| {
                clear_link("reset-all-adjust", "重置全部", {
                    let vh = vh.clone();
                    move |_, _w, cx| {
                        if let Some(e) = vh.upgrade() {
                            cx.update_entity(&e, |view, cx| {
                                view.set_adjustment(photo_domain::AdjustParams::default(), cx);
                            });
                        }
                    }
                })
                .into_any_element()
            }),
        ))
        // 曝光（EV ±2.0，步进 0.05）；自绘 slider（on_mouse_* 驱动，见 simple_slider）；每项独立重置
        .child(adjust_slider_row(
            "曝光",
            format!("{:+.2} EV", params.exposure),
            params.exposure != 0.0,
            simple_slider(0, SliderTarget::Exposure, params.exposure, -2.0, 2.0, 0.05, &vh),
            reset_adjust_button("reset-exposure", params.exposure != 0.0, &vh, |view, cx| {
                let mut p = view.current_adjust;
                p.exposure = 0.0;
                view.set_adjustment(p, cx);
            }),
        ))
        // 对比度（±100）
        .child(adjust_slider_row(
            "对比度",
            format!("{:+}", params.contrast),
            params.contrast != 0,
            simple_slider(1, SliderTarget::Contrast, params.contrast as f32, -100.0, 100.0, 1.0, &vh),
            reset_adjust_button("reset-contrast", params.contrast != 0, &vh, |view, cx| {
                let mut p = view.current_adjust;
                p.contrast = 0;
                view.set_adjustment(p, cx);
            }),
        ))
        // 饱和度（±100）
        .child(adjust_slider_row(
            "饱和度",
            format!("{:+}", params.saturation),
            params.saturation != 0,
            simple_slider(2, SliderTarget::Saturation, params.saturation as f32, -100.0, 100.0, 1.0, &vh),
            reset_adjust_button("reset-saturation", params.saturation != 0, &vh, |view, cx| {
                let mut p = view.current_adjust;
                p.saturation = 0;
                view.set_adjustment(p, cx);
            }),
        ))
        // ── 导出行 ──
        .child(
            h_flex()
                .justify_end()
                .pt_1()
                .child(
                    Button::new("adjust-export")
                        .primary()
                        .small()
                        .label(if view.adjust_exporting {
                            "导出中…"
                        } else {
                            "导出…"
                        })
                        .disabled(view.adjust_exporting)
                        .on_click({
                            let vh = vh.clone();
                            move |_, _window, cx| {
                                if let Some(e) = vh.upgrade() {
                                    let _ = cx.update_entity(&e, |view, cx| {
                                        view.export_adjusted(cx);
                                    });
                                }
                            }
                        }),
                ),
        )
        // 导出结果消息（成功/失败；取消目录选择时 msg 为 None 不显示）
        .when_some(view.adjust_export_msg.as_deref(), |this, msg| {
            let color = if msg.starts_with("已导出") {
                colors.success
            } else if msg.starts_with("导出失败") {
                colors.error
            } else {
                colors.text_muted
            };
            this.child(
                div()
                    .text_color(color)
                    .text_size(px(11.))
                    .child(msg.to_string()),
            )
        })
}

/// 调整 slider 行：左侧标签 + 中部自绘滑块 + 右侧数值 + 独立重置按钮（占位防跳动）
fn adjust_slider_row(
    label: &str,
    value_text: String,
    non_neutral: bool,
    slider: impl IntoElement,
    reset: Option<AnyElement>,
) -> impl IntoElement {
    h_flex()
        .gap_2()
        .items_center()
        .child(
            div()
                .w(px(44.))
                .flex_shrink_0()
                .text_color(theme::colors().text_muted)
                .text_size(px(12.))
                .child(label.to_string()),
        )
        .child(div().flex_1().child(slider))
        .child(
            div()
                .w(px(68.))
                .flex_shrink_0()
                .text_right()
                .font_family(theme::MONO_FONT_FAMILY)
                // 非中性时数值用 accent 强调
                .text_color(if non_neutral {
                    theme::colors().text_accent
                } else {
                    theme::colors().text
                })
                .text_size(px(12.))
                .child(value_text),
        )
        // 重置按钮固定位（无按钮时留空占位，滑块宽度不跳变）
        .child(
            div()
                .w(px(44.))
                .flex_shrink_0()
                .flex()
                .justify_end()
                .children(reset),
        )
}

/// 自绘 slider 目标字段
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SliderTarget {
    Exposure,
    Contrast,
    Saturation,
}

/// 将拖动目标值写入调整参数并触发重算（set_adjustment_live 无条件 notify，拖动实时刷新；
/// 持久化走 350ms 去抖，停止拖动后才写 DB，不逐帧 UPSERT）
fn apply_slider_value(view: &mut RootView, target: SliderTarget, value: f32, cx: &mut Context<RootView>) {
    let mut p = view.current_adjust;
    match target {
        SliderTarget::Exposure => p.exposure = value,
        SliderTarget::Contrast => p.contrast = value as i32,
        SliderTarget::Saturation => p.saturation = value as i32,
    }
    view.set_adjustment_live(p, cx);
}

/// 鼠标窗口 x → slider 值（按元素边界归一化，step 取整，夹紧范围）
fn slider_value_from_pos(x: f32, bounds: (f32, f32, f32), min: f32, max: f32, step: f32) -> f32 {
    let (left, _, width) = bounds;
    if width <= 0.0 {
        return min;
    }
    let pct = ((x - left) / width).clamp(0.0, 1.0);
    let v = min + (max - min) * pct;
    ((v / step).round() * step).clamp(min, max)
}

/// 自绘 slider：轨道 + 中性点→当前值填充 + thumb，on_mouse_down/move/up 驱动（项目验证路径，
/// 不依赖 gpui-component Slider 的 on_drag——锁定 gpui 4ebc154 下拖动实测无效）。
/// 拖动中每帧 set_adjustment_live（取消式重算 <8ms，实时；持久化 350ms 去抖，见状态层）；
/// on_prepaint 记录边界供算值。
/// 注意：GPUI `relative()` 取小数比例（1.0 = 100%），禁止传百分数（曾导致填充条溢出整行）。
fn simple_slider(
    idx: usize,
    target: SliderTarget,
    value: f32,
    min: f32,
    max: f32,
    step: f32,
    vh: &gpui::WeakEntity<RootView>,
) -> impl IntoElement {
    let colors = theme::colors();
    let pct = ((value - min) / (max - min)).clamp(0.0, 1.0);
    // 双极滑块（min<0<max）：填充从中性点（0 值位置）画到当前值，直观表达正负方向
    let zero_pct = ((0.0 - min) / (max - min)).clamp(0.0, 1.0);
    let (fill_left, fill_width) = if min < 0.0 && max > 0.0 {
        (pct.min(zero_pct), (pct - zero_pct).abs())
    } else {
        (0.0, pct)
    };
    let vh_down = vh.clone();
    let vh_move = vh.clone();
    let vh_up = vh.clone();
    let vh_prepaint = vh.clone();
    div()
        .id(ElementId::Name(format!("adjust-slider-{idx}").into()))
        .relative()
        .h(px(20.))
        .w_full()
        .cursor(CursorStyle::PointingHand)
        // 轨道（4px，比边框色更可见的 element_active）
        .child(
            div()
                .absolute()
                .left(px(0.))
                .right(px(0.))
                .top(px(8.))
                .h(px(4.))
                .rounded_full()
                .bg(colors.element_active),
        )
        // 填充（中性点 → 当前值）
        .child(
            div()
                .absolute()
                .left(relative(fill_left))
                .top(px(8.))
                .h(px(4.))
                .w(relative(fill_width))
                .rounded_full()
                .bg(colors.text_accent),
        )
        // thumb（12px 圆点 + accent 描边 + 阴影，始终可见）
        .child(
            div()
                .absolute()
                .left(relative(pct))
                .ml(-px(6.))
                .top(px(4.))
                .size(px(12.))
                .rounded_full()
                .bg(colors.elevated_surface_background)
                .border_2()
                .border_color(colors.text_accent)
                .shadow_sm(),
        )
        .on_mouse_down(
            MouseButton::Left,
            move |e: &MouseDownEvent, _window, cx| {
                if let Some(v) = vh_down.upgrade() {
                    let x: f32 = e.position.x.into();
                    let _ = cx.update_entity(&v, |view, cx| {
                        view.adjust_drag = Some(idx);
                        let bounds = view.adjust_slider_bounds[idx];
                        apply_slider_value(view, target, slider_value_from_pos(x, bounds, min, max, step), cx);
                    });
                }
            },
        )
        .on_mouse_move(move |e: &MouseMoveEvent, _window, cx| {
            if e.pressed_button != Some(MouseButton::Left) {
                return;
            }
            if let Some(v) = vh_move.upgrade() {
                let x: f32 = e.position.x.into();
                let _ = cx.update_entity(&v, |view, cx| {
                    if view.adjust_drag == Some(idx) {
                        let bounds = view.adjust_slider_bounds[idx];
                        apply_slider_value(view, target, slider_value_from_pos(x, bounds, min, max, step), cx);
                    }
                });
            }
        })
        .on_mouse_up(
            MouseButton::Left,
            move |_e: &MouseUpEvent, _window, cx| {
                if let Some(v) = vh_up.upgrade() {
                    let _ = cx.update_entity(&v, |view, _cx| {
                        if view.adjust_drag == Some(idx) {
                            view.adjust_drag = None;
                        }
                    });
                }
            },
        )
        .on_prepaint(move |bounds, _window, cx| {
            if let Some(v) = vh_prepaint.upgrade() {
                let x: f32 = bounds.origin.x.into();
                let y: f32 = bounds.origin.y.into();
                let w: f32 = bounds.size.width.into();
                let _ = cx.update_entity(&v, |view, _cx| {
                    view.adjust_slider_bounds[idx] = (x, y, w);
                });
            }
        })
}

/// 单调整项独立重置按钮：非中性时显示（文字按钮），点击将该参数归零（ADR 0007）
fn reset_adjust_button(
    id: &'static str,
    visible: bool,
    vh: &gpui::WeakEntity<RootView>,
    apply: impl Fn(&mut RootView, &mut Context<RootView>) + 'static,
) -> Option<AnyElement> {
    visible.then(|| {
        let vh = vh.clone();
        Button::new(id)
            .ghost()
            .small()
            .label("重置")
            .on_click(move |_, _window, cx| {
                if let Some(e) = vh.upgrade() {
                    let _ = cx.update_entity(&e, |view, cx| apply(view, cx));
                }
            })
            .into_any_element()
    })
}

// ── Hero 卡片 ─────────────────────────────────────────────────────────────

/// Hero 卡片：缩略图 + 文件名/格式徽标 + 鸟种（条件）+ 分辨率/文件大小
fn render_hero(view: &RootView) -> impl IntoElement {
    let colors = theme::colors();
    let focused = view.get_focused_capture();

    let Some(meta) = focused else {
        return panel_card()
            .child(
                div()
                    .h(px(140.))
                    .w_full()
                    .rounded_md()
                    .bg(colors.element_hover)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        gpui_component::Icon::new(IconName::GalleryVerticalEnd)
                            .text_color(colors.text_placeholder.opacity(0.4)),
                    ),
            )
            .child(empty_hint("未选择图片"));
    };

    // 焦点 capture 索引 → 已解码缩略图/预览图（加载由 layout.rs 渲染前触发）
    let capture_idx = view
        .focus_index
        .and_then(|di| view.display_order.get(di).copied());
    let image = capture_idx
        .and_then(|ci| view.thumbnail_data.get(&ci).cloned())
        .or_else(|| capture_idx.and_then(|ci| view.preview_data.get(&ci).cloned()));

    panel_card()
        // 缩略图（封面裁切；未就绪时占位图标）
        .child(
            div()
                .h(px(140.))
                .w_full()
                .rounded_md()
                .overflow_hidden()
                .bg(colors.element_hover)
                .flex()
                .items_center()
                .justify_center()
                .child(match image {
                    Some(img) => gpui::img(ImageSource::from(img))
                        .object_fit(ObjectFit::Cover)
                        .size_full()
                        .into_any_element(),
                    None => gpui_component::Icon::new(IconName::GalleryVerticalEnd)
                        .text_color(colors.text_placeholder.opacity(0.4))
                        .into_any_element(),
                }),
        )
        // 文件名 + 格式徽标
        .child(
            h_flex()
                .justify_between()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(13.))
                        .text_color(colors.text)
                        .truncate()
                        .child(meta.display_name()),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .px_1()
                        .rounded_sm()
                        .border_1()
                        .border_color(colors.border)
                        .text_size(px(10.))
                        .font_family(theme::MONO_FONT_FAMILY)
                        .text_color(colors.text_muted)
                        .child(meta.primary_format.to_uppercase()),
                ),
        )
        // 鸟种中文名（存在时显示）
        .when_some(meta.bird_name.as_ref(), |this, name| {
            this.child(
                div()
                    .text_size(px(13.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(colors.text_accent)
                    .truncate()
                    .child(name.clone()),
            )
        })
        // 分辨率 · 文件大小
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .font_family(theme::MONO_FONT_FAMILY)
                        .text_size(px(12.))
                        .text_color(colors.text)
                        .child(match (meta.image_width, meta.image_height) {
                            (Some(w), Some(h)) => format!("{} × {}", w, h),
                            _ => "— × —".into(),
                        }),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(colors.text_placeholder)
                        .child("·"),
                )
                .child(
                    div()
                        .font_family(theme::MONO_FONT_FAMILY)
                        .text_size(px(12.))
                        .text_color(colors.text_muted)
                        .child(format_file_size(meta.file_size)),
                ),
        )
}

// ── EXIF 卡片 ────────────────────────────────────────────────────────────

/// 拍摄参数格：小标签 + 等宽值（element_hover 底圆角格）
fn stat_cell(label: &str, value: &str) -> Div {
    let colors = theme::colors();
    div()
        .flex_1()
        .rounded_md()
        .bg(colors.element_hover)
        .px_2()
        .py_1()
        .flex()
        .flex_col()
        .gap_0p5()
        .child(
            div()
                .text_size(px(10.))
                .text_color(colors.text_muted)
                .child(label.to_string()),
        )
        .child(
            div()
                .font_family(theme::MONO_FONT_FAMILY)
                .text_size(px(13.))
                .text_color(colors.text)
                .truncate()
                .child(value.to_string()),
        )
}

/// 信息行：标签（左 muted 小字） 值（右对齐）
fn info_row(label: &str, value: &str) -> impl IntoElement {
    h_flex()
        .justify_between()
        .gap_2()
        .child(
            div()
                .flex_shrink_0()
                .text_size(px(12.))
                .text_color(theme::colors().text_muted)
                .child(label.to_string()),
        )
        .child(
            div()
                .min_w_0()
                .truncate()
                .text_size(px(12.))
                .text_color(theme::colors().text)
                .child(value.to_string()),
        )
}

fn render_exif_section(
    focused: Option<&CaptureMeta>,
) -> impl IntoElement {
    let Some(meta) = focused else {
        return panel_card()
            .child(card_title_row("拍摄信息", None))
            .child(empty_hint("无 EXIF 信息"));
    };

    // 相机行：厂商 + 型号拼接（如 "Canon EOS R5"）
    let camera = match (meta.camera_make.as_deref(), meta.camera_model.as_deref()) {
        (Some(make), Some(model)) => format!("{make} {model}"),
        (Some(make), None) => make.to_string(),
        (None, Some(model)) => model.to_string(),
        (None, None) => "—".into(),
    };
    let dash = "\u{2014}";

    panel_card()
        .child(card_title_row("拍摄信息", None))
        // 关键参数 2×2 网格
        .child(
            h_flex()
                .gap_2()
                .child(stat_cell("焦距", meta.focal_length.as_deref().unwrap_or(dash)))
                .child(stat_cell("光圈", meta.f_number.as_deref().unwrap_or(dash))),
        )
        .child(
            h_flex()
                .gap_2()
                .child(stat_cell("快门", meta.exposure_time.as_deref().unwrap_or(dash)))
                .child(stat_cell(
                    "ISO",
                    &meta.iso.map(|v| v.to_string()).unwrap_or_else(|| dash.into()),
                )),
        )
        // 次级信息行
        .child(info_row("相机", &camera))
        .child(info_row("镜头", meta.lens.as_deref().unwrap_or(dash)))
        .child(info_row("日期", meta.date_taken.as_deref().unwrap_or(dash)))
}

// ── 评分卡片 ─────────────────────────────────────────────────────────────

fn render_rating_section(
    focused: Option<&CaptureMeta>,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    let vh = cx.entity().downgrade();

    let Some(meta) = focused else {
        return panel_card()
            .child(card_title_row("评分", None))
            .child(empty_hint("未选择图片"));
    };

    let rating_value = meta.rating as usize;
    let has_rating = meta.rating != DomainRating::None;

    panel_card()
        .child(card_title_row(
            "评分",
            has_rating.then(|| {
                clear_link("clear-rating", "清除", {
                    let vh = vh.clone();
                    move |_, _w, cx| {
                        if let Some(entity) = vh.upgrade() {
                            cx.update_entity(&entity, |view, cx| {
                                view.dispatch_action(Action::Rate0, cx)
                            });
                        }
                    }
                })
                .into_any_element()
            }),
        ))
        .child(
            Rating::new("rating")
                .with_size(gpui_component::Size::Medium)
                .color(*theme::colors::LABEL_YELLOW)
                .value(rating_value)
                .max(5)
                .on_click({
                    let vh = vh.clone();
                    move |val, _window, cx| {
                        let action = match *val {
                            0 => Action::Rate0, // 点击当前已选星可递减，1 星再点即清除
                            1 => Action::Rate1,
                            2 => Action::Rate2,
                            3 => Action::Rate3,
                            4 => Action::Rate4,
                            5 => Action::Rate5,
                            // GPUI 回调中 panic 会 abort，异常值直接忽略
                            _ => return,
                        };
                        if let Some(entity) = vh.upgrade() {
                            cx.update_entity(&entity, |view, cx| {
                                view.dispatch_action(action, cx)
                            });
                        }
                    }
                }),
        )
}

// ── 颜色标签卡片 ─────────────────────────────────────────────────────────

fn render_color_label_section(
    focused: Option<&CaptureMeta>,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    let vh = cx.entity().downgrade();

    let Some(meta) = focused else {
        return panel_card()
            .child(card_title_row("颜色标签", None))
            .child(empty_hint("未选择图片"));
    };

    let current_label = meta.color_label;

    fn label_dot(
        color: Hsla,
        action: Action,
        id: &str,
        is_selected: bool,
        cx: &mut Context<RootView>,
    ) -> impl IntoElement {
        // 固定 20px + 恒 2px 描边：选中态改描边色（不改尺寸），布局零跳动
        let border_color = if is_selected {
            theme::colors().text
        } else {
            theme::colors().border_transparent
        };
        div()
            .id(ElementId::Name(id.into()))
            .size(px(20.))
            .rounded_full()
            .bg(color)
            .border_2()
            .border_color(border_color)
            .cursor_pointer()
            .hover(|style| {
                style.border_color(if is_selected {
                    theme::colors().text
                } else {
                    theme::colors().text_muted
                })
            })
            .on_click(cx.listener(move |view, _event: &ClickEvent, _window, cx| {
                view.dispatch_action(action, cx);
            }))
    }

    panel_card()
        .child(card_title_row(
            "颜色标签",
            (current_label != ColorLabel::None).then(|| {
                clear_link("clear-label", "清除", {
                    let vh = vh.clone();
                    move |_, _w, cx| {
                        if let Some(entity) = vh.upgrade() {
                            cx.update_entity(&entity, |view, cx| {
                                view.dispatch_action(Action::LabelNone, cx)
                            });
                        }
                    }
                })
                .into_any_element()
            }),
        ))
        .child(
            h_flex()
                .justify_between()
                .child(label_dot(*theme::colors::LABEL_RED, Action::LabelRed, "color-label-red", current_label == ColorLabel::Red, cx))
                .child(label_dot(*theme::colors::LABEL_YELLOW, Action::LabelYellow, "color-label-yellow", current_label == ColorLabel::Yellow, cx))
                .child(label_dot(*theme::colors::LABEL_GREEN, Action::LabelGreen, "color-label-green", current_label == ColorLabel::Green, cx))
                .child(label_dot(*theme::colors::LABEL_BLUE, Action::LabelBlue, "color-label-blue", current_label == ColorLabel::Blue, cx))
                .child(label_dot(*theme::colors::LABEL_PURPLE, Action::LabelPurple, "color-label-purple", current_label == ColorLabel::Purple, cx)),
        )
}

// ── 旗标卡片 ─────────────────────────────────────────────────────────────

fn render_flag_section(
    focused: Option<&CaptureMeta>,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    let vh = cx.entity().downgrade();

    let Some(meta) = focused else {
        return panel_card()
            .child(card_title_row("旗标", None))
            .child(empty_hint("未选择图片"));
    };

    let current_flag: Option<Flag> = meta.flag;

    panel_card()
        .child(card_title_row("旗标", None))
        .child(
            h_flex()
                .justify_between()
                .child(segmented_button(
                    "flag-pick",
                    "入选",
                    current_flag == Some(Flag::Pick),
                    {
                        let vh = vh.clone();
                        move |_, _window, cx| {
                            if let Some(entity) = vh.upgrade() {
                                cx.update_entity(&entity, |view, cx| {
                                    view.dispatch_action(Action::FlagPick, cx)
                                });
                            }
                        }
                    },
                ))
                .child(segmented_button(
                    "flag-reject",
                    "淘汰",
                    current_flag == Some(Flag::Reject),
                    {
                        let vh = vh.clone();
                        move |_, _window, cx| {
                            if let Some(entity) = vh.upgrade() {
                                cx.update_entity(&entity, |view, cx| {
                                    view.dispatch_action(Action::FlagReject, cx)
                                });
                            }
                        }
                    },
                ))
                .child(segmented_button(
                    "flag-none",
                    "无",
                    current_flag.is_none(),
                    {
                        let vh = vh.clone();
                        move |_, _window, cx| {
                            if let Some(entity) = vh.upgrade() {
                                cx.update_entity(&entity, |view, cx| {
                                    view.dispatch_action(Action::FlagNone, cx)
                                });
                            }
                        }
                    },
                )),
        )
}

// ── 识别卡片 ─────────────────────────────────────────────────────────────

/// 置信度阈值色：>=80 success / >=50 warning / <50 info
fn confidence_color(confidence: f32) -> Hsla {
    if confidence >= 80.0 {
        theme::colors().success
    } else if confidence >= 50.0 {
        theme::colors().warning
    } else {
        theme::colors().info
    }
}

/// 4px 细进度条：条色按阈值动态，底为 element_active
fn confidence_bar(confidence: f32) -> impl IntoElement {
    let color = confidence_color(confidence);
    let pct = confidence.clamp(0.0, 100.0) / 100.0;
    div()
        .h(px(4.))
        .w_full()
        .bg(theme::colors().element_active)
        .rounded_full()
        .child(
            div()
                .h_full()
                .w(relative(pct))
                .bg(color)
                .rounded_full(),
        )
}

/// 三层派生语义 chip：圆角 4px，1px 描边 + 文字 + 底色
fn status_chip(label: &str, text_color: Hsla, bg_color: Hsla, border_color: Hsla) -> impl IntoElement {
    div()
        .px_2()
        .py_0p5()
        .rounded_sm()
        .text_size(px(11.))
        .text_color(text_color)
        .bg(bg_color)
        .border_1()
        .border_color(border_color)
        .child(label.to_string())
}

/// 识别卡片：五态切换 + 重新识别/检测框按钮
fn render_recognition_section(
    view: &RootView,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    let vh = cx.entity().downgrade();
    let colors = theme::colors();

    // 计算当前聚焦 capture 的 index（用于 comparing recognizing_single）
    let focused_cap_index = view
        .focus_index
        .and_then(|di| view.display_order.get(di).copied());

    let is_busy = focused_cap_index.is_some()
        && view.recognizing_single == focused_cap_index;

    // 非图片格式（视频等）：识别按钮禁用
    let is_other = view
        .get_focused_capture()
        .is_some_and(|m| m.primary_format.to_uppercase() == "OTHER");

    let no_focus = view.get_focused_capture().is_none();

    panel_card()
        .child(card_title_row("识别", None))
        .child(match (view.focused_recognition.as_ref(), is_busy) {
            // ── 识别中 ──
            (_, true) => {
                let stage = view
                    .recognize_stage
                    .as_deref()
                    .unwrap_or("识别中...");
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(gpui_component::spinner::Spinner::new().with_size(gpui_component::Size::Small))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(colors.text_muted)
                            .child(stage.to_string()),
                    )
                    .into_any_element()
            }
            // ── 未识别（无记录） ──
            (None, false) => {
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(colors.text_muted)
                            .child(if no_focus { "未选择图片" } else { "尚未识别" }),
                    )
                    .child(
                        if is_other {
                            div()
                                .text_size(px(11.))
                                .text_color(colors.text_muted)
                                .child("非图片格式，不支持识别")
                                .into_any_element()
                        } else {
                            Button::new("recognize-photo")
                                .primary()
                                .small()
                                .label("识别此照片 (b)")
                                .disabled(no_focus)
                                .on_click({
                                    let vh = vh.clone();
                                    move |_, _window, cx| {
                                        if let Some(e) = vh.upgrade() {
                                            cx.update_entity(&e, |view, cx| {
                                                view.dispatch_action(Action::Recognize, cx);
                                            });
                                        }
                                    }
                                })
                                .into_any_element()
                        },
                    )
                    .into_any_element()
            }
            // ── 有识别记录 ──
            (Some(r), false) => render_recognition_content(r),
        })
        // ToggleBbox 按钮（有识别记录且非识别中时显示，与重新识别并排）
        .when(
            view.focused_recognition.is_some() && !is_busy,
            |this| {
                this.child(render_recognition_actions(view, &vh))
            },
        )
        // 手动修正鸟种入口（有焦点图且非识别中/非视频时显示）
        .when(
            view.get_focused_capture().is_some() && !is_busy && !is_other,
            |this| {
                this.child(
                    v_flex()
                        .gap_1()
                        .child(
                            h_flex()
                                .justify_end()
                                .child(
                                    Button::new("correct-bird-toggle")
                                        .ghost()
                                        .small()
                                        .label(if view.correction_open {
                                            "收起修正"
                                        } else {
                                            "修正鸟种…"
                                        })
                                        .on_click({
                                            let vh = vh.clone();
                                            move |_, _window, cx| {
                                                if let Some(e) = vh.upgrade() {
                                                    let _ = cx.update_entity(&e, |view, cx| {
                                                        view.correction_open = !view.correction_open;
                                                        if view.correction_open {
                                                            // 展开：加载名录并重建下拉（选中态复位）
                                                            view.ensure_correction_species(cx);
                                                            view.correction_select_dirty = true;
                                                        }
                                                        cx.notify();
                                                    });
                                                }
                                            }
                                        }),
                                ),
                        )
                        // 展开后显示搜索下拉（全量名录，选即修正）
                        .when(view.correction_open, |this| {
                            this.when_some(view.correction_select.as_ref(), |this, sel| {
                                this.child(
                                    Combobox::new(sel)
                                        .placeholder("搜索鸟种...")
                                        .search_placeholder("搜索鸟种...")
                                        .menu_width(px(240.))
                                        .small(),
                                )
                            })
                        }),
                )
            },
        )
}

/// 有识别记录时的内容渲染
fn render_recognition_content(
    r: &Recognition,
) -> AnyElement {
    let colors = theme::colors();

    match r.status {
        RecognitionStatus::Confirmed => {
            let confidence = r.confidence.unwrap_or(0.0);
            let conf_color = confidence_color(confidence);

            div()
                .flex()
                .flex_col()
                .gap_2()
                // 状态 chip + 置信度数字
                .child(
                    h_flex()
                        .justify_between()
                        .items_center()
                        .child(status_chip(
                            "已识别",
                            colors.success,
                            colors.success_background,
                            colors.success_border,
                        ))
                        .child(
                            div()
                                .font_family(theme::MONO_FONT_FAMILY)
                                .text_size(px(15.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(conf_color)
                                .child(format!("{:.1}%", confidence)),
                        ),
                )
                .child(confidence_bar(confidence))
                // 鸟眼锐度
                .when_some(r.eye_sharpness, |this, s| {
                    this.child(eye_sharpness_row(&colors, s))
                })
                .into_any_element()
        }
        RecognitionStatus::NeedsReview => {
            let failure_msg = r.failure_stage.user_message();

            // 取 candidates 中第一个 bird 非 None 项
            let best_candidate = r.candidates.iter().find_map(|c| {
                c.bird.as_ref().map(|b| (b.cn_name.clone(), c.confidence))
            });

            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(status_chip(
                    "待复核",
                    colors.warning,
                    colors.warning_background,
                    colors.warning_border,
                ))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(colors.warning)
                        .child(failure_msg.to_string()),
                )
                .when_some(best_candidate, |this, (name, conf)| {
                    this.child(
                        div()
                            .text_size(px(12.))
                            .text_color(colors.text_muted)
                            .child(format!("最接近：{} {:.1}%", name, conf)),
                    )
                })
                // 鸟眼锐度
                .when_some(r.eye_sharpness, |this, s| {
                    this.child(eye_sharpness_row(&colors, s))
                })
                .into_any_element()
        }
        RecognitionStatus::Unrecognized => {
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(status_chip(
                    "未检测到鸟类",
                    colors.text_muted,
                    colors.element_background,
                    colors.border_variant,
                ))
                .when_some(r.eye_sharpness, |this, s| {
                    this.child(eye_sharpness_row(&colors, s))
                })
                .into_any_element()
        }
    }
}

/// 鸟眼锐度行：分数 + info 图标（悬浮显示评分公式）
fn eye_sharpness_row(colors: &theme::ThemeColors, score: f32) -> AnyElement {
    h_flex()
        .gap_1()
        .items_center()
        .child(
            div()
                .text_size(px(12.))
                .text_color(colors.text_muted)
                .child(format!("眼锐度 {:.2}", score)),
        )
        .child(
            div()
                .id("eye-sharpness-help")
                .tooltip(|window, cx| {
                    gpui_component::tooltip::Tooltip::new(
                        "0.5·ln(1+拉普拉斯方差) + 0.3·ln(1+梯度幅值均值) + 0.2·ln(1+边缘密度)；仅保证单调性，越高越锐利，权重待样片标定",
                    )
                    .build(window, cx)
                })
                .child(
                    gpui_component::Icon::new(IconName::Info)
                        .small()
                        .text_color(colors.text_muted),
                ),
        )
        .into_any_element()
}

/// 识别动作按钮行：重新识别 + 切换检测框
fn render_recognition_actions(
    view: &RootView,
    vh: &gpui::WeakEntity<RootView>,
) -> impl IntoElement {
    let vh = vh.clone();
    let bbox_label = if view.bbox_visible {
        "隐藏检测框"
    } else {
        "显示检测框"
    };

    h_flex()
        .justify_end()
        .gap_1()
        .child(
            // 重新识别
            Button::new("re-recognize")
                .ghost()
                .small()
                .label("重新识别")
                .on_click({
                    let vh = vh.clone();
                    move |_, _window, cx| {
                        if let Some(e) = vh.upgrade() {
                            cx.update_entity(&e, |view, cx| {
                                view.dispatch_action(Action::Recognize, cx);
                            });
                        }
                    }
                }),
        )
        .child(
            // 显示/隐藏检测框
            Button::new("toggle-bbox")
                .ghost()
                .small()
                .label(bbox_label)
                .on_click({
                    let vh = vh.clone();
                    move |_, _window, cx| {
                        if let Some(e) = vh.upgrade() {
                            cx.update_entity(&e, |view, cx| {
                                view.dispatch_action(Action::ToggleBbox, cx);
                            });
                        }
                    }
                }),
        )
}

#[cfg(test)]
mod tests {
    // 不用 super::*：gpui 根导出的 test 属性宏会遮蔽内建 #[test] 导致宏展开递归
    use super::slider_value_from_pos;

    /// 鼠标 x → slider 值：边界端点、越界夹紧
    #[test]
    fn test_slider_value_from_pos_endpoints_and_clamp() {
        let b = (100.0, 0.0, 200.0); // left=100, width=200
        assert_eq!(slider_value_from_pos(100.0, b, -2.0, 2.0, 0.05), -2.0);
        assert_eq!(slider_value_from_pos(300.0, b, -2.0, 2.0, 0.05), 2.0);
        assert_eq!(slider_value_from_pos(50.0, b, -2.0, 2.0, 0.05), -2.0);
        assert_eq!(slider_value_from_pos(500.0, b, -2.0, 2.0, 0.05), 2.0);
    }

    /// 中点 → 0；四分之一 → ±1.0；step 取整（曝光 0.05 网格）
    #[test]
    fn test_slider_value_from_pos_midpoint_and_step() {
        let b = (100.0, 0.0, 200.0);
        assert_eq!(slider_value_from_pos(200.0, b, -2.0, 2.0, 0.05), 0.0);
        assert_eq!(slider_value_from_pos(150.0, b, -2.0, 2.0, 0.05), -1.0);
        assert_eq!(slider_value_from_pos(250.0, b, -2.0, 2.0, 0.05), 1.0);
        let v = slider_value_from_pos(201.25, b, -2.0, 2.0, 0.05);
        assert!((v * 20.0).fract().abs() < 1e-6, "值应在 0.05 网格上: {v}");
    }

    /// 对比度/饱和度步进 1，范围 ±100
    #[test]
    fn test_slider_value_from_pos_percent_range() {
        let b = (100.0, 0.0, 200.0);
        assert_eq!(slider_value_from_pos(150.0, b, -100.0, 100.0, 1.0), -50.0);
        assert_eq!(slider_value_from_pos(250.0, b, -100.0, 100.0, 1.0), 50.0);
        assert_eq!(slider_value_from_pos(200.0, b, -100.0, 100.0, 1.0), 0.0);
    }

    /// 宽度为 0（未布局）时返回 min，不除零
    #[test]
    fn test_slider_value_from_pos_zero_width_safe() {
        assert_eq!(slider_value_from_pos(0.0, (0.0, 0.0, 0.0), -2.0, 2.0, 0.05), -2.0);
    }
}
