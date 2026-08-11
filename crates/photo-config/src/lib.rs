use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use rusqlite_migration::{Migrations, M};

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("Migration error: {0}")]
    Migration(#[from] rusqlite_migration::Error),
    #[error("JSON 序列化失败: {0}")]
    Json(#[from] serde_json::Error),
    #[error("TOML 解析失败: {0}")]
    Toml(#[from] toml::de::Error),
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Theme {
    Light,
    Dark,
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppConfig {
    pub thumbnail_size: u32,
    pub favorite_dirs: Vec<String>,
    pub last_directory: Option<String>,
    #[serde(default)]
    pub recent_directories: Vec<String>,
    pub theme: Theme,
    pub left_panel_width: u32,
    pub right_panel_visible: bool,
    /// 右侧信息面板宽度（px）。旧配置无此字段时默认 200。
    #[serde(default = "default_right_panel_width")]
    pub right_panel_width: u32,
    pub font_family: String,
    /// 批量识别线程数（1-4）。低配设备减小，高配设备加大。默认 4（8 核以上 CPU）。
    #[serde(default = "default_recognition_threads")]
    pub recognition_thread_count: u32,
    /// 导出预设列表（T1 批次：命名模板/长边/质量组合）。旧配置无此字段时为空。
    #[serde(default)]
    pub export_presets: Vec<ExportPreset>,
    /// 扫描是否包含子目录（递归扫描全部子层）。默认 false = 单层扫描（保持现状）。
    /// 布尔字段无需钳制；改动后需重新扫描生效（scan 编排处按此值选单层/递归）。
    #[serde(default)]
    pub include_subdirectories: bool,
}

/// 导出预设（T1 批次）：导出对话框的可复用组合（预设名 + 长边 + JPEG 质量 + 命名模板）。
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ExportPreset {
    /// 预设名（对话框下拉显示）
    pub name: String,
    /// 长边像素上限（None = 原尺寸；0 在钳制时归一为 None）
    pub long_edge: Option<u32>,
    /// JPEG 质量 1-100（保存/使用时钳制）
    pub quality: u8,
    /// 命名模板（占位符语法见 engine template.rs：{name}/{species}/{date}/{seq}/{camera}）
    pub template: String,
}

impl ExportPreset {
    /// 字段钳制：质量 1-100；长边 0 → None（无缩放）。
    pub fn clamped(mut self) -> Self {
        self.quality = self.quality.clamp(1, 100);
        if self.long_edge == Some(0) {
            self.long_edge = None;
        }
        self
    }
}

impl Default for ExportPreset {
    fn default() -> Self {
        Self {
            name: "原图".to_string(),
            long_edge: None,
            quality: 95,
            template: "{name}".to_string(),
        }
    }
}

fn default_right_panel_width() -> u32 {
    200
}

fn default_font_family() -> String {
    "Microsoft YaHei UI".to_string()
}

fn default_recognition_threads() -> u32 {
    4
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            thumbnail_size: 220,
            favorite_dirs: vec![],
            last_directory: None,
            recent_directories: vec![],
            theme: Theme::Light,
            left_panel_width: 180,
            right_panel_visible: true,
            right_panel_width: 200,
            font_family: default_font_family(),
            recognition_thread_count: default_recognition_threads(),
            include_subdirectories: false,
            export_presets: vec![ExportPreset::default()],
        }
    }
}

/// 配置表迁移
fn config_migrations() -> Migrations<'static> {
    Migrations::new(vec![M::up(
        "CREATE TABLE IF NOT EXISTS config (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            data TEXT NOT NULL
         );",
    )])
}

