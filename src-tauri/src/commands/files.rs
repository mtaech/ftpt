use std::path::PathBuf;
use photo_tool_core::domain::DeleteMode;
use photo_tool_core::ops;

#[tauri::command]
pub async fn delete_captures(capture_paths: Vec<Vec<String>>, mode: String) -> Result<(), String> {
    let delete_mode = match mode.as_str() {
        "permanent" => DeleteMode::Permanent,
        _ => DeleteMode::Trash,
    };
    for paths in &capture_paths {
        for p in paths {
            let path = PathBuf::from(p);
            if path.exists() {
                ops::delete_file(&path, delete_mode).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn move_captures(capture_paths: Vec<Vec<String>>, dest: String) -> Result<(), String> {
    let dest_dir = PathBuf::from(&dest);
    std::fs::create_dir_all(&dest_dir).map_err(|e| e.to_string())?;
    for paths in &capture_paths {
        for p in paths {
            let path = PathBuf::from(p);
            if path.exists()
                && let Some(name) = path.file_name() {
                    let target = dest_dir.join(name);
                    if target.exists() { std::fs::remove_file(&target).ok(); }
                    std::fs::rename(&path, &target).map_err(|e| e.to_string())?;
                }
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn copy_captures(capture_paths: Vec<Vec<String>>, dest: String) -> Result<(), String> {
    let dest_dir = PathBuf::from(&dest);
    std::fs::create_dir_all(&dest_dir).map_err(|e| e.to_string())?;
    for paths in &capture_paths {
        for p in paths {
            let path = PathBuf::from(p);
            if path.exists()
                && let Some(name) = path.file_name() {
                    let target = dest_dir.join(name);
                    std::fs::copy(&path, &target).map_err(|e| e.to_string())?;
                }
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn rename_captures(items: Vec<(String, String)>) -> Result<(), String> {
    for (old_path, new_name) in &items {
        let old = PathBuf::from(old_path);
        if let Some(parent) = old.parent() {
            let new = parent.join(new_name);
            std::fs::rename(&old, &new).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
