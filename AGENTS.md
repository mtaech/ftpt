# Repository Guidelines — Photo Tool

## 项目概述

Photo Tool 是一个**照片管理与筛选（culling）**应用，用于浏览、标记、识别和转换照片。Cargo workspace 含 5 个成员：

- `photo-domain` — 纯类型叶子 crate（Capture, ExifMetadata, XmpMetadata, Recognition 类型, 枚举），依赖仅 serde + chrono
- `photo-engine` — 文件操作引擎（scanner, ops, exif, thumbnail, convert, folder_db, batch_ops），**全同步**
- `photo-recognize` — 鸟类识别管线（YOLO 检测 → 鸟种分类 → 名录映射 → 鸟眼锐度，ONNX Runtime），**全同步**
- `photo-config` — 配置读写（TOML + SQLite 持久化）
- `photo-tool-app` — GPUI 前端（暗色主题，三栏布局，全键盘操作）
核心工作流：**目录扫描 → 浏览/标记/筛选 → 鸟类识别（单张/批量）→ 文件操作（删除/移动/复制/重命名）→ 格式转换**。识别子系统设计见 `docs/adr/0003-recognition-subsystem.md`。

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

1. **scanner** → `Vec<Capture>`：walkdir 单层（`max_depth(1)`）扫描，**每个图片文件一个 Capture**（JPG/RAW 不再堆叠配对）；扫描期不做任何筛选，识别状态等元数据在扫描后才从 folder_db 读取，全部筛选由 app 层 `state/filter.rs` 在 CaptureMeta 层面执行
2. **Capture** → **exif**：提取 EXIF（常规图 kamadak-exif，RAW 走 `rawlib::exif`）；`CaptureMeta::enrich_with_exif` 回填摘要（类型 `ExifMetadata` 定义在 domain，提取机械在 engine）
3. **Capture** → **ops**：删除（回收站/永久）/移动（跨设备 copy+delete 回退）/复制/批量重命名
4. **SourceFile** → **thumbnail**：磁盘缓存 JPEG 字节（缓存键 = `DefaultHasher(path+size)` 的 `{:016x}.jpg`，目录 = 照片目录 `.pt/thumbs`，每文件夹独立）；RAW 完整解码（half_size 预览选项）母版按 `u32::MAX` 键存一份，网格缩略图/预览/全分辨率/识别均从母版 DCT 派生（不落盘）；内嵌 JPEG 长边 ≥2048（RW2/DNG 大内嵌）时直接用作母版省解码；常规图优先 EXIF 内嵌缩略图
5. **Capture** → **convert**：RAW 内嵌预览→JPEG、常规图缩放（Lanczos3）
6. **Capture** → **recognize**：`photo-recognize` 管线（YOLO 检测鸟体 → 整图 eye.onnx 检测眼（双槽一致性选点，CPU 推理）→ bird_model 分类 Top-5 → `sp_cls_map` JOIN `animal_info` 名录映射）→ `Recognition` 三态（Confirmed/NeedsReview/Unrecognized）+ 连续鸟眼锐度分（NULL 兜底，不影响三态）→ `folder_db` upsert 到文件夹级 `.pt/data.db`。鸟眼锐度设计见 `docs/adr/0005-eye-sharpness-stage.md`
7. **import**（近期移除，待重建）：检测可移动设备 → DCIM 递归扫描 → 按 EXIF 日期建子目录 → 委托 **ops** 移动/复制

### 模块依赖关系

- `photo-tool-app` 依赖其余四个 crate
- `photo-engine` 依赖 `photo-domain`（单向 DAG，由 crate 边界强制）
- `photo-recognize` 依赖 `photo-domain`（RAW 输入解码复用 `photo_engine::thumbnail::decode_raw_preview`，不反向依赖）
- `photo-config` 独立，无 crate 内依赖
- `domain` 是纯叶子：依赖仅 `std` + `serde` + `chrono`，不引用任何内部模块

---

## 关键目录

