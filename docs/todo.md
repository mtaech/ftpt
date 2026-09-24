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
> **2026-09-22 Batch 1 落地（commit `1053c73`）**：#2 重复检测接线 + #1b 谎报按钮（`duplicates_smoke` 21 项）、
> #18 清理项经核实条目已过期。
>
> **2026-09-23 Batch 5 落地（commit `9a70423`，家犬跟进另见 #7）**：#7 补「家养/外来」名单并进同一包（93,480 → 93,484 类，拍猫实测 云猫 67.6% → 家猫 69.5%，家犬/家猪口径见 #7 · `cdf82e1`）、
> #8 拍板不随包世界包、#10 / #11 维持 ⏸ 并消掉「建议顺序」里要求做 Batch 3 的矛盾。
>
> **2026-09-24 Batch 8 落地：最后两条也补完了**：#13.4 后半的**无障碍语义**（网格 Grid/Row/GridCell +
> 胶片条 List/ListItem + 右键菜单 Menu/MenuItem，新增 `a11y_smoke` 21 项读原生 a11y 属性断言）
> 与 #15 的**发布包校验通道**（`package.ps1 -VerifyOnly` 跨平台跑通 + fail-closed 矩阵）。
> **现在 todo 里没有「能做而未做」的工作项了**：只剩需要 Windows 才能验的打包产物，与两处有意留痕的
> 未验证点（a11y 行列下标属性、真实误检场景）。

> **2026-09-24 Batch 7 落地**：**#13.3 / #13.4 / #17 / #16 全部做完**——
> 镜头/物种多选筛选器（用 gpui-component 的 Combobox，自带方向键导航与搜索）、
> 自绘右键菜单（Up/Down/Enter/Esc + 240px 最大高滚动，`context_menu_smoke` 20 项）、
> 识别器空闲自动卸载（默认关闭，`region_smoke` 用真实模型验证 busy 保护与自动释放）、
> org_det 多框交叉验证（`class_is_consistent` + 全丢时退回整图）。
> 仍开着的只剩「网格无表格语义」与 `package.ps1` 端到端未验证。

> **2026-09-24 Batch 6 落地**：**#13 的第 1/2 条其实是「整条链路从没接线」**——GPUI 版左栏
> 「复制到目录 / 移动到目录」弹完确认框什么都不做、批量重命名连入口都没有，而 `op_journal`
> 全仓从未 `record`（Ctrl+Z 永远回答「没有可撤销的批量操作」，手册承诺的撤销从没生效过）。
> 本轮把 `batch_ops::execute_journaled` / `ops::rename_captures_templated_journaled` /
> 回收站恢复接通，补 Delete 键确认框、批量重命名弹窗、无筛选时的「显式确认全量」逃生门，
> 新增无头冒烟 `batch_ops_smoke` 25 项；另做完 **#13 第 5 条**（信息栏失败阶段 + 最接近候选）、
> **#14 第 7 条另一半**（导入目标目录记忆）、**#1 遗留**（导出预设新建/保存/删除）、
> **#17 的「释放识别器」按钮**，并核实 **#14 第 6 条**（面板宽度已是配置单一真源）。
> **#13 第 3/4 条（左栏镜头/物种多选下拉、右键菜单）在 GPUI 版根本没有这两个控件**——
> 条目是按 Tauri 版手册写的，见下文标注。
>
> **2026-09-24 构建资源**：`.cargo/config.toml` 加 `[build] jobs = 8` +
> `rustc-wrapper = scripts/nice-rustc.sh`（rustc 及其派生的链接器降到 nice 19 / ionice 最低档），
> 前台不再被 cargo 抢占。

> **2026-09-23 Batch 4 落地（commit `9ec8a08`）**：#5 / #6 语义收敛（信息栏改「相似度 + top1−top2 间隔」、不设硬拒识阈值；
> 中文名缺失回落「属中文名 + 学名」）。

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
- **遗留（2026-09-24 全部关闭）**：手册 §9.10 的预设「新建 / 保存 / 删除」已在 Batch 6 做完
  （`model/export.rs` 的 `preset_from_draft` / `preset_index_for_draft` + 弹窗按钮 + 单测）；
  eBird 导出入口已在 Batch 3 补上（见 #9）。本条无剩余项。

### 1b. ✅ 重复检测弹窗的「开始检测」同样谎报成功（详见 #2）

