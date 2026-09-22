# 待解答的问题 —— BioCLIP 2 整合

这份文档是「**需要你拍板的**」问题清单，配 [ADR 0008](adr/0008-recognition-backend-and-taxon-scope.md)
与整合结果一起看。你出门期间我按 ADR 把 BioCLIP 后端接进了 photo_tool，实现过程中的
取舍都记在这里；**每一条都是「我按某种默认做法先做了，但可能不是你想要的」**。

> **2026-09-20 更新**：BirdModel（`bird_model.onnx`）识别后端已按要求整体移除，
> 识别器只剩 BioCLIP 一个（见 [ADR 0009](adr/0009-single-bioclip-backend.md)）。
> 下文 §1 里对 `bird_model` 置信度语义的对比、以及一切涉及「切换后端」的条目已成历史，
> 保留作决策记录。

## 一、需要你定的

### 1. 置信度的语义（影响 UI 文案与筛选）

`bird_model` 的 confidence 是 softmax 概率（0–100，同一张图内可比）；
BioCLIP 是**余弦相似度 ×100**——实测 top-1 落在 65~79 这个窄带里，不同物种之间不可比
（0.75 对鸟算高，对植物算什么不知道）。当前实现直接 `cos × 100` 填进同一列。
- 要不要在 UI 上把 BioCLIP 的这一列改叫「相似度」？
- 要不要设一个「低于 X 判 NeedsReview」的门槛？bioclip_demo 的实测结论是**没有干净的阈值**
  （纯噪声图 0.711 > 真实照片最小值 0.645），所以我没有设。相对指标（top1−top2 间隔）
  可能更靠谱，但需要你确认要不要做。

### 2. 随包的是中国包，世界包要不要

现在随包的是**中国名录子集**：93,452 类 / 287MB（`data/taxon/`），由
`bioclip_demo/data/build_taxon_pack.py --match proxy` 生成。
- 世界包 = 851,968 类 / 2.62GB。要不要也随包，或者做成可选的第二个下载？
- 中国包是**硬地理先验**：境外照片、动物园、家养动物会被强行给一个中国物种
  （实测 `Felis catus` 家猫、`Bubalus bubalis` 家水牛都不在 CoL China 里，拍猫得不到猫）。
  要不要补一个「家养/常见外来」小包？还是接受？

### 3. 一张图多个主体（当前不支持）

管线与 `recognition` 表都是**一张图一个结论**（主键 `rel_path`）。BioCLIP 的强项之一是
把多主体图拆开逐只识别（bioclip_demo 的 `--detect` + 三步 prune 有实测对比数据）。
- 要不要支持多主体？要的话 `recognition` 表主键要改成 `(rel_path, subject_index)`，
  UI（网格/信息栏/统计）都要跟着改。

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

### 5. 中文名覆盖 26% 缺失

中国包内 94% 的标签有中文名；全量空间里 26% 三个中文名源都没有。
现在的行为：没有中文名就显示**学名**（`TaxonMatch::display_name()`）。
bioclip_demo 的设计是显示「属名 + 学名」（例如 `粉褶蕈属 Entoloma`），要不要照做？

### 6. 修正/检索对话框仍只搜鸟纲 —— **已于 2026-09-22 整体删除（修正对话框 + 名录检索 API），本条关闭，见 ADR 0010**

`list_all_species` / `search_catalog` 还带 `gang_cn = '鸟纲'` 过滤（1505 种）。
本地名录本身只有 5.6 万条**动物**（昆虫 4.8 万 / 鱼 0.5 万 / 鸟 1505 / 兽 736 …），
植物与真菌 0 条。
- 要不要把 CoL China 的 16 万条（含植物/真菌中文名）并进名录？并进去的话修正对话框才能
  修正植物预测，否则植物预测只能显示不能纠正。

### 7. eBird 导出

`export_ebird` 现在仍按鸟种聚合。全物种下它只对鸟纲有意义——要不要在 UI 上按类群门控
（非鸟照片不给「导出 eBird」入口）？我没动它，只在文件头注释里标了。

### 8. 发布包与署名

BioCLIP 2 权重是 **MIT**、TreeOfLife-200M 是 **CC0-1.0**，随包分发没问题，
但 MIT 要求保留版权声明。
- 要不要在发布包里放一份 `NOTICE`（写明两者）？`scripts/package.ps1` 也要跟着加
  `data/taxon/` 与 VERSION 校验，并把 `Compress-Archive` 换掉（2GB+ 随机 float 压不动且慢）。

### 9. 性能与并发

BioCLIP int8 ViT-L 在 CPU 上实测 **0.27s/图**（48 张 12.8s），比 `bird_model` 慢一个数量级；
峰值 RSS 1.29GB（含 307MB 模型 + 287MB 文本 embedding）。
- `recognitionThreadCount` 配置项至今没有接线（AGENTS.md 里也记着）。要不要做并发 worker？

## 二、已知未做（不用你回答，只是状态记录）

顺带一条：`AGENTS.md` 里「engine 134 个测试」是陈旧数字，实际基线是 **187 passed + 1 ignored**
（`cargo test -p photo-engine`）。我改文档时会顺手订正。

### 整合过程中顺手发现/顺手做的（供你复核）

- **统计页的物种名拿不到 `display_name()`**：它来自 engine 的 `global_db::SpeciesStat.bird_name`，
  而全仓**没有任何 `species_index` 的写入方**（`GlobalDb::replace_folder`/`upsert_rows` 无人调用）。
  这看着像既有缺口（不是本次引入），但意味着统计页的数据源本身要接线；接好之后才能把
  「有中文名用中文名、没有用学名」的回落做进去。
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
| 多主体、通用检测器、拒识阈值 | 没做（见上） |
| 中国包的 GBIF 抽样验证 | 做了：`bioclip_demo/data/verify_china_pack.py`，结论与误差写在 `bioclip_demo/data/README.md` |
