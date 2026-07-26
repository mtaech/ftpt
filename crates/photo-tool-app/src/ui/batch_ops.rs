use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_component::{h_flex, v_flex};

use photo_domain::BatchOpType;

use crate::state::app::RootView;
use crate::ui::theme;

/// 渲染左侧边栏的"文件操作" tab
pub fn render_batch_ops_section(
    view: &RootView,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    let vh = cx.entity().downgrade();

    let compare_dir = view.batch_compare_dir.clone();
    let source_fmt = view.batch_source_format.clone();
    let compare_fmt = view.batch_compare_format.clone();
    let op_type = view.batch_op_type;
    let in_progress = view.batch_in_progress;
    let has_results = !view.batch_results.is_empty();
    let results = view.batch_results.clone();
    let compare_dir: SharedString = if view.batch_compare_dir.is_empty() {
        "选择对比目录...".into()
    } else {
        view.batch_compare_dir.clone().into()
    };
    v_flex()
        .gap_2()
        // ── 对比目录 ──
        .child(
            v_flex()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme::colors().text_muted)
                        .child("对比目录"),
                )
                .child(
                    h_flex()
                        .gap_1()
                        .child({
                            let vh = vh.clone();
                            div()
                                .id("batch-compare-dir")
                                .flex_1()
                                .px_2()
                                .py_1()
                                .rounded_sm()
                                .bg(theme::colors().element_background)
                                .text_xs()
                                .text_color(if compare_dir.is_empty() {
                                    theme::colors().text_muted
                                } else {
                                    theme::colors().text
                                })
                                .child(compare_dir.clone())
                                .cursor_pointer()
                                .on_click(move |_, _window, cx| {
                                    if let Some(e) = vh.upgrade() {
                                        cx.update_entity(&e, |view, cx| {
                                            view.pick_batch_compare_dir(cx);
                                        });
                                    }
                                })
                        }),
                ),
        )
        // ── 格式设置 ──
        .child(
            h_flex()
                .gap_2()
                .child(format_input("源格式", source_fmt, "留空=全部"))
                .child(format_input("对比格式", compare_fmt, "留空=全部")),
        )
        // ── 操作类型下拉 ──
        .child(
            v_flex()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme::colors().text_muted)
                        .child("操作类型"),
                )
                .child({
                    let items: Vec<gpui::AnyElement> = BatchOpType::all()
                        .iter()
                        .map(|op| {
                            let vh = vh.clone();
                            let is_active = op_type == *op;
                            let label: SharedString = op.to_string().into();
                            div()
                                .id(label.clone())
                                .px_2()
                                .py_1()
                                .rounded_sm()
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
                                            cx.notify();
                                        });
                                    }
                                })
                                .child(label)
                                .into_any_element()
                        })
                        .collect();
                    v_flex().gap_1().children(items)
                }),
        )
        // ── 开始执行按钮 ──
        .child(
            div()
                .mt_2()
                .child({
                    let vh = vh.clone();
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
                        .child(if in_progress {
                            "执行中..."
                        } else {
                            "开始执行"
                        })
                }),
        )
        // ── 结果列表 ──
        .when(has_results, |parent| {
            parent.child({
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
                v_flex()
                    .mt_2()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::colors().text_muted)
                            .child("执行结果"),
                    )
                    .child(div().flex_1().children(items))
            })
        })
}

/// 格式输入框组件（已去消所有借用，只持 owned String）
fn format_input(label: &'static str, value: String, placeholder: &'static str) -> impl IntoElement {
    let is_empty = value.is_empty();
    let display: SharedString = if is_empty {
        placeholder.into()
    } else {
        value.into()
    };
    v_flex()
        .flex_1()
        .gap_1()
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme::colors().text_muted)
                .child(label),
        )
        .child(
            div()
                .id(SharedString::from(label))
                .px_2()
                .py_1()
                .rounded_sm()
                .bg(theme::colors().element_background)
                .text_xs()
                .text_color(if is_empty {
                    theme::colors().text_muted
                } else {
                    theme::colors().text
                })
                .child(display),
        )
}
