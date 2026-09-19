# Repository Guidelines — Photo Tool

## 项目概述

Photo Tool 是一个**照片管理与筛选（culling）**应用，用于浏览、标记、识别和转换照片。Cargo workspace 含 6 个成员：

- `photo-domain` — 纯类型叶子 crate（Capture, ExifMetadata, XmpMetadata, Recognition 类型, 枚举），依赖仅 serde + chrono
- `photo-engine` — 文件操作引擎（scanner, ops, exif, thumbnail, convert, folder_db, batch_ops, global_db 跨文件夹鸟种索引, histogram 直方图/剪切, import SD 卡导入, template 命名模板, undo 批量撤销日志），**全同步**
- `photo-recognize` — 鸟类识别管线（YOLO 检测 → 鸟种分类 → 名录映射 → 鸟眼锐度，ONNX Runtime），**全同步**
- `photo-config` — 配置读写（TOML + SQLite 持久化）
- `photo-ui` — **GPUI 桌面前端**（gpui-kit 0.6.1：Dock 三栏 + 网格/预览/幻灯片/统计 + 导入/设置，Material You 主题），2026-09 起是唯一前端（原 Tauri v2 + Vue 3 版 `crates/photo-tauri` 已删除）
- `rawlib` — LibRaw 的 Rust 封装（第三方 crate，本地 path 依赖）

核心工作流：**目录扫描（单层/递归可配）→ 浏览/标记/筛选 → 鸟类识别（单张/批量）→ 文件操作（删除/移动/复制/重命名，可撤销）→ 格式转换/导出预设**；另含 **SD 卡导入、全局鸟种统计、幻灯片**。历史迁移（Tauri v2 → GPUI 重写）见 git 历史与 `docs/gui-design-manual.md`（GPUI 版设计手册）。

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
     │ scanner,ops  │  │ detect,classify│ │ TOML+SQLite  │
     │ exif,xmp,    │  │ catalog,     │  │ 便携配置     │
     │ thumbnail,   │  │ pipeline     │  └──────────────┘
     │ convert,     │  └──────────────┘         ▲
     │ folder_db    │         ▲                 │
     └──────────────┘         │        ┌──────────────────────┐
     所有模块全同步            └────────│      photo-ui        │
                            依赖      │  (GPUI 桌面前端 crate) │
                                      │   直接函数调用，无 IPC  │
                                      └──────────────────────┘
