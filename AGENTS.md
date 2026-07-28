# Repository Guidelines — Photo Tool

## 项目概述

Photo Tool 是一个**照片管理与筛选（culling）**应用，用于浏览、标记、识别和转换照片。Cargo workspace 含 5 个成员：

- `photo-domain` — 纯类型叶子 crate（Capture, ExifMetadata, XmpMetadata, Recognition 类型, 枚举），零外部依赖
- `photo-engine` — 文件操作引擎（scanner, exif/xmp 读写, thumbnail, ops, convert, folder_db），**全同步**
- `photo-recognize` — 鸟类识别管线（YOLO 检测 → 鸟种分类 → 名录映射，ONNX Runtime），**全同步**
- `photo-config` — 配置读写（TOML + SQLite 持久化）
- `photo-tool-app` — GPUI 前端（暗色主题，三栏布局，全键盘操作）
核心工作流：**目录扫描 → RAW+JPEG 配对 → 浏览/标记/筛选 → 鸟类识别（单张/批量）→ 文件操作（删除/移动/复制/重命名）→ 格式转换**。识别子系统设计见 `docs/adr/0003-recognition-subsystem.md`。

---

## 架构与数据流

```
                    ┌─────────────────────────────────────┐
                    │            photo-domain              │
                    │  Capture, ExifMetadata, XmpMetadata  │
                    │  ImageFormat, 枚举（纯数据，零依赖）   │
                    └──────────┬──────────────────────────┘
                               │
              ┌────────────────┼────────────────┐
              ▼                ▼                ▼
     ┌──────────────┐  ┌──────────────┐  ┌──────────────┐
     │ photo-engine │  │photo-recognize│ │ photo-config │
     │ scanner,ops  │  │ detect,classify│ │ TOML+SQLite │
     │ exif,xmp,    │  │ catalog,     │  │ 便携配置     │
     │ thumbnail,   │  │ pipeline     │  └──────────────┘
     │ convert,     │  └──────────────┘         ▲
     │ folder_db    │         ▲                 │
     └──────────────┘         │        ┌──────────────┐
     所有模块全同步            └────────│ photo-tool-app│
                            依赖      │ GPUI 前端     │
                                      │ state/ ui/    │
                                      │ worker        │
                                      └──────────────┘
```

- `photo-tool-app` 依赖其余四者；`photo-recognize` 依赖 `photo-domain`（RAW 输入解码调用 `photo_engine::thumbnail::decode_raw_preview`）

### 核心数据流

1. **scanner** → `Vec<Capture>`：walkdir 单层（`max_depth(1)`）扫描，按文件名 stem 小写归组，配对 JPEG+RAW+sidecar，`primary_index` 取 `display_priority()` 最小的非旁车文件
2. **Capture** → **exif**：提取 EXIF（常规图 kamadak-exif，RAW 走 `rawlib::exif`）；`CaptureMeta::enrich_with_exif` 回填摘要（类型 `ExifMetadata` 定义在 domain，提取机械在 engine）
3. **Capture** → **ops**：删除（回收站/永久）/移动（跨设备 copy+delete 回退）/复制/批量重命名
4. **SourceFile** → **thumbnail**：磁盘缓存 JPEG 字节（缓存键 = `DefaultHasher(path+size)` 的 `{:016x}.jpg`）；RAW 提取内嵌预览，常规图优先 EXIF 内嵌缩略图
5. **Capture** → **convert**：RAW 内嵌预览→JPEG、常规图缩放（Lanczos3）
6. **Capture** → **recognize**：`photo-recognize` 管线（YOLO 检测鸟体 → bird_model 分类 Top-5 → `sp_cls_map` JOIN `animal_info` 名录映射）→ `Recognition` 三态（Confirmed/NeedsReview/Unrecognized）→ `folder_db` upsert 到文件夹级 `.pt/data.db`
7. **import**（近期移除，待重建）：检测可移动设备 → DCIM 递归扫描 → 按 EXIF 日期建子目录 → 委托 **ops** 移动/复制

### 模块依赖关系

