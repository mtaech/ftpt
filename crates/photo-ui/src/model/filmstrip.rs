//! 胶片条几何与「切图自动定位」的纯逻辑（对应 §9.6）。
//!
//! 抽出来的原因：这三处数字（项宽 / 间距 / 左右 padding）以前只写在 `views/filmstrip.rs` 里，
//! 而「把当前那张滚进可视区」需要在状态层按同一套公式反推偏移量——两处各写一遍必然漂移
//! （导入弹窗那对公式就吃过这个亏）。

/// 缩略图项宽（px，手册 §9.6）
pub const THUMB_W: f32 = 96.0;
/// 缩略图间距（px，手册 §9.6）
pub const THUMB_GAP: f32 = 6.0;
/// 胶片条内容左右内边距（px，两边各 8）
pub const PAD: f32 = 8.0;

/// 胶片条内容总宽：`n × 96 + (n−1) × 6 + 2 × 8`。
///
/// **横向滚动的内容必须显式撑宽**：flex 项默认被拉伸到容器宽，只给 `h_full()` 的横排内容
/// 撑不出滚动范围（滚动条拿到 0 长度）。
pub fn content_width(count: usize) -> f32 {
    count as f32 * THUMB_W + count.saturating_sub(1) as f32 * THUMB_GAP + PAD * 2.0
}

/// 第 `pos` 项左缘在**内容坐标**里的位置。
pub fn item_left(pos: usize) -> f32 {
    PAD + pos as f32 * (THUMB_W + THUMB_GAP)
}

/// 让 `[item_left, item_left + item_w]` 进入可视窗口所需的横向偏移（内容坐标，非负）；
/// **已经在窗口里就返回 `None`**（不动，免得每次渲染都把用户的滚动位置拉回来）。
///
/// `current` / `viewport_w` / `max_offset` 都是内容坐标的正数口径（视口左缘 = `current`）。
/// 定位策略是「尽量居中」：箭头键翻到窗口外那张时，前后邻居都还看得见。
pub fn ensure_visible_offset(
    current: f32,
    viewport_w: f32,
    max_offset: f32,
    item_left: f32,
    item_w: f32,
) -> Option<f32> {
    if viewport_w <= 0.0 {
        return None;
    }
    let visible_left = current;
    let visible_right = current + viewport_w;
    if item_left >= visible_left && item_left + item_w <= visible_right {
        return None;
    }
    let centered = item_left - (viewport_w - item_w) / 2.0;
    let target = centered.clamp(0.0, max_offset.max(0.0));
    if (target - current).abs() < 0.5 {
        return None;
    }
    Some(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_content_width_matches_manual_layout() {
        assert_eq!(content_width(0), 16.0);
        assert_eq!(content_width(1), 112.0);
        assert_eq!(content_width(3), 3.0 * 96.0 + 2.0 * 6.0 + 16.0);
    }

    #[test]
    fn test_item_left_walks_by_thumb_plus_gap() {
        assert_eq!(item_left(0), 8.0);
        assert_eq!(item_left(1), 8.0 + 102.0);
    }

    #[test]
    fn test_ensure_visible_offset_is_none_when_item_already_visible() {
        // 第 0 项在 [0, 400) 里 → 不动
        assert_eq!(ensure_visible_offset(0.0, 400.0, 1000.0, item_left(0), THUMB_W), None);
        // 窗口往右滚了一屏，第 4 项（左缘 8+4×102=416）仍在 [204, 604) 里 → 不动
        assert_eq!(ensure_visible_offset(204.0, 400.0, 1000.0, item_left(4), THUMB_W), None);
    }

    #[test]
    fn test_ensure_visible_offset_centers_and_clamps() {
        // 第 20 项在窗口右边之外 → 居中（内容坐标 8+20×102=2048，视口 400）
        let target = ensure_visible_offset(0.0, 400.0, 5000.0, item_left(20), THUMB_W).unwrap();
        assert_eq!(target, 2048.0 - (400.0 - 96.0) / 2.0);
        // 靠近末尾时被 max_offset 夹住
        assert_eq!(
            ensure_visible_offset(0.0, 400.0, 1000.0, item_left(30), THUMB_W),
            Some(1000.0)
        );
        // 在窗口左边之外 → 往回滚（不能是负数）
        assert_eq!(
            ensure_visible_offset(3000.0, 400.0, 5000.0, item_left(0), THUMB_W),
            Some(0.0)
        );
    }
}
