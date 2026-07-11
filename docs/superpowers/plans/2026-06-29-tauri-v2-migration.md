# Photo Tool Tauri v2 迁移计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。步骤使用复选框（`- [ ]`）语法来跟踪进度。

**目标：** 将 photo_tool 从 Iced 0.14 原生 GUI 迁移到 Tauri v2（Vue 3 + Vite 前端，Rust 后端）

**架构：** Cargo workspace 拆分为 `photo-tool-core`（纯 Rust 库，所有业务逻辑）和 `src-tauri`（Tauri 应用壳，命令注册）。前端为 Vue 3 + Vite + Pinia + vue-router + Reka UI。IPC 通过 Tauri 命令桥接，状态按混合架构（核心数据在 Rust，UI 瞬态在前端）管理。

**技术栈：** Tauri v2, Vue 3, Vite 6, TypeScript, Pinia, vue-router, Reka UI, Rust (photo-tool-core)

---

## 文件结构

```
photo_tool/
├── Cargo.toml                          # Workspace root
├── photo-tool-core/                    # [现有] 纯 Rust 核心库
│   ├── Cargo.toml
│   ├── src/
│   │   ├── lib.rs                      # 模块声明
│   │   ├── config.rs                   # [现有] 配置读写
│   │   ├── convert.rs                  # [现有] 图片转换
│   │   ├── domain.rs                   # [现有] + Serialize 派生 + CaptureMeta
│   │   ├── exif.rs                     # [现有] EXIF 提取
│   │   ├── import.rs                   # [现有] 导入功能
│   │   ├── ops.rs                      # [现有] 文件操作
│   │   ├── scanner.rs                  # [现有] 目录扫描
│   │   ├── thumbnail.rs                # [现有] 缩略图缓存
│   │   └── xmp.rs                      # [现有] XMP 旁车文件
│
├── src-tauri/                          # [新建] Tauri 应用壳
│   ├── Cargo.toml
│   ├── build.rs
│   ├── src/
│   │   ├── main.rs                     # Tauri 入口
│   │   ├── lib.rs                      # 命令注册
│   │   └── commands/
│   │       ├── mod.rs
│   │       ├── browse.rs               # open_directory, directory_tree, expand_directory
│   │       ├── thumbnails.rs           # get_thumbnail, cache_management
│   │       ├── exif_cmd.rs             # get_exif
│   │       ├── xmp.rs                  # read_xmp, write_xmp
│   │       ├── files.rs                # delete, move, copy, rename
│   │       ├── config.rs               # load_config, save_config
│   │       ├── convert_cmd.rs          # convert_image
│   │       └── import_cmd.rs           # detect_drives, import_captures
│   ├── tauri.conf.json
│   ├── capabilities/default.json
│   └── icons/                          # 应用图标
│
├── src/                                # [新建] Vue 前端
│   ├── main.ts
│   ├── App.vue
│   ├── env.d.ts
│   ├── assets/
│   ├── styles/
│   │   ├── variables.css               # CSS custom properties（对应 theme.rs 配色）
│   │   └── global.css                  # 全局样式
│   ├── types/
│   │   ├── index.ts                    # CaptureMeta, ExifMetadata 等前端类型
│   │   └── tauri.ts                    # Tauri invoke 封装
│   ├── stores/
│   │   ├── browse.ts                   # 主浏览状态（captures, selection, filter, sort）
│   │   ├── config.ts                   # 应用配置
│   │   └── ui.ts                       # UI 布局状态（mode, panels, dialog states）
│   ├── composables/
│   │   ├── useThumbnail.ts             # 缩略图加载缓存 composable
│   │   └── useKeyboard.ts              # 键盘导航 composable
│   ├── router/
│   │   └── index.ts                    # vue-router 配置
│   ├── views/
│   │   ├── BrowseView.vue              # 主浏览视图
│   │   └── CompareView.vue             # 对比视图
│   ├── components/
│   │   ├── DirectoryTree.vue           # 目录树面板
│   │   ├── ThumbnailGrid.vue           # 缩略图网格
│   │   ├── ThumbnailCell.vue           # 单个缩略图单元格
│   │   ├── PreviewPanel.vue            # 右侧预览面板
│   │   ├── ExifTable.vue              # EXIF 信息表格
│   │   ├── Toolbar.vue                 # 工具栏（排序/筛选/操作）
│   │   ├── StatusBar.vue               # 底部状态栏
│   │   ├── ContextMenu.vue             # 右键上下文菜单
│   │   ├── Layout.vue                  # 三栏布局容器
│   │   └── dialogs/
│   │       ├── ImportDialog.vue        # 导入对话框
│   │       ├── RenameDialog.vue        # 重命名对话框
│   │       ├── SettingsDialog.vue      # 设置对话框
│   │       └── ConvertDialog.vue       # 转换对话框
│   └── widgets/
│       ├── StarRating.vue              # 星级评分
│       ├── ColorLabelPicker.vue        # 颜色标签选择器
│       └── StackBadge.vue              # 堆叠徽章
│
├── index.html
├── package.json
├── vite.config.ts
├── tsconfig.json
└── tsconfig.node.json
```

---

### 任务 1: 恢复 workspace Cargo.toml 并重建 photo-tool-core

**文件：**
- 修改：`Cargo.toml`
- 修改：`photo-tool-core/Cargo.toml`
- 创建：`photo-tool-core/src/lib.rs`

- [ ] **步骤 1: 确认 workspace Cargo.toml 内容**

```toml
[workspace]
resolver = "2"
members = ["photo-tool-core", "src-tauri"]

[workspace.package]
version = "0.1.0"
edition = "2024"
```

