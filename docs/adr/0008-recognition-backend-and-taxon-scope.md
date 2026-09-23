# 0008 — 识别后端可切换 + 全物种扩展（TaxonMatch 与名录子集资产）

## 状态

已接受（设计阶段；P0 领域泛化起逐步实施）。**部分由 [ADR 0009](0009-single-bioclip-backend.md) 取代**（2026-09-20）：`BirdModel` 档位与后端切换机制已整体移除，识别器只有 BioCLIP；本 ADR 的领域泛化（`TaxonMatch`）、BioCLIP 后端与资产随包决策仍然有效。

## 背景

现有识别子系统是**鸟类闭环**：单类鸟检测器 → `bird_model.onnx`（10573 类）→ `sp_cls_map` JOIN `animal_info` → `bird_id`。实测 `data/bird_catalog.db` 里 11136 行映射只有 **1385 个类别号**能真正解析出鸟种（9746 行的拉丁名在 `animal_info` 里不存在），Mapping 失败是 NeedsReview 的主要来源。

目标改为**全物种**（不再假设是鸟），且需要能在两个识别器之间切换做 A/B。已确定：**资产随包**，不做首启下载。

关键数据（`bioclip_demo` 实测，2026-09）：

- BioCLIP 2 标签空间 851968 类（TreeOfLife-200M 文本 embedding，尾部 15487 个零占位列已裁）；f32 2.62GB / f16 1.31GB
- 全量检索是**内存带宽瓶颈、不是算力瓶颈**：sgemm 67ms/图（B=1）→ 16ms/图（B=8）→ 5.7ms/图（B=32）
- 按 CoL China（162647 行 / 162645 物种）裁出「中国包」：精确学名 58055 类；精确 ∪（科+种加词）**93452 类 → 287MB f32**（目录合计 301MB，中文名覆盖 87934 条）
- BioCLIP 2 权重 **MIT**，TreeOfLife-200M **CC0-1.0**；我们只再分发权重与派生的文本 embedding，不再分发原始图像

## 决策

### 1. 领域模型以 Taxon 为中心

名录不可能覆盖 85 万标签，所以**模型的标签本身就是结论**，本地名录只做增强：

```rust
pub struct TaxonMatch {
    pub taxon_id: Option<i64>,   // 命中本地名录才有（bird_catalog / taxon 库）
    pub latin_name: String,
    pub cn_name: String,         // 空 = 无中文名
    pub cn_level: CnLevel,       // 种 / 属 / 科 / 无 —— 三级回落的结果
    pub ranks: [String; 7],      // 界门纲目科属种,空串表示缺
}
```

- `Recognition.bird: Option<BirdMatch>` → `Recognition.taxon: Option<TaxonMatch>`
- **`Confirmed` 语义改为「模型给出了物种结论」**，不再要求名录命中；`RecognitionFailureStage::Mapping` 在全物种下基本消失，`NeedsReview` 承载「低分 / 多主体 / 检测器与分类器矛盾」这类不确定
- 中文名按 种 → 属 → 科 三级回落（bioclip_demo 的 `name_zh_level` 口径），26% 无中文名的标签显示「属中文名 + 学名」
- XMP 不涉及物种字段（只有 rating/color/flag），无需迁移

### 2. 识别后端 = 可切换的「档位」

```rust
pub enum RecognitionBackend { BioClip, BirdModel }   // photo-config, serde(default)
```

配置项进 `AppConfig`（照 `DetectionSource` 的写法），设置页「推理模型与资产状态」组里做成可选。**后端是一个档位，不是单个模型**——检测器组合跟着档位走，避免 2×2 组合：

| 档位 | 检测 | 分类 | 定位 |
|---|---|---|---|
| `BioClip`（默认） | `org_det.onnx` 通用开放词表（19 个生物界粗词）+ 可选 `detect.onnx` 补召回 | `bioclip2_model_int8.onnx` + 名录子集检索 | 全物种 |
| `BirdModel` | `detect.onnx` | `bird_model.onnx` | 鸟类快速通道 / A-B 对照 |

### 3. 代码接缝：Classifier trait（映射收进后端内部）

```rust
pub trait Classifier {
    fn classify(&mut self, img: &DynamicImage, bbox: BBox) -> Result<Recognized, RecognizeError>;
    fn label_space(&self) -> LabelSpace;   // backend id + 资产版本
}
pub struct Recognized { pub taxon: TaxonMatch, pub confidence: f32, pub candidates: Vec<TaxonCandidate> }
```

两个后端的映射方式不同（`class_index → sp_cls_map` vs `label → 学名 → taxon`），所以把「分类 + 映射」一起放进后端；`pipeline.rs` 上游（检测、eye 锐度、进度、取消、落库）零改动。

### 4. 持久化必须按后端记账