pub fn determine_config_path() -> Result<PathBuf, std::io::Error> {
    let exe = std::env::current_exe()?;
    let exe_dir = exe
        .parent()
        .ok_or_else(|| std::io::Error::other("无法获取可执行文件目录"))?;
    let portable = exe_dir.join("PT.db");

    if portable.exists() {
        return Ok(portable);
    }

    if !is_system_location(&exe) {
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

/// 判断 exe 是否位于系统安装位置。
/// 按路径段/前缀匹配而非子串匹配，避免 `E:\backup\Windows\ftpt.exe` 这类路径被误判为系统安装。
fn is_system_location(exe: &Path) -> bool {
    let lower = exe.to_string_lossy().to_lowercase();
    // Unix 系统目录前缀
    if lower.starts_with("/usr/") || lower.starts_with("/opt/") || lower.starts_with("/snap/") {
        return true;
    }
    // Windows：仅当系统目录紧跟盘符根目录时才视为系统安装（C:\Windows\...、C:\Program Files\...），
    // 普通目录下同名文件夹（如 E:\backup\Windows\...）仍视为便携
    if lower.as_bytes().get(1) == Some(&b':') && lower.as_bytes().get(2) == Some(&b'\\') {
        let rest = &lower[3..];
        return rest.starts_with("windows\\")
            || rest.starts_with("program files\\")
            || rest.starts_with("program files (x86)\\");
    }
    false
}

pub fn load_config(path: &Path) -> Result<AppConfig, ConfigError> {
    if path.exists() {
        return load_from_sqlite(path);
    }
    let toml_path = path.with_file_name("PT.toml");
    if toml_path.exists() {
        match load_from_toml(&toml_path) {
            Ok(cfg) => {
                save_config(path, &cfg)?;
                return Ok(cfg);
            }
            Err(e) => {
                // 旧 TOML 损坏时不静默迁移成全默认 PT.db：保留原文件，仅记日志
                tracing::warn!("PT.toml 解析失败，跳过迁移（沿用默认配置）: {e}");
            }
        }
    }
    Ok(AppConfig::default())
}

fn load_from_toml(path: &Path) -> Result<AppConfig, ConfigError> {
    let content = std::fs::read_to_string(path)?;
    Ok(toml::from_str(&content)?)
}

fn load_from_sqlite(path: &Path) -> Result<AppConfig, ConfigError> {
    let load = || -> Result<AppConfig, ConfigError> {
        let mut conn = rusqlite::Connection::open(path)?;
        config_migrations().to_latest(&mut conn)?;
        let json: Option<Option<String>> = conn
            .query_row(
                "SELECT data FROM config WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        Ok(match json.flatten() {
            Some(j) => serde_json::from_str(&j).unwrap_or_default(),
            None => AppConfig::default(),
        })
    };
    match load() {
        Ok(cfg) => Ok(cfg),
        // PT.db 被截断/写成垃圾字节时，open 惰性成功、迁移/查询阶段才报 SQLITE_NOTADB。
        // 此时把损坏文件改名保留现场，重建空库并返回默认配置，避免后续 load/save 永久失败。
        Err(e) if is_database_corruption(&e) => {
            let mut backup_name = path.as_os_str().to_os_string();
            backup_name.push(".bak");
            let backup = PathBuf::from(backup_name);
            // 先清理旧备份，避免 Windows 上 rename 因目标已存在而失败
            let _ = std::fs::remove_file(&backup);
            std::fs::rename(path, &backup)?;
            rebuild_fresh_database(path)?;
            Ok(AppConfig::default())
        }
        Err(e) => Err(e),
    }
}

/// 遍历错误链，判断是否为“库文件损坏”（SQLITE_NOTADB：目标文件不是 SQLite 数据库）。
/// 普通 IO 错误（权限、路径不存在等）不属于损坏，仍会照常传播。
fn is_database_corruption(err: &ConfigError) -> bool {
    let mut cur: Option<&(dyn std::error::Error + 'static)> = Some(err);
    while let Some(e) = cur {
        if let Some(sqlite_err) = e.downcast_ref::<rusqlite::Error>()
            && sqlite_err.sqlite_error_code() == Some(rusqlite::ErrorCode::NotADatabase)
        {
            return true;
        }
        cur = e.source();
    }
    false
}

/// 以全新库重建（空库并应用迁移），保证后续 load/save 可正常落盘。
fn rebuild_fresh_database(path: &Path) -> Result<(), ConfigError> {
    let mut conn = rusqlite::Connection::open(path)?;
    config_migrations().to_latest(&mut conn)?;
    Ok(())
}

pub fn save_config(path: &Path, config: &AppConfig) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut conn = rusqlite::Connection::open(path)?;
    config_migrations().to_latest(&mut conn)?;
    let json = serde_json::to_string_pretty(config)?;
    conn.execute(
        "INSERT OR REPLACE INTO config (id, data) VALUES (1, ?1)",
        rusqlite::params![json],
    )?;
    Ok(())
}

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
        assert_eq!(cfg.thumbnail_size, 220);
        assert_eq!(cfg.theme, Theme::Light);
        // 扫描子目录开关默认关闭（保持单层扫描现状）
        assert!(!cfg.include_subdirectories);
    }

    #[test]
    fn test_missing_json_field_falls_back_to_default() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("PT.db");
        let legacy_json = r#"{"sidecarExtensions":["xmp"],"thumbnailSize":220}"#;
        let mut conn = rusqlite::Connection::open(&path).unwrap();
        config_migrations().to_latest(&mut conn).unwrap();
        conn.execute(
            "INSERT INTO config (id, data) VALUES (1, ?1)",
            rusqlite::params![legacy_json],
        ).unwrap();
        let loaded = load_config(&path).unwrap();
        assert_eq!(loaded.font_family, "Microsoft YaHei UI");
        // 旧配置无 includeSubdirectories 字段 → serde(default) 回退 false
        assert!(!loaded.include_subdirectories);
    }

    #[test]
    fn test_include_subdirectories_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("PT.db");
        let cfg = AppConfig {
            include_subdirectories: true,
            ..Default::default()
        };
        save_config(&path, &cfg).unwrap();
        let loaded = load_config(&path).unwrap();
        assert!(loaded.include_subdirectories);
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
    }

    #[test]
    fn test_config_path() {
        let result = determine_config_path();
        assert!(result.is_ok());
    }

    #[test]
    fn test_export_preset_default_and_clamp() {
        let p = ExportPreset::default();
        assert_eq!(p.name, "原图");
        assert_eq!(p.long_edge, None);
        assert_eq!(p.quality, 95);
        assert_eq!(p.template, "{name}");

        // 质量钳制 1-100；长边 0 → None
        let low = ExportPreset {
            quality: 0,
            long_edge: Some(0),
            ..ExportPreset::default()
        }
        .clamped();
        assert_eq!(low.quality, 1);
        assert_eq!(low.long_edge, None);

        let high = ExportPreset {
            quality: 200,
            long_edge: Some(8000),
            ..ExportPreset::default()
        }
        .clamped();
        assert_eq!(high.quality, 100);
        assert_eq!(high.long_edge, Some(8000));
    }

    #[test]
    fn test_export_presets_default_list() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.export_presets.len(), 1);
        assert_eq!(cfg.export_presets[0], ExportPreset::default());
    }

    #[test]
    fn test_export_presets_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("PT.db");
        let cfg = AppConfig {
            export_presets: vec![
                ExportPreset {
                    name: "网络分享".into(),
                    long_edge: Some(2000),
                    quality: 85,
                    template: "{species}_{seq}".into(),
                },
                ExportPreset::default(),
            ],
            ..Default::default()
        };
        save_config(&path, &cfg).unwrap();
        let loaded = load_config(&path).unwrap();
        assert_eq!(loaded.export_presets.len(), 2);
        assert_eq!(loaded.export_presets[0].name, "网络分享");
        assert_eq!(loaded.export_presets[0].long_edge, Some(2000));
        assert_eq!(loaded.export_presets[0].quality, 85);
        assert_eq!(loaded.export_presets[0].template, "{species}_{seq}");
        assert_eq!(loaded.export_presets[1], ExportPreset::default());
    }
}
