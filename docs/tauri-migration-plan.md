# Tauri v2 迁移计划（GPUI → Tauri v2）

> 2026-08-10 定稿。决策经逐条拷问确认；功能 parity 基线 = scout 盘点清单（见附录 A）。

## 0. 决策记录

| # | 决策点 | 结论 |
|---|---|---|
| Q1 | Spike 先行？ | **跳过正式 spike，直接动工**；性能风险前置到 Phase 1 出口（最早可验证点先做预览缩放原型） |
| Q2 | 迁移策略 | **并行新 app**：workspace 新增 Tauri 前端，`photo-tool-app`（GPUI）保留可用，parity 验收后删除 |
| Q3 | 前端框架 | **Vue 3 + Pinia**（Composition API + script setup） |
| Q4 | 组件库 | **shadcn-vue**（底层 = Reka UI headless + Tailwind，组件现成且样式全可控，Catppuccin Mocha 暗色 + 亮色主题经 CSS 变量定制） |
| Q5 | 状态归属 | **前端主导**：扫描后全量 CaptureMeta 一次发前端（1 万张约几 MB），筛选/排序移植 TS（filter.rs 275 行纯逻辑），筛选零 IPC；Rust 侧保留权威扫描结果 + 识别状态事件推送 |
| Q6 | 平台 | **Windows + Linux 同等支持**（Linux 需 WebKitGTK 打包，进验收） |
| Q7 | parity 范围 | **全量 parity**，一项不砍（含 scout 新发现的调整 tab、修正鸟种、收藏/最近目录等全部功能） |
| Q8 | 键盘操作 | **一等公民**：自建 keybinding 层（焦点上下文 + 快捷键表），复刻 layout.rs 全部键位，进 Phase 2 验收 |
| Q9 | 半成品功能 | **补齐 UI 入口**：日期筛选控件、颜色标签筛选入口、缩略图尺寸设置控件（逻辑均已存在，仅缺控件） |
| Q10 | 交互不一致 | **原样移植**，改进另记清单（见 §6），不在迁移中夹带行为变更 |

技术决策（实现层，不再逐项评审）：

- **图像 serving**：自定义 protocol `ptimg://`，流式读 `.pt/thumbs/*.jpg`（网格缩略图）与 RAW 母版/全尺寸母版（预览/1:1），**不走 IPC base64**；缓存未命中时前端经 command 触发生成，完成后事件通知刷新
- **类型共享**：specta + tauri-specta 从 serde 类型自动生成 TS 类型与 typed invoke（domain 类型已全量 derive Serialize，零成本接入）
- **调整功能**：曝光/对比度/饱和度渲染**留在 Rust**（engine/adjustments.rs + 16-bit 链路复用），前端 slider 350ms 去抖发参数，后端重渲 1600px 预览经 `ptimg://` 刷新；导出 = 后端全尺寸烘焙，架构与现状一致
- **文件对话框**：tauri-plugin-dialog（替代 rfd；GPUI 版为规避 RefCell 重入在 worker 线程开 rfd，Tauri 无此问题）
- **Worker 双池语义**：Tauri command 内 `tauri::async_runtime::spawn_blocking` + 保留 rayon 双池（批量池 + 4 线程 fast 池），on_done 哨兵复位改由前端 store 状态机承担
- **便携约定保持**：`models/`、`data/pica_ref.db`、PT.db/PT.toml、logs/ 仍在 exe 同级；Tauri 打包按 portable 布局配置资源
- **字体枚举**：font_kit 系统字体枚举留 Rust 侧，command 暴露给设置面板字体下拉
- **shadcn-vue 主题接入**：组件样式走 CSS 变量，Catppuccin Mocha（暗）/ Latte（亮）两套变量对应 theme.rs 现有 token；高密度布局靠 Tailwind 工具类收紧，不硬套 shadcn 默认留白

## 1. 目录结构（新增，不动现有 crate）

