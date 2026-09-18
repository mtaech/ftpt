# Repository Guidelines — Photo Tool

## 项目概述

Photo Tool 是一个**照片管理与筛选（culling）**应用，用于浏览、标记、识别和转换照片。Cargo workspace 含 5 个成员：

- `photo-domain` — 纯类型叶子 crate（Capture, ExifMetadata, XmpMetadata, Recognition 类型, 枚举），依赖仅 serde + chrono
- `photo-engine` — 文件操作引擎（scanner, ops, exif, thumbnail, convert, folder_db, batch_ops, global_db 跨文件夹鸟种索引, histogram 直方图/剪切, import SD 卡导入, template 命名模板, undo 批量撤销日志），**全同步**
- `photo-recognize` — 鸟类识别管线（YOLO 检测 → 鸟种分类 → 名录映射 → 鸟眼锐度，ONNX Runtime），**全同步**
- `photo-config` — 配置读写（TOML + SQLite 持久化）
- `photo-tauri` — **Tauri v2 前端**（Vue 3 + Pinia + Tailwind v4 + shadcn-vue），2026-08 迁移完成

核心工作流：**目录扫描（单层/递归可配）→ 浏览/标记/筛选 → 鸟类识别（单张/批量）→ 文件操作（删除/移动/复制/重命名，可撤销）→ 格式转换/导出预设**；另含 **SD 卡导入、全局鸟种统计、连拍对比、幻灯片**。迁移决策与功能清单记录于 git 历史 `docs/tauri-migration-plan.md`（Phase 1–3 完成，Phase 4 打包/验收收尾中）。

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
     └──────────────┘         │        ┌──────────────┐
     所有模块全同步            └────────│ photo-tauri  │
                            依赖      │ src-tauri/    │
                                      │ (Rust 后端)   │
                                      └──────┬───────┘
                                             │ IPC（invoke + 事件）
                                      ┌──────┴───────┐
                                      │ crates/      │
                                      │ photo-tauri/ │
                                      │ (Vue3 前端)  │
                                      └──────────────┘
```

- `photo-tauri` 依赖其余四者；`photo-recognize` 依赖 `photo-domain`（RAW 输入解码调用 `photo_engine::thumbnail::decode_raw_preview`）
- **Rust 后端**（`crates/photo-tauri/src-tauri/src/lib.rs`）：39 个 commands（`#[tauri::command]` + `#[specta::specta]`，注册于 `specta_builder`）+ 12 个事件（`app.emit` 明文通道，含 import/export 进度）+ `ptimg://` 自定义协议（缩略图/预览母版/1:1 全尺寸三路流式 serve，不走 IPC base64）
- **前端**（`crates/photo-tauri/src/`）：Vue 3 + Pinia stores（captures/selection/preview/filter/recognition/batch/config/contextMenu）+ keymap.ts（全键位 action 分发）+ 组件（App 三栏布局 / PhotoGrid 虚拟网格 / PhotoPreview / Filmstrip / FilterBar / InfoPanel 双 tab / Sidebar / StatusBar / BatchOpsPanel / SettingsModal / ContextMenu）
- **类型共享**：specta + tauri-specta 从 serde 类型生成 TS 绑定 → `crates/photo-tauri/src/lib/bindings.ts`（由 `cargo run -p photo-tauri --bin export_bindings` 生成；Rust 侧 `specta_builder` 需保持 pub）

### 核心数据流

