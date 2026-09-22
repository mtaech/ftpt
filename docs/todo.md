# 待办总账（TODO）

> 2026-09-22 由一次全仓盘点生成。来源：`docs/open-questions.md`（需拍板项）、
> `docs/gui-design-manual.md` §13（设计/实现层缺口）、`AGENTS.md` 各条「遗留 / 未做」，
> 以及**代码里实际核实过**的死路与占位。
>
> 与 `open-questions.md` 的分工：那份是「**需要你回答的问题**」，这份是「**活**」。
> 同一条可能两份都出现，**以本文件的「状态」为准**。
>
> **2026-09-22 复核修正**：#2 / #9 / #12 三条原描述与代码不符（#2 引擎侧其实已就绪、#9 连入口
> 都没有、#12 后台补提取早已存在），已按实测改写并同步调整估工与「建议顺序」；#10 / #11 因收益
> 低于维护成本降级为 ⏸ 暂缓。
>
> **2026-09-22 Batch 1 落地**：#2 重复检测接线 + #1b 谎报按钮（`duplicates_smoke` 21 项）、#18 清理项
> 经核实条目已过期。下一步按「建议顺序」第 6 条做 #5 / #6 语义收敛。

## 状态标记

| 标记 | 含义 |
|---|---|
| 🔴 P0 | 用户能撞上、且现在就是坏的（含「按钮骗人」） |
| 🟠 P1 | 整块能力缺失，工作流上明显缺口 |
| 🟡 P2 | 需要先拍板的语义/资产问题，或数据落库不完整 |
| ⚪ P3 | 打磨项、合规项、已知取舍 |
| ⏸ | 暂缓（已判定收益低于维护成本，等前置条件满足再动） |
| ✅ | 已完成（保留一行，免得重复讨论） |

---

## 🔴 P0 — 现在就是坏的

### 1. ✅ 导出整条链路（原：链路是死的 + 一个谎报成功的按钮）

- **原状（2026-09-22 已修）**：`views/dialogs/export_dialog.rs:188` 的「开始导出」只 `set_status_message("已开始导出照片")`
  然后关窗，**不产出任何文件**。更彻底的是：**全仓没有任何地方设 `ActiveDialog::Export`**
  （`app.rs:593` 只负责渲染它）——所以这个弹窗**连入口都没有**。
- **已有的**：`photo-engine/src/convert.rs` 的 `export_with_preset` / `export_adjusted` 早就绪，
  还带单测（`test_export_adjusted_jpeg_tone_applied` 等）。
- **要做**：① 加导出入口（左栏/顶栏/快捷键）并设 `ActiveDialog::Export`；② `engine_ops` 加
  `start_export`（后台执行 + 进度 + 逐文件结果，套 §8.1 状态机与取消令牌）；③ 把弹窗的
  预设/长边/质量/命名模板真正接进 state 并传下去；④ 失败给真实错误，**不能只报成功**。
- **要谁定**：入口放左栏还是顶栏——小问题，可直接定。
- **估工**：半天。
- **已完成（2026-09-22 · commit `686f766`）**：① 入口 = `Export` action + `Ctrl+E` + 顶栏「导出」按钮 +
  左栏批量面板「导出...」；② 参数 = 目标目录（输入框 + 系统目录对话框）/ 长边档位 / JPEG 质量滑杆 /
  命名模板（实时预览第一张），草稿落 `model/export.rs`；③ 执行 = `engine_ops::start_export`（后台线程 +
  250ms 进度 + 取消 + 逐文件真实错误），状态栏加导出分支；④ 目标目录写回 `AppConfig.export_dir`（#14 导出侧）。
  **验证**：`cargo check --workspace --all-targets` 0 warning；`cargo test` 全绿（photo-ui 61→70）；
  新增无头冒烟 `export_smoke` 21 项全过（已选 1 张 / 未选全量 5 张 / `{seq}` 补零 / 同 stem `_1` 去重 /
  长边 600 / 质量 95 体积 > 85 / 原文件字节不变 / 目录不可创建报真实错误）；`lease_smoke` 15、
  `grid_scroll_smoke` 4、`filter_bar_smoke` 8、`adjust_smoke` 16 回归全过。
