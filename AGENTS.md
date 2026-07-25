# Repository Guidelines — Photo Tool

## 项目概述

Photo Tool 是一个**照片管理与筛选（culling）**应用，用于浏览、标记、导入和转换照片。Cargo workspace 含 2 个成员：

- `photo-tool-core` — 纯 Rust 领域逻辑库（**全同步**，无 async/tokio），9 个模块
- `photo-tool-app` — GPUI 前端（暗色主题，三栏布局，全键盘操作）
核心工作流：**目录扫描 → RAW+JPEG 配对 → 浏览/标记/筛选 → 文件操作（删除/移动/复制/重命名）→ 导入 → 格式转换**。

---

## 架构与数据流

```
┌─────────────────────────────────────────────────────┐
│                  photo-tool-core（全同步）             │
│  scanner ──► Vec<Capture> ──► exif / ops / thumbnail │
│                              / convert / import      │
│  domain（Capture, SourceFile, ImageFormat, 枚举）      │
│  xmp（pt: 命名空间附属文件）  config（TOML, 便携优先）   │
└─────────────────────────────────────────────────────┘
```

### 核心数据流

1. **scanner** → `Vec<Capture>`：walkdir 单层（`max_depth(1)`）扫描，按文件名 stem 小写归组，配对 JPEG+RAW+sidecar，`primary_index` 取 `display_priority()` 最小的非旁车文件
2. **Capture** → **exif**：提取 EXIF（常规图 kamadak-exif，RAW 走 `rawlib::exif`）；`CaptureMeta::enrich_with_exif` 回填摘要
3. **Capture** → **ops**：删除（回收站/永久）/移动（跨设备 copy+delete 回退）/复制/批量重命名
4. **SourceFile** → **thumbnail**：磁盘缓存 JPEG 字节（缓存键 = `DefaultHasher(path+size)` 的 `{:016x}.jpg`）；RAW 提取内嵌预览，常规图优先 EXIF 内嵌缩略图
5. **Capture** → **convert**：RAW 内嵌预览→JPEG、常规图缩放（Lanczos3）
6. **import**：检测可移动设备（Linux 扫 `/media` 等，Windows 枚举 A-Z 跳过 C）→ DCIM 递归扫描 → 按 EXIF 日期建子目录 → 委托 **ops** 移动/复制

### 模块依赖关系

- `import` 依赖 `exif` + `ops`
- `domain` ↔ `exif` 双向（`CaptureMeta::enrich_with_exif` 回调 `exif::extract_exif`）
- `config` 独立，无 crate 内依赖；`lib.rs` 无逻辑

---

## 关键目录

| 路径 | 用途 |
|---|---|
| `photo-tool-core/src/` | 领域逻辑：9 个模块，全部同步 |
| `photo-tool-app/src/` | GPUI 前端：`state/`（RootView + 15 方法）、`ui/`（14 组件）、`worker.rs`（rayon 桥接） |
| `photo-tool-app/src/state/app.rs` | RootView：全局状态 + `dispatch_action()` 路由所有交互 |
| `photo-tool-app/src/ui/layout.rs` | 三栏弹性布局（sidebar \| grid/preview \| info_panel） |
| `photo-tool-app/src/ui/theme.rs` | Catppuccin Mocha 暗色主题常量 |
| `local-lib/` | 预编译 Linux `libraw.so`/`libraw_r.so`（不纳入版本控制） |
| `CONTEXT.md` | 中文领域术语表（泛在语言） |
| `docs/adr/` | 架构决策记录（预留） |

---

## 开发命令

| 操作 | 命令 |
|---|---|
| 全量构建 | `cargo build` |
| 只构建核心库 | `cargo build -p photo-tool-core` |
| 运行全部核心测试 | `cargo test -p photo-tool-core` |
| 运行单个测试 | `cargo test -p photo-tool-core -- <test_name>` |
| 按模块跑测试 | `cargo test -p photo-tool-core scanner::tests` |
| 显示测试输出 | `cargo test -p photo-tool-core -- --nocapture` |
| Clippy 检查 | `cargo clippy --all-targets` |

---

## 代码规范与常见模式

### 模块组织

- `lib.rs` 仅 9 行 `pub mod` 声明（domain, config, scanner, thumbnail, exif, xmp, ops, import, convert），**无 re-export**、无 prelude
- 消费者写全路径：`photo_tool_core::scanner::scan_directory`

### 错误处理

