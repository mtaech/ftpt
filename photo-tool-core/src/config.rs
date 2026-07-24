use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("JSON 序列化失败: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Theme {
    Light,
    Dark,
}

/// 缩略图缓存模式
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ThumbnailCacheMode {
    /// 不使用缓存
    None,
    /// 缓存到磁盘，退出时保留
    Persistent,
    /// 缓存到磁盘，退出时清理
    Volatile,
}

impl Default for ThumbnailCacheMode {
    fn default() -> Self {
        Self::Persistent
    }
}

impl ThumbnailCacheMode {
    pub fn is_enabled(&self) -> bool {
        !matches!(self, Self::None)
    }

    pub fn is_volatile(&self) -> bool {
        matches!(self, Self::Volatile)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)] // 缺字段一律回退 Default（FFI/旧配置兼容）
pub struct AppConfig {
    pub sidecar_extensions: Vec<String>,
    pub thumbnail_size: u32,
    pub favorite_dirs: Vec<String>,
    pub last_directory: Option<String>,
    /// 最近打开的目录（最多 10 个，最新在前）
    #[serde(default)]
    pub recent_directories: Vec<String>,
    pub theme: Theme,
    pub default_delete_mode: String,
    pub window_width: u32,
    pub window_height: u32,
    pub left_panel_width: u32,
    pub right_panel_visible: bool,
    pub thumbnail_cache_mode: ThumbnailCacheMode,
    pub max_cache_size_mb: u64,
    pub font_family: String,
}
fn default_font_family() -> String {
    "Microsoft YaHei UI".to_string()
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            sidecar_extensions: vec!["xmp".to_string()],
            thumbnail_size: 220,
            favorite_dirs: vec![],
            last_directory: None,
            recent_directories: vec![],
            theme: Theme::Light,
            default_delete_mode: "trash".to_string(),
            window_width: 1400,
            window_height: 900,
            left_panel_width: 260,
            right_panel_visible: true,
            thumbnail_cache_mode: ThumbnailCacheMode::default(),
            max_cache_size_mb: 500,
            font_family: default_font_family(),
        }
    }
}

/// 确定配置数据库路径：便携模式优先
///
/// 1. 检查二进制同目录是否存在 PT.db
/// 2. 若 exe 不在系统目录（如 /usr、C:\\Program Files）则默认使用便携路径
/// 3. 否则回落平台标准配置目录（`dirs::config_dir()/PT/PT.db`）
pub fn determine_config_path() -> Result<PathBuf, std::io::Error> {
    let exe = std::env::current_exe()?;
    let exe_dir = exe
        .parent()
        .ok_or_else(|| std::io::Error::other("无法获取可执行文件目录"))?;
    let portable = exe_dir.join("PT.db");

    if portable.exists() {
        return Ok(portable);
    }

    let exe_str = exe.to_string_lossy().to_lowercase();
    let is_system = exe_str.contains("\\program files")
        || exe_str.contains("\\windows")
        || exe_str.starts_with("/usr/")
        || exe_str.starts_with("/opt/")
        || exe_str.starts_with("/bin/");

    if !is_system {
        return Ok(portable);
    }

    let cfg = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("PT")
        .join("PT.db");
    if let Some(parent) = cfg.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(cfg)
}

/// 从 SQLite 数据库加载配置，文件不存在时返回默认配置。
/// 如果旧版 PT.toml 存在而 PT.db 不存在，自动迁移。
pub fn load_config(path: &Path) -> Result<AppConfig, ConfigError> {
    if path.exists() {
        return load_from_sqlite(path);
    }
    // 迁移：尝试从旧版 PT.toml 读取
    let toml_path = path.with_file_name("PT.toml");
    if toml_path.exists() {
        if let Ok(cfg) = load_from_toml(&toml_path) {
            save_config(path, &cfg)?;
            return Ok(cfg);
        }
    }
    Ok(AppConfig::default())
}

fn load_from_toml(path: &Path) -> Result<AppConfig, ConfigError> {
    let content = std::fs::read_to_string(path)?;
    Ok(toml::from_str(&content).unwrap_or_default())
}

