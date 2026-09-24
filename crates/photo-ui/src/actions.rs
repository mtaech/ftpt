//! 动作定义（对应 §5.6 键位总表）。

use gpui_kit::*;

actions!(
    ftpt,
    [
        // 导航与视图
        Prev,
        Next,
        Home,
        End,
        PrevMember,
        NextMember,
        ToggleView,
        Slideshow,
        TogglePlay,
        Stats,
        Map,
        ZoomIn,
        ZoomOut,
        Escape,
        // 标记 (0-5, 6-9/Ctrl+6, P/X/U, K, Delete)
        Rate0,
        Rate1,
        Rate2,
        Rate3,
        Rate4,
        Rate5,
        SetColorRed,
        SetColorYellow,
        SetColorGreen,
        SetColorBlue,
        SetColorPurple,
        SetFlagPick,
        SetFlagReject,
        ClearFlag,
        BurstCull,
        Delete,
        // 识别与叠加 (B, Ctrl+B, Ctrl+Shift+B, V, F, O)
        RecognizeSelected,
        RecognizeAllUnrecognized,
        ReRecognizeAll,
        ToggleBbox,
        ToggleFocus,
        ToggleClipping,
        ToggleRegionSelect,
        // 调整参数的复制 / 粘贴（Ctrl+Shift+C / Ctrl+Shift+V；不用系统剪贴板）
        CopyAdjustments,
        PasteAdjustments,
        // 调整微调（[ / ] = 0.3 EV，Shift+[ / Shift+] = 0.9 EV）
        AdjustExposureUp,
        AdjustExposureDown,
        AdjustExposureUpCoarse,
        AdjustExposureDownCoarse,
        // 选择与面板与系统
        SelectAll,
        DeselectAll,
        Undo,
        // 剪贴板（Ctrl+C：复制当前照片的全尺寸 RGBA 到系统剪贴板）
        CopyImage,
        ToggleLeftPanel,
        ToggleRightPanel,
        Rescan,
        OpenDirectory,
        OpenSettings,
        // 导出（Ctrl+E：打开导出弹窗，导出当前筛选结果 / 已选照片）
        Export,
        ToggleThemeMode,
    ]
);
