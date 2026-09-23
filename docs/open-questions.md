# 待解答的问题 —— BioCLIP 2 整合

这份文档是「**需要你拍板的**」问题清单，配 [ADR 0008](adr/0008-recognition-backend-and-taxon-scope.md)
与整合结果一起看。你出门期间我按 ADR 把 BioCLIP 后端接进了 photo_tool，实现过程中的
取舍都记在这里；**每一条都是「我按某种默认做法先做了，但可能不是你想要的」**。

> **2026-09-20 更新**：BirdModel（`bird_model.onnx`）识别后端已按要求整体移除，
> 识别器只剩 BioCLIP 一个（见 [ADR 0009](adr/0009-single-bioclip-backend.md)）。
> 下文 §1 里对 `bird_model` 置信度语义的对比、以及一切涉及「切换后端」的条目已成历史，
> 保留作决策记录。

## 一、需要你定的

### 1. 相似度的语义（影响 UI 文案与筛选）—— **已定（2026-09-23）**

`bird_model` 的 confidence 是 softmax 概率（0–100，同一张图内可比）；
BioCLIP 是**余弦相似度 ×100**——实测 top-1 落在 65~79 这个窄带里，不同物种之间不可比
（0.75 对鸟算高，对植物算什么不知道）。当前实现直接 `cos × 100` 填进同一列。
- **已定**：① UI 改叫「相似度」（信息栏文案 + 手册 §9.7）；② 上相对指标 **top1 − top2 间隔**，
  与相似度同处展示（`CaptureMeta.taxon_gap`）；③ **不设硬拒识阈值**（实测没有干净阈值的结论成立）。
  实施见 `docs/todo.md` #5。

### 2. 随包的是中国包，世界包要不要

现在随包的是**中国名录子集**：93,452 类 / 287MB（`data/taxon/`），由
`bioclip_demo/data/build_taxon_pack.py --match proxy` 生成。
- 世界包 = 851,968 类 / 2.62GB。要不要也随包，或者做成可选的第二个下载？
- 中国包是**硬地理先验**：境外照片、动物园、家养动物会被强行给一个中国物种
  （实测 `Felis catus` 家猫、`Bubalus bubalis` 家水牛都不在 CoL China 里，拍猫得不到猫）。
  要不要补一个「家养/常见外来」小包？还是接受？

### 3. 一张图多个主体 —— **已于 2026-09-22 支持，见 ADR 0011**

管线与 `recognition` 表都是**一张图一个结论**（主键 `rel_path`）。BioCLIP 的强项之一是
把多主体图拆开逐只识别（bioclip_demo 的 `--detect` + 三步 prune 有实测对比数据）。
- 现状：org_det 多框 → 每主体各分类一次 → 主体列表以 `recognition.subjects` JSON 列持久化
  （主键仍 `rel_path`，顶层列恒 = 主主体，兼容旧展示/旧数据；未采用改主键方案的原因见 ADR 0011）。
- 统计（global_db 跨文件夹索引）已接线（2026-09-22）：扫描完成 `replace_folder` 全量替换、
  单张识别完成 `upsert_rows`，多主体按主体展开入库（主键扩为 `(folder, rel_path, subject_index)`）。

### 4. 通用检测器还没接线（**实测有影响**）——**已于 2026-09-22 接入，见 ADR 0010**

包里已经放了 `models/org_det.onnx`（YOLOE 开放词表 19 词，42MB），但**管线还在用单类鸟检测器**：
- 现在：鸟检测器给框 → BioCLIP 分类；**一个框都没有时退回整图识别**（这条是新加的，
  所以花卉/昆虫/真菌这类单主体照片能正常工作）。
- 实测到的代价（2026-09-19，`DSC_4110.jpg`，梅花）：单类鸟检测器在花朵照片上给了个
  **假框** `[0.696, 0.000, 0.776, 0.108]`（右上角一小块），BioCLIP 从这块裁切里照样认出了
  `Prunus mume`（梅，74.6%），但框本身是错的；另一张 `DSC_4096.jpg` 没给框、走整图回退，结果同样正确。
  bioclip_demo 里记的就是这个问题（花瓣被误检成鸟 → 整图本来正确的植物结果被挤掉），
  它的解法正是通用检测器 + 交叉验证。
- 没做：多框 + 交叉验证 + 去背景 + 去重（bioclip_demo `prune` 那三步，有实测收益）。
要不要现在做？（代码在 `bioclip_demo/src/detect.rs` 与 `main.rs::prune`，约 250 行，可直接搬）

### 5. 中文名覆盖 26% 缺失 —— **已定（2026-09-23）**

中国包内 94% 的标签有中文名；全量空间里 26% 三个中文名源都没有。
- **已定**：照 bioclip_demo 的做法——只有属/科级中文名（种级缺失）时显示
  **「属中文名 + 学名」**（如 `粉褶蕈属 Entoloma nitidum`）；三级都没有则只显示学名。
  实施见 `docs/todo.md` #6。

### 6. 修正/检索对话框仍只搜鸟纲 —— **已于 2026-09-22 整体删除（修正对话框 + 名录检索 API），本条关闭，见 ADR 0010**

`list_all_species` / `search_catalog` 还带 `gang_cn = '鸟纲'` 过滤（1505 种）。
本地名录本身只有 5.6 万条**动物**（昆虫 4.8 万 / 鱼 0.5 万 / 鸟 1505 / 兽 736 …），
植物与真菌 0 条。
- 要不要把 CoL China 的 16 万条（含植物/真菌中文名）并进名录？并进去的话修正对话框才能
  修正植物预测，否则植物预测只能显示不能纠正。

