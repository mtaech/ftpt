use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
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
/// （拍摄时间差 ≤2s 的连拍合并，前端按 dateTaken 聚类）。默认 None = 不堆叠。
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum StackMode {
    None,
    ByFileName,
    ByTime,
}

impl Default for StackMode {
    fn default() -> Self {
        Self::None
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
    /// Material You 主题 seed 色（`#RRGGBB`）。None = 用默认墨黑 seed `#111111`。
    /// 读入/保存时统一归一为小写带 `#` 的六位十六进制；非法值归一为 None。
    #[serde(default)]
    pub accent_color: Option<String>,
    pub left_panel_width: u32,
    pub right_panel_visible: bool,
    /// 右侧信息面板宽度（px）。旧配置无此字段时默认 200。
    #[serde(default = "default_right_panel_width")]
    pub right_panel_width: u32,
    pub font_family: String,
    /// 识别鸟体定位来源（默认 Yolo = 全图 YOLO 检测；Focus = 优先相机对焦点 ROI，
    /// 无对焦点时回退 YOLO）。枚举无钳制；改动后对下次批量识别生效。
    #[serde(default)]
    pub detection_source: DetectionSource,
    /// 导出预设列表（T1 批次：命名模板/长边/质量组合）。旧配置无此字段时为空。
    #[serde(default)]
    pub export_presets: Vec<ExportPreset>,
    /// 调整预设列表（右栏「调整」tab：曝光/对比度/饱和度组合）。旧配置无此字段时为空。
    #[serde(default)]
    pub adjust_presets: Vec<AdjustPreset>,
    /// 上次使用的导出目标目录（导出对话框预填；None = 未设置，回退 `<当前目录>/exports`）。
    /// 旧配置无此字段时为 None（serde 静默忽略）；导入目标目录仍不记忆（#14 另一半）。
    #[serde(default)]
    pub export_dir: Option<String>,
    /// 常驻识别器空闲 N 分钟后自动释放（0 = 关闭，默认关闭）。
    /// 常驻约 600MB；释放后下次识别/框选自动重新装配（约 1-2 秒）。手改配置越界时钳到 0..=240。
    #[serde(default)]
    pub recognizer_idle_unload_minutes: u32,
    /// 上次使用的导入目标根目录（导入弹窗预填；None = 未设置，回退当前浏览目录）。
    /// 旧配置无此字段时为 None（serde 静默忽略）。
    #[serde(default)]
    pub import_dir: Option<String>,
    /// 扫描是否包含子目录（递归扫描全部子层）。默认 false = 单层扫描（保持现状）。
    /// 布尔字段无需钳制；改动后需重新扫描生效（scan 编排处按此值选单层/递归）。
    #[serde(default)]
    pub include_subdirectories: bool,
    /// 网格堆叠模式（默认 None = 不堆叠；旧配置无此字段时回退默认）。
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

/// 调整预设（ADR 0007 补充）：一组可复用的曝光/对比度/饱和度。
///
/// 与 sliders 同口径：曝光 ±3.0 EV 且 0.3 量化、对比度/饱和度 ±100。
/// 空列表是合法默认（调整没有「原图预设」这种有意义的东西）。
#[cfg_attr(feature = "specta", derive(specta::Type))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct AdjustPreset {
    pub name: String,
    pub exposure: f32,
    pub contrast: i32,
    pub saturation: i32,
    pub shadows: i32,
    pub highlights: i32,
    pub temperature: i32,
    pub tint: i32,
}

impl AdjustPreset {
    /// 字段钳制：与 UI 的量化口径一致（0.3 步进、±3.0 / ±100）。
    /// 非有限曝光（NaN/inf）归零——否则存进 TOML 会写出读不回来的 nan。
    pub fn clamped(mut self) -> Self {
        let ev = if self.exposure.is_finite() {
            self.exposure
        } else {
            0.0
        };
        self.exposure = ((ev / 0.3).round() * 0.3).clamp(-3.0, 3.0);
        self.exposure = (self.exposure * 100.0).round() / 100.0;
        self.contrast = self.contrast.clamp(-100, 100);
        self.saturation = self.saturation.clamp(-100, 100);
        self.shadows = self.shadows.clamp(-100, 100);
        self.highlights = self.highlights.clamp(-100, 100);
        self.temperature = self.temperature.clamp(-100, 100);
        self.tint = self.tint.clamp(-100, 100);
        self
    }
}

impl Default for AdjustPreset {
    fn default() -> Self {
        Self {
            name: "预设".to_string(),
            exposure: 0.0,
            contrast: 0,
            saturation: 0,
            shadows: 0,
            highlights: 0,
            temperature: 0,
            tint: 0,
        }
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
            left_panel_width: 200,
            right_panel_visible: true,
            right_panel_width: 200,
            font_family: default_font_family(),
            detection_source: DetectionSource::default(),
            include_subdirectories: false,
            export_presets: vec![ExportPreset::default()],
            adjust_presets: vec![],
            export_dir: None,
            import_dir: None,
            recognizer_idle_unload_minutes: 0,
            stack_mode: StackMode::default(),
            grid_columns: default_grid_columns(),
            ui_scale: default_ui_scale(),
        }
    }
}

