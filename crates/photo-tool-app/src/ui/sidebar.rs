use gpui::{prelude::FluentBuilder, *};
use std::path::PathBuf;
use gpui_component::button::Button;
use gpui_component::menu::ContextMenuExt as _;
use gpui_component::{h_flex, v_flex, Icon, IconName, Sizable};

use crate::state::app::RootView;
use crate::ui::theme;

/// 左侧边栏 tab 名称与索引（收藏夹已并入文件树 tab）
const SIDEBAR_TABS: &[(&str, usize)] = &[("文件树", 0), ("文件操作", 1)];

/// Render the left sidebar with tab bar: 文件树 | 筛选 | 文件操作
pub fn render_sidebar(view: &RootView, cx: &mut Context<RootView>) -> impl IntoElement {
    let vh = cx.entity().downgrade();
    let active_tab = view.sidebar_section;

    let content: gpui::AnyElement = match active_tab {
        1 => crate::ui::batch_ops::render_batch_ops_section(view, cx).into_any_element(),
        _ => render_directory_section(view, cx).into_any_element(),
    };

    v_flex()
        .size_full()
        .child(
            // ── Tab 栏 ──
            h_flex()
                .gap_0()
                .px_2()
                .py_1()
                .border_b_1()
                .border_color(theme::colors().border_variant)
                .children(SIDEBAR_TABS.iter().map(|&(name, idx)| {
                    let vh = vh.clone();
                    let is_active = active_tab == idx;
                    div()
                        .id(SharedString::from(name))
                        .flex()
                        .flex_1()
                        .items_center()
                        .justify_center()
                        .py_1()
                        .px_2()
                        .rounded_sm()
                        
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(if is_active {
                            theme::colors().text
                        } else {
                            theme::colors().text_muted
                        })
                        .bg(if is_active {
                            theme::colors().element_hover
                        } else {
                            hsla(0., 0., 0., 0.)
                        })
                        .cursor_pointer()
                        .on_click(move |_, _window, cx| {
                            if let Some(e) = vh.upgrade() {
                                cx.update_entity(&e, |view, cx| {
                                    view.sidebar_section = idx;
                                    cx.notify();
                                });
                            }
                        })
                        .child(name)
                })),
        )
        // ── 内容区 ──
        .child(
            div()
                .flex_1()
                .px_3()
                .py_2()
                .child(content),
        )
}

// ── Directory Section ────────────────────────────────────────────────────

fn render_directory_section(
    view: &RootView,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    // 目录名（取最后一段）
    let dir_name: String = view
        .dir_path
        .as_ref()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
        .unwrap_or_default();
    let has_dir = view.dir_path.is_some();
    let photo_count = view.captures.len();

    v_flex()
        .gap_1()
        .child(crate::ui::controls::section_header("目录"))
        .child(
            Button::new("open-dir-btn")
                .icon(IconName::FolderOpen)
                .outline()
                .w_full()
                .label("打开目录")
                .on_click(cx.listener(|view, _event: &ClickEvent, _window, cx| {
                    view.pick_and_scan_directory(cx);
                })),
        )
        .child(
            // 交易终端 watchlist 风格目录行：左缘 2px accent 竖条 + accent_dim 底色
            // 边框遵循卡片规范（theme::card），高亮态仅覆盖 bg
            h_flex()
                .rounded_md()
                .border_1()
                .border_color(theme::colors().border_variant)
                .bg(theme::accent_dim())
                .child(
                    // 左侧 2px accent 竖条（替代整行亮色选中态）
                    div()
                        .w(px(2.))
                        .bg(theme::colors().text_accent)
                        .flex_none(),
                )
                .child(
                    div()
                        .flex_1()
                        .px_2()
                        .py_1()
                        .child(if has_dir {
                            // 双行：上行目录名，下行照片计数（等宽字体）
                            v_flex()
                                .child(
                                    div()
                                        
                                        .text_color(theme::colors().text)
                                        .truncate()
                                        .child(dir_name),
                                )
                                .child(
                                    div()
                                        
                                        .text_color(theme::colors().text_muted)
                                        .font_family(theme::MONO_FONT_FAMILY)
                                        .child(format!("{photo_count} 张")),
                                )
                                .into_any_element()
                        } else {
                            div()
                                
                                .text_color(theme::colors().text_muted)
                                .child("未打开目录")
                                .into_any_element()
                        }),
                ),
        )
        .child(render_folder_groups(view, cx))
}

// ── 收藏与最近打开（合并文件树 tab）─────────────────────────────────────

