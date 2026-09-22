# 0010 — 识别管线通用化收尾：接入 org_det、移除鸟眼锐度与修正对话框

## 状态

已接受（2026-09-22）

## 背景

[ADR 0009](0009-single-bioclip-backend.md) 把识别后端收敛为 BioCLIP 单一后端后，识别管线仍残留三类
「鸟类专属」成分，与「通用物种识别」的目标冲突：

1. **检测器还是单类鸟检测器**（`detect.onnx`，只认 `bird`）。BioCLIP 是全物种，但上游闸门只给鸟框：
   花 / 虫 / 菌照片要么一个框都不给（退整图）、要么被鸟检测器给假框（实测梅花被误检成鸟）。
   仓库里早已随包放了通用检测器 `org_det.onnx`（Ultralytics YOLOE-26s-seg，开放词表 19 个生物界粗词），
   却一直没接线。
2. **鸟眼锐度阶段**（`eye.onnx` → 关键点 → 连续锐度分）是鸟类专用的第四阶段，非鸟照片没有意义。
3. **人工修正物种对话框**（`correct_dialog`）只搜鸟纲，植物 / 真菌预测无法纠正；它带动的一整套
   名录检索 / 命中率 / 审计日志（`correction_log`）都是为「鸟种修正」设计的。

用户要求：删除 bird_model / 锐度评分相关 / 修正对话框，并接入 org_det，做成通用识别模型。

## 决策

### 1. 检测器换成 org_det（通用开放词表）

- `detect.rs` 重写为解码 `org_det.onnx` 输出 `[1,300,38]`（布局实测确定：每行
  `[x1,y1,x2,y2, conf, cls, 32×掩码系数]`，640 空间像素坐标，图内已 NMS，空槽位全 0）。
  现管线只消费框，不消费掩码（抠图 / 去背景是后续增强）。
- 候选选择：`conf ≥ 0.25` 中取最高分；近满幅（面积 ≥ 90%）的候选视为背景（整片树冠 / 天空），
  当图内存在更小的主体框时让位（实测 小黑领噪鹛：整幅 tree 0.617 vs bird 0.681）。
- `detect.onnx` 退役：代码不再加载，文件随本轮删除（备份在 /tmp/pt-asset-backup/）。
- 实测 A/B（`examples/detect_ab.rs`，已随工具用途完成删除）：鸟类照片 7 张中 6 张框位与旧检测器
  基本重合、1 张漏检（退化为整图识别，BioCLIP 仍给出正确物种）；花卉照片旧检测器 0 框、org_det
  给出 `flower 0.78` → 端到端识别出紫薇。

### 2. 鸟眼锐度整链删除

- recognize：`eye.rs`、`sharpness.rs`、`run_eye_stage`、`Recognizer::eye_session` 删除；
  `examples/debug_eye.rs` / `calibrate_eye.rs` 删除。
- domain：`EyeSharpness` 排序变体、`Recognition.eye_sharpness / eye_bbox`、
  `CaptureMeta.eye_sharpness` 删除。
- engine：`photo-engine` 的 `quality.rs`（无调用方的技术质量分死代码，眼锐度权重占 0.5）整体删除；
  `folder_db` recognition 表末尾追加 `DROP COLUMN eye_sharpness / eye_bbox` 迁移；
  `global_db` species_index 同样 DROP 锐度列。
- ui：排序下拉去掉「鸟眼锐度」档（`SortBy::EyeSharpness` 删除，旧配置值回退文件名）；
  连拍选优 `best_frame` 回到「文件尺寸 → 路径」；信息栏 / 统计页去掉锐度展示；
  设置页去掉「鸟眼关键点回归模型」条目与「鸟眼角标」文案。
- 资产：`models/eye.onnx` 删除（备份）。

### 3. 人工修正物种整链删除

- ui：`views/dialogs/correct_dialog.rs` 删除（该对话框本就无打开入口）；
  `ActiveDialog::Correct` 变体删除；`engine_ops` 的 `apply_species_correction` /
  `taxon_from_catalog_entry` 删除。
- recognize：`CatalogEntry`、`all_species / search_catalog / get_species_by_id / get_catalog_entry`
  删除，名录查询只留 BioCLIP 主路径 `resolve_latin`。
- engine：`folder_db.update_recognition_species` 删除；`global_db` 的
  `correction_log` 表、`log_correction / correction_stats / frequent_species` 删除。
- 资产：`data/bird_catalog.db` 重建为 `animal_info(id, latin_name, cn_name)` 三列
  （56,542 行不变，11.5MB → 3.9MB）；`sp_cls_map` 表与 12 个无消费方列删除。
  `data/global.db`（派生的跨文件夹索引，含已删除的锐度列）删除，重扫自动重建。

## 后果

- **正向**：检测与分类终于一致——花 / 虫 / 菌照片有正确的主体框（不再依赖整图回退兜底）；
  管线更短（无眼阶段）；随包体积减小（bird_model 26MB + eye 19MB + detect 19MB + 名录瘦身 7.6MB）。
- **反向**：org_det 对个别鸟类照片的检出置信低于旧专用检测器（实测 7 张漏 1），漏检时退化到整图识别，
  物种结论仍正确但丢失主体定位；连拍选优失去锐度判据，只能靠文件尺寸近似；人工修正不可用——
  植物 / 真菌预测只能展示，无法人工纠正（本来也只搜鸟纲，纠正价值有限）。
- **遗留**：`detect_ab.rs` / `debug_org_det.rs` 为开发期工具；`debug_org_det.rs` 保留作
  org_det 输出布局探针。掩码抠图 / 去背景、多主体、置信度阈值语义等仍是 open-questions 的待办。

## 资产变更清单

| 文件 | 处置 |
|---|---|
| `models/org_det.onnx` | 接入（保留） |
| `models/detect.onnx` / `eye.onnx` / `bird_model.onnx` | 删除（备份 /tmp/pt-asset-backup/） |
| `data/bird_catalog.db` | 瘦身重建（备份原 11.5MB） |
| `data/global.db` | 删除（派生索引，备份，重扫重建） |