- 每模块一个 `thiserror::Error` 枚举（共 8 个：`ConfigError`/`ScanError`/`OpError`/`ThumbnailError`/`ExifError`/`XmpError`/`ImportError`/`ConvertError`），均以 `Io(#[from] std::io::Error)` 起步；外部错误多数 `#[from]`，rawlib/kamadak-exif 错误转成 `String` 变体
- 批量操作返回 `Vec<(PathBuf, Result<(), Error>)>`，逐文件报告

```rust
#[derive(Debug, thiserror::Error)]
pub enum OpError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Trash error: {0}")]
    Trash(#[from] trash::Error),
    #[error("File not found: {0}")]
    NotFound(PathBuf),
}
```

### 序列化

- 跨边界结构体统一 `#[derive(Serialize, Deserialize)]` + `#[serde(rename_all = "camelCase")]`；纯枚举（`Rating`/`ColorLabel`/`Flag`/`Theme` 等）不加 rename
- XMP 不用 XML 解析器：字符串查找 + `regex_lite` 正则重写 `pt:Rating`/`pt:ColorLabel`/`pt:Flag` 属性（自定义命名空间 `xmlns:pt="http://ns.phototool.app/pt/1.0/"`）

### 同步 vs 异步

- **core 层全同步**（grep 无 async/await/tokio 命中）
- 平台分支用 `#[cfg(target_os = ...)]`（`import.rs`：windows/linux/macos）

### 命名惯例

- 模块/函数 snake_case，类型/枚举 PascalCase，测试统一 `test_<subject>_<scenario>`
- 谓词 `is_*`；动词前缀 `get_or_*`/`extract_*`/`set_*`
- 错误类型 `ModuleNameError`；注释全部为中文

### 已知陷阱

- `quick-xml`、`log` 在 `photo-tool-core/Cargo.toml` 声明但 src 中无引用
- `scanner::apply_filter` 当前**仅实现 `text_search`**，`FilterCriteria` 其余字段未生效
- 使用了 let-chains（edition 2024 特性），如 `config.rs` 便携路径判断

### 调试：GPUI 事件处理器无声失败

GPUI 会**静默吞掉**事件处理器（`on_click`、`cx.listener` 等）中的 panic——应用不崩溃，只是"点了没反应"。排查这类问题**必须打日志**而非猜：

```rust
.on_click({
    move |_, window, cx| {
        tracing::info!("STEP 1: click handler fired");
        do_something(window, cx);
        tracing::info!("STEP 2: do_something returned");
    }
})
```

日志没出现 = 事件没触发。日志只到 STEP 1 = 中间某步 panic 了。

终端中运行：`$env:RUST_LOG="info"; cargo run -p photo-tool-app`

### gpui-component：`open_window` 必须在 `cx.spawn` 里

`cx.open_window()` **不能**直接在 `app.run()` 回调中调用，必须放在 `cx.spawn(async …)` 内，否则 `Root` 不会正确注册为窗口根视图，导致 `window.root::<Root>()` 返回 `None`——所有依赖 `Root` 的功能（Dialog、Sheet、Notification 等）全部静默失效。

```rust
// ❌ 错误 —— Root 不会注册
app.run(move |cx| {
    cx.open_window(..., |window, cx| {
        cx.new(|cx| Root::new(view, window, cx))
    })
});

// ✅ 正确 —— Quick Start 文档的写法
app.run(move |cx| {
    gpui_component::init(cx);
    cx.spawn(async move |cx| {
        cx.update(|cx| {
            cx.open_window(..., |window, cx| {
                cx.new(|cx| Root::new(view, window, cx))
            })
        });
    })
    .detach();
});
```

---

## 重要文件

| 文件 | 作用 |
|---|---|
| `Cargo.toml` | workspace：resolver v2，1 个成员，version 0.1.0，edition 2024，无 profile 配置 |
| `rust-toolchain.toml` | 固定 nightly 频道（edition 2024 需要） |
| `.cargo/config.toml` | 仅 Linux target：`rustflags = ["-L", "local-lib"]` + `[env] LD_LIBRARY_PATH=local-lib`（libraw.so 链接） |
| `photo-tool-app/Cargo.toml` | gpui + gpui-component + rayon + tracing + rfd |
| `CONTEXT.md` | 领域术语表（Capture/Stack/Rating 等泛在语言） |
| `.gitignore` | 含 `libraw.so`、`local-lib/`、`nul`（Windows 保留名产物） |


## gpui-component 本地源码与文档

gpui-component 项目位于 `D:\Dev\Code\gpui-component`，含完整源码和本地文档：