```

- `photo-ui` 依赖其余四者（**同进程直接调库，无 IPC、无绑定层**）；`photo-recognize` 依赖 `photo-domain`（RAW 输入解码调用 `photo_engine::thumbnail::decode_raw_preview`）
- **前端**（`crates/photo-ui/src/`）：GPUI 单进程应用——`app.rs` 组装 Dock 三栏（左：文件树/批量操作；中：网格/预览/幻灯片/统计；右：信息/调整）、注册键位；`actions.rs` 定义 action；`views/` 出视图；`state/`（`AppState` / `import` / `engine_ops` 引擎桥）；`model/`（筛选/排序/堆叠/连拍/预览数学等纯逻辑）；`image/`（`ImageManager` 进程内解码）
- **无跨语言绑定**：Rust 类型不再导出 TS；domain/config/recognize 上的 `specta` **可选 feature** 是 Tauri 版遗留，默认构建不启用

### 核心数据流

1. **scanner** → `Vec<Capture>`：walkdir 单层（`max_depth(1)`）扫描，**每个图片文件一个 Capture**（扫描模型不配对）；网格**显示层**按配置堆叠模式分组（`crates/photo-ui/src/model/stacks.rs`：ByTime 同组照片堆叠按拍摄时间 ≤2s 聚类、ByFileName 同 stem 合并、None 关闭；主格式默认 JPEG 优先，堆叠组底部成员缩略图带点击直达激活格式、语义徽标区分同画面多格式/连拍，方向键在堆叠组间导航、Q/E 组内切换）；扫描期不做任何筛选，识别状态等元数据在扫描后才从 folder_db 读取，全部筛选由 `crates/photo-ui/src/model/filter.rs` 在 CaptureMeta 层面执行
2. **Capture** → **exif**：提取 EXIF（统一走 `ExifProvider` 抽象：**exiftool 主后端**（`-stay_open` 长驻进程 + JSON，覆盖 JPEG/RAW 及厂商私有对焦点），**rawlib 回退后端**（RAW 专用，Fuji FocusPixel / Nikon AFInfo blob 本地解析）；kamadak-exif 已移除）；`CaptureMeta::enrich_with_exif` 回填摘要（类型 `ExifMetadata` 定义在 domain，提取机械在 engine）
3. **Capture** → **ops**：删除（回收站/永久）/移动（跨设备 copy+delete 回退）/复制/批量重命名
4. **SourceFile** → **thumbnail**：磁盘缓存 JPEG 字节（缓存键 = `DefaultHasher(path+size)` 的 `{:016x}.jpg`，目录 = 照片目录 `.pt/thumbs`，每文件夹独立）；RAW 完整解码（half_size 预览选项）母版按 `u32::MAX` 键存一份，网格缩略图/预览/全分辨率均从母版 DCT 派生（不落盘）；内嵌 JPEG 长边 ≥2048（RW2/DNG 大内嵌）时直接用作母版省解码；常规图优先 EXIF 内嵌缩略图
5. **Capture** → **convert**：RAW 内嵌预览→JPEG、常规图缩放（Lanczos3）；`export_adjusted` 全尺寸烘焙（调整参数渲染）
6. **Capture** → **recognize**：`photo-recognize` 管线（YOLO 检测鸟体 → 整图 eye.onnx 检测眼（双槽一致性选点，CPU 推理）→ bird_model 分类 Top-5 → `sp_cls_map` JOIN `animal_info` 名录映射）→ `Recognition` 三态（Confirmed/NeedsReview/Unrecognized）+ 连续鸟眼锐度分（NULL 兜底，不影响三态）→ `folder_db` upsert 到文件夹级 `.pt/data.db`
7. **import**：检测可移动设备（Windows GetDriveTypeW FFI；Linux 解析 /proc/mounts + /sys/class/block removable；其余平台退化手动选源）→ 递归扫描源（只 stat 取文件修改时间，不读 EXIF）→ 按文件日期建 YYYY-MM-DD 子目录 → 去重（同名同大小跳过）→ 委托 **ops** 移动/复制；`DriveInfo.path` 跨平台统一为根路径（Windows 盘符根 / Linux 挂载点）

### 模块依赖关系

- `photo-ui` 依赖其余四者；`photo-engine` 依赖 `photo-domain`（单向 DAG，由 crate 边界强制）；`photo-recognize` 依赖 `photo-domain`；`photo-config` 独立；`domain` 是纯叶子

---

## 关键目录

|---|---|
|`crates/photo-domain/src/domain.rs`|纯类型（Capture, ExifMetadata, XmpMetadata, 枚举），零外部 crate 依赖；`specta::Type` derive 经 feature 门控（`cfg_attr(feature = "specta", ...)`）|
|`crates/photo-engine/src/`|文件机械：scanner, ops, exif, thumbnail, convert, folder_db, batch_ops, adjustments（全部同步）|
|`crates/photo-engine/src/folder_db.rs`|文件夹级 SQLite（`.pt/data.db`）：exif_cache / xmp_meta / recognition / adjustments 四表，rusqlite_migration 版本化|
|`crates/photo-recognize/src/`|识别管线：lib.rs(Recognizer 门面), detect(YOLO), classify(bird_model), catalog(名录映射), pipeline, eye, sharpness|
|`crates/photo-config/src/lib.rs`|配置读写（TOML + SQLite 持久化）；AppConfig 含 favorite_dirs/recent_directories/theme(默认 Light)/leftPanelWidth/rightPanelWidth/thumbnailSize/recognitionThreadCount/stackMode(默认 None 不堆叠) 等|
|`crates/photo-ui/src/app.rs`|GPUI 应用装配：窗口、键位表（46 个 `KeyBinding`）、action 分发、Dock 工作区创建|
|`crates/photo-ui/src/views/dock_panels.rs`|gpui-kit Dock 工作区：左右停靠区 + 中央主视图；`DockPanel` 的 `panel_name`/`closable`/`zoomable`/`zoom_control` 语义在这里|
|`crates/photo-ui/src/views/`|grid / preview / slideshow / stats / filmstrip / info_panel / left_panel / filter_bar / header / status_bar / dialogs/|
|`crates/photo-ui/src/state/`|`AppState`（单实体）/ `import.rs`（导入状态机）/ `engine_ops.rs`（调同步引擎的唯一桥）|
|`crates/photo-ui/src/model/`|adjust / filter / sort / stacks / burst / preview_math / best_frame（纯逻辑 + 内联单测）|
|`crates/photo-ui/src/theme/`|Material You 动态取色：`material.rs`（HCT/CAM16）+ `scheme.rs`（seed → 亮/暗语义色）|
|`docs/exiftool-update.md`|ExifTool 本地运行时更新指引（EXIF 后端依赖，进 git）|
|`local-lib/`|预编译 Linux `libraw.so`/`libraw_r.so` + `exiftool/`（ExifTool 跨平台运行时：`windows/` perl.exe+exiftool.pl、`linux/` 源码包、VERSION.txt；更新指引 `docs/exiftool-update.md`，均不纳入版本控制）|

---

## 开发命令

|操作|命令|
|---|---|
|Rust 全量构建|`cargo build`|
|前端检查|`cargo check -p photo-ui`|
|运行核心测试|`cargo test -p photo-engine -p photo-recognize -p photo-domain -p photo-config`|
|前端单测|`cargo test -p photo-ui`（`src/model/` 纯逻辑 + `theme/` 色彩用例）|
|开发运行（GPUI 桌面窗口）|`cargo run -p photo-ui`|
|无头冒烟|`XDG_CONFIG_HOME=/tmp/ptlease-config xvfb-run -a cargo run -p photo-ui --example lease_smoke`（另有 `preview_full_smoke`、`clipboard_smoke`、`adjust_smoke`、`grid_scroll_smoke`、`recognize_smoke`、`master_bench`）|
|EXIF 提取验证|`cargo run -p photo-engine --example focus_check -- <图片>`（打印 ExifMetadata + 对焦点；exiftool 不可用或残留进程时先 `taskkill //F //IM perl.exe`）|

---

## 代码规范与常见模式

### 模块组织

- `photo-domain/src/lib.rs` 声明 `pub mod domain` + `pub use domain::*`（re-export 让消费者直接 `photo_domain::Capture`）
- `photo-engine/src/lib.rs` 声明 `pub mod`（scanner, ops, exif, thumbnail, convert, folder_db, batch_ops, adjustments）；XMP 读写实现在 folder_db 的 xmp_meta 表，无独立 xmp.rs
- `photo-config/src/lib.rs` 即库根——config 模块就是 lib.rs 本身
- `photo-ui`：`main.rs` 起 GPUI 应用（`logging::init` → `AppState` → 开窗）；`app.rs` 注册键位与 action，`views/` 组装界面，`state/engine_ops.rs` 是调同步引擎的唯一桥
- 消费者写全路径：`photo_engine::scanner::scan_directory`

### 错误处理

- 每模块一个 `thiserror::Error` 枚举（`ConfigError`/`ScanError`/`OpError`/`ThumbnailError`/`ExifError`/`XmpError`/`ConvertError`/`FolderDbError`/`RecognizeError`），均以 `Io(#[from] std::io::Error)` 起步；外部错误多数 `#[from]`，rawlib/exiftool 错误转成 `String` 变体
- 批量操作返回 `Vec<(PathBuf, Result<(), Error>)>`，逐文件报告
- UI 层不跨边界返回 `Result`；引擎错误在 `state/engine_ops.rs` 落日志并转成状态栏/弹窗提示（GPUI 无 IPC 边界）

### 序列化

- 跨边界结构体统一 `#[derive(Serialize, Deserialize)]` + `#[serde(rename_all = "camelCase")]`；纯枚举（`Rating`/`ColorLabel`/`Flag`/`Theme` 等）不加 rename
- `#[cfg_attr(feature = "specta", derive(specta::Type))]` 门控保留在 domain/config/recognize（Tauri 版导出 TS 绑定用）；默认构建不启用，也没有消费方

### 同步 vs 异步

