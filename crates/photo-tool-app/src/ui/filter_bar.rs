use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_component::button::{Button, ButtonVariants as _, DropdownButton};
use gpui_component::rating::Rating as GcRating;
use gpui_component::{h_flex, v_flex, Icon, IconName, Sizable};
use photo_domain::{Flag, ImageFormat, Rating, RecognitionFilter, SortBy, SortDirection};

use crate::action::{Action, ContextMenuAction};
use crate::state::app::RootView;
use crate::ui::theme;

/// 5 种排序方式的标签（用于下拉菜单与折叠栏）)
fn sort_by_label(sort_by: SortBy) -> &'static str {
    match sort_by {
        SortBy::FileName => "文件名",
        SortBy::DateTaken => "拍摄日期",
        SortBy::FileSize => "文件大小",
        SortBy::Rating => "评分",
        SortBy::Modified => "修改时间",
    }
}

fn recognition_filter_label(f: RecognitionFilter) -> &'static str {
    match f {
        RecognitionFilter::All => unreachable!(),
        RecognitionFilter::Confirmed => "已识别",
        RecognitionFilter::NeedsReview => "待复核",
        RecognitionFilter::Unrecognized => "未检测到",
        RecognitionFilter::NotRecognized => "未识别",
    }
}

/// 可折叠的网格视图筛选栏（含排序控件）
pub fn render_filter_bar(
    view: &RootView,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    let vh = cx.entity().downgrade();
    let expanded = view.filter_bar_expanded;
    let has_filters = view.has_active_filters();
    let sort_by = view.sort_by;
    let is_asc = view.sort_dir == SortDirection::Ascending;

    // ── 折叠态：筛选摘要芯片 ──
    let mut summary_chips: Vec<AnyElement> = Vec::new();
    if let Some(text) = &view.filter.text_search {
        summary_chips.push(summary_chip(
            "sum-search",
            format!("搜索: {text}"),
            vh.clone(),
            |view, _| view.filter.text_search = None,
        ));
    }
    if let Some(r) = view.filter.min_rating {
        summary_chips.push(summary_chip(
            "sum-rating",
            format!("评分≥{}星", r as u8),
            vh.clone(),
            |view, _| view.filter.min_rating = None,
        ));
    }
    if let Some(f) = view.filter.flag_filter {
        let label = match f {
            Flag::Pick => "旗标: 入选",
            Flag::Reject => "旗标: 淘汰",
        };
        summary_chips.push(summary_chip("sum-flag", label, vh.clone(), |view, _| {
            view.filter.flag_filter = None;
        }));
    }
    if view.filter.unflagged_filter {
        summary_chips.push(summary_chip("sum-unflagged", "旗标: 未标记", vh.clone(), |view, _| {
            view.filter.unflagged_filter = false;
        }));
    }
    if view.filter.recognition_filter != RecognitionFilter::All {
        summary_chips.push(summary_chip(
            "sum-recognition",
            format!("识别: {}", recognition_filter_label(view.filter.recognition_filter)),
            vh.clone(),
            |view, _| view.filter.recognition_filter = RecognitionFilter::All,
        ));
    }
    if let Some(p) = view.filter.paired_only {
        summary_chips.push(summary_chip(
            "sum-paired",
            if p { "仅配对" } else { "仅单张" },
            vh.clone(),
            |view, _| view.filter.paired_only = None,
        ));
    }
    if let Some(fmt) = &view.filter.format_filter {
        summary_chips.push(summary_chip(
            "sum-format",
            format!("格式: {fmt}"),
            vh.clone(),
            |view, _| view.filter.format_filter = None,
        ));
    }
    if view.filter.date_from.is_some() || view.filter.date_to.is_some() {
        let from = view
            .filter
            .date_from
            .map(|d| d.to_string())
            .unwrap_or_default();
        let to = view
            .filter
            .date_to
            .map(|d| d.to_string())
            .unwrap_or_default();
        summary_chips.push(summary_chip(
            "sum-date",
            format!("日期: {from}~{to}"),
            vh.clone(),
            |view, _| {
                view.filter.date_from = None;
                view.filter.date_to = None;
            },
        ));
    }

    // ── 折叠态：排序控件 ──
    let sort_dropdown = DropdownButton::new("sort-fbar-btn")
        .button(
            Button::new("sort-fbar-inner")
                .label(format!("排序: {}", sort_by_label(sort_by)))
                .ghost()
                .small(),
        )
        .dropdown_menu({
            move |menu, _, _| {
                let mut m = menu;
                for sb in [
                    SortBy::FileName,
                    SortBy::DateTaken,
                    SortBy::FileSize,
                    SortBy::Rating,
                    SortBy::Modified,
                ] {
                    let label = if sb == sort_by {
                        format!("✓ {}", sort_by_label(sb))
                    } else {
                        sort_by_label(sb).to_string()
                    };
                    m = m.menu(label, Box::new(ContextMenuAction(Action::SetSortBy(sb))));
                }
                m
            }
        });

    let sort_dir_btn = Button::new("sort-dir-btn")
        .icon(if is_asc {
            IconName::SortAscending
        } else {
            IconName::SortDescending
        })
        .ghost()
        .small()
        .tooltip(if is_asc {
            "升序，点击切换降序"
        } else {
            "降序，点击切换升序"
        })
        .on_click({
            let vh = vh.clone();
            move |_, _window, cx| {
                if let Some(e) = vh.upgrade() {
                    let _ = cx.update_entity(&e, |view, cx| {
                        view.dispatch_action(Action::ToggleSortDir, cx);
                    });
                }
            }
        });

    // ── 折叠行 ──
    let collapse_row = h_flex()
        .py_1()
        .px_3()
        .gap_2()
        .items_center()
        .child(
            // 展开/折叠切换
            h_flex()
                .id("filter-bar-toggle")
                .gap_1()
                .items_center()
                .cursor_pointer()
                .text_color(theme::colors().text)
                .on_click({
                    let vh = vh.clone();
                    move |_, _window, cx| {
                        if let Some(e) = vh.upgrade() {
                            let _ = cx.update_entity(&e, |view, cx| {
                                view.filter_bar_expanded = !view.filter_bar_expanded;
                                cx.notify();
                            });
                        }
                    }
                })
                .child(if expanded {
                    Icon::new(IconName::ChevronDown).xsmall()
                } else {
                    Icon::new(IconName::ChevronRight).xsmall()
                })
                .child("筛选"),
        )
        .children(summary_chips)
        .child(div().flex_grow(1.0))
        .child(sort_dropdown)
        .child(sort_dir_btn);

    // ── 展开态：分组筛选表单 ──
    let expanded_form = if expanded {
        let vh2 = vh.clone();
        v_flex()
            .px_3()
            .py_2()
            .gap_2()
            .border_t_1()
            .border_color(theme::colors().border_variant)
            .child(filter_group("搜索", render_text_search(view, cx)))
            .child(filter_group("评分", render_rating_filter(view, cx)))
            .child(filter_group("旗标", render_flag_filter(view, cx)))
            .child(filter_group("识别", render_recognition_filter(view, cx)))
            .child(filter_group("格式", render_format_filter(view, cx)))
            .when(has_filters, |parent| {
                parent.child(
                    h_flex()
                        .justify_end()
                        .pt_1()
                        .child(
                            Button::new("clear-all-filters")
                                .label("清除全部")
                                .ghost()
                                .small()
                                .on_click(move |_, _window, cx| {
                                    if let Some(e) = vh2.upgrade() {
                                        let _ = cx.update_entity(&e, |view, cx| {
                                            view.clear_filters();
                                            cx.notify();
                                        });
                                    }
                                }),
                        ),
                )
            })
            .into_any_element()
    } else {
        div().into_any_element()
    };

    v_flex()
        .w_full()
        .bg(theme::colors().surface_background)
        .border_b_1()
        .border_color(theme::colors().border_variant)
        .child(collapse_row)
        .child(expanded_form)
}