- [ ] **步骤 2: 确认 photo-tool-core/Cargo.toml**

```toml
[package]
name = "photo-tool-core"
version.workspace = true
edition.workspace = true

[dependencies]
rawlib = "0.3"
kamadak-exif = "0.6"
image = { version = "0.25", default-features = false, features = ["jpeg", "png", "tiff", "webp", "bmp", "gif"] }
trash = "4"
dirs = "6"
serde = { version = "1", features = ["derive"] }
toml = "0.8"
walkdir = "2"
quick-xml = { version = "0.37", features = ["serialize"] }
chrono = { version = "0.4", features = ["serde"] }
thiserror = "2"
log = "0.4"
regex-lite = "0.1"

[dev-dependencies]
tempfile = "3"
```

- [ ] **步骤 3: 确认 photo-tool-core/src/lib.rs**

```rust
pub mod config;
pub mod convert;
pub mod domain;
pub mod exif;
pub mod import;
pub mod ops;
pub mod scanner;
pub mod thumbnail;
pub mod xmp;
```

- [ ] **步骤 4: 向 domain.rs 添加 CaptureMeta + 补充 Serialize 派生**

在 `domain.rs` 中 `Capture` 后面增加：

```rust
/// 发送到前端的拍摄摘要（轻量，不含 SourceFile 完整信息）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureMeta {
    pub index: usize,
    pub base_name: String,
    pub primary_path: String,
    pub primary_format: String,
    pub stack_count: usize,
    pub file_size: Option<u64>,
    pub date_taken: Option<String>,
    pub has_xmp: bool,
    pub extensions: Vec<String>,
}

impl From<&Capture> for CaptureMeta {
    fn from(c: &Capture) -> Self {
        let primary = &c.source_files[c.primary_index];
        let ext_list: Vec<String> = c.source_files.iter()
            .filter(|f| !f.is_sidecar)
            .map(|f| f.path.extension()
                .and_then(|e| e.to_str())
                .map(|s| s.to_uppercase())
                .unwrap_or_default())
            .collect();
        Self {
            base_name: c.base_name.clone(),
            primary_path: primary.path.to_string_lossy().to_string(),
            primary_format: format!("{}", primary.format),
            stack_count: c.source_files.iter()
                .enumerate()
                .filter(|(i, f)| *i != c.primary_index && !f.is_sidecar)
                .count(),
            file_size: std::fs::metadata(&primary.path).ok().map(|m| m.len()),
            date_taken: None,
            has_xmp: c.source_files.iter().any(|f| f.is_sidecar),
            extensions: ext_list,
        }
    }
}
```

并为 `ImageFormat` 添加 `Display` 实现：

```rust
impl std::fmt::Display for ImageFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Jpeg => write!(f, "JPEG"),
            Self::Png => write!(f, "PNG"),
            Self::Tiff => write!(f, "TIFF"),
            Self::Heif => write!(f, "HEIF"),
            Self::WebP => write!(f, "WebP"),
            Self::Bmp => write!(f, "BMP"),
            Self::Gif => write!(f, "GIF"),
            Self::Raw(r) => write!(f, "{}", r),
        }
    }
}
```

- [ ] **步骤 5: 运行 `cargo build` 验证 photo-tool-core 编译通过**

```bash
cargo build -p photo-tool-core
```

- [ ] **步骤 6: Commit**

```bash
git add -A
git commit -m "refactor: split into workspace with photo-tool-core crate"
```

---

### 任务 2: 创建 src-tauri crate（Tauri 应用壳）

**文件：**
- 创建：`src-tauri/Cargo.toml`
- 创建：`src-tauri/build.rs`
- 创建：`src-tauri/src/main.rs`
- 创建：`src-tauri/src/lib.rs`
- 创建：`src-tauri/src/commands/mod.rs`
- 创建：`src-tauri/tauri.conf.json`
- 创建：`src-tauri/capabilities/default.json`

- [ ] **步骤 1: 创建 src-tauri/Cargo.toml**

```toml
[package]
name = "photo-tool-tauri"
version.workspace = true
edition.workspace = true

[dependencies]
photo-tool-core = { path = "../photo-tool-core" }
tauri = { version = "2", features = ["devtools"] }
tauri-plugin-dialog = "2"
tauri-plugin-fs = "2"
tauri-plugin-log = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }

[build-dependencies]
tauri-build = { version = "2", features = [] }
```

- [ ] **步骤 2: 创建 src-tauri/build.rs**

```rust
fn main() {
    tauri_build::build()
}
```

- [ ] **步骤 3: 创建 src-tauri/src/main.rs**

```rust
// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    photo_tool_tauri::run()
}
```

- [ ] **步骤 4: 创建 src-tauri/src/lib.rs**

```rust
mod commands;

use commands::browse;
use commands::thumbnails;
use commands::exif_cmd;
use commands::xmp;
use commands::files;
use commands::config;
use commands::convert_cmd;
use commands::import_cmd;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_log::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            browse::open_directory,
            browse::get_directory_tree,
            browse::expand_directory,
            thumbnails::get_thumbnail,
            thumbnails::clear_cache,
            thumbnails::get_cache_stats,
            exif_cmd::get_exif,
            xmp::read_capture_xmp,
            xmp::write_capture_xmp,
            files::delete_captures,
            files::move_captures,
            files::copy_captures,
            files::rename_captures,
            config::load_config,
            config::save_config,
            convert_cmd::convert_images,
            import_cmd::detect_drives,
            import_cmd::import_captures,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
```

- [ ] **步骤 5: 创建 src-tauri/src/commands/mod.rs**

