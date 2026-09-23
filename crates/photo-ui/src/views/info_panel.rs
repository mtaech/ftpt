//! 右栏组件：信息与调整（对应 §9.7）。
//!
//! - Info Tab（七张卡）：
//!   1. Hero：直方图与基本文件信息
//!   2. 拍摄信息：2x2 曝光四格（焦距/光圈/快门/ISO）+ 扩展信息
//!   3. 识别：状态 Chip、物种名、相似度与 top1−top2 间隔、重测
//!   4. 评分：五星点选
//!   5. 颜色标签：红黄绿蓝紫 + 无
//!   6. 旗标：入选 / 淘汰 / 无
//!   7. 关键词：Chips 列表与添加
//! - Adjustments Tab：
//!   - 曝光 / 对比度 / 饱和度 滑杆

use gpui_kit::component::{
    ActiveTheme as _, IconName, Selectable as _, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    rating::Rating as ComponentRating,
    slider::{Slider, SliderState},
    tag::Tag,
    v_flex,
};
use gpui_kit::{Context, Entity, IntoElement, Window, div, prelude::*, px};
use photo_domain::{ColorLabel, Flag, Rating, RecognitionStatus};

use crate::actions::{RecognizeSelected, ToggleBbox};
use crate::model::adjust::{AdjustField, format_exposure, format_tone};
use crate::state::AppState;
use crate::state::engine_ops::{set_color_label, set_flag, set_rating};
use crate::theme::{
    COLOR_LABEL_BLUE, COLOR_LABEL_GREEN, COLOR_LABEL_PURPLE, COLOR_LABEL_RED, COLOR_LABEL_YELLOW,
};

