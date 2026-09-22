//! 导出对话框的纯逻辑（Batch 1.1）：草稿参数、目标集与输出命名去重。
//!
//! 只回答「选哪些照片、输出叫什么名、参数是否合法」；真正写文件的是
//! `photo_engine::convert::export_with_preset`（由 `state::engine_ops::start_export`
//! 在后台线程调用）。不依赖 GPUI，可单元测试。

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// 导出草稿：对话框控件绑定的参数 + 目标目录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportDraft {
    /// 目标目录（空 = 未选，不能开始导出）
    pub dest_dir: String,
    /// 长边像素上限（None = 原尺寸，不放大）
    pub long_edge: Option<u32>,
    /// JPEG 质量（1-100，执行前钳制）
    pub quality: u8,
    /// 命名模板（占位符语义见 `photo_engine::template`）
    pub template: String,
}

impl Default for ExportDraft {
    /// 与 `photo_config::ExportPreset::default()` 同口径：原尺寸 / 质量 95 / `{name}`。
    fn default() -> Self {
        Self {
            dest_dir: String::new(),
            long_edge: None,
            quality: 95,
            template: "{name}".to_string(),
        }
    }
}

impl ExportDraft {
    /// 执行前钳制（与 `ExportPreset::clamped` 同口径）：质量 1-100、长边 0 → None、
    /// 空白模板回退 `{name}`（否则整批会退化成同名，全靠去重后缀）。
    pub fn clamped(mut self) -> Self {
        self.quality = self.quality.clamp(1, 100);
        if self.long_edge == Some(0) {
            self.long_edge = None;
        }
        if self.template.trim().is_empty() {
            self.template = "{name}".to_string();
        }
        self
    }

    /// 参数是否完整（目标目录非空）；不完整时「开始导出」应禁用。
    pub fn is_ready(&self) -> bool {
        !self.dest_dir.trim().is_empty()
    }

    /// 套用配置里的导出预设（预设按钮点击时）。**不动目标目录**——它是本次会话的选择。
    pub fn apply_preset(&mut self, preset: &photo_config::ExportPreset) {
        let clamped = preset.clone().clamped();
        self.long_edge = clamped.long_edge;
        self.quality = clamped.quality;
        self.template = clamped.template;
    }

    /// 质量档位标签（滑杆旁的数值 chip）。
    pub fn quality_label(&self) -> String {
        format!("{}%", self.quality.clamp(1, 100))
    }

    /// 长边标签（chip 与结果摘要共用）。
    pub fn long_edge_label(&self) -> String {
        match self.long_edge {
            None => "原尺寸".to_string(),
            Some(px) => format!("长边 {px}"),
        }
    }
}

/// 长边档位（None = 原尺寸，不放大）。与手册 §9.10「长边」规格同义，避免手填越界值。
pub const LONG_EDGE_OPTIONS: &[(Option<u32>, &str)] = &[
    (None, "原尺寸"),
    (Some(3840), "3840"),
    (Some(2560), "2560"),
    (Some(1920), "1920"),
    (Some(1280), "1280"),
];

/// JPEG 质量档位（手动滑杆之外的快捷值；滑杆本身 1-100 连续）。
pub const QUALITY_OPTIONS: &[u8] = &[95, 90, 85, 75];

/// 导出目标集：有选中 = 选中项 ∩ 当前筛选结果（顺序跟显示序，保证 `{seq}` 稳定）；
/// 无选中 = 当前筛选结果全部（与批量操作「未选 = 全量」口径一致）。
pub fn export_targets(selected: &[usize], display_order: &[usize]) -> Vec<usize> {
    if selected.is_empty() {
        return display_order.to_vec();
    }
    let set: HashSet<usize> = selected.iter().copied().collect();
    display_order
        .iter()
        .copied()
        .filter(|i| set.contains(i))
        .collect()
}