### 7. eBird 导出 —— **入口已补（2026-09-22），类群门控待你定**

`export_ebird` 按物种聚合。全物种下它只对鸟纲有意义——要不要在 UI 上按类群门控？

- **已做**：入口落在统计页顶栏「导出记录 (CSV)」（手册 §10.6 规定的位置），后台汇总 + 写 CSV +
  状态栏报结果；门控用「当前目录有带物种结论的照片」（`model/ebird.rs::ebird_candidates`）。
- **卡在哪**：真正的「按类群」门控做不了——`recognition` 表不持久化类群（`ranks` 见 `docs/todo.md` #11，
  已暂缓），UI 无从判断一条结论是鸟还是花。要门控就得先把类群落库（与 #10 / #11 合并一次 migration）。
- **要你定**：① 接受现状（导出含全部类群，非鸟记录自行剔除；当前实现）；② 还是先落库类群/`ranks`
  再做真正的鸟类门控（会动 schema，而 `ranks` 目前仍无消费方）。

### 8. 发布包与署名 —— **已做（2026-09-23），但查出新问题**

- **已做**：仓库根新增 `NOTICE`，逐项写明随包资产与许可：BioCLIP 2 = **MIT**（要求保留版权声明）、
  TreeOfLife-200M = **CC0-1.0**（数据集自身声明底层图片/文本混有多种 CC 许可）、
  Catalogue of Life China 名录 = **CC BY**（要求署名，`bird_catalog.db` 同源）。
  `scripts/package.ps1` 已加 `data/taxon/` 必需校验与收集（**之前根本没拷**）、`VERSION` 字段校验、
  `NOTICE`/`LICENSE` 随包，并把 `Compress-Archive` 换成系统自带 bsdtar。
- **新问题（已定，非阻塞）**：`models/org_det.onnx` 基于 **Ultralytics YOLOE**，仓库与权重是
  **AGPL-3.0**（或商业 Enterprise License）。**AGPL 是 copyleft，不是「非商业」限制，不收费不豁免它**；
  它的要求是分发时整体按 AGPL-3.0 授权 + 提供对应源码 + 附许可文本——本项目源码公开且不收费，
  三条本就满足。所以取定：**含该权重的发布包整体按 AGPL-3.0 分发（`LICENSE-AGPL-3.0.txt` 随包），
  本仓源码仍 MIT**。唯一实质影响是别人不能把你这份代码连同该权重搬进闭源产品。
  若以后想彻底避开：换一个许可宽松的检测模型（如 Apache-2.0 家系的 RT-DETR / YOLOX 类），
  或把整仓改成 AGPL-3.0。

### 9. 性能与并发

BioCLIP int8 ViT-L 在 CPU 上实测 **0.27s/图**（48 张 12.8s），比 `bird_model` 慢一个数量级；
峰值 RSS 1.29GB（含 307MB 模型 + 287MB 文本 embedding）。
- ~~`recognitionThreadCount` 配置项至今没有接线。要不要做并发 worker？~~ **已解决（2026-09-22）：实测并发无收益**（4 worker 1.10–1.61x / 内存 +0.5GB；8 worker 1.01x / 4189MB），配置项连同设置页那项**已删除**。详见 AGENTS.md 顶部配置记录与 perf 记录。

## 二、已知未做（不用你回答，只是状态记录）

顺带一条：`AGENTS.md` 的 engine 测试数随本提交更新为 **185 passed + 1 ignored**
（`cargo test -p photo-engine`，本次 +6：多主体索引展开 / replace_folder 主体收敛 / 照片去重 / 集成链路）。

### 整合过程中顺手发现/顺手做的（供你复核）

- **统计页数据源已接线（2026-09-22）**：`SpeciesRow::from_recognition` 直接取 `display_name()`
  （有中文名用中文名、没有用学名）；扫描完成 `replace_folder` 全量替换、单张识别完成 `upsert_rows`、
  删除后重扫自动清行；`species_index` 主键扩为 `(folder, rel_path, subject_index)` 支持多主体展开，
  统计按主体记录计数、照片列表按 `DISTINCT rel_path` 去重。
- **纠错弹窗以前只改内存、不落库**：改动前 photo-ui 全仓没有调用
  `folder_db.update_recognition_species` 的地方（弹窗只更新内存摘要）。本次把它接上了
  （`apply_species_correction`：内存 + folder_db + `global_db.log_correction`）。
  如果你本意是「别动这个弹窗」，回退 `views/dialogs/correct_dialog.rs` 并删掉这两个函数即可。
- **信息栏置信度曾被显示成 8850%**：`Recognition.confidence` 本来就是 0–100，旧代码又乘了 100。
  已顺手修正（人工修正的写回值同步成 100.0，与 folder_db 里存的尺度一致）。

| 项 | 状态 |
|---|---|
| `backend` / `asset_version` 落库 | **没做**。ADR 里写了要记（切换后端后要能标出「这行是另一个识别器产生的」），但 recognition 表还没加这两列；当前靠 `config.toml` 里的后端 + 手动重跑 |
| `ranks`（七级分类）持久化 | **没做**。内存里有，落库丢了；重载后只剩学名与中文名 |
| 世界包 / f16 压缩 / PCA | 没做 |
| 拒识阈值 | 没做（**已定 2026-09-23**：不设硬阈值，改用 top1−top2 间隔，见 §1） |
| 中国包的 GBIF 抽样验证 | 做了：`bioclip_demo/data/verify_china_pack.py`，结论与误差写在 `bioclip_demo/data/README.md` |