- `photo-tool-app` 依赖其余四个 crate
- `photo-engine` 依赖 `photo-domain`（单向 DAG，由 crate 边界强制）
- `photo-recognize` 依赖 `photo-domain`（RAW 输入解码复用 `photo_engine::thumbnail::decode_raw_preview`，不反向依赖）
- `photo-config` 独立，无 crate 内依赖
- `domain` 是纯叶子：依赖仅 `std` + `serde` + `chrono` + `thiserror`，不引用任何内部模块

---

## 关键目录

|-|-|
| `crates/photo-domain/src/domain.rs` | 纯类型（Capture, ExifMetadata, XmpMetadata, 枚举），零外部 crate 依赖 |
| `crates/photo-engine/src/` | 文件机械：scanner, ops, exif, xmp, thumbnail, convert, folder_db（全部同步） |
| `crates/photo-engine/src/folder_db.rs` | 文件夹级 SQLite（`.pt/data.db`）：exif_cache / xmp_meta / **recognition** 三表，rusqlite_migration 版本化 |
| `crates/photo-recognize/src/` | 识别管线：lib.rs(Recognizer 门面), detect(YOLO), classify(bird_model), catalog(名录映射), pipeline |
| `crates/photo-config/src/lib.rs` | 配置读写（TOML + SQLite 持久化）|
| `crates/photo-tool-app/src/state/app.rs` | RootView：全局状态 + `dispatch_action()` 路由所有交互 |
| `crates/photo-tool-app/src/ui/layout.rs` | 三栏弹性布局（sidebar \| grid/preview \| info_panel） |
| `crates/photo-tool-app/src/ui/theme.rs` | Catppuccin Mocha 暗色主题常量 |
| `local-lib/` | 预编译 Linux `libraw.so`/`libraw_r.so`（不纳入版本控制） |
| `CONTEXT.md` | 中文领域术语表（泛在语言） |
| `docs/adr/` | 架构决策记录 |

---

## 开发命令

| 操作 | 命令 |
|---|---|
| 全量构建 | `cargo build` |
| 只构建 | `cargo build -p photo-engine` |
| 运行全部核心测试 | `cargo test` |
| 运行单个包测试 | `cargo test -p photo-engine` |
| 按模块跑测试 | `cargo test -p photo-engine -- scanner::tests` |
| 显示测试输出 | `cargo test -- --nocapture` |
| Clippy 检查 | `cargo clippy --all-targets` |

---

## 代码规范与常见模式

### 模块组织

- `photo-domain/src/lib.rs` 声明 `pub mod domain` + `pub use domain::*`（re-export 让消费者直接 `photo_domain::Capture`）
- `photo-engine/src/lib.rs` 声明 7 个 `pub mod`（scanner, ops, exif, xmp, thumbnail, convert, folder_db）
- `photo-config/src/lib.rs` 即库根——config 模块就是 lib.rs 本身
- 消费者写全路径：`photo_engine::scanner::scan_directory`

### 错误处理

- 每模块一个 `thiserror::Error` 枚举（`ConfigError`/`ScanError`/`OpError`/`ThumbnailError`/`ExifError`/`XmpError`/`ConvertError`/`FolderDbError`/`RecognizeError`），均以 `Io(#[from] std::io::Error)` 起步；外部错误多数 `#[from]`，rawlib/kamadak-exif 错误转成 `String` 变体
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

- `quick-xml` 在根 `Cargo.toml` 的 `[workspace.dependencies]` 中声明但 `photo-engine` src 中无引用
- `scanner::apply_filter` 当前仅实现 `text_search`（已扩展为同时匹配文件名与 `bird_name`）与 `recognition_filter`；`FilterCriteria` 其余字段未生效（`paired_only`, `date_range`, `flag_filter`, `unflagged_filter` 等被 scanner 忽略）
- 使用了 let-chains（edition 2024 特性），如 `photo-config/config.rs` 便携路径判断

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
| `Cargo.toml` | workspace：resolver v2，5 个成员，`[workspace.dependencies]` 集中管理所有版本 |
| `rust-toolchain.toml` | 固定 nightly 频道（edition 2024 需要） |
| `.cargo/config.toml` | 仅 Linux target：`rustflags = ["-L", "local-lib"]` + `[env] LD_LIBRARY_PATH=local-lib`（libraw.so 链接） |
| `crates/photo-tool-app/Cargo.toml` | gpui + gpui-component + rayon + tracing + rfd |
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
- **无 CI/CD**（无 `.github/`）、**无 rustfmt/clippy 配置**（用默认）；唯一脚本是 `scripts/package.ps1`（发布打包：release 构建 + 收集 exe/DLL/模型/名录库 → zip，非日常构建依赖）

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
| `ort 2.0.0-rc.10`（photo-recognize） | ONNX Runtime 绑定；`download-binaries` 静态链接运行时，Windows 走 DirectML EP（失败回退 CPU），仅需随包携带 `DirectML.dll` |
| `regex-lite 0.1` | XMP 属性重写 |
| `thiserror 2` | 错误 derive |
| `tempfile 3`（dev） | 测试临时目录 |