1. **scanner** → `Vec<Capture>`：walkdir 单层（`max_depth(1)`）扫描，**每个图片文件一个 Capture**（扫描模型不配对）；网格**显示层**按配置堆叠模式分组（`src/lib/stacks.ts`：ByTime 同组照片堆叠按拍摄时间 ≤2s 聚类、ByFileName 同 stem 合并、None 关闭；主格式默认 JPEG 优先，堆叠组底部成员缩略图带点击直达激活格式、语义徽标区分同画面多格式/连拍，方向键在堆叠组间导航、Q/E 组内切换）；扫描期不做任何筛选，识别状态等元数据在扫描后才从 folder_db 读取，全部筛选由前端 `src/lib/filter.ts` 在 CaptureMeta 层面执行
2. **Capture** → **exif**：提取 EXIF（统一走 `ExifProvider` 抽象：**exiftool 主后端**（`-stay_open` 长驻进程 + JSON，覆盖 JPEG/RAW 及厂商私有对焦点），**rawlib 回退后端**（RAW 专用，Fuji FocusPixel / Nikon AFInfo blob 本地解析）；kamadak-exif 已移除）；`CaptureMeta::enrich_with_exif` 回填摘要（类型 `ExifMetadata` 定义在 domain，提取机械在 engine）
3. **Capture** → **ops**：删除（回收站/永久）/移动（跨设备 copy+delete 回退）/复制/批量重命名
4. **SourceFile** → **thumbnail**：磁盘缓存 JPEG 字节（缓存键 = `DefaultHasher(path+size)` 的 `{:016x}.jpg`，目录 = 照片目录 `.pt/thumbs`，每文件夹独立）；RAW 完整解码（half_size 预览选项）母版按 `u32::MAX` 键存一份，网格缩略图/预览/全分辨率均从母版 DCT 派生（不落盘）；内嵌 JPEG 长边 ≥2048（RW2/DNG 大内嵌）时直接用作母版省解码；常规图优先 EXIF 内嵌缩略图
5. **Capture** → **convert**：RAW 内嵌预览→JPEG、常规图缩放（Lanczos3）；`export_adjusted` 全尺寸烘焙（调整参数渲染）
6. **Capture** → **recognize**：`photo-recognize` 管线（YOLO 检测鸟体 → 整图 eye.onnx 检测眼（双槽一致性选点，CPU 推理）→ bird_model 分类 Top-5 → `sp_cls_map` JOIN `animal_info` 名录映射）→ `Recognition` 三态（Confirmed/NeedsReview/Unrecognized）+ 连续鸟眼锐度分（NULL 兜底，不影响三态）→ `folder_db` upsert 到文件夹级 `.pt/data.db`
7. **import**：检测可移动设备（Windows GetDriveTypeW FFI；Linux 解析 /proc/mounts + /sys/class/block removable；其余平台退化手动选源）→ 递归扫描源（只 stat 取文件修改时间，不读 EXIF）→ 按文件日期建 YYYY-MM-DD 子目录 → 去重（同名同大小跳过）→ 委托 **ops** 移动/复制；`DriveInfo.path` 跨平台统一为根路径（Windows 盘符根 / Linux 挂载点）

### 模块依赖关系

- `photo-tauri` 依赖其余四者；`photo-engine` 依赖 `photo-domain`（单向 DAG，由 crate 边界强制）；`photo-recognize` 依赖 `photo-domain`；`photo-config` 独立；`domain` 是纯叶子

---

## 关键目录

|---|---|
|`crates/photo-domain/src/domain.rs`|纯类型（Capture, ExifMetadata, XmpMetadata, 枚举），零外部 crate 依赖；`specta::Type` derive 经 feature 门控（`cfg_attr(feature = "specta", ...)`）|
|`crates/photo-engine/src/`|文件机械：scanner, ops, exif, thumbnail, convert, folder_db, batch_ops, adjustments（全部同步）|
|`crates/photo-engine/src/folder_db.rs`|文件夹级 SQLite（`.pt/data.db`）：exif_cache / xmp_meta / recognition / adjustments 四表，rusqlite_migration 版本化|
|`crates/photo-recognize/src/`|识别管线：lib.rs(Recognizer 门面), detect(YOLO), classify(bird_model), catalog(名录映射), pipeline, eye, sharpness|
|`crates/photo-config/src/lib.rs`|配置读写（TOML + SQLite 持久化）；AppConfig 含 favorite_dirs/recent_directories/theme(默认 Light)/leftPanelWidth/rightPanelWidth/thumbnailSize/recognitionThreadCount/stackMode(默认 None 不堆叠) 等|
|`crates/photo-tauri/src-tauri/src/lib.rs`|Tauri 后端：24 commands + 8 事件 + ptimg 协议 + 启动（配置便携优先 + 模型预热 + 上次目录自动恢复）|
|`crates/photo-tauri/src-tauri/src/bin/export_bindings.rs`|specta TS 绑定导出（生成前端 bindings.ts）|
|`crates/photo-tauri/src/stores/`|Pinia：captures（扫描/标记/事件接线）/ selection（多选锚点语义）/ preview（缩放平移/检测框/框选）/ filter（筛选排序）/ recognition（批量识别状态机）/ batch（批量操作两阶段）/ config（设置）/ contextMenu|
|`crates/photo-tauri/src/lib/`|bindings.ts（specta 生成）/ ipc.ts（typed invoke 薄封装 + 事件）/ filter.ts（filter.rs 纯移植 + vitest）/ previewMath.ts（preview_math.rs 移植）/ mock.ts（浏览器 mock 模式）/ format.ts|
|`docs/exiftool-update.md`|ExifTool 本地运行时更新指引（EXIF 后端依赖，进 git）|
|`local-lib/`|预编译 Linux `libraw.so`/`libraw_r.so` + `exiftool/`（ExifTool 跨平台运行时：`windows/` perl.exe+exiftool.pl、`linux/` 源码包、VERSION.txt；更新指引 `docs/exiftool-update.md`，均不纳入版本控制）|