impl AppConfig {
    /// 字段钳制（加载与保存共用）：手改配置文件越界时读入即归一，避免
    /// 字号 0 / 列数 0 这类值把 UI 拖坏；与 set_app_config 保存路径同一套范围。
    pub fn clamped(mut self) -> Self {
        self.thumbnail_size = self.thumbnail_size.clamp(64, 1024);
        self.left_panel_width = self.left_panel_width.clamp(200, 480);
        self.right_panel_width = self.right_panel_width.clamp(200, 480);
        self.grid_columns = self.grid_columns.clamp(2, 5);
        self.ui_scale = self.ui_scale.clamp(70, 200);
        // 空闲自动卸载：0 = 关闭；上限 240 分钟（再长就没有「空闲回收」的意义了）
        self.recognizer_idle_unload_minutes = self.recognizer_idle_unload_minutes.min(240);
        self.accent_color = self.accent_color.as_deref().and_then(normalize_accent_hex);
        self.export_presets = self
            .export_presets
            .into_iter()
            .map(ExportPreset::clamped)
            .collect();
        // 调整预设：逐条钳制 + 上限 20 条（配置文件不该被无界增长拖大）
        self.adjust_presets = self
            .adjust_presets
            .into_iter()
            .map(AdjustPreset::clamped)
            .take(20)
            .collect();
        self
    }

    /// 保留 `current` 里的运行时字段（收藏 / 上次目录 / 最近打开）。
    /// 这三个由后端命令维护（扫描、add/remove_favorite、remove_recent），而设置面板提交的
    /// 是启动时的快照——照单全收会把「本次启动后新打开的目录 / 新收藏」清掉。
    pub fn keep_runtime_state(mut self, current: &AppConfig) -> Self {
        self.favorite_dirs = current.favorite_dirs.clone();
        self.last_directory = current.last_directory.clone();
        self.recent_directories = current.recent_directories.clone();
        self
    }
}

/// 归一化 Material You seed 色：接受 `#RRGGBB` / `RRGGBB`（大小写不限），
/// 统一输出小写带 `#`。三位缩写、带 alpha 的八位、含非法字符一律判为 None
/// （由调用方回退默认 seed），避免把坏值喂给 HCT 生成器。
pub fn normalize_accent_hex(input: &str) -> Option<String> {
    let s = input.trim();
    let body = s.strip_prefix('#').unwrap_or(s);
    if body.len() != 6 || !body.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(format!("#{}", body.to_ascii_lowercase()))
}

/// 默认 seed：近黑。色相无关、chroma ≈ 0，生成"墨白"单色 accent。
pub const DEFAULT_ACCENT_HEX: &str = "#111111";

