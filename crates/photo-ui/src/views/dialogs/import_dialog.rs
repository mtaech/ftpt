//! 导入弹窗（对应手册 §9.10 与 §10.3）。
//!
//! 顶部 导入 / 添加 分段：
//! - 导入：可移动盘或浏览目录选源 → 扫描 → 目标根目录 + 子目录模式 + 重命名模板
//!   → 复制/移动 → 干跑计划预览（N 张 → M 个目标目录 + 跳过清单）→ 执行进度 → 结果；
//!   全部成功自动关闭并重扫结果目录，有失败留在面板看明细。
//! - 添加：选目录直接打开浏览，不动任何文件。
//!
//! 遮罩点击与 Esc 都不关闭（有状态的扫描/计划流程），只走右上角 ×。

use std::path::PathBuf;

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Selectable as _, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::Input,
    tab::{Tab, TabBar},
    v_flex,
};
use gpui_kit::{AnyElement, Context, IntoElement, SharedString, Window, div, prelude::*, px};
use photo_engine::import::{ImportMode, ImportSubfolder};

use crate::state::AppState;
use crate::state::engine_ops::defer_entity_action;
use crate::state::import::{self, ImportTab};

pub fn render_import_dialog(
    state: &AppState,
    _window: &mut Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement + use<> {
    let is_import = state.import.tab == ImportTab::Import;
    let running = state.import.running;

    div()
        .id("import-modal-overlay")
        .occlude()
        .absolute()
        .inset_0()
        .bg(gpui_kit::rgba(0x00000088))
        .flex()
        .items_center()
        .justify_center()
        .child(
            v_flex()
                .w(px(680.))
                .h(px(640.))
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
                                .child("照片导入 (SD 卡 / 目录)"),
                        )
                        .child(
                            Button::new("close-import")
                                .ghost()
                                .small()
                                .icon(IconName::Close)
                                .disabled(running)
                                .on_click(cx.listener(|state, _, _, cx| {
                                    state.active_dialog = None;
                                    cx.notify();
                                })),
                        ),
                )
                // ── 内容 ──
                .child(
                    v_flex()
                        .id("import-scroll")
                        .w_full()
                        .flex_1()
                        .p_4()
                        .gap_3()
                        .overflow_y_scroll()
                        .child(render_tabs(state, cx))
                        .child(if is_import {
                            render_import_pane(state, cx)
                        } else {
                            render_add_pane(state, cx)
                        }),
                )
                // ── 底栏 ──
                .child(render_footer(state, cx)),
        )
}

/// 顶部 导入 / 添加 分段（与顶栏视图 tab 同一 underline 规格）。
fn render_tabs(state: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let import_selected = state.import.tab == ImportTab::Import;
    TabBar::new("import-tabs")
        .underline()
        .selected_index(if import_selected { 0 } else { 1 })
        .on_click(cx.listener(|state, &ix, _window, cx| {
            state.import.tab = if ix == 0 {
                ImportTab::Import
            } else {
                ImportTab::Add
            };
            cx.notify();
        }))
        .child(Tab::new().label("导入"))
        .child(Tab::new().label("添加"))
        .into_any_element()
}

/// 小号卡片容器（来源 / 目标 / 选项 / 计划共用）。
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

/// 可选中的小按钮（子目录模式 / 复制移动共用）。
fn choice_button(
    id: &'static str,
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
            // 任何选项变化都让已生成的计划失效（需重新干跑）
            state.import.plan = None;
            state.import.result = None;
            cx.notify();
        }))
}

fn render_import_pane(state: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    v_flex()
        .w_full()
        .gap_3()
        .child(render_source_card(state, cx))
        .child(render_target_card(state, cx))
        .child(render_options_card(state, cx))
        .child(render_plan_card(state, cx))
        .when_some(state.import.error.clone(), |this, (message, _)| {
            this.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().warning)
                    .child(message),
            )
        })
        .into_any_element()
}

