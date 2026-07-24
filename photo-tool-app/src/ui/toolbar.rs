use gpui::*;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::select::Select;
use gpui_component::{IconName, Selectable};
use gpui_component::Sizable;
use gpui_component::h_flex;

use photo_tool_core::domain::SortBy;

use crate::action::Action;
use crate::state::app::RootView;
use crate::ui::theme;

/// 渲染顶部工具栏：目录信息 | 视图切换 tab | 操作区
/// 交易终端风格：近黑底色、下划线 tab、等宽计数、icon-only 按钮
pub fn render_toolbar(
    view: &RootView,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    let vh = cx.entity().downgrade();

    let dir_name: SharedString = view
        .dir_path
        .as_ref()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .map(|s| s.into())
        .unwrap_or_else(|| "未打开目录".into());

    let count = view.captures.len();
    let is_grid = view.view_mode == crate::state::app::ViewMode::Grid;

    let render_tab = |label: &'static str, active: bool| {
        let vh = vh.clone();
        div()
            .id(SharedString::from(label))
            .flex()
            .flex_col()
            .items_center()
            .cursor_pointer()
            .px_3()
            .h_full()
            .on_click(move |_, _window, cx| {
                if let Some(entity) = vh.upgrade() {
                    cx.update_entity(&entity, |view, cx| {
                        view.dispatch_action(Action::ToggleGridPreview, cx);
                    });
                }
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .h_full()
                    .text_sm()
                    .text_color(if active { theme::colors().text } else { theme::colors().text_muted })
                    .child(label),
            )
            .child(
                div()
                    .h(px(2.))
                    .w_full()
                    .bg(if active { theme::colors().text_accent } else { hsla(0., 0., 0., 0.) }),
            )
    };

    div()
        .flex()
        .flex_row()
        .items_center()
        .h(px(44.))
        .px_2()
        .bg(theme::colors().surface_background)
        .border_b_1()
        .border_color(theme::colors().border_variant)
        .child(
            h_flex()
                .gap_1p5()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .max_w(px(180.))
                        .truncate()
                        .child(dir_name),
                )
                .child(
                    div()
                        .text_xs()
                        .font_family(theme::MONO_FONT_FAMILY)
                        .text_color(theme::colors().text_muted)
                        .child(format!("{} 项", count)),
                ),
        )
        .child(div().flex_grow(1.0))
        .child(
            h_flex()
                .gap_0()
                .h_full()
                .child(render_tab("网格", is_grid))
                .child(render_tab("预览", !is_grid)),
        )
        .child(div().flex_grow(1.0))
        .child(
            h_flex()
                .gap_1()
                .child(
                    Button::new("sort-btn")
                        .icon(IconName::SortAscending)
                        .ghost()
                        .small()
                        .label(match view.sort_by {
                            SortBy::FileName => "文件名",
                            SortBy::DateTaken => "拍摄日期",
                            SortBy::FileSize => "文件大小",
                            SortBy::Rating => "评分",
                            SortBy::Modified => "修改时间",
                        })
                        .tooltip("排序方式")
                        .on_click({
                            let vh = vh.clone();
                            move |_, _window, cx| {
                                if let Some(entity) = vh.upgrade() {
                                    cx.update_entity(&entity, |view, cx| {
                                        view.sort_by = match view.sort_by {
                                            SortBy::FileName => SortBy::DateTaken,
                                            SortBy::DateTaken => SortBy::FileSize,
                                            SortBy::FileSize => SortBy::Rating,
                                            SortBy::Rating => SortBy::Modified,
                                            SortBy::Modified => SortBy::FileName,
                                        };
                                        view.apply_filter_and_sort();
                                        cx.notify();
                                    });
                                }
                            }
                        }),
                )
                .child(
                    Button::new("refresh-btn")
                        .icon(gpui_component::Icon::empty().path("icons/refresh-cw.svg"))
                        .ghost()
                        .small()
                        .tooltip("重新扫描")
                        .on_click({
                            let vh = vh.clone();
                            move |_, _window, cx| {
                                if let Some(entity) = vh.upgrade() {
                                    cx.update_entity(&entity, |view, cx| {
                                        view.dispatch_action(Action::Refresh, cx);
                                    });
                                }
                            }
                        }),
                )
        )
}

// ── Settings Dialog Overlay ──────────────────────────────────────────

