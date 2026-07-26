use std::sync::Arc;

use photo_domain::{CaptureMeta, ColorLabel, Flag, ImageFormat, Rating};

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
        hsla(0., 0., 0., 0.) // 透明占位，避免选中/未选中之间布局跳动
    };

    let bg = if is_selected {
        theme::colors().element_hover
    } else {
        theme::colors().surface_background
    };

    div()
        .flex()
        .flex_col()
        .size_full()
        .bg(bg)
        .border_color(border_color)
        .border_2()
        .rounded_md()
        .overflow_hidden()
        .child(
            // Thumbnail 区域（填满卡片剩余空间，作为徽标/旗标定位容器）
            div()
                .relative()
                .flex()
                .flex_grow(1.0)
                .min_h(px(0.))
                .items_center()
                .justify_center()
                .overflow_hidden()
                .bg(theme::colors().element_background)
                .child(render_thumbnail(capture, thumbnail_bytes))
                .child(render_format_badge(capture))
                .child(render_flag_overlay(capture.flag)),
        )
        .child(
            // 底部信息区：文件名 + 大小 / 星级
            div()
                .flex()
                .flex_col()
                .flex_shrink_0()
                .gap_1()
                .px_2()
                .py_1p5()
                .bg(theme::colors().surface_background)
                .child(
                    // 第一行：文件名
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_1()
                        .child(
                            div()
                                .text_sm()
                                .text_color(theme::colors().text)
                                .truncate()
                                .child(format!("{}.{}", capture.base_name, capture.primary_format.to_lowercase())),
                        ),
                )
                .child(
                    // 第二行：文件大小（等宽）+ 星级
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_xs()
                                .font_family(theme::MONO_FONT_FAMILY)
                                .text_color(theme::colors().text_muted)
                                .child(format_file_size(capture.file_size)),
                        )
                        .child(render_rating_stars(capture.rating)),
                ),
        )
        .when(capture.color_label != ColorLabel::None, |d| {
            d.child(render_color_label_bar(capture.color_label))
        })
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
        let image = Image::from_bytes(gpui::ImageFormat::Jpeg, bytes.clone());
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
                    .child(format!("{}.{}", capture.base_name, capture.primary_format.to_lowercase())),
            )
            .into_any_element()
    }
}

/// 缩略图左上角格式徽标：RAW / JPG / RAW+JPG
fn render_format_badge(capture: &CaptureMeta) -> impl IntoElement {
    let text = derive_format_label(&capture.extensions);
    if text.is_empty() {
        return div().into_any_element();
    }
    div()
        .absolute()
        .top(px(4.))
        .left(px(4.))
        .px_1()
        .rounded_sm()
        .bg(*theme::colors::BADGE_BG)
        .child(
            div()
                .text_xs()
                .text_color(hsla(0., 0., 1., 1.))
                .child(text),
        )
        .into_any_element()
}

/// 从扩展名列表推导格式徽标文本
fn derive_format_label(extensions: &[String]) -> &'static str {
    let has_raw = extensions
        .iter()
        .any(|e| ImageFormat::is_raw_extension(e.to_lowercase().as_str()));
    let has_jpeg = extensions
        .iter()
        .any(|e| matches!(e.to_lowercase().as_str(), "jpg" | "jpeg"));

    match (has_raw, has_jpeg) {
        (true, true) => "RAW+JPG",
        (true, false) => "RAW",
        (false, true) => "JPG",
        (false, false) => "",
    }
}

/// 缩略图右上角旗标覆层：Pick → 绿底白勾，Reject → 红底白叉
fn render_flag_overlay(flag: Option<Flag>) -> impl IntoElement {
    let (bg_color, icon_name) = match flag {
        Some(Flag::Pick) => (*theme::colors::PICK, IconName::Check),
        Some(Flag::Reject) => (*theme::colors::REJECT, IconName::Close),
        None => return div().into_any_element(),
    };

    div()
        .absolute()
        .top(px(4.))
        .right(px(4.))
        .w(px(18.))
        .h(px(18.))
        .rounded_full()
        .bg(bg_color)
        .flex()
        .items_center()
        .justify_center()
        .child(
            Icon::new(icon_name)
                .xsmall()
                .text_color(hsla(0., 0., 1., 1.)),
        )
        .into_any_element()
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

/// cell 底部 3px 全宽色条
fn render_color_label_bar(label: ColorLabel) -> impl IntoElement {
    let color = match label {
        ColorLabel::Red => *theme::colors::LABEL_RED,
        ColorLabel::Yellow => *theme::colors::LABEL_YELLOW,
        ColorLabel::Green => *theme::colors::LABEL_GREEN,
        ColorLabel::Blue => *theme::colors::LABEL_BLUE,
        ColorLabel::Purple => *theme::colors::LABEL_PURPLE,
        _ => return div(),
    };
    div()
        .h(px(3.))
        .w_full()
        .flex_shrink_0()
        .bg(color)
}
