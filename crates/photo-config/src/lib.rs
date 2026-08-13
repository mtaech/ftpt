use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("TOML 解析失败: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("TOML 序列化失败: {0}")]
    TomlSer(#[from] toml::ser::Error),
}

#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Theme {
    Light,
    Dark,
}

/// 网格堆叠模式：None = 不堆叠（每文件一项）；ByFileName = 同文件名（stem）合并
/// （JPG/NEF 同画面，前端 stacks.ts 按 baseName 分组）；ByTime = 同组照片堆叠
/// （拍摄时间差 ≤2s 的连拍合并，前端按 dateTaken 聚类）。默认 ByTime。
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum StackMode {
    None,
    ByFileName,
    ByTime,
}

impl Default for StackMode {
    fn default() -> Self {
        Self::ByTime
    }
}

/// 识别鸟体定位来源：Yolo = 全图 YOLO 检测（默认，现状）；
/// Focus = 优先用相机对焦点构造 ROI 直接分类（相机对焦位置先验可靠），
/// 无对焦点的照片回退 YOLO 全图检测。
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum DetectionSource {
    Yolo,
    Focus,
}

impl Default for DetectionSource {
    fn default() -> Self {
        Self::Yolo
    }
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
    /// Material You 主题 seed 色（`#RRGGBB`）。None = 前端用默认蓝 seed `#3b82f6`。
    #[serde(default)]
    pub accent_color: Option<String>,
    pub left_panel_width: u32,
    pub right_panel_visible: bool,
    /// 右侧信息面板宽度（px）。旧配置无此字段时默认 200。
    #[serde(default = "default_right_panel_width")]
    pub right_panel_width: u32,
    pub font_family: String,
    /// 批量识别线程数（1-4）。低配设备减小，高配设备加大。默认 4（8 核以上 CPU）。
    #[serde(default = "default_recognition_threads")]
    pub recognition_thread_count: u32,
    /// 识别鸟体定位来源（默认 Yolo = 全图 YOLO 检测；Focus = 优先相机对焦点 ROI，
    /// 无对焦点时回退 YOLO）。枚举无钳制；改动后对下次批量识别生效。
    #[serde(default)]
    pub detection_source: DetectionSource,
    /// 导出预设列表（T1 批次：命名模板/长边/质量组合）。旧配置无此字段时为空。
    #[serde(default)]
    pub export_presets: Vec<ExportPreset>,
    /// 扫描是否包含子目录（递归扫描全部子层）。默认 false = 单层扫描（保持现状）。
    /// 布尔字段无需钳制；改动后需重新扫描生效（scan 编排处按此值选单层/递归）。
    #[serde(default)]
    pub include_subdirectories: bool,
    /// 网格堆叠模式（默认 ByTime = 同组照片堆叠；旧配置无此字段时回退默认）。
    #[serde(default)]
    pub stack_mode: StackMode,
    /// 网格每行图片数（2-5，默认 4）。固定列数后 cell 宽由容器自适应，
    /// 缩略图尺寸（thumbnail_size）保留为缩略图生成尺寸（缓存键），不再驱动列数。
    #[serde(default = "default_grid_columns")]
    pub grid_columns: u32,
    /// 界面缩放比例（百分比 80-130，默认 100 = 基准字号 15px）。
    /// html font-size = 15 × ui_scale/100，Tailwind 全 rem 等比缩放整体 UI。
    #[serde(default = "default_ui_scale")]
    pub ui_scale: u32,
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

fn default_grid_columns() -> u32 {
    4
}

fn default_ui_scale() -> u32 {
    100
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            thumbnail_size: 220,
            favorite_dirs: vec![],
            last_directory: None,
            recent_directories: vec![],
            theme: Theme::Light,
            accent_color: None,
            left_panel_width: 180,
            right_panel_visible: true,
            right_panel_width: 200,
            font_family: default_font_family(),
            recognition_thread_count: default_recognition_threads(),
            detection_source: DetectionSource::default(),
            include_subdirectories: false,
            export_presets: vec![ExportPreset::default()],
            stack_mode: StackMode::default(),
            grid_columns: default_grid_columns(),
            ui_scale: default_ui_scale(),
        }
    }
}

pub fn determine_config_path() -> Result<PathBuf, std::io::Error> {
    // 全平台统一 ~/.config/pt/config.toml（Windows 为 %USERPROFILE%\.config\pt\config.toml）
    let home = dirs::home_dir()
        .ok_or_else(|| std::io::Error::other("无法定位用户主目录"))?;
    let dir = home.join(".config").join("pt");
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join("config.toml"))
}

pub fn load_config(path: &Path) -> Result<AppConfig, ConfigError> {
    if path.exists() {
        return load_from_toml(path);
    }
    Ok(AppConfig::default())
}

fn load_from_toml(path: &Path) -> Result<AppConfig, ConfigError> {
    let content = std::fs::read_to_string(path)?;
    Ok(toml::from_str(&content)?)
}

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
        assert_eq!(cfg.thumbnail_size, 220);
        assert_eq!(cfg.theme, Theme::Light);
        // 扫描子目录开关默认关闭（保持单层扫描现状）
        assert!(!cfg.include_subdirectories);
        // 识别鸟体定位默认 YOLO 全图检测（焦点优先为新开关，默认不改变现状）
        assert_eq!(cfg.detection_source, DetectionSource::Yolo);
    }

    #[test]
    fn test_detection_source_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        let cfg = AppConfig {
            detection_source: DetectionSource::Focus,
            ..Default::default()
        };
        save_config(&path, &cfg).unwrap();
        let loaded = load_config(&path).unwrap();
        assert_eq!(loaded.detection_source, DetectionSource::Focus);
    }

    #[test]
    fn test_missing_toml_field_falls_back_to_default() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "thumbnailSize = 220\n").unwrap();
        let loaded = load_config(&path).unwrap();
        assert_eq!(loaded.font_family, "Microsoft YaHei UI");
        assert!(!loaded.include_subdirectories);
    }

    #[test]
    fn test_include_subdirectories_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        let cfg = AppConfig {
            include_subdirectories: true,
            ..Default::default()
        };
        save_config(&path, &cfg).unwrap();
        let loaded = load_config(&path).unwrap();
        assert!(loaded.include_subdirectories);
    }

    #[test]
    fn test_accent_color_roundtrip() {
        // 带 seed 色：TOML 序列化往返后字段保留
        let cfg = AppConfig {
            accent_color: Some("#3b82f6".into()),
            ..Default::default()
        };
        let toml_str = toml::to_string_pretty(&cfg).unwrap();
        let loaded: AppConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(loaded.accent_color.as_deref(), Some("#3b82f6"));
        // 缺 accent_color 键的 TOML → serde(default) 回退 None
        let legacy: AppConfig = toml::from_str("theme = \"Light\"\nthumbnailSize = 220\n").unwrap();
        assert_eq!(legacy.accent_color, None);
    }

    #[test]
    fn test_save_and_load() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
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
    fn test_load_nonexistent_returns_default() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.toml");
        let loaded = load_config(&path).unwrap();
        assert_eq!(loaded.thumbnail_size, 220);
    }

    #[test]
    fn test_config_path() {
        let result = determine_config_path().unwrap();
        // 全平台统一 ~/.config/pt/config.toml（Windows 为 %USERPROFILE%\.config\...）
        let rel = result
            .strip_prefix(dirs::home_dir().unwrap())
            .expect("配置路径应在主目录下")
            .to_path_buf();
        assert_eq!(rel, PathBuf::from(".config/pt/config.toml"));
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
        let path = dir.path().join("config.toml");
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
