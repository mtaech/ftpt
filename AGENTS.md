# Repository Guidelines — Photo Tool

## 项目概述

Photo Tool 是一个**照片管理与筛选（culling）**应用，用于浏览、标记、识别和转换照片。Cargo workspace 含 6 个成员：

- `photo-domain` — 纯类型叶子 crate（Capture, ExifMetadata, XmpMetadata, Recognition 类型, 枚举），依赖仅 serde + chrono
- `photo-engine` — 文件操作引擎（scanner, ops, exif, thumbnail, convert, folder_db, batch_ops, global_db 跨文件夹鸟种索引, histogram 直方图/剪切, import SD 卡导入, template 命名模板, undo 批量撤销日志），**全同步**
- `photo-recognize` — 鸟类识别管线（YOLO 检测 → 鸟种分类 → 名录映射 → 鸟眼锐度，ONNX Runtime），**全同步**
- `photo-config` — 配置读写（TOML + SQLite 持久化）
- `photo-ui` — **GPUI 桌面前端**（gpui-kit 0.6.1：Dock 三栏 + 网格/预览/对比/幻灯片/统计 + 导入/设置，Material You 主题），2026-09 起是唯一前端（原 Tauri v2 + Vue 3 版 `crates/photo-tauri` 已删除）
- `rawlib` — LibRaw 的 Rust 封装（第三方 crate，本地 path 依赖）

核心工作流：**目录扫描（单层/递归可配）→ 浏览/标记/筛选 → 鸟类识别（单张/批量）→ 文件操作（删除/移动/复制/重命名，可撤销）→ 格式转换/导出预设**；另含 **SD 卡导入、全局鸟种统计、连拍对比、幻灯片**。历史迁移（Tauri v2 → GPUI 重写）见 git 历史与 `docs/gui-design-manual.md`（GPUI 版设计手册）。

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
- **前端**（`crates/photo-ui/src/`）：GPUI 单进程应用——`app.rs` 组装 Dock 三栏（左：文件树/批量操作；中：网格/预览/对比/幻灯片/统计；右：信息/调整）、注册键位；`actions.rs` 定义 action；`views/` 出视图；`state/`（`AppState` / `import` / `engine_ops` 引擎桥）；`model/`（筛选/排序/堆叠/连拍/预览数学等纯逻辑）；`image/`（`ImageManager` 进程内解码）
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
|`crates/photo-ui/src/views/`|grid / preview / compare / slideshow / stats / filmstrip / info_panel / left_panel / filter_bar / header / status_bar / dialogs/|
|`crates/photo-ui/src/state/`|`AppState`（单实体）/ `import.rs`（导入状态机）/ `engine_ops.rs`（调同步引擎的唯一桥）|
|`crates/photo-ui/src/model/`|filter / sort / stacks / burst / preview_math / best_frame（纯逻辑 + 内联单测）|
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
|无头冒烟|`XDG_CONFIG_HOME=/tmp/ptlease-config xvfb-run -a cargo run -p photo-ui --example lease_smoke`（另有 `preview_full_smoke`、`master_bench`）|
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
- **模型/名录库/全局索引库定位（`data_root()`，lib.rs）**：`PHOTO_DATA_DIR` env → exe 同级 `models/`+`data/`（打包便携）→ 仓库根（开发回退，从 CARGO_MANIFEST_DIR/cwd 向上找同时含 `models/` 与 `data/bird_catalog.db` 的目录）；`cargo run -p photo-ui` 下模型在仓库根，否则会报「YOLO 模型文件不存在: <target>/debug/models/detect.onnx」
- 使用了 let-chains（edition 2024 特性），如 `photo-config/config.rs` 便携路径判断
- **评分/旗标/色标筛选在 UI 侧执行**（`crates/photo-ui/src/model/filter.rs`）；`FilterCriteria::has_active_filter` 语义 = 批量操作安全边界（无筛选时禁用）

---

## 测试与 QA

- Rust：`#[test]` 分布在 5 个 crate 的源文件内联 `#[cfg(test)] mod tests`（domain 26 / config 10 / engine 134 / recognize 32 / photo-ui 约 39），另有 2 个 `#[ignore]` 真机冒烟
- 真机识别冒烟：`cargo test -p photo-recognize -- --ignored`（需 worktree/发布根有 `models/` 与 `data/bird_catalog.db`）；单文件手动识别工具：`cargo run -p photo-recognize --example recognize_file -- <图片路径>`
- 无头冒烟：`lease_smoke` 15 项 / `preview_full_smoke` 5 项 / `master_bench` 计时，`xvfb-run` 下无需真实显示器（见开发命令）

### 测试分布

