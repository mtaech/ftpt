use gpui::{prelude::FluentBuilder, *};
use std::path::PathBuf;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::{h_flex, v_flex, Icon, IconName, Sizable};
use photo_tool_core::domain::{Flag, Rating};

use crate::state::app::RootView;
use crate::ui::theme;

/// 左侧边栏 tab 名称与索引
const SIDEBAR_TABS: &[(&str, usize)] = &[("文件树", 0), ("收藏夹", 1), ("筛选", 2)];

/// Render the left sidebar with tab bar: 文件树 | 收藏夹 | 筛选
pub fn render_sidebar(view: &RootView, cx: &mut Context<RootView>) -> impl IntoElement {
    let vh = cx.entity().downgrade();
    let active_tab = view.sidebar_section;

    let content: gpui::AnyElement = match active_tab {
        1 => render_favorites_section(view, cx).into_any_element(),
        2 => render_filter_section(view, cx).into_any_element(),
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
                        .text_xs()
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
                .ghost()
                .label("打开目录")
                .on_click(cx.listener(|view, _event: &ClickEvent, _window, cx| {
                    view.pick_and_scan_directory(cx);
                })),
        )
        .child(
            // 交易终端 watchlist 风格目录行：左缘 2px accent 竖条 + accent_dim 底色
            h_flex()
                .rounded_sm()
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
                                        .text_sm()
                                        .text_color(theme::colors().text)
                                        .truncate()
                                        .child(dir_name),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(theme::colors().text_muted)
                                        .font_family(theme::MONO_FONT_FAMILY)
                                        .child(format!("{photo_count} 张")),
                                )
                                .into_any_element()
                        } else {
                            div()
                                .text_sm()
                                .text_color(theme::colors().text_muted)
                                .child("未打开目录")
                                .into_any_element()
                        }),
                ),
        )
        .children(render_recent_directories(view, cx))
}

// ── Recent Directories ──────────────────────────────────────────────────

fn render_recent_directories(
    view: &RootView,
    cx: &mut Context<RootView>,
) -> impl IntoIterator<Item = impl IntoElement> {
    let vh = cx.entity().downgrade();
    let current = view.dir_path.as_ref().map(|p| p.to_string_lossy().to_string());
    let recents: Vec<String> = view
        .config
        .recent_directories
        .iter()
        .filter(|d| Some(*d) != current.as_ref())
        .cloned()
        .collect();

    let mut elements: Vec<AnyElement> = Vec::new();
    if recents.is_empty() {
        return elements;
    }

    elements.push(
        div()
            .pt_2()
            .child(crate::ui::controls::section_header("最近打开"))
            .into_any_element(),
    );
    for dir in recents {
        let display = std::path::Path::new(&dir)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&dir)
            .to_string();
        let dir_open = dir.clone();
        let vh_open = vh.clone();
        elements.push(
            div()
                .id(ElementId::Name(SharedString::from(format!("recent-{dir}"))))
                .px_2()
                .py_0p5()
                .rounded_sm()
                .cursor_pointer()
                .hover(|style| style.bg(theme::colors().element_hover))
                .child(
                    div()
                        .text_sm()
                        .text_color(theme::colors().text_muted)
                        .truncate()
                        .child(display),
                )
                .on_click(move |_, _window, cx| {
                    if let Some(view) = vh_open.upgrade() {
                        let path = dir_open.clone();
                        let _ = cx.update_entity(&view, |root_view, root_cx| {
                            root_view.scan_directory(PathBuf::from(&path), root_cx);
                        });
                    }
                })
                .into_any_element(),
        );
    }
    elements
}

// ── Favorites Section ────────────────────────────────────────────────────