|-|-|
| `crates/photo-domain/src/domain.rs` | 纯类型（Capture, ExifMetadata, XmpMetadata, 枚举），零外部 crate 依赖 |
| `crates/photo-engine/src/` | 文件机械：scanner, ops, exif, thumbnail, convert, folder_db, batch_ops, cache（全部同步） |
| `crates/photo-engine/src/folder_db.rs` | 文件夹级 SQLite（`.pt/data.db`）：exif_cache / xmp_meta / **recognition** 三表，rusqlite_migration 版本化 |
| `crates/photo-recognize/src/` | 识别管线：lib.rs(Recognizer 门面), detect(YOLO), classify(bird_model), catalog(名录映射), pipeline |
| `crates/photo-config/src/lib.rs` | 配置读写（TOML + SQLite 持久化）|
| `crates/photo-tool-app/src/state/` | 状态层（拆分后 8 文件）：app.rs(RootView + dispatch_action 路由) / scan.rs(扫描+EXIF回填+DB同步) / image_cache.rs(缩略图/预览/全分辨率加载) / recognition.rs(单张/批量/框选识别) / metadata.rs(评分/旗标/色标/删除) / filter.rs(筛选排序+鸟种下拉) / batch_ops.rs(批量文件操作) / preview_math.rs(缩放平移坐标纯函数) |
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
- `photo-engine/src/lib.rs` 声明 8 个 `pub mod`（scanner, ops, exif, thumbnail, convert, folder_db, batch_ops, cache）；XMP 读写实现在 folder_db 的 xmp_meta 表，无独立 xmp.rs
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
- 评分/色标/旗标（XMP 元数据）不走 XML 解析：由 folder_db 的 `xmp_meta` 表持久化（`put_xmp`/`get_xmp`，键 = 完整路径），ops 层 `sync_*_xmp` 在删除/移动/重命名时同步

### 同步 vs 异步

- **core 层全同步**（grep 无 async/await/tokio 命中）
- 平台分支用 `#[cfg(target_os = ...)]`（`import.rs`：windows/linux/macos）

### 命名惯例

- 模块/函数 snake_case，类型/枚举 PascalCase，测试统一 `test_<subject>_<scenario>`
- 谓词 `is_*`；动词前缀 `get_or_*`/`extract_*`/`set_*`
- 错误类型 `ModuleNameError`；注释全部为中文

### 已知陷阱

- `quick-xml` 在根 `Cargo.toml` 的 `[workspace.dependencies]` 中声明但各 crate src 无引用
- 筛选全部在 app 层 `state/filter.rs::apply_filter_and_sort` 的 CaptureMeta 层面执行（format/鸟种/日期/评分≥N/色标/旗标/未标记旗标/识别状态均已实现）；scanner 不做筛选，`FilterCriteria` 传给 `scan_directory` 仅为签名兼容
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

gpui-component 项目位于 `E:\Dev\Code\gpui-component`，含完整源码和本地文档：

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
| `rawlib 0.7.1`（workspace 内嵌 `crates/rawlib`） | RAW 解码/EXIF/内嵌缩略图；`build.rs` 自动链接 `libraw/` 预编译库（Windows msvc 静态库；Linux 优先系统库，缺失回退 bundled gnu），并编译 `half_size.c`。源码内嵌便于修改解码逻辑（勿回退 crates.io 版本） |
| `trash 4` | 移到回收站（跨平台） |
| `walkdir 2` | 单层目录扫描 |
| `chrono 0.4` / `toml 0.8` / `serde 1` | 日期 / 配置 / 序列化 |
| `ort 2.0.0-rc.10`（photo-recognize） | ONNX Runtime 绑定；`download-binaries` 静态链接运行时，Windows 走 DirectML EP（失败回退 CPU），仅需随包携带 `DirectML.dll` |
| `thiserror 2` | 错误 derive |
| `tempfile 3`（dev） | 测试临时目录 |

### 平台要求

- **Linux**：需 `libraw.so` 可链接/可加载——放 `local-lib/` 或系统安装；`.cargo/config.toml` 已配 `-L local-lib` 与 `LD_LIBRARY_PATH`；识别管线在 Linux 走 CPU EP
- **Windows**：无特殊配置（Windows 11 为开发/测试环境）；识别走 DirectML（系统内置），发布包需附带 `DirectML.dll`
- **识别资产**：`models/`（detect.onnx + bird_model.onnx + eye.onnx）与 `data/pica_ref.db`（名录库）必须位于 **exe 同级目录**（便携约定，不入库，`.gitignore` 已排除）；开发时即 `target/debug/` 或 `target/release/` 下