pub fn render_info_tab(
    _state: &AppState,
    meta: Option<&photo_domain::CaptureMeta>,
    cx: &mut Context<AppState>,
) -> impl IntoElement + use<> {
    let Some(meta) = meta else {
        return div()
            .w_full()
            .py_12()
            .flex()
            .items_center()
            .justify_center()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child("未选择图片")
            .into_any_element();
    };

    v_flex()
        .w_full()
        .gap_2()
        // ── 1. Hero：文件名、格式、尺寸（首段不带顶线，避免与 tab 下划线撞）──
        .child(
            crate::theme::section_first(cx)
                .gap_1p5()
                .child(
                    div()
                        .id("info-file-name")
                        .font_semibold()
                        .text_xs()
                        .text_color(cx.theme().foreground)
                        .truncate()
                        .tooltip({
                            let full = meta.display_name();
                            move |window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(full.clone())
                                    .build(window, cx)
                            }
                        })
                        .child(meta.display_name()),
                )
                .child(
                    h_flex()
                        .items_center()
                        .justify_between()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(div().child(
                            if let (Some(w), Some(h)) = (meta.image_width, meta.image_height) {
                                format!("{w} × {h}")
                            } else {
                                "未知分辨率".to_string()
                            },
                        ))
                        .child(div().child(if let Some(sz) = meta.file_size {
                            format!("{:.1} MB", sz as f64 / 1_048_576.0)
                        } else {
                            "-".to_string()
                        })),
                ),
        )
        // ── 2. 拍摄信息：2x2 曝光四格卡片 ──
        .child(
            crate::theme::section(cx)
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .font_medium()
                        .text_color(cx.theme().muted_foreground)
                        .child("曝光参数"),
                )
                .child(
                    v_flex()
                        .w_full()
                        .gap_2()
                        .child(
                            h_flex()
                                .w_full()
                                .gap_2()
                                .child(exposure_card(
                                    "焦距",
                                    meta.focal_length.clone().unwrap_or_else(|| "-".to_string()),
                                    cx,
                                ))
                                .child(exposure_card(
                                    "光圈",
                                    if let Some(ref f) = meta.f_number {
                                        if f.starts_with('f') || f.starts_with('F') {
                                            f.clone()
                                        } else {
                                            format!("f/{f}")
                                        }
                                    } else {
                                        "-".to_string()
                                    },
                                    cx,
                                )),
                        )
                        .child(
                            h_flex()
                                .w_full()
                                .gap_2()
                                .child(exposure_card(
                                    "快门",
                                    meta.exposure_time
                                        .clone()
                                        .unwrap_or_else(|| "-".to_string()),
                                    cx,
                                ))
                                .child(exposure_card(
                                    "ISO",
                                    if let Some(iso) = meta.iso {
                                        format!("{iso}")
                                    } else {
                                        "-".to_string()
                                    },
                                    cx,
                                )),
                        ),
                ),
        )
        // ── 3. 物种识别卡 ──
        .child(
            crate::theme::section(cx)
                .gap_2()
                .child(
                    h_flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_xs()
                                .font_medium()
                                .text_color(cx.theme().muted_foreground)
                                .child("物种识别"),
                        )
                        .child(match meta.recognition_status {
                            Some(RecognitionStatus::Confirmed) => {
                                Tag::success().small().child("已识别")
                            }
                            Some(RecognitionStatus::NeedsReview) => {
                                Tag::warning().small().child("待复核")
                            }
                            Some(RecognitionStatus::Unrecognized) => {
                                Tag::secondary().small().child("未识别")
                            }
                            None => Tag::secondary().small().child("未检测"),
                        }),
                )
                .child(
                    h_flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .font_semibold()
                                .text_sm()
                                .text_color(cx.theme().foreground)
                                .child(
                                    meta.taxon_name
                                        .clone()
                                        .unwrap_or_else(|| "无记录".to_string()),
                                ),
                        )
                        .child(if let Some(conf) = meta.taxon_confidence {
                            div()
                                .text_xs()
                                .font_medium()
                                .text_color(cx.theme().muted_foreground)
                                // 相似度 = 余弦相似度 × 100（0–100，跨物种不可比，所以不叫置信度）；
                                // 间隔 = top1−top2，两个物种贴得越近越该怀疑（见 open-questions §1）
                                .child(match meta.taxon_gap {
                                    Some(gap) => format!("相似度 {conf:.1}% · 间隔 {gap:.1}"),
                                    None => format!("相似度 {conf:.1}%"),
                                })
                        } else {
                            div().child("")
                        }),
                )
                // 多主体：主主体在标题行，这里列出其余主体
                .when(meta.subjects.len() > 1, |this| {
                    this.child(
                        div()
                            .w_full()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .font_medium()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!("另有 {} 个主体", meta.subjects.len() - 1)),
                            )
                            .children(meta.subjects.iter().skip(1).enumerate().map(|(i, s)| {
                                h_flex()
                                    .w_full()
                                    .items_center()
                                    .justify_between()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!("{}. {}", i + 2, s.display_name))
                                    .child(
                                        s.confidence
                                            .map(|c| format!("{c:.1}%"))
                                            .unwrap_or_else(|| "–".to_string()),
                                    )
                                    .into_any_element()
                            })),
                    )
                })
                .child(
                    h_flex()
                        .items_center()
                        .gap_1p5()
                        .pt_1()
                        .child(
                            Button::new("btn-recognize")
                                .secondary()
                                .xsmall()
                                .flex_1()
                                .justify_center()
                                .icon(IconName::Asterisk)
                                .label("重新识别")
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(RecognizeSelected), cx);
                                }),
                        )
                        .child(
                            Button::new("btn-toggle-bbox")
                                .secondary()
                                .xsmall()
                                .flex_1()
                                .justify_center()
                                .icon(IconName::Eye)
                                .label("检测框")
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(ToggleBbox), cx);
                                }),
                        ),
                ),
        )
        // ── 4. 评分卡 ──
        .child({
            let cur_val = match meta.rating {
                Rating::None => 0,
                Rating::One => 1,
                Rating::Two => 2,
                Rating::Three => 3,
                Rating::Four => 4,
                Rating::Five => 5,
            };

            crate::theme::section(cx)
                .gap_2()
                .child(
                    h_flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_xs()
                                .font_medium()
                                .text_color(cx.theme().muted_foreground)
                                .child("照片评分"),
                        )
                        .child(
                            Button::new("clear-rating")
                                .ghost()
                                .xsmall()
                                .label("清除")
                                .on_click(cx.listener(|state, _, _, cx| {
                                    set_rating(state, Rating::None);
                                    cx.notify();
                                })),
                        ),
                )
                .child(
                    ComponentRating::new("info-panel-rating")
                        .small()
                        .value(cur_val)
                        .max(5)
                        .on_click(cx.listener(|state, &new_val, _window, cx| {
                            let target = match new_val {
                                1 => Rating::One,
                                2 => Rating::Two,
                                3 => Rating::Three,
                                4 => Rating::Four,
                                5 => Rating::Five,
                                _ => Rating::None,
                            };
                            set_rating(state, target);
                            cx.notify();
                        })),
                )
        })
        // ── 5. 色标卡 ──
        .child(
            crate::theme::section(cx)
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .font_medium()
                        .text_color(cx.theme().muted_foreground)
                        .child("颜色标签"),
                )
                .child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .child(render_color_dot(
                            ColorLabel::Red,
                            COLOR_LABEL_RED,
                            meta.color_label,
                            cx,
                        ))
                        .child(render_color_dot(
                            ColorLabel::Yellow,
                            COLOR_LABEL_YELLOW,
                            meta.color_label,
                            cx,
                        ))
                        .child(render_color_dot(
                            ColorLabel::Green,
                            COLOR_LABEL_GREEN,
                            meta.color_label,
                            cx,
                        ))
                        .child(render_color_dot(
                            ColorLabel::Blue,
                            COLOR_LABEL_BLUE,
                            meta.color_label,
                            cx,
                        ))
                        .child(render_color_dot(
                            ColorLabel::Purple,
                            COLOR_LABEL_PURPLE,
                            meta.color_label,
                            cx,
                        ))
                        .child(render_color_dot(
                            ColorLabel::None,
                            cx.theme().popover,
                            meta.color_label,
                            cx,
                        )),
                ),
        )
        // ── 6. 旗标卡 ──
        .child(
            crate::theme::section(cx)
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .font_medium()
                        .text_color(cx.theme().muted_foreground)
                        .child("旗标标记"),
                )
                .child(
                    h_flex()
                        .w_full()
                        .p(px(2.))
                        .gap(px(2.))
                        .rounded_full()
                        .bg(cx.theme().muted.opacity(0.6))
                        .border_1()
                        .border_color(cx.theme().border.opacity(0.4))
                        .child(
                            Button::new("flag-pick")
                                .flex_1()
                                .xsmall()
                                .rounded_full()
                                .justify_center()
                                .icon(IconName::Check)
                                .label("入选")
                                .selected(meta.flag == Some(Flag::Pick))
                                .on_click(cx.listener(|state, _, _, cx| {
                                    set_flag(state, Some(Flag::Pick));
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("flag-reject")
                                .flex_1()
                                .xsmall()
                                .rounded_full()
                                .justify_center()
                                .icon(IconName::Close)
                                .label("淘汰")
                                .selected(meta.flag == Some(Flag::Reject))
                                .on_click(cx.listener(|state, _, _, cx| {
                                    set_flag(state, Some(Flag::Reject));
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("flag-clear")
                                .flex_1()
                                .xsmall()
                                .rounded_full()
                                .justify_center()
                                .label("无")
                                .selected(meta.flag.is_none())
                                .on_click(cx.listener(|state, _, _, cx| {
                                    set_flag(state, None);
                                    cx.notify();
                                })),
                        ),
                ),
        )
        .into_any_element()
}

/// 曝光参数卡片：Material You 层次卡片（10px 圆角 + 柔和容器背景）
fn exposure_card(
    label: &'static str,
    value: String,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    v_flex()
        .flex_1()
        .px_2p5()
        .py_2()
        .gap_0p5()
        .rounded(px(10.))
        .bg(cx.theme().muted.opacity(0.6))
        .border_1()
        .border_color(cx.theme().border.opacity(0.4))
        .child(
            div()
                .text_size(px(10.))
                .font_medium()
                .text_color(cx.theme().muted_foreground)
                .child(label),
        )
        .child(
            div()
                .font_family(cx.theme().mono_font_family.clone())
                .font_semibold()
                .text_size(px(12.))
                .text_color(cx.theme().foreground)
                .truncate()
                .child(value),
        )
}

fn render_color_dot(
    label: ColorLabel,
    color: impl Into<gpui_kit::Hsla>,
    current: ColorLabel,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let is_selected = label == current;
    let color_hsla: gpui_kit::Hsla = color.into();

    div()
        .id(format!("color-dot-{:?}", label))
        .w(px(22.))
        .h(px(22.))
        .rounded_full()
        .cursor_pointer()
        .bg(color_hsla)
        .border_2()
        .border_color(if is_selected {
            cx.theme().primary
        } else {
            cx.theme().border.opacity(0.5)
        })
        .when(is_selected, |this| this.shadow(crate::theme::panel_card_shadow(cx)))
        .on_click(cx.listener(move |state, _, _, cx| {
            let target = if is_selected { ColorLabel::None } else { label };
            set_color_label(state, target);
            cx.notify();
        }))
}

/// 调整 tab（ADR 0007）：曝光 / 对比度 / 饱和度三条滑杆。
///
/// 每行 = 标签 + 数值 chip + 单项重置，滑杆独占下一行整宽（§9.7：单行布局在 200px 宽下不可用）。
/// 拖动只改内存参数——落盘（350ms 去抖）与预览重算都由 AppState 发起，主线程零像素工作。
pub fn render_adjustments_tab(
    state: &mut AppState,
    meta: Option<&photo_domain::CaptureMeta>,
    window: &mut Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement + use<> {
    let Some(meta) = meta else {
        return div()
            .w_full()
            .py_12()
            .flex()
            .items_center()
            .justify_center()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child("未选择图片")
            .into_any_element();
    };

    // 面板可见即可靠地装载（与预览渲染共用同一入口，切图不会漏 flush/load）
    state.ensure_adjustments_loaded(&meta.primary_path, window, cx);
    state.ensure_adjust_sliders(window, cx);

    let params = state.adjust;
    let (exposure_slider, contrast_slider, saturation_slider) = {
        let sliders = state
            .adjust_sliders
            .as_ref()
            .expect("ensure_adjust_sliders 刚执行过");
        (
            sliders.exposure.clone(),
            sliders.contrast.clone(),
            sliders.saturation.clone(),
        )
    };
    let is_adjusted = !params.is_neutral();

    v_flex()
        .w_full()
        .gap_4()
        .child(
            h_flex()
                .w_full()
                .items_center()
                .justify_between()
                .child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .text_xs()
                                .font_medium()
                                .text_color(cx.theme().muted_foreground)
                                .child("调整参数"),
                        )
                        .when(is_adjusted, |this| {
                            this.child(
                                div()
                                    .px_1p5()
                                    .py_0p5()
                                    .rounded(px(6.))
                                    .bg(cx.theme().primary.opacity(0.15))
                                    .text_size(px(10.))
                                    .text_color(cx.theme().primary)
                                    .child("已调整"),
                            )
                        }),
                )
                .when(is_adjusted, |this| {
                    this.child(
                        Button::new("adjust-reset-all")
                            .ghost()
                            .xsmall()
                            .label("全部重置")
                            .on_click(cx.listener(|state, _, window, cx| {
                                state.reset_all_adjustments(window, cx);
                                cx.notify();
                            })),
                    )
                }),
        )
        .child(render_adjust_row(
            "曝光 (EV)",
            format_exposure(params.exposure),
            params.exposure == 0.0,
            AdjustField::Exposure,
            exposure_slider,
            cx,
        ))
        .child(render_adjust_row(
            "对比度",
            format_tone(params.contrast),
            params.contrast == 0,
            AdjustField::Contrast,
            contrast_slider,
            cx,
        ))
        .child(render_adjust_row(
            "饱和度",
            format_tone(params.saturation),
            params.saturation == 0,
            AdjustField::Saturation,
            saturation_slider,
            cx,
        ))
        .child(
            div()
                .text_size(px(10.))
                .text_color(cx.theme().muted_foreground)
                .child("参数随图片保存，原图永不被修改"),
        )
        .into_any_element()
}

/// 一行调整控件：标签 + 数值 chip + 单项重置；滑杆独占下一行整宽。
fn render_adjust_row(
    label: &'static str,
    value: String,
    neutral: bool,
    field: AdjustField,
    slider: Entity<SliderState>,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    v_flex()
        .w_full()
        .gap_2()
        .child(
            h_flex()
                .w_full()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().foreground)
                        .child(label),
                )
                .child(
                    h_flex()
                        .items_center()
                        .gap_1()
                        .child(
                            div()
                                .px_1p5()
                                .py_0p5()
                                .rounded(px(6.))
                                .bg(cx.theme().muted.opacity(0.7))
                                .font_family(cx.theme().mono_font_family.clone())
                                .text_size(px(11.))
                                .text_color(if neutral {
                                    cx.theme().muted_foreground
                                } else {
                                    cx.theme().primary
                                })
                                .child(value),
                        )
                        .child(
                            Button::new(format!("adjust-reset-{label}"))
                                .ghost()
                                .xsmall()
                                .label("重置")
                                .on_click(cx.listener(move |state, _, window, cx| {
                                    state.reset_adjust_field(field, window, cx);
                                    cx.notify();
                                })),
                        ),
                ),
        )
        .child(div().w_full().child(Slider::new(&slider).horizontal()))
}