- **core 层全同步**（grep 无 async/await/tokio 命中）；`photo-ui` 在 GPUI executor（`cx.spawn` / 后台线程）里跑同步引擎调用，靠 `AppState` 的 loading/进度字段驱动重绘

### 命名惯例

- 模块/函数 snake_case，类型/枚举 PascalCase，测试统一 `test_<subject>_<scenario>`
- 谓词 `is_*`；动词前缀 `get_or_*`/`extract_*`/`set_*`
- 错误类型 `ModuleNameError`；注释全部为中文
- 前端就是 Rust（GPUI），沿用同一套命名；没有 TS 层

### 已知陷阱

- **dev 构建必须给像素密集的 crate 开 `opt-level = 3`**：根 `Cargo.toml` 的 `[profile.dev.package.*]` 除 `image`/`rawlib`/`rusqlite` 外，还要有 `photo-engine`/`jpeg-decoder`/`zune-jpeg`/`zune-core`。实测生成一张 39MP JPEG 的 2560 预览母版：**debug 7.45s vs release 0.55s**——漏开就是"点开一张图卡 7 秒"
- `quick-xml` 在根 `Cargo.toml` 的 `[workspace.dependencies]` 中声明但各 crate src 无引用
- **exiftool `-stay_open` 长驻进程不能加 `-q`**：`-q` 同时抑制 `{ready}` 标记，导致 execute 读不到结果边界挂起
- **Windows 官方 exiftool(-k).exe 内嵌 `-k`（每命令后等 ENTER）**：程序化调用必须用 `perl.exe exiftool.pl`（photo-engine 已自动处理）；开发时残留 perl.exe 进程会让后续 cargo 命令假死，`taskkill //F //IM perl.exe` 清理（cfg(test) 已跳过真实 spawn；正常退出由 `photo-ui` 的 `main` 调 `shutdown_provider`，仅手动 example 需注意）
- **exiftool 定位优先级**：`PHOTO_EXIFTOOL` env → exe 同级 `exiftool/`（打包）→ 仓库 `local-lib/exiftool/`（开发）→ PATH；升级版本见 `docs/exiftool-update.md`
- **配置目录定位（`photo_config::config_dir()`）**：`PHOTO_CONFIG_DIR`（值就是目录本身）→ `XDG_CONFIG_HOME`（取 `pt/` 子目录）→ `~/.config/pt`；空串按未设置。刻意不用 `dirs::config_dir()`（它在 Windows 给 `%APPDATA%`，会破坏全平台同一相对布局）。`config.toml` 与 `logs/` 都在这个目录下（`logging::init` 从 `determine_config_path()` 的父目录派生）。**无头冒烟的 `XDG_CONFIG_HOME=...` 隔离靠这条生效**：2026-09-19 之前这里硬编码 home，冒烟其实一直读写用户真实配置，`lease_smoke` 的「左停靠区宽度来自配置」检查因此在左栏被拖宽过的机器上假失败。
- **arboard 剪贴板必须留常驻持有者（X11）**：X11 剪贴板的数据是进程应答 X 请求时才提供的，`arboard::Clipboard` 一 drop，选择所有权就没了——「复制图片到剪贴板」会静默变成什么都没复制。`engine_ops` 用 `CLIPBOARD_OWNER` 静态把实例持有到进程退出（Wayland 的 data-control 由合成器接管，不受影响）。`clipboard_smoke` 长期没抓到这条：它以前跑在用户的 Wayland 会话上（窗口根本没落到 Xvfb），读回的是合成器接管的那份数据。
- **无头冒烟必须钉住 X11 后端**：GPUI 选后端只看环境变量（`platform::guess_compositor()`）：`WAYLAND_DISPLAY` 非空 → Wayland，其次 `DISPLAY` → X11。而 `xvfb-run` 只准备 X 显示：在 Wayland 会话里跑冒烟时窗口会落到**用户真实桌面**，帧由真实合成器决定（被遮挡/最小化时不产帧），于是**渲染驱动的行为根本没被测**——渲染驱动的检查（如「对比窗格加载母版」）就因此同一份代码一会儿 13/13 一会儿 12/13（母版加载只在视图真的渲染时才发起；`compare_smoke` 当年就是这样抓出来的，该视图已移除）。所以 6 个 GPUI 冒烟都在 `gpui_kit::application()` 之前调 `photo_ui::app::prepare_headless_smoke()`（把 `WAYLAND_DISPLAY` 置空，GPUI 判空即视为未设置）；要真机 Wayland 目检时设 `PHOTO_SMOKE_ALLOW_WAYLAND=1`。
- **模型/名录库/全局索引库定位（`data_root()`，lib.rs）**：`PHOTO_DATA_DIR` env → exe 同级 `models/`+`data/`（打包便携）→ 仓库根（开发回退，从 CARGO_MANIFEST_DIR/cwd 向上找同时含 `models/` 与 `data/bird_catalog.db` 的目录）；`cargo run -p photo-ui` 下模型在仓库根，否则会报「YOLO 模型文件不存在: <target>/debug/models/detect.onnx」
- 使用了 let-chains（edition 2024 特性），如 `photo-config/config.rs` 便携路径判断
- **评分/旗标/色标筛选在 UI 侧执行**（`crates/photo-ui/src/model/filter.rs`）；`FilterCriteria::has_active_filter` 语义 = 批量操作安全边界（无筛选时禁用）
- **窗口根视图必须是 gpui-component 的 `Root`**：`Button::tooltip` 最终调 `Root::tooltip_overlay(window, cx)`，而它靠 `window.root::<Root>()` 查找——根视图不是 `Root`（例如直接把 `AppState` 当根视图）时**所有 tooltip 被静默丢弃**，表现就是「图标按钮悬停没有提示」（曾把整个 app 的 tooltip 都吃掉：侧边栏 5 个图标、Dock 折叠按钮、预览工具条）。`main.rs` 已用 `Root::new(app_state, window, cx).bordered(false)` 包一层（系统装饰窗口不要再叠自绘边框），无头冒烟 `clipboard_smoke` 会断言根视图是 `Root`。另注意 gpui 的 `InteractiveElement::disabled` 会屏蔽 hover，**disabled 按钮不显示 tooltip**（如未选中照片时的「识别」）

---

## 测试与 QA

