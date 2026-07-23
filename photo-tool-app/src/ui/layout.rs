use gpui_component::{h_flex, v_flex};
use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_component::{Icon, IconName};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::scroll::ScrollableElement;

use crate::action::Action;
use crate::state::app::RootView;
use crate::ui::toolbar::render_settings_overlay;
use crate::ui::theme;

/// Render the three-panel layout: sidebar | content | info_panel.
pub fn render_layout(
    view: &RootView,
    _window: &mut Window,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    let mut wizard_state = crate::ui::import_wizard::ImportWizardState::default();

        v_flex()
        .size_full()
        .bg(theme::colors().background)
        .text_color(theme::colors().text)
        // Keyboard shortcuts
        .on_key_down(cx.listener(
            |view: &mut RootView, event: &KeyDownEvent, _window, cx| {
                let key = event.keystroke.key.as_str();
                let ctrl = event.keystroke.modifiers.control;

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
                    // View
                    ("g", false) => view.dispatch_action(Action::ToggleGridPreview, cx),
                    ("e", false) => view.dispatch_action(Action::ToggleGridPreview, cx),
                    // Navigation
                    ("left", false) => view.dispatch_action(Action::Prev, cx),
                    ("right", false) => view.dispatch_action(Action::Next, cx),
                    // Delete
                    ("delete", false) => view.dispatch_action(Action::Delete, cx),
                    ("delete", true) => view.dispatch_action(Action::PermanentDelete, cx),
                    // Selection
                    ("a", true) => view.dispatch_action(Action::SelectAll, cx),
                    ("d", true) => view.dispatch_action(Action::DeselectAll, cx),
                    // Import
                    ("i", true) => view.dispatch_action(Action::OpenImport, cx),
                    // Refresh
                    ("escape", false) => {
                        if view.show_settings {
                            view.show_settings = false;
                            cx.notify();
                        }
                    }
                    ("f5", false) => view.dispatch_action(Action::Refresh, cx),
                    _ => {}
                }
            },
        ))
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
                    // Left sidebar
                    v_flex()
                        .h_full()
                        .w(px(view.config.left_panel_width as f32))
                        .bg(theme::colors().surface_background)
                        .border_color(theme::colors().border_variant)
                        .overflow_y_scrollbar()
                        .child(crate::ui::sidebar::render_sidebar(view, cx)),
                )
                .child(
                    // Center content area (grid, preview, or empty state)
                    v_flex()
                        .h_full()
                        .flex_grow(1.0)
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
                                            if let Some(path) = rfd::FileDialog::new().pick_folder() {
                                                view.scan_directory(path, cx);
                                            }
                                        })),
                                )
                                .into_any_element()
                        } else {
                            match view.view_mode {
                                crate::state::app::ViewMode::Grid => {
                                    crate::ui::grid::render_grid(view, cx).into_any_element()
                                }
                                crate::state::app::ViewMode::Preview => {
                                    crate::ui::preview::render_preview(view, cx).into_any_element()
                                }
                            }
                        }),
                )
                .child(
                    // Right info panel (conditionally visible)
                    if view.config.right_panel_visible {
                        v_flex()
                            .h_full()
                            .bg(theme::colors().background)
                            .border_color(theme::colors().border_variant)
                            .border_l_1()
                            .child(crate::ui::info_panel::render_info_panel(view, cx))
                            .into_any_element()
                    } else {
                        div().into_any_element()
                    },
                ),
        )
        .child(
            // Bottom status bar
            crate::ui::status_bar::render_status_bar(view, cx),
        )
        // Import wizard overlay
        .when(view.show_import_wizard, |parent| {
            parent.child(crate::ui::import_wizard::render_import_wizard(
                view,
                cx,
                &mut wizard_state,
            ))
        })
        // Settings overlay
        .when(view.show_settings, |parent| {
            parent.child(render_settings_overlay(view, cx))
        })
}
