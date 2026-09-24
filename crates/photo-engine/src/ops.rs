use std::collections::HashSet;
use std::ffi::OsString;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use thiserror::Error;

use photo_domain::Capture;

use crate::folder_db::{FolderDb, FolderDbError};
use crate::template::NameTemplateContext;
use crate::undo::{TrashedItem, UndoOp};

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
    delete_file_to_trash(path).map(|_| ())
}

/// 删除单个文件到回收站，并返回可用于撤销恢复的回收站条目。
///
/// 逆操作依赖回收站条目（Linux 的 .trashinfo 路径 / Windows 的 shell 标识），
/// 这里用「删除前后 list() 差集 + 原路径匹配」定位：删除动作本身不会因为定位失败
/// 而回滚，定位不到时返回 `Ok(None)`——文件已进回收站，只是这一条不可撤销，
/// 调用方据此不记撤销日志。
pub fn delete_file_to_trash(path: &Path) -> Result<Option<TrashedItem>, OpError> {
    if !path.exists() {
        return Err(OpError::NotFound(path.to_path_buf()));
    }
    let before = trash_ids();
    trash::delete(path)?;
    Ok(locate_trashed(path, &before))
}

/// 回收站条目里能做恢复的平台（Linux freedesktop / Windows）
#[cfg(any(
    target_os = "windows",
    all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android")))
))]
fn trash_ids() -> HashSet<OsString> {
    trash::os_limited::list()
        .map(|items| items.into_iter().map(|i| i.id).collect())
        .unwrap_or_default()
}

/// 平台不支持回收站恢复（macOS）：没有 id 可记
#[cfg(not(any(
    target_os = "windows",
    all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android")))
)))]
fn trash_ids() -> HashSet<OsString> {
    HashSet::new()
}

/// 删除后回查：找出这次新出现、且原路径匹配的回收站条目
#[cfg(any(
    target_os = "windows",
    all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android")))
))]
fn locate_trashed(path: &Path, before: &HashSet<OsString>) -> Option<TrashedItem> {
    let want = canonical_original(path);
    trash::os_limited::list()
        .ok()?
        .into_iter()
        .find(|i| !before.contains(&i.id) && i.original_path() == want)
        .map(|i| TrashedItem {
            id: i.id,
            name: i.name,
            original_parent: i.original_parent,
            time_deleted: i.time_deleted,
        })
}

#[cfg(not(any(
    target_os = "windows",
    all(unix, not(any(target_os = "macos", target_os = "ios", target_os = "android")))
)))]
fn locate_trashed(_path: &Path, _before: &HashSet<OsString>) -> Option<TrashedItem> {
    None
}

/// freedesktop 的 `TrashItem.original_parent` 是 canonicalize 过的父目录；
/// 要跟它逐字相等才能匹配，这里对齐同一种形态
fn canonical_original(path: &Path) -> PathBuf {
    match (path.parent().and_then(|p| p.canonicalize().ok()), path.file_name()) {
        (Some(parent), Some(name)) => parent.join(name),
        _ => path.to_path_buf(),
    }
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
            // 目标与源同一路径（移动到自己所在目录）→ 无操作
            if dest == *path {
                continue;
            }
            // 防覆盖：std::fs::rename 在 Unix（rename(2)）与 Windows
            // （MoveFileEx + MOVEFILE_REPLACE_EXISTING）都会静默覆盖已存在的目标，
            // 必须在此统一拦截；否则合并两个卡/回导时同名旧文件永久丢失。
            if dest.exists() {
                return Err(OpError::Io(std::io::Error::new(
                    ErrorKind::AlreadyExists,
                    format!("目标文件已存在: {}", dest.display()),
                )));
            }
            // 尝试快速重命名（同文件系统）
            match std::fs::rename(path, &dest) {
                Ok(()) => {}
                Err(e) if e.kind() == ErrorKind::CrossesDevices => {
                    // 跨文件系统：copy + delete 回退（目标存在性已在上方统一检查）
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
            // 目标与源同一路径（复制到自己所在目录）→ 无操作
            if dest == *path {
                continue;
            }
            // 防覆盖：目标已存在且未允许覆盖时报错（批量操作据此报告失败，
            // 而不是静默跳过却显示成功）
            if dest.exists() && !overwrite {
                return Err(OpError::Io(std::io::Error::new(
                    ErrorKind::AlreadyExists,
                    format!("目标文件已存在: {}", dest.display()),
                )));
            }
            std::fs::copy(path, &dest)?;
        }
    }
    Ok(())
}