- **现状**：`views/dialogs/duplicates_dialog.rs:97` 点「开始检测当前目录」只
  `set_status_message("正在进行相似照片检测...")` 随即关窗，**不做任何检测**；而入口是活的
  （`views/left_panel.rs:287`），用户现在就能撞上。
- **与 #2 的关系**：这是同一块能力的「按钮骗人」面。修 #2 时一并关闭；若 #2 延期，先做诚实化
  （禁用按钮 + 「未实现」提示），别继续骗。
- **估工**：随 #2；只做诚实化则 10 分钟。
- **已完成（2026-09-22 · 随 #2 Batch 1 · commit `1053c73`）**：弹窗改成真检测（后台 dHash + 聚类 + 真实进度条 +
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
- **已完成（2026-09-22 · Batch 1 · commit `1053c73`）**：
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
- **遗留**：① 分组是贪心锚点单链聚类，同构图连拍会串成大组（UI 已用「keeper + 显式确认」兜住；
  另注：2026-09-24 完成的 **#16 是识别管线的多框交叉验证**，与本条的照片近重复聚类不是同一件事——
  原文「要更准见 #16」指的是思路可借鉴（用分类结论给聚类兜底），不是「#16 已修好本条」）；
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

### 5. ✅ 相似度语义（UI 文案）与间隔判据

- BioCLIP 的 confidence 是**余弦相似度 × 100**，实测 top-1 落在 65–79 这个窄带里，
  **跨物种不可比**（0.75 对鸟算高，对植物算什么不知道）。
- **已完成（2026-09-23 · `9ec8a08`）**：① UI 改口径 —— 信息栏不再是裸百分数，改成「相似度 68.3% · 间隔 12.1」
  （信息栏 + 手册 §9.7）；② 上**相对判据**：`CaptureMeta.taxon_gap` = top1 − top2
  （`photo_domain::gap_to_runner_up`，`candidates` 不含 Top-1 自身，首位就是第二名），
  与相似度同处展示；③ **不上硬拒识阈值**（实测无干净阈值，共识见 open-questions §1）。
- 验证：domain 35→**37**（`display_name` 三级回落 + `gap_to_runner_up` 边界）、photo-ui 81
  （回退主体也要带回间隔）；`cargo check --workspace --all-targets` 0 warning。
- 参考：`docs/open-questions.md` §1。

### 6. ✅ 中文名缺失时的展示形态

- 中国包内 94% 的标签有中文名；全量空间里 26% 三个中文名源都没有。
- **已完成（2026-09-23 · `9ec8a08`）**：`TaxonMatch::display_name()` 改成按 `cn_level` 分口径——
  种级中文名直接用；只有属/科级中文名（种级缺失）→ **「属中文名 + 学名」**
  （`粉褶蕈属 Entoloma nitidum`，单看属名会被当成种名）；三级都没有才只显示学名。
  与 bioclip_demo `data/README.md` 的 UX 一致。返回值由 `&str` 改成 `String`（要拼接）。
- 验证：domain 新增 `test_display_name_falls_back_by_cn_level`（种/属/科/缺 四档）。
- 参考：`docs/open-questions.md` §5。

### 7. ✅ 中国包的地理先验 —— 已补「家养/外来」名单 + 家犬/家猪口径（**含一次被数据否决的尝试**）

- **原状**：中国包是**硬地理先验**：境外照片、动物园、**家养动物会被强行给一个中国物种**
  （`Felis catus` 家猫、`Bubalus bubalis` 家水牛都不在 CoL China 里——拍猫得不到猫）。
- **决定（2026-09-23）**：补名单，但**不另做第二个包**——并进现有 `data/taxon/` 一个包里（Rust 侧零改动）。
- **已完成**：`bioclip_demo/data/build_taxon_pack.py` 加 `--extra FILE`（学名精确补收，行可带
  `<TAB>中文名` 补缺/纠正）与 `--computed DIR`（追加**文本塔另算列**，并支持 `replaces` 顶替旧列）；
  名单 `bioclip_demo/data/domestic_exotic.txt`（30 个学名）、另算清单 `data/computed_taxa.json`（3 个）。
  重生成后 93,452 → **93,484 类**（93,481 来自标签空间 + 3 文本塔另算）。
  **实测对照**（两张真猫图，旧包 → 新包）：云猫 `Pardofelis marmorata` 67.6% → **家猫 `Felis catus` 69.5%**；
  云猫 64.5% → **家猫 70.4%**。中文名同步补上绵羊/山羊/马/驴，并把「吐綬雞」纠成「火鸡」。
