use std::collections::HashSet;
use std::path::{Path, PathBuf};

use photo_domain::{BatchOpType, Capture, ImageFormat};

use crate::ops;
use crate::scanner::{self, ScanError};

/// 批量操作错误
#[derive(Debug, thiserror::Error)]
pub enum BatchOpError {
    #[error("扫描错误: {0}")]
    Scan(#[from] ScanError),
    #[error("目标目录未指定")]
    MissingTargetDir,
    #[error("对比目录不存在: {0}")]
    CompareDirNotFound(PathBuf),
}

/// 在对比目录中查找匹配/非匹配的 capture 索引
///
/// `source_captures` — 当前已扫描的 capture 列表（操作目录）
/// `compare_dir` — 对比目录路径
/// `source_format` — 可选：仅统计含此格式的 capture（如 Some("RW2")）
/// `compare_format` — 可选：仅统计含此格式的对比 capture（如 Some("JPG")）
///
/// 返回 (matched_indices, unmatched_indices)：
/// - matched: 对比目录中存在同名 capture 的索引
/// - unmatched: 对比目录中不存在同名 capture 的索引
pub fn find_matching(
    source_captures: &[Capture],
    compare_dir: &Path,
    source_format: Option<&str>,
    compare_format: Option<&str>,
) -> Result<(Vec<usize>, Vec<usize>), BatchOpError> {
    if !compare_dir.exists() {
        return Err(BatchOpError::CompareDirNotFound(compare_dir.to_path_buf()));
    }

    // 扫描对比目录（batch 匹配只认文件名）
    let compare_captures =
        scanner::scan_directory(compare_dir, &Default::default(), None)?;

    // 构建对比目录的 base_name 集合（按需过滤格式）
    let compare_names: HashSet<String> = compare_captures
        .iter()
        .filter(|c| {
            compare_format.map_or(true, |fmt| capture_has_format(c, fmt))
        })
        .map(|c| c.base_name.clone())
        .collect();

    let mut matched = Vec::new();
    let mut unmatched = Vec::new();

    for (idx, capture) in source_captures.iter().enumerate() {
        // 源目录按格式过滤
        if let Some(fmt) = source_format {
            if !capture_has_format(capture, fmt) {
                continue;
            }
        }
        if compare_names.contains(&capture.base_name) {
            matched.push(idx);
        } else {
            unmatched.push(idx);
        }
    }

    Ok((matched, unmatched))
}

/// 执行批量操作，返回每个文件的可读结果
///
/// `target_dir` — 复制/移动的目标目录（删除操作时可为 None）
/// `on_progress` — 处理完每个文件后回调 (completed, total)
pub fn execute(
    source_captures: &[Capture],
    matched_indices: &[usize],
    op_type: BatchOpType,
    target_dir: Option<&Path>,
    on_progress: impl Fn(u32, u32),
) -> Vec<String> {
    let mut results = Vec::new();
    let total = matched_indices.len() as u32;

    let target_dir = if op_type.needs_target_dir() {
        match target_dir {
            Some(d) if !d.as_os_str().is_empty() => Some(d),
            _ => return vec!["错误：目标目录未指定".into()],
        }
    } else {
        None
    };

    for (i, &idx) in matched_indices.iter().enumerate() {
        let Some(capture) = source_captures.get(idx) else { continue };
        let name = &capture.base_name;
        let verb = op_type.action_label();

        let result = match op_type {
            BatchOpType::CopySame | BatchOpType::CopyNotSame => {
                ops::copy_capture(capture, target_dir.unwrap(), false)
                    .map(|_| format!("{verb}: {}", name))
            }
            BatchOpType::DeleteSame | BatchOpType::DeleteNotSame => {
                ops::delete_capture(capture)
                    .map(|_| format!("{verb}: {}", name))
            }
            BatchOpType::MoveSame | BatchOpType::MoveNotSame => {
                ops::move_capture(capture, target_dir.unwrap())
                    .map(|_| format!("{verb}: {}", name))
            }
        };

        match result {
            Ok(msg) => results.push(msg),
            Err(e) => results.push(format!("{verb}失败: {} — {e}", name)),
        }

        on_progress(i as u32 + 1, total);
    }

    results
}

/// 判断 capture 是否包含指定格式（大小写不敏感，接受 "jpg"/"jpeg"/"JPG" 等）
fn capture_has_format(capture: &Capture, format: &str) -> bool {
    let Some(target) = ImageFormat::from_extension(format) else { return false };
    capture.source_files.iter().any(|f| f.format == target)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_files(dir: &TempDir, names: &[(&str, &str)]) {
        for (stem, ext) in names {
            std::fs::write(dir.path().join(format!("{stem}.{ext}")), b"test").unwrap();
        }
    }

    #[test]
    fn test_find_matching_same_base_name() {
        let src_dir = TempDir::new().unwrap();
        let cmp_dir = TempDir::new().unwrap();

        // 源目录: IMG_001.RW2, IMG_002.RW2, IMG_003.RW2
        create_files(&src_dir, &[("IMG_001", "RW2"), ("IMG_002", "RW2"), ("IMG_003", "RW2")]);
        // 对比目录: IMG_001.jpg, IMG_002.jpg (IMG_003 不在)
        create_files(&cmp_dir, &[("IMG_001", "jpg"), ("IMG_002", "jpg")]);

        let source = scanner::scan_directory(src_dir.path(), &Default::default(), None).unwrap();

        let (matched, unmatched) = find_matching(&source, cmp_dir.path(), None, None).unwrap();

        assert_eq!(matched.len(), 2, "IMG_001, IMG_002 matched");
        assert_eq!(unmatched.len(), 1, "IMG_003 unmatched");
    }

    #[test]
    fn test_find_matching_with_format_filter() {
        let src_dir = TempDir::new().unwrap();
        let cmp_dir = TempDir::new().unwrap();

        create_files(&src_dir, &[("A", "RW2"), ("B", "RW2"), ("C", "jpg")]);
        create_files(&cmp_dir, &[("A", "jpg"), ("B", "jpg"), ("C", "jpg")]);

        let source = scanner::scan_directory(src_dir.path(), &Default::default(), None).unwrap();

        // 仅找源目录中的 RW2，对比目录中的 JPG
        let (matched, _unmatched) =
            find_matching(&source, cmp_dir.path(), Some("RW2"), Some("JPG")).unwrap();

        // A(RW2) 和 B(RW2) 在对比目录中有同名 jpg → matched
        assert_eq!(matched.len(), 2);
    }

    #[test]
    fn test_find_matching_compare_dir_not_found() {
        let source: Vec<Capture> = vec![];
        let result = find_matching(&source, Path::new("/nonexistent"), None, None);
        assert!(matches!(result, Err(BatchOpError::CompareDirNotFound(_))));
    }

    #[test]
    fn test_execute_delete_requires_no_target() {
        let dir = TempDir::new().unwrap();
        create_files(&dir, &[("del_me", "RW2")]);
        let source = scanner::scan_directory(dir.path(), &Default::default(), None).unwrap();

        let results = execute(&source, &[0], BatchOpType::DeleteSame, None, |_, _| {});
        assert!(results.iter().any(|r| r.contains("删除")));
        assert!(results.iter().all(|r| !r.contains("失败")));
    }
}