|模块|测试数|覆盖内容|
|---|---|---|
|`photo-domain::domain.rs`|26|扩展名解析、RAW 白名单、enrich_with_xmp/recognition、ExifMetadata 摘要、XmpMetadata 枚举转换、BBox/RecognitionStatus/RecognitionFilter 序列化与状态映射、EyeSharpness 排序枚举、GPS DMS 转换|
|`photo-config::lib.rs`|10|默认值、TOML 保存/加载往返、配置路径、AppConfig 字段钳制（含 include_subdirectories、export_presets）|
|`photo-engine`|134 + 1 ignore|scanner 单层/递归、ops 移动/复制/重命名/删除（含 sidecar）、识别行同步、thumbnail 缓存键、exif 摘要、convert、folder_db 建表/迁移/upsert/rename 同步/多表清理、adjustments、global_db 索引/修正日志/命中率、histogram 直方图/剪切、import 分组/去重/复制移动、template 占位符渲染、undo 三类逆操作、keywords 表|
|`photo-recognize`|32 + 1 ignore|阶段→状态映射、输入源解析（JPEG/RAW）、检测框变换、softmax/Top-5、名录映射、进度回调、眼关键点、锐度融合单调性|
|`photo-ui`|约 39|筛选/排序/堆叠/连拍/预览数学纯逻辑（`src/model/`）、Material You 色彩（`theme/`）|

---

## GPUI 前端参考

> 技术栈：Rust + GPUI（经 gpui-kit 0.6.1：Dock / 组件库 / Material You 主题）。**单进程、无 WebView、无 IPC、无 TS 绑定**。
> 逐节设计规范见 `docs/gui-design-manual.md`（GPUI 版手册）。

### 架构概述

- **状态中心**：`AppState` 是唯一的 GPUI 实体，各视图 `cx.observe(&app)` 后重绘；`state/engine_ops.rs` 是调同步引擎的唯一桥，没有 command/事件/IPC 那一层
- **工作区**：`views/dock_panels.rs` 用 gpui-kit Dock 组装左右停靠区与中央主视图；面板靠 `panel_name` 持久化，`set_locked(true)` 锁重排不锁拖宽；边缘宽度写回 `AppConfig.leftPanelWidth/rightPanelWidth`
- **主视图**：`ViewMode` 驱动 grid / preview / compare / slideshow / stats；`views/` 下另有 filmstrip、filter_bar、info_panel、left_panel、header、status_bar、dialogs/
- **纯逻辑**：`model/`（filter / sort / stacks / burst / preview_math / best_frame）不依赖 GPUI，内联单测——筛选/排序/堆叠/连拍/预览数学都在这里
- **图片**：`image/`（`ImageManager` 进程内解码 + 缓存：缩略图 → 2560 显示母版 → 1:1 全分辨率）
- **交互**：`actions.rs` 定义 action，`app.rs` 注册 46 条 `KeyBinding` 并分发（评分/旗标/色标/识别/视图切换/对比/幻灯片/缩放等）
- **主题**：`theme/`（Material You HCT 动态取色，seed 来自配置；`theme_config` → GPUI `ThemeConfig`）

### 日志（tracing 统一管道）

- `crates/photo-ui/src/logging.rs` 在 `main()` 安装全局订阅者，落盘滚动日切文件 `<配置目录>/logs/ftpt.YYYY-MM-DD.log`（Linux 即 `~/.config/pt/logs/`，与 `~/.config/pt/config.toml` 同级），保留 14 份；`PHOTO_LOG_LEVEL` 覆盖级别（debug 构建默认 DEBUG、release INFO）；stderr 同步输出（`cargo run -p photo-ui` 控制台可见）
- `tracing_log::LogTracer` 把 rawlib 等第三方 `log::*` 接入同一管道；`WorkerGuard` 由 `main` 持有到进程退出，否则非阻塞 writer 会丢缓冲日志
- 日志目录经 `logging::log_dir()` / `today_log_file()` 暴露（设置页展示与打开）

---

## 近期修复记录

