use gpui::*;
use gpui_component::button::{Button, ButtonVariants as _, DropdownButton};
use gpui_component::setting::{SettingField, SettingGroup, SettingItem, SettingPage, Settings};
use gpui_component::setting::NumberFieldOptions;
use gpui_component::{h_flex, v_flex, Disableable, IconName};

use crate::action::{Action, ContextMenuAction};
use crate::state::app::{RootView, SYSTEM_FONTS};
use crate::ui::theme;

/// 设置弹窗独立 View：拥有自身 render 生命周期，不随 RootView 每帧重建
pub struct SettingsOverlay {
    pub vh: gpui::WeakEntity<RootView>,
}

impl Render for SettingsOverlay {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let vh = self.vh.clone();
        let colors = theme::colors();
        let settings = Settings::new("app-settings")
            .page(settings_page(vh.clone()))
            .page(shortcuts_page())
            .page(about_page());

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
            .on_click(cx.listener(|this, _: &ClickEvent, _w, cx| {
                if let Some(view) = this.vh.upgrade() {
                    let _ = cx.update_entity(&view, |view, cx| {
                        view.show_settings = false;
                        view.settings_overlay = None;
                        cx.notify();
                    });
                }
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _w, cx| {
                if event.keystroke.key.as_str() == "escape" {
                    if let Some(view) = this.vh.upgrade() {
                        let _ = cx.update_entity(&view, |view, cx| {
                            view.show_settings = false;
                            view.settings_overlay = None;
                            cx.notify();
                        });
                    }
                }
            }))
            .child(
                div()
                    .w(px(900.))
                    .h(px(680.))
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
                                h_flex()
                                    .items_center()
                                    .justify_between()
                                    .px_4()
                                    .py_3()
                                    .border_b_1()
                                    .border_color(colors.border_variant)
                                    .child(
                                        div().text_base().font_weight(FontWeight::SEMIBOLD).child("设置"),
                                    )
                                    .child(
                                        Button::new("settings-x")
                                            .icon(IconName::Close)
                                            .ghost()
                                            .on_click({
                                                let vh = self.vh.clone();
                                                move |_, _window, cx| {
                                                    if let Some(view) = vh.upgrade() {
                                                        let _ = cx.update_entity(&view, |view, cx| {
                                                            view.show_settings = false;
                                                            view.settings_overlay = None;
                                                            cx.notify();
                                                        });
                                                    }
                                                }
                                            }),
                                    ),
                            )
                            .child(settings),
                    ),
            )
    }
}

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
                        
                        .font_weight(FontWeight::SEMIBOLD)
                        .max_w(px(180.))
                        .truncate()
                        .child(dir_name),
                )
                .child(
                    div()
                        
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
                    DropdownButton::new("recognize-btn")
                        .button(
                            Button::new("recognize-inner")
                                .label("识别")
                                .ghost()
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
                        .label("同步")
                        .outline()
                        .tooltip("重新扫描目录并同步数据库缓存")
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
                        app.update_entity(&e, |view, cx| {
                            view.config.font_family = value.to_string();
                            view.save_config();
                            // save_config 只落盘不触发重绘，补 notify 让新字体立即生效
                            cx.notify();
                        });
                    }
                }
            },
        )
    };

    // ── 识别线程数 ──
    let thread_field = {
        let vh = vh.clone();
        SettingField::<f64>::number_input(
            NumberFieldOptions {
                min: 1.0,
                max: 4.0,
                step: 1.0,
            },
            {
                let vh = vh.clone();
                move |app: &App| {
                    vh.upgrade()
                        .map(|e| e.read(app).config.recognition_thread_count as f64)
                        .unwrap_or(2.0)
                }
            },
            {
                let vh = vh.clone();
                move |value: f64, app: &mut App| {
                    let count = value.round().clamp(1.0, 4.0) as u32;
                    if let Some(e) = vh.upgrade() {
                        app.update_entity(&e, |view, cx| {
                            view.config.recognition_thread_count = count;
                            view.save_config();
                            // save_config 只落盘不触发重绘，补 notify 刷新设置项显示
                            cx.notify();
                        });
                    }
                }
            },
        )
        .default_value(2.0_f64)
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
                .title("识别")
                .description("鸟类识别设置")
                .item(
                    SettingItem::new("识别线程数", thread_field)
                        .description("批量识别时并发的线程数（1-4），线程越多占用内存越高"),
                ),
        )
}

