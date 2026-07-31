use std::collections::HashSet;

use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_component::scroll::ScrollableElement;
use gpui_component::v_flex;
use gpui_component::h_flex;

use photo_domain::BatchOpType;

use crate::state::app::RootView;
use crate::ui::theme;

/// 左侧边栏「文件操作」tab（ADR 0006：筛选驱动 + 画面粒度）
///
/// 操作对象 = 当前筛选结果；[移动到…] [复制到…] 一步式选目录即执行；
/// [删除] 独立红色，弹确认（含同名同步数量）；「同步同名文件」开关 + 格式多选。
pub fn render_batch_ops_section(
    view: &RootView,
    _cx: &mut Context<RootView>,
) -> impl IntoElement {
    let vh = _cx.entity().downgrade();

    let count = view.display_order.len();
    let empty = count == 0;
    let sync_enabled = view.batch_sync_enabled;
    let sync_extra = view.batch_sync_extra;
    let in_progress = view.batch_in_progress;
    let has_results = !view.batch_results.is_empty();
    let results = view.batch_results.clone();

    v_flex()
        .gap_2()
        // ── 操作对象说明（数量随筛选实时联动）──
        .child({
            let rule = if sync_enabled {
                format!("操作对象：当前筛选结果（{count} 张），同步 +{sync_extra}")
            } else {
                format!("操作对象：当前筛选结果（{count} 张）")
            };
            div()
                .px_2()
                .py_1()
                .rounded_sm()
                .bg(theme::colors().surface_background)
                .text_size(px(11.))
                .text_color(theme::colors().text_muted)
                .child(rule)
        })
        // ── 同步开关 ──
        .child({
            let vh = vh.clone();
            div()
                .id("batch-sync-toggle")
                .flex()
                .items_center()
                .gap_1()
                .cursor_pointer()
                .on_click(move |_, _window, cx| {
                    if let Some(e) = vh.upgrade() {
                        cx.update_entity(&e, |view, cx| {
                            view.set_batch_sync_enabled(!view.batch_sync_enabled, cx);
                        });
                    }
                })
                .child(
                    div()
                        .w(px(14.))
                        .h(px(14.))
                        .rounded_sm()
                        .border_1()
                        .border_color(if sync_enabled {
                            theme::colors().text_accent
                        } else {
                            theme::colors().border_variant
                        })
                        .bg(if sync_enabled {
                            theme::colors().text_accent
                        } else {
                            hsla(0., 0., 0., 0.)
                        })
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(if sync_enabled {
                            div()
                                .text_size(px(10.))
                                .text_color(theme::colors().background)
                                .child("✓")
                                .into_any_element()
                        } else {
                            div().into_any_element()
                        }),
                )
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(theme::colors().text)
                        .child("同步同名文件"),
                )
        })
        // ── 同步格式多选（勾选开关后出现，默认全选）──
        .when(sync_enabled, |parent| {
            // 目录实际出现的格式（ImageFormat Display 大写，与 engine 匹配键一致）
            let mut all_formats: HashSet<String> = HashSet::new();
            for m in &view.captures {
                all_formats.insert(m.primary_format.to_string().to_uppercase());
            }
            let mut options: Vec<String> = all_formats.into_iter().collect();
            options.sort();
            let mut chips: Vec<AnyElement> = options
                .iter()
                .map(|fmt| {
                    let vh = vh.clone();
                    let fmt = fmt.clone();
                    let active = view.batch_sync_formats.contains(&fmt);
                    div()
                        .id(SharedString::from(format!("sync-fmt-{fmt}")))
                        .px_2()
                        .py_0p5()
                        .rounded_full()
                        .text_size(px(11.))
                        .cursor_pointer()
                        .text_color(if active {
                            theme::colors().background
                        } else {
                            theme::colors().text_muted
                        })
                        .bg(if active {
                            theme::colors().text_accent
                        } else {
                            theme::colors().element_background
                        })
                        .hover(|style| style.bg(if active { theme::accent_hover() } else { theme::colors().element_hover }))
                        .on_click({
                            let fmt_click = fmt.clone();
                            move |_, _window, cx| {
                                if let Some(e) = vh.upgrade() {
                                    cx.update_entity(&e, |view, cx| {
                                        view.toggle_batch_sync_format(&fmt_click, cx);
                                    });
                                }
                            }
                        })
                        .child(fmt.clone())
                        .into_any_element()
                })
                .collect();
            if chips.is_empty() {
                chips.push(
                    div()
                        .text_size(px(11.))
                        .text_color(theme::colors().text_muted)
                        .child("目录中没有其他格式的同名文件")
                        .into_any_element(),
                );
            }
            parent.child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(theme::colors().text_muted)
                            .child("同步格式"),
                    )
                    .child(div().flex().flex_wrap().gap_1().children(chips)),
            )
        })
        // ── 主操作横排：移动到… / 复制到… ──
        .child({
            let vh = vh.clone();
            h_flex()
                .gap_2()
                .child(render_action_button(
                    "batch-move",
                    "移动到…",
                    BatchOpType::Move,
                    empty,
                    vh.clone(),
                ))
                .child(render_action_button(
                    "batch-copy",
                    "复制到…",
                    BatchOpType::Copy,
                    empty,
                    vh,
                ))
        })
        // ── 删除（独立红色，与主操作隔离）──
        .child({
            let vh = vh.clone();
            div()
                .mt_1()
                .child(render_delete_button("batch-delete", empty, vh))
        })
        // ── 执行中提示 ──
        .when(in_progress, |parent| {
            parent.child(
                div()
                    .text_size(px(11.))
                    .text_color(theme::colors().text_muted)
                    .child("执行中…"),
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
                        
                        .text_color(if is_err {
                            theme::colors().error
                        } else {
                            theme::colors().text
                        })
                        .child(msg.clone())
                        .into_any_element()
                })
                .collect();
            let ok = results.iter().filter(|m| !m.contains("失败")).count();
            let fail = results.len() - ok;
            parent.child(
                v_flex()
                    .mt_2()
                    .gap_1()
                    .child(label("执行结果"))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(if fail > 0 {
                                theme::colors().warning
                            } else {
                                theme::colors().text_muted
                            })
                            .child(format!("成功 {ok} / 失败 {fail}")),
                    )
                    .child(div().max_h(px(240.)).overflow_y_scrollbar().children(items)),
            )
        })
}

