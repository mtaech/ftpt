//! 左栏组件：文件树与批量操作（对应 §9.3 与 §10.4）。

use std::path::PathBuf;

use gpui_kit::assets::IconName as SharedIconName;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    tag::Tag,
    v_flex,
};
use gpui_kit::{Context, IntoElement, SharedString, div, prelude::*, px};

use crate::model::filter::has_active_filters;
use crate::state::engine_ops::{defer_entity_action, pick_batch_target_dir, start_scan};
use crate::state::{ActiveDialog, AppState};

/// 目录列表里的一行（子目录 / 收藏 / 最近打开共用）。
///
/// 刻意不用 Button 铺满整行：gpui-component 的 Button 内容默认**居中**，
/// 名字长短不一（"鸟" vs "2026-09-06"）时看起来就像缩进错乱的树。
/// 这里左对齐 + 统一图标槽 + hover 底色 + 当前目录高亮。
fn render_dir_row(
    id: String,
    dir: PathBuf,
    icon: SharedIconName,
    show_parent: bool,
    current: bool,
    cx: &mut Context<AppState>,
) -> impl IntoElement + use<> {
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| dir.to_string_lossy().to_string());
    let parent_name = dir
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let full_path = dir.to_string_lossy().to_string();

    let hover_bg = cx.theme().accent;
    let highlight = cx.theme().selection;
    let foreground = cx.theme().foreground;
    let muted = cx.theme().muted_foreground;
    let accent = cx.theme().primary;

    h_flex()
        .id(SharedString::from(id))
        .w_full()
        .items_center()
        .gap_2()
        .px_2p5()
        .py_1p5()
        .rounded(px(8.))
        .cursor_pointer()
        .aria_label(SharedString::from(full_path))
        .when(current, |this| {
            this.bg(highlight)
                .border_1()
                .border_color(accent.opacity(0.3))
        })
        .when(!current, |this| this.hover(move |s| s.bg(hover_bg)))
        // 当前选中时左侧的 Material You 垂直小指示条
        .when(current, |this| {
            this.child(div().w(px(3.)).h(px(14.)).rounded_full().bg(accent))
        })
        .child(
            Icon::new(icon)
                .size(px(14.))
                .text_color(if current { accent } else { muted }),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_xs()
                .when(current, |this| this.font_medium().text_color(accent))
                .when(!current, |this| this.text_color(foreground))
                .child(name),
        )
        // 右侧弱化的上级目录名：同名文件夹（多个"图片"）靠它区分
        .when(show_parent && !parent_name.is_empty(), |this| {
            this.child(
                div()
                    .flex_shrink_0()
                    .max_w(px(76.))
                    .truncate()
                    .text_size(px(10.))
                    .text_color(muted)
                    .child(parent_name),
            )
        })
        .on_click(cx.listener(move |_state, _event, _window, cx| {
            // 点击时 AppState 正被租借，不能同步 update；排到 effect 之后
            let dir = dir.clone();
            let entity = cx.entity().clone();
            defer_entity_action(cx, entity, move |entity, cx| {
                // 与 F5 / 打开目录同一口径：是否递归由配置决定
                let recursive = entity.read(cx).app_config.include_subdirectories;
                start_scan(entity, dir, recursive, cx);
            });
        }))
}