```rust
pub mod browse;
pub mod thumbnails;
pub mod exif_cmd;
pub mod xmp;
pub mod files;
pub mod config;
pub mod convert_cmd;
pub mod import_cmd;
```

- [ ] **步骤 6: 创建 src-tauri/tauri.conf.json**

```json
{
  "$schema": "https://raw.githubusercontent.com/tauri-apps/tauri/dev/crates/tauri-config-schema/schema.json",
  "productName": "PT - Photo Tool",
  "version": "0.1.0",
  "identifier": "app.phototool.desktop",
  "build": {
    "beforeDevCommand": "pnpm dev",
    "devUrl": "http://localhost:1420",
    "beforeBuildCommand": "pnpm build",
    "frontendDist": "../dist"
  },
  "app": {
    "windows": [
      {
        "title": "PT - Photo Tool",
        "width": 1280,
        "height": 800,
        "resizable": true,
        "fullscreen": false
      }
    ],
    "security": {
      "csp": null
    }
  },
  "bundle": {
    "active": true,
    "targets": "all",
    "icon": [
      "icons/32x32.png",
      "icons/128x128.png",
      "icons/128x128@2x.png",
      "icons/icon.icns",
      "icons/icon.ico"
    ]
  }
}
```

- [ ] **步骤 7: 创建 src-tauri/capabilities/default.json**

```json
{
  "identifier": "default",
  "description": "Capability for the main window",
  "windows": ["main"],
  "permissions": [
    "core:default",
    "dialog:allow-open",
    "dialog:allow-ask",
    "dialog:allow-message",
    "fs:allow-read",
    "fs:allow-exists",
    "shell:allow-open"
  ]
}
```

- [ ] **步骤 8: 创建应用图标目录**

```bash
mkdir -p src-tauri/icons
# 生成占位 PNG 图标（32×32, 128×128, 256×256）
```

- [ ] **步骤 9: 运行 `cargo build -p photo-tool-tauri` 验证编译**

- [ ] **步骤 10: Commit**

```bash
git add -A
git commit -m "feat: add src-tauri crate with command scaffolding"
```

---

### 任务 3: 实现 Tauri 命令 — browse.rs

**文件：**
- 创建：`src-tauri/src/commands/browse.rs`

- [ ] **步骤 1: 创建 browse.rs**

```rust
use std::path::PathBuf;
use tauri::State;
use photo_tool_core::domain::{CaptureMeta, Capture};
use photo_tool_core::scanner;
use photo_tool_core::config::AppConfig;

/// 目录树节点（序列化版）
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct TreeNode {
    pub path: String,
    pub name: String,
    pub is_favorite: bool,
    pub has_children: bool,
    pub children: Vec<TreeNode>,
}

/// 打开目录扫描结果
#[derive(serde::Serialize)]
pub struct OpenDirectoryResult {
    pub captures: Vec<CaptureMeta>,
    pub tree: Vec<TreeNode>,
    pub total_count: usize,
}

#[tauri::command]
pub fn open_directory(path: String, sidecar_extensions: Vec<String>) -> Result<OpenDirectoryResult, String> {
    let dir = PathBuf::from(&path);
    let config = AppConfig {
        sidecar_extensions,
        ..Default::default()
    };

    let captures = scanner::scan_directory(&dir, &config.sidecar_extensions, &Default::default())
        .map_err(|e| e.to_string())?;

    let metas: Vec<CaptureMeta> = captures.iter().map(|c| CaptureMeta::from(c)).collect();
    let total = metas.len();
    let tree = build_tree(dir.parent().unwrap_or(&dir));

    Ok(OpenDirectoryResult {
        captures: metas,
        tree,
        total_count: total,
    })
}

#[tauri::command]
pub fn get_directory_tree() -> Vec<TreeNode> {
    let mut roots = Vec::new();

    // ~/Pictures
    if let Some(home) = dirs::home_dir() {
        let pics = home.join("Pictures");
        if pics.exists() {
            let node = build_single_node(&pics, false);
            if let Some(n) = node { roots.push(n); }
        }
    }

    // /media, /mnt
    for base in &["/media", "/mnt"] {
        let base_path = PathBuf::from(base);
        if base_path.exists() {
            if let Ok(entries) = std::fs::read_dir(base_path) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        roots.push(TreeNode {
                            path: path.to_string_lossy().to_string(),
                            name: path.file_name().unwrap_or_default().to_string_lossy().to_string(),
                            is_favorite: false,
                            has_children: has_subdirs(&path),
                            children: Vec::new(),
                        });
                    }
                }
            }
        }
    }

    roots
}

#[tauri::command]
pub fn expand_directory(path: String) -> Result<Vec<TreeNode>, String> {
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
    if let Some(parent) = active_dir.parent() {
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
    }
    roots
}

fn build_single_node(path: &PathBuf, is_favorite: bool) -> Option<TreeNode> {
    if !path.exists() { return None; }
    Some(TreeNode {
        path: path.to_string_lossy().to_string(),
        name: path.file_name().unwrap_or_default().to_string_lossy().to_string(),
        is_favorite,
        has_children: has_subdirs(path),
        children: Vec::new(),
    })
}

fn has_subdirs(path: &PathBuf) -> bool {
    if let Ok(entries) = std::fs::read_dir(path) {
        entries.flatten().any(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
    } else {
        false
    }
}
```

- [ ] **步骤 2: 确认编译通过**

```bash
cargo build -p photo-tool-tauri
```

- [ ] **步骤 3: Commit**

```bash
git add -A
git commit -m "feat: add browse commands with directory tree and capture scanning"
```

---