/// 主操作按钮（移动/复制）：accent 底，空筛选时禁用
fn render_action_button(
    id: &'static str,
    label: &'static str,
    op: BatchOpType,
    disabled: bool,
    vh: WeakEntity<RootView>,
) -> impl IntoElement {
    div()
        .id(ElementId::Name(id.into()))
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .py_2()
        .rounded_md()
        
        .font_weight(FontWeight::MEDIUM)
        .text_size(px(12.))
        .cursor_pointer()
        .bg(theme::colors().text_accent)
        .text_color(theme::colors().background)
        .hover(|style| style.bg(theme::accent_hover()))
        .when(disabled, |style| style.opacity(0.4).cursor_default())
        .on_click(move |_, _window, cx| {
            if disabled {
                return;
            }
            if let Some(e) = vh.upgrade() {
                cx.update_entity(&e, |view, cx| {
                    view.batch_move_or_copy(op, cx);
                });
            }
        })
        .child(label)
}

/// 删除按钮：独立红色，空筛选时禁用
fn render_delete_button(id: &'static str, disabled: bool, vh: WeakEntity<RootView>) -> impl IntoElement {
    div()
        .id(ElementId::Name(id.into()))
        .flex()
        .items_center()
        .justify_center()
        .py_2()
        .rounded_md()
        
        .font_weight(FontWeight::MEDIUM)
        .text_size(px(12.))
        .cursor_pointer()
        .bg(theme::colors().error)
        .text_color(theme::colors().background)
        .hover(|style| style.bg(theme::accent_hover()))
        .when(disabled, |style| style.opacity(0.4).cursor_default())
        .on_click(move |_, _window, cx| {
            if disabled {
                return;
            }
            if let Some(e) = vh.upgrade() {
                cx.update_entity(&e, |view, cx| {
                    view.batch_delete(cx);
                });
            }
        })
        .child("删除")
}

fn label(text: &'static str) -> impl IntoElement {
    div()
        
        .font_weight(FontWeight::MEDIUM)
        .text_color(theme::colors().text_muted)
        .child(text)
}
