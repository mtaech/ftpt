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
    /// 识别命中/选中的照片（作用域 = AppState::mark_indices()：多选时整批，与键盘 R 同口径）
    Recognize,
    MoveToTrash,
}

/// 一个菜单项
#[derive(Debug, Clone, PartialEq)]
pub struct MenuItem {
    pub label: String,
    pub action: ContextMenuAction,
    pub disabled: bool,
}

fn item(label: impl Into<String>, action: ContextMenuAction, disabled: bool) -> MenuItem {
    MenuItem {
        label: label.into(),
        action,
        disabled,
    }
}

/// 照片右键菜单项。10 项是有意的：菜单最大高 240px、每项 28px——超过 8 项就会
/// 真的出现滚动条（§13.4 要求「有最大高度、能滚」），否则滚动这条永远测不到。
///
/// - `in_preview`：已经在预览里 → 「在预览中打开」置灰（仍占位，菜单项位置稳定）。
/// - `affected`：**这一份菜单上的批量动作会作用于几张照片**——命中项已在选中集里时
///   = 选中集张数，否则 1（见 AppState::context_menu_scope_count）。
///
/// 多选时文案必须自己说出作用域（「评 5 星（3 张）」「识别选中的 3 张」）。用户报过
/// 「批量选中后右键菜单还只对一张生效」：那次根因是动作分发把多选塌成了单选；但如果
/// 只修行为不改文案，「识别这一张」在 3 张被选中时就成了假话。只作用于**命中那一张**的
/// 动作（预览 / 复制图片 / 打开所在文件夹）文案恒定——剪贴板一次也只装得下一张图。
pub fn photo_menu_items(in_preview: bool, affected: usize) -> Vec<MenuItem> {
    let affected = affected.max(1);
    let batch = affected > 1;
    let scope = |base: &str| {
        if batch {
            format!("{base}（{affected} 张）")
        } else {
            base.to_string()
        }
    };
    vec![
        item("在预览中打开", ContextMenuAction::OpenPreview, in_preview),
        item("复制图片到剪贴板", ContextMenuAction::CopyImage, false),
        item(
            if batch {
                format!("复制 {affected} 个文件路径")
            } else {
                "复制文件路径".to_string()
            },
            ContextMenuAction::CopyPath,
            false,
        ),
        item("打开所在文件夹", ContextMenuAction::OpenFolder, false),
        item(scope("标识为 Pick"), ContextMenuAction::SetPick, false),
        item(scope("标为 Reject"), ContextMenuAction::SetReject, false),
        item(scope("清除旗标"), ContextMenuAction::ClearFlag, false),
        item(scope("评 5 星"), ContextMenuAction::RateFive, false),
        item(
            if batch {
                format!("识别选中的 {affected} 张")
            } else {
                "识别这一张".to_string()
            },
            ContextMenuAction::Recognize,
            false,
        ),
        item(scope("移至回收站…"), ContextMenuAction::MoveToTrash, false),
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

/// 菜单单项高度（视图与「内容高」换算共用同一常量）
pub const ITEM_H: f32 = 28.0;

/// 菜单内容高 = 项数 × 单项高 + 上下内边距。
///
/// 内边距给了 16px（实际 py_1 只有 7.5px 左右）：留几像素余量，
/// 免得小数取整后内容比卡片高 1-2px——那种"看起来全在，却能滚一点"的缝隙很像 bug。
pub fn content_height(item_count: usize) -> f32 {
    item_count as f32 * ITEM_H + 16.0
}

/// 菜单实际高度 = 内容高，但不超过「窗口高 - 边距」——**放得下就不滚动**。
///
/// 用户报过「右键菜单太矮」：原来是固定 240px 上限，10 项（288px）被砍掉两项还得滚。
/// 现在按内容给高，只有真的放不下（项很多 / 窗口很矮）才限高滚动。
pub fn menu_height(item_count: usize, viewport_height: f32) -> f32 {
    let cap = (viewport_height - 8.0).max(120.0);
    content_height(item_count).min(cap)
}

/// 该菜单是否需要滚动（内容高超过实际高度）
pub fn needs_scroll(item_count: usize, viewport_height: f32) -> bool {
    content_height(item_count) > menu_height(item_count, viewport_height) + 0.5
}

#[cfg(test)]
mod tests {
    use super::{content_height, menu_height, needs_scroll, ITEM_H};

    /// 高度口径：10 项一次看全；项多或窗口矮才限高滚动
    #[test]
    fn test_menu_height_fits_content_until_viewport_limits() {
        assert_eq!(content_height(0), 16.0);
        assert_eq!(content_height(10), 10.0 * ITEM_H + 16.0);
        // 10 项（296px）在 760px 窗口里放得下 → 不滚动，高度就是内容高
        assert_eq!(menu_height(10, 760.0), 296.0);
        assert!(!needs_scroll(10, 760.0));
        // 30 项（856px）超过 760-8 → 限高并需要滚动
        assert_eq!(menu_height(30, 760.0), 752.0);
        assert!(needs_scroll(30, 760.0));
        // 窗口极矮也要留一个可用的最小值（120px），不能算成 0
        assert_eq!(menu_height(10, 60.0), 120.0);
        assert!(needs_scroll(10, 60.0));
    }

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
        let items = photo_menu_items(false, 1);
        assert_eq!(items.len(), 10);
        assert!(items.iter().all(|i| !i.disabled), "不在预览里时每项都可用");

        // 已经在预览里 → 「在预览中打开」置灰，但仍然占位（菜单项位置稳定）
        let in_preview = photo_menu_items(true, 1);
        assert!(in_preview[0].disabled);
        assert!(!in_preview[1].disabled);
    }

    /// 多选时批量动作的文案必须说出作用域，单选时保持原来的字面（用户报过
    /// 「批量选中后右键菜单只对一张生效」——行为修了，文案也得跟着说清楚）
    #[test]
    fn test_photo_menu_items_labels_scale_with_selection_scope() {
        let single = photo_menu_items(false, 1);
        assert_eq!(single[2].label, "复制文件路径");
        assert_eq!(single[4].label, "标识为 Pick");
        assert_eq!(single[8].label, "识别这一张");
        assert_eq!(single[9].label, "移至回收站…");

        let batch = photo_menu_items(false, 3);
        assert_eq!(batch.len(), 10);
        assert_eq!(batch[2].label, "复制 3 个文件路径");
        assert_eq!(batch[4].label, "标识为 Pick（3 张）");
        assert_eq!(batch[5].label, "标为 Reject（3 张）");
        assert_eq!(batch[6].label, "清除旗标（3 张）");
        assert_eq!(batch[7].label, "评 5 星（3 张）");
        assert_eq!(batch[8].label, "识别选中的 3 张");
        assert_eq!(batch[9].label, "移至回收站…（3 张）");

        // 只作用于命中那一张的动作：文案恒定（剪贴板一次只装得下一张图）
        assert_eq!(batch[0].label, "在预览中打开");
        assert_eq!(batch[1].label, "复制图片到剪贴板");
        assert_eq!(batch[3].label, "打开所在文件夹");

        // affected = 0 是调用方的口径错误：兜底成单选文案，绝不能出现「0 张」
        let zero = photo_menu_items(false, 0);
        assert_eq!(zero[8].label, "识别这一张");
        assert!(!zero[4].label.contains('0'));
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
