#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    // Navigation
    Next,
    Prev,
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