/// 配置目录（config.toml 与 logs/ 都在这里）。
///
/// 解析优先级（与 PHOTO_EXIFTOOL / PHOTO_DATA_DIR 同一风格）：
/// 1. `PHOTO_CONFIG_DIR` —— 显式覆盖，值就是目录本身（无头冒烟、多套配置并行用）
/// 2. `XDG_CONFIG_HOME` —— Linux/BSD 惯例，取 `$XDG_CONFIG_HOME/pt`
/// 3. 否则 `<home>/.config/pt`（Windows 即 `%USERPROFILE%\.config\pt`）
///
/// 空串按「未设置」处理（POSIX）。
///
/// 第 2 条是必须的：冒烟脚本一直写着 `XDG_CONFIG_HOME=/tmp/... 隔离配置`，
/// 而此前这里硬编码 home，于是每次冒烟都在读写**用户真实配置**——`lease_smoke`
/// 的「左停靠区宽度」检查会因为真实配置里被拖宽过的值而假失败。
///
/// 刻意不用 `dirs::config_dir()`：它在 Windows 给 `%APPDATA%\`，
/// 会破坏「全平台同一相对布局」这个约定。
pub fn config_dir() -> Result<PathBuf, std::io::Error> {
    config_dir_from(
        std::env::var_os("PHOTO_CONFIG_DIR"),
        std::env::var_os("XDG_CONFIG_HOME"),
        dirs::home_dir(),
    )
}

/// `config_dir` 的纯逻辑：环境变量与 home 由调用方传入，便于单测（不动进程全局 env）。
fn config_dir_from(
    override_dir: Option<std::ffi::OsString>,
    xdg_config_home: Option<std::ffi::OsString>,
    home: Option<PathBuf>,
) -> Result<PathBuf, std::io::Error> {
    if let Some(dir) = override_dir.filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    if let Some(dir) = xdg_config_home.filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(dir).join("pt"));
    }
    let home = home.ok_or_else(|| std::io::Error::other("无法定位用户主目录"))?;
    Ok(home.join(".config").join("pt"))
}

pub fn determine_config_path() -> Result<PathBuf, std::io::Error> {
    // 全平台统一：目录由 config_dir() 决定（默认 ~/.config/pt），文件名固定 config.toml
    let dir = config_dir()?;
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
    // 手改配置可能越界（应用自带「打开配置文件」入口）：读入即钳制
    Ok(toml::from_str::<AppConfig>(&content)?.clamped())
}