- **遗留**：手册 §9.10 的预设「新建 / 保存 / 删除」未做（当前只能套用配置文件里的预设）；
  eBird 导出仍无入口（见 #9）。

### 1b. ✅ 重复检测弹窗的「开始检测」同样谎报成功（详见 #2）

- **现状**：`views/dialogs/duplicates_dialog.rs:97` 点「开始检测当前目录」只
  `set_status_message("正在进行相似照片检测...")` 随即关窗，**不做任何检测**；而入口是活的
  （`views/left_panel.rs:287`），用户现在就能撞上。
- **与 #2 的关系**：这是同一块能力的「按钮骗人」面。修 #2 时一并关闭；若 #2 延期，先做诚实化
  （禁用按钮 + 「未实现」提示），别继续骗。
- **估工**：随 #2；只做诚实化则 10 分钟。
- **已完成（2026-09-22 · 随 #2 Batch 1）**：弹窗改成真检测（后台 dHash + 聚类 + 真实进度条 +
  取消），「开始检测当前目录」不再是「弹提示就关窗」；没有照片时按钮禁用并给 tooltip 说明。
  验证：新冒烟 `duplicates_smoke` 21 项全过（含「入口开窗 / 真出分组 / 状态栏报真实组数」）。

## 🟠 P1 — 整块没做

### 2. ✅ 重复/相似照片检测（接线）

- **现状（2026-09-22 复核修正）**：UI 侧 `views/dialogs/duplicates_dialog.rs` 仍是**纯占位**——
  函数签名 `_state: &AppState`、`_window: &mut Window` 两个参数**都不用**，没有分组、没有优选；
  但左栏有入口按钮（`views/left_panel.rs:287`）。
- **更正**：原写「engine 里也没有 `find_duplicates`」**不准确**——引擎侧早已就绪：
  `photo-engine/src/phash.rs`（2026-08-30 commit `78041b9`）含 `dhash` / `hamming` /
  `group_duplicates` / `compute_hashes` + 9 个单测，还做了「命中缩略图磁盘缓存则零解码」的优化。
  但它**全仓 0 调用方**（仅 `lib.rs` 的 `pub mod phash;` 与 `thumbnail.rs` 的一句注释引用），
  所以本条性质是**接线**而非从零实现。
- **手册要求**：§9.10 / §10.5，以及 §13.8「重复检测的阈值与结果都不落盘，重启后需重算」。
- **要做**：① `engine_ops::start_duplicates`（`compute_hashes` + `group_duplicates`，后台 + 进度）；
  ② 弹窗真出分组（组内缩略图、保留首张、其余一键批量操作）；③ 阈值/结果落 `folder_db`；
  ④ 接 UI 前先用真实目录量一遍阈值 10 的分组质量。
- **估工**：1–1.5 天（原估 2–3 天，因引擎侧已完成而下调）。
- **已完成（2026-09-22 · Batch 1）**：
  ① **引擎**：`phash::compute_hashes` 加取消参数（取消返回 `Ok(None)`，**不返回半份结果**，新增 1 个单测）；
     `folder_db` 新增 **`duplicates` 表**（migration 10：`rel_path/group_index/keeper/threshold/computed_at`）
     + `replace_duplicates` / `load_duplicates`（顺手清磁盘已不存在的幽灵行）/ `clear_duplicates`（3 个单测）。
  ② **UI 纯逻辑** 新增 `model/duplicates.rs`：`duplicate_scope` = 当前目录全部照片，**剔除同 stem 多格式组**
     （RAW+JPEG 是同一画面，dHash 必然近似，放进来只会制造假重复）；`group_views`（组内首张 = keeper）、
     `summarize`、`to_rel_groups` / `to_full_groups`（5 个单测）。
  ③ **引擎桥**：`engine_ops::start_duplicates`（前台取作用域 + 阈值快照 → 后台逐张 dHash 聚类 → 200ms 进度
     → 落内存 + 落 `folder_db`；检测途中切目录则丢弃结果）+ `delete_duplicate_extras`（复用 `delete_paths`：
     回收站 + 重扫）。
  ④ **弹窗**（`views/dialogs/duplicates_dialog.rs` 从占位改真件）：阈值档位 **6/8/10/12/16**（手册 §9.10 档位，
     默认 10）+ 进度条 + 取消 + 汇总行（组数 / 多余张数 / 阈值 / 时间）+ 分组卡片（组内缩略图，首张带「保留」
     徽标）+ 两个动作：**「其余标 Rejected」**（非破坏性，手册原始规格，`set_flag_for_paths` 同步 folder_db 的 xmp）
     与 **「保留首张，其余移入回收站」**（二次确认后 `delete_paths`）。
  ⑤ **入口 / 状态**：左栏入口改走 `open_duplicates_dialog`（先把落库结果读回来，重启免重算）；Esc 关窗时若
     检测进行中顺手取消（否则用户失去取消入口）；状态栏加重复检测分支（进度 + 取消）。
