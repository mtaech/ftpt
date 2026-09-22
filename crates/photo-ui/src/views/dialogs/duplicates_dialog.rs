//! 相似与重复照片检测弹窗（对应 §9.10 与 §10.5）。
//!
//! 基于 dHash 感知哈希（引擎 `photo-engine/src/phash.rs`）+ 汉明距离贪心聚类，支持
//! 相似分组浏览与「保留首张」的一键清理。
//!
//! 三态（与导出弹窗同一套规格）：
//! 1. **参数/结果态**：阈值档位 + 开始检测；已有结果时直接列分组（结果落 `folder_db`）；
//! 2. **进度态**：`n/m` + 细进度条 + 取消；
//! 3. **分组态**：每组缩略图（首张标「保留」）+ 「保留首张，其余移入回收站」二次确认。
//!
//! 以前这里是一个只 `set_status_message("正在进行相似照片检测...")` 就关窗的假按钮
//! （docs/todo.md #1b），Batch 1 把 #2 接线时一并修掉。
//!
//! **作用域说明**：检测当前目录**全部照片**（与筛选栏无关），但剔除「同 stem 不同格式」
//! 的多格式组——它们是同一画面的 RAW/JPEG 对（见 `model::duplicates` 文档）。

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    tag::Tag,
    v_flex,
};
use gpui_kit::{AnyElement, Context, IntoElement, Window, div, img, prelude::*, px};

use photo_domain::Flag;

use crate::image::{THUMB_SIZE_GRID, source_file_of};
use crate::model::duplicates::{DuplicateGroup, THRESHOLD_OPTIONS, group_views, summarize};
use crate::state::AppState;
use crate::state::engine_ops::{
    defer_entity_action, delete_duplicate_extras, set_flag_for_paths, start_duplicates,
};