fn render_folder_groups(
    view: &RootView,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    let vh = cx.entity().downgrade();
    let current = view.dir_path.as_ref().map(|p| p.to_string_lossy().to_string());
    // 收藏置前；最近打开排除当前目录与已收藏（同一目录只出现一次）
    let favs: Vec<String> = view
        .config
        .favorite_dirs
        .iter()
        .filter(|d| Some(*d) != current.as_ref())
        .cloned()
        .collect();
    let recents: Vec<String> = view
        .config
        .recent_directories
        .iter()
        .filter(|d| Some(*d) != current.as_ref())
        .filter(|d| !favs.contains(*d))
        .cloned()
        .collect();

    // 分区之间：分隔线 + pt_3 拉开层次（border_variant 是设计系统的弱边框色）
    let divider = || {
        div()
            .h(px(1.))
            .w_full()
            .bg(theme::colors().border_variant)
    };

    v_flex()
        .gap_1()
        .child(div().pt_3().child(divider()))
        .child(crate::ui::controls::section_header("收藏"))
        .child(if favs.is_empty() {
            div()
                
                .text_color(theme::colors().text_muted)
                .child("右键文件夹卡片可加入收藏")
                .into_any_element()
        } else {
            v_flex()
                .gap_0p5()
                .children(favs.iter().enumerate().map(|(i, dir)| {
                    folder_card(format!("fav-{i}"), dir, true, vh.clone())
                }))
                .into_any_element()
        })
        .child(div().pt_3().child(divider()))
        .child(crate::ui::controls::section_header("最近打开"))
        .child(if recents.is_empty() {
            div()
                
                .text_color(theme::colors().text_muted)
                .child("暂无历史记录")
                .into_any_element()
        } else {
            v_flex()
                .gap_0p5()
                .children(recents.iter().enumerate().map(|(i, dir)| {
                    folder_card(format!("recent-{i}"), dir, false, vh.clone())
                }))
                .into_any_element()
        })
}

/// 文件夹卡片：双行（目录名 + 完整路径），收藏带星标。
/// 左键打开目录；右键菜单「加入/取消收藏」「从列表移除」。
fn folder_card(
    id: String,
    dir: &str,
    is_fav: bool,
    vh: gpui::WeakEntity<RootView>,
) -> AnyElement {
    let display = std::path::Path::new(dir)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(dir)
        .to_string();
    let path_text = dir.to_string();
    let dir_open = dir.to_string();
    let dir_menu = dir.to_string();
    let dir_tip = dir.to_string();
    let vh_open = vh.clone();
    let vh_menu = vh;

    theme::card(
        div()
            .id(ElementId::Name(SharedString::from(id)))
            .flex()
            .flex_row()
            .items_center(),
    )
        .cursor_pointer()
        .hover(|style| style.bg(theme::colors().element_hover))
        // 路径截断后通过 tooltip 展示完整路径
        .tooltip(move |window, cx| {
            gpui_component::tooltip::Tooltip::new(dir_tip.clone()).build(window, cx)
        })
        .child(
            div()
                .flex_1()
                .min_w_0()
                .px_2()
                .py_1()
                .child(
                    v_flex()
                        .w_full()
                        .child(
                            h_flex()
                                .w_full()
                                .gap_1()
                                .items_center()
                                .when(is_fav, |d| {
                                    d.child(
                                        Icon::new(IconName::StarFill)
                                            .xsmall()
                                            .text_color(theme::colors().text_accent),
                                    )
                                })
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .text_color(theme::colors().text)
                                        .truncate()
                                        .child(display),
                                ),
                        )
                        .child(
                            div()
                                .w_full()
                                .text_color(theme::colors().text_muted)
                                .font_family(theme::MONO_FONT_FAMILY)
                                .truncate()
                                .child(path_text),
                        ),
                ),
        )
        .on_click(move |_, _window, cx| {
            if let Some(view) = vh_open.upgrade() {
                let path = dir_open.clone();
                let _ = cx.update_entity(&view, |root_view, root_cx| {
                    root_view.scan_directory(PathBuf::from(&path), root_cx);
                });
            }
        })
        .context_menu(move |menu, _window, cx| {
            // 右键先记录目标目录，菜单命令统一作用于 folder_menu_dir
            if let Some(view) = vh_menu.upgrade() {
                let _ = cx.update_entity(&view, |root_view, _cx| {
                    root_view.folder_menu_dir = Some(dir_menu.clone());
                });
            }
            crate::ui::context_menu::folder_menu(menu, is_fav)
        })
        .into_any_element()
}