```
crates/photo-tauri/           # 新前端根（npm + tauri 项目）
├── package.json              # vue3 + pinia + tailwind + shadcn-vue(reka-ui) + @tauri-apps/api
├── components.json           # shadcn-vue 配置
├── src/                      # Vue 前端
│   ├── stores/               # Pinia：captures（含 filter.ts 移植）、selection、preview、recognition、settings
│   ├── components/           # layout / grid / preview / filmstrip / sidebar / infoPanel / filterBar / statusBar
│   │   └── ui/               # shadcn-vue 组件（button/dialog/select/slider/toast/context-menu…）
│   ├── keymap.ts             # keybinding 层（复刻 layout.rs 全部键位）
│   └── types.ts              # specta 自动生成，不入库
└── src-tauri/                # Rust crate（workspace 第 6 成员）
    ├── src/main.rs           # commands + events + ptimg protocol + 启动（恢复 last_directory、模型预热）
    └── Cargo.toml            # 依赖 photo-engine/recognize/domain/config + tauri + specta
```

## 2. 阶段计划

### Phase 1 — 底座（约 4–5 天）

> 2026-08-10 状态：除「specta 类型生成」与「性能验证」外已全部落地（并行开发 wave 1 后合并验证）。

- [x] Tauri v2 项目脚手架 + workspace 接入（`cargo check -p photo-tauri` 全绿；`ftpt-next.exe` 冒烟可启动；Linux target 尚未验证）
- [x] Tailwind + shadcn-vue 接入，Catppuccin 双主题 CSS 变量落地（style.css：Mocha 暗默认 + Latte 亮预留）
- [~] specta 类型生成打通：domain/config 已加 `specta` feature + derive；**导出测试在本机 0xc0000139 崩溃（仅 lib 测试 harness，`ftpt-next` bin 正常；导入表与可运行 bin 逐项一致，疑似环境/加载器问题）**——当前以手写 stub `src/lib/bindings.ts` 为契约，集成时改独立 bin（`specta_builder` 提 pub + `src-tauri/src/bin/export_bindings.rs`，`cargo run -p photo-tauri --bin export-bindings`）绕开 harness 再修
- [x] `ptimg://` protocol：缩略图 + RAW 母版 + 全尺寸母版三路流式 serve（lib.rs ptimg_handler）
- [x] 核心 commands：`pick_directory`/`scan_directory`/`get_captures`/`set_rating`/`set_flag`/`set_color_label`（scan 后 EXIF 后台回填 + 缩略图预生成 + 四事件推进度；乐观更新回滚由前端 store 承担）
- [x] **性能风险前置验证**：真机冒烟通过（WebView2 真实渲染：378 项网格虚拟化 24 cell + ptimg 缩略图 440×293 秒显；预览缩放平移组件就绪）——放行 Phase 2 后续
- **出口**：✅ 前端能打开真实目录（E:\图片\2026-06-28 自动恢复 378 张）、显示照片网格（虚拟化+徽标）、单图预览可缩放平移（PhotoPreview + filmstrip + 检测框）

### Phase 2 — 核心浏览（约 7–9 天）

> 2026-08-10 wave 1/2 并行完成（6 agent + UI 对齐专项）：TauriBackend（17 commands + 8 事件）、SelectionGrid、KeymapLayer、FilterLayer（22 vitest）、PreviewEnhance、LayoutShell、UiParityLayout（三栏+InfoPanel+RightRail+空态）。契约见会话 local://tauri-contract.md。