// ── 分组标题行 ──

fn filter_group(
    label: &'static str,
    content: impl IntoElement,
) -> impl IntoElement {
    h_flex()
        .gap_2()
        .items_start()
        .child(
            div()
                .flex_none()
                .pt(px(3.))
                .text_color(theme::colors().text_muted)
                .child(label),
        )
        .child(div().flex_1().min_w_0().child(content))
}

// ── 筛选芯片（从 sidebar 移植）─────────────────────────────────

/// 交易终端风格筛选 chip：小圆角、1px 描边、text_xs。
/// 选中态 = accent 描边 + accent 文字 + accent_dim 底色，
/// 未选中 = border 描边 + muted 文字。
#[allow(dead_code)]
pub fn filter_chip(
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

/// 折叠态摘要芯片：accent 风格、带 × 清除按钮
fn summary_chip(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    vh: WeakEntity<RootView>,
    clear: impl Fn(&mut RootView, &mut Context<RootView>) + 'static + Clone,
) -> AnyElement {
    let label: SharedString = label.into();
    div()
        .id(id)
        .flex()
        .gap_0p5()
        .items_center()
        .px_2()
        .py_0p5()
        .rounded_sm()
        .border_1()
        .border_color(theme::colors().text_accent)
        .text_color(theme::colors().text_accent)
        .bg(theme::accent_dim())
        .cursor_default()
        .child(label)
        .child(
            div()
                .id(ElementId::Name("sum-chip-close".into()))
                .cursor_pointer()
                .ml_1()
                .child(Icon::new(IconName::Close).xsmall().text_color(theme::colors().text_accent))
                .on_click(move |_, _window, cx| {
                    if let Some(e) = vh.upgrade() {
                        let _ = cx.update_entity(&e, |view, cx| {
                            clear(view, cx);
                            view.apply_filter_and_sort();
                            cx.notify();
                        });
                    }
                }),
        )
        .into_any_element()
}

// ── 条件组（从 sidebar 移植，适配水平过滤栏布局）─────────────────

fn render_text_search(view: &RootView, cx: &mut Context<RootView>) -> impl IntoElement {
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

    // pill 形搜索框
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
                        .id(ElementId::Name("fbar-text-search".into()))
                        .flex_grow(1.0)
                        .h(px(28.))
                        .px_1()
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
                                        if let Some(ch) = &event.keystroke.key_char {
                                            if ch.chars().all(|c| !c.is_control()) {
                                                let mut text = view
                                                    .filter
                                                    .text_search
                                                    .clone()
                                                    .unwrap_or_default();
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
                            .id(ElementId::Name("fbar-clear-text".into()))
                            .cursor_pointer()
                            .child(
                                Icon::new(IconName::Close)
                                    .xsmall()
                                    .text_color(theme::colors().error),
                            )
                            .on_click(
                                cx.listener(|view, _event: &ClickEvent, _window, cx| {
                                    view.filter.text_search = None;
                                    view.apply_filter_and_sort();
                                    cx.notify();
                                }),
                            ),
                    )
                }),
        )
}

