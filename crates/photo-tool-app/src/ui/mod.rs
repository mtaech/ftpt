pub mod activity_rail;
pub mod batch_ops;
pub mod context_menu;
pub mod controls;
pub mod filter_bar;
pub mod filmstrip;
pub mod grid;
pub mod grid_cell;
pub mod info_panel;
pub mod layout;
pub mod preview;
pub mod right_rail;
pub mod sidebar;
pub mod status_bar;
pub mod theme;
pub mod toolbar;

/// 文件大小格式化为可读字符串（B/KB/MB），网格与信息面板共用
pub fn format_file_size(size: Option<u64>) -> String {
    match size {
        Some(bytes) if bytes >= 1_048_576 => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
        Some(bytes) if bytes >= 1024 => format!("{:.1} KB", bytes as f64 / 1024.0),
        Some(bytes) => format!("{bytes} B"),
        None => "\u{2014}".into(),
    }
}
