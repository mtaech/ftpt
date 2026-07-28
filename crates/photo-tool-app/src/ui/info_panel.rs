use gpui::*;
use gpui::prelude::FluentBuilder;
use photo_domain::{CaptureMeta, ColorLabel, Flag};
use photo_domain::{Rating as DomainRating, Recognition, RecognitionStatus};

use crate::action::Action;
use crate::state::app::RootView;
use gpui_component::rating::Rating;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::{Sizable};

use crate::ui::controls::{clear_link, section_header, segmented_button};
use crate::ui::theme;
use gpui_component::h_flex;

/// Render the right info panel with EXIF info + rating/label/flag controls.
pub fn render_info_panel(view: &RootView, cx: &mut Context<RootView>) -> impl IntoElement {
    let focused = view.get_focused_capture();

    div()
        .flex()
        .flex_col()
        .size_full()
        .p_3()
        .gap_2()
        // ── Hero ──
        .child(render_hero(focused))
        .child(section_divider())
        // ── 识别 ──
        .child(render_recognition_section(view, cx))
        .child(section_divider())
        .child(render_exif_section(focused))
        .child(section_divider())
        .child(render_rating_section(focused, cx))
        .child(section_divider())
        .child(render_color_label_section(focused, cx))
        .child(section_divider())
        .child(render_flag_section(focused, cx))
}

/// 1px 分隔线
fn section_divider() -> impl IntoElement {
    div().h(px(1.)).w_full().bg(theme::colors().border_variant)
}

/// Hero 区：文件名（粗体截断）+ 鸟种（条件显示）+ 分辨率/文件大小（等宽强调）
fn render_hero(focused: Option<&CaptureMeta>) -> impl IntoElement {
    match focused {
        None => div()
            .text_sm()
            .text_color(theme::colors().text_placeholder)
            .child("未选择图片")
            .into_any_element(),
        Some(meta) => div()
            .flex()
            .flex_col()
            .gap_0p5()
            .child(
                h_flex()
                    .justify_between()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::colors().text)
                            .truncate()
                            .child(meta.base_name.clone()),
                    )
                    .child(
                        div()
                            .font_family(theme::MONO_FONT_FAMILY)
                            .text_xs()
                            .text_color(theme::colors().text_muted)
                            .child(format_file_size(meta.file_size)),
                    ),
            )
            // 鸟种中文名（存在时显示）
            .when_some(meta.bird_name.as_ref(), |this, name| {
                this.child(
                    div()
                        .text_base()
                        .text_color(theme::colors().text_accent)
                        .truncate()
                        .child(name.clone()),
                )
            })
            .child(
                div()
                    .font_family(theme::MONO_FONT_FAMILY)
                    .text_xl()
                    .text_color(theme::colors().text_accent)
                    .child(match (meta.image_width, meta.image_height) {
                        (Some(w), Some(h)) => format!("{} × {}", w, h),
                        _ => "\u{2014} × \u{2014}".into(),
                    }),
            )
            .into_any_element(),
    }
}

fn format_file_size(size: Option<u64>) -> String {
    match size {
        Some(s) if s < 1024 => format!("{} B", s),
        Some(s) if s < 1024 * 1024 => format!("{:.1} KB", s as f64 / 1024.0),
        Some(s) => format!("{:.1} MB", s as f64 / (1024.0 * 1024.0)),
        None => "\u{2014}".into(),
    }
}

/// 信息行：标签（左 muted 小字） 值（右等宽对齐）
fn info_row(label: &str, value: &str) -> impl IntoElement {
    let label = format!("{}:", label);
    h_flex()
        .justify_between()
        .text_xs()
        .child(
            div()
                .text_color(theme::colors().text_muted)
                .child(label),
        )
        .child(
            div()
                .font_family(theme::MONO_FONT_FAMILY)
                .text_color(theme::colors().text)
                .child(value.to_string()),
        )
}

// ── EXIF Section ─────────────────────────────────────────────────────────

