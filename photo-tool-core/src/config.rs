use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("TOML parse error: {0}")]
    TomlParse(#[from] toml::de::Error),
    #[error("TOML serialize error: {0}")]
    TomlSer(#[from] toml::ser::Error),
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
    /// 全局缓存（默认路径：系统缓存目录/PT/thumbnails）
    Global,
    /// 全局缓存，用户指定路径
    GlobalCustom(PathBuf),
    /// 每个目录下 .PT-thumbnails/（路径不可更改）
    PerDirectory,
}

impl Default for ThumbnailCacheMode {
    fn default() -> Self {
        Self::Global
    }
}

impl ThumbnailCacheMode {
    /// 解析缓存目录的实际路径
    ///
    /// - `Global` → `dirs::cache_dir()/PT/thumbnails`
    /// - `GlobalCustom(p)` → `p`
    /// - `PerDirectory` → 调用方自行拼接 `<scan_dir>/.PT-thumbnails`
    pub fn resolve_path(&self) -> Option<PathBuf> {
        match self {
            Self::Global => dirs::cache_dir().map(|p| p.join("PT").join("thumbnails")),
            Self::GlobalCustom(p) => Some(p.clone()),
            Self::PerDirectory => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)] // 缺字段一律回退 Default（FFI/旧配置兼容）
pub struct AppConfig {
    /// 旁车文件扩展名列表（不含点，如 ["xmp"]）
    pub sidecar_extensions: Vec<String>,
    /// 缩略图尺寸（像素，正方形）
    pub thumbnail_size: u32,
    /// 常用目录列表
    pub favorite_dirs: Vec<PathBuf>,
    /// 上次打开的目录
    pub last_directory: Option<PathBuf>,
    /// 主题
    pub theme: Theme,
    /// 默认删除模式："trash" | "permanent"
    pub default_delete_mode: String,
    /// 窗口宽度
    pub window_width: u32,
    /// 窗口高度
    pub window_height: u32,
    /// 左侧面板宽度
    pub left_panel_width: u32,  // 目录树面板宽度
    /// 右侧面板可见
    pub right_panel_visible: bool,
    /// 缩略图缓存模式
    pub thumbnail_cache_mode: ThumbnailCacheMode,
    /// 最大缩略图缓存大小（MB）
    pub max_cache_size_mb: u64,
    /// 界面字体家族名
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

/// 确定配置文件路径：便携模式优先
///
/// 1. 检查二进制同目录是否存在 PT.toml
/// 2. 若 exe 不在系统目录（如 /usr、C:\Program Files）则默认使用便携路径
/// 3. 否则回落平台标准配置目录（`dirs::config_dir()/PT/PT.toml`）
pub fn determine_config_path() -> Result<PathBuf, std::io::Error> {
    if let Ok(exe_path) = std::env::current_exe()
        && let Some(exe_dir) = exe_path.parent()
    {
            let portable_config = exe_dir.join("PT.toml");
            if portable_config.exists() {
                return Ok(portable_config);
            }

            // 检查 exe_dir 是否为系统目录
            let dir_str = exe_dir.to_string_lossy();
            let is_system_dir = cfg!(target_os = "linux")
                .then(|| dir_str.starts_with("/usr") || dir_str.starts_with("/bin")
                    || dir_str.starts_with("/sbin") || dir_str.starts_with("/opt"))
                .unwrap_or(false)
                || (cfg!(target_os = "windows")
                    && (dir_str.starts_with("C:\\Windows") || dir_str.starts_with("C:\\Program")));

            if !is_system_dir {
                // 便携模式：二进制同目录
                return Ok(portable_config);
            }
        }

    // 回落平台标准目录
    let config_dir = dirs::config_dir()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "系统无配置目录"))?;
    let pt_dir = config_dir.join("PT");
    std::fs::create_dir_all(&pt_dir)?;
    Ok(pt_dir.join("PT.toml"))
}

/// 从 TOML 文件加载配置，文件不存在时返回默认配置
pub fn load_config(path: &Path) -> Result<AppConfig, ConfigError> {
    if !path.exists() {
        return Ok(AppConfig::default());
    }
    let content = std::fs::read_to_string(path)?;
    Ok(toml::from_str(&content)?)
}

/// 保存配置到 TOML 文件（自动创建父目录）
pub fn save_config(path: &Path, config: &AppConfig) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = toml::to_string_pretty(config)?;
    std::fs::write(path, content)?;
    Ok(())
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
    fn test_old_toml_without_font_family() {
        // 旧版 PT.toml 缺少 fontFamily 字段时应回落到默认值
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("PT.toml");

        // 序列化一份完整配置，然后删掉 fontFamily 行模拟旧文件
        let full = toml::to_string_pretty(&AppConfig::default()).unwrap();
        let legacy: String = full
            .lines()
            .filter(|l| !l.starts_with("fontFamily"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!legacy.contains("fontFamily"));
        std::fs::write(&path, legacy).unwrap();

        let loaded = load_config(&path).unwrap();
        assert_eq!(loaded.font_family, "Microsoft YaHei UI");
    }

    #[test]
    fn test_font_family_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("PT.toml");

        let cfg = AppConfig {
            font_family: "Microsoft YaHei".to_string(),
            ..Default::default()
        };
        save_config(&path, &cfg).unwrap();

        // 确认序列化为 camelCase 键
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("fontFamily"));

        let loaded = load_config(&path).unwrap();
        assert_eq!(loaded.font_family, "Microsoft YaHei");
    }

    #[test]
    fn test_save_and_load() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("PT.toml");

        let cfg = AppConfig {
            thumbnail_size: 300,
            theme: Theme::Dark,
            favorite_dirs: vec![PathBuf::from("/home/user/Pictures")],
            ..Default::default()
        };

        save_config(&path, &cfg).unwrap();
        assert!(path.exists());

        let loaded = load_config(&path).unwrap();
        assert_eq!(loaded.thumbnail_size, 300);
        assert_eq!(loaded.theme, Theme::Dark);
        assert_eq!(loaded.favorite_dirs.len(), 1);
    }

    #[test]
    fn test_config_path() {
        let result = determine_config_path();
        // 测试环境中应返回一个有效路径（便携或无写权限时回落平台目录）
        assert!(result.is_ok());
    }
}