- **2026-09-18 chore：删除 Tauri v2 版前端（crates/photo-tauri）**：GPUI 版（crates/photo-ui）已是唯一前端，删除 Tauri v2 + Vue 3 的前后端目录（167 个跟踪文件，含 node_modules 共 171MB）——根 `Cargo.toml` 移除只被它使用的 workspace 依赖（tauri / tauri-build / tauri-plugin-dialog·opener·clipboard-manager·single-instance / tauri-specta / specta-typescript / embed-resource / font-kit / rust-embed），domain/config/recognize 上可选的 `specta` feature 保留（默认不启用、无消费方）；`.gitignore` 去掉 tauri target 项；README / AGENTS / CONTEXT 改为描述 GPUI 版。photo-tauri 此前已不在 workspace members 里，删除不影响 `cargo build`。验证：`cargo metadata` 成员 6 个、`cargo check -p photo-ui` 通过、`Cargo.lock` 无变化
- **2026-09-18 feat(photo-ui 预览)：1:1 接上全分辨率图源（修「预览糊」）**：现象「单张预览发糊」经实测定位为**图源选择**而非解码质量——统一缩到 812px 显示尺寸下，派生 2560 母版 lapVar 165.6 与 AHD 全尺寸 171.3 基本无差（fit 视图不糊），但 1:1 时 `one_to_one = zoom==0 && !is_raw` 对 RAW 恒 false，显示框按 EXIF 自然尺寸（8152×5432）排布、画的却是 2560 母版 → **放大 3.18 倍**，lapVar 4.3 vs 真 1:1 的 48.7（糊 11 倍）；母版未就绪时显示的 440px 网格缩略图占位 lapVar 35.0 是另一个糊源。photo-tauri 的 `ptimg://full` 早有全分辨率路径（`ThumbnailCache::get_or_generate_full`：AHD 全尺寸 + `full` 变体落盘缓存），photo-ui 未接线。本次补上：`ImageManager::load_full_image`（内存只留最后一张，44MP 全尺寸 RGBA ≈177MB）+ `engine_ops::load_preview_full` + AppState `preview_full/preview_full_request`；判据抽成纯函数 `preview_math::exceeds_master_res`（显示长边 > 母版 2560 → 换真原图，1:1 与高倍放大同一条路）。实测 P1082800.RW2：全尺寸冷启 1.97s / 内存命中 9µs（8152×5432）。验证：`cargo test -p photo-ui`（38，含边界用例）、`lease_smoke` 15 项、新增无头冒烟 `preview_full_smoke` 5 项（fit 不加载 / 1:1 加载 / 路径不串 / 字节量级）、`master_bench` 增加 1:1 档计时
- **2026-09-18 fix(RAW 预览偏暗)：显示母版重新开启自动亮度 + 内嵌快路径修复**：事故现象「RAW 预览比机内 JPEG 明显偏暗」根因不在白平衡——`rawlib::DecodeOptions::preview()` 为省一趟直方图扫描把 `no_auto_bright` 置 true，而 `full()`/`preview16()` 均为 false，导致 fit 母版与 1:1/调整页曝光不一致。实测 P1082800.RW2（Lumix 47MP RW2）：机内 JPG luma 76.6 / 内嵌 JPEG 75.9 / 应用母版 **48.1** / full() 93.9；白平衡 R/G=1.17 B/G=0.966 与内嵌 JPEG 1.21/0.969 基本一致（关相机 WB 则 B/G=0.65 明显偏黄），故只改曝光口径：新增 `display_preview_options()`（half_size+bilinear+sRGB+相机 WB+**自动亮度**），`decode_raw_impl` 改用它 → 母版 luma 93.8≈full() 93.9。另修死代码快路径：LibRaw 的 `libraw_processed_image_t` 对 JPEG 类型**不填 width/height（恒 0）**，原判据 `thumb.width.max(thumb.height) >= 2048` 永不成立，「大内嵌直接当母版」从未生效（每张 RAW 预览都白付完整解码），改由 `embedded_long_edge()` 解 JPEG 头取真实长边；`CACHE_VERSION` 4→5 让陈旧偏暗母版失效。手动诊断工具 `cargo run --release -p photo-engine --example raw_brightness_check -- <RAW>`
- **2026-09-18 photo-ui：左右边栏改 Dock + 导入流程补齐**：`crates/photo-ui` 工作区从自绘三栏改为 gpui-kit Dock（`DockArea` + `DockSkin`，新增 `views/dock_panels.rs`）——左停靠区（文件树/批量操作）、右停靠区（信息/调整）、中央主视图；边缘可拖宽、Ctrl+[ / Ctrl+] 折叠、宽度 350ms 去抖写回 `AppConfig.leftPanelWidth/rightPanelWidth`，`set_locked(true)` 锁重排不锁拖宽。导入弹窗从占位实现补齐（新增 `state/import.rs`）：可移动盘/浏览选源 → 扫描 → 目标根目录 + 4 种子目录模式 + 重命名模板 → 复制/移动 → 干跑计划预览 → 进度 → 全成功自动关闭并重扫结果目录。无头冒烟 `XDG_CONFIG_HOME=/tmp/ptlease-config xvfb-run -a cargo run -p photo-ui --example lease_smoke` 15 项全过；修复 7 个弹窗的滚动穿透（遮罩缺 id + occlude，滚轮会落到主视图）

- **2026-08-12 堆叠显示改造（A+E）**：网格堆叠从「×N 徽标循环点击」改为 cell 底部成员缩略图带（点击直达激活+选中，长连拍横向滚动），新增语义徽标区分同画面多格式（Copy 蓝）/连拍多帧（Layers 橙），连拍徽标仅在单成员组显示避免重复；新增 Q/E 组内切换激活成员（网格态）；修复 `openPath` 同目录早退导致目录为空（mock 无后端自动扫描/启动自愈后扫描失败）时无法重扫的死路
- **2026-08-10 迁移 wave 1-3**：Tauri v2 迁移（计划见 git 历史 `docs/tauri-migration-plan.md`）；specta 真实绑定导出（bin 绕开 harness 0xc0000139）；启动自愈（自动恢复目录事件早于挂载）；主题默认 Light；mock 层 batchOpExecute detached `this` 修复
- 历史（旧版时代，引擎层均保留）：copy_recognitions_to 索引错位修复、批量操作 ADR 0006 重构（筛选驱动）、全分辨率 DCT 降采样、RAW 母版缓存、Worker panic 兜底（前端 store 状态机继承）、OTHER 格式徽标、调整功能 ADR 0007（无 crop）