fn render_favorites_section(
    view: &RootView,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    let favs = &view.config.favorite_dirs;
    let vh = cx.entity().downgrade();

    v_flex()
        .gap_1()
        .child(crate::ui::controls::section_header("常用目录"))
        .child(
            if favs.is_empty() {
                div()
                    .text_sm()
                    .text_color(theme::colors().text_muted)
                    .child("暂无常用目录")
                    .into_any_element()
            } else {
                v_flex()
                    .gap_0p5()
                    .children(favs.iter().enumerate().map(|(i, dir)| {
                        let display = std::path::Path::new(dir)
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("?")
                            .to_string();
                        let dir_open = dir.clone();
                        let dir_remove = dir.clone();
                        let id = format!("fav-{i}");
                        let vh_open = vh.clone();
                        let vh_remove = vh.clone();

                        // 单行常用目录，hover 时右侧浮现移除按钮
                        div()
                            .id(ElementId::Name(SharedString::from(id.clone())))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_1()
                            .px_2()
                            .py_0p5()
                            .rounded_sm()
                            .hover(|style| style.bg(theme::colors().element_hover))
                            .child(
                                div()
                                    .flex_1()
                                    .text_sm()
                                    .truncate()
                                    .child(display),
                            )
                            .child(
                                // hover 时浮现的移除按钮（group_hover 控制 visible）
                                div()
                                    .id(ElementId::Name(SharedString::from(format!("fav-rm-{i}"))))
                                    .invisible()
                                    .group_hover("", |this| this.visible())
                                    .cursor_pointer()
                                    .child(
                                        Icon::new(IconName::Close)
                                            .xsmall()
                                            .text_color(theme::colors().text_muted),
                                    )
                                    .on_click(move |_event: &ClickEvent, _window, cx| {
                                        // 阻止冒泡，避免触发外层「打开目录」
                                        cx.stop_propagation();
                                        if let Some(view) = vh_remove.upgrade() {
                                            let _ = cx.update_entity(&view, |root_view, root_cx| {
                                                root_view.config.favorite_dirs.retain(|d| d != &dir_remove);
                                                root_view.save_config();
                                                root_cx.notify();
                                            });
                                        }
                                    }),
                            )
                            .on_click(move |_, _window, cx| {
                                if let Some(view) = vh_open.upgrade() {
                                    let path = dir_open.clone();
                                    let _ = cx.update_entity(&view, |root_view, root_cx| {
                                        root_view.scan_directory(PathBuf::from(&path), root_cx);
                                    });
                                }
                            })
                    }))
                    .into_any_element()
            },
        )
}


fn render_filter_section(
    view: &RootView,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    v_flex()
        .gap_2()
        .child(crate::ui::controls::section_header("筛选条件"))
        .child(render_text_search(view, cx))
        .child(render_rating_filter(view, cx))
        .child(render_flag_filter(view, cx))
}

fn render_text_search(
    view: &RootView,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    // TODO: migrate to gpui_component Input when Window access is available
    let current = view.filter.text_search.as_deref().unwrap_or("");
    let placeholder = if current.is_empty() {
        "输入名称...".to_string()
    } else {
        current.to_string()
    };
    let placeholder_color = if current.is_empty() {
        theme::colors().text_muted
    } else {
        theme::colors().text
    };
    let has_text = view.filter.text_search.is_some();

    // pill 形搜索框：rounded_full + element_background 底 + 前置搜索图标
    div()
        .bg(theme::colors().element_background)
        .border_1()
        .border_color(theme::colors().border_variant)
        .rounded_full()
        .px_3()
        .py_1()
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_1()
                .child(
                    Icon::new(IconName::Search)
                        .small()
                        .text_color(theme::colors().text_muted),
                )
                .child(
                    div()
                        .id(ElementId::Name("text-search-input".into()))
                        .flex_grow(1.0)
                        .h(px(28.))
                        .px_1()
                        .text_sm()
                        .text_color(placeholder_color)
                        .cursor_text()
                        .child(placeholder)
                        .focusable()
                        .on_key_down(
                            cx.listener(|view, event: &KeyDownEvent, _window, cx| {
                                let changed = match event.keystroke.key.as_str() {
                                    "backspace" => {
                                        let mut text =
                                            view.filter.text_search.clone().unwrap_or_default();
                                        text.pop();
                                        view.filter.text_search =
                                            if text.is_empty() { None } else { Some(text) };
                                        true
                                    }
                                    "escape" => {
                                        let had = view.filter.text_search.is_some();
                                        view.filter.text_search = None;
                                        had
                                    }
                                    _ => {
                                        if let Some(ref ch) = event.keystroke.key_char {
                                            if ch.chars().all(|c| !c.is_control()) {
                                                let mut text =
                                                    view.filter.text_search.clone().unwrap_or_default();
                                                text.push_str(ch);
                                                view.filter.text_search = Some(text);
                                                true
                                            } else {
                                                false
                                            }
                                        } else {
                                            false
                                        }
                                    }
                                };
                                if changed {
                                    view.apply_filter_and_sort();
                                    cx.notify();
                                }
                            }),
                        ),
                )
                .when(has_text, |this| {
                    this.child(
                        div()
                            .id(ElementId::Name("clear-text-search".into()))
                            .text_xs()
                            .cursor_pointer()
                            .child(Icon::new(IconName::Close).xsmall().text_color(theme::colors().error))
                            .on_click(cx.listener(|view, _event: &ClickEvent, _window, cx| {
                                view.filter.text_search = None;
                                view.apply_filter_and_sort();
                                cx.notify();
                            })),
                    )
                }),
        )
}