---

## 开发命令

|操作|命令|
|---|---|
|Rust 全量构建|`cargo build`|
|Tauri 后端检查|`cargo check -p photo-tauri`|
|运行核心测试|`cargo test -p photo-engine -p photo-recognize -p photo-domain -p photo-config`|
|前端类型检查|`cd crates/photo-tauri && npx vue-tsc -b --noEmit`（**必须带 -b**：solution-style tsconfig 下不带 -b 不检查任何文件）|
|前端单测|`cd crates/photo-tauri && npx vitest run`（filter.ts 22 用例）|
|前端生产构建|`cd crates/photo-tauri && npm run build`|
|开发运行（tauri dev）|`cd crates/photo-tauri && npm run tauri dev`（vite 1420 端口 + WebView2）|
|生成 TS 绑定|`cargo run -p photo-tauri --bin export_bindings`（覆盖 bindings.ts；specta 不导出的类型手写追加段需手动保留）|
|浏览器 mock 模式|`cd crates/photo-tauri && npm run dev` + 浏览器开 localhost:1420（无 `__TAURI_INTERNALS__` 走 mock 数据流）|
|EXIF 提取验证|`cargo run -p photo-engine --example focus_check -- <图片>`（打印 ExifMetadata + 对焦点；exiftool 不可用或残留进程时先 `taskkill //F //IM perl.exe`）|

**注意**：`cargo test -p photo-tauri` 在本机崩溃（lib test harness 0xc0000139，环境问题；`ftpt` bin 正常）——specta 导出用 bin 而非测试。

---

## 代码规范与常见模式

### 模块组织

- `photo-domain/src/lib.rs` 声明 `pub mod domain` + `pub use domain::*`（re-export 让消费者直接 `photo_domain::Capture`）
- `photo-engine/src/lib.rs` 声明 `pub mod`（scanner, ops, exif, thumbnail, convert, folder_db, batch_ops, adjustments）；XMP 读写实现在 folder_db 的 xmp_meta 表，无独立 xmp.rs
- `photo-config/src/lib.rs` 即库根——config 模块就是 lib.rs 本身
- `photo-tauri` 后端：`src-tauri/src/lib.rs` 单文件组织（事件负载 → AppState → commands → 扫描编排 → ptimg handler → run/specta_builder → 导出测试）；前端组件在 `src/components/`，store 在 `src/stores/`
- 消费者写全路径：`photo_engine::scanner::scan_directory`

### 错误处理

- 每模块一个 `thiserror::Error` 枚举（`ConfigError`/`ScanError`/`OpError`/`ThumbnailError`/`ExifError`/`XmpError`/`ConvertError`/`FolderDbError`/`RecognizeError`），均以 `Io(#[from] std::io::Error)` 起步；外部错误多数 `#[from]`，rawlib/exiftool 错误转成 `String` 变体
- 批量操作返回 `Vec<(PathBuf, Result<(), Error>)>`，逐文件报告
- Tauri commands 返回 `Result<T, String>`，前端经 `unwrap`/`unwrapVoid`（ipc.ts）解包为 reject（tauri-specta 默认 Result 模式 resolve `{status:'ok'|'error'}`）

### 序列化

- 跨边界结构体统一 `#[derive(Serialize, Deserialize)]` + `#[serde(rename_all = "camelCase")]`；纯枚举（`Rating`/`ColorLabel`/`Flag`/`Theme` 等）不加 rename
- 跨 Rust→TS 边界的类型加 `#[cfg_attr(feature = "specta", derive(specta::Type))]`（domain/config 已全量）；specta 导出经 `specta_builder` 的 commands + `.typ::<T>()` 登记

### 同步 vs 异步

