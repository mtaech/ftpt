//! 筛选栏组件（仅网格态渲染，对应 §9.4）。
//!
//! - 折叠态（36px）：筛选开关 + 激活 chips + 排序下拉 + 升降序 + 列数下拉
//! - 展开态：格式单选、最低星级、旗标单选、识别状态单选、颜色标签、重置全部

use gpui_kit::component::{
    ActiveTheme as _, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    combobox::Combobox,
    h_flex,
    select::Select,
    tag::Tag,
    v_flex,
};
use gpui_kit::{ClickEvent, Context, IntoElement, Window, div, prelude::*, px};
use photo_domain::{ColorLabel, Flag, ImageFormat, Rating, RecognitionFilter, SortDirection};

use crate::model::filter::{default_filter_criteria, has_active_filters};
use crate::state::AppState;

pub fn render_filter_bar(
    state: &AppState,
    window: &mut Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement + use<> {
    let has_filter = has_active_filters(&state.criteria);
    let is_expanded = state.filter_bar_expanded;

    // 镜头 / 物种下拉的候选项随目录变化：扫描完成后重建一次。
    // 渲染期才有 window（set_items 需要），所以走 defer_in 把实体先归还再改。
    if state.filter_options_generation != state.scan_generation {
        cx.defer_in(window, |state, window, cx| {
            state.sync_filter_option_selects(window, cx);
        });
    }

    v_flex()
        .w_full()
        .bg(cx.theme().popover)
        .border_b_1()
        .border_color(cx.theme().border)
        .px_3()
        .py_1()
        .gap_2()
        // ── 顶部单行折叠行（36px） ──
        .child(
            h_flex()
                .w_full()
                .h(px(32.))
                .items_center()
                .justify_between()
                .gap_2()
                // 左侧：筛选展开按钮与 Chips
                .child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .flex_1()
                        .overflow_x_hidden()
                        .child(
                            Button::new("btn-filter-expand")
                                .small()
                                .ghost()
                                .icon(if is_expanded {
                                    IconName::ChevronDown
                                } else {
                                    IconName::ChevronsUpDown
                                })
                                .label(if has_filter { "已筛选" } else { "筛选" })
                                .text_color(if has_filter {
                                    cx.theme().primary
                                } else {
                                    cx.theme().muted_foreground
                                })
                                .on_click(cx.listener(
                                    move |state: &mut AppState, _event, _window, cx| {
                                        state.filter_bar_expanded = !state.filter_bar_expanded;
                                        cx.notify();
                                    },
                                )),
                        )
                        // 激活的摘要 chips (gpui-component Tag)
                        .when(state.criteria.format_filter.is_some(), |this| {
                            let fmt = state.criteria.format_filter.clone().unwrap();
                            this.child(
                                Tag::secondary().small().rounded_full().child(
                                    h_flex()
                                        .items_center()
                                        .gap_1()
                                        .child(format!("{fmt:?}"))
                                        .child(
                                            Button::new("clear-format")
                                                .icon(IconName::Close)
                                                .ghost()
                                                .xsmall()
                                                .tooltip("清除格式筛选")
                                                .on_click(cx.listener(|state, _, _, cx| {
                                                    state.criteria.format_filter = None;
                                                    state.recompute_pipeline();
                                                    cx.notify();
                                                })),
                                        ),
                                ),
                            )
                        })
                        .when(state.criteria.min_rating.is_some(), |this| {
                            let r = state.criteria.min_rating.unwrap();
                            this.child(
                                Tag::secondary().small().rounded_full().child(
                                    h_flex()
                                        .items_center()
                                        .gap_1()
                                        .child(format!("≥ {r:?}"))
                                        .child(
                                            Button::new("clear-rating")
                                                .icon(IconName::Close)
                                                .ghost()
                                                .xsmall()
                                                .tooltip("清除评分筛选")
                                                .on_click(cx.listener(|state, _, _, cx| {
                                                    state.criteria.min_rating = None;
                                                    state.recompute_pipeline();
                                                    cx.notify();
                                                })),
                                        ),
                                ),
                            )
                        })
                        .when(state.criteria.flag_filter.is_some(), |this| {
                            let f = state.criteria.flag_filter.unwrap();
                            this.child(
                                Tag::secondary().small().rounded_full().child(
                                    h_flex()
                                        .items_center()
                                        .gap_1()
                                        .child(format!("{f:?}"))
                                        .child(
                                            Button::new("clear-flag")
                                                .icon(IconName::Close)
                                                .ghost()
                                                .xsmall()
                                                .tooltip("清除旗标筛选")
                                                .on_click(cx.listener(|state, _, _, cx| {
                                                    state.criteria.flag_filter = None;
                                                    state.recompute_pipeline();
                                                    cx.notify();
                                                })),
                                        ),
                                ),
                            )
                        })
                        .when(state.criteria.color_label.is_some(), |this| {
                            let cl = state.criteria.color_label.unwrap();
                            this.child(
                                Tag::secondary().small().rounded_full().child(
                                    h_flex()
                                        .items_center()
                                        .gap_1()
                                        .child(format!("{cl:?}"))
                                        .child(
                                            Button::new("clear-color")
                                                .icon(IconName::Close)
                                                .ghost()
                                                .xsmall()
                                                .tooltip("清除色标筛选")
                                                .on_click(cx.listener(|state, _, _, cx| {
                                                    state.criteria.color_label = None;
                                                    state.recompute_pipeline();
                                                    cx.notify();
                                                })),
                                        ),
                                ),
                            )
                        })
                        .when(
                            state.criteria.recognition_filter != RecognitionFilter::All,
                            |this| {
                                let rf = state.criteria.recognition_filter;
                                this.child(
                                    Tag::secondary().small().rounded_full().child(
                                        h_flex()
                                            .items_center()
                                            .gap_1()
                                            .child(format!("{rf:?}"))
                                            .child(
                                                Button::new("clear-recog")
                                                    .icon(IconName::Close)
                                                    .ghost()
                                                    .xsmall()
                                                    .tooltip("清除识别筛选")
                                                    .on_click(cx.listener(|state, _, _, cx| {
                                                        state.criteria.recognition_filter =
                                                            RecognitionFilter::All;
                                                        state.recompute_pipeline();
                                                        cx.notify();
                                                    })),
                                            ),
                                    ),
                                )
                            },
                        )
                        // 镜头 / 物种多选：激活后在折叠行里给出可清除的 chip
                        .when(!state.criteria.lens_filter.is_empty(), |this| {
                            let values = state.criteria.lens_filter.clone();
                            this.child(multi_value_chip(MultiFilter::Lens, &values, cx))
                        })
                        .when(!state.criteria.taxon_names.is_empty(), |this| {
                            let values = state.criteria.taxon_names.clone();
                            this.child(multi_value_chip(MultiFilter::Taxon, &values, cx))
                        }),
                )
                // 右侧：排序选项与网格列数
                .child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        // 排序方式（下拉选择，替代原来的「点击循环」）
                        .when_some(state.sort_select.clone(), |this, select| {
                            this.child(
                                Select::new(&select)
                                    .small()
                                    .w(px(116.))
                                    .menu_width(px(148.)),
                            )
                        })
                        // 升序/降序切换
                        .child(
                            Button::new("btn-sort-dir")
                                .small()
                                .ghost()
                                .icon(if state.sort_direction == SortDirection::Ascending {
                                    IconName::ArrowUp
                                } else {
                                    IconName::ArrowDown
                                })
                                .tooltip(if state.sort_direction == SortDirection::Ascending {
                                    "升序"
                                } else {
                                    "降序"
                                })
                                .on_click(cx.listener(|state, _, _, cx| {
                                    state.sort_direction = match state.sort_direction {
                                        SortDirection::Ascending => SortDirection::Descending,
                                        SortDirection::Descending => SortDirection::Ascending,
                                    };
                                    state.recompute_pipeline();
                                    cx.notify();
                                })),
                        )
                        // 每行列数（下拉选择，替代原来的「点击循环」）
                        .when_some(state.grid_cols_select.clone(), |this, select| {
                            this.child(
                                Select::new(&select)
                                    .small()
                                    .w(px(84.))
                                    .menu_width(px(96.)),
                            )
                        }),
                ),
        )
        // ── 展开面板（条件组） ──
        .when(is_expanded, |this| {
            this.child(
                v_flex()
                    .w_full()
                    .pt_2()
                    .pb_2()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .gap_2()
                    // 1. 格式
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .w(px(60.))
                                    .text_color(cx.theme().muted_foreground)
                                    .child("格式"),
                            )
                            .child(render_filter_chip(
                                "全部",
                                state.criteria.format_filter.is_none(),
                                cx,
                                cx.listener(|state, _, _, cx| {
                                    state.criteria.format_filter = None;
                                    state.recompute_pipeline();
                                    cx.notify();
                                }),
                            ))
                            .child(render_filter_chip(
                                "JPEG",
                                matches!(state.criteria.format_filter, Some(ImageFormat::Jpeg)),
                                cx,
                                cx.listener(|state, _, _, cx| {
                                    state.criteria.format_filter = Some(ImageFormat::Jpeg);
                                    state.recompute_pipeline();
                                    cx.notify();
                                }),
                            ))
                            .child(render_filter_chip(
                                "RAW",
                                matches!(state.criteria.format_filter, Some(ImageFormat::Raw(_))),
                                cx,
                                cx.listener(|state, _, _, cx| {
                                    state.criteria.format_filter =
                                        Some(ImageFormat::Raw("RAW".to_string()));
                                    state.recompute_pipeline();
                                    cx.notify();
                                }),
                            )),
                    )
                    // 2. 星级
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .w(px(60.))
                                    .text_color(cx.theme().muted_foreground)
                                    .child("星级"),
                            )
                            .child(render_filter_chip(
                                "全部",
                                state.criteria.min_rating.is_none(),
                                cx,
                                cx.listener(|state, _, _, cx| {
                                    state.criteria.min_rating = None;
                                    state.recompute_pipeline();
                                    cx.notify();
                                }),
                            ))
                            .child(render_filter_chip(
                                "≥ 1★",
                                state.criteria.min_rating == Some(Rating::One),
                                cx,
                                cx.listener(|state, _, _, cx| {
                                    state.criteria.min_rating = Some(Rating::One);
                                    state.recompute_pipeline();
                                    cx.notify();
                                }),
                            ))
                            .child(render_filter_chip(
                                "≥ 3★",
                                state.criteria.min_rating == Some(Rating::Three),
                                cx,
                                cx.listener(|state, _, _, cx| {
                                    state.criteria.min_rating = Some(Rating::Three);
                                    state.recompute_pipeline();
                                    cx.notify();
                                }),
                            ))
                            .child(render_filter_chip(
                                "≥ 5★",
                                state.criteria.min_rating == Some(Rating::Five),
                                cx,
                                cx.listener(|state, _, _, cx| {
                                    state.criteria.min_rating = Some(Rating::Five);
                                    state.recompute_pipeline();
                                    cx.notify();
                                }),
                            )),
                    )
                    // 3. 旗标
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .w(px(60.))
                                    .text_color(cx.theme().muted_foreground)
                                    .child("旗标"),
                            )
                            .child(render_filter_chip(
                                "全部",
                                state.criteria.flag_filter.is_none()
                                    && !state.criteria.unflagged_filter,
                                cx,
                                cx.listener(|state, _, _, cx| {
                                    state.criteria.flag_filter = None;
                                    state.criteria.unflagged_filter = false;
                                    state.recompute_pipeline();
                                    cx.notify();
                                }),
                            ))
                            .child(render_filter_chip(
                                "入选",
                                state.criteria.flag_filter == Some(Flag::Pick),
                                cx,
                                cx.listener(|state, _, _, cx| {
                                    state.criteria.flag_filter = Some(Flag::Pick);
                                    state.criteria.unflagged_filter = false;
                                    state.recompute_pipeline();
                                    cx.notify();
                                }),
                            ))
                            .child(render_filter_chip(
                                "未标记",
                                state.criteria.unflagged_filter,
                                cx,
                                cx.listener(|state, _, _, cx| {
                                    state.criteria.flag_filter = None;
                                    state.criteria.unflagged_filter = true;
                                    state.recompute_pipeline();
                                    cx.notify();
                                }),
                            ))
                            .child(render_filter_chip(
                                "淘汰",
                                state.criteria.flag_filter == Some(Flag::Reject),
                                cx,
                                cx.listener(|state, _, _, cx| {
                                    state.criteria.flag_filter = Some(Flag::Reject);
                                    state.criteria.unflagged_filter = false;
                                    state.recompute_pipeline();
                                    cx.notify();
                                }),
                            )),
                    )
                    // 4. 识别状态
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .w(px(60.))
                                    .text_color(cx.theme().muted_foreground)
                                    .child("物种识别"),
                            )
                            .child(render_filter_chip(
                                "全部",
                                state.criteria.recognition_filter == RecognitionFilter::All,
                                cx,
                                cx.listener(|state, _, _, cx| {
                                    state.criteria.recognition_filter = RecognitionFilter::All;
                                    state.recompute_pipeline();
                                    cx.notify();
                                }),
                            ))
                            .child(render_filter_chip(
                                "已识别",
                                state.criteria.recognition_filter == RecognitionFilter::Confirmed,
                                cx,
                                cx.listener(|state, _, _, cx| {
                                    state.criteria.recognition_filter =
                                        RecognitionFilter::Confirmed;
                                    state.recompute_pipeline();
                                    cx.notify();
                                }),
                            ))
                            .child(render_filter_chip(
                                "待人工确认",
                                state.criteria.recognition_filter == RecognitionFilter::NeedsReview,
                                cx,
                                cx.listener(|state, _, _, cx| {
                                    state.criteria.recognition_filter =
                                        RecognitionFilter::NeedsReview;
                                    state.recompute_pipeline();
                                    cx.notify();
                                }),
                            ))
                            .child(render_filter_chip(
                                "未识别",
                                state.criteria.recognition_filter
                                    == RecognitionFilter::Unrecognized,
                                cx,
                                cx.listener(|state, _, _, cx| {
                                    state.criteria.recognition_filter =
                                        RecognitionFilter::Unrecognized;
                                    state.recompute_pipeline();
                                    cx.notify();
                                }),
                            )),
                    )
                    // 5. 色标
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .w(px(60.))
                                    .text_color(cx.theme().muted_foreground)
                                    .child("色标"),
                            )
                            .child(render_filter_chip(
                                "全部",
                                state.criteria.color_label.is_none(),
                                cx,
                                cx.listener(|state, _, _, cx| {
                                    state.criteria.color_label = None;
                                    state.recompute_pipeline();
                                    cx.notify();
                                }),
                            ))
                            .child(render_filter_chip(
                                "红",
                                state.criteria.color_label == Some(ColorLabel::Red),
                                cx,
                                cx.listener(|state, _, _, cx| {
                                    state.criteria.color_label = Some(ColorLabel::Red);
                                    state.recompute_pipeline();
                                    cx.notify();
                                }),
                            ))
                            .child(render_filter_chip(
                                "黄",
                                state.criteria.color_label == Some(ColorLabel::Yellow),
                                cx,
                                cx.listener(|state, _, _, cx| {
                                    state.criteria.color_label = Some(ColorLabel::Yellow);
                                    state.recompute_pipeline();
                                    cx.notify();
                                }),
                            ))
                            .child(render_filter_chip(
                                "绿",
                                state.criteria.color_label == Some(ColorLabel::Green),
                                cx,
                                cx.listener(|state, _, _, cx| {
                                    state.criteria.color_label = Some(ColorLabel::Green);
                                    state.recompute_pipeline();
                                    cx.notify();
                                }),
                            ))
                            .child(render_filter_chip(
                                "蓝",
                                state.criteria.color_label == Some(ColorLabel::Blue),
                                cx,
                                cx.listener(|state, _, _, cx| {
                                    state.criteria.color_label = Some(ColorLabel::Blue);
                                    state.recompute_pipeline();
                                    cx.notify();
                                }),
                            ))
                            .child(render_filter_chip(
                                "紫",
                                state.criteria.color_label == Some(ColorLabel::Purple),
                                cx,
                                cx.listener(|state, _, _, cx| {
                                    state.criteria.color_label = Some(ColorLabel::Purple);
                                    state.recompute_pipeline();
                                    cx.notify();
                                }),
                            )),
                    )
                    // 5. 镜头（多选 + 可搜索；候选 = 当前目录照片的 EXIF lens）
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .w(px(60.))
                                    .text_color(cx.theme().muted_foreground)
                                    .child("镜头"),
                            )
                            .when_some(state.lens_filter_select.clone(), |this, select| {
                                this.child(
                                    div().w(px(300.)).child(
                                        Combobox::new(&select)
                                            .placeholder("全部镜头")
                                            .menu_width(px(320.))
                                            .menu_max_h(px(320.))
                                            .search_placeholder("搜索镜头…")
                                            .cleanable(true),
                                    ),
                                )
                            })
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("多选：命中任一即保留（候选来自本目录 EXIF）"),
                            ),
                    )
                    // 6. 物种（多选 + 可搜索；候选 = 顶层展示名 + 各主体展示名）
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .w(px(60.))
                                    .text_color(cx.theme().muted_foreground)
                                    .child("物种"),
                            )
                            .when_some(state.species_filter_select.clone(), |this, select| {
                                this.child(
                                    div().w(px(300.)).child(
                                        Combobox::new(&select)
                                            .placeholder("全部物种")
                                            .menu_width(px(320.))
                                            .menu_max_h(px(320.))
                                            .search_placeholder("搜索物种…")
                                            .cleanable(true),
                                    ),
                                )
                            })
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("多选：命中任一即保留（含多主体的次要物种）"),
                            ),
                    )
                    // 重置全部按钮
                    .when(has_filter, |this| {
                        this.child(
                            h_flex().w_full().justify_end().pt_1().child(
                                Button::new("btn-reset-filters")
                                    .small()
                                    .ghost()
                                    .label("重置全部筛选")
                                    .on_click(cx.listener(|state, _, _, cx| {
                                        state.criteria = default_filter_criteria();
                                        // 两个多选下拉也要清，否则界面上还勾着已重置的选项
                                        if let Some(select) = state.lens_filter_select.clone() {
                                            select.update(cx, |s, cx| s.clear_selection(cx));
                                        }
                                        if let Some(select) = state.species_filter_select.clone() {
                                            select.update(cx, |s, cx| s.clear_selection(cx));
                                        }
                                        state.recompute_pipeline();
                                        cx.notify();
                                    })),
                            ),
                        )
                    }),
            )
        })
}

