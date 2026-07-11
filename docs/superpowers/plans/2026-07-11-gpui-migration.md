# Photo Tool GPUI 迁移实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。步骤使用复选框（`- [ ]`）语法来跟踪进度。

**目标：** 将 Photo Tool 从 Tauri + Vue 3 全量迁移到 GPUI + gpui-component，实现零 IPC 的原生 GPU 渲染。

**架构：** 新建 `photo-tool-gpui` crate（加入 workspace），保留 `photo-tool-core` 不动。后台线程通过通道与 GPUI 主循环通信。纹理按需从磁盘缓存加载。

**技术栈：** GPUI (git)、gpui-component (git)、rfd 0.17、photo-tool-core（本地 workspace 依赖）

**正确依赖（来自官方教程）：**

```toml
[dependencies]
gpui = { git = "https://github.com/zed-industries/zed" }
gpui_platform = { git = "https://github.com/zed-industries/zed", features = ["font-kit"] }
gpui-component = { git = "https://github.com/longbridge/gpui-component" }
gpui-component-assets = { git = "https://github.com/longbridge/gpui-component" }
rfd = "0.17"
photo-tool-core = { path = "../photo-tool-core" }
anyhow = "1.0"
```

---

```

---

## 文件结构

```

photo-tool-gpui/
├── Cargo.toml
└── src/
    ├── main.rs              # Application 入口、事件循环启动
    ├── state.rs             # 全局应用状态（目录、筛选、选中等）
    ├── theme.rs             # 主题定义（Light/Dark，颜色 token）
    ├── components/
    │   ├── mod.rs
    │   ├── sidebar.rs       # 目录树侧边栏（树形展开/折叠 + 常用目录）
    │   ├── toolbar.rs       # 顶部工具栏（排序、筛选、搜索）
    │   ├── grid.rs          # 缩略图网格（VirtualList + 纹理按需加载）
    │   ├── preview.rs       # 右侧预览面板（大图 GPU 纹理 + 缩放/拖拽）
    │   ├── status_bar.rs    # 底部状态栏
    │   ├── context_menu.rs  # 右键菜单（复制/删除/重命名/导出）
    │   └── dialogs.rs      # 模态对话框（导入/重命名/设置/转换）
    └── workers/
        ├── mod.rs
        ├── scanner.rs       # 后台扫描线程（通道 → 主线程）
        └── thumbnail.rs     # 纹理加载线程（磁盘缓存 → GPU 纹理）

