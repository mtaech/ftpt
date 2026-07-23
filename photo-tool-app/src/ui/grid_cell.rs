use std::sync::Arc;

use photo_tool_core::domain::{CaptureMeta, ColorLabel, Flag, Rating};

use gpui_component::{Icon, IconName, Sizable};
use gpui::*;

use crate::ui::theme;

/// Render a single thumbnail cell in the grid.
pub fn render_grid_cell(
    capture: &CaptureMeta,
    _index: usize,
    is_selected: bool,
    thumbnail_bytes: Option<&Vec<u8>>,
) -> impl IntoElement {
    let border_color = if is_selected {
        theme::colors().text_accent
    } else {
        theme::colors().border_variant
    };

    let bg = if is_selected {
        theme::colors().element_hover
    } else {
        theme::colors().surface_background
    };

    let border_w = if is_selected { px(2.) } else { px(1.) };

    div()
        .flex()
        .flex_col()
        .bg(bg)
        .border_color(border_color)
        .border(border_w)
        .rounded_md()
        .overflow_hidden()
        .child(
            // Thumbnail area
            div()
                .flex()
                .flex_grow(1.0)
                .items_center()
                .justify_center()
                .bg(theme::colors().element_background)
                .child(render_thumbnail(capture, thumbnail_bytes)),
        )
        .child(
            // Info bar
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .px_1()
                .py_0p5()
                .bg(theme::colors().surface_background)
                .child(
                    // Filename + flag + color dot
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_1()
                        .child(render_flag_indicator(None))
                        .child(render_color_label_dot(ColorLabel::None))
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme::colors().text_muted)
                                .truncate()
                                .child(capture.base_name.clone()),
                        ),
                )
                .child(
                    // Rating stars
                    render_rating_stars(Rating::None),
                ),
        )
}

fn render_thumbnail(
    capture: &CaptureMeta,
    thumbnail_bytes: Option<&Vec<u8>>,
) -> impl IntoElement {
    if let Some(bytes) = thumbnail_bytes {
        let image = Image::from_bytes(ImageFormat::Jpeg, bytes.clone());
        img(Arc::new(image))
            .object_fit(ObjectFit::Cover)
            .size_full()
            .into_any_element()
    } else {
        div()
            .flex()
            .items_center()
            .justify_center()
            .size_full()
            .child(
                div()
                    .text_xs()
                    .text_color(theme::colors().text_muted)
                    .child(capture.base_name.clone()),
            )
            .into_any_element()
    }
}

fn render_rating_stars(rating: Rating) -> impl IntoElement {
    let n = match rating {
        Rating::None => 0,
        Rating::One => 1,
        Rating::Two => 2,
        Rating::Three => 3,
        Rating::Four => 4,
        Rating::Five => 5,
    };

    div()
        .flex()
        .flex_row()
        .gap_px()
        .children((0..5).map(|i| {
            let color = if i < n {
                theme::colors::RATING[i]
            } else {
                theme::colors().text_muted
            };
            Icon::new(if i < n { IconName::StarFill } else { IconName::Star })
                .xsmall()
                .text_color(color)
        }))
}

pub fn render_flag_indicator(flag: Option<Flag>) -> impl IntoElement {
    let color = match flag {
        Some(Flag::Pick) => *theme::colors::PICK,
        Some(Flag::Reject) => *theme::colors::REJECT,
        None => return div(),
    };
    div()
        .w(px(8.))
        .h(px(8.))
        .rounded_full()
        .bg(color)
}

pub fn render_color_label_dot(label: ColorLabel) -> impl IntoElement {
    let color = match label {
        ColorLabel::None => return div(),
        ColorLabel::Red => *theme::colors::LABEL_RED,
        ColorLabel::Yellow => *theme::colors::LABEL_YELLOW,
        ColorLabel::Green => *theme::colors::LABEL_GREEN,
        ColorLabel::Blue => *theme::colors::LABEL_BLUE,
        ColorLabel::Purple => *theme::colors::LABEL_PURPLE,
    };
    div()
        .w(px(8.))
        .h(px(8.))
        .rounded_full()
        .bg(color)
}