/// 多值筛选的种类（chip 的 × 要同时清 criteria 与下拉选中集）
#[derive(Clone, Copy)]
enum MultiFilter {
    Lens,
    Taxon,
}

/// 多值筛选的激活 chip：显示前 2 个值（更多则「等 N 项」），× 一次清空该筛选。
///
/// 清除必须**同时**清 criteria 与下拉选中集：只清 criteria 的话下拉里还勾着，
/// 用户再点一下就会把刚清掉的筛选值加回来（候选同步只在扫描后跑，不会兜这个底）。
fn multi_value_chip(
    kind: MultiFilter,
    values: &[String],
    cx: &Context<AppState>,
) -> impl IntoElement {
    let shown = values
        .iter()
        .take(2)
        .cloned()
        .collect::<Vec<_>>()
        .join(" / ");
    let suffix = if values.len() > 2 {
        format!(" 等 {} 项", values.len())
    } else {
        String::new()
    };
    let text = match kind {
        MultiFilter::Lens => format!("镜头: {shown}{suffix}"),
        MultiFilter::Taxon => format!("物种: {shown}{suffix}"),
    };

    Tag::secondary()
        .small()
        .rounded_full()
        .child(
            h_flex()
                .items_center()
                .gap_1()
                .child(text)
                .child(
                    Button::new(match kind {
                        MultiFilter::Lens => "clear-lens-filter",
                        MultiFilter::Taxon => "clear-taxon-filter",
                    })
                    .icon(IconName::Close)
                    .ghost()
                    .xsmall()
                    .tooltip(match kind {
                        MultiFilter::Lens => "清除镜头筛选",
                        MultiFilter::Taxon => "清除物种筛选",
                    })
                    .on_click(cx.listener(move |state, _, _window, cx| {
                        match kind {
                            MultiFilter::Lens => {
                                state.criteria.lens_filter.clear();
                                if let Some(select) = state.lens_filter_select.clone() {
                                    select.update(cx, |s, cx| s.clear_selection(cx));
                                }
                            }
                            MultiFilter::Taxon => {
                                state.criteria.taxon_names.clear();
                                if let Some(select) = state.species_filter_select.clone() {
                                    select.update(cx, |s, cx| s.clear_selection(cx));
                                }
                            }
                        }
                        state.recompute_pipeline();
                        cx.notify();
                    })),
                ),
        )
}

fn render_filter_chip(
    label: &'static str,
    active: bool,
    cx: &Context<AppState>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut gpui_kit::App) + 'static,
) -> impl IntoElement {
    let theme = cx.theme();
    h_flex()
        .id(label)
        .items_center()
        .gap_1()
        .px_3()
        .py_1()
        .rounded_full()
        .cursor_pointer()
        .text_xs()
        .when(active, |this| {
            this.bg(theme.selection)
                .text_color(theme.primary)
                .font_medium()
                .border_1()
                .border_color(theme.primary.opacity(0.4))
                .shadow(crate::theme::panel_card_shadow(cx))
                .child(
                    gpui_kit::component::Icon::new(IconName::Check)
                        .size(px(11.))
                        .text_color(theme.primary),
                )
        })
        .when(!active, |this| {
            this.bg(theme.popover)
                .text_color(theme.muted_foreground)
                .border_1()
                .border_color(theme.border.opacity(0.6))
                .hover(move |s| s.bg(theme.accent).text_color(theme.foreground))
        })
        .child(label)
        .on_click(on_click)
}