### 任务 4: 实现 Tauri 命令 — thumbnails.rs, exif_cmd.rs, xmp.rs

**文件：**
- 创建：`src-tauri/src/commands/thumbnails.rs`
- 创建：`src-tauri/src/commands/exif_cmd.rs`
- 修改：`src-tauri/src/commands/xmp.rs`

- [ ] **步骤 1: 创建 thumbnails.rs**

```rust
use std::path::PathBuf;
use photo_tool_core::domain::{ImageFormat, SourceFile};
use photo_tool_core::thumbnail::ThumbnailCache;

#[tauri::command]
pub fn get_thumbnail(path: String, size: u32) -> Result<Vec<u8>, String> {
    let cache_dir = get_cache_dir();
    let cache = ThumbnailCache::new(cache_dir);

    let path_buf = PathBuf::from(&path);
    let ext = path_buf.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let format = ImageFormat::from_extension(&ext).unwrap_or(ImageFormat::Jpeg);
    let source = SourceFile {
        path: path_buf,
        format,
        is_sidecar: false,
    };

    cache.get_or_generate(&source, size)
        .map_err(|e| e.to_string())
}

fn get_cache_dir() -> PathBuf {
    let mut dir = dirs::cache_dir().unwrap_or_else(|| PathBuf::from(".cache"));
    dir.push("PT");
    dir.push("thumbnails");
    dir
}

#[tauri::command]
pub fn clear_cache() -> Result<(), String> {
    let cache = ThumbnailCache::new(get_cache_dir());
    cache.clear().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_cache_stats() -> Result<(usize, u64), String> {
    let cache = ThumbnailCache::new(get_cache_dir());
    cache.stats().map_err(|e| e.to_string())
}
```

- [ ] **步骤 2: 创建 exif_cmd.rs**

```rust
use std::path::PathBuf;
use photo_tool_core::domain::ImageFormat;
use photo_tool_core::exif;

#[tauri::command]
pub fn get_exif(path: String) -> Result<exif::ExifMetadata, String> {
    let path_buf = PathBuf::from(&path);
    let ext = path_buf.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let format = ImageFormat::from_extension(&ext).unwrap_or(ImageFormat::Jpeg);
    exif::extract_exif(&path_buf, &format).map_err(|e| e.to_string())
}
```

- [ ] **步骤 3: 创建 src-tauri/src/commands/xmp.rs**

```rust
use std::path::PathBuf;
use photo_tool_core::xmp::{self, XmpMetadata};

#[tauri::command]
pub fn read_capture_xmp(primary_path: String) -> Result<XmpMetadata, String> {
    let path = PathBuf::from(&primary_path);
    let xp = xmp::xmp_path(&path);
    xmp::read_xmp(&xp).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn write_capture_xmp(primary_path: String, metadata: XmpMetadata) -> Result<(), String> {
    let path = PathBuf::from(&primary_path);
    let xp = xmp::xmp_path(&path);
    xmp::write_xmp(&xp, &metadata).map_err(|e| e.to_string())
}
```

- [ ] **步骤 4: 确认编译通过**

```bash
cargo build -p photo-tool-tauri
```

- [ ] **步骤 5: Commit**

```bash
git add -A
git commit -m "feat: add thumbnail, exif, and xmp commands"
```

---

### 任务 5: 实现 Tauri 命令 — files.rs, config.rs, convert_cmd.rs, import_cmd.rs

**文件：**
- 创建：`src-tauri/src/commands/files.rs`
- 创建：`src-tauri/src/commands/config.rs`
- 创建：`src-tauri/src/commands/convert_cmd.rs`
- 创建：`src-tauri/src/commands/import_cmd.rs`

> (命令实现作为练习——直接从现有 photo-tool-core 函数封装，参考 thumbnails.rs 模式)

---

### 任务 6: Vue 前端项目初始化

**文件：**
- 创建：`package.json`
- 创建：`vite.config.ts`
- 创建：`tsconfig.json`
- 创建：`tsconfig.node.json`
- 创建：`index.html`
- 创建：`src/main.ts`
- 创建：`src/env.d.ts`
- 创建：`src/styles/variables.css`
- 创建：`src/styles/global.css`

- [ ] **步骤 1: 创建 package.json**

```json
{
  "name": "photo-tool",
  "private": true,
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "vue-tsc --noEmit && vite build",
    "preview": "vite preview",
    "tauri": "tauri"
  },
  "dependencies": {
    "vue": "^3.5",
    "pinia": "^2.2",
    "vue-router": "^4.4",
    "@tauri-apps/api": "^2.0",
    "@tauri-apps/plugin-dialog": "^2.0",
    "@tauri-apps/plugin-fs": "^2.0",
    "reka-ui": "^1.0"
  },
  "devDependencies": {
    "@tauri-apps/cli": "^2.0",
    "@vitejs/plugin-vue": "^5.0",
    "typescript": "^5.5",
    "vue-tsc": "^2.0",
    "vite": "^6.0"
  }
}
```

- [ ] **步骤 2: 创建 vite.config.ts**

```typescript
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

export default defineConfig({
  plugins: [vue()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  envPrefix: ['VITE_', 'TAURI_'],
  build: {
    target: process.env.TAURI_PLATFORM === 'windows' ? 'chrome105' : 'safari14',
    minify: !process.env.TAURI_DEBUG ? 'esbuild' : false,
    sourcemap: !!process.env.TAURI_DEBUG,
  },
})
```

- [ ] **步骤 3: 创建 tsconfig.json**

