use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::{h_flex, v_flex};
use photo_domain::BatchOpType;

use crate::state::app::RootView;
use crate::ui::theme;

const FORMAT_OPTIONS: &[(&str, &str)] = &[
    ("全部类型", ""),
    ("JPG", "JPG"),
    ("PNG", "PNG"),
    ("TIFF", "TIFF"),
    ("HEIF", "HEIF"),
    ("WebP", "WebP"),
    ("BMP", "BMP"),
    ("GIF", "GIF"),
    ("RW2", "RW2"),
    ("CR2", "CR2"),
    ("CR3", "CR3"),
    ("NEF", "NEF"),
    ("ARW", "ARW"),
    ("ORF", "ORF"),
    ("DNG", "DNG"),
];

pub fn render_batch_ops_section(
    view: &RootView,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    let vh = cx.entity().downgrade();

    let compare_dir: SharedString = if view.batch_compare_dir.is_empty() {
        "选择对比目录...".into()
    } else {
        view.batch_compare_dir.clone().into()
    };
    let source_fmt = view.batch_source_format.clone();
    let compare_fmt = view.batch_compare_format.clone();
    let op_type = view.batch_op_type;
    let has_results = !view.batch_results.is_empty();
    let results = view.batch_results.clone();
    let in_progress = view.batch_in_progress;

    v_flex()
        .gap_2()
        // ── 对比目录 ──
        .child({
            let vh = vh.clone();
            v_flex()
                .gap_1()
                .child(label("对比目录"))
                .child(
                    Button::new("batch-compare-dir")
                        .ghost()
                        .tooltip("点击选择要对比的目录")
                        .label(if compare_dir.is_empty() {
                            SharedString::from("选择对比目录...")
                        } else {
                            compare_dir.clone()
                        })
                        .on_click(move |_, _window, cx| {
                            if let Some(e) = vh.upgrade() {
                                cx.update_entity(&e, |view, cx| {
                                    view.pick_batch_compare_dir(cx);
                                });
                            }
                        }),
                )
        })
        // ── 源格式 ──
        .child(format_dropdown("源格式", &source_fmt, "batch-source-fmt",
            view.batch_source_fmt_open, &vh))
        // ── 对比格式 ──
        .child(format_dropdown("对比格式", &compare_fmt, "batch-compare-fmt",
            view.batch_compare_fmt_open, &vh))
        // ── 操作类型 ──
        .child({
            let vh_click = vh.clone();
            let vh_options = vh.clone();
            v_flex()
                .gap_1()
                .child(label("操作类型"))
                .child(
                    div()
                        .id("batch-op-selected")
                        .flex()
                        .items_center()
                        .justify_between()
                        .px_2()
                        .py_1()
                        .rounded_sm()
                        .bg(theme::colors().element_background)
                        .text_xs()
                        .cursor_pointer()
                        .on_click(move |_, _window, cx| {
                            if let Some(e) = vh_click.upgrade() {
                                cx.update_entity(&e, |view, cx| {
                                    view.batch_op_dropdown_open = !view.batch_op_dropdown_open;
                                    view.batch_source_fmt_open = false;
                                    view.batch_compare_fmt_open = false;
                                    cx.notify();
                                });
                            }
                        })
                        .child(SharedString::from(op_type.to_string()))
                        .child(arrow()),
                )
                .when(view.batch_op_dropdown_open, move |parent| {
                    let items: Vec<gpui::AnyElement> = BatchOpType::all()
                        .iter()
                        .map(|op| {
                            let vh = vh_options.clone();
                            let is_active = op_type == *op;
                            let label: SharedString = op.to_string().into();
                            div()
                                .id(label.clone())
                                .px_2()
                                .py_1()
                                .text_xs()
                                .cursor_pointer()
                                .text_color(if is_active {
                                    theme::colors().text_accent
                                } else {
                                    theme::colors().text
                                })
                                .bg(if is_active {
                                    theme::accent_dim()
                                } else {
                                    hsla(0., 0., 0., 0.)
                                })
                                .hover(|style| style.bg(theme::colors().element_hover))
                                .on_click(move |_, _window, cx| {
                                    if let Some(e) = vh.upgrade() {
                                        cx.update_entity(&e, |view, cx| {
                                            view.batch_op_type = *op;
                                            view.batch_op_dropdown_open = false;
                                            cx.notify();
                                        });
                                    }
                                })
                                .child(label)
                                .into_any_element()
                        })
                        .collect();
                    parent.child(option_list(items))
                })
        })
        // ── 开始执行按钮 ──
        .child({
            let vh = vh.clone();
            div()
                .mt_2()
                .child(
                    div()
                        .id("batch-execute")
                        .flex()
                        .items_center()
                        .justify_center()
                        .py_2()
                        .rounded_md()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .cursor_pointer()
                        .bg(theme::colors().text_accent)
                        .text_color(theme::colors().background)
                        .hover(|style| style.bg(theme::accent_hover()))
                        .when(in_progress, |style| {
                            style.opacity(0.5).cursor_default()
                        })
                        .on_click(move |_, _window, cx| {
                            if let Some(e) = vh.upgrade() {
                                cx.update_entity(&e, |view, cx| {
                                    view.execute_batch_ops(cx);
                                });
                            }
                        })
                        .child(if in_progress { "执行中..." } else { "开始执行" }),
                )
        })
        // ── 结果列表 ──
        .when(has_results, |parent| {
            let items: Vec<gpui::AnyElement> = results
                .iter()
                .map(|msg| {
                    let is_err = msg.contains("失败");
                    div()
                        .py_0p5()
                        .px_1()
                        .text_xs()
                        .text_color(if is_err {
                            theme::colors().error
                        } else {
                            theme::colors().text
                        })
                        .child(msg.clone())
                        .into_any_element()
                })
                .collect();
            parent.child(
                v_flex()
                    .mt_2()
                    .gap_1()
                    .child(label("执行结果"))
                    .child(div().flex_1().children(items)),
            )
        })
}

