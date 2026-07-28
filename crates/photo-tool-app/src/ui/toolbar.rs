use gpui::*;
use gpui_component::button::{Button, ButtonVariants as _, DropdownButton};
use gpui_component::setting::{SettingField, SettingGroup, SettingItem, SettingPage, Settings};
use gpui_component::{h_flex, v_flex, Disableable, IconName, Sizable};

use crate::action::{Action, ContextMenuAction};
use crate::state::app::{RootView, SYSTEM_FONTS};
use crate::ui::theme;
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
                    div()
                        .w(px(120.))
                        .child(
                            div()
                                .text_sm()
                                .text_color(theme::colors().text)
                                .child("排序: 文件名")
                        ),
                )
                .child(
                    DropdownButton::new("recognize-btn")
                        .button(
                            Button::new("recognize-inner")
                                .label("识别")
                                .ghost()
                                .small()
                        )
                        .disabled(view.batch_recognizing)
                        .dropdown_menu(move |menu, _, _| {
                            menu
                                .menu("识别未识别照片  ctrl+b", Box::new(ContextMenuAction(Action::RecognizeUnrecognized)))
                                .menu("重新识别全部…  ctrl+shift+b", Box::new(ContextMenuAction(Action::RecognizeAll)))
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

// ── 设置弹窗（gpui-component Settings，story 风格）────────────────

fn settings_page(vh: WeakEntity<RootView>) -> SettingPage {
    // ── 字体下拉 ──
    let fonts: Vec<(SharedString, SharedString)> = SYSTEM_FONTS
        .iter()
        .map(|f| {
            let s: SharedString = f.clone().into();
            (s.clone(), s)
        })
        .collect();
    let font_field = {
        let vh = vh.clone();
        SettingField::<SharedString>::scrollable_dropdown(
            fonts,
            {
                let vh = vh.clone();
                move |app: &App| {
                    vh.upgrade()
                        .map(|e| e.read(app).config.font_family.clone().into())
                        .unwrap_or_default()
                }
            },
            {
                let vh = vh.clone();
                move |value: SharedString, app: &mut App| {
                    if let Some(e) = vh.upgrade() {
                        app.update_entity(&e, |view, _cx| {
                            view.config.font_family = value.to_string();
                            view.save_config();
                        });
                    }
                }
            },
        )
    };

    // ── 删除模式 ──
    let delete_field = {
        let vh = vh.clone();
        SettingField::<SharedString>::dropdown(
            vec![
                ("trash".into(), "回收站".into()),
                ("permanent".into(), "永久删除".into()),
            ],
            {
                let vh = vh.clone();
                move |app: &App| {
                    vh.upgrade()
                        .map(|e| e.read(app).config.default_delete_mode.clone().into())
                        .unwrap_or_default()
                }
            },
            {
                let vh = vh.clone();
                move |value: SharedString, app: &mut App| {
                    if let Some(e) = vh.upgrade() {
                        app.update_entity(&e, |view, _cx| {
                            view.config.default_delete_mode = value.to_string();
                            view.save_config();
                        });
                    }
                }
            },
        )
    };

    // ── 缓存大小 ──
    let cache_field = {
        let vh = vh.clone();
        SettingField::<SharedString>::dropdown(
            vec![
                ("256".into(), "256 MB".into()),
                ("512".into(), "512 MB".into()),
                ("1024".into(), "1 GB".into()),
                ("2048".into(), "2 GB".into()),
            ],
            {
                let vh = vh.clone();
                move |app: &App| {
                    vh.upgrade()
                        .map(|e| {
                            let size = e.read(app).config.max_cache_size_mb;
                            SharedString::from(size.to_string())
                        })
                        .unwrap_or_default()
                }
            },
            {
                let vh = vh.clone();
                move |value: SharedString, app: &mut App| {
                    if let Ok(size) = value.parse::<u64>() {
                        if let Some(e) = vh.upgrade() {
                            app.update_entity(&e, |view, _cx| {
                                view.config.max_cache_size_mb = size;
                                view.save_config();
                            });
                        }
                    }
                }
            },
        )
    };

    SettingPage::new("通用")
        .icon(IconName::Settings)
        .description("应用通用设置")
        .default_open(true)
        .resettable(false)
        .group(
            SettingGroup::new()
                .title("界面")
                .description("字体与外观")
                .item(
                    SettingItem::new("字体", font_field)
                        .description("应用界面字体"),
                ),
        )
        .group(
            SettingGroup::new()
                .title("文件操作")
                .description("删除与移动")
                .item(
                    SettingItem::new("默认删除模式", delete_field)
                        .description("删除文件时的默认操作"),
                ),
        )
        .group(
            SettingGroup::new()
                .title("缓存")
                .description("缩略图缓存")
                .item(
                    SettingItem::new("最大缓存", cache_field)
                        .description("缩略图磁盘缓存上限"),
                ),
        )
}

pub fn render_settings_overlay(_view: &RootView, cx: &mut Context<RootView>) -> impl IntoElement {
    let vh = cx.entity().downgrade();
    let colors = theme::colors();
    let settings = Settings::new("app-settings").page(settings_page(vh));

    div()
        .size_full()
        .absolute()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgba(0x00000033))
        .id("settings-backdrop")
        .occlude()
        .on_scroll_wheel(|_, _, _| {})
        .on_click(cx.listener(|v, _: &ClickEvent, _w, cx| {
            v.show_settings = false;
            cx.notify();
        }))
        .on_key_down(cx.listener(|v, event: &KeyDownEvent, _w, cx| {
            if event.keystroke.key.as_str() == "escape" {
                v.show_settings = false;
                cx.notify();
            }
        }))
        .child(
            div()
                .w(px(720.))
                .h(px(560.))
                .bg(colors.elevated_surface_background)
                .rounded_lg()
                .border_1()
                .border_color(colors.border)
                .shadow(theme::ElevationIndex::ModalSurface.shadow())
                .overflow_hidden()
                .id("settings-card")
                .on_click(|_, _, cx| cx.stop_propagation())
                .child(
                    v_flex()
                        .size_full()
                        .child(
                            // ── Header ──
                            h_flex()
                                .items_center()
                                .justify_between()
                                .px_4()
                                .py_3()
                                .border_b_1()
                                .border_color(colors.border_variant)
                                .child(
                                    div()
                                        .text_base()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child("设置"),
                                )
                                .child(
                                    Button::new("settings-x")
                                        .icon(IconName::Close)
                                        .ghost()
                                        .on_click(cx.listener(|v, _, _w, cx| {
                                            v.show_settings = false;
                                            cx.notify();
                                        })),
                                ),
                        )
                        .child(settings),
                ),
        )
}