### 平台要求

- **Linux**：需 `libraw.so` 可链接/可加载——放 `local-lib/` 或系统安装；`.cargo/config.toml` 已配 `-L local-lib` 与 `LD_LIBRARY_PATH`；识别管线在 Linux 走 CPU EP
- **Windows**：无特殊配置（Windows 11 为开发/测试环境）；识别走 DirectML（系统内置），发布包需附带 `DirectML.dll`
- **识别资产**：`models/`（yolo26l.onnx + bird_model.onnx，约 250MB）与 `data/pica_ref.db`（名录库）必须位于 **exe 同级目录**（便携约定，不入库，`.gitignore` 已排除）；开发时即 `target/debug/` 或 `target/release/` 下

---

## 测试与 QA

- 全部 **123 个 `#[test]`**（+ 1 个 `#[ignore]` 真机冒烟）分布在 4 个 crate 的源文件末尾内联 `#[cfg(test)] mod tests
- 无外部 `tests/` 目录、无异步测试、无第三方测试框架（唯一 dev-dep：`tempfile`）
- 真机识别冒烟：`cargo test -p photo-recognize -- --ignored`（需 worktree/发布根有 `models/` 与 `data/pica_ref.db`）；单文件手动识别工具：`cargo run -p photo-recognize --example recognize_file -- <图片路径>`

### 测试分布

| 模块（新 crate） | 测试数 | 覆盖内容 |
|---|---|---|
| `photo-domain::domain.rs` | 25 | 扩展名解析、RAW 白名单、enrich_with_xmp/recognition、ExifMetadata 默认/摘要、XmpMetadata 枚举转换、BBox/RecognitionStatus/RecognitionFilter 序列化与状态映射 |
| `photo-config::lib.rs` (config) | 6 | 默认值、TOML 保存/加载往返、配置路径 |
| `photo-engine::scanner.rs` | 8 | JPEG+RAW 配对、大小写、sidecar 分离、忽略视频 |
| `photo-engine::ops.rs` | 8+ | 移动/复制/重命名/删除（含 sidecar）、命名冲突、批量跳过缺失、识别行同步（sync_delete/copy/rename） |
| `photo-engine::thumbnail.rs` | 6 | 缓存命中、键唯一性、stats/clear、prune 淘汰最旧、错误路径 |
| `photo-engine::exif.rs` | 5 | 无 EXIF 报错、不存在文件、file_size 始终填充 |
| `photo-engine::convert.rs` | 7 | resize 三分支、格式分发、RAW 错误、输出路径命名 |
| `photo-engine::xmp.rs` | 6 | xmp_path、读写往返、不存在的文件返回默认值、更新已有文件 |
| `photo-engine::folder_db.rs` | 7 | 识别表建表/迁移、upsert/get/delete、rename/copy 同步、all_recognitions、旧版 cache.db 迁移 |
| `photo-recognize::lib.rs/pipeline.rs` | 22 | 阶段→状态映射、输入源解析（JPEG/RAW）、检测框变换、softmax/Top-5、名录映射 0/1/多、进度回调；另 1 个 `#[ignore]` 真机冒烟 |
| `photo-tool-app` | 9 | action/状态工具函数 |

### 测试辅助

- 各模块私有 helper + `tempfile::TempDir` 做 FS 隔离
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
