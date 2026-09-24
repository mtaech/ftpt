//! 直方图与剪切统计的渲染（§7.4）：调整 tab 与信息 tab Hero 共用同一块卡片。
//!
//! 统计在 photo_engine::histogram，显示口径的纯换算在 model::histogram，
//! 这里只负责把两组柱高画出来：当前图（实柱）叠在原图基线（幽灵柱）上。

use gpui_kit::component::{ActiveTheme as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::{Context, IntoElement, div, prelude::*, px};
use photo_engine::histogram::HistogramData;

use crate::model::histogram::{bin_heights, clip_percent, luma_bins};
use crate::state::AppState;

/// 图表高度（px）：与右栏两张卡片的行高相称，也不至于把滑杆挤出视野。
const CHART_H: f32 = 64.0;

/// 直方图卡片：当前图（实柱）叠在原图基线（幽灵柱）上，右上角给高光/死黑百分比。
///
/// data / baseline 为 None 时只画空底板——直方图是辅助信息，加载中保持安静，
/// 不弹「加载中」文案（与缩略图占位同一态度）。
pub fn render_histogram_card(
    data: Option<&HistogramData>,
    baseline: Option<&HistogramData>,
    cx: &mut Context<AppState>,
) -> impl IntoElement + use<> {
    let bins = data.map(luma_bins);
    let base_bins = baseline.map(luma_bins);
    // 两组柱高共用同一个峰值（当前 + 基线的最大桶），否则两张图不可比
    let heights = bins
        .as_ref()
        .map(|cur| bin_heights(cur, base_bins.as_deref(), CHART_H));
    let base_heights = base_bins
        .as_ref()
        .map(|base| bin_heights(base, bins.as_deref(), CHART_H));
    let (high_pct, low_pct) = data.map(clip_percent).unwrap_or((0.0, 0.0));

    let ghost_color = cx.theme().muted_foreground;
    let solid_color = cx.theme().primary;

    // 先基线（幽灵柱）、后当前图（实柱）：同尺寸叠放，后者画在上面
    let baseline_layer = match base_heights {
        Some(hs) => h_flex()
            .absolute()
            .top(px(0.))
            .left(px(0.))
            .w_full()
            .h_full()
            .items_end()
            .gap(px(1.))
            .children(hs.into_iter().map(move |h| {
                div()
                    .flex_1()
                    .h(px(bar_height(h)))
                    .bg(ghost_color.opacity(0.22))
            }))
            .into_any_element(),
        None => div().into_any_element(),
    };
    let current_layer = match heights {
        Some(hs) => h_flex()
            .absolute()
            .top(px(0.))
            .left(px(0.))
            .w_full()
            .h_full()
            .items_end()
            .gap(px(1.))
            .children(hs.into_iter().map(move |h| {
                div()
                    .flex_1()
                    .h(px(bar_height(h)))
                    .bg(solid_color.opacity(0.75))
            }))
            .into_any_element(),
        None => div().into_any_element(),
    };

    v_flex()
        .w_full()
        .gap_1()
        .child(
            h_flex()
                .w_full()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_xs()
                        .font_medium()
                        .text_color(cx.theme().muted_foreground)
                        .child("直方图"),
                )
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("高光 {high_pct:.1}% · 死黑 {low_pct:.1}%")),
                ),
        )
        .child(
            div()
                .relative()
                .w_full()
                .h(px(CHART_H))
                .rounded(px(6.))
                .bg(cx.theme().muted.opacity(0.35))
                .overflow_hidden()
                .child(baseline_layer)
                .child(current_layer),
        )
}

/// 单根柱子的高度：空桶给 0（画 1px 会连成一条假的地平线），非空桶至少 1px 以保证可见。
fn bar_height(h: f32) -> f32 {
    if h <= 0.0 { 0.0 } else { h.max(1.0) }
}
