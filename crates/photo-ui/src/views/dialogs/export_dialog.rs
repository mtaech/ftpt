//! 导出对话框（对应手册 §9.10 与 §10.6）。
//!
//! 三态（与导入弹窗同一套「不可关闭的进度」规格）：
//! 1. **参数态**：预设 + 目标目录 + 长边 + JPEG 质量滑杆 + 命名模板（实时预览第一张）；
//! 2. **进度态**：`n/m · 当前文件` + 细进度条 + 取消（进行中不可关闭）；
//! 3. **结果态**：成功 / 失败计数 + 逐文件失败原因（**真实错误**，不再只报成功）。
//!
//! 以前这里是一个只会 `set_status_message("已开始导出照片")` 的假按钮，且全仓没有
//! 任何地方设 `ActiveDialog::Export`（弹窗连入口都没有）——Batch 1.1 把整条链路接上。

use std::sync::atomic::Ordering;

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Selectable as _, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::Input,
    slider::Slider,
    v_flex,
};
use gpui_kit::{AnyElement, Context, IntoElement, SharedString, Window, div, prelude::*, px};

use crate::model::export::{
    LONG_EDGE_OPTIONS, QUALITY_OPTIONS, export_targets, preset_from_draft, preset_index_for_draft,
};
use crate::state::AppState;
use crate::state::engine_ops::{defer_entity_action, pick_export_dest, start_export};

