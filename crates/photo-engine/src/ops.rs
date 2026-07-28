use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use thiserror::Error;

use photo_domain::{Capture, DeleteMode};

use crate::folder_db::{FolderDb, FolderDbError};

#[derive(Error, Debug)]
pub enum OpError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Trash error: {0}")]
    Trash(#[from] trash::Error),
    #[error("File not found: {0}")]
    NotFound(PathBuf),
}

/// 收集一次拍摄涉及的所有文件路径
fn all_capture_paths(capture: &Capture) -> Vec<PathBuf> {
    capture
        .source_files
        .iter()
        .map(|f| f.path.clone())
        .collect()
}

/// 删除文件：回收站或永久删除
pub fn delete_file(path: &Path, mode: DeleteMode) -> Result<(), OpError> {
    if !path.exists() {
        return Err(OpError::NotFound(path.to_path_buf()));
    }
    match mode {
        DeleteMode::Trash => {
            trash::delete(path)?;
        }
        DeleteMode::Permanent => {
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}

/// 删除一次拍摄的所有文件
pub fn delete_capture(capture: &Capture, mode: DeleteMode) -> Result<(), OpError> {
    for path in &all_capture_paths(capture) {
        if path.exists() {
            delete_file(path, mode)?;
        }
    }
    Ok(())
}

/// 批量删除拍摄，返回每个文件的结果
pub fn delete_captures(
    captures: &[&Capture],
    mode: DeleteMode,
) -> Vec<(PathBuf, Result<(), OpError>)> {
    captures
        .iter()
        .flat_map(|c| {
            all_capture_paths(c).into_iter().map(move |p| {
                let result = if p.exists() {
                    delete_file(&p, mode)
                } else {
                    Err(OpError::NotFound(p.clone()))
                };
                (p, result)
            })
        })
        .collect()
}

/// 移动一次拍摄的所有文件到目标目录
/// 先尝试 rename，若跨文件系统（EXDEV）则 copy + 删除源文件回退
pub fn move_capture(capture: &Capture, dest_dir: &Path) -> Result<(), OpError> {
    std::fs::create_dir_all(dest_dir)?;
    for path in &all_capture_paths(capture) {
        if !path.exists() {
            continue;
        }
        if let Some(name) = path.file_name() {
            let dest = dest_dir.join(name);
            // 尝试快速重命名（同文件系统）
            match std::fs::rename(path, &dest) {
                Ok(()) => {}
                Err(e) if e.kind() == ErrorKind::CrossesDevices => {
                    // 跨文件系统：copy + delete 回退
                    std::fs::copy(path, &dest)?;
                    std::fs::remove_file(path)?;
                }
                Err(e) => return Err(OpError::Io(e)),
            }
        }
    }
    Ok(())
}

/// 复制一次拍摄的所有文件到目标目录
pub fn copy_capture(capture: &Capture, dest_dir: &Path, overwrite: bool) -> Result<(), OpError> {
    std::fs::create_dir_all(dest_dir)?;
    for path in &all_capture_paths(capture) {
        if !path.exists() {
            continue;
        }
        if let Some(name) = path.file_name() {
            let dest = dest_dir.join(name);
            if dest.exists() && !overwrite {
                continue;
            }
            std::fs::copy(path, &dest)?;
        }
    }
    Ok(())
}

/// 批量重命名拍摄
pub fn rename_captures(
    captures: &[&Capture],
    new_prefix: &str,
    start_seq: u32,
    digit_count: usize,
) -> Vec<(PathBuf, Result<(), OpError>)> {
    let mut results = Vec::new();

    for (i, capture) in captures.iter().enumerate() {
        let seq = start_seq + i as u32;
        let new_base = format!("{}{:0width$}", new_prefix, seq, width = digit_count);

        for source_file in &capture.source_files {
            let old_path = &source_file.path;
            if !old_path.exists() {
                results.push((old_path.clone(), Err(OpError::NotFound(old_path.clone()))));
                continue;
            }
            if let Some(ext) = old_path.extension() {
                let new_name = format!("{}.{}", new_base, ext.to_string_lossy());
                let new_path = old_path.with_file_name(&new_name);
                let result = std::fs::rename(old_path, &new_path).map_err(OpError::from);
                results.push((new_path, result));
            }
        }
    }

    results
}

/// 删除文件后同步删除对应识别行。
/// rel_paths 是相对于文件夹根路径的路径列表（正斜杠）。
pub fn sync_delete_recognitions(db: &FolderDb, rel_paths: &[String]) -> Result<(), FolderDbError> {
    db.delete_recognitions(rel_paths)
}

/// 复制文件后同步复制对应识别行到目标库。
/// entries: (源 rel_path, 目标 rel_path)
pub fn sync_copy_recognitions(src_db: &FolderDb, dst_db: &mut FolderDb, entries: &[(String, String)]) -> Result<(), FolderDbError> {
    src_db.copy_recognitions_to(dst_db, entries)
}

/// 移动文件后同步迁移识别行到目标库。
/// 等价于 sync_copy_recognitions + sync_delete_recognitions（源库的删除由调用方负责，
/// 因为 move 的语义是先复制到目标再删除源）。
pub fn sync_move_recognitions(src_db: &FolderDb, dst_db: &mut FolderDb, entries: &[(String, String)]) -> Result<(), FolderDbError> {
    src_db.copy_recognitions_to(dst_db, entries)?;
    let src_paths: Vec<String> = entries.iter().map(|(s, _)| s.clone()).collect();
    src_db.delete_recognitions(&src_paths)
}

/// 重命名文件后同步重命名对应识别行。
pub fn sync_rename_recognition(db: &FolderDb, old_rel: &str, new_rel: &str) -> Result<(), FolderDbError> {
    db.rename_recognition(old_rel, new_rel)
}

/// 生成不会冲突的文件名：如果 path 已存在，追加 _1/_2/... 后缀
pub fn resolve_name_conflict(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }

    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_default();
    let parent = path.parent().unwrap_or(Path::new("."));

    for i in 1.. {
        let new_name = if ext.is_empty() {
            format!("{}_{}", stem, i)
        } else {
            format!("{}_{}.{}", stem, i, ext)
        };
        let new_path = parent.join(&new_name);
        if !new_path.exists() {
            return new_path;
        }
    }

    path.to_path_buf() // unreachable, but satisfies compiler
}

#[cfg(test)]
mod tests {
    use super::*;
    use photo_domain::{ImageFormat, SourceFile};
    use tempfile::TempDir;

    fn make_test_capture(dir: &TempDir, base: &str, exts: &[&str]) -> Capture {
        let mut source_files = Vec::new();
        for ext in exts {
            let path = dir.path().join(format!("{}.{}", base, ext));
            // 创建文件
            std::fs::write(&path, b"test data").unwrap();
            let is_sidecar = *ext == "xmp";
            let format = if is_sidecar || *ext == "jpg" || *ext == "jpeg" {
                ImageFormat::Jpeg
            } else if *ext == "NEF" || *ext == "nef" {
                ImageFormat::Raw("NEF".into())
            } else {
                ImageFormat::Jpeg
            };
            source_files.push(SourceFile {
                path,
                format,
                is_sidecar,
                file_size: None,
            });
        }
        // 主显示图片取第一个非旁车
        let primary_index = source_files.iter().position(|f| !f.is_sidecar).unwrap_or(0);
        Capture {
            base_name: base.to_string(),
            source_files,
            primary_index,
        }
    }

    #[test]
    fn test_move_capture_with_sidecar() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        let capture = make_test_capture(&src, "img", &["jpg", "NEF", "xmp"]);

        let result = move_capture(&capture, dst.path());
        assert!(result.is_ok(), "move failed: {:?}", result.err());

        // 目标目录文件存在
        assert!(dst.path().join("img.jpg").exists(), "jpg missing in dest");
        assert!(dst.path().join("img.NEF").exists(), "NEF missing in dest");
        assert!(dst.path().join("img.xmp").exists(), "xmp missing in dest");

        // 源目录文件不存在
        assert!(!src.path().join("img.jpg").exists(), "jpg still in src");
        assert!(!src.path().join("img.NEF").exists(), "NEF still in src");
    }

    #[test]
    fn test_permanent_delete() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.jpg");
        std::fs::write(&path, b"data").unwrap();
        assert!(path.exists());

        let result = delete_file(&path, DeleteMode::Permanent);
        assert!(result.is_ok());
        assert!(!path.exists(), "file still exists after permanent delete");
    }

    #[test]
    fn test_copy_capture() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        let capture = make_test_capture(&src, "copy_test", &["jpg", "NEF"]);

        let result = copy_capture(&capture, dst.path(), false);
        assert!(result.is_ok(), "copy failed: {:?}", result.err());

        // 目标文件存在
        assert!(dst.path().join("copy_test.jpg").exists());
        assert!(dst.path().join("copy_test.NEF").exists());
        // 源文件仍在
        assert!(src.path().join("copy_test.jpg").exists());
    }

    #[test]
    fn test_rename_captures() {
        let dir = TempDir::new().unwrap();
        let capture = make_test_capture(&dir, "DSC_0001", &["jpg", "NEF"]);

        let results = rename_captures(&[&capture], "旅行_", 1, 3);
        assert_eq!(results.len(), 2);

        // 所有结果应为 Ok
        for (_, r) in &results {
            let ok = r.is_ok();
            assert!(ok, "rename failed");
        }

        // 新文件名存在
        assert!(dir.path().join("旅行_001.jpg").exists());
        assert!(dir.path().join("旅行_001.NEF").exists());
        // 旧文件名不存在
        assert!(!dir.path().join("DSC_0001.jpg").exists());
    }

    #[test]
    fn test_resolve_name_conflict() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.jpg");
        std::fs::write(&path, b"data").unwrap();

        let resolved = resolve_name_conflict(&path);
        assert_ne!(resolved, path);
        assert_eq!(resolved.file_name().unwrap(), "test_1.jpg");

        // 第二次调用
        std::fs::write(&resolved, b"data").unwrap();
        let resolved2 = resolve_name_conflict(&path);
        assert_eq!(resolved2.file_name().unwrap(), "test_2.jpg");
    }

    #[test]
    fn test_delete_capture() {
        let dir = TempDir::new().unwrap();
        let capture = make_test_capture(&dir, "delete_me", &["jpg", "NEF"]);

        let result = delete_capture(&capture, DeleteMode::Permanent);
        assert!(result.is_ok());
        assert!(!dir.path().join("delete_me.jpg").exists());
        assert!(!dir.path().join("delete_me.NEF").exists());
    }

    #[test]
    fn test_delete_file_not_found() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("does_not_exist.jpg");
        let result = delete_file(&path, DeleteMode::Permanent);
        assert!(result.is_err());
        match result {
            Err(OpError::NotFound(p)) => assert_eq!(p, path),
            _ => panic!("expected NotFound error"),
        }
    }

    #[test]
    fn test_delete_nonexistent_skipped_in_batch() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("exists.jpg");
        std::fs::write(&path, b"data").unwrap();
        let capture = Capture {
            base_name: "exists".into(),
            source_files: vec![
                SourceFile {
                    path: path.clone(),
                    format: ImageFormat::Jpeg,
                    is_sidecar: false,
                    file_size: None,
                },
                SourceFile {
                    path: dir.path().join("nonexistent.jpg"),
                    format: ImageFormat::Jpeg,
                    is_sidecar: false,
                    file_size: None,
                },
            ],
            primary_index: 0,
        };

        // delete_capture should succeed despite one missing file
        let result = delete_capture(&capture, DeleteMode::Permanent);
        assert!(result.is_ok());
    }
}