fn shortcuts_page() -> SettingPage {
    SettingPage::new("快捷键")
        .icon(IconName::BookOpen)
        .description("键盘快捷键参考")
        .default_open(true)
        .resettable(false)
        .group(
            SettingGroup::new()
                .title("常用操作")
                .item(SettingItem::render(|_, _, _| {
                    let shortcuts: &[(&str, &str)] = &[
                        ("上一张", "←"),
                        ("下一张", "→"),
                        ("第一张", "Home"),
                        ("最后一张", "End"),
                        ("切换网格/预览", "G"),
                        ("删除到回收站", "Delete"),
                        ("刷新目录", "F5"),
                    ];
                    shortcuts_table(shortcuts).into_any_element()
                }).keywords(["prev", "next", "first", "last", "grid", "preview", "delete", "refresh"])),
        )
        .group(
            SettingGroup::new()
                .title("标记")
                .item(SettingItem::render(|_, _, _| {
                    let shortcuts: &[(&str, &str)] = &[
                        ("评分 1-5 星", "1-5"),
                        ("清除评分", "0"),
                        ("红色标签", "6"),
                        ("黄色标签", "7"),
                        ("绿色标签", "8"),
                        ("蓝色标签", "9"),
                        ("标记为入选", "P"),
                        ("标记为淘汰", "X"),
                        ("清除旗标", "U"),
                    ];
                    shortcuts_table(shortcuts).into_any_element()
                }).keywords(["rating", "label", "flag", "pick", "reject"])),
        )
        .group(
            SettingGroup::new()
                .title("识别")
                .item(SettingItem::render(|_, _, _| {
                    let shortcuts: &[(&str, &str)] = &[
                        ("识别当前图片", "B"),
                        ("识别未识别的", "Ctrl+B"),
                        ("重新识别全部", "Ctrl+Shift+B"),
                        ("切换检测框", "V"),
                    ];
                    shortcuts_table(shortcuts).into_any_element()
                }).keywords(["recognize", "bird", "bbox"])),
        )
        .group(
            SettingGroup::new()
                .title("选择")
                .item(SettingItem::render(|_, _, _| {
                    let shortcuts: &[(&str, &str)] = &[
                        ("全选", "Ctrl+A"),
                        ("取消全选", "Ctrl+D"),
                    ];
                    shortcuts_table(shortcuts).into_any_element()
                }).keywords(["select", "all", "deselect"])),
        )
        .group(
            SettingGroup::new()
                .title("面板")
                .item(SettingItem::render(|_, _, _| {
                    let shortcuts: &[(&str, &str)] = &[
                        ("切换左侧面板", "Ctrl+["),
                        ("切换右侧面板", "Ctrl+]"),
                        ("取消/关闭", "Esc"),
                    ];
                    shortcuts_table(shortcuts).into_any_element()
                }).keywords(["sidebar", "panel", "escape"])),
        )
}

fn shortcuts_table(items: &[(&'static str, &'static str)]) -> impl IntoElement {
    v_flex()
        .gap_2()
        .children(items.iter().map(|&(desc, key)| {
            h_flex()
                .justify_between()
                .items_center()
                .child(desc)
                .child(
                    div()
                        .px_2()
                        .py_0p5()
                        .rounded_sm()
                        .font_family(theme::MONO_FONT_FAMILY)
                        .bg(gpui::hsla(0., 0., 0., 0.06))
                        .text_color(theme::colors().text)
                        .child(key),
                )
        }))
}

fn about_page() -> SettingPage {
    SettingPage::new("关于")
        .icon(IconName::Info)
        .description("版本与技术信息")
        .default_open(true)
        .resettable(false)
        .group(
            SettingGroup::new()
                .title("Photo Tool")
                .description("照片管理与筛选工具")
                .item(SettingItem::render(|_, _, _| {
                    let info: &[(&'static str, &'static str)] = &[
                        ("版本", "0.1.0"),
                        ("Rust 频道", "nightly"),
                        ("UI 框架", "GPUI"),
                        ("组件库", "gpui-component"),
                        ("识别引擎", "ONNX Runtime (DirectML)"),
                        ("检测模型", "YOLOv8n 0.5"),
                        ("分类模型", "bird_model"),
                        ("名录库", "pica_ref.db"),
                    ];
                    shortcuts_table(info).into_any_element()
                }).keywords(["版本", "version", "about", "info", "技术"])),
        )
        .group(
            SettingGroup::new()
                .title("支持的文件格式")
                .item(SettingItem::render(|_, _, _| {
                    let fmts: &[(&'static str, &'static str)] = &[
                        ("RAW", "CR2, CR3, NEF, ARW, DNG, RAF, ORF, RW2, PEF, SRW, 3FR, IIQ, KDC, PXN, X3F, MOS"),
                        ("常规", "JPEG, PNG, TIFF, WEBP, BMP, GIF"),
                    ];
                    shortcuts_table(fmts).into_any_element()
                }).keywords(["format", "raw", "jpeg", "tiff", "png"])),
        )
}