---

## 近期修复记录（2026-07-31 审查修复）

全仓代码审查后按严重度修复，含：

- **数据正确性**：`copy_recognitions_to` 索引错位修复（识别行不再写入错误目标）；跨设备 move 不再静默覆盖已存在目标；批量移动/复制后的识别行/XMP 同步函数已就绪但 app 层尚未接线（`execute_batch_ops` 只搬文件，见 ops.rs sync_*）
- **崩溃路径**：空目录批量识别 `chunks(0)` 卡死已加忙/空守卫；status_bar 长中文路径截断改按字符边界；反向/越界 bbox 的 u32 下溢（classify/debug_eye）归一化修复；损坏 pica_ref.db 不再 panic（降级 NeedsReview）
- **功能**：评分/旗标/色标/识别状态筛选在 CaptureMeta 层全部实现（scanner 不再清空）；含 alpha 图片转 JPG 展平；常规图 EXIF 现提取 GPS；乐观更新失败回滚 UI
- **状态一致性**：扫描/删除换代（`scan_generation`）丢弃过期加载/回填结果，防缩略图/预览张冠李戴；`scan_task` 死字段替换为换代计数 + `scan_in_progress` 指示；导航清空未完成框选
- **健壮性**：损坏 PT.db 自愈（改名 .bak + 重建）；`CaptureMeta::from_capture(c, index)` 显式索引；config 死字段/domain 死枚举（DeleteMode）/engine 死模块（cache.rs）/死依赖清除；folder_db 补上缺失 #[test]；stage_to_status 生产化后测试驱动真实映射；`ThumbnailCache::prune` 曾接线（扫描后按 `max_cache_size_mb` 清理），后随缓存按文件夹隔离（`.pt/thumbs`）移除
- **结构**：app.rs（约 2870 行）按接缝拆分为 state/ 8 文件；ui 死主题 token、重复 format_file_size、调试日志清除

## RAW 预览加载优化（2026-07-31）

- **rawlib 内嵌**：`E:\Dev\Code\rawlib` 复制为 workspace 成员 `crates/rawlib`（`build.rs` 自动链接 `libraw/` 预编译库，勿回退 crates.io 版本）
- **直接解码**：RAW 预览不再用内嵌小图（多数相机 160-640px，放大糊）——`decode_raw_impl` 直接完整解码（`DecodeOptions::preview` = half_size+bilinear+8bit，约 4x 加速）；内嵌 JPEG 长边 ≥2048（RW2/DNG）时仍直接用省解码
- **母版缓存**：`ThumbnailCache` 对 RAW 只存一份母版（`u32::MAX` 键，完整解码 JPEG），缩略图/预览/全分辨率从母版 DCT 派生（不落盘）——同一文件不再按尺寸重复 LibRaw open+解码；`CACHE_VERSION` 2→3（旧内嵌小图缓存作废）
- **双线程池**：`Worker` 拆 `pool`（批量：预加载/EXIF/同步/识别）+ `fast_pool`（交互：预览/全分辨率/网格懒加载，2 线程）——预览不再被 50 个预加载任务排队阻塞；预览预取焦点图优先入队；preload 补 `grid_loading` 哨兵防与懒加载重复生成
- **EXIF 后台化**：扫描闭包只查 `exif_cache`，未命中交给 `spawn_enrich_tasks` 并发提取并写回缓存（不再串行 LibRaw open）；全部完成后重排一次（日期排序正确）
- **convert**：RAW→JPEG 转换从内嵌小图改为完整解码（输出清晰；`max_dimension=0` 映射 `u32::MAX`）
- **缓存按文件夹隔离**：缩略图缓存目录改为照片目录 `.pt/thumbs`（扫描时重建，与 `.pt/data.db` 同级），删除文件夹即清空缓存；移除全局 `max_cache_size_mb` 配置与 `prune` 调用（config 字段、设置面板 UI 一并删除）
- **批量文件操作两阶段**：点「开始执行」→ 干跑预览（扫描+匹配，只展示文件名不动文件）→ 按钮变「确认执行（N 个）」→ 真执行；删除类操作按钮红色警告（批量删除本就走回收站 `ops::delete_capture` → `trash::delete`，非永久删除）；`BatchOpType` 新增 `description()`（下拉副标题）与 `is_delete()`；切换目录/格式/操作类型自动使预览失效；结果区新增「成功 N / 失败 M」汇总，列表可滚动
- **批量操作重构（ADR 0006）**：对比目录匹配引擎（`find_matching` 与 6 种 Same/NotSame 操作）整体移除——操作对象改为**当前筛选结果**（纯筛选驱动，`display_order`）；动作收敛为移动/复制/删除三种；目标目录执行时用户选择（一步式，拒绝目标=源目录）；「同步同名文件」开关（默认关）+ 格式多选（默认全选）按 stem 将兄弟文件纳入操作集（新增 `batch_ops::expand_with_siblings`，触发点自身格式不在同步集合时也会保留）；删除走**弹窗确认**（含「其中 M 个来自同名同步」警告 + 清单）；移动/删除完成后**全量重扫**刷新网格（复制不动源列表）；完成 toast 摘要 + 侧栏失败详情