/// ① 来源：可移动盘列表（点击即选）+ 浏览目录 + 重新扫描。
fn render_source_card(state: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let drives = state.import.drives.clone();
    let source_text = state
        .import
        .source
        .as_ref()
        .map(|p| p.to_string_lossy().to_string());
    let scanning = state.import.scanning;
    let candidate_count = state.import.candidates.len();

    let mut body = card(cx)
        .child(card_title("来源", cx))
        .child(if drives.is_empty() {
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("未检测到可移动磁盘或 SD 卡，请插入后重试，或手动浏览目录")
                .into_any_element()
        } else {
            v_flex()
                .w_full()
                .gap_1()
                .children(drives.into_iter().map(|drive| {
                    let path = drive.path.clone();
                    let label = drive.label.clone().unwrap_or_else(|| drive.path.clone());
                    let selected = source_text.as_deref() == Some(path.as_str());
                    let pick = path.clone();
                    let row_id = SharedString::from(format!("import-drive-{path}"));
                    h_flex()
                        .id(row_id)
                        .w_full()
                        .p_2()
                        .rounded(cx.theme().radius)
                        .bg(if selected {
                            cx.theme().selection
                        } else {
                            cx.theme().background
                        })
                        .border_1()
                        .border_color(cx.theme().border)
                        .items_center()
                        .justify_between()
                        .text_xs()
                        .child(div().text_color(cx.theme().foreground).child(label))
                        .child(div().text_color(cx.theme().muted_foreground).child(path))
                        .on_click(cx.listener(move |_state, _, _window, cx| {
                            let entity = cx.entity();
                            import::set_source_and_scan(entity, PathBuf::from(pick.clone()), cx);
                            cx.notify();
                        }))
                }))
                .into_any_element()
        });

    body = body.child(
        h_flex()
            .w_full()
            .items_center()
            .gap_2()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(source_text.unwrap_or_else(|| "未选择来源".to_string())),
            )
            .child(
                Button::new("import-browse-source")
                    .small()
                    .secondary()
                    .label("浏览...")
                    .disabled(scanning)
                    .on_click(cx.listener(|_state, _, window, cx| {
                        import::pick_source(window, cx);
                    })),
            )
            .child(
                Button::new("import-rescan-source")
                    .small()
                    .ghost()
                    .label(if scanning {
                        "扫描中…"
                    } else {
                        "重新扫描"
                    })
                    .disabled(scanning || state.import.source.is_none())
                    .on_click(cx.listener(|_state, _, _window, cx| {
                        let entity = cx.entity();
                        import::rescan(entity, cx);
                    })),
            ),
    );

    if !scanning && candidate_count > 0 {
        body = body.child(
            div()
                .text_xs()
                .text_color(cx.theme().primary)
                .child(format!("已扫描 {candidate_count} 张，可以选目标并生成计划")),
        );
    }
    body.into_any_element()
}

/// ② 目标根目录（选择或手输，不存在自动创建）。
fn render_target_card(state: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    card(cx)
        .child(card_title("目标根目录", cx))
        .child(
            h_flex()
                .w_full()
                .items_center()
                .gap_2()
                .when_some(state.import_dest_input.clone(), |this, input| {
                    this.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&input).small().w_full()),
                    )
                })
                .child(
                    Button::new("import-pick-dest")
                        .small()
                        .secondary()
                        .label("选择...")
                        .on_click(cx.listener(|_state, _, window, cx| {
                            import::pick_dest(window, cx);
                        })),
                ),
        )
        .into_any_element()
}

/// ③ 子目录模式 + 重命名模板 + 复制/移动。
fn render_options_card(state: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let subfolder = state.import.subfolder;
    let mode = state.import.mode;
    card(cx)
        .child(card_title("目录与命名", cx))
        .child(
            h_flex()
                .w_full()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .w(px(64.))
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("子目录"),
                )
                .child(
                    h_flex()
                        .flex_wrap()
                        .gap_1()
                        .child(choice_button(
                            "import-subfolder-none",
                            "不建".into(),
                            subfolder == ImportSubfolder::None,
                            |state| state.import.subfolder = ImportSubfolder::None,
                            cx,
                        ))
                        .child(choice_button(
                            "import-subfolder-dash",
                            "YYYY-MM-DD".into(),
                            subfolder == ImportSubfolder::DateDash,
                            |state| state.import.subfolder = ImportSubfolder::DateDash,
                            cx,
                        ))
                        .child(choice_button(
                            "import-subfolder-slash",
                            "YYYY/MM/DD".into(),
                            subfolder == ImportSubfolder::DateSlash,
                            |state| state.import.subfolder = ImportSubfolder::DateSlash,
                            cx,
                        ))
                        .child(choice_button(
                            "import-subfolder-compact",
                            "YYYYMMDD".into(),
                            subfolder == ImportSubfolder::DateCompact,
                            |state| state.import.subfolder = ImportSubfolder::DateCompact,
                            cx,
                        )),
                ),
        )
        .child(
            h_flex()
                .w_full()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .w(px(64.))
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("重命名"),
                )
                .when_some(state.import_rename_input.clone(), |this, input| {
                    this.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&input).small().w_full()),
                    )
                }),
        )
        .child(
            h_flex()
                .w_full()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .w(px(64.))
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("方式"),
                )
                .child(
                    h_flex()
                        .gap_1()
                        .child(choice_button(
                            "import-mode-copy",
                            "复制（源保留）".into(),
                            mode == ImportMode::Copy,
                            |state| state.import.mode = ImportMode::Copy,
                            cx,
                        ))
                        .child(choice_button(
                            "import-mode-move",
                            "移动（源删除）".into(),
                            mode == ImportMode::Move,
                            |state| state.import.mode = ImportMode::Move,
                            cx,
                        )),
                ),
        )
        .into_any_element()
}

