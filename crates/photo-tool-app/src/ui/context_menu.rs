//! 网格与预览共用的图片右键菜单（gpui-component PopupMenu）。
//! 菜单项全部映射到 crate::action::Action，由 layout 根节点的 on_action 统一分发。

use gpui::{Context, Window};
use gpui_component::menu::PopupMenu;
use photo_domain::{CaptureMeta, ColorLabel, Flag, Rating};

use crate::action::{Action, ContextMenuAction};

fn cmd(action: Action) -> Box<ContextMenuAction> {
    Box::new(ContextMenuAction(action))
}

/// 构建图片右键菜单。
/// `meta` 为右键目标（用于勾选当前评分/标签/标记）；`in_preview` 决定首项文案与缩放项；
/// `selected_count` 用于多选时调整菜单文案。
pub fn capture_menu(
    menu: PopupMenu,
    meta: Option<&CaptureMeta>,
    in_preview: bool,
    selected_count: usize,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let recognize_label = if selected_count > 1 {
        format!("识别所选照片 ({}张) (b)", selected_count)
    } else {
        "识别此照片 (b)".to_string()
    };
    let menu = menu.menu(
        if in_preview { "返回网格" } else { "在预览中打开" },
        cmd(Action::ToggleGridPreview),
    )
    .menu(recognize_label.as_str(), cmd(Action::Recognize));

    let Some(meta) = meta else {
        return menu;
    };

    // submenu 闭包要求 'static，先取出 Copy 的当前值用于勾选
    let rating = meta.rating;
    let color_label = meta.color_label;
    let flag = meta.flag;

    let menu = menu
        .separator()
        .submenu("评分", window, cx, move |m, _, _| {
            m.menu_with_check("无评分", rating == Rating::None, cmd(Action::Rate0))
                .menu_with_check("1 星", rating == Rating::One, cmd(Action::Rate1))
                .menu_with_check("2 星", rating == Rating::Two, cmd(Action::Rate2))
                .menu_with_check("3 星", rating == Rating::Three, cmd(Action::Rate3))
                .menu_with_check("4 星", rating == Rating::Four, cmd(Action::Rate4))
                .menu_with_check("5 星", rating == Rating::Five, cmd(Action::Rate5))
        })
        .submenu("颜色标签", window, cx, move |m, _, _| {
            m.menu_with_check("无标签", color_label == ColorLabel::None, cmd(Action::LabelNone))
                .menu_with_check("红色", color_label == ColorLabel::Red, cmd(Action::LabelRed))
                .menu_with_check("黄色", color_label == ColorLabel::Yellow, cmd(Action::LabelYellow))
                .menu_with_check("绿色", color_label == ColorLabel::Green, cmd(Action::LabelGreen))
                .menu_with_check("蓝色", color_label == ColorLabel::Blue, cmd(Action::LabelBlue))
                .menu_with_check("紫色", color_label == ColorLabel::Purple, cmd(Action::LabelPurple))
        })
        .submenu("标记", window, cx, move |m, _, _| {
            m.menu_with_check("无标记", flag.is_none(), cmd(Action::FlagNone))
                .menu_with_check("留用", flag == Some(Flag::Pick), cmd(Action::FlagPick))
                .menu_with_check("排除", flag == Some(Flag::Reject), cmd(Action::FlagReject))
        });

    let menu = if in_preview {
        menu.separator()
            .menu("放大", cmd(Action::ZoomIn))
            .menu("缩小", cmd(Action::ZoomOut))
            .menu("适应窗口", cmd(Action::ZoomToFit))
    } else {
        menu
    };

    menu.separator()
        .menu("删除（移至回收站）", cmd(Action::Delete))
}

/// 构建侧栏文件夹卡片的右键菜单。
/// 菜单命令作用于 `RootView::folder_menu_dir`（卡片 context_menu 闭包先行设置）。
/// `is_fav` 决定首项文案：已收藏显示「取消收藏」，否则「加入收藏」。
pub fn folder_menu(menu: PopupMenu, is_fav: bool) -> PopupMenu {
    menu.menu(
        if is_fav { "取消收藏" } else { "加入收藏" },
        cmd(Action::ToggleContextDirFavorite),
    )
    .separator()
    .menu("从列表移除", cmd(Action::RemoveContextDir))
}