```json
{
  "compilerOptions": {
    "target": "ES2021",
    "useDefineForClassFields": true,
    "module": "ESNext",
    "lib": ["ES2021", "DOM", "DOM.Iterable"],
    "skipLibCheck": true,
    "moduleResolution": "bundler",
    "allowImportingTsExtensions": true,
    "resolveJsonModule": true,
    "isolatedModules": true,
    "noEmit": true,
    "jsx": "preserve",
    "strict": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "noFallthroughCasesInSwitch": true,
    "paths": {
      "@/*": ["./src/*"]
    }
  },
  "include": ["src/**/*.ts", "src/**/*.tsx", "src/**/*.vue"],
  "references": [{ "path": "./tsconfig.node.json" }]
}
```

- [ ] **步骤 4: 创建 tsconfig.node.json**

```json
{
  "compilerOptions": {
    "composite": true,
    "skipLibCheck": true,
    "module": "ESNext",
    "moduleResolution": "bundler",
    "allowSyntheticDefaultImports": true
  },
  "include": ["vite.config.ts"]
}
```

- [ ] **步骤 5: 创建 index.html**

```html
<!DOCTYPE html>
<html lang="zh-CN">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>PT - Photo Tool</title>
  </head>
  <body>
    <div id="app"></div>
    <script type="module" src="/src/main.ts"></script>
  </body>
</html>
```

- [ ] **步骤 6: 创建 src/main.ts**

```typescript
import { createApp } from 'vue'
import { createPinia } from 'pinia'
import App from './App.vue'
import router from './router'
import './styles/global.css'

const app = createApp(App)
app.use(createPinia())
app.use(router)
app.mount('#app')
```

- [ ] **步骤 7: 创建 src/env.d.ts**

```typescript
/// <reference types="vite/client" />

declare module '*.vue' {
  import type { DefineComponent } from 'vue'
  const component: DefineComponent<{}, {}, any>
  export default component
}
```

- [ ] **步骤 8: 创建 src/styles/variables.css**

```css
:root {
  /* Background */
  --bg-page: #f1f2f4;
  --bg-surface: #ffffff;
  --bg-hover: #f6f7f9;

  /* Border */
  --border: #e1e2e5;
  --border-focus: #618cf2;

  /* Text */
  --text: #17171c;
  --text-muted: #6b7082;

  /* Brand */
  --accent: #618cf2;
  --accent-hover: #4d7af0;
  --accent-subtle: #e6edfd;

  /* Semantic */
  --danger: #e84d3d;
  --danger-hover: #d13d2e;
  --success: #2ea44f;

  /* Selection */
  --selection: rgba(97, 140, 242, 0.18);
  --selection-border: #618cf2;

  /* Special */
  --star: #f5b812;
  --flag-pick: #2ea44f;
  --flag-reject: #e84d3d;

  /* Layout */
  --left-panel-width: 260px;
  --right-panel-width: 380px;
  --toolbar-height: 44px;
  --statusbar-height: 32px;
}
```

- [ ] **步骤 9: 创建 src/styles/global.css**

```css
@import './variables.css';

* {
  margin: 0;
  padding: 0;
  box-sizing: border-box;
}

html, body, #app {
  height: 100%;
  width: 100%;
  overflow: hidden;
  font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, 'Helvetica Neue', Arial, sans-serif;
  font-size: 14px;
  color: var(--text);
  background: var(--bg-page);
  -webkit-font-smoothing: antialiased;
}

::-webkit-scrollbar {
  width: 8px;
  height: 8px;
}

::-webkit-scrollbar-track {
  background: transparent;
}

::-webkit-scrollbar-thumb {
  background: var(--border);
  border-radius: 4px;
}

::-webkit-scrollbar-thumb:hover {
  background: var(--text-muted);
}
```

- [ ] **步骤 10: 安装前端依赖**

```bash
pnpm install
```

- [ ] **步骤 11: 确认 Vite dev 启动没问题**

```bash
pnpm dev
# 预期：Vite 在 http://localhost:1420 启动（此时页面空白）
```

- [ ] **步骤 12: Commit**

```bash
git add -A
git commit -m "feat: initialize Vue frontend project"
```

---

### 任务 7: 前端类型定义与 Tauri invoke 封装

**文件：**
- 创建：`src/types/index.ts`
- 创建：`src/types/tauri.ts`

- [ ] **步骤 1: 创建 src/types/index.ts**

```typescript
export interface CaptureMeta {
  baseName: string
  primaryPath: string
  primaryFormat: string
  stackCount: number
  fileSize: number | null
  dateTaken: string | null
  hasXmp: boolean
  extensions: string[]
}

export interface TreeNode {
  path: string
  name: string
  isFavorite: boolean
  hasChildren: boolean
  children: TreeNode[]
}

export interface OpenDirectoryResult {
  captures: CaptureMeta[]
  tree: TreeNode[]
  totalCount: number
}

export interface ExifMetadata {
  camera: { make: string | null; model: string | null; lens: string | null }
  shooting: {
    exposureTime: string | null
    fNumber: string | null
    iso: number | null
    focalLength: string | null
    exposureCompensation: string | null
    whiteBalance: string | null
  }
  gps: {
    latitude: [number, number, number] | null
    longitude: [number, number, number] | null
    altitude: number | null
  }
  dateTimeOriginal: string | null
  imageWidth: number | null
  imageHeight: number | null
  fileSize: number | null
  colorSpace: string | null
  orientation: number | null
}

export interface XmpMetadata {
  rating: number
  colorLabel: string
  flag: string
}

export interface AppConfig {
  sidecarExtensions: string[]
  thumbnailSize: number
  favoriteDirs: string[]
  lastDirectory: string | null
  defaultDeleteMode: string
  windowWidth: number
  windowHeight: number
  leftPanelWidth: number
  rightPanelVisible: boolean
  maxCacheSizeMb: number
}

export interface CacheStats {
  count: number
  totalSize: number
}

export type SortBy = 'FileName' | 'DateTaken' | 'FileSize'
export type SortDirection = 'Ascending' | 'Descending'
export type DeleteMode = 'Trash' | 'Permanent'
```

