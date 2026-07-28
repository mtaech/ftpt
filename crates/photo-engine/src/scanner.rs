use std::collections::HashMap;
use std::path::{Path, PathBuf};
use thiserror::Error;
use walkdir::WalkDir;

use photo_domain::{Capture, FilterCriteria, ImageFormat, SourceFile};

#[derive(Error, Debug)]
pub enum ScanError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// 扫描目录，将所有图片文件按 stem 归并为 Capture 列表
///
/// `on_progress` — 可选进度回调，接收 0-100 表示扫描+归并的百分比
pub fn scan_directory(
    dir: &Path,
    filter: &FilterCriteria,
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
        stem: String,
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
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
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
            stem: stem.to_string(),
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

    // 归组并报告进度
    let mut base_map: HashMap<String, Vec<SourceFile>> = HashMap::new();
    for (i, e) in entries.into_iter().enumerate() {
        let base_key = e.stem.to_lowercase();
        let sf = SourceFile {
            path: e.path,
            format: e.format,
            file_size: e.file_size,
        };
        base_map.entry(base_key).or_default().push(sf);
        report(((i + 1) as f64 / total as f64 * 100.0).min(100.0) as u32);
    }

    report(100);

    // 归并为 Capture
    let mut captures: Vec<Capture> = base_map
        .into_iter()
        .map(|(base_name, source_files)| {
            let primary_index = source_files
                .iter()
                .enumerate()
                .min_by_key(|(_, f)| f.format.display_priority())
                .map(|(i, _)| i)
                .unwrap_or(0);

            let display_name = source_files
                .first()
                .and_then(|f| f.path.file_stem())
                .and_then(|s| s.to_str())
                .unwrap_or(&base_name)
                .to_string();

            Capture {
                base_name: display_name,
                source_files,
                primary_index,
            }
        })
        .collect();

    apply_filter(&mut captures, filter);
    captures.sort_by_key(|a| a.base_name.to_lowercase());

    Ok(captures)
}

/// 应用筛选条件：当前操作在 Capture 层面（无 enrich_with_recognition），
/// text_search 只能匹配 base_name；recognition_filter 在 Capture 层面
/// 全部为未识别（recognition_status=None），所以只 All/NotRecognized 保留。
///
/// 待扫描完成后，在 `apply_filter_and_sort`（app.rs）中对 CaptureMeta
/// 做更精确的识别筛选和 bird_name 文本搜索。
fn apply_filter(captures: &mut Vec<Capture>, filter: &FilterCriteria) {
    if let Some(ref text) = filter.text_search {
        let text_lower = text.to_lowercase();
        captures.retain(|c| {
            c.base_name.to_lowercase().contains(&text_lower)
        });
    }
    // recognition_filter：Capture 层面全部为未识别
    if filter.recognition_filter != photo_domain::RecognitionFilter::All
        && filter.recognition_filter != photo_domain::RecognitionFilter::NotRecognized
    {
        // 对 Capture 而言，Confirmed/NeedsReview/Unrecognized 都不可能有，
        // 因为识别扫描还没跑——直接清空
        captures.clear();
    }
}

/// 计算堆叠数：除主显示图片外的图像文件数量
pub fn stack_count(capture: &Capture) -> usize {
    capture
        .source_files
        .iter()
        .enumerate()
        .filter(|(i, _f)| *i != capture.primary_index)
        .count()
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
    fn test_pair_jpeg_raw() {
        let dir = TempDir::new().unwrap();
        create_test_files(&dir, &["DSC_0001.jpg", "DSC_0001.NEF"]);

        let captures = scan_directory(dir.path(), &FilterCriteria::default(), None).unwrap();
        assert_eq!(captures.len(), 1);
        let c = &captures[0];
        assert_eq!(c.base_name, "DSC_0001");
        assert_eq!(c.source_files.len(), 2);
        assert_eq!(c.source_files[c.primary_index].format, ImageFormat::Jpeg);
    }

    #[test]
    fn test_case_insensitive_pairing() {
        let dir = TempDir::new().unwrap();
        create_test_files(&dir, &["photo.JPG", "Photo.NEF"]);

        let captures = scan_directory(dir.path(), &FilterCriteria::default(), None).unwrap();
        assert_eq!(captures.len(), 1);
        assert_eq!(captures[0].source_files.len(), 2);
    }

    #[test]
    fn test_jpeg_only_no_pairing() {
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
    fn test_multiple_raws_same_capture() {
        let dir = TempDir::new().unwrap();
        create_test_files(&dir, &["img.jpg", "img.NEF", "img.DNG"]);

        let captures = scan_directory(dir.path(), &FilterCriteria::default(), None).unwrap();
        assert_eq!(captures.len(), 1);
        assert_eq!(captures[0].source_files.len(), 3);
    }

    #[test]
    fn test_xmp_files_ignored() {
        let dir = TempDir::new().unwrap();
        create_test_files(&dir, &["img.jpg", "img.NEF", "img.xmp"]);

        let captures = scan_directory(dir.path(), &FilterCriteria::default(), None).unwrap();
        assert_eq!(captures.len(), 1);
        // .xmp 不再被当作旁车文件——非 viewable 扩展名被忽略
        assert_eq!(captures[0].source_files.len(), 2);
        for sf in &captures[0].source_files {
            assert!(!sf.path.extension().unwrap().to_str().unwrap().eq_ignore_ascii_case("xmp"));
        }
    }

    #[test]
    fn test_ignore_video() {
        let dir = TempDir::new().unwrap();
        create_test_files(&dir, &["img.jpg", "img.mp4"]);

        let captures = scan_directory(dir.path(), &FilterCriteria::default(), None).unwrap();
        assert_eq!(captures.len(), 1);
        // mp4 不被当作图片源文件
        assert_eq!(captures[0].source_files.len(), 1);
    }

    #[test]
    fn test_multiple_captures() {
        let dir = TempDir::new().unwrap();
        create_test_files(&dir, &["a.jpg", "a.NEF", "b.jpg", "b.CR2", "c.jpg"]);

        let captures = scan_directory(dir.path(), &FilterCriteria::default(), None).unwrap();
        assert_eq!(captures.len(), 3);
    }
}
