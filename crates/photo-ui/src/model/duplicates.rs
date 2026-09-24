//! 近重复检测的纯逻辑（不依赖 GPUI，可单元测试）。
//!
//! 引擎侧 `photo_engine::phash` 已提供 dHash / 汉明距离 / 贪心聚类（含 9 个单测），
//! UI 侧只需要三件决策——都收在这里：
//!
//! 1. **检测作用域**（哪些照片参与）：当前目录全部照片，但**剔除同 stem 的多格式组**。
//!    本仓库的堆叠语义把同 stem 的 JPEG/RAW 当「同一画面多格式」（`model/stacks.rs`
//!    的 ByFileName），它们的 dHash 天然几乎相同，放进检测只会每张都凑出一组假重复。
//! 2. **分组视图**：组内**首张 = 保留锚点**（keeper），其余为待处理项。keeper 由路径排序
//!    决定，结果确定可复现（不随文件系统枚举顺序漂移）。
//! 3. **落库键转换**：完整路径 ⇄ 相对路径（`folder_db::duplicates` 表口径）。
//!
//! 阈值档位是实测选的：`dup_calibrate` 在 654 张真实鸟照上量得真重复（同图另存/重命名）
//! 汉明距离 ≤3、无关照片 ≥16，10 是手册默认值、也落在两峰之间的平台区。

use std::collections::HashMap;
use std::path::Path;

use photo_domain::CaptureMeta;

/// 可调阈值档位（汉明距离，dHash 64bit；越小越严）。档位与手册 §9.10 的
/// 「6/8/10/12/16，默认 10」一致。
pub const THRESHOLD_OPTIONS: [u32; 5] = [6, 8, 10, 12, 16];

/// 默认阈值：与引擎 `phash::DEFAULT_HASH_THRESHOLD` 同一常量，别写死两份
pub const DEFAULT_THRESHOLD: u32 = photo_engine::phash::DEFAULT_HASH_THRESHOLD;

/// 一组近重复照片：`keeper` 是保留锚点，`extras` 是「多余的那几张」。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateGroup {
    pub keeper: String,
    pub extras: Vec<String>,
}

impl DuplicateGroup {
    /// 组内总张数
    pub fn len(&self) -> usize {
        self.extras.len() + 1
    }
}

/// 检测作用域：`items` 里的照片完整路径，按路径排序。
///
/// **剔除同 stem 多格式组**（见模块文档第 1 条）。`base_name` 即 file_stem，大小写不敏感
/// 比较（`.JPG`/`.RW2` 同 stem 也算一组）。
pub fn duplicate_scope(items: &[CaptureMeta]) -> Vec<String> {
    let mut stem_count: HashMap<String, usize> = HashMap::new();
    for m in items {
        *stem_count.entry(m.base_name.to_lowercase()).or_insert(0) += 1;
    }
    let mut paths: Vec<String> = items
        .iter()
        .filter(|m| stem_count.get(&m.base_name.to_lowercase()).copied() == Some(1))
        .map(|m| m.primary_path.clone())
        .collect();
    paths.sort();
    paths.dedup();
    paths
}

/// 引擎原始分组（`Vec<Vec<完整路径>>`，组内首张为锚点）→ 视图分组。
/// 少于 2 张的组剔除（引擎 `group_duplicates` 已过滤，这里是双保险）。
pub fn group_views(groups: &[Vec<String>]) -> Vec<DuplicateGroup> {
    groups
        .iter()
        .filter(|g| g.len() >= 2)
        .map(|g| DuplicateGroup {
            keeper: g[0].clone(),
            extras: g[1..].to_vec(),
        })
        .collect()
}

/// 汇总：(组数, 多余张数)——多余 = 每组除 keeper 外的全部
pub fn summarize(groups: &[Vec<String>]) -> (usize, usize) {
    let views = group_views(groups);
    (views.len(), views.iter().map(|g| g.extras.len()).sum())
}

/// 完整路径分组 → 相对路径分组（落库键）。不在 `dir` 下的路径整组丢弃
/// （宁可少落一组，也不写出算不回来的键）。
pub fn to_rel_groups(dir: &Path, groups: &[Vec<String>]) -> Vec<Vec<String>> {
    groups
        .iter()
        .filter_map(|g| {
            g.iter()
                .map(|p| crate::model::adjust::rel_path_of(dir, Path::new(p)))
                .collect::<Option<Vec<String>>>()
        })
        .collect()
}