- [ ] **步骤 2: 创建 src/types/tauri.ts**

```typescript
import { invoke } from '@tauri-apps/api/core'
import type {
  CaptureMeta, TreeNode, OpenDirectoryResult,
  ExifMetadata, XmpMetadata, AppConfig, CacheStats
} from './index'

// ── Browse ──
export const openDirectory = (path: string, sidecarExtensions: string[]) =>
  invoke<OpenDirectoryResult>('open_directory', { path, sidecarExtensions })

export const getDirectoryTree = () =>
  invoke<TreeNode[]>('get_directory_tree')

export const expandDirectory = (path: string) =>
  invoke<TreeNode[]>('expand_directory', { path })

// ── Thumbnails ──
export const getThumbnail = (path: string, size: number) =>
  invoke<number[]>('get_thumbnail', { path, size })

export const clearCache = () =>
  invoke<void>('clear_cache')

export const getCacheStats = () =>
  invoke<[number, number]>('get_cache_stats')

// ── EXIF ──
export const getExif = (path: string) =>
  invoke<ExifMetadata>('get_exif', { path })

// ── XMP ──
export const readCaptureXmp = (primaryPath: string) =>
  invoke<XmpMetadata>('read_capture_xmp', { primaryPath })

export const writeCaptureXmp = (primaryPath: string, metadata: XmpMetadata) =>
  invoke<void>('write_capture_xmp', { primaryPath, metadata })

// ── Files ──
export const deleteCaptures = (capturePaths: string[], mode: string) =>
  invoke<void>('delete_captures', { capturePaths, mode })

export const moveCaptures = (capturePaths: string[], dest: string) =>
  invoke<void>('move_captures', { capturePaths, dest })

export const copyCaptures = (capturePaths: string[], dest: string) =>
  invoke<void>('copy_captures', { capturePaths, dest })

export const renameCaptures = (items: Array<{ oldPath: string; newName: string }>) =>
  invoke<void>('rename_captures', { items })

// ── Config ──
export const loadConfig = () =>
  invoke<AppConfig>('load_config')

export const saveConfig = (config: AppConfig) =>
  invoke<void>('save_config', { config })

// ── Convert ──
export const convertImages = (paths: string[], options: {
  outputDir: string
  outputFormat: string
  jpegQuality: number
  maxDimension: number
}) => invoke<void>('convert_images', { paths, options })

// ── Import ──
export const detectDrives = () =>
  invoke<string[]>('detect_drives')

export const importCaptures = (paths: string[], options: {
  destRoot: string
  behavior: string
  dateFormat: string
  overwriteStrategy: string
}) => invoke<void>('import_captures', { paths, options })
```

- [ ] **步骤 3: Commit**

```bash
git add -A
git commit -m "feat: add TypeScript types and Tauri invoke wrappers"
```

---

### 任务 8: Pinia stores

**文件：**
- 创建：`src/stores/browse.ts`
- 创建：`src/stores/config.ts`
- 创建：`src/stores/ui.ts`

- [ ] **步骤 1: 创建 src/stores/browse.ts**