`recognition` 表新增 `backend TEXT` 与 `asset_version TEXT`（rusqlite_migration 一条 `M::up`）。理由：① 切换后端后旧结果不重跑会与当前设置不一致，UI 需要能标出「这行是另一个识别器产生的」并一键重跑；② `class_index` 的语义（`bird_model` 类号 vs BioCLIP 标签下标）依赖资产版本，换一次 embedding 不记账就会让历史结果全部错位。**主键保持 `rel_path` 不动**：双后端并存要改主键，迁移代价不划算。A/B 的形态是「全局切换 + 重跑」以及预览页「用另一个识别器再认一次」（结果只对当前选中照片，写 `recognition_compare` 轻表，不入主表）。

### 5. 资产随包 + 名录子集

默认随包**中国包**（0.30GB），世界包作为可选补充（2.62GB）。便携布局与 `data_root()` 现有约定一致：

```
ftpt.exe
├── models/  detect.onnx │ bird_model.onnx │ eye.onnx │ org_det.onnx │ bioclip2_model_int8.onnx
└── data/    bird_catalog.db │ taxon/{txt_emb_bioclip-2.npy, txt_emb_bioclip-2.json, zh_names.json, taxonomy.json, VERSION}
```

子集包与 `bioclip_demo/class_emb/` **同名同格式**，Rust 侧只是换目录读，零改代码（已实测：把 `class_emb/` 指向子集包目录即可跑通，输出从「851968 类」变成「93452 类」）。生成工具 `bioclip_demo/data/build_taxon_pack.py`（三种口径 exact / proxy / loose），验收工具 `data/verify_china_pack.py`（GBIF 中国出现记录抽样估误差）。`VERSION` 记口径/规模/来源，供第 4 条的 `asset_version` 用。

**随包的三个后果**：① 安装包从 ~76MB 资产涨到 ~0.65GB（中国包）或 ~3GB（世界包）；② `scripts/package.ps1` 需加 `data/taxon/`、加 VERSION 校验，并把 `Compress-Archive` 换掉（2GB+ 随机 float 压不动且慢），Linux 侧需补打包脚本；③ `data_root()` 的便携判定要从「有 `models/`」升级为「`models/` + `data/bird_catalog.db` + `data/taxon/VERSION`」三者齐全，否则资产不全时会静默回退到仓库根。

### 6. 检测：通用检测器 + `prune` 三步移植

全物种必须用通用主体检测器（单类鸟检测器会把荷花/花瓣误检成鸟，反而挤掉本来正确的植物结果）。`bioclip_demo` 的候选框后处理三步（**交叉验证 → 去背景 → 跨检测器去重**，顺序有回归测试）搬进 `photo-recognize`。`eye.onnx` 的鸟眼锐度只对鸟有意义，全物种下按 `class == Aves` 门控（非鸟 → `eye_sharpness = None`）。eBird 导出同理门控。

### 7. ort 统一到 workspace 的 rc.12

BioCLIP 代码移植进 `photo-recognize`，用 workspace 既有的 `ort = "=2.0.0-rc.12"`（directml + download-binaries/copy-dylibs）。**不引入 `bioclip_demo` 的 rc.13 + load-dynamic**：预发布版本互不兼容，同一个二进制里会编两份 ort/ort-sys（一份链接、一份 dlopen），等于两个 ONNX Runtime 实例。rc.12 已有 `init_from` / `metadata().custom` / `try_extract_tensor`，API 差异极小。

## 被否决的选项

| 选项 | 否决理由 |
|---|---|
| 只做鸟类（裁到 Aves，34MB） | 与「扩展到全物种」的产品方向冲突 |
| 全量标签但按名字后处理过滤地理 | 会错杀：星头啄木鸟在中国名录里叫 `Picoides canicapillus`，BioCLIP 标签是 `Yungipicus canicapillus` |
| ANN / PQ 索引压缩 | 检索本来就不是瓶颈（16ms/图），白换召回率与复杂度 |
| 两套 ort（rc.12 + rc.13 共存） | 双 ONNX Runtime 实例，内存与符号冲突风险 |
| 首启下载资产 | 已定「资产随包」 |
| sidecar CLI 调 `bioclip_demo` | 双 ORT、双解码、进度/取消/落库一致性都要重造 |
| 把中国包做成硬编码过滤 | 做成构建期资产变体，代码零分支，世界包可后补 |

## 后果

