use std::path::{Path, PathBuf};
use thiserror::Error;
use walkdir::WalkDir;

use photo_domain::{Capture, FilterCriteria, ImageFormat, SourceFile};

#[derive(Error, Debug)]
pub enum ScanError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// 扫描目录，每个图片文件各自成为一个 Capture（JPG 与 RAW 不再堆叠）
///
/// `filter` — 仅保留签名（供调用方传当前筛选条件），扫描期不做任何筛选：
/// 识别状态等元数据在扫描后才从 folder_db 读取，全部筛选由 app 层
/// `apply_filter_and_sort` 在 CaptureMeta 层面执行。
/// `on_progress` — 可选进度回调，接收 0-100 表示扫描+归并的百分比
pub fn scan_directory(
    dir: &Path,
    _filter: &FilterCriteria,
    on_progress: Option<Box<dyn Fn(u32) + Send>>,
) -> Result<Vec<Capture>, ScanError> {
    let report = |pct: u32| {
        if let Some(ref cb) = on_progress {
            cb(pct);
        }
    };

    // 单轮收集：一次 walkdir，暂存到临时 vec（消除双轮 I/O）
    struct Entry {
        path: PathBuf,
        format: ImageFormat,
        file_size: Option<u64>,
    }

    let mut entries: Vec<Entry> = Vec::new();

    for entry in WalkDir::new(dir)
        .max_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        if path.file_stem().is_none() {
            continue;
        };

        let ext_lower = ext.to_lowercase();
        if !ImageFormat::is_viewable(&ext_lower) {
            continue;
        }
        let Some(format) = ImageFormat::from_extension(&ext_lower) else {
            continue;
        };

        // 从目录项元数据获取文件大小（NTFS 目录项已缓存，零额外开销）
        let file_size = entry.metadata().ok().map(|m| m.len());

        entries.push(Entry {
            path: path.to_path_buf(),
            format,
            file_size,
        });
    }

    let total = entries.len() as u32;
    if total == 0 {
        report(100);
        return Ok(vec![]);
    }
    report(0);

    // 每文件一个 Capture，转换并报告进度
    let mut captures: Vec<Capture> = entries
        .into_iter()
        .enumerate()
        .map(|(i, e)| {
            let display_name = e
                .path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            report(((i + 1) as f64 / total as f64 * 100.0).min(100.0) as u32);
            Capture {
                base_name: display_name,
                source_files: vec![SourceFile {
                    path: e.path,
                    format: e.format,
                    file_size: e.file_size,
                }],
                primary_index: 0,
            }
        })
        .collect();

    report(100);

    // 按完整文件名小写排序：同名 JPG/RAW 相邻且顺序确定（jpg < nef）
    captures.sort_by_key(|c| {
        c.source_files[c.primary_index]
            .path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_lowercase()
    });

    Ok(captures)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_files(dir: &TempDir, files: &[&str]) {
        for f in files {
            let path = dir.path().join(f);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&path, b"dummy").unwrap();
        }
    }

    #[test]
    fn test_jpeg_raw_split() {
        let dir = TempDir::new().unwrap();
        create_test_files(&dir, &["DSC_0001.jpg", "DSC_0001.NEF"]);

        let captures = scan_directory(dir.path(), &FilterCriteria::default(), None).unwrap();
        assert_eq!(captures.len(), 2);
        assert!(captures.iter().all(|c| c.source_files.len() == 1));
        // 按文件名排序：jpg 在前，NEF 在后
        assert_eq!(captures[0].source_files[0].format, ImageFormat::Jpeg);
        assert!(matches!(captures[1].source_files[0].format, ImageFormat::Raw(_)));
        assert!(captures.iter().all(|c| c.base_name == "DSC_0001"));
    }

    #[test]
    fn test_case_insensitive_extensions_split() {
        let dir = TempDir::new().unwrap();
        create_test_files(&dir, &["photo.JPG", "Photo.NEF"]);

        let captures = scan_directory(dir.path(), &FilterCriteria::default(), None).unwrap();
        assert_eq!(captures.len(), 2);
    }

    #[test]
    fn test_jpeg_only() {
        let dir = TempDir::new().unwrap();
        create_test_files(&dir, &["solo.jpg"]);

        let captures = scan_directory(dir.path(), &FilterCriteria::default(), None).unwrap();
        assert_eq!(captures.len(), 1);
        assert_eq!(captures[0].source_files.len(), 1);
    }

    #[test]
    fn test_raw_only() {
        let dir = TempDir::new().unwrap();
        create_test_files(&dir, &["raw_only.NEF"]);

        let captures = scan_directory(dir.path(), &FilterCriteria::default(), None).unwrap();
        assert_eq!(captures.len(), 1);
        assert!(matches!(
            captures[0].source_files[captures[0].primary_index].format,
            ImageFormat::Raw(_)
        ));
    }

    #[test]
    fn test_same_stem_multiple_formats_split() {
        let dir = TempDir::new().unwrap();
        create_test_files(&dir, &["img.jpg", "img.NEF", "img.DNG"]);

        let captures = scan_directory(dir.path(), &FilterCriteria::default(), None).unwrap();
        assert_eq!(captures.len(), 3);
        // 同名三格式按文件名小写排序（dng < jpg < nef），顺序确定
        let names: Vec<String> = captures
            .iter()
            .map(|c| c.source_files[0].path.file_name().unwrap().to_str().unwrap().to_string())
            .collect();
        assert_eq!(names, vec!["img.DNG", "img.jpg", "img.NEF"]);
    }

    #[test]
    fn test_xmp_files_ignored() {
        let dir = TempDir::new().unwrap();
        create_test_files(&dir, &["img.jpg", "img.NEF", "img.xmp"]);

        let captures = scan_directory(dir.path(), &FilterCriteria::default(), None).unwrap();
        // .xmp 非 viewable 扩展名被忽略
        assert_eq!(captures.len(), 2);
        for c in &captures {
            assert!(!c.source_files[0].path.extension().unwrap().to_str().unwrap().eq_ignore_ascii_case("xmp"));
        }
    }

    #[test]
    fn test_ignore_video() {
        let dir = TempDir::new().unwrap();
        create_test_files(&dir, &["img.jpg", "img.mp4"]);

        let captures = scan_directory(dir.path(), &FilterCriteria::default(), None).unwrap();
        assert_eq!(captures.len(), 1);
    }

    #[test]
    fn test_multiple_captures() {
        let dir = TempDir::new().unwrap();
        create_test_files(&dir, &["a.jpg", "a.NEF", "b.jpg", "b.CR2", "c.jpg"]);

        let captures = scan_directory(dir.path(), &FilterCriteria::default(), None).unwrap();
        // 每文件一个 Capture：a.jpg/a.NEF/b.jpg/b.CR2/c.jpg 共 5 个
        assert_eq!(captures.len(), 5);
    }
}