```typescript
import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import type { CaptureMeta, TreeNode, SortBy, SortDirection } from '@/types'
import { openDirectory as invokeOpenDir } from '@/types/tauri'

export const useBrowseStore = defineStore('browse', () => {
  // 核心数据
  const captures = ref<CaptureMeta[]>([])
  const filteredIndices = ref<number[]>([])
  const directoryTree = ref<TreeNode[]>([])
  const currentPath = ref<string>('')

  // 选中状态
  const selectedIndices = ref<Set<number>>(new Set())
  const focusedIndex = ref<number | null>(null)

  // 排序/筛选
  const sortBy = ref<SortBy>('FileName')
  const sortDirection = ref<SortDirection>('Ascending')
  const searchText = ref('')

  // 预览
  const zoomLevel = ref(1.0)
  const fitToWindow = ref(true)

  // 计算属性
  const totalCount = computed(() => captures.value.length)
  const filteredCount = computed(() => filteredIndices.value.length)
  const selectedCount = computed(() => selectedIndices.value.size)

  const filteredCaptures = computed(() =>
    filteredIndices.value.map(i => captures.value[i])
  )

  // 选中项对应的CaptureMeta列表
  const selectedCaptures = computed(() =>
    Array.from(selectedIndices.value).map(i => captures.value[i])
  )

  // 操作
  async function openDirectory(path: string) {
    const result = await invokeOpenDir(path, ['xmp'])
    captures.value = result.captures
    directoryTree.value = result.tree
    filteredIndices.value = result.captures.map((_, i) => i)
    currentPath.value = path
    selectedIndices.value = new Set()
    focusedIndex.value = null
    applyFilters()
  }

  function selectCapture(idx: number) {
    selectedIndices.value = new Set([idx])
  }

  function toggleSelect(idx: number) {
    const s = new Set(selectedIndices.value)
    if (s.has(idx)) s.delete(idx)
    else s.add(idx)
    selectedIndices.value = s
  }

  function selectRange(idx: number) {
    if (selectedIndices.value.size === 0) {
      selectedIndices.value = new Set([idx])
      return
    }
    const sorted = Array.from(selectedIndices.value).sort((a, b) => a - b)
    const last = sorted[sorted.length - 1]
    const start = Math.min(last, idx)
    const end = Math.max(last, idx)
    const range = new Set<number>()
    for (let i = start; i <= end; i++) range.add(i)
    selectedIndices.value = range
  }

  function selectAll() {
    selectedIndices.value = new Set(filteredIndices.value)
  }

  function invertSelection() {
    const all = new Set(filteredIndices.value)
    const s = new Set(selectedIndices.value)
    selectedIndices.value = new Set(Array.from(all).filter(i => !s.has(i)))
  }

  function clearSelection() {
    selectedIndices.value = new Set()
    focusedIndex.value = null
  }

  function setSort(by: SortBy, dir: SortDirection) {
    sortBy.value = by
    sortDirection.value = dir
    applyFilters()
  }

  function setSearch(text: string) {
    searchText.value = text
    applyFilters()
  }

  function applyFilters() {
    let indices = captures.value.map((_, i) => i)

    // 文本搜索
    if (searchText.value) {
      const q = searchText.value.toLowerCase()
      indices = indices.filter(i =>
        captures.value[i].baseName.toLowerCase().includes(q)
      )
    }

    // 排序
    indices.sort((a, b) => {
      const ca = captures.value[a]
      const cb = captures.value[b]
      let cmp = 0
      switch (sortBy.value) {
        case 'FileName':
          cmp = ca.baseName.localeCompare(cb.baseName)
          break
        case 'FileSize':
          cmp = (ca.fileSize ?? 0) - (cb.fileSize ?? 0)
          break
        case 'DateTaken':
          cmp = (ca.dateTaken ?? '').localeCompare(cb.dateTaken ?? '')
          break
      }
      return sortDirection.value === 'Ascending' ? cmp : -cmp
    })

    filteredIndices.value = indices
  }

  function focusNext() {
    if (filteredIndices.value.length === 0) return
    const next = (focusedIndex.value ?? -1) + 1
    focusedIndex.value = Math.min(next, filteredIndices.value.length - 1)
  }

  function focusPrev() {
    if (filteredIndices.value.length === 0) return
    const prev = (focusedIndex.value ?? filteredIndices.value.length) - 1
    focusedIndex.value = Math.max(prev, 0)
  }

  function setZoom(delta: number) {
    zoomLevel.value = Math.max(0.25, Math.min(5.0, zoomLevel.value + delta))
    if (Math.abs(zoomLevel.value - 1.0) < 0.01) {
      fitToWindow.value = true
    } else {
      fitToWindow.value = false
    }
  }

  function toggleFitToWindow() {
    fitToWindow.value = !fitToWindow.value
    if (fitToWindow.value) zoomLevel.value = 1.0
  }

  return {
    captures, filteredIndices, directoryTree, currentPath,
    selectedIndices, focusedIndex,
    sortBy, sortDirection, searchText,
    zoomLevel, fitToWindow,
    totalCount, filteredCount, selectedCount, filteredCaptures, selectedCaptures,
    openDirectory, selectCapture, toggleSelect, selectRange,
    selectAll, invertSelection, clearSelection,
    setSort, setSearch, applyFilters,
    focusNext, focusPrev, setZoom, toggleFitToWindow,
  }
})
```

- [ ] **步骤 2: 创建 src/stores/config.ts** (轻)

```typescript
import { defineStore } from 'pinia'
import { ref } from 'vue'
import type { AppConfig } from '@/types'

export const useConfigStore = defineStore('config', () => {
  const config = ref<AppConfig>({
    sidecarExtensions: ['xmp'],
    thumbnailSize: 220,
    favoriteDirs: [],
    lastDirectory: null,
    defaultDeleteMode: 'trash',
    windowWidth: 1400,
    windowHeight: 900,
    leftPanelWidth: 260,
    rightPanelVisible: true,
    maxCacheSizeMb: 500,
  })

  async function load() {
    // Will call Tauri command
    const { loadConfig: loadCfg } = await import('@/types/tauri')
    try { config.value = await loadCfg() } catch {}
  }

  async function save() {
    const { saveConfig: saveCfg } = await import('@/types/tauri')
    try { await saveCfg(config.value) } catch {}
  }

  return { config, load, save }
})
```

- [ ] **步骤 3: 创建 src/stores/ui.ts**

```typescript
import { defineStore } from 'pinia'
import { ref } from 'vue'

export type AppMode = 'browse' | 'compare' | 'import' | 'rename' | 'settings' | 'convert'

export const useUiStore = defineStore('ui', () => {
  const mode = ref<AppMode>('browse')
  const rightPanelVisible = ref(true)
  const leftPanelWidth = ref(260)

  // 上下文菜单
  const contextMenu = ref<{ index: number; x: number; y: number } | null>(null)

  // 对比模式
  const compareIndices = ref<[number, number]>([0, 0])

  // 对话框状态
  const importOpen = ref(false)
  const renameOpen = ref(false)
  const settingsOpen = ref(false)
  const convertOpen = ref(false)

  function openContextMenu(index: number, x: number, y: number) {
    contextMenu.value = { index, x, y }
  }

  function closeContextMenu() {
    contextMenu.value = null
  }

  function enterCompare(left: number, right: number) {
    compareIndices.value = [left, right]
    mode.value = 'compare'
  }

  function exitCompare() {
    mode.value = 'browse'
  }

  function toggleRightPanel() {
    rightPanelVisible.value = !rightPanelVisible.value
  }

  return {
    mode, rightPanelVisible, leftPanelWidth,
    contextMenu, compareIndices,
    importOpen, renameOpen, settingsOpen, convertOpen,
    openContextMenu, closeContextMenu,
    enterCompare, exitCompare, toggleRightPanel,
  }
})
```

