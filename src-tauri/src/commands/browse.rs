use photo_tool_core::config::AppConfig;
use photo_tool_core::domain::CaptureMeta;
use photo_tool_core::scanner;
use std::path::PathBuf;
use tauri::Emitter;

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TreeNode {
    pub path: String,
    pub name: String,
    pub is_favorite: bool,
    pub has_children: bool,
    pub children: Vec<TreeNode>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenDirectoryResult {
    pub captures: Vec<CaptureMeta>,
    pub tree: Vec<TreeNode>,
    pub total_count: usize,
}

#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ScanProgress {
    percent: u32,
    path: String,
    phase: String,
}

#[tauri::command]
pub async fn open_directory(
    path: String,
    sidecar_extensions: Vec<String>,
    app: tauri::AppHandle,
) -> Result<OpenDirectoryResult, String> {
    let dir = PathBuf::from(&path);
    let config = AppConfig {
        sidecar_extensions,
        ..Default::default()
    };

    // 扫描阶段 (0-70%)
    let path_clone = path.clone();
    let app_clone = app.clone();
    let on_scan_progress: Box<dyn Fn(u32) + Send> = Box::new(move |pct| {
        let scaled = (pct as f64 * 0.7) as u32;
        let _ = app_clone.emit(
            "scan-progress",
            ScanProgress {
                percent: scaled,
                path: path_clone.clone(),
                phase: "scanning".into(),
            },
        );
    });

    let captures = scanner::scan_directory(
        &dir,
        &config.sidecar_extensions,
        &Default::default(),
        Some(on_scan_progress),
    )
    .map_err(|e| e.to_string())?;

    // 构建元数据阶段 (70-100%)
    let total = captures.len() as u32;
    for (i, _c) in captures.iter().enumerate() {
        if total > 0 && i % 5 == 0 {
            let scaled = 70 + ((i as f64 / total as f64) * 30.0) as u32;
            let _ = app.emit(
                "scan-progress",
                ScanProgress {
                    percent: scaled.min(100),
                    path: path.clone(),
                    phase: "building".into(),
                },
            );
        }
    }

    let metas: Vec<CaptureMeta> = captures.iter().map(CaptureMeta::from).collect();
    let total_count = metas.len();
    let tree = build_tree(&dir);

    // 完成后发送 100%
    let _ = app.emit(
        "scan-progress",
        ScanProgress {
            percent: 100,
            path: path.clone(),
            phase: "done".into(),
        },
    );

    Ok(OpenDirectoryResult {
        captures: metas,
        tree,
        total_count,
    })
}

#[tauri::command]
pub async fn get_directory_tree() -> Vec<TreeNode> {
    let mut roots = Vec::new();
    if let Some(home) = dirs::home_dir() {
        let pics = home.join("Pictures");
        if pics.exists()
            && let Some(n) = build_single_node(&pics, false) {
                roots.push(n);
            }
    }
    for base in &["/media", "/mnt"] {
        let base_path = PathBuf::from(base);
        if base_path.exists()
            && let Ok(entries) = std::fs::read_dir(base_path) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        roots.push(TreeNode {
                            path: path.to_string_lossy().to_string(),
                            name: path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string(),
                            is_favorite: false,
                            has_children: has_subdirs(&path),
                            children: Vec::new(),
                        });
                    }
                }
            }
    }
    roots
}

#[tauri::command]
pub async fn expand_directory(path: String) -> Result<Vec<TreeNode>, String> {
    let dir = PathBuf::from(&path);
    if !dir.is_dir() {
        return Err("Not a directory".into());
    }
    let mut children = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        let mut dirs: Vec<_> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .collect();
        dirs.sort_by_key(|d| d.file_name());
        for entry in dirs {
            children.push(TreeNode {
                path: entry.path().to_string_lossy().to_string(),
                name: entry.file_name().to_string_lossy().to_string(),
                is_favorite: false,
                has_children: has_subdirs(&entry.path()),
                children: Vec::new(),
            });
        }
    }
    Ok(children)
}

fn build_tree(active_dir: &PathBuf) -> Vec<TreeNode> {
    let mut roots = Vec::new();
    if let Some(name) = active_dir.file_name() {
        let mut children = Vec::new();
        if let Ok(entries) = std::fs::read_dir(active_dir) {
            let mut dirs: Vec<_> = entries
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .collect();
            dirs.sort_by_key(|d| d.file_name());
            for entry in dirs {
                children.push(TreeNode {
                    path: entry.path().to_string_lossy().to_string(),
                    name: entry.file_name().to_string_lossy().to_string(),
                    is_favorite: false,
                    has_children: has_subdirs(&entry.path()),
                    children: Vec::new(),
                });
            }
        }
        roots.push(TreeNode {
            path: active_dir.to_string_lossy().to_string(),
            name: name.to_string_lossy().to_string(),
            is_favorite: false,
            has_children: !children.is_empty(),
            children,
        });
    }
    roots
}

fn build_single_node(path: &PathBuf, is_favorite: bool) -> Option<TreeNode> {
    if !path.exists() {
        return None;
    }
    Some(TreeNode {
        path: path.to_string_lossy().to_string(),
        name: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string(),
        is_favorite,
        has_children: has_subdirs(path),
        children: Vec::new(),
    })
}

fn has_subdirs(path: &PathBuf) -> bool {
    if let Ok(entries) = std::fs::read_dir(path) {
        entries
            .flatten()
            .any(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
    } else {
        false
    }
}