fn render_exif_section(
    focused: Option<&CaptureMeta>,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .py_2()
        .child(section_header("信息"))
        .child(match focused {
            None => div()
                .text_xs()
                .text_color(theme::colors().text_muted)
                .child("无 EXIF 信息")
                .into_any_element(),
            Some(meta) => div()
                .flex()
                .flex_col()
                .gap_0p5()
                .child(info_row(
                    "相机",
                    meta.camera_make.as_deref().unwrap_or("\u{2014}"),
                ))
                .child(info_row(
                    "型号",
                    meta.camera_model.as_deref().unwrap_or("\u{2014}"),
                ))
                .child(info_row(
                    "镜头",
                    meta.lens.as_deref().unwrap_or("\u{2014}"),
                ))
                .child(info_row(
                    "焦距",
                    meta.focal_length.as_deref().unwrap_or("\u{2014}"),
                ))
                .child(info_row(
                    "光圈",
                    meta.f_number.as_deref().unwrap_or("\u{2014}"),
                ))
                .child(info_row(
                    "快门",
                    meta.exposure_time.as_deref().unwrap_or("\u{2014}"),
                ))
                .child(info_row(
                    "ISO",
                    &meta
                        .iso
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "\u{2014}".into()),
                ))
                .child(info_row(
                    "日期",
                    meta.date_taken.as_deref().unwrap_or("\u{2014}"),
                ))
                .into_any_element(),
        })
}

// ── Rating Section ───────────────────────────────────────────────────────

fn render_rating_section(
    focused: Option<&CaptureMeta>,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    if focused.is_none() {
        return div()
            .flex()
            .flex_col()
            .gap_1()
            .py_2()
            .child(section_header("评分"))
            .child(
                div()
                    .text_xs()
                    .text_color(theme::colors().text_muted)
                    .child("未选择图片"),
            )
            .into_any_element();
    }

    let current_rating = focused.map(|m| m.rating).unwrap_or(DomainRating::None);
    let rating_value = current_rating as usize;

    let vh = cx.entity().downgrade();

    div()
        .flex()
        .flex_col()
        .gap_1()
        .py_2()
        .child(section_header("评分"))
        .child(
            Rating::new("rating")
                .value(rating_value)
                .max(5)
                .on_click({
                    let vh = vh.clone();
                    move |val, _window, cx| {
                        let action = match *val {
                            1 => Action::Rate1,
                            2 => Action::Rate2,
                            3 => Action::Rate3,
                            4 => Action::Rate4,
                            5 => Action::Rate5,
                            _ => unreachable!(),
                        };
                        if let Some(entity) = vh.upgrade() {
                            cx.update_entity(&entity, |view, cx| {
                                view.dispatch_action(action, cx)
                            });
                        }
                    }
                }),
        )
        .child(
            h_flex()
                .justify_end()
                .child(clear_link("clear-rating", "清除评分", {
                    let vh = vh.clone();
                    move |_, _w, cx| {
                        if let Some(entity) = vh.upgrade() {
                            cx.update_entity(&entity, |view, cx| {
                                view.dispatch_action(Action::Rate0, cx)
                            });
                        }
                    }
                })),
        )
        .into_any_element()
}

// ── Color Label Section ──────────────────────────────────────────────────

fn render_color_label_section(
    focused: Option<&CaptureMeta>,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    if focused.is_none() {
        return div()
            .flex()
            .flex_col()
            .gap_1()
            .py_2()
            .child(section_header("颜色标签"))
            .child(
                div()
                    .text_xs()
                    .text_color(theme::colors().text_muted)
                    .child("未选择图片"),
            )
            .into_any_element();
    }

    let current_label = focused.map(|m| m.color_label).unwrap_or(ColorLabel::None);
    let vh = cx.entity().downgrade();

    fn label_dot(
        color: Hsla,
        action: Action,
        id: &str,
        is_selected: bool,
        cx: &mut Context<RootView>,
    ) -> impl IntoElement {
        let size = if is_selected { px(28.) } else { px(22.) };
        let border_color = if is_selected {
            theme::colors().text_accent
        } else {
            theme::colors().border_variant
        };
        let border_w = if is_selected { px(3.) } else { px(1.) };
        div()
            .id(ElementId::Name(id.into()))
            .w(size)
            .h(size)
            .rounded_full()
            .bg(color)
            .border(border_w)
            .border_color(border_color)
            .cursor_pointer()
            .hover(|style| style.opacity(0.8))
            .on_click(cx.listener(move |view, _event: &ClickEvent, _window, cx| {
                view.dispatch_action(action, cx);
            }))
    }

    div()
        .flex()
        .flex_col()
        .gap_1()
        .py_2()
        .child(section_header("颜色标签"))
        .child(
            h_flex()
                .justify_between()
                .child(label_dot(*theme::colors::LABEL_RED, Action::LabelRed, "color-label-red", current_label == ColorLabel::Red, cx))
                .child(label_dot(*theme::colors::LABEL_YELLOW, Action::LabelYellow, "color-label-yellow", current_label == ColorLabel::Yellow, cx))
                .child(label_dot(*theme::colors::LABEL_GREEN, Action::LabelGreen, "color-label-green", current_label == ColorLabel::Green, cx))
                .child(label_dot(*theme::colors::LABEL_BLUE, Action::LabelBlue, "color-label-blue", current_label == ColorLabel::Blue, cx))
                .child(label_dot(*theme::colors::LABEL_PURPLE, Action::LabelPurple, "color-label-purple", current_label == ColorLabel::Purple, cx)),
        )
        .child(
            h_flex()
                .justify_end()
                .child(clear_link("clear-label", "清除标签", {
                    let vh = vh.clone();
                    move |_, _w, cx| {
                        if let Some(entity) = vh.upgrade() {
                            cx.update_entity(&entity, |view, cx| {
                                view.dispatch_action(Action::LabelNone, cx)
                            });
                        }
                    }
                })),
        )
        .into_any_element()
}