- **阈值标定（第 ④ 条，工具 `cargo run -p photo-engine --example dup_calibrate -- <目录>`）**：654 张真实鸟照
  （缩略图缓存命中，debug 39.9s）最近邻距离直方图 = 0-7 段 54 张 / 8-11 段 39 / 12-15 段 157 / 16-19 段 330
  （无关照片主峰）/ 20+ 段 74 → **两峰之间没有干净低谷**；阈值 10 得 30 组、75 张入组、最大组 8。人工核对：
  真重复（`- 副本`、`_1`、`已增强-降噪` 与原图同画面）**全部命中**，但也出现**同构图连拍串链**（最大组 8 张横跨
  两年）→ 所以「其余」的默认动作必须显式确认，**不能自动删**（实现与之一致：二次确认 + 另给非破坏性标 Rejected）。
- **验证**：`cargo check --workspace --all-targets` 0 warning；`cargo test` 全绿（photo-engine 185→**189**、
  photo-ui 74→**79**）；新增无头冒烟 **`duplicates_smoke` 21 项**全过（入口开窗 / 作用域 8 张 / 2 组各 2 张 /
  无关照片不误报 / keeper = 路径首张 / 落库阈值 + keeper + 时间 / 标 Rejected 不动文件且同步 xmp / 重开弹窗
  读回落库结果 / 同 stem 多格式被排除 / 渲染多帧不 panic）；Xvfb 截图目检到阈值 chips、汇总行
  「2 组 · 2 张多余 · 阈值 10 · 2026-09-22 18:26:20」与两张分组卡片；`lease_smoke` / `stats_smoke` /
  `export_smoke` / `grid_scroll_smoke` / `filter_bar_smoke` / `adjust_smoke` 回归全过。
- **遗留**：① 分组是贪心锚点单链聚类，同构图连拍会串成大组（UI 已用「keeper + 显式确认」兜住；要更准见 #16）；
  ② 结果按目录落库、不跨目录共享（重复检测本就是目录级体检）；③ 递归扫描模式的子目录照片不参与（扫描器单层）。

### 3. ✅ 胶片条横向滚动条

- 手册 §9.6 要求自绘；未做。
- **估工**：2 小时。
- **已完成（2026-09-22 · commit 见 Batch 2）**：胶片条改用公用 `views/scroll_area.rs::scroll_area_h`
  （内容宽 = n × 96 + (n−1) × 6 + padding，显式撑宽 + `track_scroll` + 底部叠 `Scrollbar::horizontal`）。
  验证：`stats_smoke` 新增「胶片条横向可滚（16 张，视口宽 780、可滚 862）」全过。

### 4. ✅ 列表滚动条覆盖不全

- 统计页左栏排行、统计页右栏照片网格、导入弹窗内容都还是裸 `overflow_y_scroll()`，**没有滚动条**
  （网格视图已于 2026-09-19 修好，做法见 AGENTS 对应记录）。
- **要做**：抽一个公用 wrapper（`track_scroll` + `Scrollbar::vertical` 叠层），三处复用。
- **估工**：2 小时。
- **已完成（2026-09-22 · commit 见 Batch 2）**：新增 `crates/photo-ui/src/views/scroll_area.rs`
  （`scroll_area_v` / `scroll_area_h`），四处复用（统计页左栏 + 右栏、导入弹窗内容、胶片条），
  `AppState` 持有 4 个 `ScrollHandle`。**踩到的坑**：内容不加 `flex_shrink_0`（横向不加显式宽度）
  时 flex 会把内容压到容器高 → `max_offset = 0`，表现成「有滚动条但滚不动」；已写进 AGENTS 已知陷阱。
  验证：`stats_smoke` 8 → 15 项（物种榜可滚 850 / 照片网格可滚 1531 / 偏移跨帧保留 / 胶片条 862 /
  导入弹窗视口 502），`lease_smoke` / `adjust_smoke` / `grid_scroll_smoke` / `filter_bar_smoke` 回归全过。

