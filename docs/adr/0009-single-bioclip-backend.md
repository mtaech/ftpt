# 0009 — 识别后端收敛为 BioCLIP 单一后端（移除 BirdModel）

## 状态

已接受（2026-09-20）。**后续见 [ADR 0010](0010-general-recognition-cleanup.md)**：
`models/bird_model.onnx` 已随资产清理删除（备份 /tmp/pt-asset-backup/），`org_det.onnx` 通用检测器已接入。

## 背景

[ADR 0008](0008-recognition-backend-and-taxon-scope.md) 把识别子系统泛化成「物种」，同时保留了
`RecognitionBackend { BioClip, BirdModel }` 两个档位与设置页切换开关，用途是 A/B 对照。落地后的实际情况：

- 产品的识别目标已明确为**通用物种**（花、虫、菌、鸟一视同仁），而 BirdModel 只认 10573 类鸟，
  与目标不符；它的类别号→名录映射（`sp_cls_map` JOIN `animal_info`）只有 1385/10573 能解析出物种
  （实测同一张三宝鸟，bird_model 置信度更高但候选全是「未映射」）。
- 两个档位意味着一份要长期维护的映射表路径、一套 224 ImageNet 预处理 + softmax 代码、
  一个额外的 26MB 模型资产，以及配置 / UI / 状态栏 / 示例工具里的分支。
- 切换机制本身也没有闭环：`backend` / `asset_version` 从未落库（ADR 0008「未落地」清单），
  切后端后旧结果无法标记，A/B 只能靠手动重跑。

## 决策

**移除 BirdModel 档位，识别器只有 BioCLIP 一个后端。**

- `photo-config`：删除 `RecognitionBackend` 枚举与 `AppConfig.recognition_backend` 字段。
  旧 `config.toml` 里的 `recognitionBackend` 键被 serde 静默忽略（`AppConfig` 无 `deny_unknown_fields`），
  保存时自然消失，无需迁移代码。
- `photo-recognize`：`Recognizer::new(models_dir, catalog_db)` 去掉 `backend` 参数，固定装配 BioCLIP；
  删除 `BirdModelClassifier`、`classify.rs`（224 预处理 + softmax Top-5）、
  `catalog.rs` 的 `resolve_class` / `resolve_top_candidates` / `query_animal` / `ClassificationOutput`
  与 `sp_cls_map` 相关测试。**保留 `Classifier` trait 与 `Classified` 接缝**——管线不关心后端实现，
  将来换分类头或接通用检测器时上游零改动。
- `photo-ui`：删除设置页「识别器后端」分段开关；模型列表把「Bird Model 分类网络」换成
  「物种分类网络（BioCLIP 2）」与「离线物种标签包（`data/taxon/`）」。
- 资产：`models/bird_model.onnx` 不再被任何代码引用（文件本身暂留磁盘，未删除）。

## 后果

- 单张 640 图 BioCLIP 前向 CPU 约 0.27–0.3s，比 BirdModel 慢一个数量级：批量识别耗时上升。
  `recognitionThreadCount` 仍未接线（并发 worker 是下一个性能手段）。
- 随包体积可再减 26MB（不再需要 `bird_model.onnx`）；`data/bird_catalog.db` 的 `sp_cls_map` 表
  成为无消费方的死表（`animal_info` 仍用于按学名补中文名与 `taxon_id`）。
- 眼模型调试工具（`debug_eye` / `calibrate_eye`）原先借 BirdModel 快速初始化，现在会一并加载
  BioCLIP 与 287MB 标签包，启动更慢——它们其实只用到 eye session。
- 待办（见 `docs/open-questions.md`）：通用检测器 `models/org_det.onnx` 仍未接线（当前被单类鸟检测器
  给假框的问题仍在）；修正 / 检索对话框仍按 `gang_cn = '鸟纲'` 过滤，全物种下植物 / 真菌预测无法人工纠正。
