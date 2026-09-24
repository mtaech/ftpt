//! 无障碍（AccessKit）语义的**文案纯逻辑**（§13.4 的「网格无表格语义」）。
//!
//! GPUI 把「带 id + role」的元素变成 a11y 节点，读屏软件念的是 `aria_label`；
//! 本模块只负责把「文件名 + 可见状态」组合成**一条**可读、稳定、可单测的名称，
//! 视图里不拼字符串（网格 / 胶片条 / 右键菜单三处共用）。
//!
//! 为什么这条值得做：此前网格的选中框、评分星、旗标角标、物种名**全在像素里**，
//! a11y 树里只有一个没名字的容器与一堆 `GenericContainer` —— 读屏用户既听不到
//! 「这是哪张照片」也听不到「第几行第几列、有没有选中、是什么物种」。

use photo_domain::{Flag, Rating};

/// 网格容器名称：「照片网格：48 张照片，5 列，已选 1 张」
pub fn grid_container_label(total: usize, cols: usize, selected: usize) -> String {
    format!("照片网格：{total} 张照片，{cols} 列，已选 {selected} 张")
}

/// 网格 cell 名称：文件名 + 选中 + 评分 + 旗标 + 物种（缺项跳过，不补占位词）。
pub fn grid_cell_label(
    name: &str,
    selected: bool,
    rating: Rating,
    flag: Option<Flag>,
    taxon: Option<&str>,
) -> String {
    let mut parts = vec![name.to_string()];
    if selected {
        parts.push("已选中".to_string());
    }
    let stars = rating as usize;
    if stars > 0 {
        parts.push(format!("{stars} 星"));
    }
    match flag {
        Some(Flag::Pick) => parts.push("标识为 Pick".to_string()),
        Some(Flag::Reject) => parts.push("标为 Reject".to_string()),
        None => {}
    }
    if let Some(taxon) = taxon.filter(|t| !t.is_empty()) {
        parts.push(taxon.to_string());
    }
    parts.join("，")
}

/// 胶片条整体名称：「胶片条：48 张」
pub fn filmstrip_label(count: usize) -> String {
    format!("胶片条：{count} 张")
}

/// 胶片条单项名称：文件名（当前照片额外说明）。
pub fn filmstrip_item_label(name: &str, is_current: bool) -> String {
    if is_current {
        format!("{name}（当前照片）")
    } else {
        name.to_string()
    }
}

/// 右键菜单项名称：置灰项明说「不可用」——a11y 里没有 aria-disabled 通道，
/// 读屏时若不加这句，用户会以为点了会有反应。
pub fn menu_item_label(label: &str, disabled: bool) -> String {
    if disabled {
        format!("{label}（不可用）")
    } else {
        label.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grid_container_label_counts() {
        assert_eq!(
            grid_container_label(48, 5, 1),
            "照片网格：48 张照片，5 列，已选 1 张"
        );
        assert_eq!(grid_container_label(0, 2, 0), "照片网格：0 张照片，2 列，已选 0 张");
    }

    #[test]
    fn test_grid_cell_label_composes_only_present_states() {
        // 干净的一张：只有名字
        assert_eq!(
            grid_cell_label("三宝鸟.jpg", false, Rating::None, None, None),
            "三宝鸟.jpg"
        );
        // 状态齐全：选中 + 4 星 + Pick + 物种
        assert_eq!(
            grid_cell_label(
                "三宝鸟.jpg",
                true,
                Rating::Four,
                Some(Flag::Pick),
                Some("三宝鸟")
            ),
            "三宝鸟.jpg，已选中，4 星，标识为 Pick，三宝鸟"
        );
        // Reject 与空物种：空物种不补占位词
        assert_eq!(
            grid_cell_label("a.RW2", false, Rating::None, Some(Flag::Reject), Some("")),
            "a.RW2，标为 Reject"
        );
    }

    #[test]
    fn test_filmstrip_labels() {
        assert_eq!(filmstrip_label(48), "胶片条：48 张");
        assert_eq!(filmstrip_item_label("a.jpg", true), "a.jpg（当前照片）");
        assert_eq!(filmstrip_item_label("a.jpg", false), "a.jpg");
    }

    #[test]
    fn test_menu_item_label_marks_disabled() {
        assert_eq!(menu_item_label("复制文件路径", false), "复制文件路径");
        assert_eq!(menu_item_label("在预览中打开", true), "在预览中打开（不可用）");
    }
}