## 🟡 P2 — 需要拍板 / 落库不完整

### 5. 置信度语义与拒识阈值（**需要你定**）

- BioCLIP 的 confidence 是**余弦相似度 × 100**，实测 top-1 落在 65–79 这个窄带里，
  **跨物种不可比**（0.75 对鸟算高，对植物算什么不知道）；UI 这一列仍叫「置信度」。
- **没有拒识阈值**：实测**找不到干净阈值**（纯噪声图 0.711 > 真实照片最小值 0.645）；
  相对指标（top1 − top2 间隔）可能更靠谱。
- **要定**：① 是否把这列改叫「相似度」；② 是否上「间隔」类相对判据。
- 参考：`docs/open-questions.md` §1。

### 6. 中文名缺失时的展示形态（**需要你定**）

- 中国包内 94% 的标签有中文名；全量空间里 26% 三个中文名源都没有 → 现在**直接显示学名**。
- bioclip_demo 的做法是「属名 + 学名」（例如 `粉褶蕈属 Entoloma`）。
- 参考：`docs/open-questions.md` §5。

### 7. 中国包的地理先验（**需要你定**）

- 中国包是**硬地理先验**：境外照片、动物园、**家养动物会被强行给一个中国物种**
  （`Felis catus` 家猫、`Bubalus bubalis` 家水牛都不在 CoL China 里——拍猫得不到猫）。
- **要定**：补一个「家养 / 常见外来」小包？还是接受现状？
- 参考：`docs/open-questions.md` §2。

### 8. 世界包 / 压缩（**需要你定**）

- 世界包 = 851,968 类 / 2.62GB：要不要随包、或做成可选下载？
- f16 压缩 / PCA 降维都没做。
- 参考：`docs/open-questions.md` §2 与文末状态表。

### 9. ✅ eBird 导出（先补入口，再谈门控）

- **更正（2026-09-22 复核）**：比「按类群门控」更前置的问题是——`photo-engine/src/export_ebird.rs`
  的 `build_rows` / `write_csv` 早已就绪，但 **`crates/photo-ui/` 里 0 引用**，整个 eBird 导出
  **连入口都没有**（与 #1 同属「链路断在 UI」）。所以门控不是第一步。
- **要做**：① 接入口 + 后台导出（可复用 #1 的 `start_export` 状态机）；② 门控直接按「选中集里
  有鸟种结论才显示入口」实现，比在设置里加开关更省。
- **估工**：入口 0.5 天（门控随入口一并落）。
- **优先级**：入口部分不依赖任何拍板、可直接做（「建议顺序」第 4 位）；只有「门控规则」需要你确认。
- **已完成（2026-09-22 · commit 见 Batch 3）**：入口 = 统计页顶栏「导出记录 (CSV)」（手册 §10.6 的位置），
  `engine_ops::start_ebird_export` 前台取快照 → 后台 `export_ebird::build_rows` + `write_csv` → 状态栏报
  「已导出观鸟记录 N 条 → 路径」；目标目录复用 `AppConfig.export_dir`，文件名 `ebird_YYYYMMDD.csv`（同日重复去重）。
  门控 = 新增纯逻辑 `model/ebird.rs::ebird_candidates`（Confirmed/NeedsReview 且带物种名，与引擎计入口径一致）。
- **门控为什么不是「按类群」**：`recognition` 表**不持久化类群**（`ranks` 见 #11，暂缓），UI 无法可靠区分
  鸟与非鸟；所以退一步按「有物种结论」放行，tooltip 与文档都写明「含全部类群，非鸟记录请自行剔除」。
  **要真按类群门控，前置是把类群/`ranks` 落库**（与 #10 / #11 合并做一次 migration）。