|路径|内容|
|---|---|
|`crates/ui/src/`|组件库 Rust 源码（Button/Select/Input 等）|
|`crates/ui/src/theme/mod.rs`|`Theme` 结构体与 `font_family` 字段定义|
|`docs/docs/theme.md`|主题系统文档|
|`docs/docs/`|更多组件文档|
|`crates/story/src/stories/`|各组件 Story/示例代码（如 `select_story.rs`）|

所有 gpui-component API 查询都应优先阅读本地源码而非网络文档。
---

## 运行时与工具链偏好

- **Rust**：nightly 频道，edition 2024；包管理 Cargo（workspace）
- **无 CI/CD**（无 `.github/`）、**无 rustfmt/clippy 配置**（用默认）、**无任何构建脚本**（无 Makefile/justfile/sh/ps1/bat）

### 关键外部依赖

| Crate | 用途 |
| `gpui`（git = zed-industries/zed） | GPU 加速 UI 框架 |
| `gpui-component`（git = longbridge/gpui-component） | 60+ 桌面组件库 |
| `rayon` | 线程池，core 同步调用异步化 |
| `image 0.25` | JPEG/PNG/TIFF/WEBP/BMP/GIF 编解码与缩放（default-features=false + 6 features） |
| `kamadak-exif 0.6` | 常规图 EXIF 解析 |
| `trash 4` | 移到回收站（跨平台） |
| `walkdir 2` | 单层目录扫描 |
| `chrono 0.4` / `toml 0.8` / `serde 1` | 日期 / 配置 / 序列化 |
| `regex-lite 0.1` | XMP 属性重写 |
| `thiserror 2` | 错误 derive |
| `tempfile 3`（dev） | 测试临时目录 |

### 平台要求

- **Linux**：需 `libraw.so` 可链接/可加载——放 `local-lib/` 或系统安装；`.cargo/config.toml` 已配 `-L local-lib` 与 `LD_LIBRARY_PATH`
- **Windows**：无特殊配置（Windows 11 为开发/测试环境）；`import.rs` 有 `#[cfg(target_os)]` 分支

---

## 测试与 QA

- 全部 **64 个 `#[test]`** 为 `photo-tool-core/src/*.rs` 文件末尾的内联 `#[cfg(test)] mod tests`
- 无外部 `tests/` 目录、无异步测试、无第三方测试框架（唯一 dev-dep：`tempfile`）

### 测试分布

| 模块 | 测试数 | 覆盖内容 |
|---|---|---|
| `domain.rs` | 9 | 扩展名解析（大小写不敏感）、RAW 白名单、display_priority、is_viewable |
| `config.rs` | 3 | 默认值、TOML 保存/加载往返、配置路径 |
| `scanner.rs` | 8 | JPEG+RAW 配对、大小写、sidecar 分离、忽略视频 |
| `ops.rs` | 8 | 移动/复制/重命名/删除（含 sidecar）、命名冲突、批量跳过缺失 |
| `thumbnail.rs` | 6 | 缓存命中、键唯一性、stats/clear、prune 淘汰最旧、错误路径 |
| `exif.rs` | 7 | 无 EXIF 报错、summary 格式、不存在文件、file_size 始终填充 |
| `import.rs` | 8 | date_subfolder 三种格式与回落、设备检测不崩溃、空导入 |
| `convert.rs` | 7 | resize 三分支、格式分发、RAW 错误、输出路径命名 |
| `xmp.rs` | 8 | 默认值、枚举↔字符串转换、读写往返、重复写更新 |

### 测试辅助

- 各模块私有 helper + `tempfile::TempDir` 做 FS 隔离：`create_test_files`（scanner）、`make_test_capture`（ops）、`create_test_jpeg`（convert/thumbnail）、`create_test_jpeg_with_exif`（exif）
- 断言用 `assert!`/`assert_eq!`，Result 直接 `unwrap()`；无共享 fixture
- 命名：`test_<subject>_<scenario>`，一个测试验证一个行为或边界

---

## GPUI 前端参考