- **家犬：试过一次，被数据否决**（这是本条最有价值的记录）：
  - 上游标签空间**没有 `Canis lupus familiaris` 条目**，只有 `Canis lupus`——而它的**上游英文俗名恰恰是
    `Domestic Dog`**（`txt_emb_bioclip-2.json` 第 354571 条；猪同理：`Sus scrofa` 俗名 `Pig`）。
    也就是说这一列本来就是狗味的文本向量，所以狗照以前就命中它、只是被我们的中文名显示成「狼」。
  - 我先按上游配方（`TreeOfLife-toolbox/.../make_txt_embedding.py` + `templates.py`：
    `an image of <七级路径> with common name <俗名>.`，L2 归一化）用 BioCLIP 2 文本塔
    （`hf-mirror` 上的 `open_clip_model.safetensors` 1.71GB）另算了 `Canis lupus familiaris` + `Gray Wolf`
    两列并顶替旧列。**8 张真狗照实测：两个文本列互有胜负、差 ≤1.5 分**（家犬赢 3 / 平 1 / 狼赢 4），
    且我们重算的列与数据集列的平均余弦只有 **0.9736**（p10 0.960）——**这个保真度差异本身就大于要区分的差距**，
    所以用文本塔分狗/狼在这套空间里不可行。**已放弃另算的犬列**，改为把这一列的中文名定成
    「家犬（狼）」（猪同理「家猪（野猪）」）。
  - 8 张真狗照（dog.ceo 临时下载，仅本地验证、未入库）：旧包 7/8 显示「狼」，新包 7/8 显示「家犬（狼）」；
    剩 1 张松狮新旧都判成川金丝猴（与本次改动无关）。
- **顺带把 3 个「名字在但没 embedding」的补上了**（`--computed`）：`Solanum tuberosum` 马铃薯、
  `Vitis vinifera` 葡萄、`Platycladus orientalis` 侧柏——它们的列号 > 851,968，落在**全零填充区**；
  另 `Sansevieria trifasciata`（虎尾兰旧名）标签空间没收录，改用现行名 `Dracaena trifasciata`（有真列可切）。
- **遗留**：野生狼/野猪的照片会显示成「家犬（狼）」「家猪（野猪）」（标签空间分不开亚种，见上）；
  境外物种/动物园动物仍会被归到相近的中国物种（要动就是 #8 世界包，已决定不随包）。
- 参考：`docs/open-questions.md` §2。

### 8. ✅ 世界包 / 压缩 —— 拍板：不随包

- 世界包 = 851,968 类 / 2.62GB。**决定（2026-09-23）**：**不随包**，也不做 f16 压缩 / PCA 降维；
  保持 `data/taxon/` 中国包（93,480 类 / 301MB）。
- **理由**：仓库与发布包体积（+2.6GB）与收益不成比例；而 #7 已把「家养/外来」这个真实痛点解掉，
  剩下的是「境外物种/动物园」这类低频场景。
- **要自制时怎么做（不随包但路径不变）**：`bioclip_demo/data/build_taxon_pack.py` 原本就支持
  `--match exact|proxy|loose` 与 `--out`；把全量标签空间当包用只需把 `class_emb/` 指向 `data/taxon/`
  （同构、Rust 侧零改动，见 bioclip_demo `data/README.md`）。
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

### 13. 手册 §13 设计层缺口（逐条独立）

1. ✅ **「显式确认全量」逃生门**（2026-09-24）：无筛选时左栏批量按钮仍默认禁用，但多了一个
   「我确认，对全目录 N 项执行」按钮解锁；解锁状态**任何筛选变化都会自动收回**
   （`AppState::recompute_pipeline`），避免「上次解锁过 → 这次清空筛选就拿到全目录权限」。
   同时补上了**批量重命名**（在此之前 GPUI 版连入口都没有）。
   验证：`batch_ops_smoke` 断言默认锁定 / 显式解锁 / 筛选一变自动收回。
