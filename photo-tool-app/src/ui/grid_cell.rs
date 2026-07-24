use std::sync::Arc;

use photo_tool_core::domain::{CaptureMeta, ColorLabel, Flag, Rating};

use gpui_component::{Icon, IconName, Sizable};
use gpui::prelude::FluentBuilder;
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
        .size_full()
        .bg(bg)
        .border_color(border_color)
        .border(border_w)
        .rounded_md()
        .overflow_hidden()
        .child(
            // Thumbnail area（填满卡片剩余空间）
            div()
                .flex()
                .flex_grow(1.0)
                .min_h(px(0.))
                .items_center()
                .justify_center()
                .overflow_hidden()
                .bg(theme::colors().element_background)
                .child(render_thumbnail(capture, thumbnail_bytes)),
        )
        .child(
            // 底部信息区（两行）：文件名 / 大小 + 星级
            div()
                .flex()
                .flex_col()
                .flex_shrink_0()
                .gap_1()
                .px_2()
                .py_1p5()
                .bg(theme::colors().surface_background)
                .child(
                    // 第一行：旗标 + 颜色点 + 文件名
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_1()
                        .when(capture.flag.is_some(), |d| {
                            d.child(render_flag_indicator(capture.flag))
                        })
                        .when(capture.color_label != ColorLabel::None, |d| {
                            d.child(render_color_label_dot(capture.color_label))
                        })
                        .child(
                            div()
                                .text_sm()
                                .text_color(theme::colors().text)
                                .truncate()
                                .child(capture.base_name.clone()),
                        ),
                )
                .child(
                    // 第二行：文件大小 + 星级
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme::colors().text_muted)
                                .child(format_file_size(capture.file_size)),
                        )
                        .child(render_rating_stars(capture.rating)),
                ),
        )
}

/// 文件大小格式化为 KB/MB 可读字符串
fn format_file_size(size: Option<u64>) -> String {
    match size {
        Some(bytes) if bytes >= 1_048_576 => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
        Some(bytes) if bytes >= 1024 => format!("{:.0} KB", bytes as f64 / 1024.0),
        Some(bytes) => format!("{bytes} B"),
        None => "\u{2014}".into(),
    }
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