- **core 层全同步**（grep 无 async/await/tokio 命中）；Tauri 后端用 `tauri::async_runtime::spawn_blocking` 包裹同步引擎调用；前端 store 承担哨兵复位（loading/进度状态机）

### 命名惯例

- 模块/函数 snake_case，类型/枚举 PascalCase，测试统一 `test_<subject>_<scenario>`
- 谓词 `is_*`；动词前缀 `get_or_*`/`extract_*`/`set_*`
- 错误类型 `ModuleNameError`；注释全部为中文
- TS：事件 payload 类型在 ipc.ts（`XxxPayload`）；command 薄封装 `export const xxx: (...) => Promise<T> = api.xxx`

### 已知陷阱

- **dev 构建必须给像素密集的 crate 开 `opt-level = 3`**：根 `Cargo.toml` 的 `[profile.dev.package.*]` 除 `image`/`rawlib`/`rusqlite` 外，还要有 `photo-engine`/`jpeg-decoder`/`zune-jpeg`/`zune-core`。实测生成一张 39MP JPEG 的 2560 预览母版：**debug 7.45s vs release 0.55s**——漏开就是"点开一张图卡 7 秒"
- `quick-xml` 在根 `Cargo.toml` 的 `[workspace.dependencies]` 中声明但各 crate src 无引用
- **exiftool `-stay_open` 长驻进程不能加 `-q`**：`-q` 同时抑制 `{ready}` 标记，导致 execute 读不到结果边界挂起
- **Windows 官方 exiftool(-k).exe 内嵌 `-k`（每命令后等 ENTER）**：程序化调用必须用 `perl.exe exiftool.pl`（photo-engine 已自动处理）；开发时残留 perl.exe 进程会让后续 cargo 命令假死，`taskkill //F //IM perl.exe` 清理（cfg(test) 已跳过真实 spawn、photo-tauri 退出走 shutdown_provider，仅手动 example 需注意）
- **exiftool 定位优先级**：`PHOTO_EXIFTOOL` env → exe 同级 `exiftool/`（打包）→ 仓库 `local-lib/exiftool/`（开发）→ PATH；升级版本见 `docs/exiftool-update.md`
- **模型/名录库/全局索引库定位（`data_root()`，lib.rs）**：`PHOTO_DATA_DIR` env → exe 同级 `models/`+`data/`（打包便携）→ 仓库根（开发回退，从 CARGO_MANIFEST_DIR/cwd 向上找同时含 `models/` 与 `data/bird_catalog.db` 的目录）；cargo run/tauri dev 下模型在仓库根，否则会报「YOLO 模型文件不存在: <target>/debug/models/detect.onnx」
- 使用了 let-chains（edition 2024 特性），如 `photo-config/config.rs` 便携路径判断
- **tauri-specta 生成 bindings.ts 会覆盖手写追加段**：specta 不导出的类型（FilterCriteria/SortBy 等——Rust 侧无 command 引用它们）需在文件尾部手写保留，重新导出后手动恢复
- **export_bindings 按 cwd 相对路径写文件**：必须 `cd crates/photo-tauri/src-tauri` 再 `cargo run -p photo-tauri --bin export_bindings`；从 workspace 根跑会把 bindings.ts 写到 `crates/src/lib/` 错位置
- **前端验证命令必须带 `-b`**：`npx vue-tsc -b --noEmit`；`npx vue-tsc --noEmit` 在 solution-style tsconfig 下 exit 0 假绿
- **启动自动恢复扫描事件早于页面挂载**（缓存命中 ~200ms）：前端 `captures.init()` 主动拉 `getAppConfig().lastDirectory` + `getCaptures()` 自愈
- **mock 模式数据分叉**：多人同时编辑时 vite HMR 堆叠多份模块实例，mock 模块级数组分叉（表现为 store 与 getRecognition 数据不一致）——全量刷新页面即可
- **评分/旗标/色标筛选全部在前端执行**（`src/lib/filter.ts` 移植自 filter.rs）；`FilterCriteria::has_active_filter` 语义 = 批量操作安全边界（无筛选时禁用）

---

## 测试与 QA