pub fn render_file_tree_tab(
    state: &AppState,
    cx: &mut Context<AppState>,
) -> impl IntoElement + use<> {
    let dir_name = state
        .current_dir
        .as_ref()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "未打开目录".to_string());

    v_flex()
        .w_full()
        .flex_1()
        .gap_3()
        // 导入按钮（精致副按钮卡片）。刻意不写「从 SD 卡」——卡/外接盘/普通目录
        // 都是它的来源，「单纯打开一个文件夹看看」也走这里（弹窗左栏的「仅添加并浏览」）
        .child(
            Button::new("btn-import-photos")
                .secondary()
                .small()
                .w_full()
                .icon(IconName::ArrowDown)
                .label("导入照片...")
                .on_click(cx.listener(|state, _, window, cx| {
                    state.open_import_dialog(window, cx);
                })),
        )
        // 当前目录卡片（dir-card-active：accent-dim 底 + 左缘 2px accent 竖条，§6.7）
        .child(
            v_flex()
                .w_full()
                .relative()
                .p_3()
                .pl(px(10.))
                .rounded(cx.theme().radius)
                .bg(crate::theme::accent_dim(cx))
                .border_1()
                .border_color(cx.theme().border)
                .gap_1p5()
                .child(
                    div()
                        .absolute()
                        .left_0()
                        .top_1()
                        .bottom_1()
                        .w(px(2.))
                        .rounded(cx.theme().radius)
                        .bg(cx.theme().primary),
                )
                .child(
                    h_flex()
                        .items_center()
                        .justify_between()
                        .child(
                            h_flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    Icon::new(IconName::Folder)
                                        .size(px(14.))
                                        .text_color(cx.theme().foreground),
                                )
                                .child(
                                    div()
                                        .font_semibold()
                                        .text_xs()
                                        .text_color(cx.theme().foreground)
                                        .truncate()
                                        .child(dir_name),
                                ),
                        )
                        .child(
                            Tag::secondary()
                                .small()
                                .child(format!("{} 张", state.items.len())),
                        ),
                )
                .when_some(state.current_dir.as_ref(), |this, dir| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .truncate()
                            .child(dir.to_string_lossy().to_string()),
                    )
                }),
        )
        // 子目录卡片
        .when(!state.subdirs.is_empty(), |this| {
            let subdirs = state.subdirs.clone();
            this.child(
                crate::theme::section(cx)
                    .gap_1()
                    .child(
                        div()
                            .px_2()
                            .py_0p5()
                            .text_size(px(11.))
                            .font_medium()
                            .text_color(cx.theme().muted_foreground)
                            .child("子目录"),
                    )
                    .children(subdirs.into_iter().map(|dir| {
                        let current = state.current_dir.as_deref() == Some(dir.as_path());
                        render_dir_row(
                            format!("subdir-{}", dir.to_string_lossy()),
                            dir,
                            SharedIconName::Folder,
                            false,
                            current,
                            cx,
                        )
                    })),
            )
        })
        // 收藏夹卡片
        .when(!state.favorite_dirs.is_empty(), |this| {
            let favs = state.favorite_dirs.clone();
            this.child(
                crate::theme::section(cx)
                    .gap_1()
                    .child(
                        div()
                            .px_2()
                            .py_0p5()
                            .text_size(px(11.))
                            .font_medium()
                            .text_color(cx.theme().muted_foreground)
                            .child("收藏目录"),
                    )
                    .children(favs.into_iter().map(|dir| {
                        let current = state.current_dir.as_deref() == Some(dir.as_path());
                        render_dir_row(
                            format!("fav-{}", dir.to_string_lossy()),
                            dir,
                            SharedIconName::Star,
                            true,
                            current,
                            cx,
                        )
                    })),
            )
        })
        // 最近打开卡片
        .when(!state.recent_dirs.is_empty(), |this| {
            let recents: Vec<PathBuf> = state.recent_dirs.iter().take(5).cloned().collect();
            this.child(
                crate::theme::section(cx)
                    .gap_1()
                    .child(
                        div()
                            .px_2()
                            .py_0p5()
                            .text_size(px(11.))
                            .font_medium()
                            .text_color(cx.theme().muted_foreground)
                            .child("最近打开"),
                    )
                    .children(recents.into_iter().map(|dir| {
                        let current = state.current_dir.as_deref() == Some(dir.as_path());
                        render_dir_row(
                            format!("recent-{}", dir.to_string_lossy()),
                            dir,
                            SharedIconName::Clock,
                            true,
                            current,
                            cx,
                        )
                    })),
            )
        })
        // 底部重复照片检测入口
        .child(div().flex_1())
        .child(
            Button::new("btn-find-duplicates")
                .ghost()
                .small()
                .w_full()
                .icon(IconName::Copy)
                .label("查找近重复照片...")
                .on_click(cx.listener(|state, _, _, cx| {
                    // 走统一入口：先把上次落盘的结果读回来，再开窗
                    state.open_duplicates_dialog(cx);
                })),
        )
}