- [x] 三栏布局：activity rail 48px + 可拖宽侧栏（200–480 钳制 + localStorage 持久化）+ status bar 24px；亮/暗主题（Catppuccin，**默认 Light 对齐 GPUI AppConfig::default，跟随后端 config.theme**）
- [x] 虚拟网格：固定 4 列、行级虚拟化（±2 行缓冲，378 项 DOM 仅 24 cell）、cell 全套徽标（格式/旗标/星级/鸟种状态/色标条/OTHER）、单击/Ctrl/Shift 选择语义（anchor 逻辑移植）、双击进预览、方向键滚动跟随、筛选结果驱动（filteredIndices）
- [x] keybinding 层：keymap.ts 复刻 layout.rs 全部键位（含 Ctrl+B/Ctrl+Shift+B/V/G/方向键/Home/End/Delete/Ctrl+A/Ctrl+D/Esc/F5/Ctrl+[/Ctrl+]=面板开关），焦点上下文隔离 + 修饰键精确匹配；识别/删除/后退前进为 Phase 3 占位
- [x] 预览：滚轮以光标为中心缩放（×1.25 步进）、拖拽平移、缩放栏（−/%/+/适应/1:1/检测框）、加载浮层、filmstrip（点击跳转/焦点高亮/横滚预取）、检测框+框选（Shift+拖拽 <8px 忽略 + pending 框，V 键同步）
- [x] 筛选栏：折叠 chips + 排序下拉 + 鸟种多选搜索 + 评分≥N + 旗标/识别/格式 chips + 清除全部；**补齐**日期筛选控件与色标筛选入口（Q9）
- [x] 右侧 InfoPanel（GPUI 对应）：信息/调整双 tab + hero/EXIF/评分/色标/旗标/识别六卡片 + 调整三 slider（350ms 去抖 set_adjustments 后端持久化渲染）；右 rail 48px + Ctrl+] 切换
- [x] 空态：无目录时大图标 + 「打开目录开始浏览照片」+ 主按钮（对照 layout.rs）
- [x] 启动自愈：自动恢复扫描（setup spawn，缓存命中 ~200ms）在页面挂载前完成 → init 时主动拉后端状态补齐 directory/items
- [ ] **出口验收**：浏览/标记/筛选/预览全流程 parity 对照附录 A §1-§11 逐项打勾（wave 3 前做）
- [ ] 虚拟网格：固定 4 列、行级虚拟化、可见 ±2 行预取、cell 全套徽标（格式/旗标/星级/鸟种状态/色标条/OTHER）、单击/Ctrl/Shift 选择语义（anchor 逻辑移植 metadata.rs）、双击进预览
- [ ] keybinding 层：复刻 layout.rs 全部 24 组键位（含 1-5/0/6-9/P/X/U/B/Ctrl+B/Ctrl+Shift+B/V/G/方向键/Home/End/Delete/Ctrl+A/Ctrl+D/Esc/F5/Ctrl+[ / Ctrl+]），焦点上下文隔离（输入框内不触发）
- [ ] 预览：滚轮以光标为中心缩放（×1.25 步进）、拖拽平移、缩放栏（−/%/+/1:1/检测框开关）、加载浮层、filmstrip（点击跳转/焦点高亮/横滚预取）、检测框+眼角标叠加（V 键同步）
- [ ] Shift+拖拽框选识别（<8px 忽略、实时 accent 框、pending 框）
- [ ] 筛选栏：折叠 chips + 排序下拉 + 鸟种多选搜索（shadcn-vue Combobox）+ 评分≥N + 旗标/识别/格式 chips + 清除全部；**补齐**日期筛选控件与色标筛选入口（Q9）
- **出口**：浏览/标记/筛选/预览全流程 parity；全部快捷键可用（对照附录 A §1 逐键验证）

### Phase 3 — 识别与操作（约 6–8 天）

- [ ] 识别 UI 全家桶：单张（B/按钮/右键）、多选、批量未识别（Ctrl+B）、重新识别全部（Ctrl+Shift+B + 确认框）、修正鸟种名录搜索下拉（Confirmed/100%/保留框眼数据）、状态栏进度（n/m·文件名·统计 + ✕/Esc 取消）、逐张回填
- [ ] 识别线程数配置生效（1–4）
- [ ] 批量文件操作两阶段：筛选驱动（无筛选禁用 + 黄字警告）、同步同名开关 + 格式 chips、移动到/复制到（一步式对话框，目标=源拒绝 toast）、删除（红色确认框 + 前 20 条清单 + 同名同步计数警告）、进度条 + 进度弹窗 + 结果明细（成功 N/失败 M）、移动/删除后全量重扫
- [ ] 上下文菜单两套：capture_menu（网格/预览变体）+ folder_menu（收藏/移除）
- [ ] 信息面板全 tab：hero/识别（状态 chip/置信度条/眼锐度+公式 tooltip）/拍摄信息/评分/色标/旗标 + **调整 tab**（三 slider + 独立重置 + 重置全部 + 导出，Rust 渲染链路）
- [ ] 侧栏：打开目录、当前目录卡片、收藏/最近打开文件夹卡片 + 右键菜单
- **出口**：识别与文件操作 parity（对照附录 A §13/§14/§16 逐项）

### Phase 4 — 收尾与切换（约 4–5 天）

