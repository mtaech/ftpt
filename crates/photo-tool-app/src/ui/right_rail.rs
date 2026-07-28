use gpui::*;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::{IconName, v_flex};

use crate::action::Action;
use crate::state::app::RootView;
use crate::ui::theme;

/// 右侧 Activity Rail：48px 宽，与左侧 rail 对称。
/// 顶部预留空间给后续右侧面板相关图标按钮。
pub fn render_right_rail(
    view: &RootView,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    let vh = cx.entity().downgrade();

    v_flex()
        .h_full()
        .w(px(crate::ui::layout::RAIL_WIDTH))
        .bg(theme::colors().surface_background)
        .border_l_1()
        .border_color(theme::colors().border_variant)
        .items_center()
        .pt(px(8.))
        .gap(px(4.))
        // 右侧面板切换
        .child(
            Button::new("rail-right-panel-btn")
                .icon(if view.config.right_panel_visible {
                    IconName::PanelRight
                } else {
                    IconName::PanelRightOpen
                })
                .ghost()
                .tooltip(if view.config.right_panel_visible {
                    "隐藏右侧面板"
                } else {
                    "显示右侧面板"
                })
                .on_click({
                    let vh = vh.clone();
                    move |_, _window, cx| {
                        if let Some(e) = vh.upgrade() {
                            let _ = cx.update_entity(&e, |view, cx| {
                                view.dispatch_action(Action::ToggleRightPanel, cx);
                            });
                        }
                    }
                }),
        )
        // 弹性占位，后续图标按钮加在这里
        .child(div().flex_grow(1.0))
}