---

## 测试与 QA

- 全部 **166 个 `#[test]`**（+ 1 个 `#[ignore]` 真机冒烟）分布在 5 个 crate 的源文件末尾内联 `#[cfg(test)] mod tests
- 无外部 `tests/` 目录、无异步测试、无第三方测试框架（唯一 dev-dep：`tempfile`）
- 真机识别冒烟：`cargo test -p photo-recognize -- --ignored`（需 worktree/发布根有 `models/` 与 `data/pica_ref.db`）；单文件手动识别工具：`cargo run -p photo-recognize --example recognize_file -- <图片路径>`（全管线）/ `recognize_region -- <图片> <x1> <y1> <x2> <y2>`（跳过检测，手动框选区域直接分类）

### 测试分布

| 模块（新 crate） | 测试数 | 覆盖内容 |
|---|---|---|
| `photo-domain::domain.rs` | 23 | 扩展名解析、RAW 白名单、enrich_with_xmp/recognition、ExifMetadata 默认/摘要、XmpMetadata 枚举转换、BBox/RecognitionStatus/RecognitionFilter 序列化与状态映射 |
| `photo-config::lib.rs` (config) | 6 | 默认值、TOML 保存/加载往返、配置路径 |
| `photo-engine::scanner.rs` | 8 | 每文件一个 Capture、大小写、sidecar 分离、忽略视频 |
| `photo-engine::ops.rs` | 8+ | 移动/复制/重命名/删除（含 sidecar）、命名冲突、批量跳过缺失、识别行同步（sync_delete/copy/rename） |
| `photo-engine::thumbnail.rs` | 6 | 缓存命中、键唯一性、stats/clear、prune 淘汰最旧、错误路径 |
| `photo-engine::exif.rs` | 5 | 无 EXIF 报错、不存在文件、file_size 始终填充（含 GPS 提取） |
| `photo-engine::convert.rs` | 7 | resize 三分支、格式分发、RAW 错误、输出路径命名 |
| `photo-engine::folder_db.rs` | 16 | 识别表建表/迁移（含 eye_sharpness/eye_bbox 列）、upsert/get/delete、rename/copy 同步、all_recognitions、旧版 cache.db 迁移、sync_with_scan 三表清理+指纹重提取 |
| `photo-recognize::lib.rs/pipeline.rs/eye.rs/sharpness.rs` | 31 | 阶段→状态映射（生产函数 stage_to_status 驱动）、输入源解析（JPEG/RAW）、检测框变换、softmax/Top-5、名录映射 0/1/多、进度回调、眼关键点解析与坐标映射、锐度融合单调性、eye.onnx 缺失报错；另 1 个 `#[ignore]` 真机冒烟 |
| `photo-tool-app` | 18 | action/状态工具函数（含预览数学纯函数） |

### 测试辅助

- 各模块私有 helper + `tempfile::TempDir` 做 FS 隔离
- 断言用 `assert!`/`assert_eq!`，Result 直接 `unwrap()`；无共享 fixture
- 命名：`test_<subject>_<scenario>`，一个测试验证一个行为或边界

---

## GPUI 前端参考

> 来源：`E:\Dev\Code\zed-main\crates\gpui\docs\` 和 `docs\src\development\glossary.md`

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