- [ ] 设置弹窗：通用（字体下拉 + 识别线程数 + **缩略图尺寸控件** Q9）、快捷键参考页、关于页
- [ ] 单删无确认等交互按 Q10 **原样**移植
- [ ] 打包：Windows NSIS（WebView2 bootstrap + DirectML.dll + models/ + data/ 便携布局）；Linux deb/AppImage（WebKitGTK 依赖声明 + libraw 链接沿用 .cargo/config.toml 方案）
- [ ] **parity 验收**：双 app 同目录对照，按附录 A 清单逐项打勾（Q1 决策的代价在此兜底——任何性能不达标项此处暴露并返工）
- [ ] 删除 `photo-tool-app` crate，更新 AGENTS.md / CONTEXT.md / 本计划标注完成
- **出口**：验收清单全绿 + 两平台安装包人工冒烟通过

**总工期估算：约 4–5 周（1 人全职）**，不含验收返工缓冲。

## 3. 风险清单（按严重度）

| 风险 | 缓解 |
|---|---|
| webview 24MP 1:1 平移缩放卡顿（Q1 跳过的 spike 风险后置） | Phase 1 出口即验证；备选 canvas 分层/瓦片渲染 |
| 快速滚动时网格缩略图渐显慢（浏览器解码排队） | `ptimg://` 流式 + content-visibility + 预取；必要时 `createImageBitmap` 离屏解码池 |
| 调整 slider 实时性（350ms 去抖 + Rust 重渲 + 图片刷新链路延迟） | Phase 3 实测；必要时就地 16-bit→WebGL shader 前端渲染（仅曝光/对比度/饱和度三参数，shader 简单） |
| Linux WebKitGTK 版本碎片化（HiDPI/输入法差异） | Phase 1 即建 Linux 构建，不等 Phase 4 |
| specta 对 chrono/复杂枚举的类型生成缺口 | 缺口处手写 TS 类型补丁，记录在 types 层 |

## 4. 复用与废弃

- **零改动复用**：photo-engine / photo-recognize / photo-domain / photo-config（6,230 + 1,340 行，150+ 测试全保留）
- **移植**：state/filter.rs → TS；state/preview_math.rs → TS；state/metadata.rs 选择语义 → TS store
- **废弃**（parity 验收后删）：photo-tool-app 全部 10,694 行 + GPUI/gpui-component 依赖 + rfd
- **保留行为**：乐观更新回滚、scan 换代防张冠李戴、缓存增量保留、双池隔离、Worker panic 兜底（前端 store 状态机承担哨兵复位）

## 5. 验收方式

- parity 基准 = 附录 A（scout 功能盘点），双 app 同目录并列对照逐项验证
- 引擎层回归：`cargo test`（现有 150+ 测试不受影响）
- 前端纯逻辑：filter/preview_math 移植带 vitest 单元测试（对照 Rust 版测试用例）
- 两平台安装包各一次冷启动冒烟（扫描 → 识别 → 批量操作 → 调整导出）

## 6. 延期改进清单（Q10 决策：不在本次迁移做）

1. 单张删除（Delete）无确认框 vs 批量删除有确认框——交互不一致
2. zoom 放大/缩小无键盘快捷键
3. 紫色标签无快捷键（红黄绿蓝有 6–9）
4. import（可移动设备导入）功能重建（GPUI 版已移除，与迁移无关）
5. 网格列数/缩略图尺寸实时调节（Q9 只补设置项，不做拖拽调节）

---

## 附录 A：parity 功能基线（scout 盘点 2026-08-10 摘要）

> 全清单见会话记录 agent://FeatureInventory；此处为章节索引 + 关键数量。
> §0 启动/便携约定 · §1 快捷键 24 组 · §2 工具栏 · §3 左 rail · §4 侧栏（文件树+收藏+最近打开）· §5 筛选栏 · §6 网格 · §7 预览+filmstrip+框选 · §8 信息面板（信息/调整双 tab）· §9 右 rail · §10 状态栏 · §11 设置弹窗 · §12 上下文菜单 2 套 · §13 对话框/toast · §14 批量操作两阶段 · §15 调整功能（ADR 0007，无 crop）· §16 识别 UI 汇总（6 种入口）· §17 半成品与未记载项
