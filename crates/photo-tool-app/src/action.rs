#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    // Navigation
    Next,
    Prev,
    First,
    Last,
    // View
    ToggleGridPreview,
    ZoomIn,
    ZoomOut,
    ZoomToFit,
    // Rating
    Rate0,
    Rate1,
    Rate2,
    Rate3,
    Rate4,
    Rate5,
    // Color Label
    LabelRed,
    LabelYellow,
    LabelGreen,
    LabelBlue,
    LabelPurple,
    LabelNone,
    // Flag
    FlagPick,
    FlagReject,
    FlagNone,
    // Selection
    SelectAll,
    DeselectAll,
    // File ops
    Delete,
    PermanentDelete,
    // Other
    Refresh,
    ToggleLeftPanel,
    ToggleRightPanel,
    ToggleSettings,
}

/// 右键菜单命令：gpui-component 的 PopupMenu 菜单项要求 Box<dyn gpui::Action>，
/// 这里把应用内 Action 包一层走 GPUI Action 分发（layout 根节点 on_action 接收）。
/// no_json：不从 keymap JSON 构建，无需 serde。
#[derive(Debug, Clone, PartialEq, Eq, gpui::Action)]
#[action(namespace = photo_tool, no_json)]
pub struct ContextMenuAction(pub Action);
