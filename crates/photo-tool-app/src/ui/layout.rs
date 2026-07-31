use gpui_component::{h_flex, v_flex};
use gpui_component::resizable::{h_resizable, resizable_panel};
use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_component::{Icon, IconName};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::scroll::ScrollableElement;

use crate::action::{Action, ContextMenuAction};
use crate::state::app::RootView;
use crate::ui::theme;

/// 左侧 Activity Rail 固定宽度（px），网格列宽计算依赖该值
pub const RAIL_WIDTH: f32 = 48.0;

/// Render the three-panel layout: sidebar | content | info_panel.
pub fn render_layout(
    view: &mut RootView,
    window: &mut Window,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    // 调整视图激活（右侧面板调整 tab + 非中性参数）时，确保调整显示源/渲染就绪。
    // preview.rs 的 render_preview 签名是 &RootView（拿不到 &mut），故在此（渲染三栏之前）挂载。
    if view.right_panel_tab == 1 && !view.current_adjust.is_neutral() {
        view.ensure_adjust_ready(cx);
    }

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
                    ("delete", _) => view.dispatch_action(Action::Delete, cx),
                    // Selection
                    ("a", true) => view.dispatch_action(Action::SelectAll, cx),
                    ("d", true) => view.dispatch_action(Action::DeselectAll, cx),
                    // Refresh / Cancel
                    ("escape", false) => {
                        if view.show_settings {
                            view.dispatch_action(Action::ToggleSettings, cx);
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
        // 拖拽面板时逐帧重渲染，使 grid 能动态重算可用宽度。
        // 仅在按住鼠标（拖拽进行中）时 notify，避免鼠标移动引发持续重绘。
        .on_mouse_move({
            let vh = cx.entity().downgrade();
            move |event: &MouseMoveEvent, _window, cx| {
                if event.pressed_button.is_none() {
                    return;
                }
                if let Some(view) = vh.upgrade() {
                    let visible = view.read(cx).sidebar_visible
                        || view.read(cx).config.right_panel_visible;
                    if visible {
                        let _ = cx.update_entity(&view, |_, cx| cx.notify());
                    }
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
                    // 必须套 flex_1（basis=0）槽位：h_resizable 的内容固有宽度是各面板
                    // basis 之和（含 ResizableState 记住的旧尺寸），flex_grow 只增不减，
                    // 超过剩余空间时整行溢出、右 rail 与右面板被挤出屏幕。
                    div()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .child(
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
                                        // 筛选栏：仅网格模式 + 已有目录时显示
                                        .when(
                                            view.dir_path.is_some()
                                                && view.view_mode == crate::state::app::ViewMode::Grid,
                                            |parent| {
                                                parent.child(crate::ui::filter_bar::render_filter_bar(
                                                    &mut *view,
                                                    &mut *window,
                                                    cx,
                                                ))
                                            },
                                        )
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_h_0()
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
                                                }),
                                        )
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
                                        .child(crate::ui::info_panel::render_info_panel(
                                            view, window, cx,
                                        ))
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
                    // 右侧 Activity Rail（与左侧 rail 对称）
                    crate::ui::right_rail::render_right_rail(view, cx),
                ),
        )
        .child(
            // Bottom status bar
            crate::ui::status_bar::render_status_bar(view, cx),
        )
        // Settings overlay
        .when(view.show_settings, |parent| {
            if let Some(overlay) = &view.settings_overlay {
                parent.child(overlay.clone())
            } else {
                parent
            }
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
        // 批量删除确认对话框（ADR 0006：删除强制确认，含同名同步数量）
        .when(view.batch_delete_confirm.is_some(), |parent| {
            let vh = cx.entity().downgrade();
            let preview = view.batch_delete_confirm.clone().expect("checked");
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
                            .w(px(420.))
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
                                            .text_color(theme::colors().error)
                                            .child("删除文件"),
                                    )
                                    .child(
                                        div()
                                            
                                            .text_color(theme::colors().text_muted)
                                            .child(if preview.synced > 0 {
                                                format!("将删除 {} 个文件（其中 {} 个来自同名同步），移入回收站。",
                                                    preview.total, preview.synced)
                                            } else {
                                                format!("将删除 {} 个文件，移入回收站。", preview.total)
                                            }),
                                    )
                                    .child(
                                        // 清单（前 20 条）
                                        div()
                                            .max_h(px(160.))
                                            .overflow_y_scrollbar()
                                            .rounded_sm()
                                            .bg(theme::colors().element_background)
                                            .children(preview.files.iter().map(|f| {
                                                div()
                                                    .py_0p5()
                                                    .px_1()
                                                    .text_size(px(11.))
                                                    .text_color(theme::colors().text)
                                                    .child(f.clone())
                                                    .into_any_element()
                                            })),
                                    )
                                    .child(
                                        h_flex()
                                            .justify_end()
                                            .gap_2()
                                            .pt_2()
                                            .child(
                                                Button::new("cancel-batch-delete")
                                                    .label("取消")
                                                    .ghost()
                                                    .on_click({
                                                        let vh = vh.clone();
                                                        move |_, _window, cx| {
                                                            if let Some(e) = vh.upgrade() {
                                                                cx.update_entity(&e, |view, cx| {
                                                                    view.cancel_batch_delete(cx);
                                                                });
                                                            }
                                                        }
                                                    }),
                                            )
                                            .child(
                                                Button::new("confirm-batch-delete")
                                                    .label("确认删除")
                                                    .danger()
                                                    .on_click({
                                                        let vh = vh.clone();
                                                        move |_, _window, cx| {
                                                            if let Some(e) = vh.upgrade() {
                                                                cx.update_entity(&e, |view, cx| {
                                                                    view.confirm_batch_delete(cx);
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
                                    .child(div().font_weight(FontWeight::BOLD).child("批量操作进度"))
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
                                                    
                                                    .text_color(theme::colors().text)
                                                    .font_family(theme::MONO_FONT_FAMILY)
                                                    .child(format!("{done}/{total}")),
                                            ),
                                    )
                                    .child(
                                        // 状态消息
                                        div()
                                            
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
