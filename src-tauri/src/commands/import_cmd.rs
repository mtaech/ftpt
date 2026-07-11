use std::path::PathBuf;
use photo_tool_core::domain::Capture;
use photo_tool_core::import::{self, ImportOptions};

#[derive(serde::Deserialize)]
pub struct ImportInput {
    pub dest_root: String,
    pub behavior: String,
    pub date_format: String,
    pub overwrite_strategy: String,
}

#[tauri::command]
pub async fn detect_drives() -> Vec<String> {
    import::detect_removable_drives()
        .into_iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect()
}

#[tauri::command]
pub async fn import_captures(paths: Vec<Vec<String>>, options: ImportInput) -> Result<(), String> {
    // paths is a Vec of (Vec of file paths per capture)
    // For simplicity, we use the first path of each capture group
    let captures: Vec<Capture> = paths.iter().map(|group| {
        let files: Vec<_> = group.iter().map(PathBuf::from).collect();
        let stem = files.first()
            .and_then(|f| f.file_stem())
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string();
        let primary_path = files.first().cloned().unwrap_or_default();
        let dir = primary_path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
        Capture {
            base_name: stem,
            directory: dir,
            source_files: vec![],
            primary_index: 0,
        }
    }).collect();

    let opts = ImportOptions {
        behavior: options.behavior,
        date_format: options.date_format,
        overwrite_strategy: options.overwrite_strategy,
        delete_after_copy: false,
    };

    let refs: Vec<&Capture> = captures.iter().collect();
    import::import_captures(&refs, &PathBuf::from(&options.dest_root), &opts);
    Ok(())
}