2. ✅ **删除到回收站：确认 + 撤销**（2026-09-24）。**原描述还低估了**：`Ctrl+Z` 当时**不是**
   「只覆盖移动/复制/重命名」——`op_journal` 全仓从未 `record`，撤销永远是空转；左栏
   「复制/移动到目录」弹完确认框也什么都不做（引擎侧 `batch_ops::execute` 早已就绪，
   又是「链路断在 UI」）。本轮：
   - 引擎：`ops::delete_file_to_trash` 回查回收站条目 → `UndoOp::Trash`；
     `undo::restore_from_trash` 走 freedesktop/Windows 回收站 API 真恢复（macOS 报可读原因）；
     `batch_ops::execute_journaled` 与 `ops::rename_captures_templated_journaled` 产出逐文件撤销记录。
   - UI：Delete 键改为先弹确认框（`ActiveDialog::DeleteConfirm`）；`start_batch_op` /
     `start_batch_rename` 做「重扫取作用域 → 后台执行 → 记 journal → 重扫」；Ctrl+Z 报告
     成功/跳过原因，回收站恢复单独播报；左栏批量面板常驻显示上次结果（状态栏那行会被
     随后的「扫描完成…」顶掉，用户看不到「Ctrl+Z 可撤销」）。
   验证：`ops`/`undo` 新增 8 个单测（engine 191→**199**）；新冒烟 `batch_ops_smoke`
   **25 项全过**（含「确认后进回收站 → Ctrl+Z 真恢复」「移动/复制/重命名各自撤销」）。
3. ✅ **镜头 / 物种多选筛选器已建出来**（2026-09-24）。原条目写「无方向键导航」，
   实际 GPUI 版**连这两个控件都没有**（`FilterCriteria.lens_filter` / `taxon_names`
   一直没有任何 UI 入口）。本轮直接用了 gpui-component 的 **Combobox**（多选 + 可搜索 +
   自带方向键导航），所以老问题不会重现：
   - 纯逻辑 `model/filter.rs::lens_options` / `taxon_options`（去重排序；物种候选 =
     顶层展示名 + **各主体展示名**——筛选语义是「任一主体命中即保留」，只列顶层会让
     多主体照片的次要物种选不到；占位名「未识别」不进候选）
   - AppState 持有两个 `ComboboxState`，Change/Confirm 写回 criteria 并重算管线；
     候选随扫描世代在**渲染期经 defer_in 重建**（set_items 需要 Window）
   - 筛选栏展开面板加「镜头 / 物种」两行；折叠行加可清除 chip（清除时**同时**清 criteria
     与下拉选中集，否则下拉还勾着、再点一下就把筛选加回来了）；「重置全部筛选」一并清
   验证：`filter_bar_smoke` 8→**17 项**全过（候选已在渲染期同步 / 单值生效 2/3 张 /
   与镜头取交集 1/3 / 多值 = 命中任一即保留 / 清空归位 / 计入 `has_active_filters`）；
   photo-ui 85→**87**（2 个候选收集单测）。
