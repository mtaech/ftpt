//! 状态模块导出

pub mod app_state;
pub mod engine_ops;
pub mod folder_picker;
pub mod import;

pub use app_state::{ActiveDialog, AppState, SettingsTab, ViewMode};
pub use engine_ops::{
    defer_entity_action, delete_paths, delete_selected_to_trash, open_stats_photo, select_stats_species,
    set_color_label, set_flag, set_rating, start_recognition, start_region_recognition, start_scan,
};
pub use import::{ImportOutcome, ImportState, ImportTab};