- **验证**：`cargo check --workspace --all-targets` 0 warning；`cargo test` 全绿（photo-ui 72→74，
  新增 2 个门控单测：Confirmed+NeedsReview 计数 / Unrecognized·无物种名不计）；`export_smoke` 21 → **27 项**
  全过（造识别记录 → 门控放行 1 张 → CSV 落盘 / BOM + 表头 / 物种聚合行 `大嘴乌鸦,Corvus macrorhynchos,1,…`）。
- 参考：`docs/open-questions.md` §7（已同步：入口已补，类群门控待定）。

### 10. ⏸ 暂缓：`backend` / `asset_version` 未落库

- `recognition` 表没有这两列 → 换后端后分不清「这行是哪个识别器产生的」，只能靠手动重跑。
- 参考：ADR 0008 的实施状态。
- **暂缓理由（2026-09-22 复核）**：当前只有 BioCLIP 一个后端（ADR 0009），区分「这行是哪个
  识别器产生的」暂时无消费方；等出现第二个后端再落库，届时与 #11 合并成一次 migration。

### 11. ⏸ 暂缓：`ranks`（七级分类）未持久化

- 内存里有，落库丢了；重载后只剩学名 + 中文名。
- **暂缓理由（2026-09-22 复核）**：`folder_db.rs:361` 注释本身就写着「落库无消费方」，重载后
  展示只需学名 + 中文名，暂时没有查询/筛选用到七级分类。

### 12. ✅ 首次扫描后全局索引的 `date_taken` 缺失（原描述部分过时）

- **更正（2026-09-22 复核）**：后台补提取**早就有了**——`state/engine_ops.rs:551`
  `start_background_enrich`（由 `start_scan` 在 `:138` 触发）会在扫描后提取 EXIF 并写回
  `folder_db`，UI 侧 `date_taken` 会异步补上，**不需要「重扫一次」**。
- **真正的残留**：① `start_scan` 在扫描那一刻就用 `meta.date_taken` 组装全局索引行，此刻
  enrich 还没跑 → **首次扫描 `global.db` 行的 `date_taken` 恒为 None**；② enrich 完成后只
  `recompute_pipeline()`，**没有回写 `global_db`**，该文件夹的日期会一直缺到下次重扫。
- **要做**：enrich 完成后对当前文件夹补一次 `global_db` 同步（`replace_folder` 或增量
  `upsert_rows`）。
- **估工**：2 小时。
- **已完成（2026-09-22 · commit 见 Batch 2）**：新增 `engine_ops::rewrite_folder_index_dates`
  （只对本批补到日期、且有识别记录的照片按 rel_path 增量 upsert；`upsert_rows` 是 REPLACE 语义，
  其余列原样带回），由 `start_background_enrich` 在补提取完成后调用。消费方是
  `global_db::species_stats()` 的 `first_date` / `last_date`（UI 暂未展示，但字段与查询已就位）。
  验证：photo-ui 新增 2 个单测（回写后 `first_date` 从 None 变成补到的日期；无识别记录的照片
  回写是空操作、不新建索引行），`cargo test` 全绿（photo-ui 70→72）。

## ⚪ P3 — 打磨 / 合规 / 已知取舍

### 13. 手册 §13 设计层缺口（小活，逐条独立）

1. 批量操作与重命名依赖「必须存在激活筛选」，**没有「显式确认全量」的逃生门**。
2. **删除到回收站无确认、无撤销**（`Ctrl+Z` 只覆盖移动/复制/重命名）——与批量删除的二次确认不一致。
3. 左栏两个多选下拉（镜头 / 物种）**无方向键导航**。
4. 右键菜单无键盘导航、无最大高度滚动；网格无表格语义（虚拟列表未做行列语义）。

### 14. 手册 §13 实现层缺口

6. 面板宽度双轨（本地存储 vs 配置字段），不会跨设备同步。
7. ✅ 导出侧已记忆（`AppConfig.export_dir`，2026-09-22 · commit `686f766`）；**导入目标目录仍不记忆**（另一半未做）。

### 15. 发布与署名