/// 单选组渲染 helper：横排可选按钮，选中态高亮，点击直接写 View config
fn settings_radio_row(
    vh: WeakEntity<RootView>,
    label: &'static str,
    options: &'static [(&'static str, &'static str)],
    current: &str,
    on_pick: impl Fn(&str, &mut RootView) + 'static + Clone,
) -> impl IntoElement {
    h_flex()
        .items_center()
        .justify_between()
        .child(div().text_color(theme::colors().text_muted).child(label))
        .child(
            h_flex()
                .gap_1()
                .children(options.iter().map(move |(id, display)| {
                    let id = id.to_string();
                    let display = display.to_string();
                    let is_selected = current == id;
                    let vh = vh.clone();
                    let act = on_pick.clone();
                    Button::new(format!("setting-{id}"))
                        .ghost()
                        .selected(is_selected)
                        .label(display)
                        .on_click(move |_, _, cx| {
                            if let Some(e) = vh.upgrade() {
                                cx.update_entity(&e, |view, cx| {
                                    act(&id, view);
                                    view.save_config();
                                    cx.notify();
                                });
                            }
                        })
                })),
        )
}

/// 渲染设置弹窗：卡片式布局，Escape/X 关闭，不拦截 Select 子弹出层。
pub fn render_settings_overlay(view: &RootView, cx: &mut Context<RootView>) -> impl IntoElement {
    let vh = cx.entity().downgrade();
    let delete_mode = view.config.default_delete_mode.clone();
    let cache_size = view.config.max_cache_size_mb;

    div()
        .size_full()
        .absolute()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgba(0x00000055))
        .child(
            div()
                .w(px(560.))
                .bg(theme::colors().surface_background)
                .rounded_md()
                .border_1()
                .border_color(theme::colors().border_variant)
                .shadow(theme::ElevationIndex::ModalSurface.shadow())
                .p_4()
                .gap_3()
                .flex()
                .flex_col()
                .on_key_down(cx.listener(move |v, event: &KeyDownEvent, _w, cx| {
                    if event.keystroke.key.as_str() == "escape" {
                        v.show_settings = false;
                        cx.notify();
                    }
                }))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .child(div().text_lg().child("设置"))
                        .child(
                            Button::new("settings-x")
                                .icon(IconName::Close)
                                .ghost()
                                .on_click(cx.listener(move |v, _, _w, cx| {
                                    v.show_settings = false;
                                    cx.notify();
                                })),
                        ),
                )
                .child(div().h(px(1.)).w_full().bg(theme::colors().border_variant))
                .child(section("界面"))
                .child(
                    h_flex()
                        .items_center()
                        .justify_between()
                        .child(div().text_color(theme::colors().text_muted).child("字体"))
                        .child(
                            div()
                                .w(px(320.))
                                .child(Select::new(&view.font_select).search_placeholder("输入字体名过滤")),
                        ),
                )
                .child(div().h(px(1.)).w_full().bg(theme::colors().border_variant))
                .child(section("文件操作"))
                .child(settings_radio_row(
                    vh.clone(),
                    "默认删除模式",
                    &[("trash", "回收站"), ("permanent", "永久删除")],
                    &delete_mode,
                    |id, view| view.config.default_delete_mode = id.to_string(),
                ))
                .child(section("缓存"))
                .child(
                    h_flex()
                        .items_center()
                        .justify_between()
                        .child(div().text_color(theme::colors().text_muted).child("最大缓存"))
                        .child(
                            h_flex()
                                .gap_1()
                                .children([256u64, 512, 1024, 2048].map(|opt| {
                                    let vh = vh.clone();
                                    Button::new(format!("cache-{opt}"))
                                        .ghost()
                                        .selected(cache_size == opt)
                                        .label(format!("{} MB", opt))
                                        .on_click(move |_, _, cx| {
                                            if let Some(e) = vh.upgrade() {
                                                cx.update_entity(&e, |view, cx| {
                                                    view.config.max_cache_size_mb = opt;
                                                    view.save_config();
                                                    cx.notify();
                                                });
                                            }
                                        })
                                })),
                        ),
                )
                .child(
                    h_flex()
                        .justify_end()
                        .pt_3()
                        .child(
                            Button::new("settings-close")
                                .primary()
                                .label("关闭")
                                .on_click(cx.listener(move |v, _, _w, cx| {
                                    v.show_settings = false;
                                    cx.notify();
                                })),
                        ),
                ),
        )
}

fn section(label: &str) -> impl IntoElement {
    div()
        .text_color(theme::colors().text_muted)
        .font_weight(FontWeight::MEDIUM)
        .child(label.to_string())
}