- Rust：`#[test]` 分布在 5 个 crate 的源文件内联 `#[cfg(test)] mod tests`（domain 26 / config 10 / engine 134 / recognize 32 / photo-ui 51），另有 2 个 `#[ignore]` 真机冒烟
- 真机识别冒烟：`cargo test -p photo-recognize -- --ignored`（需 worktree/发布根有 `models/` 与 `data/bird_catalog.db`）；单文件手动识别工具：`cargo run -p photo-recognize --example recognize_file -- <图片路径>`
- 无头冒烟：`lease_smoke` 15 项 / `preview_full_smoke` 5 项 / `clipboard_smoke`（派发 CopyImage 动作 → 解码 → arboard 写剪贴板 → 读回校验尺寸）/ `adjust_smoke` 16 项（自建 1200×800 JPEG：预览装载参数 / 三条滑杆实体 / 面板可渲染 / 滑杆 Change 事件量化 0.37→0.35 / +1 EV 后预览更亮 149.0→202.9 / 350ms 去抖落库 / 原文件字节不变 / 重置回原图并复位）/ `grid_scroll_smoke` 4 项（自建 48 张 JPEG：网格滚动句柄拿到真实视口 / 内容高于视口 / 写句柄偏移跨帧保留；另支持 `PHOTO_SMOKE_HOLD_MS` 保持窗口供 Xvfb 外部截图目检 thumb）/ `recognize_smoke` 5 项（需 models/：扫描素材 / 识别途中能观察到中间进度 / 跑完 done==total 且每张有状态 / 取消 5s 内生效）/ `master_bench` 计时；6 个 GPUI 冒烟都先调 `app::prepare_headless_smoke()` 把后端钉在 Xvfb 的 X11 上（否则 Wayland 会话下窗口落到真实桌面、渲染帧不可控），`xvfb-run` 下无需真实显示器（见开发命令）

### 测试分布

|模块|测试数|覆盖内容|
|---|---|---|
|`photo-domain::domain.rs`|26|扩展名解析、RAW 白名单、enrich_with_xmp/recognition、ExifMetadata 摘要、XmpMetadata 枚举转换、BBox/RecognitionStatus/RecognitionFilter 序列化与状态映射、EyeSharpness 排序枚举、GPS DMS 转换|
|`photo-config::lib.rs`|10|默认值、TOML 保存/加载往返、配置路径、AppConfig 字段钳制（含 include_subdirectories、export_presets）|
|`photo-engine`|134 + 1 ignore|scanner 单层/递归、ops 移动/复制/重命名/删除（含 sidecar）、识别行同步、thumbnail 缓存键、exif 摘要、convert、folder_db 建表/迁移/upsert/rename 同步/多表清理、adjustments、global_db 索引/修正日志/命中率、histogram 直方图/剪切、import 分组/去重/复制移动、template 占位符渲染、undo 三类逆操作、keywords 表|
|`photo-recognize`|32 + 1 ignore|阶段→状态映射、输入源解析（JPEG/RAW）、检测框变换、softmax/Top-5、名录映射、进度回调、眼关键点、锐度融合单调性|
|`photo-ui`|51|筛选/排序/堆叠/连拍/预览数学/调整参数（量化、chip 文案、相对路径、字段操作）纯逻辑（`src/model/`）、Material You 色彩（`theme/`）、剪贴板解码|

---

## GPUI 前端参考

> 技术栈：Rust + GPUI（经 gpui-kit 0.6.1：Dock / 组件库 / Material You 主题）。**单进程、无 WebView、无 IPC、无 TS 绑定**。
> 逐节设计规范见 `docs/gui-design-manual.md`（GPUI 版手册）。

### 架构概述

- **状态中心**：`AppState` 是唯一的 GPUI 实体，各视图 `cx.observe(&app)` 后重绘；`state/engine_ops.rs` 是调同步引擎的唯一桥，没有 command/事件/IPC 那一层
- **工作区**：`views/dock_panels.rs` 用 gpui-kit Dock 组装左右停靠区与中央主视图；面板靠 `panel_name` 持久化，`set_locked(true)` 锁重排不锁拖宽；边缘宽度写回 `AppConfig.leftPanelWidth/rightPanelWidth`
- **主视图**：`ViewMode` 驱动 grid / preview / slideshow / stats；`views/` 下另有 filmstrip、filter_bar、info_panel、left_panel、header、status_bar、dialogs/
- **纯逻辑**：`model/`（adjust / filter / sort / stacks / burst / preview_math / best_frame）不依赖 GPUI，内联单测——筛选/排序/堆叠/连拍/预览数学/调整换算都在这里
- **图片**：`image/`（`ImageManager` 进程内解码 + 缓存：缩略图 → 2560 显示母版 → 1:1 全分辨率）；**复制到剪贴板**（Ctrl+C / 预览工具条「复制」）在 `state/engine_ops.rs` 解码全尺寸 RGBA 后走 `arboard`——GPUI 自带的剪贴板在 Linux 只写文本，图片项会被静默丢弃
- **交互**：`actions.rs` 定义 action，`app.rs` 注册 46 条 `KeyBinding` 并分发（评分/旗标/色标/识别/视图切换/幻灯片/缩放/剪贴板复制等）
- **主题**：`theme/`（Material You HCT 动态取色，seed 来自配置；`theme_config` → GPUI `ThemeConfig`）

### 日志（tracing 统一管道）

- `crates/photo-ui/src/logging.rs` 在 `main()` 安装全局订阅者，落盘滚动日切文件 `<配置目录>/logs/ftpt.YYYY-MM-DD.log`（Linux 即 `~/.config/pt/logs/`，与 `~/.config/pt/config.toml` 同级），保留 14 份；`PHOTO_LOG_LEVEL` 覆盖级别（debug 构建默认 DEBUG、release INFO）；stderr 同步输出（`cargo run -p photo-ui` 控制台可见）
- `tracing_log::LogTracer` 把 rawlib 等第三方 `log::*` 接入同一管道；`WorkerGuard` 由 `main` 持有到进程退出，否则非阻塞 writer 会丢缓冲日志
- 日志目录经 `logging::log_dir()` / `today_log_file()` 暴露（设置页展示与打开）

---

## 近期修复记录