pub fn render_batch_ops_tab(
    state: &AppState,
    cx: &mut Context<AppState>,
) -> impl IntoElement + use<> {
    let has_filter = has_active_filters(&state.criteria);
    // 无筛选时默认锁住批量操作（防误操作全目录）；用户在下面「显式确认」后解锁（§13 逃生门）
    let unlocked = has_filter || state.batch_ignore_filter;
    let total_count = state.items.len();
    let target_count = state.display_order.len();

    v_flex()
        .w_full()
        .flex_1()
        .gap_3()
        // 筛选驱动提示
        .when(!has_filter, |this| {
            this.child(
                v_flex()
                    .p_3()
                    .rounded(cx.theme().radius)
                    .bg(cx.theme().muted.opacity(0.5))
                    .border_1()
                    .border_color(cx.theme().border)
                    .gap_1p5()
                    .child(
                        h_flex()
                            .items_center()
                            .gap_1p5()
                            .child(
                                Icon::new(IconName::TriangleAlert)
                                    .size(px(14.))
                                    .text_color(cx.theme().warning),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .font_medium()
                                    .text_color(cx.theme().warning)
                                    .child("安全限制：需激活筛选"),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(
                                "批量操作仅作用于当前筛选结果。无筛选时按钮禁用以防误操作全目录。",
                            ),
                    )
                    .child(
                        Button::new("btn-batch-unlock")
                            .danger()
                            .small()
                            .w_full()
                            .label(format!("我确认，对全目录 {total_count} 项执行"))
                            .on_click(cx.listener(|state, _, _, cx| {
                                state.batch_ignore_filter = true;
                                cx.notify();
                            })),
                    ),
            )
        })
        // 已显式确认：把「没有筛选保护」这件事一直摆在脸上，并留一键收回
        .when(!has_filter && state.batch_ignore_filter, |this| {
            this.child(
                v_flex()
                    .p_3()
                    .rounded(cx.theme().radius)
                    .bg(cx.theme().danger.opacity(0.12))
                    .border_1()
                    .border_color(cx.theme().danger.opacity(0.5))
                    .gap_1p5()
                    .child(
                        div()
                            .text_xs()
                            .font_medium()
                            .text_color(cx.theme().danger)
                            .child(format!("已解除筛选保护：将对全目录 {total_count} 项执行")),
                    )
                    .child(
                        Button::new("btn-batch-relock")
                            .ghost()
                            .small()
                            .w_full()
                            .label("恢复筛选保护")
                            .on_click(cx.listener(|state, _, _, cx| {
                                state.batch_ignore_filter = false;
                                cx.notify();
                            })),
                    ),
            )
        })
        .when(has_filter, |this| {
            this.child(
                crate::theme::section(cx)
                    .text_xs()
                    .text_color(cx.theme().primary)
                    .child(format!("将对当前筛选出的 {target_count} 项执行操作")),
            )
        })
        // 最近一次批量操作结果（常驻：状态栏那条会被随后的「扫描完成…」顶掉）
        .when_some(state.batch_result.clone(), |this, msg| {
            this.child(
                crate::theme::section(cx)
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(msg),
            )
        })
        // 批量操作按钮卡片
        .child(
            crate::theme::section(cx)
                .gap_2()
                .child(
                    Button::new("btn-batch-copy")
                        .secondary()
                        .w_full()
                        .disabled(!unlocked)
                        .icon(IconName::Copy)
                        .label(format!("复制到目录 ({target_count} 项)..."))
                        .on_click(cx.listener(|_state, _, window, cx| {
                            // 先选目录再确认：确认框里显示的必须是真正要写进去的目录
                            pick_batch_target_dir(
                                photo_domain::BatchOpType::Copy,
                                window,
                                cx,
                            );
                        })),
                )
                .child(
                    Button::new("btn-batch-move")
                        .secondary()
                        .w_full()
                        .disabled(!unlocked)
                        .icon(IconName::FolderOpen)
                        .label(format!("移动到目录 ({target_count} 项)..."))
                        .on_click(cx.listener(|_state, _, window, cx| {
                            pick_batch_target_dir(
                                photo_domain::BatchOpType::Move,
                                window,
                                cx,
                            );
                        })),
                )
                .child(
                    Button::new("btn-batch-delete")
                        .danger()
                        .w_full()
                        .disabled(!unlocked)
                        .icon(IconName::Delete)
                        .label(format!("移至回收站 ({target_count} 项)..."))
                        .on_click(cx.listener(|state, _, _, cx| {
                            state.active_dialog = Some(ActiveDialog::BatchConfirm(
                                photo_domain::BatchOpType::Delete,
                            ));
                            cx.notify();
                        })),
                )
                // 批量重命名：同样作用于当前筛选/可见顺序
                .child(
                    Button::new("btn-batch-rename")
                        .secondary()
                        .w_full()
                        .disabled(!unlocked)
                        .icon(IconName::FileText)
                        .label(format!("批量重命名 ({target_count} 项)..."))
                        .on_click(cx.listener(|state, _, window, cx| {
                            state.open_rename_dialog(window, cx);
                        })),
                )
                // 导出不依赖「必须存在激活筛选」：未选 = 当前筛选结果全部
                .child(
                    Button::new("btn-batch-export")
                        .secondary()
                        .w_full()
                        .icon(IconName::ExternalLink)
                        .label("导出...")
                        .disabled(state.items.is_empty() || state.is_exporting)
                        .on_click(cx.listener(|state, _, window, cx| {
                            state.open_export_dialog(window, cx);
                        })),
                ),
        )
}