/// 复制单个文件到显式目标路径（导入重命名用；父目录自动创建）。
/// 防覆盖语义与 `copy_capture` 一致：目标已存在且 `!overwrite` 时报错。
pub fn copy_file_to(src: &Path, dest: &Path, overwrite: bool) -> Result<(), OpError> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if dest == src {
        return Ok(());
    }
    if dest.exists() {
        if !overwrite {
            return Err(OpError::Io(std::io::Error::new(
                ErrorKind::AlreadyExists,
                format!("目标文件已存在: {}", dest.display()),
            )));
        }
        std::fs::remove_file(dest)?;
    }
    std::fs::copy(src, dest)?;
    Ok(())
}

/// 移动单个文件到显式目标路径（导入重命名用；父目录自动创建）。
/// 跨文件系统（EXDEV）回退 copy + delete；目标已存在时报错（不覆盖）。
pub fn move_file_to(src: &Path, dest: &Path) -> Result<(), OpError> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if dest == src {
        return Ok(());
    }
    if dest.exists() {
        return Err(OpError::Io(std::io::Error::new(
            ErrorKind::AlreadyExists,
            format!("目标文件已存在: {}", dest.display()),
        )));
    }
    match std::fs::rename(src, dest) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::CrossesDevices => {
            std::fs::copy(src, dest)?;
            std::fs::remove_file(src)?;
            Ok(())
        }
        Err(e) => Err(OpError::Io(e)),
    }
}

/// 改名（防覆盖）：目标已存在（且非自身）时报错，避免 std::fs::rename
/// 静默覆盖已有文件。返回新路径（new_name 与原名相同 = 无操作成功）。
fn rename_to(old_path: &Path, new_name: &str) -> Result<PathBuf, OpError> {
    let new_path = old_path.with_file_name(new_name);
    if new_path == old_path {
        return Ok(new_path);
    }
    if new_path.exists() {
        return Err(OpError::Io(std::io::Error::new(
            ErrorKind::AlreadyExists,
            format!("目标文件已存在: {}", new_path.display()),
        )));
    }
    std::fs::rename(old_path, &new_path)?;
    Ok(new_path)
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
                let target = old_path.with_file_name(&new_name);
                match rename_to(old_path, &new_name) {
                    Ok(new_path) => results.push((new_path, Ok(()))),
                    Err(e) => results.push((target, Err(e))),
                }
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
    meta_fn: F,
) -> Vec<(PathBuf, Result<(), OpError>)>
where
    F: FnMut(&Capture) -> NameTemplateContext,
{
    rename_captures_templated_journaled(captures, template, start_seq, meta_fn).results
}

/// 模板批量重命名的结果 + 撤销记录（UI 的 Ctrl+Z 靠它）
#[derive(Debug, Default)]
pub struct RenameOutcome {
    /// (结果路径, 结果)：成功为新路径，失败为期望的目标路径（与旧函数同形）
    pub results: Vec<(PathBuf, Result<(), OpError>)>,
    /// 逐文件撤销记录（仅成功的改名；未改名的无操作不记录）
    pub undo_ops: Vec<UndoOp>,
    pub ok_count: usize,
    pub fail_count: usize,
}