- **2026-09-19 chore(photo-ui)：删掉顶栏那个标错名的「连拍」tab**：用户问「连拍 tab 是干啥的」——查清它是 GPUI 重写时的错标：pill id 是 `pill-view-slideshow`、派发 `Slideshow`、进 `ViewMode::Slideshow`，渲的是 `render_slideshow`（全屏单图 + 3s 自动播放），手册 §3.3/§5.6 与设置页快捷键都叫它「幻灯片」。按用户「删除」的要求移除该 pill（顶栏现在是 `网格 / 单张 / 统计`，正合手册 §9.1 的规格）。**幻灯片视图本身与 `S` 键保留**（入口变成只走键位），要整体删除该视图另说。验证：`cargo check -p photo-ui --examples` 无 warning；`grid_scroll_smoke` 通过；Xvfb 截图确认顶栏只剩三个 tab。
- **2026-09-19 chore(photo-ui)：按用户要求整体删除「对比」功能**：用户报「把对比删除」。删掉的是整套 CompareView 而不只是入口：`views/compare.rs` / `model/compare.rs` / `examples/compare_smoke.rs` 三个文件，`Compare` 与 `ToggleCompareAxis` 两个 action，`C` / `L` 两个键位（键位表 48 → 46），顶栏「对比」pill，`ViewMode::Compare` 变体与 `view_before_compare` / `compare_indices` / `compare_focused_slot` / `compare_axis` / `compare_images` / `compare_requested` 六个状态字段，`engine_ops::load_compare_master`，以及设置页的「连拍对比模式」快捷键条目、`model/tests.rs` 末尾 4 个对比用例与手册 §3.3 / §4.6 / §5.3 / §5.4 / §5.6 / §10.2 / §13 的对比规格。`mark_indices` 与 `Esc` 优先级链相应简化（对比分支删除后重新编号）。注意：删除前 `views/compare.rs` 有未提交改动、`model/compare.rs` 与 `compare_smoke.rs` 还是未跟踪文件，已按要求先备份到仓库外 `/tmp/pt-compare-backup/`（含 `views_compare.patch`）。验证：`cargo check -p photo-ui --examples` 无 warning、全仓再无 `ViewMode::Compare`/`compare_*` 残留标识符；`cargo test -p photo-ui --lib` 51 passed（原 54 减去 3 个对比用例）；`lease_smoke` / `adjust_smoke` / `grid_scroll_smoke` 全过。
- **2026-09-19 fix(photo-ui)：网格视图没有滚动条（用户报「网格视图，没有滚动条」）**：现象是网格能滚（滚轮生效）但右缘什么都不显示，长目录里完全不知道自己在第几屏。根因是 **GPUI 的溢出滚动容器只负责滚、不负责画**：`uniform_list` 自己只设 `overflow.y = Scroll`，滚动条是独立组件；`render_photo_grid` 既没 `track_scroll` 绑句柄也没渲染 `Scrollbar`，而 `theme/mod.rs` 里 `scrollbar_thumb` 那套令牌早就定义好了、只是没人消费。改法：`AppState` 新增 `grid_scroll: UniformListScrollHandle`（跨帧持有——滚动位置与滚动条同源），网格每帧 `uniform_list(...).track_scroll(&state.grid_scroll)` 并把 `Scrollbar::vertical(&state.grid_scroll)` 叠在 `relative()` 包装层的列表视口上（thumb 的 offset/长度由同一句柄推导，颜色与圆角走 gpui-component 主题投影）。验证：新增无头冒烟 `grid_scroll_smoke` 4 项（自建 48 张 JPEG：句柄拿到真实视口 595px / 内容 3952px 高于视口 / 写句柄偏移跨帧保留 -200px）；Xvfb 截图目检到右缘 8px 圆角 thumb（限高 82px 的有界灰条），改偏移后 thumb 的 y 从 186 移到 195；`cargo test -p photo-ui --lib` 54 passed，`lease_smoke` / `compare_smoke` / `adjust_smoke` 全过。已知同类遗留：统计页鸟种榜与导入弹窗内容用的是裸 `overflow_y_scroll()`（同样没有滚动条），胶片条按手册 §9.6 要自绘横向滚动条也还没做。
- **2026-09-19 fix(photo-ui)：调整面板整个是占位（用户报「调整面板没用」）**：现象是右栏「调整」tab 只有三根静态灰条——拖不动、没有数值、改不了图。根因是 GPUI 重写时只搬了骨架：`render_adjustments_tab` 的形参写成 `_state / _meta`（故意不用），全仓既没有调整参数状态，也没有 folder_db 读写与预览重算；engine 侧 `adjustments::apply_tone8/16`、`folder_db.adjustments`、ADR 0007 早就绪，只是没人调用。本次把 ADR 0007 的 v1（除导出）接回 GPUI：① 纯逻辑 `model/adjust.rs`（曝光 0.05 量化 / 对比度饱和度取整、chip 文案、相对路径键、`AdjustField`，10 单测）；② `ImageManager::render_adjusted_master` 取 2560 母版像素（内存缓存最近 2 张，拖动只重算色调 + 重编码，不重复解码原图）→ `apply_tone8` → JPEG，跑后台 executor、主线程零像素工作，中性参数短路回未调整母版；③ `AppState` 状态机——`ensure_adjustments_loaded`（面板与预览两个渲染入口都调用，幂等）切图先 flush 旧图、350ms 去抖写 `folder_db.adjustments`、`adjust_render_seq` 只认最新一帧、`Drop` 与换目录前强制 flush（换目录必须先 flush，否则相对路径会算到新目录）；④ 面板换成三条 gpui-component `Slider`（懒创建 `SliderState` + Change 事件订阅）+ 数值 chip + 单项重置 + 「全部重置」+「已调整」徽标。顺带堵两个竞态：未调整母版加载晚于调整预览完成时不覆盖它；有调整时 1:1 不切原图（原文件没有烘焙参数，否则调整效果静默消失）。已知遗留：有调整时 1:1 是放大母版（偏软）；RAW 调整走的是 8-bit 2560 母版（ADR 0007 设想的 half_size + 16-bit 母版未做，大范围拉曝光仍会条带）；全尺寸重算与「调整 tab 导出」未接线（导出弹窗本身也还是占位）。验证：`cargo test -p photo-ui --lib` 54 passed；新增无头冒烟 `adjust_smoke` 16 项（自建 JPEG：装载参数 → 滑杆事件量化 0.37→0.35 → +1 EV 后预览亮度 149.0→202.9 → 350ms 落库 → 原文件字节不变 → 重置回原图并复位）。
- **2026-09-19 fix(photo-ui)：X11 下「复制图片到剪贴板」其实什么都没复制**：给冒烟钉住 X11 后端之后 `clipboard_smoke` 立刻红了——`写剪贴板成功` 但读回为空。根因是 `write_image_to_system_clipboard` 每次都 `arboard::Clipboard::new()`、设完图片函数返回即 drop：**X11 的剪贴板内容由进程持有**（应答 X 请求时才提供数据），drop 就等于放弃选择所有权，于是复制静默失效。Wayland 的 data-control 由合成器接管，所以之前在桌面会话里用着是好的。改法：`engine_ops` 加 `static CLIPBOARD_OWNER: OnceLock<Mutex<Option<arboard::Clipboard>>>`，实例复用到进程退出（与多数 X11 应用一致）。验证：`clipboard_smoke` 在 Xvfb/X11 下 `CLIPBOARD_E2E_OK 64x48`。
- **2026-09-19 fix(冒烟)：无头冒烟在 Wayland 会话下没走 Xvfb，渲染驱动的检查会撒谎**：查「全部识别没有进度」时发现 `compare_smoke` 的「对比窗格加载的是 2560 母版」开始稳定失败（`requested = []`、`images = []`）——加 `eprintln` 确认 `render_compare_view` **一次都没被调用**。根因不在业务代码：GPUI 的 `platform::guess_compositor()` 只看环境变量，`WAYLAND_DISPLAY` 非空就选 Wayland，而 `xvfb-run` 只提供 X 显示——于是冒烟窗口落到用户真实桌面，帧由真实合成器决定（遮挡/最小化时不产帧），渲染驱动的行为（对比母版、预览母版）就随环境时好时坏：同一份代码 00:23/00:37（Wayland 合成器恰好产帧）13/13，01:00 起稳定 12/13；`env -u WAYLAND_DISPLAY` 跑立刻 13/13 且 `[cmp-render]` 打印 8 次。改法：新增 `photo_ui::app::prepare_headless_smoke()`（把 `WAYLAND_DISPLAY` 置空 → GPUI 判空即未设置 → 走 X11），5 个 GPUI 冒烟在 `application()` 前调用，`PHOTO_SMOKE_ALLOW_WAYLAND=1` 可跳过。顺带把 `compare_smoke` 的失败信息改成打印 `(thumb_len, master_len)` / requested / images，下次一眼能看出是没加载还是没渲染。验证：`lease_smoke` 15/15、`clipboard_smoke`、`compare_smoke` 13/13、`recognize_smoke` 5/5 全部在文档里的原命令（带 `WAYLAND_DISPLAY`）下通过。
- **2026-09-19 fix(photo-ui)：批量识别占住前台执行器 → 状态栏没有进度**：现象「点了全部识别看不出在跑」。根因是 `start_recognition` 的 `cx.spawn` 循环里**没有任何 `.await`**，而循环体是同步 ONNX 推理（每张几百 ms）：`recognize_done/recognize_current` 每张都在更新也 `cx.notify()` 了，但 GPUI 前台执行器（渲染 + 事件循环）整批被占住，帧根本没机会画——进度直到全部结束才闪一下，「取消识别」按钮同样点不动。改法：与缩略图后台管线同构——`Recognizer::new` + 逐张 `recognize` 移进 `background_executor().spawn`，结果经 `std::sync::mpsc` 回传；前台每 250ms `try_recv` 一批，**在前台**落 `items` 与 `folder_db`（SQLite 连接不跨线程）、推进 `recognize_done`、`cx.notify()`；后台跑完或被取消都补一个 `Finished` 哨兵，前台据此收尾（取消时状态栏文案区分「识别已取消：完成 n/m」）。状态栏识别分支补 4px 进度条与文件名截断。验证：新增无头冒烟 `recognize_smoke` 5 项——识别途中能观察到 `0 < done < total`（旧实现下必挂）/ 跑完 `done == total` 且每张都有识别状态 / 置取消标志后 5s 内停下。已知遗留：`recognitionThreadCount` 配置项仍未接线（并发 worker 未做）。
- **2026-09-19 fix(config)：配置目录终于认 `XDG_CONFIG_HOME`（顺带修 `lease_smoke` 假失败）**：`determine_config_path()` 原先硬编码 `~/.config/pt`，而所有无头冒烟的跑法都写着 `XDG_CONFIG_HOME=/tmp/... 隔离配置，不碰真实配置`——**那句注释是假的**，每次冒烟都在读写用户真实配置（我验证时也触发过它重写 `~/.config/pt/config.toml`）。后果之一就是 `lease_smoke` 的「左停靠区宽度来自配置」检查硬编码期望 `200`，而真实配置里 `leftPanelWidth = 324`（拖宽回写的）→ 稳定 14 过 1 挂；用 `HOME=/tmp/pt-fake-home` 跑则 15 过，证明与业务无关。改法：新增 `config_dir()`，优先级 `PHOTO_CONFIG_DIR`（值即目录）→ `XDG_CONFIG_HOME/pt` → `~/.config/pt`（Windows 保持 `%USERPROFILE%\.config\pt`，刻意不用 `dirs::config_dir()` 的 `%APPDATA%`），解析逻辑抽成纯函数 `config_dir_from()` 并覆盖 5 条分支单测；`lease_smoke` 的期望值改成直接读配置文件（`clamp(200,480)`），不再硬编码。验证：`cargo test -p photo-config` 18 passed；`lease_smoke` 真实配置 15 过、`XDG_CONFIG_HOME` 隔离跑 15 过且真实配置 mtime 不变。
- **2026-09-18 feat(photo-ui)：顶栏加「全部识别」按钮**：批量识别的入口此前只有键位（`B` 所选 / `Ctrl+B` 目录全部未识别 / `Ctrl+Shift+B` 强制重跑），鼠标用户看不见。顶栏右侧（扫描进度与刷新之间）加一个 secondary 实心按钮，直接 `dispatch_action(RecognizeAllUnrecognized)`——与 `Ctrl+B` 同一条动作路径，不复制识别逻辑。口径就是 `recognition_status == None` 的照片，label 直接带剩余张数（`全部识别 3`）；识别中显示「识别中…」禁用，没有未识别照片显示「全部已识别」禁用，无照片显示「全部识别」禁用。**gpui 的 disabled 会屏蔽 hover，禁用态没有 tooltip**，所以状态必须写进 label 本身。验证：`cargo test -p photo-ui` 44 passed；Xvfb 截图核对两种状态（「全部识别 3」实心可点 / 「全部已识别」灰掉）。
- **2026-09-18 fix(photo-ui)：对比入口静默失败（用户报「对比没用」）**（**该功能已于 2026-09-19 按用户要求整体删除，见顶部记录；本条仅作历史**）：现象是点顶栏「对比」/ 按 `C` 之后什么都没发生，用户根本不知道要 Ctrl 多选 2 张。根因不在对比视图本身——`app.rs` 的 Compare 监听器只在凑够 2 张时才切视图，凑不够时仅写一条 4 秒状态栏消息，而状态栏右侧被后台「缩略图 x/y」进度占着优先级（`status_bar.rs` 优先级 3 > 4），消息根本露不出来。改法：**凑不够 2 张也进对比视图**，中间显示引导卡片（现状说明 + 三条编号步骤 + 「返回网格去多选」按钮），状态栏消息保留。顺带修同一监听器里的下标错位：`burst_groups` 的键是「显示序位置」而旧代码拿 items 下标去查、又把查出来的位置当 items 下标渲染，只要排序变化或筛选收窄，连拍组对比就画**别的照片**；换算收进 `model/compare.rs::resolve_compare_indices` 并补 3 个单测。对比窗格同时接上 §7 的母版图源（原来用 440 网格缩略图，窗格一大就糊，母版未就绪先用缩略图占位、同尺寸排布无跳变）并改成两列（3–4 张走 2×2）。另加**对比方向切换**：顶栏「方向」分段控件或 `L` 键在「左右」/「上下」间切——左右先摊行、上下先摊列，2 张时即 1×2 / 2×1（横幅选上下、竖幅选左右最省空白），3–4 张两种方向都是 2×2 只是摊开顺序不同；`h_flex` 默认 `items_center`，跨轴显式 `items_stretch` 否则窗格会缩成内容高度；顶栏加 `flex_wrap`，中央区窄时「方向 + 退出」折到第二行（此前退出按钮直接被裁掉）。Xvfb 下截图逐帧核对过两种方向的窗格几何与顶栏。验证：`cargo test -p photo-ui` 44 passed；新增无头冒烟 `compare_smoke` 13 项全过。已知遗留：§4.6 的同步缩放/平移仍未接线（见手册 §13）
- **2026-09-18 chore(photo-ui)：单图预览图标换成 🖼（Lucide `image`）**：左活动栏「单图预览 (G)」与顶栏「单张」原来都用 `Frame`，渲染出来是个 `#`，跟旁边的「网格」图标分不清；改用语义直白的图片图标。顺带把这两个文件的 `IconName` 从 gpui-component 的精简集（~100 个）换成完整 Lucide 目录 `gpui_kit::assets::IconName`（photo-ui 已 `with_assets(AllAssets)`，1830 个图标全可用），不再受精简集限制
- **2026-09-18 chore(photo-ui)：删除左活动栏的「智能识别」图标**：没选中照片时它是 disabled（gpui 禁用态不显示 tooltip），图标语义也不直观——认知成本大于收益，直接删掉。识别入口保留 `B` 键与右栏「重新识别」。左栏现为：左栏显隐 / 打开照片目录 / 进入预览，底部主题切换；`docs/gui-design-manual.md` §9.2 同步
- **2026-09-18 fix(photo-ui)：所有图标按钮悬停没有 tooltip（侧边栏尤其明显）**：根因不在按钮本身——窗口根视图直接是 `AppState`，而 gpui-component 的 tooltip 挂在 `Root` 的 overlay 上（`Root::tooltip_overlay` 用 `window.root::<Root>()` 查找，查不到就静默 return），于是**全 app 的 `Button::tooltip` 从来没渲染过**。修法：`main.rs` 用 `Root::new(app_state, window, cx).bordered(false)` 包根视图。顺带补齐 14 个「只有图标、没有提示」的按钮：预览缩放 −/＋、幻灯片上一张/播放暂停/下一张、状态栏「取消识别」、筛选栏 5 个清除按钮、3 个弹窗关闭按钮。验证：Xvfb 下用 XTest 注入鼠标移动悬停左栏图标，截图出现「打开照片目录 (Ctrl+O)」；`clipboard_smoke` 断言 `window.root::<Root>()` 存在（tooltip 前提）且剪贴板通路仍通；`cargo check -p photo-ui --examples` 通过。已知遗留：disabled 按钮不显示 tooltip（gpui 的 `InteractiveElement::disabled` 屏蔽 hover）
- **2026-09-18 feat(photo-ui)：复制图片到系统剪贴板（Ctrl+C / 预览「复制」）**：补齐删除 Tauri 版后缺的 `copy_image_to_clipboard`。解码口径与原版一致——常规格式读原文件全尺寸解码，RAW 走 `ThumbnailCache::get_or_generate_full`（AHD 全尺寸 JPEG），原文件解不开（DNG/TIFF/HEIF）回退 full 母版——再转 RGBA8 交给 `arboard::set_image`。没用 GPUI 自带剪贴板：gpui-pre 的 X11/Wayland 后端只写文本，图片项会被静默丢弃（Windows/macOS 才认）。新增依赖 `arboard`（features `wayland-data-control`，Wayland 下走 wl-clipboard-rs）。解码放后台 executor（39MP RGBA ≈157MB，不冻 UI），写剪贴板回主线程，结果落状态栏。验证：`cargo test -p photo-ui` 41 passed（新增 2 个解码用例：常规格式走原文件路径、全失败时正确报错）；Xvfb 下 arboard 图片写→读回环 4×3 字节一致
- **2026-09-18 chore：删除 Tauri v2 版前端（crates/photo-tauri）**：GPUI 版（crates/photo-ui）已是唯一前端，删除 Tauri v2 + Vue 3 的前后端目录（167 个跟踪文件，含 node_modules 共 171MB）——根 `Cargo.toml` 移除只被它使用的 workspace 依赖（tauri / tauri-build / tauri-plugin-dialog·opener·clipboard-manager·single-instance / tauri-specta / specta-typescript / embed-resource / font-kit / rust-embed），domain/config/recognize 上可选的 `specta` feature 保留（默认不启用、无消费方）；`.gitignore` 去掉 tauri target 项；README / AGENTS / CONTEXT 改为描述 GPUI 版。photo-tauri 此前已不在 workspace members 里，删除不影响 `cargo build`。验证：`cargo metadata` 成员 6 个、`cargo check -p photo-ui` 通过、`Cargo.lock` 无变化
- **2026-09-18 feat(photo-ui 预览)：1:1 接上全分辨率图源（修「预览糊」）**：现象「单张预览发糊」经实测定位为**图源选择**而非解码质量——统一缩到 812px 显示尺寸下，派生 2560 母版 lapVar 165.6 与 AHD 全尺寸 171.3 基本无差（fit 视图不糊），但 1:1 时 `one_to_one = zoom==0 && !is_raw` 对 RAW 恒 false，显示框按 EXIF 自然尺寸（8152×5432）排布、画的却是 2560 母版 → **放大 3.18 倍**，lapVar 4.3 vs 真 1:1 的 48.7（糊 11 倍）；母版未就绪时显示的 440px 网格缩略图占位 lapVar 35.0 是另一个糊源。photo-tauri 的 `ptimg://full` 早有全分辨率路径（`ThumbnailCache::get_or_generate_full`：AHD 全尺寸 + `full` 变体落盘缓存），photo-ui 未接线。本次补上：`ImageManager::load_full_image`（内存只留最后一张，44MP 全尺寸 RGBA ≈177MB）+ `engine_ops::load_preview_full` + AppState `preview_full/preview_full_request`；判据抽成纯函数 `preview_math::exceeds_master_res`（显示长边 > 母版 2560 → 换真原图，1:1 与高倍放大同一条路）。实测 P1082800.RW2：全尺寸冷启 1.97s / 内存命中 9µs（8152×5432）。验证：`cargo test -p photo-ui`（38，含边界用例）、`lease_smoke` 15 项、新增无头冒烟 `preview_full_smoke` 5 项（fit 不加载 / 1:1 加载 / 路径不串 / 字节量级）、`master_bench` 增加 1:1 档计时
- **2026-09-18 fix(RAW 预览偏暗)：显示母版重新开启自动亮度 + 内嵌快路径修复**：事故现象「RAW 预览比机内 JPEG 明显偏暗」根因不在白平衡——`rawlib::DecodeOptions::preview()` 为省一趟直方图扫描把 `no_auto_bright` 置 true，而 `full()`/`preview16()` 均为 false，导致 fit 母版与 1:1/调整页曝光不一致。实测 P1082800.RW2（Lumix 47MP RW2）：机内 JPG luma 76.6 / 内嵌 JPEG 75.9 / 应用母版 **48.1** / full() 93.9；白平衡 R/G=1.17 B/G=0.966 与内嵌 JPEG 1.21/0.969 基本一致（关相机 WB 则 B/G=0.65 明显偏黄），故只改曝光口径：新增 `display_preview_options()`（half_size+bilinear+sRGB+相机 WB+**自动亮度**），`decode_raw_impl` 改用它 → 母版 luma 93.8≈full() 93.9。另修死代码快路径：LibRaw 的 `libraw_processed_image_t` 对 JPEG 类型**不填 width/height（恒 0）**，原判据 `thumb.width.max(thumb.height) >= 2048` 永不成立，「大内嵌直接当母版」从未生效（每张 RAW 预览都白付完整解码），改由 `embedded_long_edge()` 解 JPEG 头取真实长边；`CACHE_VERSION` 4→5 让陈旧偏暗母版失效。手动诊断工具 `cargo run --release -p photo-engine --example raw_brightness_check -- <RAW>`
- **2026-09-18 photo-ui：左右边栏改 Dock + 导入流程补齐**：`crates/photo-ui` 工作区从自绘三栏改为 gpui-kit Dock（`DockArea` + `DockSkin`，新增 `views/dock_panels.rs`）——左停靠区（文件树/批量操作）、右停靠区（信息/调整）、中央主视图；边缘可拖宽、Ctrl+[ / Ctrl+] 折叠、宽度 350ms 去抖写回 `AppConfig.leftPanelWidth/rightPanelWidth`，`set_locked(true)` 锁重排不锁拖宽。导入弹窗从占位实现补齐（新增 `state/import.rs`）：可移动盘/浏览选源 → 扫描 → 目标根目录 + 4 种子目录模式 + 重命名模板 → 复制/移动 → 干跑计划预览 → 进度 → 全成功自动关闭并重扫结果目录。无头冒烟 `XDG_CONFIG_HOME=/tmp/ptlease-config xvfb-run -a cargo run -p photo-ui --example lease_smoke` 15 项全过；修复 7 个弹窗的滚动穿透（遮罩缺 id + occlude，滚轮会落到主视图）

- **2026-08-12 堆叠显示改造（A+E）**：网格堆叠从「×N 徽标循环点击」改为 cell 底部成员缩略图带（点击直达激活+选中，长连拍横向滚动），新增语义徽标区分同画面多格式（Copy 蓝）/连拍多帧（Layers 橙），连拍徽标仅在单成员组显示避免重复；新增 Q/E 组内切换激活成员（网格态）；修复 `openPath` 同目录早退导致目录为空（mock 无后端自动扫描/启动自愈后扫描失败）时无法重扫的死路
- **2026-08-10 迁移 wave 1-3**：Tauri v2 迁移（计划见 git 历史 `docs/tauri-migration-plan.md`）；specta 真实绑定导出（bin 绕开 harness 0xc0000139）；启动自愈（自动恢复目录事件早于挂载）；主题默认 Light；mock 层 batchOpExecute detached `this` 修复
- 历史（旧版时代，引擎层均保留）：copy_recognitions_to 索引错位修复、批量操作 ADR 0006 重构（筛选驱动）、全分辨率 DCT 降采样、RAW 母版缓存、Worker panic 兜底（前端 store 状态机继承）、OTHER 格式徽标、调整功能 ADR 0007（无 crop）
