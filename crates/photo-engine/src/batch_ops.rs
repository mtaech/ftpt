use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use photo_domain::{BatchOpType, Capture};

use crate::ops;

/// 按 stem 扩展操作集（画面粒度，ADR 0006）：
/// 把每个索引对应 capture 的同名兄弟文件（格式 ∈ `sync_formats`）并入操作集。
///
/// `sync_formats` 为空 → 不扩展（只操作给定索引）。
/// 返回扩展后的索引列表：去重，顺序按原索引优先、兄弟随后。
pub fn expand_with_siblings(
    captures: &[Capture],
    indices: &[usize],
    sync_formats: &HashSet<String>,
) -> Vec<usize> {
    if sync_formats.is_empty() {
        return indices.to_vec();
    }
    // (父目录, stem) → 命中格式的 capture 索引：同 stem 不同目录不是同画面
    // （递归扫描下 2024-01-01/IMG_1 与 2024-02-01/IMG_1），必须按目录隔离，
    // 否则「同步同名文件」会把别的目录的同名兄弟一起移动/删除
    let parent_of = |c: &Capture| -> PathBuf {
        c.source_files
            .get(c.primary_index)
            .and_then(|f| f.path.parent())
            .map(|p| p.to_path_buf())
            .unwrap_or_default()
    };
    let mut by_stem: HashMap<(PathBuf, &str), Vec<usize>> = HashMap::new();
    for (i, c) in captures.iter().enumerate() {
        if capture_format_matches(c, sync_formats) {
            by_stem
                .entry((parent_of(c), c.base_name.as_str()))
                .or_default()
                .push(i);
        }
    }
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for &idx in indices {
        // 触发点先入（自身格式可能不在 sync_formats，不能只走兄弟分支）
        if !seen.insert(idx) {
            continue;
        }
        out.push(idx);
        let Some(c) = captures.get(idx) else { continue };
        if let Some(sibs) = by_stem.get(&(parent_of(c), c.base_name.as_str())) {
            for &s in sibs {
                if seen.insert(s) {
                    out.push(s);
                }
            }
        }
    }
    out
}

/// 按主文件路径白名单筛选 capture 索引（批量操作 = 前端筛选结果驱动）。
///
/// 空白名单返回空索引——**不落回全量**，避免「筛选结果为空」时误操作整个目录。
pub fn select_by_paths(captures: &[Capture], paths: &HashSet<String>) -> Vec<usize> {
    if paths.is_empty() {
        return Vec::new();
    }
    captures
        .iter()
        .enumerate()
        .filter(|(_, c)| {
            c.source_files
                .get(c.primary_index)
                .is_some_and(|f| paths.contains(f.path.to_string_lossy().as_ref()))
        })
        .map(|(i, _)| i)
        .collect()
}

/// 执行批量操作，返回每个文件的结果（Ok = 成功消息，Err = 失败消息）。
///
/// 结构化返回避免调用方按「消息里是否含『失败』」这种字符串判定分类——文件名
/// 本身可能含该子串，导致成功项被误判为失败并漏记撤销日志。
///
/// `target_dir` — 复制/移动的目标目录（删除操作时可为 None）
/// `on_progress` — 处理完每个文件后回调 (completed, total)
pub fn execute(
    source_captures: &[Capture],
    indices: &[usize],
    op_type: BatchOpType,
    target_dir: Option<&Path>,
    on_progress: impl Fn(u32, u32),
) -> Vec<Result<String, String>> {
    let mut results = Vec::new();
    let total = indices.len() as u32;

    let target_dir = if op_type.needs_target_dir() {
        match target_dir {
            Some(d) if !d.as_os_str().is_empty() => Some(d),
            _ => {
                return indices
                    .iter()
                    .map(|_| Err("错误：目标目录未指定".to_string()))
                    .collect();
            }
        }
    } else {
        None
    };

    for (i, &idx) in indices.iter().enumerate() {
        let Some(capture) = source_captures.get(idx) else { continue };
        let name = &capture.base_name;
        let verb = op_type.action_label();

        let result = match op_type {
            BatchOpType::Copy => ops::copy_capture(capture, target_dir.unwrap(), false)
                .map(|_| format!("{verb}: {}", name)),
            BatchOpType::Delete => ops::delete_capture(capture)
                .map(|_| format!("{verb}: {}", name)),
            BatchOpType::Move => ops::move_capture(capture, target_dir.unwrap())
                .map(|_| format!("{verb}: {}", name)),
        };

        match result {
            Ok(msg) => results.push(Ok(msg)),
            Err(e) => results.push(Err(format!("{verb}失败: {} — {e}", name))),
        }

        on_progress(i as u32 + 1, total);
    }

    results
}

