use gpui::{prelude::FluentBuilder, *};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::Selectable;
use gpui_component::{h_flex, v_flex, Icon, IconName, Sizable};
use photo_tool_core::domain::{Flag, Rating};

use crate::state::app::RootView;
use crate::ui::theme;

/// Render the left sidebar: directory browser + filter controls.
pub fn render_sidebar(view: &RootView, cx: &mut Context<RootView>) -> impl IntoElement {
    v_flex()
        .size_full()
        .px_4()
        .py_3()
        .gap_2()
        .child(render_directory_section(view, cx))
        .child(div().h(px(1.)).my_2().bg(theme::colors().border_variant))
        .child(render_favorites_section(view, cx))
        .child(div().h(px(1.)).my_2().bg(theme::colors().border_variant))
        .child(render_filter_section(view, cx))
}

// ── helpers ──────────────────────────────────────────────────────────────

fn section_header(label: &str) -> Div {
    let label = label.to_string();
    div()
        .font_weight(FontWeight::BOLD)
        .text_xs()
        .text_color(theme::colors().text_muted)
        .px_3()
        .pt_4()
        .pb_1()
        .child(label)
}

// ── Directory Section ────────────────────────────────────────────────────

fn render_directory_section(
    view: &RootView,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    let path_display: String = view
        .dir_path
        .as_ref()
        .and_then(|p| p.to_str())
        .unwrap_or("未打开目录")
        .to_string();

    v_flex()
        .gap_1()
        .child(section_header("目录"))
        .child(
            Button::new("open-dir-btn")
                .icon(IconName::FolderOpen)
                .ghost()
                .label("打开目录")
                .on_click(cx.listener(|view, _event: &ClickEvent, _window, cx| {
                    if let Some(path) = rfd::FileDialog::new().pick_folder() {
                        view.scan_directory(path, cx);
                    }
                })),
        )
        .child(
            div()
                .text_xs()
                .truncate()
                .text_color(theme::colors().text_muted)
                .child(path_display),
        )
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
        .child(section_header("常用目录"))
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
                        let display = dir
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("?")
                            .to_string();
                        let dir_clone = dir.clone();
                        let id = format!("fav-{i}");
                        let vh = vh.clone();
                        Button::new(id)
                            .ghost()
                            .child(display)
                            .on_click(move |_, _window, cx| {
                                if let Some(view) = vh.upgrade() {
                                    let path = dir_clone.clone();
                                    let _ = cx.update_entity(&view, |root_view, root_cx| {
                                        root_view.scan_directory(path, root_cx);
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
        .child(section_header("筛选条件"))
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

    div()
        .bg(theme::colors().element_background)
        .border_1()
        .border_color(theme::colors().border_variant)
        .rounded_sm()
        .px_2()
        .py_1()
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_1()
                .child(
                    h_flex()
                        .gap_1()
                        .items_center()
                        .child(Icon::new(IconName::Search).small().text_color(theme::colors().text_muted)),
                )
                .child(
                    div()
                        .id(ElementId::Name("text-search-input".into()))
                        .flex_grow(1.0)
                        .h(px(24.))
                        .bg(theme::colors().element_background)
                        .rounded_sm()
                        .border_1()
                        .border_color(theme::colors().border_variant)
                        .px_1()
                        .text_xs()
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
                            .text_color(theme::colors().error)
                            .cursor_pointer()
                            .child("\u{2715}")
                            .on_click(cx.listener(|view, _event: &ClickEvent, _window, cx| {
                                view.filter.text_search = None;
                                view.apply_filter_and_sort();
                                cx.notify();
                            })),
                    )
                })
        )
}

fn render_rating_filter(
    view: &RootView,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    let current_min = view.filter.min_rating.map(|r| r as u8).unwrap_or(0);

    h_flex()
        .items_center()
        .gap_1()
        .child(
            div()
                .text_xs()
                .text_color(theme::colors().text_muted)
                .child("评分 ≥"),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .gap_px()
                .children((0..6).map(move |n| {
                    let active = n <= current_min;
                    let label = if n == 0 { "\u{2014}" } else { "\u{2605}" };
                    let star_color = if active && n > 0 {
                        theme::colors::RATING[(n - 1) as usize]
                    } else if active {
                        theme::colors().text_muted
                    } else {
                        theme::colors().text_muted
                    };
                    let id = format!("rating-filter-{n}");
                    div()
                        .id(ElementId::Name(id.into()))
                        .text_xs()
                        .text_color(if active && n > 0 { star_color } else { theme::colors().text_muted })
                        .px_1()
                        .py_0p5()
                        .rounded_sm()
                        .cursor_pointer()
                        .hover(|style| style.bg(theme::colors().element_hover))
                        .when(active && n == 0, |this| {
                            this.bg(theme::colors().border_variant)
                        })
                        .child(label)
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
                })),
        )
}

fn render_flag_filter(
    view: &RootView,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    let current = view.filter.flag_filter;
    let unflagged = view.filter.unflagged_filter;
    let vh = cx.entity().downgrade();

    h_flex()
        .items_center()
        .gap_1()
        .child(
            div()
                .text_xs()
                .text_color(theme::colors().text_muted)
                .child("旗标："),
        )
        .child({
            let active = current.is_none() && !unflagged;
            let vh = vh.clone();
            Button::new("flag-filter-any")
                .ghost()
                .selected(active)
                .child("任意")
                .on_click(move |_, _window, cx| {
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
            Button::new("flag-filter-pick")
                .ghost()
                .selected(active)
                .child("入选")
                .on_click(move |_, _window, cx| {
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
            Button::new("flag-filter-reject")
                .ghost()
                .selected(active)
                .child("淘汰")
                .on_click(move |_, _window, cx| {
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
            Button::new("flag-filter-unflagged")
                .ghost()
                .selected(active)
                .child("未标记")
                .on_click(move |_, _window, cx| {
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