4. ✅ **照片右键菜单已建出来（含键盘导航 + 最大高度滚动）**（2026-09-24）。原条目说的
   「无键盘导航」在 GPUI 版无从谈起——**连菜单都没有**；而 gpui-component 的
   ContextMenu/PopupMenu 只处理点击（整个 menu 模块 grep 无 `on_key_down`/`ArrowUp`），
   直接套用只会把缺口原样搬过来，所以自绘：
   - `model/context_menu.rs`：菜单项表 + 初始高亮 + 上下移动（**跳过置灰项、到边界不环绕**）
   - `views/context_menu.rs`：背景层（点空白关）+ 卡片（240px 最大高，超出走 `scroll_area_v`
     的 track_scroll + Scrollbar，贴边时往回收）；点击与悬停同步高亮
   - app.rs：根视图 `on_key_down` 处理 Up/Down/Enter（根视图持有焦点，菜单不抢焦点）；
     Esc 走 `handle_escape` 第 0 优先级
   - `engine_ops::apply_context_menu_action`：点击与回车共用一条分发——先选中命中照片，
     再 打开预览 / 复制图片 / 复制路径 / 打开所在文件夹 / Pick / Reject / 清旗标 / 评 5 星 /
     识别这一张 / 移至回收站（只弹确认框）；目标已不在目录时诚实报错、不误伤别的照片
   - 网格 cell、胶片条缩略图、预览三处都接上右键
   验证：新增 `context_menu_smoke` **20 项**全过（含「10 项超过最大高**真的产生滚动范围**
   max_offset 50px」「上下移动跳过置灰项且不环绕」「回收站只弹确认框且文件未动」）。
   **同条的「网格无表格语义」也已做完**（2026-09-24，见下方「无障碍语义」段）。
   **无障碍语义（#13.4 后半，2026-09-24 补完）**：先更正我之前的判断——GPUI（gpui-pre 0.3.4）
   **有完整的 AccessKit API**（`role` / `aria_label` / `aria_selected` /
   `aria_row_index` / `aria_column_index` / `aria_row_count` /
   `aria_column_count`），X11 与 Wayland 后端都接了 `accesskit_unix::Adapter`，
   所以这条不是「写了也拿不到效果」。做法：
   - 网格：容器 `Role::Grid`（名称「照片网格：N 张照片，M 列，已选 K 张」+ 行列数）、
     行 `Role::Row`（+ 行下标）、格子 `Role::GridCell`（+ 行列下标 + 名称 +
     `aria_selected`）；名称随状态（选中 / N 星 / Pick）实时更新
   - 胶片条：`Role::List` + 项 `Role::ListItem`（当前照片报 selected）
   - 右键菜单：`Role::Menu` + 项 `Role::MenuItem`（置灰项名称明说「（不可用）」
     ——a11y 没有 aria-disabled 通道；键盘高亮项报 `aria_selected`）
   - 文案组合收在 `model/a11y.rs`（纯函数 + 4 个单测，缺项不补占位词）
   **怎么验证的**：无头环境里 AccessKit 树不会真正下发（要 AT-SPI 客户端接入才激活），但 gpui-base 的
   `ElementSnapshot` 在元素 prepaint 时**直接读原生 a11y 属性**（与有没有读屏无关）——新增
   `a11y_smoke` **21 项**用 `gpui_kit::test::TestWindowExt::find` 读回 role / label /
   selected 断言，含「选中态迁移」「名称随 3 星 + Pick 更新」「键盘高亮项报 selected」。
   代价是 `[dev-dependencies]` 给 gpui-kit 开了 `test-support`（连带打开 gpui 的
   `leak-detection`），于是 examples 里冒烟必须 `std::process::exit(0)`——
   `lease_smoke` / `clipboard_smoke` 本轮因此暴露并修好（它们此前只 `cx.quit()`，
   检查全过却以 101 退出）。单元侧 photo-ui 92→**96**。
   **仍未做到的**：行列下标属性已挂上，但 snapshot 不暴露它们、a11y 树又要 AT 才下发，所以冒烟只能断言
   role/label/selected；索引属性靠代码评审与 GPUI 自身覆盖。另：染色/徽标等纯视觉信息未逐一进 a11y 名称
   （只进了「评分 / 旗标 / 物种」这些有语义的）。
5. ✅ **手册 §9.7 的「失败阶段中文 + 最接近候选」已接线**（2026-09-24）：
   `CaptureMeta` 新增 `failure_stage` / `candidates`（domain，最多 3 个候选，
   复用 `SubjectSummary`），由 `enrich_with_recognition` 从 `Recognition` 回填
   （数据侧 `recognition` 表早就持久化了这两列）；信息栏显示「失败原因：检测异常 /
   分类异常 / 名录映射失败 / 源图不可用」与「最接近：A 61.5% · B 55.0% · C 44.0%」，
   多主体行也从裸百分数改成「相似度 68.3%」。
   验证：domain 37→**38**（新增 Top-3 截断 + 失败阶段文案断言）。

### 14. 手册 §13 实现层缺口

6. ✅ **核实结论：GPUI 版已经是「配置单一真源」，不存在双轨**（2026-09-24）。
   `init_dock` 用 `app_config.left_panel_width/right_panel_width` 建停靠区；
   拖宽走 `DockEvent::LayoutChanged` → `sync_dock_sizes` 写回 `AppConfig` 并 350ms 去抖落盘；
   photo-ui 全仓 grep 无 `LocalStorage` / `local_storage` 之类第二条存储轨。
   「不会跨设备同步」= 配置文件本身不跨机器，属配置分发问题，不是代码缺口。
