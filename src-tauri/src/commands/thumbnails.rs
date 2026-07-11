use std::path::PathBuf;
use photo_tool_core::domain::{ImageFormat, SourceFile};
use photo_tool_core::thumbnail::ThumbnailCache;

pub fn make_cache() -> ThumbnailCache {
    let mut dir = dirs::cache_dir().unwrap_or_else(|| PathBuf::from(".cache"));
    dir.push("PT");
    dir.push("thumbnails");
    ThumbnailCache::new(dir)
}

#[tauri::command]
pub async fn get_thumbnail(
    path: String,
    size: u32,
    cache: tauri::State<'_, ThumbnailCache>,
) -> Result<Vec<u8>, String> {
    let path_buf = PathBuf::from(&path);
    let ext = path_buf
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let format = ImageFormat::from_extension(&ext).unwrap_or(ImageFormat::Jpeg);
    let source = SourceFile {
        path: path_buf,
        format,
        is_sidecar: false,
        file_size: None,
    };
    cache.get_or_generate(&source, size).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn clear_cache(
    cache: tauri::State<'_, ThumbnailCache>,
) -> Result<(), String> {
    cache.clear().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_cache_stats(
    cache: tauri::State<'_, ThumbnailCache>,
) -> Result<(usize, u64), String> {
    cache.stats().map_err(|e| e.to_string())
}