fn load_from_sqlite(path: &Path) -> Result<AppConfig, ConfigError> {
    let conn = rusqlite::Connection::open(path)?;
    init_config_table(&conn)?;
    let json: Option<String> = conn
        .query_row(
            "SELECT data FROM config WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    match json {
        Some(j) => Ok(serde_json::from_str(&j).unwrap_or_default()),
        None => Ok(AppConfig::default()),
    }
}

/// 保存配置到 SQLite 数据库（自动创建父目录和表）
pub fn save_config(path: &Path, config: &AppConfig) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = rusqlite::Connection::open(path)?;
    init_config_table(&conn)?;
    let json = serde_json::to_string_pretty(config)?;
    conn.execute(
        "INSERT OR REPLACE INTO config (id, data) VALUES (1, ?1)",
        rusqlite::params![json],
    )?;
    Ok(())
}

fn init_config_table(conn: &rusqlite::Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS config (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            data TEXT NOT NULL
         );",
    )
}

/// 辅助：将 rusqlite::Result 转为 Option
trait OptionalResult<T> {
    fn optional(self) -> rusqlite::Result<Option<T>>;
}

impl<T> OptionalResult<T> for rusqlite::Result<T> {
    fn optional(self) -> rusqlite::Result<Option<T>> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_default_config() {
        let cfg = AppConfig::default();
        assert!(cfg.sidecar_extensions.contains(&"xmp".to_string()));
        assert_eq!(cfg.thumbnail_size, 220);
        assert_eq!(cfg.theme, Theme::Light);
        assert_eq!(cfg.default_delete_mode, "trash");
        assert_eq!(cfg.window_width, 1400);
        assert_eq!(cfg.window_height, 900);
        assert_eq!(cfg.font_family, "Microsoft YaHei UI");
    }

    #[test]
    fn test_missing_json_field_falls_back_to_default() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("PT.db");
        let legacy_json = r#"{"sidecarExtensions":["xmp"],"thumbnailSize":220}"#;
        let conn = rusqlite::Connection::open(&path).unwrap();
        init_config_table(&conn).unwrap();
        conn.execute(
            "INSERT INTO config (id, data) VALUES (1, ?1)",
            rusqlite::params![legacy_json],
        ).unwrap();

        let loaded = load_config(&path).unwrap();
        assert_eq!(loaded.font_family, "Microsoft YaHei UI");
    }

    #[test]
    fn test_save_and_load() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("PT.db");
        let cfg = AppConfig {
            thumbnail_size: 300,
            theme: Theme::Dark,
            font_family: "Microsoft YaHei".to_string(),
            ..Default::default()
        };
        save_config(&path, &cfg).unwrap();
        assert!(path.exists());

        let loaded = load_config(&path).unwrap();
        assert_eq!(loaded.thumbnail_size, 300);
        assert_eq!(loaded.theme, Theme::Dark);
        assert_eq!(loaded.font_family, "Microsoft YaHei");
    }

    #[test]
    fn test_toml_migration() {
        let dir = TempDir::new().unwrap();
        let toml_path = dir.path().join("PT.toml");
        let db_path = dir.path().join("PT.db");

        let cfg = AppConfig {
            thumbnail_size: 300,
            theme: Theme::Dark,
            font_family: "Consolas".to_string(),
            ..Default::default()
        };
        let toml_str = toml::to_string_pretty(&cfg).unwrap();
        std::fs::write(&toml_path, toml_str).unwrap();

        // 只存在 PT.toml，PT.db 不存在 → 自动迁移
        let loaded = load_config(&db_path).unwrap();
        assert_eq!(loaded.thumbnail_size, 300);
        assert_eq!(loaded.theme, Theme::Dark);
        assert_eq!(loaded.font_family, "Consolas");
        assert!(db_path.exists());
    }

    #[test]
    fn test_load_nonexistent_returns_default() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.db");
        let loaded = load_config(&path).unwrap();
        assert_eq!(loaded.thumbnail_size, 220);
        assert_eq!(loaded.theme, Theme::Light);
    }

    #[test]
    fn test_config_path() {
        let result = determine_config_path();
        assert!(result.is_ok());
    }
}
