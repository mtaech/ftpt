use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use thiserror::Error;

use photo_domain::Capture;

use crate::folder_db::{FolderDb, FolderDbError};
use crate::template::NameTemplateContext;

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

/// 删除文件：移到回收站
pub fn delete_file(path: &Path) -> Result<(), OpError> {
    if !path.exists() {
        return Err(OpError::NotFound(path.to_path_buf()));
    }
    trash::delete(path)?;
    Ok(())
}

/// 删除一次拍摄的所有文件（移到回收站）
pub fn delete_capture(capture: &Capture) -> Result<(), OpError> {
    for path in &all_capture_paths(capture) {
        if path.exists() {
            delete_file(path)?;
        }
    }
    Ok(())
}

/// 批量删除拍摄，返回每个文件的结果（移到回收站）
pub fn delete_captures(
    captures: &[&Capture],
) -> Vec<(PathBuf, Result<(), OpError>)> {
    captures
        .iter()
        .flat_map(|c| {
            all_capture_paths(c).into_iter().map(move |p| {
                let result = if p.exists() {
                    delete_file(&p)
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
                    // 与同卷 rename 在 Windows 上的失败行为一致：目标已存在时报错，
                    // 避免 fs::copy 静默覆盖已有目标后再删源，造成双份数据丢失
                    if dest.exists() {
                        return Err(OpError::Io(std::io::Error::new(
                            ErrorKind::AlreadyExists,
                            format!("目标文件已存在: {}", dest.display()),
                        )));
                    }
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

/// 模板模式批量重命名（T1 批次，配合 [`crate::template::render_name_template`]）：
/// `template` 为命名模板（占位符 `{name}`/`{species}`/`{date}`/`{camera}`/`{seq}`）；
/// 每张拍摄的物种/日期/相机由 `meta_fn` 按 capture 组装（调用方持有 CaptureMeta 等
/// 元数据，engine 层不感知识别/EXIF 来源），序号从 `start_seq` 起按处理顺序递增。
/// 每条拍摄的全部源文件（JPG/NEF/XMP 兄弟）同步改名，与 `rename_captures` 一致。
pub fn rename_captures_templated<F>(
    captures: &[&Capture],
    template: &str,
    start_seq: u32,
    mut meta_fn: F,
) -> Vec<(PathBuf, Result<(), OpError>)>
where
    F: FnMut(&Capture) -> NameTemplateContext,
{
    let mut results = Vec::new();

    for (i, capture) in captures.iter().enumerate() {
        let mut ctx = meta_fn(capture);
        ctx.seq = start_seq + i as u32;
        let base = crate::template::render_name_template(template, &ctx);

        for source_file in &capture.source_files {
            let old_path = &source_file.path;
            if !old_path.exists() {
                results.push((old_path.clone(), Err(OpError::NotFound(old_path.clone()))));
                continue;
            }
            if let Some(ext) = old_path.extension() {
                let new_name = format!("{}.{}", base, ext.to_string_lossy());
                let new_path = old_path.with_file_name(&new_name);
                let result = std::fs::rename(old_path, &new_path).map_err(OpError::from);
                results.push((new_path, result));
            }
        }
    }

    results
}
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
/// 本函数负责：先复制识别行到目标库，再删除源库识别行（无需调用方额外处理删除）。
pub fn sync_move_recognitions(src_db: &FolderDb, dst_db: &mut FolderDb, entries: &[(String, String)]) -> Result<(), FolderDbError> {
    src_db.copy_recognitions_to(dst_db, entries)?;
    let src_paths: Vec<String> = entries.iter().map(|(s, _)| s.clone()).collect();
    src_db.delete_recognitions(&src_paths)
}

/// 重命名文件后同步重命名对应识别行。
pub fn sync_rename_recognition(db: &FolderDb, old_rel: &str, new_rel: &str) -> Result<(), FolderDbError> {
    db.rename_recognition(old_rel, new_rel)
}

// ── adjustments 同步（参数化调整，ADR 0007，键为相对路径）──

/// 删除文件后同步删除对应调整行。
pub fn sync_delete_adjustments(db: &FolderDb, rel_paths: &[String]) -> Result<(), FolderDbError> {
    db.delete_adjustments(rel_paths)
}

/// 复制文件后同步复制对应调整行到目标库。
/// entries: (源 rel_path, 目标 rel_path)
pub fn sync_copy_adjustments(
    src_db: &FolderDb,
    dst_db: &mut FolderDb,
    entries: &[(String, String)],
) -> Result<(), FolderDbError> {
    src_db.copy_adjustments_to(dst_db, entries)
}

/// 移动文件后同步迁移调整行到目标库（复制到目标 + 删除源）。
pub fn sync_move_adjustments(
    src_db: &FolderDb,
    dst_db: &mut FolderDb,
    entries: &[(String, String)],
) -> Result<(), FolderDbError> {
    src_db.copy_adjustments_to(dst_db, entries)?;
    let src_paths: Vec<String> = entries.iter().map(|(s, _)| s.clone()).collect();
    src_db.delete_adjustments(&src_paths)
}

/// 重命名文件后同步重命名对应调整行。
pub fn sync_rename_adjustment(db: &FolderDb, old_rel: &str, new_rel: &str) -> Result<(), FolderDbError> {
    db.rename_adjustment(old_rel, new_rel)
}

// ── xmp_meta 同步（评分/色标/旗标，键为完整路径）──

/// 删除文件后同步删除对应评分/色标/旗标行。
pub fn sync_delete_xmp(db: &FolderDb, paths: &[String]) -> Result<(), FolderDbError> {
    db.delete_xmp_rows(paths)
}

/// 移动文件后同步迁移 xmp_meta 行到目标库。
pub fn sync_move_xmp(src_db: &FolderDb, dst_db: &mut FolderDb, entries: &[(String, String)]) -> Result<(), FolderDbError> {
    src_db.copy_xmp_rows_to(dst_db, entries)?;
    let src_paths: Vec<String> = entries.iter().map(|(s, _)| s.clone()).collect();
    src_db.delete_xmp_rows(&src_paths)
}

/// 重命名文件后同步重命名对应 xmp_meta 行。
pub fn sync_rename_xmp(db: &FolderDb, old_path: &str, new_path: &str) -> Result<(), FolderDbError> {
    db.rename_xmp(old_path, new_path)
}

// ── keywords 同步（关键词标签，键为完整路径，与 xmp_meta 同键约定）──

/// 删除文件后同步删除对应关键词行。
pub fn sync_delete_keywords(db: &FolderDb, paths: &[String]) -> Result<(), FolderDbError> {
    db.delete_keyword_rows(paths)
}

/// 复制文件后同步复制对应关键词行到目标库。
/// entries: (源路径, 目标路径)
pub fn sync_copy_keywords(
    src_db: &FolderDb,
    dst_db: &mut FolderDb,
    entries: &[(String, String)],
) -> Result<(), FolderDbError> {
    src_db.copy_keywords_to(dst_db, entries)
}

/// 移动文件后同步迁移关键词行到目标库（复制到目标 + 删除源）。
pub fn sync_move_keywords(
    src_db: &FolderDb,
    dst_db: &mut FolderDb,
    entries: &[(String, String)],
) -> Result<(), FolderDbError> {
    src_db.copy_keywords_to(dst_db, entries)?;
    let src_paths: Vec<String> = entries.iter().map(|(s, _)| s.clone()).collect();
    src_db.delete_keyword_rows(&src_paths)
}

/// 重命名文件后同步重命名对应关键词行。
pub fn sync_rename_keywords(db: &FolderDb, old_path: &str, new_path: &str) -> Result<(), FolderDbError> {
    db.rename_keywords(old_path, new_path)
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
            std::fs::write(&path, b"test data").unwrap();
            let format = if *ext == "jpg" || *ext == "jpeg" {
                ImageFormat::Jpeg
            } else if *ext == "NEF" || *ext == "nef" {
                ImageFormat::Raw("NEF".into())
            } else {
                match ImageFormat::from_extension(ext) {
                    Some(f) => f,
                    None => ImageFormat::Jpeg, // 测试中用 Jpeg 兜底
                }
            };
            source_files.push(SourceFile {
                path,
                format,
                file_size: None,
            });
        }
        Capture {
            base_name: base.to_string(),
            source_files,
            primary_index: 0,
        }
    }

    #[test]
    fn test_move_capture_all_files() {
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
    fn test_trash_delete() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.jpg");
        std::fs::write(&path, b"data").unwrap();
        assert!(path.exists());

        let result = delete_file(&path);
        assert!(result.is_ok());
        assert!(!path.exists(), "file still exists after delete");
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

        let result = delete_capture(&capture);
        assert!(result.is_ok());
        assert!(!dir.path().join("delete_me.jpg").exists());
        assert!(!dir.path().join("delete_me.NEF").exists());
    }

    #[test]
    fn test_delete_file_not_found() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("does_not_exist.jpg");
        let result = delete_file(&path);
        assert!(result.is_err());
        match result {
            Err(OpError::NotFound(p)) => assert_eq!(p, path),
            _ => panic!("expected NotFound error"),
        }
    }

    #[test]
    fn test_rename_captures_templated() {
        let dir = TempDir::new().unwrap();
        let capture = make_test_capture(&dir, "DSC_0001", &["jpg", "NEF"]);
        let capture2 = make_test_capture(&dir, "DSC_0002", &["jpg"]);

        let results = rename_captures_templated(
            &[&capture, &capture2],
            "{name}_{species}_{seq}",
            5,
            |c| NameTemplateContext {
                name: c.base_name.clone(),
                species: Some("白鹭".to_string()),
                ..Default::default()
            },
        );
        assert_eq!(results.len(), 3);
        for (_, r) in &results {
            assert!(r.is_ok(), "rename failed: {:?}", r);
        }
        // 新文件名：原名 + 鸟种 + 补零 3 位序号
        assert!(dir.path().join("DSC_0001_白鹭_005.jpg").exists());
        assert!(dir.path().join("DSC_0001_白鹭_005.NEF").exists());
        assert!(dir.path().join("DSC_0002_白鹭_006.jpg").exists());
        // 旧文件名不存在
        assert!(!dir.path().join("DSC_0001.jpg").exists());
    }

    #[test]
    fn test_rename_captures_templated_illegal_chars_sanitized() {
        let dir = TempDir::new().unwrap();
        let capture = make_test_capture(&dir, "IMG_1", &["jpg"]);

        let results = rename_captures_templated(
            &[&capture],
            "x/y:z_{seq}",
            1,
            |c| NameTemplateContext {
                name: c.base_name.clone(),
                ..Default::default()
            },
        );
        assert!(results[0].1.is_ok(), "rename failed: {:?}", results[0].1);
        assert!(dir.path().join("xyz_001.jpg").exists());
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
                    file_size: None,
                },
                SourceFile {
                    path: dir.path().join("nonexistent.jpg"),
                    format: ImageFormat::Jpeg,
                    file_size: None,
                },
            ],
            primary_index: 0,
        };
        // delete_capture should succeed despite one missing file
        let result = delete_capture(&capture);
        assert!(result.is_ok());
    }
}
