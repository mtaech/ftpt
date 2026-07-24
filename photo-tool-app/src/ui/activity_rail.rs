use gpui::*;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::{v_flex, IconName};

use crate::action::Action;
use crate::state::app::RootView;
use crate::ui::theme;

/// 左侧 Activity Rail：48px 宽交易终端风格，竖排 icon-only 按钮。
/// 顶部：目录、导入、设置；底部：主题切换（向日葵/月亮）。
pub fn render_activity_rail(
    _view: &RootView,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    let vh = cx.entity().downgrade();
    let is_dark = theme::current_mode() == theme::ThemeMode::Dark;

    v_flex()
        .h_full()
        .w(px(crate::ui::layout::RAIL_WIDTH))
        .bg(theme::colors().surface_background)
        .border_r_1()
        .border_color(theme::colors().border_variant)
        .items_center()
        .pt(px(8.))
        .gap(px(4.))
        // 目录——切换左侧面板显隐
        .child(
            Button::new("rail-dir-btn")
                .icon(IconName::Folder)
                .ghost()
                .tooltip("切换目录面板")
                .on_click({
                    let vh = vh.clone();
                    move |_, _window, cx| {
                        if let Some(entity) = vh.upgrade() {
                            cx.update_entity(&entity, |view, cx| {
                                view.dispatch_action(Action::ToggleLeftPanel, cx);
                            });
                        }
                    }
                }),
        )
        // 设置
        .child(
            Button::new("rail-settings-btn")
                .icon(IconName::Settings)
                .ghost()
                .tooltip("设置")
                .on_click({
                    let vh = vh.clone();
                    move |_, _window, cx| {
                        if let Some(entity) = vh.upgrade() {
                            cx.update_entity(&entity, |view, cx| {
                                view.dispatch_action(Action::ToggleSettings, cx);
                            });
                        }
                    }
                }),
        )
        // 弹性占位，把主题按钮推到底部（注意：不能给 rail 自身加 flex_grow，
        // 否则它在父级 h_flex 行里会横向吃掉剩余空间）
        .child(div().flex_grow(1.0))
        // 右侧面板切换
        .child(
            Button::new("rail-right-panel-btn")
                .icon(IconName::PanelRight)
                .ghost()
                .tooltip("切换右侧面板")
                .on_click({
                    let vh = vh.clone();
                    move |_, _window, cx| {
                        if let Some(entity) = vh.upgrade() {
                            cx.update_entity(&entity, |view, cx| {
                                view.dispatch_action(Action::ToggleRightPanel, cx);
                            });
                        }
                    }
                }),
        )
        .child(
            Button::new("rail-theme-btn")
                .icon(if is_dark { IconName::Sun } else { IconName::Moon })
                .ghost()
                .tooltip("切换主题")
                .on_click({
                    let vh = vh.clone();
                    move |_, _window, cx| {
                        theme::toggle_mode();
                        // 同步 gpui-component 全局主题，组件库控件一并切换
                        let gc_mode = match theme::current_mode() {
                            theme::ThemeMode::Dark => gpui_component::theme::ThemeMode::Dark,
                            theme::ThemeMode::Light => gpui_component::theme::ThemeMode::Light,
                        };
                        gpui_component::theme::Theme::change(gc_mode, None, cx);
                        if let Some(entity) = vh.upgrade() {
                            cx.update_entity(&entity, |view, cx| {
                                // 持久化主题选择
                                view.config.theme = match theme::current_mode() {
                                    theme::ThemeMode::Light => photo_tool_core::config::Theme::Light,
                                    theme::ThemeMode::Dark => photo_tool_core::config::Theme::Dark,
                                };
                                view.save_config();
                                cx.notify();
                            });
                        }
                    }
                }),
        )
}