pub fn render_duplicates_dialog(
    state: &AppState,
    _window: &mut Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let groups = group_views(&state.dup_groups);
    let (group_count, extra_count) = summarize(&state.dup_groups);
    let detecting = state.is_detecting_duplicates;

    let mut body: Vec<AnyElement> = Vec::new();
    if detecting {
        let total = state.dup_total.max(1);
        let done = state.dup_done.min(total);
        let pct = done as f32 / total as f32;
        let cancel = state.dup_cancel.clone();
        body.push(
            v_flex()
                .w_full()
                .gap_2()
                .child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().foreground)
                                .child(format!("正在计算感知哈希… {done}/{total}")),
                        )
                        .child(div().flex_1())
                        .child(
                            Button::new("dup-cancel-run")
                                .ghost()
                                .xsmall()
                                .icon(IconName::Close)
                                .label("取消")
                                .on_click(move |_, _, _| {
                                    cancel.store(true, Ordering::Relaxed);
                                }),
                        ),
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
                )
                .into_any_element(),
        );
    } else if groups.is_empty() {
        body.push(
            v_flex()
                .w_full()
                .flex_1()
                .items_center()
                .justify_center()
                .gap_3()
                .child(
                    Icon::new(IconName::Copy)
                        .size(px(32.))
                        .text_color(cx.theme().muted_foreground),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("尚未检测。基于 dHash 感知哈希 + 汉明距离聚类，找出内容重复/高度相似的照片。"),
                )
                .into_any_element(),
        );
    } else {
        for (i, group) in groups.iter().enumerate() {
            body.push(render_group_card(state, i, group, cx));
        }
    }

    // 汇总行：只在有结果时显示（含落库时间）
    let summary_line = if detecting || groups.is_empty() {
        None
    } else {
        Some(format!(
            "{} 组 · {} 张多余 · 阈值 {} · {}{}",
            group_count,
            extra_count,
            state.dup_threshold,
            state
                .dup_computed_at
                .as_deref()
                .map(|t| t.replace('T', " ").chars().take(19).collect::<String>())
                .unwrap_or_else(|| "未落库".to_string()),
            if state.items.is_empty() {
                " · 当前目录没有照片".to_string()
            } else {
                String::new()
            },
        ))
    };

    div()
        .id("duplicates-modal-overlay")
        .occlude()
        .absolute()
        .inset_0()
        .bg(gpui_kit::rgba(0x00000088))
        .flex()
        .items_center()
        .justify_center()
        .child(
            v_flex()
                .w(px(760.))
                .h(px(560.))
                .bg(cx.theme().sidebar)
                .rounded(px(20.))
                .border_1()
                .border_color(cx.theme().border.opacity(0.6))
                .shadow(crate::theme::overlay_shadow())
                .overflow_hidden()
                // ── 标题栏 ──
                .child(
                    h_flex()
                        .w_full()
                        .h(px(48.))
                        .flex_shrink_0()
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
                                .child("相似与重复照片检测"),
                        )
                        .child(
                            Button::new("close-duplicates")
                                .ghost()
                                .small()
                                .icon(IconName::Close)
                                .tooltip("关闭 (Esc)")
                                .on_click(cx.listener(|state, _, _, cx| {
                                    state.active_dialog = None;
                                    cx.notify();
                                })),
                        ),
                )
                // ── 工具条：阈值档位 + 开始检测 ──
                .child(
                    h_flex()
                        .w_full()
                        .flex_shrink_0()
                        .items_center()
                        .gap_2()
                        .px_4()
                        .py_2()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child("阈值"),
                        )
                        .children(THRESHOLD_OPTIONS.iter().map(|&opt| {
                            let selected = state.dup_threshold == opt;
                            Button::new(("dup-threshold", opt as usize))
                                .small()
                                .when(selected, |b| b.primary())
                                .when(!selected, |b| b.ghost())
                                .label(format!("{opt}"))
                                .tooltip("汉明距离阈值：越小越严（误报少、漏报多）")
                                .on_click(cx.listener(move |state, _, _, cx| {
                                    state.dup_threshold = opt;
                                    cx.notify();
                                }))
                        }))
                        .child(div().flex_1())
                        .child(
                            Button::new("run-duplicates-detect")
                                .small()
                                .secondary()
                                .icon(IconName::Asterisk)
                                .label(if detecting { "检测中…" } else { "开始检测当前目录" })
                                .disabled(detecting || state.items.is_empty())
                                .tooltip(if state.items.is_empty() {
                                    "当前目录没有照片"
                                } else {
                                    "对当前目录全部照片计算 dHash 并聚类（同 stem 不同格式视为同一张）"
                                })
                                .on_click(cx.listener(|_, _, _, cx| {
                                    let entity = cx.entity().clone();
                                    defer_entity_action(cx, entity, |entity, cx| {
                                        start_duplicates(entity, cx)
                                    });
                                })),
                        ),
                )
                // ── 汇总行 ──
                .when_some(summary_line, |this, line| {
                    this.child(
                        div()
                            .w_full()
                            .flex_shrink_0()
                            .px_4()
                            .py_2()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(line),
                    )
                })
                // ── 主体：分组列表（带滚动条） ──
                .child(
                    v_flex()
                        .w_full()
                        .flex_1()
                        .p_4()
                        .gap_2()
                        .child(
                            crate::views::scroll_area::scroll_area_v(
                                "duplicates-scroll",
                                &state.dup_scroll,
                                v_flex().w_full().gap_2().children(body),
                            )
                            .w_full()
                            .flex_1(),
                        ),
                ),
        )
}