/// ④ 干跑计划预览 + 跳过清单 + 执行进度 + 结果。
fn render_plan_card(state: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let plan = state.import.plan.clone();
    let planning = state.import.planning;
    let running = state.import.running;
    let can_plan = state.import.can_plan();

    let mut body = card(cx).child(
        h_flex()
            .w_full()
            .items_center()
            .justify_between()
            .child(card_title("计划预览", cx))
            .child(
                Button::new("import-generate-plan")
                    .small()
                    .secondary()
                    .label(if planning {
                        "生成中…"
                    } else if plan.is_some() {
                        "重新生成"
                    } else {
                        "生成计划预览"
                    })
                    .disabled(planning || running || !can_plan)
                    .on_click(cx.listener(|_state, _, _window, cx| {
                        let entity = cx.entity();
                        defer_entity_action(cx, entity, |entity, cx| {
                            import::generate_plan(entity, cx)
                        });
                    })),
            ),
    );

    if let Some(plan) = plan {
        let count: usize = plan.groups.iter().map(|g| g.files.len()).sum();
        let skipped = plan.skipped.len();
        let skipped_note = if skipped > 0 {
            format!("，跳过 {skipped} 张")
        } else {
            String::new()
        };
        body = body.child(
            div()
                .text_xs()
                .text_color(cx.theme().foreground)
                .child(format!(
                    "{count} 张 → {} 个目标目录{skipped_note}",
                    plan.groups.len()
                )),
        );
        body = body.child(
            v_flex()
                .w_full()
                .gap_1()
                .children(plan.groups.iter().take(10).map(|group| {
                    h_flex()
                        .w_full()
                        .justify_between()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(div().child(if group.sub_dir.is_empty() {
                            "（根目录）".to_string()
                        } else {
                            group.sub_dir.clone()
                        }))
                        .child(div().child(format!("{} 张", group.files.len())))
                })),
        );
        if plan.groups.len() > 10 {
            body = body.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("…等共 {} 个日期目录", plan.groups.len())),
            );
        }
        if skipped > 0 {
            body = body.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().warning)
                    .child(format!("跳过 {skipped} 张（同名已存在 / 源内冲突）：")),
            );
            body = body.child(v_flex().w_full().gap_1().children(
                plan.skipped.iter().take(20).map(|entry| {
                    let name = entry
                        .path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| entry.path.to_string_lossy().to_string());
                    h_flex()
                        .w_full()
                        .gap_2()
                        .text_xs()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_color(cx.theme().muted_foreground)
                                .child(name),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_color(cx.theme().muted_foreground)
                                .child(entry.reason.clone()),
                        )
                }),
            ));
        }
    }

    if let Some((done, total, current)) = state.import.progress.clone() {
        let pct = if total == 0 {
            0.0
        } else {
            (done as f32 / total as f32).clamp(0.0, 1.0)
        };
        let current_name = PathBuf::from(&current)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or(current);
        body = body.child(
            v_flex()
                .w_full()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("{done}/{total} · {current_name}")),
                )
                .child(
                    div()
                        .w_full()
                        .h(px(6.))
                        .rounded_full()
                        .bg(cx.theme().border)
                        .child(
                            div()
                                .h_full()
                                .rounded_full()
                                .bg(cx.theme().primary)
                                .w(gpui_kit::relative(pct)),
                        ),
                ),
        );
    }

    if let Some(result) = state.import.result {
        body = body.child(
            div()
                .text_xs()
                .text_color(cx.theme().foreground)
                .child(format!(
                    "完成：成功 {} · 跳过 {} · 失败 {}",
                    result.imported, result.skipped, result.failed
                )),
        );
    }

    body.into_any_element()
}

/// 「添加」：选目录直接打开浏览（不动文件）。
fn render_add_pane(_state: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    card(cx)
        .child(card_title("添加目录", cx))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("选择一个目录直接在网格里浏览（递归扫描子目录），不会移动或复制任何文件。"),
        )
        .child(
            Button::new("import-add-dir")
                .small()
                .secondary()
                .icon(IconName::FolderOpen)
                .label("选择目录并打开...")
                .on_click(cx.listener(|_state, _, window, cx| {
                    import::pick_add_directory(window, cx);
                })),
        )
        .into_any_element()
}

/// 底栏：流程提示 + 取消 / 开始导入。
fn render_footer(state: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let is_import = state.import.tab == ImportTab::Import;
    let running = state.import.running;
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
                .child(if is_import {
                    "选源 → 选目标 → 生成计划 → 开始导入"
                } else {
                    "只打开目录浏览，不移动任何文件"
                }),
        )
        .child(
            h_flex()
                .gap_2()
                .child(
                    Button::new("import-cancel")
                        .ghost()
                        .small()
                        .label("取消")
                        // 执行中不可关闭：导入进度是「不可关闭的进度弹窗」（手册 §9.10）
                        .disabled(running)
                        .on_click(cx.listener(|state, _, _, cx| {
                            state.active_dialog = None;
                            cx.notify();
                        })),
                )
                .when(is_import, |this| {
                    this.child(
                        Button::new("import-execute")
                            .small()
                            .primary()
                            .label(if running {
                                "导入中…".to_string()
                            } else {
                                format!("开始导入（{} 张）", state.import.plan_count())
                            })
                            .disabled(!state.import.can_execute())
                            .on_click(cx.listener(|_state, _, _window, cx| {
                                let entity = cx.entity();
                                defer_entity_action(cx, entity, |entity, cx| {
                                    import::execute(entity, cx)
                                });
                            })),
                    )
                }),
        )
        .into_any_element()
}