- **没有 `NOTICE`**：BioCLIP 2 权重是 MIT（**要求保留版权声明**）、TreeOfLife-200M 是 CC0-1.0。
- `scripts/package.ps1` 未加 `data/taxon/` 与 VERSION 校验；`Compress-Archive` 对 2GB+ 随机 float
  压不动且慢，要换掉。
- 参考：`docs/open-questions.md` §8。

### 16. 识别管线可选增强

- org_det 多框的**交叉验证**没做——bioclip_demo 的 prune 三步只搬了「去背景 + IoU 去重 + 上限 8」。
- 代码在 `bioclip_demo/src/detect.rs` 与 `bioclip_demo/src/main.rs::prune`，约 250 行，可直接搬。

### 17. 已知取舍（近期引入，暂时接受）

- 统计页：单物种照片列表上限 **240 条**（超出提示「仅显示前 N 条」）；缩略图未生成的显示占位，
  **不在统计页触发解码生成**。
- 框选识别：**一次拖拽一次识别**（连补多个主体需再拖）。
- 识别器常驻：**约 600MB 内存长期占用**（org_det 40MB + BioCLIP int8 293MB + 名录 embedding 287MB）。
  可选做「空闲 N 分钟自动卸载」或设置页「释放识别器」按钮。
- 调整：有调整时 1:1 是**放大母版**（偏软）；RAW 调整走 8-bit 2560 母版，大范围拉曝光仍会条带
  （ADR 0007 设想的 half_size + 16-bit 母版未做）。

### 18. ✅ 小清理（条目已过期）

- 根 `Cargo.toml` 的 `quick-xml` 在 `[workspace.dependencies]` 里声明，但各 crate 的 src 无引用。
- **核实（2026-09-22）**：根 `Cargo.toml` 里**已经没有** `quick-xml` 声明；`Cargo.lock` 里的
  `quick-xml 0.41` 只是 `wayland-scanner` / `xcb` 的传递依赖（GPUI 平台层），与本仓库源码无关。
  条目按实测关闭，无需改动。

---

## 建议顺序

1. ✅ **#1 导出接线**（2026-09-22 · `686f766`，`export_smoke` 21 项）。
2. ✅ **#2 重复检测接线 + #1b 谎报按钮**（2026-09-22 · Batch 1：`phash` 接线 + `folder_db` 的 `duplicates`
   表 + 真弹窗 + `duplicates_smoke` 21 项；阈值 10 已在 654 张真实鸟照上标定，见 #2 完成记录）。
3. ✅ **#4 / #3 滚动条**（同日 Batch 2：公用 `scroll_area` + 四处复用，`stats_smoke` 15 项）。
4. ✅ **#12 全局索引 `date_taken` 回写**（同批，含 2 个单测）。
5. ✅ **#9 eBird 入口**（同日 Batch 3：统计页「导出记录 (CSV)」+ 门控纯逻辑，`export_smoke` 27 项）。
6. ⏭ **#5 / #6 语义收敛（Batch 2，已拍板可直接做）**——「置信度」列改「相似度」+ 信息栏展示
   `top1−top2` 间隔、**不上硬拒识阈值**（实测无干净阈值）；中文名缺失回落「属名 + 学名」。
   **#7 / #8 资产类按需触发**：#7 暂不补家养/外来小包（只把「境内先验」写进说明）、#8 世界包不随包。
7. **⏸ #10 / #11**——收益低于维护成本，等第二个识别后端出现再动。
8. **P3（#13–#18）**——发包前先把 #15（NOTICE / 打包校验）做掉；#18 已核实为过期条目并关闭。
9. ⏭ **Batch 3（一次 migration）**：#11 `ranks` + #10 `backend`/`asset_version` 落库，随后把 #9 的 eBird
   门控从「有物种结论」收紧成「鸟纲」——这两条此前暂缓是因为**没有消费方**，现在有了。

## 维护约定

- **做完一条**：标题前加 `✅`，下面补一行「已完成：日期 · commit · 验证方式」。**不要删条目**，
  免得以后重复讨论同一条。
- **新增待办**：放进对应优先级段落，并带上**可核实的证据**（`文件:行` / 实测数字 / 全仓 grep 结论），
  别只写感受。
- **与 `open-questions.md` 同步**：需要人拍板的项在两份里都保留，改状态时一起改。