- Rust：**127 个 `#[test]`**（+ 2 个 `#[ignore]` 真机冒烟）分布在 4 个 crate 的源文件末尾内联 `#[cfg(test)] mod tests`
- 前端：vitest 55 用例（filter.test.ts 22 + stacks.test.ts 7 堆叠分组/主格式 + burst/nameTemplate 26）
- 真机识别冒烟：`cargo test -p photo-recognize -- --ignored`（需 worktree/发布根有 `models/` 与 `data/bird_catalog.db`）；单文件手动识别工具：`cargo run -p photo-recognize --example recognize_file -- <图片路径>`
- 浏览器 mock 实测：`npm run dev` + 浏览器（无 Tauri 后端时走 mock 数据流，覆盖网格/预览/筛选/识别/批量/设置/右键全 UI）

### 测试分布

|模块|测试数|覆盖内容|
|---|---|---|
|`photo-domain::domain.rs`|26|扩展名解析、RAW 白名单、enrich_with_xmp/recognition、ExifMetadata 摘要、XmpMetadata 枚举转换、BBox/RecognitionStatus/RecognitionFilter 序列化与状态映射、EyeSharpness 排序枚举、GPS DMS 转换|
|`photo-config::lib.rs`|10|默认值、TOML 保存/加载往返、配置路径、AppConfig 字段钳制（含 include_subdirectories、export_presets）|
|`photo-engine`|134 + 1 ignore|scanner 单层/递归、ops 移动/复制/重命名/删除（含 sidecar）、识别行同步、thumbnail 缓存键、exif 摘要、convert、folder_db 建表/迁移/upsert/rename 同步/多表清理、adjustments、global_db 索引/修正日志/命中率、histogram 直方图/剪切、import 分组/去重/复制移动、template 占位符渲染、undo 三类逆操作、keywords 表|
|`photo-recognize`|32 + 1 ignore|阶段→状态映射、输入源解析（JPEG/RAW）、检测框变换、softmax/Top-5、名录映射、进度回调、眼关键点、锐度融合单调性|
|前端 vitest|48（3 文件）|filter 30（含 ISO/焦距/镜头/关键词筛选）、burst 11、nameTemplate 7|

---

## Tauri 前端参考

> 技术栈：Vue 3.5（script setup）+ Pinia 3 + Tailwind v4（CSS-first，`@theme` in style.css）+ shadcn-vue（`src/components/ui/`，底层 reka-ui）+ `@lucide/vue` 图标 + `@tauri-apps/api`

### 架构概述

- **状态归属前端主导**：扫描后全量 CaptureMeta 一次下推（`get_captures`），筛选/排序/多选/缩放数学全在 TS 侧（零 IPC）；Rust 侧保留权威扫描结果 + 识别/批量状态事件推送
- **IPC 契约**：`src/lib/ipc.ts` 是唯一 IPC 入口（typed invoke + 事件订阅 + `ptimgUrl`）；`src/lib/bindings.ts` 由 specta 生成（勿手改，追加段除外）
- **事件驱动刷新**：scan:progress/scan:done/capture:enriched/thumb:ready/recognize:progress/recognize:done/batch:progress/batch:done；前端 store `init()` 统一接线
- **图片 URL**：`ptimgUrl(kind, path, v?)`，kind ∈ `'thumb' | 'master' | 'full'`；`thumb:ready` 后 `thumbVersions[path]` 递增强制 `?v=` 刷新
- **主题**：双主题（亮 = gray-100 画布/白面板/blue-500 accent；暗 = 交易终端近黑 #0b0d11/cyan accent）CSS 变量（style.css），另有 element 层语义色与 .section-header/.panel-card/.dir-card-active 共享类；默认 Light；`html.dark` class 由 config store 按 `getAppConfig().theme` 应用；html 基准字号 14px（紧凑密度）

### 关键模式

```ts
// store 事件接线（防重复 listen）
init() {
  if (this.listening) return
  this.listening = true
  void onScanDone((p) => {
    this.scanning = false
    if (p.directory) this.directory = p.directory
    void this.reload()
  })
}

// 乐观更新骨架（失败重拉回滚）
async mutateOptimistic(paths, apply, remote) {
  const prev = this.items.map((c) => (paths.includes(c.primaryPath) ? JSON.parse(JSON.stringify(c)) : null))
  this.items = this.items.map((c) => (paths.includes(c.primaryPath) ? apply(c) : c))
  try {
    await remote()
  } catch {
    this.items = this.items.map((c, i) => prev[i] ?? c)
  }
}
```

### Keybinding 层

