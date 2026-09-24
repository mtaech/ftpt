//! 照片右键菜单浮层（§13.4）：键盘导航 + 最大高度滚动。
//!
//! 为什么自绘而不用 gpui-component 的 ContextMenu/PopupMenu：那套只处理点击，
//! 整个 menu 模块没有一处键盘处理（grep 无 on_key_down / ArrowUp / Enter），
//! 直接用等于把「右键菜单无键盘导航、无最大高度滚动」这条缺口原样搬过来。
//!
//! 本模块负责画与鼠标交互；键盘（Up/Down/Enter/Esc）在 app.rs 的根视图
//! on_key_down + handle_escape 里（根视图持有焦点，菜单作为子节点不用抢焦点）。

use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::{
    Context, IntoElement, MouseButton, Role, SharedString, TestSupportExt as _, Window, div,
    prelude::*, px,
};

use crate::model::context_menu::{ContextMenuAction, ITEM_H, menu_height};
use crate::state::AppState;
use crate::state::app_state::PhotoContextMenu;
use crate::state::engine_ops::defer_entity_action;

const MENU_W: f32 = 252.0;

/// 关菜单 + 派发动作（点击与回车共用；动作本身必须 defer 到实体归还之后）
pub fn run_context_menu_action(
    state: &mut AppState,
    action: ContextMenuAction,
    target: String,
    cx: &mut Context<AppState>,
) {
    state.context_menu = None;
    cx.notify();
    let entity = cx.entity().clone();
    defer_entity_action(cx, entity, move |entity, cx| {
        crate::state::engine_ops::apply_context_menu_action(entity, action, target, cx);
    });
}

pub fn render_photo_context_menu(
    state: &AppState,
    menu: &PhotoContextMenu,
    _window: &mut Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement + use<> {
    // 高度按内容给（放得下就不滚动）；只有窗口装不下才限高并滚动
    let viewport = _window.viewport_size();
    let viewport_h = f32::from(viewport.height);
    let height = menu_height(menu.items.len(), viewport_h);
    let x = menu
        .x
        .min((f32::from(viewport.width) - MENU_W - 4.0).max(0.0));
    let y = menu
        .y
        .min((f32::from(viewport.height) - height - 4.0).max(0.0));

    let rows = v_flex()
        .w_full()
        .py_1()
        .children(menu.items.iter().enumerate().map(|(i, item)| {
            let action = item.action;
            let target = menu.target.clone();
            let disabled = item.disabled;
            let highlighted = i == menu.selected && !disabled;
            div()
                .id(SharedString::from(format!("context-menu-item-{i}")))
                .w_full()
                .h(px(ITEM_H))
                .px_3()
                .flex()
                .items_center()
                .when(disabled, |this| this.opacity(0.4))
                .when(!disabled, |this| this.cursor_pointer())
                .when(highlighted, |this| this.bg(cx.theme().selection))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().foreground)
                        .child(item.label.clone()),
                )
                .on_mouse_move(cx.listener(move |state, _, _, cx| {
                    if disabled {
                        return;
                    }
                    if let Some(menu) = &mut state.context_menu {
                        if menu.selected != i {
                            menu.selected = i;
                            cx.notify();
                        }
                    }
                }))
                .when(!disabled, |this| {
                    this.on_mouse_down(MouseButton::Left, cx.listener(move |state, _, _, cx| {
                        run_context_menu_action(state, action, target.clone(), cx);
                    }))
                })
                // 无障碍（§13.4）：菜单项 = MenuItem + 名称；键盘高亮项报 selected。
                // a11y 里没有 aria-disabled 通道，置灰项在名称里明说「（不可用）」。
                .role(Role::MenuItem)
                .aria_label(crate::model::menu_item_label(&item.label, disabled))
                .aria_selected(highlighted)
                .test_support()
        }));

    div()
        .id("context-menu-overlay")
        .occlude()
        .absolute()
        .inset_0()
        // 背景层：点空白关菜单。卡片是它的**后一个兄弟**（在它之上），
        // 所以点菜单项不会同时命中这层（GPUI 命中取最上面的元素）。
        .child(
            div()
                .id("context-menu-backdrop")
                .absolute()
                .inset_0()
                .on_mouse_down(MouseButton::Left, cx.listener(|state, _, _, cx| {
                    state.context_menu = None;
                    cx.notify();
                })),
        )
        .child(
            v_flex()
                .id("context-menu-card")
                .role(Role::Menu)
                .aria_label("照片操作菜单")
                .test_support()
                .absolute()
                .left(px(x))
                .top(px(y))
                .w(px(MENU_W))
                .h(px(height))
                .bg(cx.theme().popover)
                .rounded(px(10.))
                .border_1()
                .border_color(cx.theme().border.opacity(0.7))
                .shadow(crate::theme::overlay_shadow())
                .overflow_hidden()
                .child(h_flex().size_full().child(
                    crate::views::scroll_area::scroll_area_v(
                        "context-menu-scroll",
                        &state.context_menu_scroll,
                        rows,
                    ),
                )),
        )
}