7. ✅ **两半都做完了**：导出侧 `AppConfig.export_dir`（2026-09-22 · `686f766`）；
   **导入侧 `AppConfig.import_dir`**（2026-09-24）——`open_import_dialog` 优先预填上次用过的
   目标根目录，`import::execute` 每次真正导入时写回配置。
   验证：photo-config 18→**19**（新增 `import_dir` 缺键回退 None + 往返）。

### 15. ✅ 发布与署名

- **已完成（2026-09-23 · `60b1a57`，AGPL 边界补充 `a8e506b`）**：① 新增仓库根 `NOTICE`（README「许可」一节指向它）——
  逐项列出随包分发的第三方资产与许可：BioCLIP 2（MIT，要求保留版权声明，附 Zenodo 引用）、
  TreeOfLife-200M（CC0-1.0，且数据集自身声明底层图片/文本混有多种 CC 许可，本程序只用标签文本
  embedding）、Catalogue of Life China 名录（**CC BY，要求署名**）、`bird_catalog.db`（同源）、
  onnxruntime（MIT）/ DirectML / ExifTool / LibRaw，以及本程序自身的 MIT。
  **⚠️ 核定中发现的一个新问题（已按「开源不收费」拍定，不再阻塞）**：`models/org_det.onnx` 基于
  **Ultralytics YOLOE**，而 Ultralytics 仓库与模型权重是 **AGPL-3.0**。**注意：AGPL 是 copyleft，
  不是「非商业」限制，「不收费」并不豁免它**——它的实际要求是分发时① 整个组合作品按 AGPL-3.0
  授权、② 提供完整对应源码、③ 附许可文本。本项目代码公开且不收费，三条本来就满足：
  **含 org_det.onnx 的发布包整体按 AGPL-3.0 分发，仓库自身文件仍 MIT**（MIT 允许被并入 AGPL 作品）。
  已新增 `LICENSE-AGPL-3.0.txt`（GNU 原文）并随包收集；唯一实质影响是别人不能把你这份代码
  连同该权重搬进闭源产品。要彻底避开可换许可宽松的检测模型，要简单可把整仓改 AGPL。
  ② `scripts/package.ps1`：把 `data/taxon/` 加进必需资产与收集步骤（**此前根本没拷**——
  打出来的包缺名录子集包，识别器会直接报 ModelLoad），并校验四个必需文件与
  `VERSION` 的 `schema`/`dim`/`labels` 字段；`NOTICE` / `LICENSE` / `LICENSE-AGPL-3.0.txt` 随包；
  zip 从 `Compress-Archive` 换成系统自带 bsdtar（`tar -a -c -f`，带 **bsdtar 身份校验**——
  PATH 前面若是 Git/MSYS 的 GNU tar，`-a` 不认 zip 会静默产出 tar 文件；另避开 `C:\` 参数差异），
  理由：2GB+ 上限 + 包里的 .onnx/.npy 接近随机字节，deflate 基本压不动只浪费时间。
- **校验通道（2026-09-24 补完）**：脚本加了 **`-VerifyOnly`**（只做必需资产 + `data/taxon/VERSION`
  字段校验 + 打印将要打进去的清单，不构建/不拷贝/不打包），exe 名改成平台感知（Windows `ftpt.exe` /
  其它 `ftpt`），于是**校验逻辑跨平台可跑**。本轮用镜像下的 PowerShell 7.6.6 便携版在真实仓库上跑通
  `pwsh scripts/package.ps1 -VerifyOnly`：schema 1 / dim 768 / labels **93484**，清单含
  `models/*.onnx + data/bird_catalog.db + data/taxon/{4 文件} + NOTICE/LICENSE/AGPL + exiftool/`。
  另做了 fail-closed 矩阵（假仓库注入 4 种缺口，全部 exit=1 且报错精确）：整个 `data/taxon` 缺失 /
  缺 `zh_names.json` / `VERSION` 缺 `dim` / `Cargo.toml` 无 version 且未指定 `-Version`。
- **仍未验证部分（真需要 Windows）**：`cargo build` 出的 Windows 产物 + `DirectML.dll` 收集 +
  bsdtar 打的 zip 本体（脚本里那条 bsdtar 身份校验在 Linux 上本就该失败——GNU tar 的 `-a` 不认 zip，
  这是有意设计）；要在 Windows 上确认的是「产物能启动 + 包内资产齐全」。
  bsdtar 的 `-a -c -f out.zip` 命令形状已在本地用 tar 核过，但 BSD/GNU 差异靠运行时身份校验兜住。
- 参考：`docs/open-questions.md` §8。

### 16. ✅ 识别管线可选增强 —— 多框交叉验证（2026-09-24）

- **原状**：org_det 多框只搬了 prune 三步里的「去背景 + IoU 去重 + 上限 8」，
  **交叉验证没搬**——于是「花被误检成鸟」这类假框仍可能把本来正确的整图结论挤掉。
- **已完成**：`detect::class_is_consistent`（19 个粗类 → 界/门/纲 规则，规则逐条对齐
  `bioclip_demo/src/detect.rs:186`）+ `pipeline::cross_validate_subjects`
  （分类完成后、整理结果之前过滤；全部被判为误检时**退回整图识别**，与「检测无框」同一条
  兜底；丢框后 index 重排成连续，0 恒 = 主主体；每个被丢的框打一条 debug 日志）。
- **与 demo 的有意差别**（写进代码注释）：本仓名录兜底路径的 `TaxonMatch.ranks` 是空 vec
  （`catalog.rs:54`），demo 那边路径永远来自标签空间。**没有分类证据时一律放行**，
  否则走名录兜底的主体（置信度不低的那批）会被整批误判成误检丢掉。
- 参考位置更正：`bioclip_demo` **不在本仓库**，是兄弟目录 `../bioclip_demo`（`prune` 在
  `src/main.rs:314`，一致性规则在 `src/detect.rs:186`）——原条目写成仓内路径，已按实际改。
- **验证**：`cargo test -p photo-recognize` 全绿（27→**33**：一致性 3 + 过滤 3）；
  真实模型 `crates/rawlib/img.jpg` → 东方白鹳 `Ciconia boyciana` 76.4%（单框、自洽，无回归）；
  `recognize_smoke` 5/5、`region_smoke` 19/19（多主体持久化未受影响）。
- **未验证到的部分（如实记录）**：没能在真机素材上复现「误检框被丢」的场景——org_det 是
  通用检测器，不再出现旧单类鸟检测器那种把花判成鸟的假框；该路径由 3 个单测精确覆盖。

### 17. 已知取舍（近期引入，暂时接受）

- 统计页：单物种照片列表上限 **240 条**（超出提示「仅显示前 N 条」）；缩略图未生成的显示占位，
  **不在统计页触发解码生成**。
- 框选识别：**一次拖拽一次识别**（连补多个主体需再拖）。
- 识别器常驻：**约 600MB 内存长期占用**（org_det 40MB + BioCLIP int8 293MB + 名录 embedding 287MB）。
  ✅ **设置页「释放识别器」按钮已做**（2026-09-24）：设置 → 识别与性能 → 「常驻识别器内存」，
  显示当前是否装配 + 一键释放（`engine_ops::release_recognizer`，在实体 update 之外析构，
  释放完给状态栏反馈）；下次识别/框选照旧懒装配。
  ✅ **「空闲 N 分钟自动卸载」也做了**（2026-09-24）：`model/recognizer.rs::should_unload_recognizer`
  纯判据（配置 0=关闭 / 未装配 / **识别进行中绝不释放** / 空闲未满阈值，四条缺一不释放）+
  `AppConfig.recognizer_idle_unload_minutes`（默认 **0 = 关闭**，手改越界钳到 240）+
  `engine_ops::start_recognizer_idle_watch`（30s 一次检查；`PHOTO_RECOGNIZER_IDLE_TICK_MS`
  仅供冒烟压到秒级）+ 设置页 关闭/5/15/30 分钟 档位。
  `recognizer_last_used` 在识别起点与「识别进行中」的每个 tick 都刷新，避免
  「空闲很久 → 跑一个 5 秒的批量」在批量刚结束时被判成空闲超时。
  **默认关闭是有意的**：开了之后下一次识别要多等 1-2 秒装配，划不划算由使用节奏定。
  验证：photo-config 19→**20**、photo-ui 87→**89**（判据 2 个单测）；
  `region_smoke` 16→**19 项**——用真实模型装配后验证「识别进行中不释放（busy 保护）」与
  「空闲超阈值自动释放 + 状态栏如实说明」。
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
2. ✅ **#2 重复检测接线 + #1b 谎报按钮**（2026-09-22 · Batch 1 · `1053c73`：`phash` 接线 + `folder_db` 的 `duplicates`
   表 + 真弹窗 + `duplicates_smoke` 21 项；阈值 10 已在 654 张真实鸟照上标定，见 #2 完成记录）。
3. ✅ **#4 / #3 滚动条**（同日 Batch 2：公用 `scroll_area` + 四处复用，`stats_smoke` 15 项）。
4. ✅ **#12 全局索引 `date_taken` 回写**（同批，含 2 个单测）。
5. ✅ **#9 eBird 入口**（同日 Batch 3：统计页「导出记录 (CSV)」+ 门控纯逻辑，`export_smoke` 27 项）。
6. ✅ **#5 / #6 语义收敛**（2026-09-23 · `9ec8a08`：信息栏改「相似度 + top1−top2 间隔」、不上硬拒识阈值；
   中文名缺失回落「属中文名 + 学名」；domain 35→**37** 测试）。
7. ✅ **#7 / #8 资产类拍板并落地**（同日 Batch 5 · `9a70423`）：**#7** 补「家养/外来」名单并并进同一个包
   （93,452 → **93,484 类**，Rust 零改动；两张真猫图实测：云猫 67.6% → **家猫 69.5%**；
   家犬/家猪经一次**被数据否决**的文本塔尝试后改为改中文名口径，见 #7 记录）；
   **#8** 决定**不随包**世界包（也不做 f16/PCA），自制路径不改。
8. ✅ **#10 / #11 矛盾消除**（同日）：两条维持 ⏸（无消费方），并把本段第 9 条的「Batch 3」
   明确改成**不做**——eBird 门控继续用「有物种结论」口径（非鸟自行剔除）。
9. ⏸ ~~**Batch 3（一次 migration）**：#11 `ranks` + #10 `backend`/`asset_version` 落库，随后把 #9 的 eBird
   门控从「有物种结论」收紧成「鸟纲」~~ —— **2026-09-23 拍板：不做**。理由：三条都仍无直接消费方
   （只有 BioCLIP 一个后端、七级分类无查询/筛选场景），而 eBird 导出本就面向观鸟用户、非鸟记录
   会在 CSV 里自己暴露；落库要动 schema 与全部读写/同步/迁移路径，收益不抵维护面。
   **什么时候再做**：真出现第二个识别后端，或需要「按类群筛选/统计」时——届时与 #9 门控一起做一次 migration。
10. ✅ **P3 第一批（2026-09-24 Batch 6）**：#13.1 / #13.2（批量操作与撤销整条接线 + Delete 确认 +
   「显式确认全量」逃生门，新冒烟 `batch_ops_smoke` 25 项）、#13.5（信息栏失败阶段 + 最接近候选）、
   #14.6（核实为已解决）、#14.7（导入目标目录记忆）、#1 遗留（导出预设新建/保存/删除）、
   #17 释放识别器按钮。
   **仍开着的**：**没有可做而未做的工作项了**。剩下两类：
   ① 需要 Windows 才能验的（`package.ps1` 的 Windows 产物 + DirectML.dll + bsdtar zip 本体）；
   ② 有意留痕的未验证点（a11y 的行列下标属性无法在无头环境断言；#16 的「误检框被丢」在真机上没有可复现素材）。
   #16 / #13.3 / #13.4（含无障碍语义）/ #17 均已落地，见各自条目。

11. **P3（#13–#18）**——✅ #15（NOTICE / 打包校验）已于 2026-09-23 做掉；核定中查出的
   `org_det.onnx`（Ultralytics YOLOE）**AGPL-3.0** 已按「开源不收费」定妥：
   含该权重的发布包整体按 AGPL-3.0 分发（`LICENSE-AGPL-3.0.txt` 随包），本仓源码仍 MIT。
   #18 已核实为过期条目并关闭。

## 维护约定

- **做完一条**：标题前加 `✅`，下面补一行「已完成：日期 · commit · 验证方式」。**不要删条目**，
  免得以后重复讨论同一条。
- **新增待办**：放进对应优先级段落，并带上**可核实的证据**（`文件:行` / 实测数字 / 全仓 grep 结论），
  别只写感受。
- **与 `open-questions.md` 同步**：需要人拍板的项在两份里都保留，改状态时一起改。