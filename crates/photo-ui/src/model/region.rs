//! 手动框选识别（漏检补充）的纯逻辑：触发条件与界面提示文案。
//!
//! 两种进入方式：工具条「框选」toggle（`AppState::region_select`），或直接**按住 Shift +
//! 左键拖拽**（不改 toggle 状态）。这里只放可单测的判定与文案；事件接线在
//! `views/preview.rs`（鼠标修饰符在 `MouseDownEvent::modifiers.shift`）。

/// 左键按下时是否进入「画框」而不是「平移」。
///
/// 显式开了框选模式，或按住 Shift——后者让用户不必先去工具条找那个 toggle。
pub fn starts_region_drag(region_select: bool, shift_held: bool) -> bool {
    region_select || shift_held
}

/// Esc 在预览区对「框选」的处置（用户报过「Shift 框选不能取消」）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionEscape {
    /// 正在拖拽画框：取消这次拖拽（框还没提交，识别也不会触发）
    CancelDrag,
    /// 框已经画出来了：清掉它
    ClearBox,
    /// 没有可处置的框选状态
    None,
}

/// 按优先级决定 Esc 该做什么：**进行中的拖拽 > 已画出的框**。
///
/// 两种情况都清掉框：拖拽中先清起点，松手时 take() 拿不到起点 → 不会触发识别，
/// 这就是「取消」的实现口径。
pub fn region_escape_action(dragging: bool, has_box: bool) -> RegionEscape {
    if dragging {
        RegionEscape::CancelDrag
    } else if has_box {
        RegionEscape::ClearBox
    } else {
        RegionEscape::None
    }
}

/// 预览区左上角常驻文字提示（把 Shift 这条快捷键摆在界面上，而不是只藏在 tooltip 里）。
///
/// 四态：拖拽中 → 说明松手即识别、Esc 可取消（用户报过「不能取消」，所以必须写出来）；
/// 显式框选模式 → 说明当前拖拽就是画框；按住 Shift → 说明现在就能拖；都没按 → 常驻提示 Shift。
pub fn region_hint_text(region_select: bool, shift_held: bool, dragging: bool) -> &'static str {
    if dragging {
        "松开左键即框选识别 · 按 Esc 取消"
    } else if region_select {
        "框选识别已开启：左键拖拽画框，松开后识别"
    } else if shift_held {
        "已按住 Shift：左键拖拽即可框选识别"
    } else {
        "按住 Shift + 左键拖拽，框选识别补充主体"
    }
}