pub fn save_config(path: &Path, config: &AppConfig) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = toml::to_string_pretty(config)?;
    // 原子写：先写同目录临时文件再 rename，避免写入中途崩溃/断电留下截断
    // 的 TOML（下次启动解析失败会静默回退默认配置，用户设置全丢）
    let tmp = path.with_file_name(format!(
        "{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy()
    ));
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, path)?;
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
    fn test_app_config_clamped_normalizes_out_of_range() {
        let cfg = AppConfig {
            thumbnail_size: 99_999,
            left_panel_width: 10,
            right_panel_width: 9_999,
            grid_columns: 99,
            ui_scale: 0,
            export_presets: vec![ExportPreset {
                quality: 0,
                long_edge: Some(0),
                ..ExportPreset::default()
            }],
            ..AppConfig::default()
        };
        let c = cfg.clamped();
        assert_eq!(c.thumbnail_size, 1024);
        assert_eq!(c.left_panel_width, 200);
        assert_eq!(c.right_panel_width, 480);
        assert_eq!(c.grid_columns, 5);
        assert_eq!(c.ui_scale, 70);
        assert_eq!(c.export_presets[0].quality, 1);
        assert_eq!(c.export_presets[0].long_edge, None);
    }

    #[test]
    fn test_recognizer_idle_unload_default_and_clamp() {
        // 默认关闭（不改变现状：不会莫名让下一次识别慢 1-2 秒）
        assert_eq!(AppConfig::default().recognizer_idle_unload_minutes, 0);

        // 手改配置文件越界 → 钳到 240；负值不可能（u32）
        let cfg = AppConfig {
            recognizer_idle_unload_minutes: 99_999,
            ..AppConfig::default()
        };
        assert_eq!(cfg.clamped().recognizer_idle_unload_minutes, 240);

        // 缺键的旧配置 → 回退默认 0
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "uiScale = 100\n").unwrap();
        assert_eq!(
            load_config(&path).unwrap().recognizer_idle_unload_minutes,
            0
        );
    }

    #[test]
    fn test_import_dir_roundtrip_and_missing_key() {
        // 旧配置没有这个键 → None（serde 静默忽略，与 export_dir 同款，#14 导入侧）
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "uiScale = 100\n").unwrap();
        let cfg = load_config(&path).unwrap();
        assert_eq!(cfg.import_dir, None);

        // 写入后能原样读回（导入弹窗预填用）
        let mut cfg = AppConfig::default();
        cfg.import_dir = Some("/tmp/pt-import-dest".to_string());
        save_config(&path, &cfg).unwrap();
        let back = load_config(&path).unwrap();
        assert_eq!(back.import_dir.as_deref(), Some("/tmp/pt-import-dest"));
    }

    #[test]
    fn test_load_clamps_out_of_range_toml() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        // 手改配置文件越界（应用自带「打开配置文件」入口）→ 读入即钳制
        // TOML 键与 serde camelCase 一致（uiScale/gridColumns）
        std::fs::write(&path, "uiScale = 0\ngridColumns = 0\n").unwrap();
        let cfg = load_config(&path).unwrap();
        assert_eq!(cfg.ui_scale, 70);
        assert_eq!(cfg.grid_columns, 2);
    }

    #[test]
    fn test_save_config_is_atomic_no_tmp_left() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        save_config(&path, &AppConfig::default()).unwrap();
        assert!(path.exists());
        // 原子写不残留临时文件
        assert!(!dir.path().join("config.toml.tmp").exists());
        assert_eq!(
            load_config(&path).unwrap().thumbnail_size,
            AppConfig::default().thumbnail_size
        );
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
    fn test_keep_runtime_state_preserves_backend_fields() {
        // 设置面板提交的是启动时快照：收藏/上次目录/最近打开必须保留后端当前值，
        // 否则改任意设置都会清掉本次启动后新打开的目录
        let current = AppConfig {
            favorite_dirs: vec!["/a".into()],
            last_directory: Some("/b".into()),
            recent_directories: vec!["/b".into(), "/a".into()],
            ..Default::default()
        };
        let incoming = AppConfig {
            favorite_dirs: vec![],
            last_directory: Some("/old".into()),
            recent_directories: vec!["/old".into()],
            thumbnail_size: 320,
            ..Default::default()
        };
        let merged = incoming.keep_runtime_state(&current);
        assert_eq!(merged.favorite_dirs, vec!["/a".to_string()]);
        assert_eq!(merged.last_directory.as_deref(), Some("/b"));
        assert_eq!(
            merged.recent_directories,
            vec!["/b".to_string(), "/a".to_string()]
        );
        // 用户设置照常生效
        assert_eq!(merged.thumbnail_size, 320);
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
    fn test_normalize_accent_hex() {
        // 合法：带/不带 #、大小写 → 统一小写带 #
        assert_eq!(normalize_accent_hex("#3B82F6").as_deref(), Some("#3b82f6"));
        assert_eq!(normalize_accent_hex("3b82f6").as_deref(), Some("#3b82f6"));
        assert_eq!(
            normalize_accent_hex("  #FFFfff  ").as_deref(),
            Some("#ffffff")
        );
        // 非法：三位缩写、八位带 alpha、非法字符、空串 → None
        assert_eq!(normalize_accent_hex("#abc"), None);
        assert_eq!(normalize_accent_hex("#3b82f6ff"), None);
        assert_eq!(normalize_accent_hex("#gggggg"), None);
        assert_eq!(normalize_accent_hex(""), None);
    }

    #[test]
    fn test_clamped_normalizes_accent_color() {
        let bad = AppConfig {
            accent_color: Some("3B82F6".into()),
            ..Default::default()
        }
        .clamped();
        assert_eq!(bad.accent_color.as_deref(), Some("#3b82f6"));

        let worse = AppConfig {
            accent_color: Some("not-a-color".into()),
            ..Default::default()
        }
        .clamped();
        assert_eq!(worse.accent_color, None);
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
    fn test_config_dir_precedence() {
        // 1. PHOTO_CONFIG_DIR 最高优先级，且值就是目录本身
        assert_eq!(
            config_dir_from(
                Some("/tmp/pt-iso".into()),
                Some("/tmp/xdg".into()),
                Some("/home/u".into())
            )
            .unwrap(),
            PathBuf::from("/tmp/pt-iso")
        );
        // 2. XDG_CONFIG_HOME 次之，取 pt 子目录（冒烟脚本靠它隔离真实配置）
        assert_eq!(
            config_dir_from(None, Some("/tmp/xdg".into()), Some("/home/u".into())).unwrap(),
            PathBuf::from("/tmp/xdg/pt")
        );
        // 3. 都没有 → ~/.config/pt（Windows 同一相对布局，刻意不用 dirs::config_dir()）
        assert_eq!(
            config_dir_from(None, None, Some("/home/u".into())).unwrap(),
            PathBuf::from("/home/u/.config/pt")
        );
        // 4. 空串按未设置处理（POSIX）
        assert_eq!(
            config_dir_from(Some("".into()), Some("".into()), Some("/home/u".into())).unwrap(),
            PathBuf::from("/home/u/.config/pt")
        );
        // 5. 既无覆盖也无 home → 报错，不静默落到 cwd
        assert!(config_dir_from(None, None, None).is_err());
    }

    #[test]
    fn test_config_path_sits_in_config_dir() {
        // 不假设 home/XDG 的具体值（开发机可能设了 XDG_CONFIG_HOME），只校验两者的关系
        let dir = config_dir().unwrap();
        let path = determine_config_path().unwrap();
        assert_eq!(path.parent(), Some(dir.as_path()));
        assert_eq!(path.file_name().and_then(|n| n.to_str()), Some("config.toml"));
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
    fn test_adjust_preset_default_and_clamp() {
        let cfg = AppConfig::default();
        assert!(cfg.adjust_presets.is_empty(), "调整预设默认空列表");

        // 曝光按 0.3 量化并钳到 ±3；对比度/饱和度钳到 ±100
        let p = AdjustPreset {

            name: "夜景".into(),
            exposure: 9.0,
            contrast: 300,
            saturation: -300,
            ..AdjustPreset::default()
        }
        .clamped();
        assert_eq!(p.exposure, 3.0);
        assert_eq!(p.contrast, 100);
        assert_eq!(p.saturation, -100);

        let q = AdjustPreset {
            exposure: 0.37,
            ..AdjustPreset::default()
        }
        .clamped();
        assert_eq!(q.exposure, 0.3);

        // 3.13 四个新参数同样逐条钳制
        let wide = AdjustPreset {
            shadows: 300,
            highlights: -300,
            temperature: 120,
            tint: -120,
            ..AdjustPreset::default()
        }
        .clamped();
        assert_eq!(wide.shadows, 100);
        assert_eq!(wide.highlights, -100);
        assert_eq!(wide.temperature, 100);
        assert_eq!(wide.tint, -100);

        // 非有限曝光归零（否则 TOML 里会出现读不回来的 nan）
        let nan = AdjustPreset {
            exposure: f32::NAN,
            ..AdjustPreset::default()
        }
        .clamped();
        assert_eq!(nan.exposure, 0.0);
    }

    #[test]
    fn test_adjust_presets_roundtrip_and_cap() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        let cfg = AppConfig {
            adjust_presets: vec![AdjustPreset {

                name: "提亮".into(),
                exposure: 0.9,
                contrast: 12,
                saturation: -5,
            ..AdjustPreset::default()
        }],
            ..Default::default()
        };
        save_config(&path, &cfg).unwrap();
        let loaded = load_config(&path).unwrap();
        assert_eq!(loaded.adjust_presets.len(), 1);
        assert_eq!(loaded.adjust_presets[0].name, "提亮");
        assert_eq!(loaded.adjust_presets[0].exposure, 0.9);
        assert_eq!(loaded.adjust_presets[0].contrast, 12);

        // 上限：手改配置塞 30 条，读入即截到 20
        let many = AppConfig {
            adjust_presets: (0..30)
                .map(|i| AdjustPreset {
                    name: format!("p{i}"),
                    ..AdjustPreset::default()
                })
                .collect(),
            ..Default::default()
        };
        save_config(&path, &many).unwrap();
        let capped = load_config(&path).unwrap();
        assert_eq!(capped.adjust_presets.len(), 20);
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