```

---

## 任务分解

### 任务 1：搭建 workspace 和空窗口

**文件：**
- 创建：`photo-tool-gpui/Cargo.toml`
- 创建：`photo-tool-gpui/src/main.rs`
- 创建：`photo-tool-gpui/src/state.rs`
- 修改：`photo-tool/Cargo.toml`（workspace members）

- [ ] **步骤 1：创建 Cargo.toml**

    ```toml
    [package]
    name = "photo-tool-gpui"
    version.workspace = true
    edition.workspace = true

    [dependencies]
    gpui = "0.2"
    gpui-component = { git = "https://github.com/longbridge/gpui-component" }
    rfd = "0.17"
    photo-tool-core = { path = "../photo-tool-core" }
    ```

- [ ] **步骤 2：编写 main.rs——初始化空窗口**（API 来自官方教程）

    ```rust
    use gpui::*;
    use gpui_component::Root;

    struct AppState;

    impl Render for AppState {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            Root::new(
                div().size_full().child("Photo Tool"),
                _window,
                _cx,
            )
        }
    }

    fn main() {
        let app = gpui_platform::application()
            .with_assets(gpui_component_assets::Assets);

        app.run(move |cx| {
            gpui_component::init(cx);

            cx.spawn(async move |cx| {
                cx.open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(
                            Bounds::centered(None, size(px(1400.), px(900.)), &cx),
                        )),
                        ..Default::default()
                    },
                    |window, cx| {
                        let view = cx.new(|_cx| AppState);
                        cx.new(|cx| Root::new(view, window, cx))
                    },
                )
                .expect("Failed to open window");
            })
            .detach();
        });
    }
    ```

    关键点：
    - `gpui_platform::application().with_assets(...)` 而非 `Application::new()`
    - `cx.spawn(async {...}).detach()` 包裹 `open_window`
    - `Root::new(view, window, cx)` 接收三个参数
    - `gpui_component::init(cx)` 在 `app.run` 闭包中尽早调用

- [ ] **步骤 3：修改 workspace Cargo.toml**

    在 `members` 数组中添加 `"photo-tool-gpui"`。

- [ ] **步骤 4：编译验证**

    运行：`cargo build -p photo-tool-gpui`
    预期：编译通过，无错误

- [ ] **步骤 5：Commit**

---

### 任务 2：全局状态管理

**文件：**
- 覆盖：`photo-tool-gpui/src/state.rs`
- 修改：`photo-tool-gpui/src/main.rs`（注入 state）

- [ ] **步骤 1：定义 AppState 模型**

    ```rust
    use gpui::*;
    use photo_tool_core::domain::{CaptureMeta, TreeNode, SortBy, SortDirection};
    use std::path::PathBuf;

    pub struct AppState {
        // 数据
        pub captures: Vec<CaptureMeta>,
        pub filtered_indices: Vec<usize>,
        pub directory_tree: Vec<TreeNode>,
        pub current_path: Option<PathBuf>,

        // 选中
        pub selected_indices: Vec<usize>,
        pub focused_index: Option<usize>,

        // 排序/筛选
        pub sort_by: SortBy,
        pub sort_direction: SortDirection,
        pub search_text: String,

        // 加载状态
        pub is_scanning: bool,
        pub scan_progress: Option<ScanProgress>,

        // 预览
        pub zoom_level: f32,
        pub fit_to_window: bool,
    }

    #[derive(Clone)]
    pub struct ScanProgress {
        pub percent: u32,
        pub path: String,
        pub phase: String,
    }

    impl AppState {
        pub fn new() -> Self { /* defaults */ todo!() }
        pub fn filtered_captures(&self) -> Vec<&CaptureMeta> { todo!() }
        pub fn apply_filters(&mut self) { todo!() }
    }
    ```

- [ ] **步骤 2：修改 Render 实现，从 state 读取并渲染 Root**

- [ ] **步骤 3：编译验证**

- [ ] **步骤 4：Commit**

---

### 任务 3：三栏 Resizable 布局

**文件：**
- 修改：`photo-tool-gpui/src/main.rs`（Render 改布局）
- 创建：`photo-tool-gpui/src/components/mod.rs`
- 使用：`gpui_component::Resizable`

- [ ] **步骤 1：在 Render 中使用 Resizable 实现左右分栏**

    ```rust
    // 伪代码——gpui-component 的 Resizable API
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        Root::new("app")
            .child(
                Resizable::horizontal()
                    .group("sidebar", px(260.), px(180.)..px(400.))
                    .child(/* Sidebar placeholder */)
                    .group("content")
                    .child(
                        div().flex_col()
                            .child(/* Toolbar */)
                            .child(/* Grid */)
                    )
                    .group("preview", px(320.), px(240.)..px(500.))
                    .child(/* Preview */)
            )
    }
    ```

- [ ] **步骤 2：添加占位文本到三个区域**（Sidebar / Grid / Preview）

- [ ] **步骤 3：编译验证布局正确渲染**

- [ ] **步骤 4：Commit**

---

### 任务 4：后台扫描 + 通道通信

**文件：**
- 创建：`photo-tool-gpui/src/workers/mod.rs`
- 创建：`photo-tool-gpui/src/workers/scanner.rs`

这是迁移中最关键的架构模块——将 `scan_directory` 放到后台线程执行，通过 `std::sync::mpsc::channel` 向主线程推送进度和结果。

- [ ] **步骤 1：定义扫描消息类型**

    ```rust
    pub enum ScanEvent {
        Progress { percent: u32, path: String, phase: String },
        Complete { captures: Vec<CaptureMeta>, tree: Vec<TreeNode> },
        Error(String),
    }
    ```

- [ ] **步骤 2：编写后台扫描函数**

    ```rust
    pub fn start_scan(
        path: PathBuf,
        sender: std::sync::mpsc::Sender<ScanEvent>,
    ) {
        std::thread::spawn(move || {
            let sidecar_extensions = vec!["xmp".to_string()];
            let report_sender = sender.clone();
            let report_path = path.to_string_lossy().to_string();

            let on_progress: Box<dyn Fn(u32) + Send> = Box::new(move |pct| {
                let _ = report_sender.send(ScanEvent::Progress {
                    percent: pct,
                    path: report_path.clone(),
                    phase: "scanning".into(),
                });
            });

            match photo_tool_core::scanner::scan_directory(
                &path, &sidecar_extensions, &Default::default(), Some(on_progress),
            ) {
                Ok(captures) => {
                    let metas: Vec<CaptureMeta> = captures.iter()
                        .map(|c| c.into()).collect();
                    let total = metas.len();
                    // build_tree... (简化为空 vec 或复用 browse.rs 里的逻辑)
                    let _ = sender.send(ScanEvent::Complete {
                        captures: metas, tree: vec![],
                    });
                }
                Err(e) => {
                    let _ = sender.send(ScanEvent::Error(e.to_string()));
                }
            }
        });
    }
    ```

- [ ] **步骤 3：在 AppState 中集成通道接收**

    在 `main.rs` 的 `cx.spawn()` 或通过 GPUI 的 `cx.on_app_quit` 方式轮询通道：

    ```rust
    // 在主循环中处理事件
    fn poll_scan_events(&mut self, rx: &std::sync::mpsc::Receiver<ScanEvent>) {
        while let Ok(event) = rx.try_recv() {
            match event {
                ScanEvent::Progress { percent, path, phase } => {
                    self.scan_progress = Some(ScanProgress { percent, path, phase });
                }
                ScanEvent::Complete { captures, tree } => {
                    self.captures = captures;
                    self.directory_tree = tree;
                    self.apply_filters();
                    self.is_scanning = false;
                    self.scan_progress = None;
                }
                ScanEvent::Error(e) => {
                    // show error dialog
                    self.is_scanning = false;
                }
            }
        }
    }
    ```

- [ ] **步骤 4：在 Sidebar 组件中调用扫描**

    目录树双击 → `start_scan(path, sender)` → 主循环轮询通道更新 UI。

- [ ] **步骤 5：编译验证**

- [ ] **步骤 6：Commit**

---

### 任务 5：目录树侧边栏

**文件：**
- 创建：`photo-tool-gpui/src/components/sidebar.rs`

- [ ] **步骤 1：实现树形组件**（使用 gpui-component 的 Tree 或手写递归 `div`）

- [ ] **步骤 2：支持展开/折叠**

- [ ] **步骤 3：双击打开目录 → 触发扫描**

- [ ] **步骤 4：调用 `rfd::FileDialog::new().set_directory(...).pick_folder()` 支持系统对话框**

- [ ] **步骤 5：编译验证**

- [ ] **步骤 6：Commit**

---

### 任务 6：缩略图纹理管理器

**文件：**
- 创建：`photo-tool-gpui/src/workers/thumbnail.rs`

- [ ] **步骤 1：TextureManager 结构**

    ```rust
    use std::collections::HashMap;
    use gpui::*;
    use photo_tool_core::thumbnail::ThumbnailCache;

    pub struct TextureManager {
        cache: ThumbnailCache,
        textures: HashMap<String, AnyTexture>,
        max_textures: usize,
    }

    impl TextureManager {
        pub fn new(cache_dir: PathBuf) -> Self { todo!() }
        pub fn get_or_load(&mut self, path: &str, size: u32, cx: &mut Context<AppState>) -> Option<&AnyTexture> {
            // 1. Check HashMap cache
            // 2. Call ThumbnailCache::get_or_generate()
            // 3. Decode JPEG bytes → gpui::Image
            // 4. Store in HashMap
            // 5. Evict LRU if over max
            // 6. Return reference
            todo!()
        }
    }
    ```

- [ ] **步骤 2：JPEG → GPU 纹理转换**

    ```rust
    // gpui 的 img() 直接接受字节，会自动 decode
    // 或者手动用 image crate decode → RgbaImage → gpui::ImageData
    fn jpeg_to_texture(bytes: &[u8], cx: &mut Context<AppState>) -> AnyTexture {
        let img = image::load_from_memory(bytes).unwrap();
        let rgba = img.to_rgba8();
        let (w, h) = rgba.dimensions();
        let data = rgba.into_raw();
        cx.new_texture(
            size(w, h),
            TextureData::from_raw(data, w, h, TextureFormat::Rgba8),
        )
    }
    ```

- [ ] **步骤 3：编译验证**（注意 GPUI 纹理 API 的实际签名）

- [ ] **步骤 4：Commit**

---

### 任务 7：缩略图网格

**文件：**
- 创建：`photo-tool-gpui/src/components/grid.rs`

- [ ] **步骤 1：使用 gpui-component 的 VirtualList 渲染缩略图**

    ```rust
    VirtualList::new()
        .items(self.state.filtered_indices.len())
        .item_size(px(220.), px(220.))  // 缩略图大小
        .render_item(|idx, cx| {
            let capture = &self.state.captures[idx];
            let texture = self.textures.get_or_load(
                &capture.primary_path, self.thumbnail_size, cx
            );
            div()
                .size(px(220.), px(220.))
                .child(img(texture))
                .child(div().child(capture.base_name.clone()))  // 文件名
        })
    ```

- [ ] **步骤 2：实现选择逻辑**（单击选中、Ctrl+单击多选、Shift+单击范围选择）

- [ ] **步骤 3：双击打开预览面板**

- [ ] **步骤 4：支持键盘导航**（方向键移动焦点）

- [ ] **步骤 5：编译验证**

- [ ] **步骤 6：Commit**

---

### 任务 8：预览面板——大图渲染

**文件：**
- 创建：`photo-tool-gpui/src/components/preview.rs`

这是需求的直接回应——消除 WebView 的 2 秒全分辨率解码延迟。

- [ ] **步骤 1：加载适配面板尺寸的图片纹理**

    ```rust
    // 不是加载 220px 缩略图，而是加载适配预览面板尺寸的版本
    // 例如面板宽度 600px → 生成 600px 缩略图
    fn load_preview(&mut self, path: &str, cx: &mut Context<Self>) {
        let size = self.panel_width as u32;
        let texture = self.textures.get_or_load(path, size, cx);
        // ...
    }
    ```

- [ ] **步骤 2：实现缩放和拖拽**

    ```rust
    // 鼠标滚轮缩放
    cx.on_mouse_wheel(|state, delta, cx| {
        state.zoom_level = (state.zoom_level + delta.y * 0.001).clamp(0.1, 5.0);
    });

    // 鼠标拖拽平移
    cx.on_mouse_down(|state, button, cx| {
        state.dragging = true;
        state.drag_start = cx.mouse_position();
    });
    ```

- [ ] **步骤 3：实现自适应缩放（fit to window）**

- [ ] **步骤 4：显示 EXIF 信息叠加层**

- [ ] **步骤 5：编译验证并测量加载时间**

    目标：6000×4000 图片加载 < 200ms（对比当前 2 秒）

- [ ] **步骤 6：Commit**

---

### 任务 9：工具栏 + 筛选 + 排序

**文件：**
- 创建：`photo-tool-gpui/src/components/toolbar.rs`

- [ ] **步骤 1：使用 gpui-component 的 Input、Select 组件**

    ```rust
    div().flex_row().gap_2()
        .child(Input::new("search-placeholder").placeholder("搜索文件名..."))
        .child(Select::new("sort-by").options(&["文件名", "日期", "大小"]))
        .child(Toggle::new("sort-dir").label("↑/↓"))
    ```

- [ ] **步骤 2：搜索文本变化 → 调用 AppState::apply_filters()**

- [ ] **步骤 3：排序变更 → 更新 sort_by/sort_direction → apply_filters()**

- [ ] **步骤 4：编译验证**

- [ ] **步骤 5：Commit**

---

### 任务 10：状态栏

**文件：**
- 创建：`photo-tool-gpui/src/components/status_bar.rs`

- [ ] **步骤 1：显示总数 / 筛选数 / 选中数**

    ```rust
    div().flex_row().gap_4()
        .child(format!("{} 张图片 (筛选: {})", state.total_count, state.filtered_count))
        .child(format!("选中: {}", state.selected_count))
        .child(format!("{} | {}", capture.primary_format, human_size(capture.file_size)))
    ```

- [ ] **步骤 2：编译验证**

- [ ] **步骤 3：Commit**

---

### 任务 11：右键菜单 + 文件操作

**文件：**
- 创建：`photo-tool-gpui/src/components/context_menu.rs`

- [ ] **步骤 1：使用 gpui-component 的 ContextMenu 组件**

- [ ] **步骤 2：实现菜单项**（删除、移动、复制、重命名、导出、打开文件夹）

- [ ] **步骤 3：对接 photo-tool-core 的 ops 模块**（delete_capture、move_capture 等）

- [ ] **步骤 4：操作后刷新列表**

- [ ] **步骤 5：编译验证**

- [ ] **步骤 6：Commit**

---

### 任务 12：对话框——导入/重命名/设置/转换

**文件：**
- 创建：`photo-tool-gpui/src/components/dialogs.rs`

- [ ] **步骤 1：实现 ImportDialog**（源目录选择 rfd、行为模式、日期格式）

- [ ] **步骤 2：实现 RenameDialog**（批量重命名预览）

- [ ] **步骤 3：实现 SettingsDialog**（缩略图大小、旁车扩展名、主题切换）

- [ ] **步骤 4：实现 ConvertDialog**（输出格式、JPEG 质量、最大尺寸）

- [ ] **步骤 5：编译验证**

- [ ] **步骤 6：Commit**

---

### 任务 13：主题系统

**文件：**
- 创建：`photo-tool-gpui/src/theme.rs`

- [ ] **步骤 1：定义 Light/Dark 颜色 token**

    ```rust
    pub struct Theme {
        pub bg: Hsla,
        pub surface: Hsla,
        pub text: Hsla,
        pub text_secondary: Hsla,
        pub accent: Hsla,
        pub border: Hsla,
    }
    ```

- [ ] **步骤 2：在 AppState 中存储 theme 状态，gpui-component 通过 `init(cx)` 的 theming 支持切换**

- [ ] **步骤 3：编译验证**

- [ ] **步骤 4：Commit**

---

### 任务 14：进度条指示器

**文件：**
- 修改：`photo-tool-gpui/src/main.rs`

- [ ] **步骤 1：在窗口右下角渲染扫描进度条**（复用现有逻辑：percent + phase + path）

- [ ] **步骤 2：编译验证**

- [ ] **步骤 3：Commit**

---

### 任务 15：测试——core 模块回归 + GPUI 集成测试

**文件：**
- 修改：`photo-tool-core/src/scanner.rs`（确保 `From<&Capture> for CaptureMeta` trait 存在）
- 创建：`photo-tool-gpui/tests/integration.rs`（如果 GPUI 提供 test harness）

- [ ] **步骤 1：运行 core 全部 64 个测试**

    运行：`cargo test -p photo-tool-core`
    预期：64 passed, 0 failed

- [ ] **步骤 2：如果 GPUI test harness 可用，编写基本渲染测试**

    ```rust
    #[gpui::test]
    async fn test_app_renders(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let state = AppState::new();
            assert!(state.captures.is_empty());
        });
    }
    ```

- [ ] **步骤 3：编写扫描事件处理测试**

    模拟通道发送 `ScanEvent::Complete` → 验证 `AppState::captures` 更新。

- [ ] **步骤 4：编译验证所有测试通过**

- [ ] **步骤 5：Commit**

---

### 任务 16：废弃旧代码

**文件：**
- 删除：`src-tauri/`
- 删除：`src/`
- 修改：`Cargo.toml`（移除 `src-tauri` 从 workspace members）
- 修改：`package.json`（移除 tauri 脚本）
- 更新：`AGENTS.md`

- [ ] **步骤 1：删除 src-tauri/ 目录**

- [ ] **步骤 2：删除 src/（Vue 前端）目录**

- [ ] **步骤 3：更新 workspace Cargo.toml**——移除 `"src-tauri"`，保留 `"photo-tool-core"` 和 `"photo-tool-gpui"`

- [ ] **步骤 4：更新 package.json**——移除 `tauri`、`@tauri-apps/*` 依赖和脚本

- [ ] **步骤 5：更新 AGENTS.md**——替换为 GPUI 技术栈说明

- [ ] **步骤 6：最终编译验证**

    运行：`cargo build --workspace`
    预期：全部 crate 编译通过

- [ ] **步骤 7：Commit**

---

## 风险与依赖

| 风险 | 缓解措施 |
|------|----------|
| GPUI API 变动（pre-1.0） | 锁定 crates.io 0.2 版本，不追 git main |
| gpui-component 组件不够用 | 自行实现缺失组件（手写 div 组合） |
| GPU 纹理内存泄漏 | 任务 6 中实现 LRU 驱逐策略 |
| Windows 特定渲染 bug | 优先在 Windows 上开发测试 |
| `From<&Capture> for CaptureMeta` 未实现 | 在 core 中实现此 trait |

---

## 完成标准

- [ ] `cargo test -p photo-tool-core` 全部 64 个测试通过
- [ ] `cargo build -p photo-tool-gpui` 零错误零警告
- [ ] `cargo clippy --workspace` 零警告
- [ ] 打开 100+ 图片目录，缩略图网格 < 1 秒渲染完毕
- [ ] 预览 6000×4000 JPEG < 200ms
- [ ] 三栏可拖拽调整
- [ ] 右键菜单功能齐全
- [ ] 导入/导出/转换/重命名操作可用
- [ ] 支持 Light/Dark 主题切换
