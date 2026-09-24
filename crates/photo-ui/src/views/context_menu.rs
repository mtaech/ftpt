//! 照片右键菜单浮层（§13.4）：键盘导航 + 最大高度滚动。
//!
//! 为什么自绘而不用 gpui-component 的 ContextMenu/PopupMenu：那套只处理点击，
//! 整个 menu 模块没有一处键盘处理（grep 无 on_key_down / ArrowUp / Enter），
//! 直接用等于把「右键菜单无键盘导航、无最大高度滚动」这条缺口原样搬过来。
//!
//! 本模块负责画与鼠标交互；键盘（Up/Down/Enter/Esc）在 app.rs 的根视图
//! on_key_down + handle_escape 里（根视图持有焦点，菜单作为子节点不用抢焦点）。

use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::{Context, IntoElement, MouseButton, SharedString, Window, div, prelude::*, px};

use crate::model::context_menu::ContextMenuAction;
use crate::state::AppState;
use crate::state::app_state::PhotoContextMenu;
use crate::state::engine_ops::defer_entity_action;

/// 单项高度（与 model/context_menu.rs 里「10 项超过最大高」的注释同口径）
const ITEM_H: f32 = 28.0;
/// 菜单最大高度：超过就滚动（§13.4）
const MAX_H: f32 = 240.0;
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
    let height = (menu.items.len() as f32 * ITEM_H + 8.0).min(MAX_H);
    // 贴着窗口右下角打开时往回收，别把菜单画到窗口外面去
    let viewport = _window.viewport_size();
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