// ── Flag Section ─────────────────────────────────────────────────────────

fn render_flag_section(
    focused: Option<&CaptureMeta>,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    if focused.is_none() {
        return div()
            .flex()
            .flex_col()
            .gap_1()
            .py_2()
            .child(section_header("旗标"))
            .child(
                div()
                    .text_xs()
                    .text_color(theme::colors().text_muted)
                    .child("未选择图片"),
            )
            .into_any_element();
    }

    let current_flag: Option<Flag> = focused.map(|m| m.flag).unwrap_or(None);
    let vh = cx.entity().downgrade();

    div()
        .flex()
        .flex_col()
        .gap_1()
        .py_2()
        .child(section_header("旗标"))
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
        .into_any_element()
}

// ── Recognition Section ─────────────────────────────────────────────────

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

/// 2px 细进度条：条色按阈值动态，底为 element_background
fn confidence_bar(confidence: f32) -> impl IntoElement {
    let color = confidence_color(confidence);
    let pct = confidence.clamp(0.0, 100.0) / 100.0;
    div()
        .h(px(2.))
        .w_full()
        .bg(theme::colors().element_background)
        .rounded_sm()
        .child(
            div()
                .h_full()
                .w(relative(pct))
                .bg(color)
                .rounded_sm(),
        )
}

/// 三层派生语义 chip：圆角 4px，1px 描边 + 文字 + 底色
fn status_chip(label: &str, text_color: Hsla, bg_color: Hsla, border_color: Hsla) -> impl IntoElement {
    div()
        .px_2()
        .py_0p5()
        .rounded_sm()
        .text_xs()
        .text_color(text_color)
        .bg(bg_color)
        .border_1()
        .border_color(border_color)
        .child(label.to_string())
}

/// 识别 section：五态切换 + 切换检测框按钮
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

    div()
        .flex()
        .flex_col()
        .gap_1()
        .py_2()
        .child(section_header("识别"))
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
                            .text_sm()
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
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .text_color(colors.text_muted)
                            .child("尚未识别"),
                    )
                    .child(
                        Button::new("recognize-photo")
                            .primary()
                            .small()
                            .label("识别此照片 (b)")
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
                .gap_1()
                // 置信度数字 + 进度条
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_0p5()
                        .child(
                            div()
                                .font_family(theme::MONO_FONT_FAMILY)
                                .text_lg()
                                .text_color(conf_color)
                                .child(format!("{:.1}%", confidence)),
                        )
                        .child(confidence_bar(confidence)),
                )
                // 状态 chip「已识别」
                .child(
                    h_flex()
                        .justify_between()
                        .child(status_chip(
                            "已识别",
                            colors.success,
                            colors.success_background,
                            colors.success_border,
                        )),
                )
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
                .gap_1()
                .child(status_chip(
                    "待复核",
                    colors.warning,
                    colors.warning_background,
                    colors.warning_border,
                ))
                .child(
                    div()
                        .text_sm()
                        .text_color(colors.warning)
                        .child(failure_msg.to_string()),
                )
                .when_some(best_candidate, |this, (name, conf)| {
                    this.child(
                        div()
                            .text_sm()
                            .text_color(colors.text_muted)
                            .child(format!("最接近：{} {:.1}%", name, conf)),
                    )
                })
                .into_any_element()
        }
        RecognitionStatus::Unrecognized => {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(status_chip(
                    "未检测到鸟类",
                    colors.text_muted,
                    colors.element_background,
                    colors.border_variant,
                ))
                .into_any_element()
        }
    }
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