- `src/keymap.ts`：`installKeymap(handlers)` 全局安装；按键 → action 名解析（焦点上下文隔离 + 修饰键精确匹配），App.vue 提供 `KeymapHandlers` 表接真实 store 调用；`BINDINGS` 导出供快捷键参考页
- 键位全集：1-5/0 评分、6-9 色标、P/X/U 旗标、B/Ctrl+B/Ctrl+Shift+B 识别、V 检测框、G 视图切换、方向键/Home/End、Delete 删除、Ctrl+A/D 选择、Esc（设置 > 批量识别取消 > 对比退出 > 框选清除 > 预览退出）、F5 重扫、Ctrl+[ / Ctrl+] 面板开关；**前端新增**：C 对比模式（多选 2–4 张 / 连拍组前 4 张，对比内 ←/→ 移聚焦格、1-5 评分聚焦格、Esc/G 退出）、T 鸟种统计、S 幻灯片（空格暂停）、O 剪切警告叠加（预览）、Ctrl+Z 撤销批量操作（移动/复制/重命名）、Ctrl+6 紫色标签、Q/E 堆叠内切换成员（网格）、=/- 缩放（预览/对比）
- 排序含 EyeSharpness（眼锐度，T0 批次）：CaptureMeta.eye_sharpness 经 enrich_with_recognition 填充，比较器在 filter.ts（None 排最前）；连拍分组纯前端（lib/burst.ts，相邻 dateTaken ≤2s 成组，仅登记 size≥2）

### 调试：真机 WebView2 DOM 验证

kimi_cu 的 UIA 树看不到 WebView2 DOM；无 vision 模型时用 CDP：
1. `tauri.conf.json` window 加 `"additionalBrowserArgs": "--remote-debugging-port=9222"`（env 变量不生效；tauri 覆盖）
2. `curl localhost:9222/json/list` → 拿页面 `webSocketDebuggerUrl` → node_repl 直连（browser 工具连不上 WebView2 页面 target；DevTools 窗口会占用 target）
3. 验证后移除该配置

### 日志（tracing 统一管道）

- Rust 侧（engine/recognize/tauri 后端）已大量 `tracing::*` 埋点，`run()` 启动时经 `src-tauri/src/logging.rs` 安装全局订阅者落盘：
  - 滚动日切文件 `<配置目录>/logs/ftpt.YYYY-MM-DD.log`（即 `~/.config/pt/logs/`，Windows 为 `%USERPROFILE%/.config/pt/logs/`），保留 14 份；`PHOTO_LOG_LEVEL` 环境变量覆盖级别（默认 debug 构建 DEBUG、release INFO）；stderr 同步输出（tauri dev 终端可见）
  - `tracing_log::LogTracer` 桥接 rawlib 等第三方 `log::*` 调用
  - 前端 console/未捕获异常经 `log` command（ipc.ts `logToBackend`）转发到同一文件（target=frontend）；App.vue `installFrontendLogging()` 全局拦截 + onErrorCaptured
  - 设置「关于」页展示当前日志路径，可打开日志文件/目录（`open_log_file` / `open_log_directory`）
- 前端 `src/lib/log.ts`：`logToFile(level, msg, ctx?)` 随时写一条；mock 模式全部 no-op

---

## 近期修复记录

