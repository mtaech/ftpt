use gpui::*;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::IconName;
use gpui_component::h_flex;

use photo_tool_core::domain::SortBy;

use crate::action::Action;
use crate::state::app::RootView;
use crate::ui::theme;


/// Render the top toolbar with import button, view toggle, sort controls, refresh.
pub fn render_toolbar(
    view: &RootView,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    let vh = cx.entity().downgrade();
    div()
        .flex()
        .flex_row()
        .items_center()
        .h(px(40.))
        .bg(theme::colors().surface_background)
        .border_b_1()
        .border_color(theme::colors().border_variant)
        .px_3()
        .gap_3()
        .child(
            Button::new("settings-btn")
                .icon(IconName::Settings)
                .ghost()
                .label("设置")
                .on_click({
                    let vh = vh.clone();
                    move |_, _window, cx| {
                        if let Some(entity) = vh.upgrade() { cx.update_entity(&entity, |view, cx| view.dispatch_action(Action::ToggleSettings, cx)); }
                    }
                }),
        )
        .child(
            Button::new("import-btn")
                .icon(IconName::File)
                .primary()
                .label("导入")
                .on_click({
                    let vh = vh.clone();
                    move |_, _window, cx| {
                        if let Some(entity) = vh.upgrade() { cx.update_entity(&entity, |view, cx| view.dispatch_action(Action::OpenImport, cx)); }
                    }
                }),
        )
        .child({
            let label = match view.view_mode {
                crate::state::app::ViewMode::Grid => "网格",
                crate::state::app::ViewMode::Preview => "预览",
            };
            Button::new("view-toggle-btn")
                .icon(IconName::LayoutDashboard)
                .ghost()
                .label(label)
                .on_click({
                    let vh = vh.clone();
                    move |_, _window, cx| {
                        if let Some(entity) = vh.upgrade() { cx.update_entity(&entity, |view, cx| view.dispatch_action(Action::ToggleGridPreview, cx)); }
                    }
                })
        })
        .child({
            let label = match view.sort_by {
                SortBy::FileName => "排序：文件名",
                SortBy::DateTaken => "排序：拍摄日期",
                SortBy::FileSize => "排序：文件大小",
                SortBy::Rating => "排序：评分",
                SortBy::Modified => "排序：修改时间",
            };
            Button::new("sort-btn")
                .icon(IconName::SortAscending)
                .ghost()
                .label(label)
                .on_click({
                    let vh = vh.clone();
                    move |_, _window, cx| {
                        if let Some(entity) = vh.upgrade() { cx.update_entity(&entity, |view, cx| {
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
                }})
            })
        .child(
            Button::new("refresh-btn")
                .icon(IconName::Redo2)
                .ghost()
                .label("刷新")
                .on_click({
                    let vh = vh.clone();
                    move |_, _window, cx| {
                        if let Some(entity) = vh.upgrade() { cx.update_entity(&entity, |view, cx| view.dispatch_action(Action::Refresh, cx)); }
                    }
                }),
        )
}

// ── Settings Dialog Overlay ──────────────────────────────────────────
pub fn render_settings_overlay(view: &RootView, cx: &mut Context<RootView>) -> impl IntoElement {
    let vh = cx.entity().downgrade();
    let font = view.config.font_family.clone();
    let thumb_size = view.config.thumbnail_size;
    let delete_mode = view.config.default_delete_mode.clone();
    let import_mode = view.config.import_behavior.clone();
    let cache_size = view.config.max_cache_size_mb;

    let click_font = vh.clone();
    let click_thumb = vh.clone();
    let click_delete = vh.clone();
    let click_import = vh.clone();
    let click_cache = vh.clone();
    let click_close = vh;

    div()
        .size_full()
        .absolute()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgba(0x00000055))
        .on_mouse_down(MouseButton::Left, cx.listener(|v, _, _w, cx| {
            v.show_settings = false;
            cx.notify();
        }))
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
                .on_mouse_down(MouseButton::Left, |_, _, _| {})
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
                                .on_click({
                                    let vh = click_close.clone();
                                    move |_, _, cx| {
                                        if let Some(entity) = vh.upgrade() {
                                            cx.update_entity(&entity, |view, cx| {
                                                view.show_settings = false;
                                                cx.notify();
                                            });
                                        }
                                    }
                                }),
                        ),
                )
                .child(div().h(px(1.)).w_full().bg(theme::colors().border_variant))
                .child(section("界面"))
                .child(setting_toggle("字体", &font, {
                    let vh = click_font;
                    move |_, _, cx| {
                        if let Some(entity) = vh.upgrade() {
                            cx.update_entity(&entity, |view, cx| {
                                let current = view.config.font_family.clone();
                                view.config.font_family = match current.as_str() {
                                    "Microsoft YaHei UI" => "Noto Sans CJK SC".into(),
                                    _ => "Microsoft YaHei UI".into(),
                                };
                                view.save_config();
                                cx.notify();
                            });
                        }
                    }
                }))
                .child(setting_toggle("缩略图尺寸", &format!("{}px", thumb_size), {
                    let vh = click_thumb;
                    move |_, _, cx| {
                        if let Some(entity) = vh.upgrade() {
                            cx.update_entity(&entity, |view, cx| {
                                view.config.thumbnail_size = match view.config.thumbnail_size {
                                    120 => 220,
                                    220 => 320,
                                    _ => 120,
                                };
                                view.save_config();
                                cx.notify();
                            });
                        }
                    }
                }))
                .child(div().h(px(1.)).w_full().bg(theme::colors().border_variant))
                .child(section("文件操作"))
                .child(setting_toggle("默认删除模式", if delete_mode == "trash" { "回收站" } else { "永久删除" }, {
                    let vh = click_delete;
                    move |_, _, cx| {
                        if let Some(entity) = vh.upgrade() {
                            cx.update_entity(&entity, |view, cx| {
                                view.config.default_delete_mode = if view.config.default_delete_mode == "trash" { "permanent".into() } else { "trash".into() };
                                view.save_config();
                                cx.notify();
                            });
                        }
                    }
                }))
                .child(div().h(px(1.)).w_full().bg(theme::colors().border_variant))
                .child(section("导入默认"))
                .child(setting_toggle("导入方式", if import_mode == "copy" { "复制" } else { "移动" }, {
                    let vh = click_import;
                    move |_, _, cx| {
                        if let Some(entity) = vh.upgrade() {
                            cx.update_entity(&entity, |view, cx| {
                                view.config.import_behavior = if view.config.import_behavior == "copy" { "move".into() } else { "copy".into() };
                                view.save_config();
                                cx.notify();
                            });
                        }
                    }
                }))
                .child(div().h(px(1.)).w_full().bg(theme::colors().border_variant))
                .child(section("缓存"))
                .child(setting_toggle("最大缓存", &format!("{} MB", cache_size), {
                    let vh = click_cache;
                    move |_, _, cx| {
                        if let Some(entity) = vh.upgrade() {
                            cx.update_entity(&entity, |view, cx| {
                                view.config.max_cache_size_mb = match view.config.max_cache_size_mb {
                                    256 => 512,
                                    512 => 1024,
                                    1024 => 2048,
                                    _ => 256,
                                };
                                view.save_config();
                                cx.notify();
                            });
                        }
                    }
                }))
                .child(
                    h_flex()
                        .justify_end()
                        .pt_3()
                        .child(
                            Button::new("settings-close")
                                .primary()
                                .label("关闭")
                                .on_click({
                                    let vh = click_close;
                                    move |_, _, cx| {
                                        if let Some(entity) = vh.upgrade() {
                                            cx.update_entity(&entity, |view, cx| {
                                                view.show_settings = false;
                                                cx.notify();
                                            });
                                        }
                                    }
                                }),
                        ),
                )
        )
}
fn section(label: &str) -> impl IntoElement {
    div()
        .text_color(theme::colors().text_muted)
        .font_weight(FontWeight::MEDIUM)
        .child(label.to_string())
}
fn setting_toggle(label: &str, value: &str, on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> impl IntoElement {
    let label_owned = label.to_string();
    let value_owned = value.to_string();
    let btn_id = label_owned.clone();
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .child(
            div()
                .text_color(theme::colors().text_muted)
                .child(label_owned),
        )
        .child(
            Button::new(btn_id)
                .ghost()
                .label(value_owned)
                .on_click(on_click),
        )
}