/// 模板模式批量重命名（带撤销记录）。
///
/// 模板占位符 {name}/{species}/{date}/{camera}/{seq} 与 rename_captures_templated
/// 同一套渲染；每条拍摄的全部源文件（JPG/NEF/XMP 兄弟）同步改名。
/// 只成功的改名进撤销日志，失败逐文件报告、不中断后续。
pub fn rename_captures_templated_journaled<F>(
    captures: &[&Capture],
    template: &str,
    start_seq: u32,
    mut meta_fn: F,
) -> RenameOutcome
where
    F: FnMut(&Capture) -> NameTemplateContext,
{
    let mut outcome = RenameOutcome::default();

    for (i, capture) in captures.iter().enumerate() {
        let mut ctx = meta_fn(capture);
        ctx.seq = start_seq + i as u32;
        let base = crate::template::render_name_template(template, &ctx);

        for source_file in &capture.source_files {
            let old_path = &source_file.path;
            if !old_path.exists() {
                outcome.fail_count += 1;
                outcome
                    .results
                    .push((old_path.clone(), Err(OpError::NotFound(old_path.clone()))));
                continue;
            }
            if let Some(ext) = old_path.extension() {
                let new_name = format!("{}.{}", base, ext.to_string_lossy());
                let target = old_path.with_file_name(&new_name);
                match rename_to(old_path, &new_name) {
                    Ok(new_path) => {
                        // 文件名没变（模板渲染结果与原名相同）= 无操作，不进撤销日志
                        if new_path != *old_path {
                            outcome.undo_ops.push(UndoOp::Rename {
                                from: old_path.clone(),
                                to: new_path.clone(),
                            });
                        }
                        outcome.ok_count += 1;
                        outcome.results.push((new_path, Ok(())));
                    }
                    Err(e) => {
                        outcome.fail_count += 1;
                        outcome.results.push((target, Err(e)));
                    }
                }
            }
        }
    }

    outcome
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
    fn test_move_capture_refuses_existing_target() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        let capture = make_test_capture(&src, "img", &["jpg"]);
        std::fs::write(dst.path().join("img.jpg"), b"existing").unwrap();

        let result = move_capture(&capture, dst.path());
        assert!(result.is_err(), "同名目标存在时必须报错而不是覆盖");
        // 源文件仍在、目标内容未被覆盖
        assert!(src.path().join("img.jpg").exists());
        assert_eq!(std::fs::read(dst.path().join("img.jpg")).unwrap(), b"existing");
    }

    #[test]
    fn test_copy_capture_refuses_existing_target() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        let capture = make_test_capture(&src, "img", &["jpg"]);
        std::fs::write(dst.path().join("img.jpg"), b"existing").unwrap();

        let result = copy_capture(&capture, dst.path(), false);
        assert!(result.is_err(), "同名目标存在时 copy(overwrite=false) 必须报错");
        assert_eq!(std::fs::read(dst.path().join("img.jpg")).unwrap(), b"existing");
    }

    #[test]
    fn test_copy_file_to_creates_dirs_and_refuses_overwrite() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("a.jpg");
        std::fs::write(&src, b"data").unwrap();
        let dest = dir.path().join("deep/nested/b.jpg");
        copy_file_to(&src, &dest, false).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"data");
        // 目标已存在 → 不覆盖报错，内容不变
        assert!(copy_file_to(&src, &dest, false).is_err());
        assert_eq!(std::fs::read(&dest).unwrap(), b"data");
    }

    #[test]
    fn test_move_file_to_renames_and_refuses_overwrite() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("a.jpg");
        std::fs::write(&src, b"data").unwrap();
        let dest = dir.path().join("out/renamed.jpg");
        move_file_to(&src, &dest).unwrap();
        assert!(dest.is_file() && !src.exists());
        // 目标已存在 → 报错且源保留
        let src2 = dir.path().join("b.jpg");
        std::fs::write(&src2, b"x").unwrap();
        assert!(move_file_to(&src2, &dest).is_err());
        assert!(src2.exists());
    }

    #[test]
    fn test_rename_captures_refuses_collision() {
        let dir = TempDir::new().unwrap();
        let a = make_test_capture(&dir, "A", &["jpg"]);
        let b = make_test_capture(&dir, "B", &["jpg"]);
        // 模板不唯一（无 {seq}）：A/B 都渲染成「旅行」，已存在的目标必须拒绝覆盖
        std::fs::write(dir.path().join("旅行.jpg"), b"occupied").unwrap();

        let results = rename_captures_templated(
            &[&a, &b],
            "旅行",
            1,
            |_| NameTemplateContext::default(),
        );
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|(_, r)| r.is_err()), "撞名必须全部失败");
        assert_eq!(std::fs::read(dir.path().join("旅行.jpg")).unwrap(), b"occupied");
        assert!(dir.path().join("A.jpg").exists() && dir.path().join("B.jpg").exists());
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
    fn test_rename_captures_templated_journaled_records_undo() {
        let dir = TempDir::new().unwrap();
        let capture = make_test_capture(&dir, "IMG_9001", &["jpg", "NEF"]);

        let outcome =
            rename_captures_templated_journaled(&[&capture], "{name}_renamed", 1, |cap| {
                NameTemplateContext {
                    name: cap.base_name.clone(),
                    ..Default::default()
                }
            });
        assert_eq!(outcome.ok_count, 2, "两个源文件都改名成功");
        assert_eq!(outcome.fail_count, 0);
        assert_eq!(outcome.undo_ops.len(), 2, "每个改名的文件一条撤销记录");
        assert!(dir.path().join("IMG_9001_renamed.jpg").exists());
        assert!(!dir.path().join("IMG_9001.jpg").exists());

        let undone = crate::undo::undo_ops(&outcome.undo_ops);
        assert!(undone.iter().all(|o| o.result.is_ok()), "{undone:?}");
        assert!(dir.path().join("IMG_9001.jpg").exists(), "撤销后改回原名");
        assert!(!dir.path().join("IMG_9001_renamed.jpg").exists());
    }

    #[test]
    fn test_rename_journaled_same_name_records_nothing() {
        // 模板渲染结果与原名相同 = 无操作，不该产生撤销记录（否则 Ctrl+Z 会「撤销」成空动作）
        let dir = TempDir::new().unwrap();
        let capture = make_test_capture(&dir, "same_name", &["jpg"]);

        let outcome = rename_captures_templated_journaled(&[&capture], "{name}", 1, |cap| {
            NameTemplateContext {
                name: cap.base_name.clone(),
                ..Default::default()
            }
        });
        assert_eq!(outcome.ok_count, 1);
        assert!(outcome.undo_ops.is_empty(), "同名改名不进撤销日志");
        assert!(dir.path().join("same_name.jpg").exists());
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
