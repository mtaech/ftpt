use gpui::*;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::{Sizable, Selectable};

use crate::ui::theme;

/// 分区标题：全应用统一的 section header（粗体 xs muted）。
pub fn section_header(label: &str) -> Div {
    div()
        .font_weight(FontWeight::SEMIBOLD)
        .text_xs()
        .text_color(theme::colors().text_muted)
        .pb_1()
        .child(label.to_string())
}

/// 分段选择按钮：用于「任意/入选/淘汰/未标记」这类互斥选项组。
pub fn segmented_button(
    id: impl Into<ElementId>,
    label: &str,
    active: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Button {
    Button::new(id)
        .small()
        .ghost()
        .selected(active)
        .label(label)
        .on_click(on_click)
}

/// 小号清除入口：替代全宽 ghost 按钮，右对齐放置。
pub fn clear_link(
    id: impl Into<ElementId>,
    label: &str,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Button {
    Button::new(id)
        .small()
        .ghost()
        .label(label)
        .on_click(on_click)
}
