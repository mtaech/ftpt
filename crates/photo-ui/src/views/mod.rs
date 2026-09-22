//! UI 视图组件模块。

pub mod activity_bar;
pub mod dialogs;
pub mod dock_panels;
pub mod filmstrip;
pub mod filter_bar;
pub mod grid;
pub mod header;
pub mod info_panel;
pub mod left_panel;
pub mod preview;
pub mod scroll_area;
pub mod slideshow;
pub mod stats;
pub mod status_bar;
pub mod title_bar;

pub use activity_bar::{render_left_activity_bar, render_right_activity_bar};
pub use dock_panels::{DockPanel, DockPanelKind, create_dock};
pub use filmstrip::render_filmstrip;
pub use filter_bar::render_filter_bar;
pub use grid::render_photo_grid;
pub use header::render_header;
pub use info_panel::{render_adjustments_tab, render_info_tab};
pub use left_panel::{render_batch_ops_tab, render_file_tree_tab};
pub use preview::render_photo_preview;
pub use scroll_area::{scroll_area_h, scroll_area_v};
pub use slideshow::render_slideshow;
pub use stats::render_stats_view;
pub use status_bar::render_status_bar;
pub use title_bar::render_title_bar;
