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

/// 预览区左上角常驻文字提示（把 Shift 这条快捷键摆在界面上，而不是只藏在 tooltip 里）。
///
/// 三态：显式框选模式 → 说明当前拖拽就是画框、松开即识别；按住 Shift → 说明现在就能拖；
/// 都没按 → 常驻提示 Shift 快捷键。
pub fn region_hint_text(region_select: bool, shift_held: bool) -> &'static str {
    if region_select {
        "框选识别已开启：左键拖拽画框，松开后识别"
    } else if shift_held {
        "已按住 Shift：左键拖拽即可框选识别"
    } else {
        "按住 Shift + 左键拖拽，框选识别补充主体"
    }
}
