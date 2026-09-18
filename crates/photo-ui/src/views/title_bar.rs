//! 标题栏组件（对应 §2.1）。
//!
//! 36px 高，无边框窗口拖拽区：logo + 应用名 + 窗口控制按钮。
//! 目录名、视图 tab 与动作按钮一律在顶栏（§9.1），不在此重复。

use gpui_kit::component::{ActiveTheme as _, Icon, IconName, TitleBar as GpuiTitleBar, h_flex};
use gpui_kit::{Context, IntoElement, Window, div, prelude::*, px};

use crate::state::AppState;

pub fn render_title_bar(
    _state: &AppState,
    _window: &mut Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    GpuiTitleBar::new().child(
        h_flex()
            .w_full()
            .h(px(36.))
            .items_center()
            .px_3()
            .text_xs()
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Icon::new(IconName::Frame)
                            .size(px(16.))
                            .text_color(cx.theme().primary),
                    )
                    .child(
                        div()
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .child("Photo Tool"),
                    ),
            ),
    )
}
