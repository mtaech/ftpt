# 0011 — 多主体识别（一张照片多个识别主体）

## 状态

已接受（2026-09-22）

## 背景

[ADR 0010](0010-general-recognition-cleanup.md) 把识别管线收敛为「org_det 检测 → BioCLIP 分类」后，
`org_det.onnx` 一次能给出**多个**候选框（图内已 NMS，每张最多 300 个槽位），但管线只取最高的一个框，
一张照片仍是「一个结论」。BioCLIP 的强项是把多主体图拆开逐只识别——多主体是开放词表检测器
接上后自然的下一步（open-questions §3 的待办）。

## 决策

### 1. 检测：多框选取

`detect.rs` 的 `pick_best` 改为 `pick_subjects`：

- 过滤 `conf ≥ 0.25` 的候选；近满幅背景框（面积 ≥ 90%）在存在更小主体框时剔除
  （只有背景时退化为整幅单主体）。
- 按置信度降序贪心选取，与已保留框 IoU ≥ 0.5 视为同一主体去重。
- 上限 8 个主体（每框一次 BioCLIP，控推理成本）。

### 2. 分类：每主体各跑一次

管线对每个检测框各调一次 `Classifier::classify`（`classify_subject`），产出
`SubjectRecognition { index, bbox, taxon, class_index, confidence, candidates, failure }`；
单个主体的分类失败只让该主体 NeedsReview，不中断其他主体。

### 3. 领域模型

- `Recognition` 新增 `subjects: Vec<SubjectRecognition>`；**顶层字段恒 = 主主体（subjects[0]）**，
  既有展示 / 筛选 / 排序 / 持久化全部兼容。
- `CaptureMeta` 新增 `subjects: Vec<SubjectSummary>`（展示名 + 置信度，供网格徽标 / 信息栏 / 筛选）。

### 4. 持久化：`subjects` JSON 列（**不改主键**）

open-questions §3 原本设想把主键改成 `(rel_path, subject_index)`，**否决**：

- 改主键会波及 upsert / get / all_recognitions / copy_recognitions_to / sync / global_db 联动，
  以及既有数据的迁移，风险远大于收益；
- 顶层列保留「主主体」去规范化，单主体场景零改动、旧行（subjects NULL）读回为空 Vec、
  UI 侧以顶层字段退化为单主体，天然向后兼容。

实现：`folder_db` migration 链末尾 `ALTER TABLE recognition ADD COLUMN subjects TEXT;`，
`upsert_recognition` 写 JSON，`row_to_recognition` 读 JSON（旧行返回空 Vec）。

### 5. UI

- 网格：>1 主体的 cell 右下角显示「×N」徽标；
- 信息栏：主主体在标题行，下方列出「另有 N 个主体」；
- 筛选：`taxon_names` 命中**任一**主体的展示名即保留。

## 后果

- 多主体照片的识别耗时 = 每主体一次 BioCLIP（约 0.3s/主体，CPU int8），批量识别在后台线程，
  进度仍按照片粒度；单主体照片耗时不变。
- 顶层 `Recognition.status`：任一主体有结论 → Confirmed；全部失败 → NeedsReview；无框 → Unrecognized。
- **遗留→已做（2026-09-22 同日）**：`global_db` 跨文件夹索引原无写入方，现已接线并支持多主体：
  扫描完成 `replace_folder` 全量替换、单张识别完成 `upsert_rows`，`SpeciesRow::from_recognition`
  按主体展开（无结论的主体跳过、旧数据以顶层 taxon 退化为单主体），`species_index` 主键扩为
  `(folder, rel_path, subject_index)`（派生索引直接重建表换主键，无迁移风险）。统计按主体记录计数，
  照片列表按 `DISTINCT rel_path` 去重；删除/移出后重扫自动清行。
- `recognize_file` 示例打印各主体。

## 实测（真实照片，2026-09-22）

- `P1046603.jpg`：org_det 给 2 框 → 2 个主体，均识别为长耳鸮（71.0% / 72.5%），顶层 = 第一个；
- `P1024672.jpg`：2 个主体（黄嘴栗啄木鸟 + 另一主体）；
- 其余单主体照片结果与改前一致。
