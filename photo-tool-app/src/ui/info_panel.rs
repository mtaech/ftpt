use gpui::*;
use photo_tool_core::domain::{CaptureMeta, ColorLabel, Flag};
use photo_tool_core::domain::Rating as DomainRating;

use crate::action::Action;
use crate::state::app::RootView;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::rating::Rating;
use gpui_component::Sizable;

use gpui_component::Selectable;
use crate::ui::theme;

/// Render the right info panel with EXIF info + rating/label/flag controls.
pub fn render_info_panel(view: &RootView, cx: &mut Context<RootView>) -> impl IntoElement {
    let focused = view.get_focused_capture();

    div()
        .flex()
        .flex_col()
        .size_full()
        .p_3()
        .gap_3()
        .child(card_section(render_exif_section(focused)))
        .child(card_section(render_rating_section(focused, cx)))
        .child(card_section(render_color_label_section(focused, cx)))
        .child(card_section(render_flag_section(focused, cx)))
}

fn card_section(content: impl IntoElement) -> impl IntoElement {
    div()
        .p_3()
        .rounded_md()
        .border_1()
        .bg(theme::colors().surface_background)
        .border_color(theme::colors().border_variant)
        .child(content)
}

// ── helpers ──────────────────────────────────────────────────────────────

fn section_header(label: &str) -> Div {
    let label = label.to_string();
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_xs()
                .text_color(theme::colors().text_muted)
                .child(label),
        )

        .child(
            div()
                .h(px(1.))
                .bg(theme::colors().border_variant),
        )
}

fn info_row(label: &str, value: &str) -> impl IntoElement {
    let label = format!("{}:", label);
    let value = value.to_string();
    div()
        .flex()
        .flex_row()
        .gap_1()
        .text_xs()
        .child(
            div()
                .text_color(theme::colors().text_muted)
                .child(label),
        )
        .child(div().text_color(theme::colors().text).child(value))
}

fn format_file_size(size: Option<u64>) -> String {
    match size {
        Some(s) if s < 1024 => format!("{} B", s),
        Some(s) if s < 1024 * 1024 => format!("{:.1} KB", s as f64 / 1024.0),
        Some(s) => format!("{:.1} MB", s as f64 / (1024.0 * 1024.0)),
        None => "\u{2014}".into(),
    }
}

// ── EXIF Section ─────────────────────────────────────────────────────────

fn render_exif_section(
    focused: Option<&CaptureMeta>,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(section_header("信息"))
        .child(match focused {
            None => div()
                .text_color(theme::colors().text_muted)
                .child("未选择图片")
                .into_any_element(),
            Some(meta) => div()
                .flex()
                .flex_col()
                .gap_0p5()
                .child(info_row("文件名", &meta.base_name))
                .child(info_row(
                    "尺寸",
                    &match (meta.image_width, meta.image_height) {
                        (Some(w), Some(h)) => format!("{} x {}", w, h),
                        _ => "\u{2014}".into(),
                    },
                ))
                .child(info_row(
                    "文件大小",
                    &format_file_size(meta.file_size),
                ))
                .into_any_element(),
        })
}
fn render_rating_section(
    focused: Option<&CaptureMeta>,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    let current_rating = DomainRating::None; // TODO: read from XMP metadata
    let rating_value = current_rating as usize;

    let vh = cx.entity().downgrade();

    div()
        .flex()
        .flex_col()
        .gap_2()
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
                        if let Some(entity) = vh.upgrade() { cx.update_entity(&entity, |view, cx| view.dispatch_action(action, cx)); }
                    }
                }),
        )
        .child(
            Button::new("clear-rating")
                .ghost()
                .label("清除评分")
                .on_click({
                    let vh = vh.clone();
                    move |_, w, cx| {
                        if let Some(entity) = vh.upgrade() { cx.update_entity(&entity, |view, cx| view.dispatch_action(Action::Rate0, cx)); }
                    }
                }),
        )
}

// ── Color Label Section ──────────────────────────────────────────────────

fn render_color_label_section(
    focused: Option<&CaptureMeta>,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    let current_label = ColorLabel::None; // TODO: read from XMP metadata
    let vh = cx.entity().downgrade();

    fn label_dot(color: Hsla, action: Action, id: &str, is_selected: bool, cx: &mut Context<RootView>) -> impl IntoElement {
        let size = if is_selected { px(28.) } else { px(22.) };
        let border_color = if is_selected { theme::colors().text } else { theme::colors().border_variant };
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
        .gap_2()
        .child(section_header("颜色标签"))
        .child(
            div()
                .flex()
                .child(label_dot(*theme::colors::LABEL_RED, Action::LabelRed, "color-label-red", current_label == photo_tool_core::domain::ColorLabel::Red, cx))
                .child(label_dot(*theme::colors::LABEL_YELLOW, Action::LabelYellow, "color-label-yellow", current_label == photo_tool_core::domain::ColorLabel::Yellow, cx))
                .child(label_dot(*theme::colors::LABEL_GREEN, Action::LabelGreen, "color-label-green", current_label == photo_tool_core::domain::ColorLabel::Green, cx))
                .child(label_dot(*theme::colors::LABEL_BLUE, Action::LabelBlue, "color-label-blue", current_label == photo_tool_core::domain::ColorLabel::Blue, cx))
                .child(label_dot(*theme::colors::LABEL_PURPLE, Action::LabelPurple, "color-label-purple", current_label == photo_tool_core::domain::ColorLabel::Purple, cx)),
        )
        .child(
            Button::new("clear-label")
                .ghost()
                .label("清除标签")
                .on_click({
                    let vh = vh.clone();
                    move |_, w, cx| {
                        if let Some(entity) = vh.upgrade() { cx.update_entity(&entity, |view, cx| view.dispatch_action(Action::LabelNone, cx)); }
                    }
                }),
        )
}

// ── Flag Section ─────────────────────────────────────────────────────────

fn render_flag_section(
    _focused: Option<&CaptureMeta>,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    let current_flag: Option<Flag> = None; // TODO: read from XMP metadata
    let vh = cx.entity().downgrade();

    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(section_header("旗标"))
        .child(
            div()
                .flex()
                .flex_row()
                .gap_2()
                .child(
                    Button::new("flag-pick")
                        .ghost()
                        .selected(current_flag == Some(Flag::Pick))
                        .label("入选")
                        .on_click({
                            let vh = vh.clone();
                            move |_, _window, cx| {
                                if let Some(entity) = vh.upgrade() { cx.update_entity(&entity, |view, cx| view.dispatch_action(Action::FlagPick, cx)); }
                            }
                        }),
                )
                .child(
                    Button::new("flag-reject")
                        .ghost()
                        .selected(current_flag == Some(Flag::Reject))
                        .label("淘汰")
                        .on_click({
                            let vh = vh.clone();
                            move |_, _window, cx| {
                                if let Some(entity) = vh.upgrade() { cx.update_entity(&entity, |view, cx| view.dispatch_action(Action::FlagReject, cx)); }
                            }
                        }),
                )
                .child(
                    Button::new("flag-none")
                        .ghost()
                        .selected(current_flag.is_none())
                        .label("无")
                        .on_click({
                            let vh = vh.clone();
                            move |_, _window, cx| {
                                if let Some(entity) = vh.upgrade() { cx.update_entity(&entity, |view, cx| view.dispatch_action(Action::FlagNone, cx)); }
                            }
                        }),
                ),
        )
}
