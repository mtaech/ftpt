//! 照片右键菜单的菜单项与键盘导航纯逻辑（§13.4）。
//!
//! GPUI 版此前**没有任何右键菜单**；而 gpui-component 的 PopupMenu 只处理点击、
//! 全模块没有一处键盘处理（grep 无 on_key_down / ArrowUp），直接用它只会把
//! 「右键菜单无键盘导航」这条缺口原样搬过来。所以菜单由应用自己画
//! （app.rs 的 render 与 views/context_menu.rs），这里只放可单测的纯逻辑：
//! 菜单项表、初始选中、上下移动（跳过置灰项、到边界不环绕）。

/// 菜单项要执行的动作（点击与回车走同一条分发：engine_ops::apply_context_menu_action）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextMenuAction {
    OpenPreview,
    CopyImage,
    CopyPath,
    OpenFolder,
    SetPick,
    SetReject,
    ClearFlag,
    RateFive,
    RecognizeThis,
    MoveToTrash,
}

/// 一个菜单项
#[derive(Debug, Clone, PartialEq)]
pub struct MenuItem {
    pub label: String,
    pub action: ContextMenuAction,
    pub disabled: bool,
}

fn item(label: &str, action: ContextMenuAction, disabled: bool) -> MenuItem {
    MenuItem {
        label: label.to_string(),
        action,
        disabled,
    }
}

/// 照片右键菜单项。10 项是有意的：菜单最大高 240px、每项 28px——超过 8 项就会
/// 真的出现滚动条（§13.4 要求「有最大高度、能滚」），否则滚动这条永远测不到。
/// `in_preview` 时「在预览中打开」置灰（已经在预览里了）。
pub fn photo_menu_items(in_preview: bool) -> Vec<MenuItem> {
    vec![
        item("在预览中打开", ContextMenuAction::OpenPreview, in_preview),
        item("复制图片到剪贴板", ContextMenuAction::CopyImage, false),
        item("复制文件路径", ContextMenuAction::CopyPath, false),
        item("打开所在文件夹", ContextMenuAction::OpenFolder, false),
        item("标识为 Pick", ContextMenuAction::SetPick, false),
        item("标为 Reject", ContextMenuAction::SetReject, false),
        item("清除旗标", ContextMenuAction::ClearFlag, false),
        item("评 5 星", ContextMenuAction::RateFive, false),
        item("识别这一张", ContextMenuAction::RecognizeThis, false),
        item("移至回收站…", ContextMenuAction::MoveToTrash, false),
    ]
}

/// 打开菜单时的初始选中项：第一个可用项；全置灰时回 0。
pub fn initial_selection(items: &[MenuItem]) -> usize {
    items.iter().position(|i| !i.disabled).unwrap_or(0)
}

/// 上下移动选中项：跳过置灰项，到边界**停住**（不环绕——环绕会让「按到底」突然跳回
/// 第一项，回车就成了误触；菜单里宁可停在最后一项）。
pub fn move_selection(items: &[MenuItem], current: usize, delta: i32) -> usize {
    if items.is_empty() {
        return 0;
    }
    let mut sel = current.min(items.len() - 1);
    let step = if delta >= 0 { 1i32 } else { -1i32 };
    let mut remaining = delta.unsigned_abs();
    while remaining > 0 {
        let mut next = sel as i32 + step;
        while next >= 0 && (next as usize) < items.len() && items[next as usize].disabled {
            next += step;
        }
        if next < 0 || next as usize >= items.len() {
            break;
        }
        sel = next as usize;
        remaining -= 1;
    }
    sel
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items_with_disabled() -> Vec<MenuItem> {
        vec![
            item("a", ContextMenuAction::OpenPreview, false),
            item("b", ContextMenuAction::CopyImage, true),
            item("c", ContextMenuAction::CopyPath, true),
            item("d", ContextMenuAction::OpenFolder, false),
        ]
    }

    #[test]
    fn test_photo_menu_items_has_scrollable_size_and_preview_gate() {
        // 10 项 × 28px > 240px 最大高 → 保证「最大高度滚动」这条有真实的滚动场景
        let items = photo_menu_items(false);
        assert_eq!(items.len(), 10);
        assert!(items.iter().all(|i| !i.disabled), "不在预览里时每项都可用");

        // 已经在预览里 → 「在预览中打开」置灰，但仍然占位（菜单项位置稳定）
        let in_preview = photo_menu_items(true);
        assert!(in_preview[0].disabled);
        assert!(!in_preview[1].disabled);
    }

    #[test]
    fn test_initial_selection_skips_disabled() {
        assert_eq!(initial_selection(&items_with_disabled()), 0);
        let mut items = items_with_disabled();
        items[0].disabled = true;
        assert_eq!(initial_selection(&items), 3, "前两项置灰 → 落到 d");
        assert_eq!(initial_selection(&[]), 0);
    }

    #[test]
    fn test_move_selection_skips_disabled_and_stops_at_edges() {
        let items = items_with_disabled();
        // 0 → 跳过 b/c → d
        assert_eq!(move_selection(&items, 0, 1), 3);
        // 到底了再按不环绕
        assert_eq!(move_selection(&items, 3, 1), 3);
        // 往回同样跳过置灰项
        assert_eq!(move_selection(&items, 3, -1), 0);
        // 到顶了再按不环绕
        assert_eq!(move_selection(&items, 0, -1), 0);
        // 连按两次 = 一次一步
        let all_enabled: Vec<MenuItem> = vec![
            item("a", ContextMenuAction::OpenPreview, false),
            item("b", ContextMenuAction::CopyImage, false),
            item("c", ContextMenuAction::CopyPath, false),
        ];
        assert_eq!(move_selection(&all_enabled, 0, 2), 2);
        assert_eq!(move_selection(&[], 0, 1), 0);
    }
}