- **正面**：全物种覆盖；地理上不可能的答案从标签空间源头消失（不需要 `build_geo.py` 那条未完成的地理先验）；Mapping 失败率大幅下降；识别器可切换，A/B 有真实数据；资产小一个数量级（中国包）。
- **代价**：中国包把地理先验变成**硬先验**——境外照片、动物园、家养动物会被强行给一个中国物种（实测 `Felis catus`、`Bubalus bubalis` 都不在 CoL China 里）；CoL China 只有 162645 种，而 BioCLIP 标签空间里装得下的是 58055（精确）/93452（含代理），**这是 TreeOfLife 覆盖上限，换过滤方式解决不了**；同物异名假阴要靠 proxy 口径缓解，残余误差由 `verify_china_pack.py` 抽样量化。
  - **2026-09-23 补充**：**家养这一项已不是硬先验的受害者**——`build_taxon_pack.py --extra data/domestic_exotic.txt` 把 27 个家养/常见外来学名（家猫/家牛/家水牛/绵羊/山羊/马/驴、宠物鸟鼠鱼、栽培作物园艺花卉）并进同一个包，93,452 → **93,480 类**；实测两张真猫图旧包 → 新包 = 云猫 67.6% → 家猫 69.5%。家犬仍无解（标签空间里没有 `Canis lupus familiaris`）；境外物种/动物园仍存先验（世界包不随包，见 `docs/todo.md` #7 / #8）。
- **性能**：BioCLIP 前向约 0.3s/目标（CPU，int8 ViT-L），比 `bird_model` 慢一个数量级；全物种批量识别需要进度与取消可见（现有 250ms 收结果的机制可直接复用）。

## 实施顺序

1. **P0 领域泛化**（行为不变）：`BirdMatch → TaxonMatch`、`Recognition.taxon`、recognition 表迁移、`鸟纲` 过滤降级、34 处 UI 文案；验收 = 现有 engine 134 测试 + `photo-ui --lib` + 无头冒烟全绿。
2. **P1 BioCLIP 后端**：移植 `load_npy`/`preprocess_img`/`rank`（纯函数）+ 第 4 个 session + 名录子集加载 + `TaxonMatch` 产出；`Classifier` trait 落地。
3. **P2 通用检测器 + prune**；eye / eBird 按类群门控。
4. **P3 切换机制**：`RecognitionBackend` 配置 + 设置页 Select + `backend`/`asset_version` 记账 + 「用另一个识别器再认一次」。
5. **P4 打包**：`package.ps1` 加 taxon 资产与 VERSION 校验、`data_root()` 判定升级、Linux 打包脚本、NOTICE 里写明 BioCLIP 2（MIT）与 TreeOfLife-200M（CC0）。

## 未决

- 世界包是否随包（+2.6GB）还是做成可选的第二个安装包
- 「境外/家养」补充包要不要做（几百类、几 MB），否则拍猫永远得不到「猫」
- 非生物输入只有检测器一道闸门（合成负样本 8/8 零检出，真实场景未验证）

## 实施状态（2026-09-19，本 ADR 落地情况）

**已落地**

| 层 | 内容 |
|---|---|
| photo-domain | `TaxonMatch`（taxon_id / cn_name / latin_name / cn_level / ranks + `display_name()`）、`TaxonCandidate`、`CnLevel`；`Recognition.taxon`；`CaptureMeta.taxon_*`；`FilterCriteria.taxon_names` |
| photo-config | `RecognitionBackend { BioClip, BirdModel }`（默认 BioClip）+ `AppConfig.recognition_backend` |
| photo-recognize | `classifier.rs`（`Classifier` trait + `Classified` + `BirdModelClassifier`）、`bioclip.rs`（npy 解析 / squash+Lanczos3+CLIP 预处理 / sgemm 余弦检索 / 种属科中文名回落 / `resolve_latin` 补 taxon_id）；`whole_image_on_no_detection` = BioCLIP 在检测无框时退回整图识别 |
| photo-engine | recognition 表新增 `latin_name` / `cn_level` 两列（migration 链末尾两条 `ALTER TABLE`；老建表语句不动，与 `eye_sharpness` 同一套做法，另有测试断言「新建库与迁移库列集一致」）；`export_ebird` 适配 `taxon` |
| 资产 | `data/taxon/`（中国包 proxy：93,452 类 / 287MB / 中文名 87,934 条）+ `models/bioclip2_model_int8.onnx`（开发期硬链到 bioclip_demo，发布时真实拷贝） |

**实测（dev 构建，单张）**：`三宝鸟.png` → BioCLIP 给出 `Eurystomus orientalis` 74.8%（
名录主键命中 id=5923）、候选全是佛法僧目；同一张图 `bird_model` 给 93.5%（也正确，但候选全是
「未映射」，暴露 1385/10573 的映射缺口）。梅花 `DSC_4110/4096` → `Prunus mume`（taxon_id=None，
名录里没有植物）——一张走鸟检测器的假框、一张走整图回退，结果都对。

**未落地**（清单与需要拍板的点见 `docs/open-questions.md`）：`backend`/`asset_version` 落库、
`ranks` 持久化、通用检测器接线（实测花朵会被单类鸟检测器给假框）、多主体、拒识阈值、世界包、
批量并发（`recognitionThreadCount` 仍未接线）。
