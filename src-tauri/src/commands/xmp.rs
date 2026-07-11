use std::path::PathBuf;
use photo_tool_core::xmp::{self, XmpMetadata};

#[tauri::command]
pub async fn read_capture_xmp(primary_path: String) -> Result<XmpMetadata, String> {
    let path = PathBuf::from(&primary_path);
    let xp = xmp::xmp_path(&path);
    xmp::read_xmp(&xp).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn write_capture_xmp(primary_path: String, metadata: XmpMetadata) -> Result<(), String> {
    let path = PathBuf::from(&primary_path);
    let xp = xmp::xmp_path(&path);
    xmp::write_xmp(&xp, &metadata).map_err(|e| e.to_string())
}