- **2026-09-18 feat(photo-ui 预览)：1:1 接上全分辨率图源（修「预览糊」）**：现象「单张预览发糊」经实测定位为**图源选择**而非解码质量——统一缩到 812px 显示尺寸下，派生 2560 母版 lapVar 165.6 与 AHD 全尺寸 171.3 基本无差（fit 视图不糊），但 1:1 时 `one_to_one = zoom==0 && !is_raw` 对 RAW 恒 false，显示框按 EXIF 自然尺寸（8152×5432）排布、画的却是 2560 母版 → **放大 3.18 倍**，lapVar 4.3 vs 真 1:1 的 48.7（糊 11 倍）；母版未就绪时显示的 440px 网格缩略图占位 lapVar 35.0 是另一个糊源。photo-tauri 的 `ptimg://full` 早有全分辨率路径（`ThumbnailCache::get_or_generate_full`：AHD 全尺寸 + `full` 变体落盘缓存），photo-ui 未接线。本次补上：`ImageManager::load_full_image`（内存只留最后一张，44MP 全尺寸 RGBA ≈177MB）+ `engine_ops::load_preview_full` + AppState `preview_full/preview_full_request`；判据抽成纯函数 `preview_math::exceeds_master_res`（显示长边 > 母版 2560 → 换真原图，1:1 与高倍放大同一条路）。实测 P1082800.RW2：全尺寸冷启 1.97s / 内存命中 9µs（8152×5432）。验证：`cargo test -p photo-ui`（38，含边界用例）、`lease_smoke` 15 项、新增无头冒烟 `preview_full_smoke` 5 项（fit 不加载 / 1:1 加载 / 路径不串 / 字节量级）、`master_bench` 增加 1:1 档计时
- **2026-09-18 fix(RAW 预览偏暗)：显示母版重新开启自动亮度 + 内嵌快路径修复**：事故现象「RAW 预览比机内 JPEG 明显偏暗」根因不在白平衡——`rawlib::DecodeOptions::preview()` 为省一趟直方图扫描把 `no_auto_bright` 置 true，而 `full()`/`preview16()` 均为 false，导致 fit 母版与 1:1/调整页曝光不一致。实测 P1082800.RW2（Lumix 47MP RW2）：机内 JPG luma 76.6 / 内嵌 JPEG 75.9 / 应用母版 **48.1** / full() 93.9；白平衡 R/G=1.17 B/G=0.966 与内嵌 JPEG 1.21/0.969 基本一致（关相机 WB 则 B/G=0.65 明显偏黄），故只改曝光口径：新增 `display_preview_options()`（half_size+bilinear+sRGB+相机 WB+**自动亮度**），`decode_raw_impl` 改用它 → 母版 luma 93.8≈full() 93.9。另修死代码快路径：LibRaw 的 `libraw_processed_image_t` 对 JPEG 类型**不填 width/height（恒 0）**，原判据 `thumb.width.max(thumb.height) >= 2048` 永不成立，「大内嵌直接当母版」从未生效（每张 RAW 预览都白付完整解码），改由 `embedded_long_edge()` 解 JPEG 头取真实长边；`CACHE_VERSION` 4→5 让陈旧偏暗母版失效。手动诊断工具 `cargo run --release -p photo-engine --example raw_brightness_check -- <RAW>`
- **2026-09-18 photo-ui：左右边栏改 Dock + 导入流程补齐**：`crates/photo-ui` 工作区从自绘三栏改为 gpui-kit Dock（`DockArea` + `DockSkin`，新增 `views/dock_panels.rs`）——左停靠区（文件树/批量操作）、右停靠区（信息/调整）、中央主视图；边缘可拖宽、Ctrl+[ / Ctrl+] 折叠、宽度 350ms 去抖写回 `AppConfig.leftPanelWidth/rightPanelWidth`，`set_locked(true)` 锁重排不锁拖宽。导入弹窗从占位实现补齐（新增 `state/import.rs`）：可移动盘/浏览选源 → 扫描 → 目标根目录 + 4 种子目录模式 + 重命名模板 → 复制/移动 → 干跑计划预览 → 进度 → 全成功自动关闭并重扫结果目录。无头冒烟 `XDG_CONFIG_HOME=/tmp/ptlease-config xvfb-run -a cargo run -p photo-ui --example lease_smoke` 15 项全过；修复 7 个弹窗的滚动穿透（遮罩缺 id + occlude，滚轮会落到主视图）

- **2026-08-12 堆叠显示改造（A+E）**：网格堆叠从「×N 徽标循环点击」改为 cell 底部成员缩略图带（点击直达激活+选中，长连拍横向滚动），新增语义徽标区分同画面多格式（Copy 蓝）/连拍多帧（Layers 橙），连拍徽标仅在单成员组显示避免重复；新增 Q/E 组内切换激活成员（网格态）；修复 `openPath` 同目录早退导致目录为空（mock 无后端自动扫描/启动自愈后扫描失败）时无法重扫的死路
- **2026-08-10 迁移 wave 1-3**：Tauri v2 迁移（计划见 git 历史 `docs/tauri-migration-plan.md`）；specta 真实绑定导出（bin 绕开 harness 0xc0000139）；启动自愈（自动恢复目录事件早于挂载）；主题默认 Light；mock 层 batchOpExecute detached `this` 修复
- 历史（旧版时代，引擎层均保留）：copy_recognitions_to 索引错位修复、批量操作 ADR 0006 重构（筛选驱动）、全分辨率 DCT 降采样、RAW 母版缓存、Worker panic 兜底（前端 store 状态机继承）、OTHER 格式徽标、调整功能 ADR 0007（无 crop）
