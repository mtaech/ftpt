//! 纯算法规范（无 IO、无 UI 依赖、完全可单元测试）。
//!
//! 对应《GUI 设计手册》§4 与附录 B。

pub mod adjust;
pub mod best_frame;
pub mod duplicates;
pub mod burst;
pub mod ebird;
pub mod export;
pub mod filter;
pub mod preview_math;
pub mod recognizer;
pub mod region;
pub mod sort;
pub mod stacks;

#[cfg(test)]
mod tests;

pub use adjust::{
    EXPOSURE_MAX, EXPOSURE_MIN, EXPOSURE_STEP, TONE_MAX, TONE_MIN, TONE_STEP, AdjustField,
    format_exposure, format_tone, has_adjustments, quantize_exposure, quantize_tone, rel_path_of,
};
pub use best_frame::{non_best_paths, pick_best_frame};
pub use duplicates::{
    DEFAULT_THRESHOLD, DuplicateGroup, THRESHOLD_OPTIONS, duplicate_scope, group_views,
    summarize, to_full_groups, to_rel_groups,
};
pub use burst::{BurstEntry, BurstGroupMap, compute_burst_groups, parse_exif_date};
pub use ebird::ebird_candidates;
pub use export::{
    ExportDraft, LONG_EDGE_OPTIONS, QUALITY_OPTIONS, export_targets, unique_output_path,
};
pub use filter::{
    FilterCriteria, default_filter_criteria, filter_captures, has_active_filters, lens_options,
    parse_focal_length_mm, taxon_options,
};
pub use preview_math::{
    clamp_pan_axis, exceeds_master_res, fit_scale, pan_after_cursor_zoom, preview_center_offset,
    region_bbox_from_drag,
};
pub use recognizer::should_unload_recognizer;
pub use region::{region_hint_text, starts_region_drag};
pub use sort::{
    GRID_COL_OPTIONS, SORT_OPTIONS, apply_filter_and_sort, compare_captures,
    grid_columns_from_value, sort_by_from_value, sort_by_value,
};
pub use stacks::{
    STACK_TIME_GAP_MS, StackGroup, group_by_time, group_singles, group_stacks, pick_primary,
};
