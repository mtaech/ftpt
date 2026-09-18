//! 幻灯片视图组件（对应 §3.3、§5.3 与 §5.6）。
//!
//! - 全屏单图展示，纯净暗底
//! - 自动播放（3s 间隔），空格键暂停/继续
//! - ← / → 切张并重置计时
//! - 1–5 星评分仅作用于当前展示张
//! - Esc / G 返回来源视图

use gpui_kit::component::{
    IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
};
use gpui_kit::{Context, IntoElement, Window, div, img, prelude::*, px, rgb};

use crate::actions::{Escape, Next, Prev, TogglePlay};
use crate::image::THUMB_SIZE_GRID;
use crate::state::AppState;

pub fn render_slideshow(
    state: &AppState,
    _window: &mut Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement + use<> {
    let total_count = state.display_order.len();
    if total_count == 0 {
        return div()
            .w_full()
            .h_full()
            .bg(rgb(0x0a0a0a))
            .flex()
            .items_center()
            .justify_center()
            .text_sm()
            .text_color(rgb(0x888888))
            .child("当前无照片可播放")
            .into_any_element();
    }

    let pos = state.slideshow_pos.min(total_count - 1);
    let item_idx = state.display_order[pos];
    let meta = state.items.get(item_idx).cloned();

    let display_path = meta.as_ref().map(|m| {
        let p = std::path::PathBuf::from(&m.primary_path);
        if p.exists() {
            p
        } else {
            state
                .image_manager
                .get_thumbnail_path(&m.primary_path, m.file_size.unwrap_or(0), THUMB_SIZE_GRID)
                .unwrap_or(p)
        }
    });

    div()
        .w_full()
        .h_full()
        .bg(rgb(0x0a0a0a))
        .relative()
        .overflow_hidden()
        // ── 居中展示照片 ──
        .child(
            div()
                .w_full()
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .child(if let Some(ref path) = display_path {
                    img(path.clone())
                        .size_full()
                        .object_fit(gpui_kit::ObjectFit::Contain)
                        .into_any_element()
                } else {
                    div().size_full().bg(rgb(0x1a1a1a)).into_any_element()
                }),
        )
        // ── 悬浮信息与控制栏（暗黑极简） ──
        .child(
            h_flex()
                .absolute()
                .bottom_6()
                .left_0()
                .right_0()
                .justify_center()
                .child(
                    h_flex()
                        .items_center()
                        .gap_3()
                        .px_5()
                        .py_2()
                        .rounded_full()
                        .bg(gpui_kit::rgba(0x1e1e24dd))
                        .border_1()
                        .border_color(gpui_kit::rgba(0xffffff22))
                        .shadow(crate::theme::floating_toolbar_shadow(cx))
                        .text_xs()
                        // 计数
                        .child(div().font_medium().text_color(rgb(0xaaaaaa)).child(format!(
                            "{}/{}",
                            pos + 1,
                            total_count
                        )))
                        // 文件名
                        .when_some(meta.as_ref(), |this, m| {
                            let fname = std::path::Path::new(&m.primary_path)
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_default();
                            this.child(div().font_medium().text_color(rgb(0xffffff)).child(fname))
                        })
                        // 鸟种
                        .when_some(
                            meta.as_ref().and_then(|m| m.bird_name.as_ref()),
                            |this, name| {
                                this.child(
                                    div()
                                        .px_2()
                                        .py_0p5()
                                        .rounded_full()
                                        .bg(gpui_kit::rgba(0x22c55e33))
                                        .text_color(rgb(0x4ade80))
                                        .child(name.clone()),
                                )
                            },
                        )
                        .child(div().w(px(1.)).h(px(12.)).bg(rgb(0x444444)))
                        // 上一张 (←)
                        .child(
                            Button::new("prev-slide")
                                .ghost()
                                .xsmall()
                                .icon(IconName::ChevronLeft)
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(Prev), cx);
                                }),
                        )
                        // 播放 / 暂停 (空格)
                        .child(
                            Button::new("toggle-play")
                                .ghost()
                                .xsmall()
                                .icon(if state.slideshow_paused {
                                    IconName::Play
                                } else {
                                    IconName::Pause
                                })
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(TogglePlay), cx);
                                }),
                        )
                        // 下一张 (→)
                        .child(
                            Button::new("next-slide")
                                .ghost()
                                .xsmall()
                                .icon(IconName::ChevronRight)
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(Next), cx);
                                }),
                        )
                        .child(div().w(px(1.)).h(px(12.)).bg(rgb(0x444444)))
                        // 退出 (Esc)
                        .child(
                            Button::new("exit-slideshow")
                                .ghost()
                                .xsmall()
                                .icon(IconName::Close)
                                .label("退出")
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(Escape), cx);
                                }),
                        ),
                ),
        )
        .into_any_element()
}