- [ ] **步骤 4: Commit**

```bash
git add -A
git commit -m "feat: add Pinia stores (browse, config, ui)"
```

---

### 任务 9: Vue Router 与 App.vue

**文件：**
- 创建：`src/router/index.ts`
- 创建：`src/App.vue`

- [ ] **步骤 1: 创建 src/router/index.ts**

```typescript
import { createRouter, createWebHashHistory } from 'vue-router'
import BrowseView from '@/views/BrowseView.vue'
import CompareView from '@/views/CompareView.vue'

const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: '/', redirect: '/browse' },
    { path: '/browse', name: 'browse', component: BrowseView },
    { path: '/compare/:left/:right', name: 'compare', component: CompareView, props: true },
  ],
})

export default router
```

- [ ] **步骤 2: 创建 src/App.vue**

```vue
<script setup lang="ts">
import { RouterView } from 'vue-router'
</script>

<template>
  <RouterView />
</template>
```

- [ ] **步骤 3: Commit**

```bash
git add -A
git commit -m "feat: add vue-router and App shell"
```

---

### 任务 10: Layout.vue — 三栏布局

**文件：**
- 创建：`src/components/Layout.vue`

```vue
<script setup lang="ts">
import { useUiStore } from '@/stores/ui'

const ui = useUiStore()
</script>

<template>
  <div class="layout">
    <div class="layout__left" :style="{ width: ui.leftPanelWidth + 'px' }">
      <slot name="left" />
    </div>
    <div class="layout__center">
      <slot name="center" />
    </div>
    <div v-if="ui.rightPanelVisible" class="layout__right">
      <slot name="right" />
    </div>
  </div>
</template>

<style scoped>
.layout {
  display: flex;
  height: 100%;
  width: 100%;
  overflow: hidden;
}
.layout__left {
  flex-shrink: 0;
  border-right: 1px solid var(--border);
  background: var(--bg-surface);
  overflow-y: auto;
}
.layout__center {
  flex: 1;
  overflow: hidden;
  display: flex;
  flex-direction: column;
}
.layout__right {
  width: var(--right-panel-width);
  flex-shrink: 0;
  border-left: 1px solid var(--border);
  background: var(--bg-surface);
  overflow-y: auto;
}
</style>
```

- [ ] **Commit**

---

### 任务 11: StatusBar.vue

```vue
<script setup lang="ts">
import { computed } from 'vue'
import { useBrowseStore } from '@/stores/browse'

const browse = useBrowseStore()

const statusText = computed(() => {
  const parts: string[] = []
  if (browse.totalCount > 0) {
    if (browse.filteredCount < browse.totalCount) {
      parts.push(`共 ${browse.totalCount} 张，筛选后 ${browse.filteredCount} 张`)
    } else {
      parts.push(`共 ${browse.totalCount} 张拍摄`)
    }
  }
  return parts.join('  |  ') || '就绪'
})
</script>

<template>
  <div class="statusbar">{{ statusText }}</div>
</template>

<style scoped>
.statusbar {
  height: var(--statusbar-height);
  display: flex;
  align-items: center;
  padding: 0 12px;
  font-size: 13px;
  color: var(--text-muted);
  background: var(--bg-surface);
  border-top: 1px solid var(--border);
  flex-shrink: 0;
}
</style>
```

---

### 任务 12-18: Vue 组件实现

后续组件按优先级依次实现：

- 任务 12: `DirectoryTree.vue` — 目录树
- 任务 13: `Toolbar.vue` — 排序/筛选/操作工具栏
- 任务 14: `ThumbnailGrid.vue` + `ThumbnailCell.vue` — 缩略图网格
- 任务 15: `PreviewPanel.vue` + `ExifTable.vue` — 预览面板
- 任务 16: `ContextMenu.vue` — 右键菜单
- 任务 17: `CompareView.vue` — 对比视图
- 任务 18: 对话框组件 (Import, Rename, Settings, Convert)

---

### 任务 19: BrowseView.vue — 主浏览视图

将所有组件组合成主视图。

```vue
<script setup lang="ts">
import Layout from '@/components/Layout.vue'
import StatusBar from '@/components/StatusBar.vue'
import DirectoryTree from '@/components/DirectoryTree.vue'
import Toolbar from '@/components/Toolbar.vue'
import ThumbnailGrid from '@/components/ThumbnailGrid.vue'
import PreviewPanel from '@/components/PreviewPanel.vue'
import ContextMenu from '@/components/ContextMenu.vue'
import { useKeyboard } from '@/composables/useKeyboard'

useKeyboard()
</script>

<template>
  <div class="browse-view">
    <Layout>
      <template #left>
        <DirectoryTree />
      </template>
      <template #center>
        <Toolbar />
        <ThumbnailGrid />
      </template>
      <template #right>
        <PreviewPanel />
      </template>
    </Layout>
    <StatusBar />
    <ContextMenu />
  </div>
</template>

<style scoped>
.browse-view {
  display: flex;
  flex-direction: column;
  height: 100%;
}
</style>
```

---

### 任务 20: 集成测试与最终调优

- [ ] `pnpm tauri dev` 启动整个应用
- [ ] 验证 app 窗口显示
- [ ] 验证目录树加载
- [ ] 验证打开目录 → 扫描 captures → 网格渲染
- [ ] 验证选取、筛选、排序
- [ ] 验证预览面板（图片+EXIF）
- [ ] 验证右键菜单
- [ ] 验证对话框
- [ ] 验证对比模式
- [ ] 验证键盘导航