pub fn render_export_dialog(
    state: &AppState,
    _window: &mut Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement + use<> {
    let total = export_targets(&state.selected_indices, &state.display_order).len();
    let running = state.is_exporting;
    let has_results = !state.export_results.is_empty();

    div()
        .id("export-modal-overlay")
        .occlude()
        .absolute()
        .inset_0()
        .bg(gpui_kit::rgba(0x00000088))
        .flex()
        .items_center()
        .justify_center()
        .child(
            v_flex()
                .w(px(560.))
                .h(px(560.))
                .bg(cx.theme().sidebar)
                .rounded(px(20.))
                .border_1()
                .border_color(cx.theme().border.opacity(0.6))
                .shadow(crate::theme::overlay_shadow())
                .overflow_hidden()
                // ── 顶栏 ──
                .child(
                    h_flex()
                        .w_full()
                        .h(px(48.))
                        .items_center()
                        .justify_between()
                        .px_4()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .child(
                            div()
                                .font_semibold()
                                .text_sm()
                                .text_color(cx.theme().foreground)
                                .child(format!("照片导出 (共 {total} 张)")),
                        )
                        .child(
                            Button::new("close-export")
                                .ghost()
                                .small()
                                .icon(IconName::Close)
                                .disabled(running)
                                .tooltip(if running {
                                    "导出进行中，完成后才能关闭"
                                } else {
                                    "关闭 (Esc)"
                                })
                                .on_click(cx.listener(|state, _, _, cx| {
                                    state.active_dialog = None;
                                    cx.notify();
                                })),
                        ),
                )
                // ── 内容 ──
                .child(
                    v_flex()
                        .id("export-body")
                        .w_full()
                        .flex_1()
                        .p_5()
                        .gap_3()
                        .overflow_y_scroll()
                        .child(if running {
                            render_progress(state, cx)
                        } else if has_results {
                            render_results(state, cx)
                        } else {
                            render_controls(state, cx)
                        }),
                )
                // ── 底栏 ──
                .child(render_footer(state, cx)),
        )
}

/// 小号卡片容器（与导入弹窗同一规格）。
fn card(cx: &mut Context<AppState>) -> gpui_kit::Div {
    let shadow = crate::theme::panel_card_shadow(cx);
    v_flex()
        .w_full()
        .p_3()
        .gap_2()
        .rounded(px(12.))
        .border_1()
        .border_color(cx.theme().border.opacity(0.6))
        .bg(cx.theme().popover)
        .shadow(shadow)
}

/// 卡片小标题。
fn card_title(text: &'static str, cx: &mut Context<AppState>) -> gpui_kit::Div {
    div()
        .font_medium()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(text)
}

/// 可选中 chip（长边 / 质量档位 / 预设共用）。
fn chip(
    id: SharedString,
    label: SharedString,
    selected: bool,
    on_pick: impl Fn(&mut AppState) + 'static,
    cx: &mut Context<AppState>,
) -> Button {
    Button::new(id)
        .small()
        .selected(selected)
        .label(label)
        .on_click(cx.listener(move |state, _, _window, cx| {
            on_pick(state);
            cx.notify();
        }))
}

/// 参数态：预设 / 目标目录 / 长边 / 质量 / 命名模板（含实时预览）。
fn render_controls(state: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let draft = &state.export;

    let mut body = v_flex().w_full().gap_3();

    // ① 预设（来自配置；点一下套用长边 + 质量 + 模板，不动目标目录）
    //    新建 / 保存 / 删除都在这里（手册 §9.10），改完立刻落配置文件
    let presets = state.app_config.export_presets.clone();
    let current_preset_ix = preset_index_for_draft(&presets, draft);
    body = body.child(
        card(cx)
            .child(card_title("预设", cx))
            .when(!presets.is_empty(), |this| {
                this.child(h_flex().w_full().flex_wrap().gap_1().children(
                    presets.iter().enumerate().map(|(i, preset)| {
                        let preset = preset.clone();
                        let selected = draft.long_edge == preset.long_edge
                            && draft.quality == preset.quality
                            && draft.template == preset.template;
                        chip(
                            SharedString::from(format!("export-preset-{i}")),
                            SharedString::from(preset.name.clone()),
                            selected,
                            move |state| {
                                state.export.apply_preset(&preset);
                            },
                            cx,
                        )
                    }),
                ))
            })
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("export-preset-save")
                            .small()
                            .secondary()
                            .label("保存当前参数为预设")
                            .on_click(cx.listener(|state, _, _, cx| {
                                let name = format!(
                                    "预设 {}",
                                    state.app_config.export_presets.len() + 1
                                );
                                let preset = preset_from_draft(name.clone(), &state.export);
                                state.app_config.export_presets.push(preset);
                                state.save_config();
                                state.set_status_message(format!("已保存导出预设「{name}」"));
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("export-preset-delete")
                            .small()
                            .ghost()
                            .disabled(current_preset_ix.is_none())
                            .label("删除当前预设")
                            .on_click(cx.listener(|state, _, _, cx| {
                                let ix = preset_index_for_draft(
                                    &state.app_config.export_presets,
                                    &state.export,
                                );
                                if let Some(ix) = ix {
                                    let removed = state.app_config.export_presets.remove(ix);
                                    state.save_config();
                                    state.set_status_message(format!(
                                        "已删除导出预设「{}」",
                                        removed.name
                                    ));
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("预设保存到配置文件，重启后仍在"),
                    ),
            ),
    );

    // ② 目标目录：输入框 + 系统目录对话框
    body = body.child(
        card(cx)
            .child(card_title("目标目录", cx))
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .gap_2()
                    .when_some(state.export_dest_input.clone(), |this, input| {
                        this.child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(Input::new(&input).small().w_full()),
                        )
                    })
                    .child(
                        Button::new("export-pick-dest")
                            .small()
                            .secondary()
                            .icon(IconName::FolderOpen)
                            .label("浏览...")
                            .on_click(cx.listener(|_state, _, window, cx| {
                                pick_export_dest(window, cx);
                            })),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("目录不存在时自动创建；同名文件自动加 _1 / _2 后缀，不覆盖已有文件"),
            ),
    );

    // ③ 长边 + 质量
    body = body.child(
        card(cx)
            .child(card_title("输出尺寸与质量", cx))
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .w(px(52.))
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("长边"),
                    )
                    .child(h_flex().gap_1().children(LONG_EDGE_OPTIONS.iter().map(
                        |(value, label)| {
                            let value = *value;
                            chip(
                                SharedString::from(format!(
                                    "export-long-edge-{}",
                                    value.map(|v| v.to_string()).unwrap_or_else(|| "orig".into())
                                )),
                                SharedString::from(*label),
                                draft.long_edge == value,
                                move |state| state.export.long_edge = value,
                                cx,
                            )
                        },
                    ))),
            )
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .w(px(52.))
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("质量"),
                    )
                    .child(
                        div()
                            .px_1p5()
                            .py_0p5()
                            .rounded(px(6.))
                            .bg(cx.theme().muted.opacity(0.7))
                            .font_family(cx.theme().mono_font_family.clone())
                            .text_size(px(11.))
                            .text_color(cx.theme().primary)
                            .child(draft.quality_label()),
                    )
                    .children(QUALITY_OPTIONS.iter().map(|q| {
                        let q = *q;
                        chip(
                            SharedString::from(format!("export-quality-{q}")),
                            SharedString::from(format!("{q}")),
                            draft.quality == q,
                            move |state| state.export.quality = q,
                            cx,
                        )
                    })),
            )
            .when_some(state.export_quality_slider.clone(), |this, slider| {
                this.child(div().w_full().child(Slider::new(&slider).horizontal()))
            }),
    );

    // ④ 命名模板（实时预览第一张目标照片的输出名）
    body = body.child(
        card(cx)
            .child(card_title("命名模板", cx))
            .when_some(state.export_template_input.clone(), |this, input| {
                this.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(Input::new(&input).small().w_full()),
                )
            })
            .child(render_template_preview(state, cx))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("占位符：{name} 原名 · {species} 物种 · {date} 拍摄日期 · {seq} 序号 · {camera} 机型"),
            ),
    );

    body.into_any_element()
}

/// 命名模板的实时预览：用第一张目标照片的元数据渲染（{seq} 固定 001）。
fn render_template_preview(state: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let first = export_targets(&state.selected_indices, &state.display_order)
        .first()
        .and_then(|i| state.items.get(*i));
    let Some(meta) = first else {
        return div()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child("当前没有可导出的照片")
            .into_any_element();
    };
    let ctx = photo_engine::template::NameTemplateContext {
        name: meta.base_name.clone(),
        species: meta.taxon_name.clone(),
        date: meta.date_taken.clone(),
        camera: meta.camera_model.clone(),
        seq: 1,
    };
    let rendered = photo_engine::template::render_name_template(&state.export.template, &ctx);
    div()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(format!("示例：{rendered}.jpg"))
        .into_any_element()
}

