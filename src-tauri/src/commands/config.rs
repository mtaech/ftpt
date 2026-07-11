use photo_tool_core::config::{self, AppConfig};

#[tauri::command]
pub async fn load_config() -> Result<AppConfig, String> {
    let config_path = config::determine_config_path().map_err(|e| e.to_string())?;
    config::load_config(&config_path).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_config(app_config: AppConfig) -> Result<(), String> {
    let config_path = config::determine_config_path().map_err(|e| e.to_string())?;
    config::save_config(&config_path, &app_config).map_err(|e| e.to_string())
}