fn render_rating_filter(view: &RootView, cx: &mut Context<RootView>) -> impl IntoElement {
    let current_min = view.filter.min_rating.map(|r| r as usize).unwrap_or(0);
    let vh = cx.entity().downgrade();

    h_flex()
        .items_center()
        .gap_2()
        .child(
            // 语义为「评分≥n」；点击当前已选星可递减，点击 1 星时清除为任意
            GcRating::new("fbar-rating")
                .small()
                .value(current_min)
                .on_click(move |value, _window, cx| {
                    if let Some(entity) = vh.upgrade() {
                        let _ = cx.update_entity(&entity, |view, cx| {
                            view.filter.min_rating = match value {
                                0 => None,
                                1 => Some(Rating::One),
                                2 => Some(Rating::Two),
                                3 => Some(Rating::Three),
                                4 => Some(Rating::Four),
                                5 => Some(Rating::Five),
                                _ => return,
                            };
                            view.apply_filter_and_sort();
                            cx.notify();
                        });
                    }
                }),
        )
        .child(
            div()
                .text_color(theme::colors().text_muted)
                .child(if current_min == 0 {
                    "任意".to_string()
                } else {
                    format!("≥{current_min} 星")
                }),
        )
}

fn render_flag_filter(view: &RootView, cx: &mut Context<RootView>) -> impl IntoElement {
    let current = view.filter.flag_filter;
    let unflagged = view.filter.unflagged_filter;
    let vh = cx.entity().downgrade();

    div()
        .flex()
        .flex_row()
        .flex_wrap()
        .gap_1()
        .child({
            let active = current.is_none() && !unflagged;
            let vh = vh.clone();
            filter_chip("fbar-flag-chip-any", "任意", active, move |_, _window, cx| {
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
            filter_chip("fbar-flag-chip-pick", "入选", active, move |_, _window, cx| {
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
            filter_chip(
                "fbar-flag-chip-reject",
                "淘汰",
                active,
                move |_, _window, cx| {
                    if let Some(view) = vh.upgrade() {
                        let _ = cx.update_entity(&view, |root_view, root_cx| {
                            root_view.filter.flag_filter = Some(Flag::Reject);
                            root_view.filter.unflagged_filter = false;
                            root_view.apply_filter_and_sort();
                            root_cx.notify();
                        });
                    }
                },
            )
        })
        .child({
            let active = unflagged;
            let vh = vh.clone();
            filter_chip(
                "fbar-flag-chip-unflagged",
                "未标记",
                active,
                move |_, _window, cx| {
                    if let Some(view) = vh.upgrade() {
                        let _ = cx.update_entity(&view, |root_view, root_cx| {
                            root_view.filter.flag_filter = None;
                            root_view.filter.unflagged_filter = true;
                            root_view.apply_filter_and_sort();
                            root_cx.notify();
                        });
                    }
                },
            )
        })
}

fn render_recognition_filter(view: &RootView, cx: &mut Context<RootView>) -> impl IntoElement {
    let current = view.filter.recognition_filter;
    let vh = cx.entity().downgrade();

    div()
        .flex()
        .flex_row()
        .flex_wrap()
        .gap_1()
        .child({
            let active = current == RecognitionFilter::All;
            let vh = vh.clone();
            filter_chip(
                "fbar-recognition-chip-all",
                "全部",
                active,
                move |_, _window, cx| {
                    if let Some(view) = vh.upgrade() {
                        let _ = cx.update_entity(&view, |root_view, root_cx| {
                            root_view.dispatch_action(
                                crate::action::Action::SetRecognitionFilter(
                                    RecognitionFilter::All,
                                ),
                                root_cx,
                            );
                        });
                    }
                },
            )
        })
        .child({
            let active = current == RecognitionFilter::Confirmed;
            let vh = vh.clone();
            filter_chip(
                "fbar-recognition-chip-confirmed",
                "已识别",
                active,
                move |_, _window, cx| {
                    if let Some(view) = vh.upgrade() {
                        let _ = cx.update_entity(&view, |root_view, root_cx| {
                            root_view.dispatch_action(
                                crate::action::Action::SetRecognitionFilter(
                                    RecognitionFilter::Confirmed,
                                ),
                                root_cx,
                            );
                        });
                    }
                },
            )
        })
        .child({
            let active = current == RecognitionFilter::NeedsReview;
            let vh = vh.clone();
            filter_chip(
                "fbar-recognition-chip-needs-review",
                "待复核",
                active,
                move |_, _window, cx| {
                    if let Some(view) = vh.upgrade() {
                        let _ = cx.update_entity(&view, |root_view, root_cx| {
                            root_view.dispatch_action(
                                crate::action::Action::SetRecognitionFilter(
                                    RecognitionFilter::NeedsReview,
                                ),
                                root_cx,
                            );
                        });
                    }
                },
            )
        })
        .child({
            let active = current == RecognitionFilter::Unrecognized;
            let vh = vh.clone();
            filter_chip(
                "fbar-recognition-chip-unrecog",
                "未检测到",
                active,
                move |_, _window, cx| {
                    if let Some(view) = vh.upgrade() {
                        let _ = cx.update_entity(&view, |root_view, root_cx| {
                            root_view.dispatch_action(
                                crate::action::Action::SetRecognitionFilter(
                                    RecognitionFilter::Unrecognized,
                                ),
                                root_cx,
                            );
                        });
                    }
                },
            )
        })
        .child({
            let active = current == RecognitionFilter::NotRecognized;
            let vh = vh.clone();
            filter_chip(
                "fbar-recognition-chip-not-recog",
                "未识别",
                active,
                move |_, _window, cx| {
                    if let Some(view) = vh.upgrade() {
                        let _ = cx.update_entity(&view, |root_view, root_cx| {
                            root_view.dispatch_action(
                                crate::action::Action::SetRecognitionFilter(
                                    RecognitionFilter::NotRecognized,
                                ),
                                root_cx,
                            );
                        });
                    }
                },
            )
        })
}

// ── 格式筛选 ────────────────────────────────────────────────────

fn render_format_filter(view: &RootView, cx: &mut Context<RootView>) -> impl IntoElement {
    let current = view.filter.format_filter.as_ref();
    let vh = cx.entity().downgrade();

    div()
        .flex()
        .flex_row()
        .flex_wrap()
        .gap_1()
        // 全部
        .child({
            let active = current.is_none();
            let vh = vh.clone();
            filter_chip("fmt-all", "全部", active, move |_, _window, cx| {
                if let Some(view) = vh.upgrade() {
                    let _ = cx.update_entity(&view, |root_view, root_cx| {
                        root_view.filter.format_filter = None;
                        root_view.apply_filter_and_sort();
                        root_cx.notify();
                    });
                }
            })
        })
        .child(format_chip("JPEG", ImageFormat::Jpeg, current, &vh))
        .child(format_chip("PNG", ImageFormat::Png, current, &vh))
        .child(format_chip("TIFF", ImageFormat::Tiff, current, &vh))
        .child(format_chip("WebP", ImageFormat::WebP, current, &vh))
        .child(format_chip("BMP", ImageFormat::Bmp, current, &vh))
        .child(format_chip("GIF", ImageFormat::Gif, current, &vh))
        .child(format_chip("HEIF", ImageFormat::Heif, current, &vh))
        .child(format_chip("RAW", ImageFormat::Raw("RAW".into()), current, &vh))
}

fn format_chip(
    label: &'static str,
    fmt: ImageFormat,
    current: Option<&ImageFormat>,
    vh: &gpui::WeakEntity<RootView>,
) -> impl IntoElement {
    let active = current.map_or(false, |c| {
        match (c, &fmt) {
            (ImageFormat::Raw(a), ImageFormat::Raw(b)) => a == b,
            (a, b) => a == b,
        }
    });
    let vh = vh.clone();
    filter_chip(
        format!("fmt-{label}"),
        label,
        active,
        move |_, _window, cx| {
            if let Some(view) = vh.upgrade() {
                let _ = cx.update_entity(&view, |root_view, root_cx| {
                    root_view.filter.format_filter = if active { None } else { Some(fmt.clone()) };
                    root_view.apply_filter_and_sort();
                    root_cx.notify();
                });
            }
        },
    )
}