/// 进度态：n/m · 当前文件 + 细进度条 + 取消。
fn render_progress(state: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    const BAR_W: f32 = 420.0;
    let total = state.export_total.max(1);
    let done = state.export_done.min(total);
    let ratio = done as f32 / total as f32;
    let cancel = state.export_cancel.clone();

    card(cx)
        .child(card_title("导出进度", cx))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().foreground)
                .child(format!("{done}/{total}")),
        )
        .child(
            div()
                .w(px(BAR_W))
                .h(px(6.))
                .rounded_full()
                .bg(cx.theme().border)
                .child(
                    div()
                        .h_full()
                        .rounded_full()
                        .bg(cx.theme().primary)
                        .w(px(BAR_W * ratio)),
                ),
        )
        .child(
            div()
                .max_w(px(BAR_W))
                .truncate()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(state.export_current.clone()),
        )
        .child(
            h_flex()
                .w_full()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("RAW 全尺寸解码较慢，导出在后台进行，界面仍可浏览"),
                )
                .child(
                    Button::new("export-cancel-run")
                        .ghost()
                        .small()
                        .label("取消导出")
                        .on_click(move |_, _window, _cx| {
                            cancel.store(true, Ordering::Relaxed);
                        }),
                ),
        )
        .into_any_element()
}

/// 结果态：成功 / 失败计数 + 逐文件失败真实错误。
fn render_results(state: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let success = state.export_results.iter().filter(|(_, r)| r.is_ok()).count();
    let failed = state.export_results.iter().filter(|(_, r)| r.is_err()).count();
    let dest = state.export.dest_dir.clone();

    let mut body = card(cx)
        .child(card_title("导出结果", cx))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().foreground)
                .child(format!("成功 {success} 张 · 失败 {failed} 张 → {dest}")),
        );

    if failed > 0 {
        body = body.child(
            div()
                .text_xs()
                .text_color(cx.theme().warning)
                .child("失败明细（原文件未被修改）："),
        );
        body = body.child(v_flex().w_full().gap_1().children(
            state
                .export_results
                .iter()
                .filter_map(|(name, r)| r.as_ref().err().map(|e| (name.clone(), e.clone())))
                .take(20)
                .map(|(name, error)| {
                    h_flex()
                        .w_full()
                        .gap_2()
                        .text_xs()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_color(cx.theme().foreground)
                                .child(name),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_color(cx.theme().warning)
                                .child(error),
                        )
                }),
        ));
        if failed > 20 {
            body = body.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("…等共 {failed} 张失败")),
            );
        }
    }

    body.into_any_element()
}

/// 底栏：口径提示 + 关闭 / 再次导出 / 开始导出。
fn render_footer(state: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let running = state.is_exporting;
    let has_results = !state.export_results.is_empty();
    let targets = export_targets(&state.selected_indices, &state.display_order);
    let scope = if state.selected_indices.is_empty() {
        "当前筛选结果全部"
    } else {
        "已选照片"
    };

    h_flex()
        .w_full()
        .h(px(56.))
        .items_center()
        .justify_between()
        .px_4()
        .border_t_1()
        .border_color(cx.theme().border)
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(format!("{} 张（{scope}）", targets.len())),
        )
        .child(
            h_flex()
                .gap_2()
                .child(
                    Button::new("cancel-export-btn")
                        .ghost()
                        .small()
                        .label(if has_results { "关闭" } else { "取消" })
                        .disabled(running)
                        .on_click(cx.listener(|state, _, _, cx| {
                            state.active_dialog = None;
                            cx.notify();
                        })),
                )
                .when(has_results, |this| {
                    this.child(
                        Button::new("export-again-btn")
                            .small()
                            .secondary()
                            .label("再次导出")
                            .disabled(running)
                            .on_click(cx.listener(|state, _, _, cx| {
                                // 回到参数态；上一批结果清掉（新一次导出重新记）
                                state.export_results.clear();
                                cx.notify();
                            })),
                    )
                })
                .when(!has_results, |this| {
                    this.child(
                        Button::new("run-export-btn")
                            .small()
                            .primary()
                            .icon(IconName::ExternalLink)
                            .label(if running {
                                "导出中…".to_string()
                            } else {
                                format!("开始导出（{} 张）", targets.len())
                            })
                            .disabled(running || targets.is_empty() || !state.export.is_ready())
                            .on_click(cx.listener(|state, _, _window, cx| {
                                // 输入框文本 → 草稿（滑杆已走 Change 订阅）
                                state.read_export_inputs(cx);
                                let entity = cx.entity();
                                defer_entity_action(cx, entity, |entity, cx| {
                                    start_export(entity, cx)
                                });
                            })),
                    )
                }),
        )
        .into_any_element()
}
