use std::path::{Path, PathBuf};
use chrono::Datelike;
use thiserror::Error;

use crate::domain::Capture;
use crate::exif::extract_exif;

#[derive(Error, Debug)]
pub enum ImportError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("No source directory specified")]
    NoSourceDir,
}

/// 导入选项
#[derive(Debug, Clone)]
pub struct ImportOptions {
    /// "copy" 或 "move"
    pub behavior: String,
    /// 日期子目录格式
    pub date_format: String,
    /// 同名处理策略: "skip" | "overwrite" | "rename"
    pub overwrite_strategy: String,
    /// 完成后删除源文件（仅 behavior=copy 时有效）
    pub delete_after_copy: bool,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            behavior: "copy".to_string(),
            date_format: "year_month_day".to_string(),
            overwrite_strategy: "skip".to_string(),
            delete_after_copy: false,
        }
    }
}

/// 检测可移动存储设备
///
/// Linux: 扫描 /media, /mnt, /run/media/$USER 下的挂载点
/// Windows: 枚举 A-Z 驱动器
pub fn detect_removable_drives() -> Vec<PathBuf> {
    let mut drives = Vec::new();

    #[cfg(target_os = "linux")]
    {
        for base in &["/media", "/mnt"] {
            if let Ok(entries) = std::fs::read_dir(base) {
                for entry in entries.flatten() {
                    if entry.path().is_dir() {
                        drives.push(entry.path());
                    }
                }
            }
        }
        let run_media = Path::new("/run/media");
        if run_media.exists() {
            if let Ok(user_entries) = std::fs::read_dir(run_media) {
                for user_entry in user_entries.flatten() {
                    if let Ok(devices) = std::fs::read_dir(user_entry.path()) {
                        for device in devices.flatten() {
                            if device.path().is_dir() {
                                drives.push(device.path());
                            }
                        }
                    }
                }
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        for letter in b'A'..=b'Z' {
            let path_str = format!("{}:\\", letter as char);
            let path = Path::new(&path_str);
            if path.exists() && letter != b'C' {
                drives.push(path.to_path_buf());
            }
        }
    }

    drives
}

/// 扫描可移动设备上的所有照片文件
pub fn scan_device_for_photos(device_path: &Path) -> Result<Vec<PathBuf>, ImportError> {
    let mut photos = Vec::new();
    let image_exts = [
        "jpg", "jpeg", "png", "tiff", "tif",
        "nef", "nrw", "cr2", "cr3", "arw", "dng",
        "orf", "raf", "rw2", "pef", "srw",
    ];

    for entry in std::fs::read_dir(device_path)?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let dcim = path.join("DCIM");
            if dcim.exists() {
                collect_photos_recursive(&dcim, &image_exts, &mut photos);
            } else {
                collect_photos_recursive(&path, &image_exts, &mut photos);
            }
        }
    }

    Ok(photos)
}

fn collect_photos_recursive(dir: &Path, extensions: &[&str], photos: &mut Vec<PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension().and_then(|e| e.to_str())
                    && extensions.contains(&ext.to_lowercase().as_str()) {
                        photos.push(path);
                    }
            } else if path.is_dir() {
                collect_photos_recursive(&path, extensions, photos);
            }
        }
    }
}

/// 根据 EXIF 拍摄日期生成目标子目录路径
pub fn date_subfolder(date: &Option<String>, format: &str) -> String {
    let dt = date
        .as_ref()
        .and_then(|d| {
            chrono::NaiveDateTime::parse_from_str(d, "%Y:%m:%d %H:%M:%S")
                .ok()
                .or_else(|| chrono::NaiveDateTime::parse_from_str(d, "%Y-%m-%d %H:%M:%S").ok())
        })
        .unwrap_or_else(|| chrono::Local::now().naive_local());

    match format {
        "year_month_day" => format!("{}/{:02}/{:02}", dt.year(), dt.month(), dt.day()),
        "iso_date" => format!("{:04}-{:02}-{:02}", dt.year(), dt.month(), dt.day()),
        "year_iso" => format!(
            "{}/{:04}-{:02}-{:02}",
            dt.year(),
            dt.year(),
            dt.month(),
            dt.day()
        ),
        _ => format!("{}/{:02}/{:02}", dt.year(), dt.month(), dt.day()),
    }
}

/// 执行导入操作
pub fn import_captures(
    captures: &[&Capture],
    dest_root: &Path,
    options: &ImportOptions,
) -> Vec<(PathBuf, Result<(), ImportError>)> {
    let mut results = Vec::new();

    for capture in captures {
        let primary = &capture.source_files[capture.primary_index];
        let exif_date = extract_exif(&primary.path, &primary.format)
            .ok()
            .and_then(|m| m.date_time_original);

        let subfolder = date_subfolder(&exif_date, &options.date_format);
        let dest_dir = dest_root.join(&subfolder);

        let exec_result = match options.behavior.as_str() {
            "move" => crate::ops::move_capture(capture, &dest_dir)
                .map_err(|e| ImportError::Io(std::io::Error::other(e.to_string()))),
            _ => {
                let overwrite = options.overwrite_strategy == "overwrite";
                crate::ops::copy_capture(capture, &dest_dir, overwrite)
                    .map_err(|e| ImportError::Io(std::io::Error::other(e.to_string())))
            }
        };

        if exec_result.is_ok()
            && options.behavior != "move"
            && options.delete_after_copy
        {
            for sf in &capture.source_files {
                let _ = std::fs::remove_file(&sf.path);
            }
        }

        results.push((dest_dir, exec_result));
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;
    // Capture is available via super::*

    #[test]
    fn test_date_subfolder_year_month_day() {
        let date = Some("2025:01:15 10:30:00".to_string());
        assert_eq!(date_subfolder(&date, "year_month_day"), "2025/01/15");
    }

    #[test]
    fn test_date_subfolder_iso_date() {
        let date = Some("2025:01:15 10:30:00".to_string());
        assert_eq!(date_subfolder(&date, "iso_date"), "2025-01-15");
    }

    #[test]
    fn test_date_subfolder_year_iso() {
        let date = Some("2025:01:15 10:30:00".to_string());
        assert_eq!(date_subfolder(&date, "year_iso"), "2025/2025-01-15");
    }

    #[test]
    fn test_date_subfolder_none_uses_current() {
        let result = date_subfolder(&None, "iso_date");
        assert_eq!(result.len(), 10, "expected 'YYYY-MM-DD'");
        assert_eq!(&result[4..5], "-");
        assert_eq!(&result[7..8], "-");
    }

    #[test]
    fn test_date_subfolder_invalid_format_falls_back() {
        let date = Some("2025:06:15 08:00:00".to_string());
        assert_eq!(date_subfolder(&date, "unknown"), "2025/06/15");
    }

    #[test]
    fn test_date_subfolder_dash_format() {
        let date = Some("2025-01-15 10:30:00".to_string());
        assert_eq!(date_subfolder(&date, "iso_date"), "2025-01-15");
    }

    #[test]
    fn test_detect_removable_drives_does_not_crash() {
        let drives = detect_removable_drives();
        assert!(drives.len() < 100, "unrealistic drive count");
    }

    #[test]
    fn test_import_captures_empty_slice() {
        let dest = tempfile::TempDir::new().unwrap();
        let options = ImportOptions::default();
        let captures: Vec<&Capture> = vec![];
        let results = import_captures(&captures, dest.path(), &options);
        assert_eq!(results.len(), 0);
    }
}