/// 单组卡片：首张缩略图标「保留」，其余为待处理项；右上角一键清理（带二次确认）。
fn render_group_card(
    state: &AppState,
    index: usize,
    group: &DuplicateGroup,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let mut members: Vec<AnyElement> = Vec::with_capacity(group.len());
    members.push(member_tile(state, &group.keeper, true, index * 100, cx));
    for (j, path) in group.extras.iter().enumerate() {
        members.push(member_tile(state, path, false, index * 100 + j + 1, cx));
    }

    let pending = state.dup_pending_group == Some(index);
    let extras: Vec<String> = group.extras.clone();
    let extra_paths: Vec<PathBuf> = extras.iter().map(PathBuf::from).collect();
    let extra_count = extras.len();

    let actions: AnyElement = if pending {
        h_flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .child(format!("确认把 {extra_count} 张移入回收站？")),
            )
            .child(
                Button::new(("dup-cancel-confirm", index))
                    .ghost()
                    .small()
                    .label("取消")
                    .on_click(cx.listener(move |state, _, _, cx| {
                        state.dup_pending_group = None;
                        cx.notify();
                    })),
            )
            .child(
                Button::new(("dup-confirm-delete", index))
                    .small()
                    .danger()
                    .label("确认移入回收站")
                    .on_click(cx.listener(move |state, _, _, cx| {
                        state.dup_pending_group = None;
                        cx.notify();
                        // listener 内 AppState 已租借：引擎调用必须 defer 到 effect 之后
                        let entity = cx.entity().clone();
                        let paths = extra_paths.clone();
                        defer_entity_action(cx, entity, move |entity, cx| {
                            delete_duplicate_extras(entity, paths, cx);
                        });
                    })),
            )
            .into_any_element()
    } else {
        let reject_paths: Vec<String> = extras.clone();
        h_flex()
            .items_center()
            .gap_2()
            .child(
                // 非破坏性路径（手册 §9.10 的原始规格）：其余标 Rejected，文件不动
                Button::new(("dup-mark-rejected", index))
                    .small()
                    .ghost()
                    .label(format!("其余标 Rejected ({extra_count})"))
                    .tooltip("只改旗标，不动文件；之后可在筛选栏按旗标复查")
                    .on_click(cx.listener(move |_state, _, _, cx| {
                        let entity = cx.entity().clone();
                        let paths = reject_paths.clone();
                        defer_entity_action(cx, entity, move |entity, cx| {
                            entity.update(cx, |state, _| {
                                set_flag_for_paths(state, &paths, Some(Flag::Reject));
                            });
                        });
                    })),
            )
            .child(
                Button::new(("dup-ask-delete", index))
                    .small()
                    .danger()
                    .label(format!("保留首张，其余移入回收站 ({extra_count})"))
                    .on_click(cx.listener(move |state, _, _, cx| {
                        state.dup_pending_group = Some(index);
                        cx.notify();
                    })),
            )
            .into_any_element()
    };

    v_flex()
        .id(("dup-group", index))
        .w_full()
        .flex_shrink_0()
        .p_3()
        .gap_2()
        .rounded(px(14.))
        .bg(cx.theme().popover)
        .border_1()
        .border_color(cx.theme().border.opacity(0.6))
        .child(
            h_flex()
                .w_full()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .font_medium()
                        .text_color(cx.theme().foreground)
                        .child(format!("第 {} 组", index + 1)),
                )
                .child(Tag::secondary().small().child(format!("{} 张", group.len())))
                .child(div().flex_1())
                .child(actions),
        )
        .child(h_flex().w_full().flex_wrap().gap_2().children(members))
        .into_any_element()
}

/// 组内单张缩略图：优先用已生成的缩略图缓存，未生成显示文件名占位（不在弹窗里触发生成）。
fn member_tile(
    state: &AppState,
    path: &str,
    keeper: bool,
    key: usize,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let size = if keeper { 84.0 } else { 68.0 };
    let file_label = PathBuf::from(path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string());
    let tip = path.to_string();
    let border = if keeper {
        cx.theme().primary
    } else {
        cx.theme().border.opacity(0.5)
    };
    let muted = cx.theme().muted;
    let thumb = state
        .items
        .iter()
        .find(|m| m.primary_path == path)
        .and_then(source_file_of)
        .and_then(|source| {
            state
                .image_manager
                .get_cached_thumb_path(&source, THUMB_SIZE_GRID)
        });

    let tile = v_flex()
        .w(px(size))
        .gap_1()
        .flex_shrink_0()
        .child(
            div()
                .id(("dup-member-img", key))
                .relative()
                .w(px(size))
                .h(px(size))
                .flex_shrink_0()
                .rounded(px(8.))
                .border_2()
                .border_color(border)
                .bg(cx.theme().background)
                .overflow_hidden()
                .tooltip(move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(tip.clone()).build(window, cx)
                })
                .when(keeper, |this| {
                    this.child(
                        div()
                            .absolute()
                            .bottom_0()
                            .left_0()
                            .px_1()
                            .bg(cx.theme().primary)
                            .text_color(cx.theme().primary_foreground)
                            .text_xs()
                            .child("保留"),
                    )
                })
                .child(match thumb {
                    Some(thumb) => img(thumb)
                        .size_full()
                        .object_fit(gpui_kit::ObjectFit::Cover)
                        .into_any_element(),
                    None => v_flex()
                        .size_full()
                        .items_center()
                        .justify_center()
                        .p_1()
                        .bg(muted)
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(file_label.clone()),
                        )
                        .into_any_element(),
                }),
        )
        .child(
            div()
                .w(px(size))
                .truncate()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(file_label),
        );

    tile.into_any_element()
}