> 来源：`D:\Dev\Code\zed-main\crates\gpui\docs\` 和 `docs\src\development\glossary.md`

### 架构概述

GPUI 是 Zed 编辑器的 GPU 加速 Rust UI 框架，pre-1.0，版本间有 breaking changes。提供三种抽象层级：

1. **Entity 状态管理** — 通过 `Entity<T>` 智能指针管理应用状态，`App` 持有所有 Entity
2. **View 声明式 UI** — `impl Render` 构建 element tree，用 tailwind 风格 API 布局和样式
3. **Element 命令式 UI** — 底层 element trait，完全控制渲染和布局（如 `uniform_list`）

### Context 类型

| Context | 生命周期 | 用途 |
|---|---|---|
| `App` (`&mut App`) | 引用，UI 线程 | 全局状态根，持有所有 Entity |
| `Context<T>` (`&mut Context<T>`) | 引用，绑定 Entity | `App` + Entity 专属方法（notify、emit） |
| `AsyncApp` | 值，UI 线程 | `App` 的 owned 版本，可跨 await 持有 |
| `WindowContext` | 引用 | `App` + 当前窗口 |
| `AsyncWindowContext` | 值 | 静态生命周期，后台线程回调 UI 的桥梁 |
| `TestAppContext` | 值 | 测试专用，模拟输入，panic 替代 fallible |

### 核心类型

```rust
Entity<T>     // 强引用，等价于 App::EntityMap 的 key
WeakEntity<T> // 弱引用，类似 std::rc::Weak，用于 async 回调中安全访问
Global<T>     // 单例，App 内唯一
Task<T>       // 已 spawn 的 future，自动运行，detach() 取消
```

### Render trait（View 模式）

```rust
struct MyView { state: String }

impl Render for MyView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(0x1e1e2e))
            .flex()
            .child("Hello")
    }
}

// 在 window 中创建：
cx.new(|cx| MyView { state: "".into() })
```

### 常用 Element API

| 方法 | 作用 |
|---|---|
| `div()` | 通用容器 |
| `.flex()` / `.flex_row()` / `.flex_col()` | flex 布局 |
| `.size_full()` / `.w(px(...))` / `.h(px(...))` | 尺寸 |
| `.flex_grow(f32)` | flex-grow |
| `.bg(Rgba)` / `.text_color(Rgba)` | 颜色 |
| `.border_1()` / `.border_color()` | 边框 |
| `.rounded_md()` / `.rounded_full()` | 圆角 |
| `.p_2()` / `.px_3()` / `.py_1()` / `.gap_2()` | 间距 |
| `.text_sm()` / `.text_xl()` / `.font_weight()` | 字体 |
| `.child(element)` / `.children(iter)` | 子元素 |
| `.when(cond, |d| d.child(...))` | 条件渲染 |
| `.hover(\|style\| style.bg(...))` | hover 样式 |
| `.cursor_pointer()` | 鼠标指针 |
| `.truncate()` | 文本截断 |
| `.overflow_hidden()` | 溢出隐藏 |
| `.absolute()` / `.relative()` | 定位 |
| `.items_center()` / `.justify_center()` / `.justify_between()` | 对齐 |

### 事件处理

```rust
div()
    .id(ElementId::Name("my-btn".into()))
    .on_click(cx.listener(|view, event: &ClickEvent, window, cx| {
        view.do_something(cx);
    }))
    .on_key_down(cx.listener(|view, event: &KeyDownEvent, window, cx| {
        match event.keystroke.key.as_str() {
            "enter" => view.confirm(cx),
            _ => {}
        }
    }))
```

**注意**: `.on_click()` 要求元素先调用 `.id()`（GPUI 的 StatefulInteractiveElement 约束）。

### Key Dispatch（Actions 系统）

用 `#[gpui::action]` 定义逻辑操作，通过 `key_context` 绑定按键：

```rust
#[gpui::action]
struct TogglePreview;

impl Render for MyView {
    fn render(&mut self, w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context("PhotoViewer")
            .on_action(|this: &mut MyView, _: &TogglePreview, _w, cx| {
                this.preview_visible = !this.preview_visible;
                cx.notify();
            })
    }
}
```

然后在 keymap JSON 中绑定：
```json
{ "context": "PhotoViewer", "bindings": { "space": "TogglePreview" } }
```

### uniform_list（虚拟列表）

```rust
gpui::uniform_list("my-list", item_count, move |range, _window, _app| {
    range.filter_map(|i| {
        let item = data.get(i)?;
        Some(div().child(item).into_any_element())
    }).collect::<Vec<_>>()
})
```

**注意**: `uniform_list` 回调中不能使用 `cx`（参数是 `&mut Window, &mut App`），需要通过 `update_entity` 回主 View。

### Async 桥接（后台任务 → UI 线程）

```rust
// 在 View 方法中
cx.spawn(|view_handle, mut cx| async move {
    let result = do_heavy_work().await;
    cx.update_entity(&view_handle, |view, cx| {
        view.data = result;
        cx.notify();
    }).ok();
}).detach();
```

### App 入口

```rust
fn main() {
    gpui_platform::application()
        .run(|cx: &mut App| {
            cx.activate(true);
            cx.open_window(WindowOptions::default(), |_window, cx| {
                cx.new(|_cx| RootView::new())
            }).unwrap();
        });
}
```
