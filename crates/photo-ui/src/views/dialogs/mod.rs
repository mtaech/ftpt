//! 弹窗家族模块（对应 §9.10）。

pub mod batch_confirm;
pub mod burst_confirm;
pub mod duplicates_dialog;
pub mod export_dialog;
pub mod import_dialog;
pub mod settings;

pub use batch_confirm::render_batch_confirm_dialog;
pub use burst_confirm::render_burst_confirm_dialog;
pub use duplicates_dialog::render_duplicates_dialog;
pub use export_dialog::render_export_dialog;
pub use import_dialog::render_import_dialog;
pub use settings::{SettingsTab, render_settings_dialog};
