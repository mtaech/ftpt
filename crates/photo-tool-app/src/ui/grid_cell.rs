use std::sync::Arc;

use photo_domain::{CaptureMeta, ColorLabel, Flag, ImageFormat, Rating, RecognitionStatus};

use gpui_component::{Icon, IconName, Sizable};
use gpui::prelude::FluentBuilder;
use gpui::*;

use crate::ui::theme;
use crate::ui::format_file_size;

/// Render a single thumbnail cell in the grid.
/// `use<>` 精确捕获：返回元素不借用 capture（全部构建 owned 值），
/// 使 uniform_list 闭包可直接借用状态而无需克隆。
pub fn render_grid_cell(
    capture: &CaptureMeta,
    _index: usize,
    is_selected: bool,
    thumbnail: Option<Arc<RenderImage>>,
) -> impl IntoElement + use<> {
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
                .child(render_thumbnail(capture, thumbnail))
                .child(render_format_badge(capture))
                .child(render_flag_overlay(capture.flag)),
        )
        .child(
            // 底部信息区：文件名 + 大小/星级 + 鸟种状态
            div()
                .flex()
                .flex_col()
                .flex_shrink_0()
                .gap_0p5()
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
                                
                                .font_family(theme::MONO_FONT_FAMILY)
                                .text_color(theme::colors().text_muted)
                                .child(format_file_size(capture.file_size)),
                        )
                        .child(render_rating_stars(capture.rating)),
                )
                // 第三行：鸟种状态（常驻占位，固定高度，虚拟化等高约束）
                .child(render_bird_status_line(capture)),
        )
        .when(capture.color_label != ColorLabel::None, |d| {
            d.child(render_color_label_bar(capture.color_label))
        })
}

fn render_thumbnail(
    capture: &CaptureMeta,
    thumbnail: Option<Arc<RenderImage>>,
) -> impl IntoElement + use<> {
    if let Some(image) = thumbnail {
        img(ImageSource::from(image))
            .object_fit(ObjectFit::Cover)
            .size_full()
            .into_any_element()
    } else if is_other_format(capture) {
        // 视频：无缩略图，居中显示格式徽标（MP4/MOV…）
        div()
            .flex()
            .items_center()
            .justify_center()
            .size_full()
            .bg(theme::colors().element_background)
            .child(
                div()
                    .px_2()
                    .py_0p5()
                    .rounded_md()
                    .bg(theme::colors().surface_background)
                    .border_1()
                    .border_color(theme::colors().border_variant)
                    .text_color(theme::colors().text_accent)
                    .font_weight(FontWeight::MEDIUM)
                    .text_size(px(12.))
                    .child(capture.primary_format.to_uppercase()),
            )
            .into_any_element()
    } else {
        div()
            .flex()
            .items_center()
            .justify_center()
            .size_full()
            .child(
                div()
                    
                    .text_color(theme::colors().text_muted)
                    .child(format!("{}.{}", capture.base_name, capture.primary_format.to_lowercase())),
            )
            .into_any_element()
    }
}

/// 判断 Capture 是否为非图片格式（视频等）（统一 OTHER 徽标）
fn is_other_format(capture: &CaptureMeta) -> bool {
    capture.primary_format.to_uppercase() == "OTHER"
}

/// 缩略图左上角格式徽标：RAW / JPG / RAW+JPG
fn render_format_badge(capture: &CaptureMeta) -> impl IntoElement + use<> {
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

/// 鸟种状态行：常驻占位，虚拟化等高约束，无记录渲染空行
fn render_bird_status_line(capture: &CaptureMeta) -> impl IntoElement + use<> {
    let colors = theme::colors();

    match capture.recognition_status {
        Some(RecognitionStatus::Confirmed) => {
            let name = capture.bird_name.as_deref().unwrap_or("");
            let confidence = capture.bird_confidence.unwrap_or(0.0);
            let conf_color = if confidence >= 80.0 {
                colors.success
            } else if confidence >= 50.0 {
                colors.warning
            } else {
                colors.info
            };

            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .h(px(18.))
                .child(
                    div()
                        
                        .text_color(conf_color)
                        .truncate()
                        .child(name.to_string()),
                )
                .child(
                    div()
                        
                        .font_family(theme::MONO_FONT_FAMILY)
                        .text_color(colors.text_muted)
                        .flex_shrink_0()
                        .child(format!("{:.1}%", confidence)),
                )
                .into_any_element()
        }
        Some(RecognitionStatus::NeedsReview) => {
            div()
                .flex()
                .flex_row()
                .items_center()
                .h(px(18.))
                .child(
                    div()
                        
                        .text_color(colors.warning)
                        .truncate()
                        .child("待复核"),
                )
                .into_any_element()
        }
        Some(RecognitionStatus::Unrecognized) => {
            div()
                .flex()
                .flex_row()
                .items_center()
                .h(px(18.))
                .child(
                    div()
                        
                        .text_color(colors.text_muted)
                        .child("未检测到鸟类"),
                )
                .into_any_element()
        }
        None => {
            // 空行占位：保持等高
            div().h(px(18.)).into_any_element()
        }
    }
}
