//! 带滚动条的滚动区（合成 §9.6 / §9.10 / §10.6 的列表滚动条）。
//!
//! GPUI 的溢出滚动容器**只负责滚、不负责画**：`overflow_y_scroll()` 不产生滚动条，
//! 必须把容器 `track_scroll` 到跨帧持有的 `ScrollHandle` 上，再把 `Scrollbar` 当兄弟
//! 节点叠在同一个 relative 容器里（网格视图 2026-09-19 的做法）。
//!
//! 统计页左栏物种榜 / 右栏照片网格 / 导入弹窗内容此前都是裸 `overflow_y_scroll()`，
//! 胶片条是裸 `overflow_x_scroll()`——能滚但看不见滚动条，长列表里不知道自己在第几屏。
//! 这里抽成公用件，四处共用同一套规格。

use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::{IntoElement, ScrollHandle, div, prelude::*, px};

/// 纵向滚动区：内容放进 `track_scroll` 的滚动容器，右缘叠竖向滚动条。
///
/// 返回 `Div`，尺寸由调用方决定（`.w_full().flex_1()` 等）——wrapper 不写死尺寸，
/// 以免在 v_flex 里用 `size_full()` 顶出父容器（内层滚动容器填满 wrapper 的确定尺寸）。
pub fn scroll_area_v(
    id: &'static str,
    handle: &ScrollHandle,
    content: impl IntoElement,
) -> gpui_kit::Div {
    div()
        .relative()
        .size_full()
        .child(
            div()
                .id(id)
                .size_full()
                .overflow_y_scroll()
                .track_scroll(handle)
                // 内容必须 `flex_shrink_0`：flex 子项默认被压缩到容器高，压缩后
                // 内容高 == 视口高 → 滚动范围为 0（有滚动条、但 thumb 满格、滚不动）。
                // 横向那支同理显式给宽度，见 `scroll_area_h`。
                .child(div().w_full().flex_shrink_0().child(content)),
        )
        .child(Scrollbar::vertical(handle))
}

/// 横向滚动区（胶片条）：内容按 `content_width` 显式撑宽，底部叠横向滚动条。
///
/// **为什么必须显式给宽度**：flex 项默认被拉伸到容器宽，只给 `h_full()` 的横排内容
/// 不会撑出滚动范围（滚动条拿到 0 长度）。胶片条的内容宽 = n × 96 + (n−1) × 6 + 左右 padding。
pub fn scroll_area_h(
    id: &'static str,
    handle: &ScrollHandle,
    content_width: f32,
    content: impl IntoElement,
) -> gpui_kit::Div {
    div()
        .relative()
        .size_full()
        .child(
            div()
                .id(id)
                .size_full()
                .overflow_x_scroll()
                .track_scroll(handle)
                .child(
                    div()
                        .w(px(content_width.max(1.0)))
                        .h_full()
                        .flex_shrink_0()
                        .child(content),
                ),
        )
        .child(Scrollbar::horizontal(handle))
}