/// capture 的任一源文件格式是否命中 `formats`（大小写不敏感，如 "NEF"/"nef"）
fn capture_format_matches(capture: &Capture, formats: &HashSet<String>) -> bool {
    if formats.is_empty() {
        return false;
    }
    capture.source_files.iter().any(|f| {
        let ext = f.format.to_string().to_uppercase();
        formats.iter().any(|fmt| fmt.to_uppercase() == ext)
    })
}

/// 目录中实际出现的格式集合（去重，UI 同步格式多选栏用）
pub fn available_formats(captures: &[Capture]) -> Vec<String> {
    let mut set: HashSet<String> = HashSet::new();
    for c in captures {
        for f in &c.source_files {
            set.insert(f.format.to_string().to_uppercase());
        }
    }
    let mut v: Vec<String> = set.into_iter().collect();
    v.sort();
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner;
    use photo_domain::ImageFormat;
    use tempfile::TempDir;

    fn create_files(dir: &TempDir, names: &[(&str, &str)]) {
        for (stem, ext) in names {
            std::fs::write(dir.path().join(format!("{stem}.{ext}")), b"test").unwrap();
        }
    }

    fn scan(dir: &TempDir) -> Vec<Capture> {
        scanner::scan_directory(dir.path(), &Default::default(), None).unwrap()
    }

    #[test]
    fn test_expand_with_siblings_same_stem() {
        let dir = TempDir::new().unwrap();
        create_files(&dir, &[("IMG_001", "JPG"), ("IMG_001", "NEF"), ("IMG_002", "JPG")]);
        let caps = scan(&dir);

        // 操作 IMG_001.JPG（按 base_name 定位，不依赖扫描排序），同步格式 {NEF} → 应并入 NEF
        let idx_jpg = caps
            .iter()
            .position(|c| c.base_name == "IMG_001" && c.source_files[0].format == ImageFormat::Jpeg)
            .expect("IMG_001.JPG");
        let sync: HashSet<String> = ["NEF".into()].into();
        let expanded = expand_with_siblings(&caps, &[idx_jpg], &sync);
        assert_eq!(expanded.len(), 2, "JPG + NEF 兄弟应并入");

        // IMG_002.JPG 无 NEF 兄弟，不扩展
        let idx2 = caps.iter().position(|c| c.base_name == "IMG_002").unwrap();
        let expanded2 = expand_with_siblings(&caps, &[idx2], &sync);
        assert_eq!(expanded2, vec![idx2], "无兄弟时保持原集");
    }

    #[test]
    fn test_expand_with_siblings_same_stem_different_dirs_not_merged() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("a")).unwrap();
        std::fs::create_dir_all(dir.path().join("b")).unwrap();
        std::fs::write(dir.path().join("a").join("IMG_1.JPG"), b"x").unwrap();
        std::fs::write(dir.path().join("b").join("IMG_1.NEF"), b"x").unwrap();
        let caps =
            scanner::scan_directory_recursive(dir.path(), &Default::default(), None).unwrap();

        let idx_jpg = caps
            .iter()
            .position(|c| {
                c.base_name == "IMG_1" && c.source_files[0].format == ImageFormat::Jpeg
            })
            .expect("a/IMG_1.JPG");
        let sync: HashSet<String> = ["NEF".into()].into();
        let expanded = expand_with_siblings(&caps, &[idx_jpg], &sync);
        assert_eq!(expanded, vec![idx_jpg], "跨目录同 stem 不应并入兄弟");
    }

    #[test]
    fn test_expand_with_siblings_empty_formats_no_expand() {
        let dir = TempDir::new().unwrap();
        create_files(&dir, &[("A", "JPG"), ("A", "NEF")]);
        let caps = scan(&dir);

        let empty: HashSet<String> = HashSet::new();
        let expanded = expand_with_siblings(&caps, &[0], &empty);
        assert_eq!(expanded, vec![0], "同步格式为空 = 不扩展");
    }

    #[test]
    fn test_expand_with_siblings_format_filter() {
        let dir = TempDir::new().unwrap();
        create_files(&dir, &[("A", "JPG"), ("A", "NEF"), ("A", "PNG")]);
        let caps = scan(&dir);

        // 只勾 NEF：PNG 不应被并入
        let idx_jpg = caps
            .iter()
            .position(|c| c.base_name == "A" && c.source_files[0].format == ImageFormat::Jpeg)
            .expect("A.JPG");
        let sync: HashSet<String> = ["NEF".into()].into();
        let expanded = expand_with_siblings(&caps, &[idx_jpg], &sync);
        let stems: Vec<String> = expanded
            .iter()
            .filter_map(|&i| caps.get(i))
            .map(|c| c.source_files[0].format.to_string())
            .collect();
        assert_eq!(stems.len(), 2, "JPG + NEF，不含 PNG");
        assert!(!stems.iter().any(|f| f == "PNG"), "PNG 不应被并入");
    }

    #[test]
    fn test_available_formats_unique_sorted() {
        let dir = TempDir::new().unwrap();
        create_files(&dir, &[("A", "JPG"), ("B", "NEF"), ("C", "PNG")]);
        let caps = scan(&dir);
        let fmts = available_formats(&caps);
        assert!(fmts.contains(&"JPEG".to_string()), "JPG → Display 为 JPEG");
        assert!(fmts.contains(&"NEF".to_string()));
        assert!(fmts.contains(&"PNG".to_string()));
    }

    #[test]
    fn test_select_by_paths_whitelist_and_empty() {
        let dir = TempDir::new().unwrap();
        create_files(&dir, &[("a", "JPG"), ("b", "JPG"), ("c", "JPG")]);
        let caps = scan(&dir);

        // 空白名单 = 空操作集（绝不落回全量）
        let empty: HashSet<String> = HashSet::new();
        assert!(select_by_paths(&caps, &empty).is_empty());

        // 只选 b：其余文件不进入操作集
        let mut allow = HashSet::new();
        let b_path = caps
            .iter()
            .find(|c| c.base_name == "b")
            .map(|c| c.source_files[c.primary_index].path.to_string_lossy().to_string())
            .unwrap();
        allow.insert(b_path);
        let idx = select_by_paths(&caps, &allow);
        assert_eq!(idx.len(), 1);
        assert_eq!(caps[idx[0]].base_name, "b");
    }

    #[test]
    fn test_execute_delete_requires_no_target() {
        let dir = TempDir::new().unwrap();
        create_files(&dir, &[("del_me", "RW2")]);
        let source = scan(&dir);

        let results = execute(&source, &[0], BatchOpType::Delete, None, |_, _| {});
        assert!(results.iter().any(|r| r.as_ref().is_ok_and(|s| s.contains("删除"))));
        assert!(results.iter().all(|r| r.is_ok()));
    }

    #[test]
    fn test_execute_copy_requires_target() {
        let dir = TempDir::new().unwrap();
        create_files(&dir, &[("keep", "JPG")]);
        let source = scan(&dir);

        let results = execute(&source, &[0], BatchOpType::Copy, None, |_, _| {});
        assert!(results
            .iter()
            .any(|r| r.as_ref().is_err_and(|e| e.contains("目标目录未指定"))));
    }
}
