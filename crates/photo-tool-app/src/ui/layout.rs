use gpui_component::{h_flex, v_flex};
use gpui_component::resizable::{h_resizable, resizable_panel};
use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_component::{Icon, IconName};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::scroll::ScrollableElement;

use crate::action::{Action, ContextMenuAction};
use crate::state::app::RootView;
use crate::ui::toolbar::render_settings_overlay;
use crate::ui::theme;

/// 左侧 Activity Rail 固定宽度（px），网格列宽计算依赖该值
pub const RAIL_WIDTH: f32 = 48.0;

/// Render the three-panel layout: sidebar | content | info_panel.
pub fn render_layout(
    view: &RootView,
    window: &mut Window,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    let font_family = view.config.font_family.clone();

        v_flex()
        .id("root-layout")
        .focusable()
        .text_color(theme::colors().text)
        .font_family(font_family)
        .size_full()
        .bg(theme::colors().background)
        // Keyboard shortcuts
        .on_key_down(cx.listener(
            |view: &mut RootView, event: &KeyDownEvent, _window, cx| {
                let key = event.keystroke.key.as_str();
                let ctrl = event.keystroke.modifiers.control;
                let shift = event.keystroke.modifiers.shift;

                match (key, ctrl) {
                    // Rating
                    ("1", false) => view.dispatch_action(Action::Rate1, cx),
                    ("2", false) => view.dispatch_action(Action::Rate2, cx),
                    ("3", false) => view.dispatch_action(Action::Rate3, cx),
                    ("4", false) => view.dispatch_action(Action::Rate4, cx),
                    ("5", false) => view.dispatch_action(Action::Rate5, cx),
                    ("0", false) => view.dispatch_action(Action::Rate0, cx),
                    // Color labels
                    ("6", false) => view.dispatch_action(Action::LabelRed, cx),
                    ("7", false) => view.dispatch_action(Action::LabelYellow, cx),
                    ("8", false) => view.dispatch_action(Action::LabelGreen, cx),
                    ("9", false) => view.dispatch_action(Action::LabelBlue, cx),
                    // Flags
                    ("p", false) => view.dispatch_action(Action::FlagPick, cx),
                    ("x", false) => view.dispatch_action(Action::FlagReject, cx),
                    ("u", false) => view.dispatch_action(Action::FlagNone, cx),
                    // Recognition
                    ("b", false) => view.dispatch_action(Action::Recognize, cx),
                    ("b", true) => {
                        if shift {
                            view.dispatch_action(Action::RecognizeAll, cx);
                        } else {
                            view.dispatch_action(Action::RecognizeUnrecognized, cx);
                        }
                    }
                    ("v", false) => view.dispatch_action(Action::ToggleBbox, cx),
                    // View
                    ("g", false) => view.dispatch_action(Action::ToggleGridPreview, cx),
                    ("left", false) => view.dispatch_action(Action::Prev, cx),
                    ("right", false) => view.dispatch_action(Action::Next, cx),
                    ("home", false) => view.dispatch_action(Action::First, cx),
                    ("end", false) => view.dispatch_action(Action::Last, cx),
                    // Delete
                    ("delete", false) => view.dispatch_action(Action::Delete, cx),
                    ("delete", true) => view.dispatch_action(Action::PermanentDelete, cx),
                    // Selection
                    ("a", true) => view.dispatch_action(Action::SelectAll, cx),
                    ("d", true) => view.dispatch_action(Action::DeselectAll, cx),
                    // Refresh / Cancel
                    ("escape", false) => {
                        if view.show_settings {
                            view.show_settings = false;
                            cx.notify();
                        } else if view.batch_recognizing {
                            view.dispatch_action(Action::CancelBatchRecognize, cx);
                        }
                    }
                    ("f5", false) => view.dispatch_action(Action::Refresh, cx),
                    // Panel toggles
                    ("[", true) => view.dispatch_action(Action::ToggleLeftPanel, cx),
                    ("]", true) => view.dispatch_action(Action::ToggleRightPanel, cx),
                    _ => {}
                }
            },
        ))
        // 右键菜单命令（网格/预览图片的 ContextMenu）统一在这里分发
        .on_action({
            let vh = cx.entity().downgrade();
            move |action: &ContextMenuAction, _window, app| {
                if let Some(view) = vh.upgrade() {
                    let _ = app.update_entity(&view, |root_view, cx| {
                        root_view.dispatch_action(action.0, cx);
                    });
                }
            }
        })
        .child(
            // Toolbar at the top
            crate::ui::toolbar::render_toolbar(view, cx),
        )
        .child(
            // Main three-panel area
            h_flex()
                .h_full()
                .flex_grow(1.0)
                .child(
                    // Activity Rail（始终可见）
                    crate::ui::activity_rail::render_activity_rail(view, cx),
                )
                .child(
                    // 三栏可拖拽布局：左栏 | 内容区 | 右栏（gpui-component h_resizable）
                    h_resizable("main-panels")
                        .child(
                            resizable_panel()
                                .size(px(view.config.left_panel_width as f32))
                                .size_range(px(200.)..px(480.))
                                .flex_none()
                                .visible(view.sidebar_visible)
                                .child(
                                    v_flex()
                                        .size_full()
                                        .bg(theme::colors().surface_background)
                                        .border_color(theme::colors().border_variant)
                                        .overflow_y_scrollbar()
                                        .child(crate::ui::sidebar::render_sidebar(view, cx))
                                        .into_any_element(),
                                ),
                        )
                        .child(
                            resizable_panel()
                                .min_w_0()
                                .child(
                                    // Center content area (grid, preview, or empty state)
                                    v_flex()
                                        .h_full()
                                        .w_full()
                                        .overflow_hidden()
                                        .child(if view.dir_path.is_none() {
                                            // Empty state: no directory loaded
                                            v_flex()
                                                .h_full()
                                                .flex_grow(1.0)
                                                .items_center()
                                                .justify_center()
                                                .gap_3()
                                                .child(
                                                    Icon::new(IconName::GalleryVerticalEnd)
                                                        .text_color(theme::colors().text_muted.opacity(0.2)),
                                                )
                                                .child(
                                                    div()
                                                        .text_color(theme::colors().text_muted)
                                                        .child("打开目录开始浏览照片"),
                                                )
                                                .child(
                                                    Button::new("empty-open-dir")
                                                        .icon(IconName::FolderOpen)
                                                        .primary()
                                                        .label("打开目录")
                                                        .on_click(cx.listener(|view, _event: &ClickEvent, _window, cx| {
                                                            view.pick_and_scan_directory(cx);
                                                        })),
                                                )
                                                .into_any_element()
                                        } else {
                                            match view.view_mode {
                                                crate::state::app::ViewMode::Grid => {
                                                    crate::ui::grid::render_grid(view, window, cx).into_any_element()
                                                }
                                                crate::state::app::ViewMode::Preview => {
                                                crate::ui::preview::render_preview(view, window, cx).into_any_element()
                                                }
                                            }
                                        })
                                        .into_any_element(),
                                ),
                        )
                        .child(
                            resizable_panel()
                                .size(px(view.config.right_panel_width as f32))
                                .size_range(px(200.)..px(480.))
                                .flex_none()
                                .visible(view.config.right_panel_visible)
                                .child(
                                    v_flex()
                                        .size_full()
                                        .bg(theme::colors().background)
                                        .border_color(theme::colors().border_variant)
                                        .border_l_1()
                                        .child(crate::ui::info_panel::render_info_panel(view, cx))
                                        .into_any_element(),
                                ),
                        )
                        .on_resize({
                            let vh = cx.entity().downgrade();
                            move |state, _window, cx| {
                                let sizes = state.read(cx).sizes().clone();
                                let Some(view) = vh.upgrade() else { return };
                                cx.update_entity(&view, |this, _cx| {
                                    // 隐藏的面板 size 为 0，不覆盖其已存宽度
                                    if this.sidebar_visible {
                                        if let Some(w) = sizes.first() {
                                            let wf: f32 = (*w).into();
                                            this.config.left_panel_width = wf.round().max(0.) as u32;
                                        }
                                    }
                                    if this.config.right_panel_visible {
                                        if let Some(w) = sizes.get(2) {
                                            let wf: f32 = (*w).into();
                                            this.config.right_panel_width = wf.round().max(0.) as u32;
                                        }
                                    }
                                    // on_resize 仅在拖拽结束（mouse up）触发，直接落盘
                                    this.save_config();
                                });
                            }
                        }),
                ),
        )
        .child(
            // Bottom status bar
            crate::ui::status_bar::render_status_bar(view, cx),
        )
        // Settings overlay
        .when(view.show_settings, |parent| {
            parent.child(render_settings_overlay(view, cx))
        })
        // 批量操作进度弹窗
        // 批量全量识别确认对话框
        .when(view.show_recognize_all_confirm, |parent| {
            let vh = cx.entity().downgrade();
            let n = view.captures.len();
            parent.child(
                div()
                    .absolute()
                    .size_full()
                    .bg(hsla(0., 0., 0., 0.4))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .w(px(400.))
                            .bg(theme::colors().surface_background)
                            .rounded_lg()
                            .shadow_lg()
                            .child(
                                v_flex()
                                    .p_4()
                                    .gap_3()
                                    .child(
                                        div()
                                            .text_base()
                                            .font_weight(FontWeight::BOLD)
                                            .child("重新识别全部"),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(theme::colors().text_muted)
                                            .child(format!("将重新识别 {} 张照片，已有的识别结果会被覆盖。", n)),
                                    )
                                    .child(
                                        h_flex()
                                            .justify_end()
                                            .gap_2()
                                            .pt_2()
                                            .child(
                                                Button::new("cancel-recognize-all")
                                                    .label("取消")
                                                    .ghost()
                                                    .on_click({
                                                        let vh = vh.clone();
                                                        move |_, _window, cx| {
                                                            if let Some(e) = vh.upgrade() {
                                                                cx.update_entity(&e, |view, cx| {
                                                                    view.show_recognize_all_confirm = false;
                                                                    cx.notify();
                                                                });
                                                            }
                                                        }
                                                    }),
                                            )
                                            .child(
                                                Button::new("confirm-recognize-all")
                                                    .label("确认")
                                                    .primary()
                                                    .on_click({
                                                        let vh = vh.clone();
                                                        move |_, _window, cx| {
                                                            if let Some(e) = vh.upgrade() {
                                                                cx.update_entity(&e, |view, cx| {
                                                                    view.show_recognize_all_confirm = false;
                                                                    view.dispatch_action(Action::ConfirmRecognizeAll, cx);
                                                                });
                                                            }
                                                        }
                                                    }),
                                            ),
                                    ),
                            ),
                    ),
            )
        })
        .when(view.batch_show_progress_popup, |parent| {
            let vh = cx.entity().downgrade();
            let (done, total) = view.batch_progress.unwrap_or((0, 1));
            let pct = if total > 0 { done as f32 / total as f32 } else { 0.0 };
            let results = view.batch_results.clone();
            parent.child(
                // 半透明遮罩
                div()
                    .absolute()
                    .size_full()
                    .bg(hsla(0., 0., 0., 0.4))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        // 弹窗卡片
                        div()
                            .w(px(400.))
                            .bg(theme::colors().surface_background)
                            .rounded_md()
                            .shadow_lg()
                            .flex()
                            .flex_col()
                            .child(
                                // 标题栏
                                h_flex()
                                    .items_center()
                                    .justify_between()
                                    .px_4()
                                    .py_3()
                                    .border_b_1()
                                    .border_color(theme::colors().border_variant)
                                    .child(div().font_weight(FontWeight::BOLD).text_sm().child("批量操作进度"))
                                    .child(
                                        div()
                                            .id("close-progress-popup")
                                            .text_color(theme::colors().text_muted)
                                            .cursor_pointer()
                                            .hover(|style| style.text_color(theme::colors().text))
                                            .child("✕")
                                            .on_click(move |_, _window, cx| {
                                                if let Some(e) = vh.upgrade() {
                                                    cx.update_entity(&e, |view, cx| {
                                                        view.batch_show_progress_popup = false;
                                                        cx.notify();
                                                    });
                                                }
                                            }),
                                    ),
                            )
                            .child(
                                // 内容区
                                div()
                                    .px_4()
                                    .py_3()
                                    .flex()
                                    .flex_col()
                                    .gap_3()
                                    .child(
                                        // 进度条
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .h(px(8.))
                                                    .rounded_full()
                                                    .bg(theme::colors().element_background)
                                                    .overflow_hidden()
                                                    .child(
                                                        div()
                                                            .h_full()
                                                            .bg(theme::colors().text_accent)
                                                            .w(px((200.0 * pct).max(2.0).min(200.0)))
                                                            .rounded_full(),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(theme::colors().text)
                                                    .font_family(theme::MONO_FONT_FAMILY)
                                                    .child(format!("{done}/{total}")),
                                            ),
                                    )
                                    .child(
                                        // 状态消息
                                        div()
                                            .text_xs()
                                            .text_color(theme::colors().text_muted)
                                            .child({
                                                let msg = if view.batch_progress_msg.is_empty() {
                                                    if view.batch_in_progress {
                                                        format!("处理中: {done}/{total}")
                                                    } else {
                                                        "已完成".to_string()
                                                    }
                                                } else {
                                                    view.batch_progress_msg.clone()
                                                };
                                                msg
                                            }),
                                    )
                                    .child(
                                        // 结果列表
                                        div()
                                            .flex_1()
                                            .max_h(px(200.))
                                            .overflow_y_scrollbar()
                                            .children(results.iter().map(|msg| {
                                                let is_err = msg.contains("失败");
                                                div()
                                                    .py_0p5()
                                                    .text_xs()
                                                    .text_color(if is_err {
                                                        theme::colors().error
                                                    } else {
                                                        theme::colors().text
                                                    })
                                                    .child(msg.clone())
                                                    .into_any_element()
                                            })),
                                    ),
                            ),
                    ),
            )
        })
}
