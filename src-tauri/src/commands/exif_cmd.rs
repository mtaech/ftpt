use std::path::PathBuf;
use photo_tool_core::domain::ImageFormat;
use photo_tool_core::exif;

#[tauri::command]
pub async fn get_exif(path: String) -> Result<exif::ExifMetadata, String> {
    let path_buf = PathBuf::from(&path);
    let ext = path_buf.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let format = ImageFormat::from_extension(&ext).unwrap_or(ImageFormat::Jpeg);
    exif::extract_exif(&path_buf, &format).map_err(|e| e.to_string())
}