/// 相对路径分组 → 完整路径分组（读回落库结果）。
pub fn to_full_groups(dir: &Path, groups: &[Vec<String>]) -> Vec<Vec<String>> {
    groups
        .iter()
        .map(|g| {
            g.iter()
                .map(|rel| dir.join(rel).to_string_lossy().to_string())
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use photo_domain::{CaptureMeta, ColorLabel, Rating};

    fn meta(base_name: &str, primary_path: &str, format: &str) -> CaptureMeta {
        CaptureMeta {
            index: 0,
            base_name: base_name.to_string(),
            primary_path: primary_path.to_string(),
            primary_format: format.to_string(),
            file_size: Some(1024),
            date_taken: None,
            extensions: vec![],
            camera_make: None,
            camera_model: None,
            lens: None,
            exposure_time: None,
            f_number: None,
            iso: None,
            focal_length: None,
            image_width: None,
            image_height: None,
            gps_lat: None,
            gps_lon: None,
            focus_point: None,
            rating: Rating::None,
            color_label: ColorLabel::None,
            flag: None,
            keywords: vec![],
            taxon_name: None,
            taxon_confidence: None,
            taxon_gap: None,
            recognition_status: None,
            taxon_bbox: None,
            subjects: vec![],
            failure_stage: None,
            candidates: Vec::new(),
            has_adjustments: false,
        }
    }

    #[test]
    fn test_scope_drops_same_stem_multi_format() {
        // a.jpg + a.rw2 是同画面多格式（base_name 同为 a）→ 不参与检测；
        // b.jpg / c.jpg 参与，且按路径排序（keeper 稳定）
        let items = vec![
            meta("a", "/photos/a.rw2", "RW2"),
            meta("b", "/photos/b.jpg", "JPEG"),
            meta("a", "/photos/a.jpg", "JPEG"),
            meta("c", "/photos/c.jpg", "JPEG"),
        ];
        assert_eq!(
            duplicate_scope(&items),
            vec!["/photos/b.jpg".to_string(), "/photos/c.jpg".to_string()]
        );
    }

    #[test]
    fn test_scope_stem_compare_is_case_insensitive() {
        let items = vec![
            meta("BIRD", "/photos/BIRD.JPG", "JPEG"),
            meta("bird", "/photos/bird.rw2", "RW2"),
            meta("other", "/photos/other.jpg", "JPEG"),
        ];
        assert_eq!(duplicate_scope(&items), vec!["/photos/other.jpg".to_string()]);
    }

    #[test]
    fn test_group_views_keeper_is_first_and_singletons_dropped() {
        let groups = vec![
            vec!["/p/a.jpg".to_string(), "/p/b.jpg".to_string()],
            vec!["/p/only.jpg".to_string()],
        ];
        let views = group_views(&groups);
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].keeper, "/p/a.jpg");
        assert_eq!(views[0].extras, vec!["/p/b.jpg".to_string()]);
        assert_eq!(views[0].len(), 2);
        assert_eq!(summarize(&groups), (1, 1));
    }

    #[test]
    fn test_rel_and_full_group_conversion_roundtrip() {
        let dir = Path::new("/photos/2026");
        let groups = vec![vec![
            "/photos/2026/a.jpg".to_string(),
            "/photos/2026/sub/b.jpg".to_string(),
        ]];
        let rel = to_rel_groups(dir, &groups);
        assert_eq!(rel, vec![vec!["a.jpg".to_string(), "sub/b.jpg".to_string()]]);
        assert_eq!(to_full_groups(dir, &rel), groups);
    }

    #[test]
    fn test_rel_groups_drop_paths_outside_root() {
        // 不在当前目录下的路径 → 整组丢弃（算不回来的键不落库）
        let dir = Path::new("/photos/2026");
        let groups = vec![
            vec!["/photos/2026/a.jpg".to_string(), "/elsewhere/b.jpg".to_string()],
            vec!["/photos/2026/c.jpg".to_string(), "/photos/2026/d.jpg".to_string()],
        ];
        assert_eq!(
            to_rel_groups(dir, &groups),
            vec![vec!["c.jpg".to_string(), "d.jpg".to_string()]]
        );
    }
}