/// 输出路径去重：`dir/base.ext` 已存在（磁盘上或本批已排定）→ `base_1.ext`、`base_2.ext`…
///
/// `used` 必须跨整批共享：模板渲染可能给多张照片同一个基名
/// （如 `{species}` 同物种多张），全靠这里兜底。返回的路径也记入 `used`。
pub fn unique_output_path(
    dir: &Path,
    base: &str,
    ext: &str,
    used: &mut HashSet<String>,
) -> PathBuf {
    let mut candidate = dir.join(format!("{base}.{ext}"));
    let mut n = 1u32;
    while used.contains(&candidate.to_string_lossy().to_string()) || candidate.exists() {
        candidate = dir.join(format!("{base}_{n}.{ext}"));
        n += 1;
    }
    used.insert(candidate.to_string_lossy().to_string());
    candidate
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_draft_default_matches_config_preset() {
        let d = ExportDraft::default();
        let p = photo_config::ExportPreset::default();
        assert_eq!(d.long_edge, p.long_edge);
        assert_eq!(d.quality, p.quality);
        assert_eq!(d.template, p.template);
        // 目标目录未选 → 不能开始导出
        assert!(!d.is_ready());
    }

    #[test]
    fn test_draft_clamp_normalizes_out_of_range() {
        let d = ExportDraft {
            dest_dir: "  /tmp/out  ".to_string(),
            long_edge: Some(0),
            quality: 0,
            template: "   ".to_string(),
        };
        let c = d.clamped();
        assert_eq!(c.long_edge, None);
        assert_eq!(c.quality, 1);
        assert_eq!(c.template, "{name}");
        // 目标目录只判非空，不 trim 写回（路径原样交给引擎）
        assert!(c.is_ready());
    }

    #[test]
    fn test_apply_preset_keeps_dest_dir() {
        let mut d = ExportDraft {
            dest_dir: "/out".to_string(),
            quality: 60,
            ..Default::default()
        };
        d.apply_preset(&photo_config::ExportPreset {
            name: "网络分享".to_string(),
            long_edge: Some(2000),
            quality: 85,
            template: "{species}_{seq}".to_string(),
        });
        assert_eq!(d.dest_dir, "/out");
        assert_eq!(d.long_edge, Some(2000));
        assert_eq!(d.quality, 85);
        assert_eq!(d.template, "{species}_{seq}");
    }

    #[test]
    fn test_labels() {
        let d = ExportDraft::default();
        assert_eq!(d.quality_label(), "95%");
        assert_eq!(d.long_edge_label(), "原尺寸");
        let d2 = ExportDraft {
            long_edge: Some(2560),
            ..Default::default()
        };
        assert_eq!(d2.long_edge_label(), "长边 2560");
    }

    #[test]
    fn test_export_targets_no_selection_means_all() {
        assert_eq!(export_targets(&[], &[2, 0, 5]), vec![2, 0, 5]);
    }

    #[test]
    fn test_export_targets_selection_follows_display_order() {
        // 点击顺序是 5,2 → 输出仍按显示序 2,5（{seq} 稳定，不随点击顺序漂移）
        assert_eq!(export_targets(&[5, 2], &[2, 0, 5]), vec![2, 5]);
        // 已选但不在当前筛选结果里 → 空集，不导出（不悄悄导出被筛掉的）
        assert_eq!(export_targets(&[9], &[2, 0, 5]), Vec::<usize>::new());
    }

    #[test]
    fn test_option_tables_are_sane() {
        assert_eq!(LONG_EDGE_OPTIONS[0].0, None);
        assert!(LONG_EDGE_OPTIONS[1..]
            .iter()
            .all(|(v, _)| v.is_some_and(|n| n > 0)));
        assert!(QUALITY_OPTIONS.iter().all(|q| (1..=100).contains(q)));
        assert!(QUALITY_OPTIONS.windows(2).all(|w| w[0] > w[1]));
    }

    #[test]
    fn test_unique_output_path_appends_suffix_on_batch_collision() {
        let dir = std::env::temp_dir().join(format!("pt_export_unique_{}", std::process::id()));
        let mut used = HashSet::new();
        // 目录不存在 → exists() 恒 false，只走 used 去重
        let a = unique_output_path(&dir, "鸟", "jpg", &mut used);
        let b = unique_output_path(&dir, "鸟", "jpg", &mut used);
        let c = unique_output_path(&dir, "鸟", "jpg", &mut used);
        assert!(a.ends_with("鸟.jpg"));
        assert!(b.ends_with("鸟_1.jpg"));
        assert!(c.ends_with("鸟_2.jpg"));
    }

    #[test]
    fn test_unique_output_path_skips_existing_file_on_disk() {
        let dir = std::env::temp_dir().join(format!("pt_export_exists_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("shot.jpg"), b"x").unwrap();
        let mut used = HashSet::new();
        let out = unique_output_path(&dir, "shot", "jpg", &mut used);
        assert!(out.ends_with("shot_1.jpg"), "已存在同名文件应让位，实际 {out:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