fn label(text: &'static str) -> impl IntoElement {
    div()
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme::colors().text_muted)
        .child(text)
}

fn arrow() -> impl IntoElement {
    div().text_xs().text_color(theme::colors().text_muted).child("▼")
}

fn option_list(items: Vec<gpui::AnyElement>) -> impl IntoElement {
    div()
        .mt_1()
        .rounded_sm()
        .bg(theme::colors().surface_background)
        .border_1()
        .border_color(theme::colors().border_variant)
        .py_1()
        .children(items)
}

fn format_dropdown(
    label_text: &'static str,
    current: &str,
    id_suffix: &'static str,
    is_open: bool,
    vh: &WeakEntity<RootView>,
) -> impl IntoElement {
    let vh_click = vh.clone();
    let vh_options = vh.clone();
    let display: SharedString = if current.is_empty() {
        "全部".into()
    } else {
        current.to_string().into()
    };

    v_flex()
        .gap_1()
        .child(label(label_text))
        .child(
            div()
                .id(SharedString::from(format!("fmt-sel-{}", id_suffix)))
                .flex()
                .items_center()
                .justify_between()
                .px_2()
                .py_1()
                .rounded_sm()
                .bg(theme::colors().element_background)
                .text_xs()
                .cursor_pointer()
                .on_click(move |_, _window, cx| {
                    if let Some(e) = vh_click.upgrade() {
                        cx.update_entity(&e, |view, cx| {
                            toggle_fmt_dropdown(id_suffix, view);
                            cx.notify();
                        });
                    }
                })
                .child(display)
                .child(arrow()),
        )
        .when(is_open, move |parent| {
            let items: Vec<gpui::AnyElement> = FORMAT_OPTIONS
                .iter()
                .map(|&(display_name, value)| {
                    let vh = vh_options.clone();
                    let is_active =
                        (current.is_empty() && value.is_empty()) || current == value;
                    div()
                        .id(SharedString::from(display_name))
                        .px_2()
                        .py_1()
                        .text_xs()
                        .cursor_pointer()
                        .text_color(if is_active {
                            theme::colors().text_accent
                        } else {
                            theme::colors().text
                        })
                        .bg(if is_active {
                            theme::accent_dim()
                        } else {
                            hsla(0., 0., 0., 0.)
                        })
                        .hover(|style| style.bg(theme::colors().element_hover))
                        .on_click(move |_, _window, cx| {
                            if let Some(e) = vh.upgrade() {
                                cx.update_entity(&e, |view, cx| {
                                    let field = id_suffix;
                                    match field {
                                        "batch-source-fmt" => {
                                            view.batch_source_format = value.to_string();
                                            view.batch_source_fmt_open = false;
                                        }
                                        "batch-compare-fmt" => {
                                            view.batch_compare_format = value.to_string();
                                            view.batch_compare_fmt_open = false;
                                        }
                                        _ => {}
                                    }
                                    cx.notify();
                                });
                            }
                        })
                        .child(display_name)
                        .into_any_element()
                })
                .collect();
            parent.child(option_list(items))
        })
}

fn toggle_fmt_dropdown(id: &str, view: &mut RootView) {
    match id {
        "batch-source-fmt" => {
            view.batch_source_fmt_open = !view.batch_source_fmt_open;
            view.batch_compare_fmt_open = false;
            view.batch_op_dropdown_open = false;
        }
        "batch-compare-fmt" => {
            view.batch_compare_fmt_open = !view.batch_compare_fmt_open;
            view.batch_source_fmt_open = false;
            view.batch_op_dropdown_open = false;
        }
        _ => {}
    }
}