fn render_rating_filter(
    view: &RootView,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    let current_min = view.filter.min_rating.map(|r| r as u8).unwrap_or(0);

    // 芯片式评分筛选：横排换行小 chip
    // 选中 chip = accent 描边+文字+底，未选中 = border 描边 + muted 文字
    div()
        .flex()
        .flex_row()
        .flex_wrap()
        .gap_1()
        .children((0..6).map(move |n| {
            let chip_selected = (n == 0 && current_min == 0) || (n > 0 && current_min == n);
            let id = format!("rating-chip-{n}");

            div()
                .id(ElementId::Name(id.into()))
                .px_2()
                .py_0p5()
                .rounded_sm()
                .text_xs()
                .border_1()
                .border_color(if chip_selected {
                    theme::colors().text_accent
                } else {
                    theme::colors().border
                })
                .text_color(if chip_selected {
                    theme::colors().text_accent
                } else {
                    theme::colors().text_muted
                })
                .when(chip_selected, |this| {
                    this.bg(theme::accent_dim())
                })
                .cursor_pointer()
                .child(if n == 0 {
                    "任意".into_any_element()
                } else {
                    Icon::new(IconName::StarFill)
                        .xsmall()
                        .text_color(if chip_selected {
                            theme::colors().text_accent
                        } else {
                            theme::colors().text_muted
                        })
                        .into_any_element()
                })
                .on_click(
                    cx.listener(move |view, _event: &ClickEvent, _window, cx| {
                        view.filter.min_rating = if n == 0 {
                            None
                        } else {
                            Some(match n {
                                1 => Rating::One,
                                2 => Rating::Two,
                                3 => Rating::Three,
                                4 => Rating::Four,
                                5 => Rating::Five,
                                _ => unreachable!(),
                            })
                        };
                        view.apply_filter_and_sort();
                        cx.notify();
                    }),
                )
        }))
}

fn render_flag_filter(
    view: &RootView,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    let current = view.filter.flag_filter;
    let unflagged = view.filter.unflagged_filter;
    let vh = cx.entity().downgrade();

    // 芯片式旗标筛选：横排换行小 chip
    div()
        .flex()
        .flex_row()
        .flex_wrap()
        .gap_1()
        .child({
            let active = current.is_none() && !unflagged;
            let vh = vh.clone();
            filter_chip("flag-chip-any", "任意", active, move |_, _window, cx| {
                if let Some(view) = vh.upgrade() {
                    let _ = cx.update_entity(&view, |root_view, root_cx| {
                        root_view.filter.flag_filter = None;
                        root_view.filter.unflagged_filter = false;
                        root_view.apply_filter_and_sort();
                        root_cx.notify();
                    });
                }
            })
        })
        .child({
            let active = current == Some(Flag::Pick);
            let vh = vh.clone();
            filter_chip("flag-chip-pick", "入选", active, move |_, _window, cx| {
                if let Some(view) = vh.upgrade() {
                    let _ = cx.update_entity(&view, |root_view, root_cx| {
                        root_view.filter.flag_filter = Some(Flag::Pick);
                        root_view.filter.unflagged_filter = false;
                        root_view.apply_filter_and_sort();
                        root_cx.notify();
                    });
                }
            })
        })
        .child({
            let active = current == Some(Flag::Reject);
            let vh = vh.clone();
            filter_chip("flag-chip-reject", "淘汰", active, move |_, _window, cx| {
                if let Some(view) = vh.upgrade() {
                    let _ = cx.update_entity(&view, |root_view, root_cx| {
                        root_view.filter.flag_filter = Some(Flag::Reject);
                        root_view.filter.unflagged_filter = false;
                        root_view.apply_filter_and_sort();
                        root_cx.notify();
                    });
                }
            })
        })
        .child({
            let active = unflagged;
            let vh = vh.clone();
            filter_chip("flag-chip-unflagged", "未标记", active, move |_, _window, cx| {
                if let Some(view) = vh.upgrade() {
                    let _ = cx.update_entity(&view, |root_view, root_cx| {
                        root_view.filter.flag_filter = None;
                        root_view.filter.unflagged_filter = true;
                        root_view.apply_filter_and_sort();
                        root_cx.notify();
                    });
                }
            })
        })
}

/// 交易终端风格筛选 chip：小圆角、1px 描边、text_xs。
/// 选中态 = accent 描边 + accent 文字 + accent_dim 底色，
/// 未选中 = border 描边 + muted 文字。
fn filter_chip(
    id: impl Into<ElementId>,
    label: &str,
    selected: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .px_2()
        .py_0p5()
        .rounded_sm()
        .text_xs()
        .border_1()
        .border_color(if selected {
            theme::colors().text_accent
        } else {
            theme::colors().border
        })
        .text_color(if selected {
            theme::colors().text_accent
        } else {
            theme::colors().text_muted
        })
        .when(selected, |this| {
            this.bg(theme::accent_dim())
        })
        .cursor_pointer()
        .child(label.to_string())
        .on_click(on_click)
}
