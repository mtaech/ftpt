use gpui::*;
use photo_domain::{CaptureMeta, ColorLabel, Flag};
use photo_domain::Rating as DomainRating;

use crate::action::Action;
use crate::state::app::RootView;
use gpui_component::rating::Rating;

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

/// Hero 区：文件名（粗体截断）+ 分辨率/文件大小（等宽强调）
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
