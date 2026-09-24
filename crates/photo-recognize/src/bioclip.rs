//! BioCLIP 2 后端：图像塔（int8 ONNX）→ 余弦检索预先算好的文本 embedding → 物种。
//!
//! **标签本身就是学名**，不需要类别号→名录的映射表：覆盖 85 万类群，
//! 本地名录只用来补中文名与名录主键（taxon_id）。
//!
//! 资产布局（`data_root()/models` 与 `data_root()/data/taxon`，随包分发）：
//!
//! ```text
//! models/bioclip2_model_int8.onnx        图像塔：image [B,3,224,224] f32 → image_features [B,768]（未归一化）
//! data/taxon/txt_emb_bioclip-2.npy       [768, K] <f4 C 序，列已 L2 归一化（名录子集，见 build_taxon_pack.py）
//! data/taxon/txt_emb_bioclip-2.json      [[七级路径, 常用名], ...]，与 npy 列一一对应
//! data/taxon/zh_names.json              {拉丁名(种/属/科): 中文名}
//! data/taxon/VERSION                    资产版本（记进结果诊断）
//! ```
//!
//! 预处理按 pybioclip 对 TreeOfLife 系列的做法：**直接 squash 到 224×224**（不保持长宽比、
//! 不中心裁剪）+ Lanczos3 + CLIP 归一化。与 `classify.rs` 的 ImageNet 预处理不是一回事。

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use image::{DynamicImage, GenericImageView, RgbImage, imageops::FilterType};
use ort::session::Session;
use ort::value::Tensor;
use photo_domain::{BBox, CnLevel, RecognitionFailureStage, TaxonCandidate, TaxonMatch};

use crate::RecognizeError;
use crate::catalog::CatalogDb;
use crate::classifier::{Classified, Classifier};

/// 图像塔输入边长
const SZ: usize = 224;
/// 图像塔输出维度
pub(crate) const DIM: usize = 768;
/// 保留的 top-k
const TOPK: usize = 5;
/// 文本类分块，控制中间分数矩阵大小
const CLASS_CHUNK: usize = 32768;

/// open_clip_config.json 的 preprocess_cfg（CLIP 归一化，不是 ImageNet）
const MEAN: [f32; 3] = [0.48145466, 0.4578275, 0.40821073];
const STD: [f32; 3] = [0.26862954, 0.26130258, 0.27577711];

/// 模型文件名与资产文件名（与 bioclip_demo 的名录子集包同名，换目录即可用）
pub const MODEL_FILE: &str = "bioclip2_model_int8.onnx";
const EMB_NPY: &str = "txt_emb_bioclip-2.npy";
const EMB_JSON: &str = "txt_emb_bioclip-2.json";
const ZH_JSON: &str = "zh_names.json";
const VERSION_FILE: &str = "VERSION";
/// 省级分布（可选资产：缺省 = 地区过滤关闭）
const REGION_JSON: &str = "bird_regions.json";

/// 名录子集资产（embedding 矩阵 + 标签 + 中文名）。
pub struct BioClipAssets {
    dim: usize,
    cols: usize,
    /// [dim, cols] 行优先；列已 L2 归一化
    emb: Vec<f32>,
    /// 每条 = (七级分类路径, 常用名)
    labels: Vec<(Vec<String>, String)>,
    /// 拉丁名（种/属/科）→ 中文名
    zh: HashMap<String, String>,
    version: Option<String>,
    /// 鸟种 → {省份: 出现记录数}（GBIF 中国坐标记录聚合；缺省 = 空，地区过滤关闭）
    bird_regions: HashMap<String, HashMap<String, u32>>,
}

impl BioClipAssets {
    /// 从资产目录加载。缺 npy/json 属系统级错误（模型不可用），中文名与 VERSION 可选。
    pub fn load(taxon_dir: &Path) -> Result<Self, RecognizeError> {
        let npy = taxon_dir.join(EMB_NPY);
        let json = taxon_dir.join(EMB_JSON);
        if !npy.exists() || !json.exists() {
            return Err(RecognizeError::ModelLoad(format!(
                "BioCLIP 资产缺失: {} 或 {}（跑一次 bioclip_demo/data/build_taxon_pack.py 生成名录子集包）",
                npy.display(),
                json.display()
            )));
        }
        let (dim, cols, emb) = load_npy(&npy)?;
        if dim != DIM {
            return Err(RecognizeError::ModelLoad(format!(
                "文本 embedding 维度应为 {DIM}，实际 {dim}（{}）",
                npy.display()
            )));
        }
        let labels: Vec<(Vec<String>, String)> = serde_json::from_slice(&std::fs::read(&json)?)?;
        if labels.len() != cols {
            return Err(RecognizeError::ModelLoad(format!(
                "标签数与 npy 列数不一致: {} vs {cols}（{}）",
                labels.len(),
                json.display()
            )));
        }
        let zh = match std::fs::read(taxon_dir.join(ZH_JSON)) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(e) => {
                tracing::warn!("没有 {}，中文名会留空: {e}", taxon_dir.join(ZH_JSON).display());
                HashMap::new()
            }
        };
        let version = parse_version(&taxon_dir.join(VERSION_FILE));
        // 省级分布是可选资产：缺失或解析失败都只关掉地区过滤，不影响识别
        let bird_regions = match std::fs::read(taxon_dir.join(REGION_JSON)) {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(map) => map,
                Err(e) => {
                    tracing::warn!(
                        "{} 解析失败，地区过滤关闭: {e}",
                        taxon_dir.join(REGION_JSON).display()
                    );
                    HashMap::new()
                }
            },
            Err(_) => {
                tracing::info!(
                    "没有 {}，地区过滤关闭（识别候选为中国包全量）",
                    taxon_dir.join(REGION_JSON).display()
                );
                HashMap::new()
            }
        };
        Ok(Self { dim, cols, emb, labels, zh, version, bird_regions })
    }

    /// 标签总数（枚举类数，诊断用）
    pub fn label_count(&self) -> usize {
        self.cols
    }
}

/// 省级行政区名称（与 `build_bird_regions.py` 用的 Aliyun DataV 区划一致），
/// 设置面板下拉与地区过滤取值共用。空串 = 全国（不过滤）。
pub const CHINA_PROVINCES: &[&str] = &[
    "北京市", "天津市", "河北省", "山西省", "内蒙古自治区", "辽宁省", "吉林省",
    "黑龙江省", "上海市", "江苏省", "浙江省", "安徽省", "福建省", "江西省",
    "山东省", "河南省", "湖北省", "湖南省", "广东省", "广西壮族自治区", "海南省",
    "重庆市", "四川省", "贵州省", "云南省", "西藏自治区", "陕西省", "甘肃省",
    "青海省", "宁夏回族自治区", "新疆维吾尔自治区", "台湾省", "香港特别行政区",
    "澳门特别行政区",
];

/// 地区过滤：把 embedding 矩阵裁剪到「该地区允许的列」，top-k 检索只在这些列里做。
///
/// 允许规则（对每个标签列）：
/// - 空地区（全国）→ 无过滤（`None`）
/// - 非鸟标签（`path[2] != "Aves"`）→ 恒允许
/// - 鸟种在 `bird_regions` 里**没有任何中国记录** → 放行（数据盲区不当证据）
/// - 鸟种有数据且目标省记录数 ≥ 1 → 允许
/// - 其余（有数据但目标省无记录）→ 排除
pub struct RegionFilter {
    /// 允许列在原始矩阵里的下标（升序）
    pub col_indices: Vec<u32>,
    /// 裁剪后的子矩阵 [dim, col_indices.len()]（列已按 col_indices 顺序搬运）
    pub emb_sub: Vec<f32>,
    pub dim: usize,
    pub cols: usize,
}

impl RegionFilter {
    pub fn active_cols(&self) -> usize {
        self.cols
    }
}

/// 纯逻辑：地区 → 允许列集合（可单测，不需模型）。
///
/// 无列被排除（含「地区数据缺失/全盲区」）时返回 `None` —— 回落无过滤路径，
/// 不给空候选也不做无意义的搬运。
pub fn build_region_filter(
    region: &str,
    labels: &[(Vec<String>, String)],
    bird_regions: &HashMap<String, HashMap<String, u32>>,
    emb: &[f32],
    dim: usize,
    full_cols: usize,
) -> Option<RegionFilter> {
    let region = region.trim();
    if region.is_empty() {
        return None;
    }
    let mut col_indices: Vec<u32> = Vec::new();
    for i in 0..full_cols {
        let (path, _common) = &labels[i];
        let is_bird = path.get(2).is_some_and(|c| c == "Aves");
        let allowed = if !is_bird {
            true
        } else {
            let sci = species_of(path);
            match bird_regions.get(&sci) {
                None => true, // 无中国记录 = 数据盲区，放行
                Some(provinces) => provinces.get(region).copied().unwrap_or(0) >= 1,
            }
        };
        if allowed {
            col_indices.push(i as u32);
        }
    }
    // 全允许 → 过滤无意义；全排除 → 宁可不过滤也不给空候选
    if col_indices.len() == full_cols || col_indices.is_empty() {
        return None;
    }
    // emb 是 [dim, full_cols] 行优先（npy 的 shape 就是 (768, K)），所以
    // 第 c 列的 768 个分量是 emb[d * full_cols + c]（**跨步 full_cols，不连续**）。
    // 子矩阵按 [dim, cols] 行优先排：emb_sub[d * cols + sub_c] = emb[d * full_cols + orig_c]。
    // （第一版按「列连续」搬运，等于把矩阵转置着抄，检索分数全错——真照片验证抓到：
    //   东方白鹳 76.4% 被抄坏成 24% 的兰花。）
    let cols = col_indices.len();
    let mut emb_sub = vec![0f32; dim * cols];
    for (sub_c, &ci) in col_indices.iter().enumerate() {
        let orig_c = ci as usize;
        for d in 0..dim {
            emb_sub[d * cols + sub_c] = emb[d * full_cols + orig_c];
        }
    }
    Some(RegionFilter { col_indices, emb_sub, dim, cols })
}

/// 从 VERSION（build_taxon_pack.py 写出的 JSON）里抠一个短版本串。
/// 读不到就返回 None —— 只影响诊断，不影响识别。
fn parse_version(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let match_rule = v.get("match")?.as_str()?;
    let labels = v.get("labels")?.as_u64()?;
    Some(format!("{match_rule}:{labels}"))
}

/// BioCLIP 后端：图像塔推理 + 余弦检索 + 名录补充。
pub struct BioClipClassifier {
    session: Session,
    assets: BioClipAssets,
    /// 地区过滤（None = 全国/无过滤）。装配时按配置地区裁剪候选。
    region: Option<RegionFilter>,
}

impl BioClipClassifier {
    pub fn new(session: Session, assets: BioClipAssets) -> Self {
        Self { session, assets, region: None }
    }

    /// 带地区过滤的装配。`region_name` 为空串 = 全国（不过滤）。
    pub fn with_region(session: Session, assets: BioClipAssets, region_name: &str) -> Self {
        let region = build_region_filter(
            region_name,
            &assets.labels,
            &assets.bird_regions,
            &assets.emb,
            assets.dim,
            assets.cols,
        );
        match &region {
            Some(f) => tracing::info!(
                "地区过滤生效：{}，允许 {}/{} 列",
                region_name,
                f.active_cols(),
                assets.cols
            ),
            None if !region_name.trim().is_empty() => tracing::info!(
                "地区 {} 无过滤数据（缺 bird_regions.json 或全为数据盲区），识别候选为中国包全量",
                region_name
            ),
            None => {}
        }
        Self { session, assets, region }
    }

    /// 当前是否启用了地区过滤（诊断/冒烟用）
    pub fn region_filter_active(&self) -> bool {
        self.region.is_some()
    }
}

impl Classifier for BioClipClassifier {
    fn classify(
        &mut self,
        catalog: &CatalogDb,
        img: &DynamicImage,
        bbox: BBox,
    ) -> Result<Classified, RecognizeError> {
        let feats = self.embed(img, bbox)?;
        let ranked = match &self.region {
            // 地区过滤：对裁剪后的子矩阵检索，再把子下标映射回原始列号
            // （class_index / candidates 仍引用中国包的原始列，与资产版本一致）
            Some(f) => rank(&feats, &f.emb_sub, f.dim, f.cols, TOPK)
                .into_iter()
                .map(|row| {
                    row.into_iter()
                        .map(|(score, sub)| (score, f.col_indices[sub as usize]))
                        .collect::<Vec<_>>()
                })
                .collect(),
            None => rank(&feats, &self.assets.emb, self.assets.dim, self.assets.cols, TOPK),
        };
        let scores = &ranked[0];
        let (top_score, top_idx) = *scores.last().ok_or(RecognizeError::ClassificationOutputEmpty)?;
        let taxon = self.build_taxon(catalog, top_idx as usize);
        let candidates = scores
            .iter()
            .rev()
            .skip(1)
            .map(|&(score, idx)| TaxonCandidate {
                class_index: idx,
                // 与 Top-1 同一标度：余弦 × 100（不是 softmax 概率，见 ADR 0008 未决项）
                confidence: score * 100.0,
                taxon: self.build_taxon(catalog, idx as usize),
            })
            .collect();
        Ok(Classified {
            taxon,
            class_index: Some(top_idx),
            confidence: Some(top_score * 100.0),
            candidates,
            // 标签本身就是物种结论 → 只要检索出了结果就算有结论（置信度阈值是未决项）
            failure: RecognitionFailureStage::None,
        })
    }

    fn backend(&self) -> &'static str {
        "bioclip"
    }

    fn asset_version(&self) -> Option<String> {
        self.assets.version.clone()
    }

    fn whole_image_on_no_detection(&self) -> bool {
        true
    }

    fn region_filter_active(&self) -> bool {
        self.region.is_some()
    }
}

impl BioClipClassifier {
    /// 裁切 → squash 224 → CLIP 归一化 → 图像塔 → L2 归一化
    fn embed(&mut self, img: &DynamicImage, bbox: BBox) -> Result<Vec<f32>, RecognizeError> {
        let data = preprocess(img, bbox);
        let input = Tensor::from_array(([1usize, 3, SZ, SZ], data.into_boxed_slice()))?;
        let outputs = self.session.run(ort::inputs![input])?;
        if outputs.len() == 0 {
            return Err(RecognizeError::ClassificationOutputEmpty);
        }
        let (_shape, flat) = outputs[0].try_extract_tensor::<f32>()?;
        if flat.len() != DIM {
            return Err(RecognizeError::ModelLoad(format!(
                "图像塔输出维度应为 {DIM}，实际 {}",
                flat.len()
            )));
        }
        let norm = flat.iter().map(|x| x * x).sum::<f32>().sqrt().max(f32::MIN_POSITIVE);
        Ok(flat.iter().map(|x| x / norm).collect())
    }

    /// 标签下标 → TaxonMatch：中文名优先用 zh_names（覆盖全部标签），名录只补主键与缺的中文名。
    fn build_taxon(&self, catalog: &CatalogDb, idx: usize) -> Option<TaxonMatch> {
        let (path, _common) = self.assets.labels.get(idx)?;
        let sci = species_of(path);
        let (cn, level) = zh_of(&self.assets.zh, path, &sci);
        let mut taxon = TaxonMatch {
            taxon_id: None,
            cn_name: cn,
            latin_name: sci,
            cn_level: level,
            ranks: path.clone(),
        };
        if let Some(from_catalog) = catalog.resolve_latin(&taxon.latin_name) {
            taxon.taxon_id = from_catalog.taxon_id;
            if taxon.cn_name.is_empty() {
                taxon.cn_name = from_catalog.cn_name;
                taxon.cn_level = CnLevel::Species;
            }
        }
        Some(taxon)
    }
}

/// 七级路径 → 学名。TreeOfLife 的 `species` 字段不一定是种加词（11317 条是
/// 「属名+种加词」或「种加词+命名人」），种加词一律小写，所以首词大写即属名。
/// 与 bioclip_demo/src/main.rs::species_of 及 build_taxon_pack.py 同一套规则。
pub(crate) fn species_of(path: &[String]) -> String {
    let genus = path.get(5).map_or("", String::as_str);
    let toks: Vec<&str> = path.get(6).map_or("", String::as_str).split_whitespace().collect();
    if toks.len() >= 2 && toks[0].starts_with(char::is_uppercase) {
        format!("{} {}", toks[0], toks[1])
    } else if let Some(first) = toks.first() {
        format!("{genus} {first}").trim().to_string()
    } else {
        genus.to_string()
    }
}

/// 中文名三级回落：种 → 属 → 科 → 无。空串不算命中。
pub(crate) fn zh_of(
    zh: &HashMap<String, String>,
    path: &[String],
    sci: &str,
) -> (String, CnLevel) {
    let genus = path.get(5).map_or("", String::as_str);
    let family = path.get(4).map_or("", String::as_str);
    for (key, level) in [(sci, CnLevel::Species), (genus, CnLevel::Genus), (family, CnLevel::Family)] {
        if let Some(v) = zh.get(key) {
            if !v.is_empty() {
                return (v.clone(), level);
            }
        }
    }
    (String::new(), CnLevel::Missing)
}

/// RGB 图 → [3,224,224] f32（squash + Lanczos3 + CLIP 归一化）。
fn preprocess(img: &DynamicImage, bbox: BBox) -> Vec<f32> {
    let (orig_w, orig_h) = img.dimensions();
    // 与 classify.rs 同一套 bbox → 像素换算：先按轴 min/max 再 clamp，避免反向框下溢
    let x1 = bbox.x1.min(bbox.x2).clamp(0.0, 1.0);
    let y1 = bbox.y1.min(bbox.y2).clamp(0.0, 1.0);
    let x2 = bbox.x2.max(bbox.x1).clamp(0.0, 1.0);
    let y2 = bbox.y2.max(bbox.y1).clamp(0.0, 1.0);
    let cx1 = ((x1 * orig_w as f32).floor() as u32).min(orig_w.saturating_sub(1));
    let cy1 = ((y1 * orig_h as f32).floor() as u32).min(orig_h.saturating_sub(1));
    let cx2 = ((x2 * orig_w as f32).ceil() as u32).max(cx1 + 1).min(orig_w.max(1));
    let cy2 = ((y2 * orig_h as f32).ceil() as u32).max(cy1 + 1).min(orig_h.max(1));
    let crop_w = (cx2 - cx1).clamp(1, orig_w.max(1));
    let crop_h = (cy2 - cy1).clamp(1, orig_h.max(1));
    let cropped: RgbImage = img.crop_imm(cx1, cy1, crop_w, crop_h).to_rgb8();
    let resized = image::imageops::resize(&cropped, SZ as u32, SZ as u32, FilterType::Lanczos3);

    let plane = SZ * SZ;
    let mut out = vec![0f32; 3 * plane];
    for (i, px) in resized.pixels().enumerate() {
        for c in 0..3 {
            out[c * plane + i] = (px[c] as f32 / 255.0 - MEAN[c]) / STD[c];
        }
    }
    out
}

/// 逐块 sgemm：feats [n, DIM] × emb [DIM, cols] → 每行 top-k 余弦，按分数升序（末尾最好）。
///
/// 检索是**内存带宽**瓶颈不是算力瓶颈（实测全量 851968 类 16ms/图，名录子集约 2ms/图）。
fn rank(
    feats: &[f32],
    emb: &[f32],
    dim: usize,
    cols: usize,
    topk: usize,
) -> Vec<Vec<(f32, u32)>> {
    assert_eq!(feats.len() % dim, 0, "特征数不是 DIM 的整数倍");
    let n = feats.len() / dim;
    let mut best: Vec<Vec<(f32, u32)>> = vec![Vec::with_capacity(topk + 1); n];
    let mut scores = vec![0f32; n * CLASS_CHUNK.min(cols)];
    for start in (0..cols).step_by(CLASS_CHUNK) {
        let cn = CLASS_CHUNK.min(cols - start);
        // SAFETY: feats 是 [n,dim] 行优先；emb 是 [dim,cols] 行优先，从第 start 列起，
        // 行跨 cols；scores 是 [n,cn] 行优先。dim/cols/cn 都由调用方与本函数保证一致。
        unsafe {
            matrixmultiply::sgemm(
                n,
                dim,
                cn,
                1.0,
                feats.as_ptr(),
                dim as isize,
                1,
                emb.as_ptr().add(start),
                cols as isize,
                1,
                0.0,
                scores.as_mut_ptr(),
                cn as isize,
                1,
            );
        }
        for (i, row) in best.iter_mut().enumerate() {
            for (j, &s) in scores[i * cn..(i + 1) * cn].iter().enumerate() {
                if row.len() == topk && s <= row[0].0 {
                    continue;
                }
                let pos = row.partition_point(|&(v, _)| v < s);
                row.insert(pos, (s, (start + j) as u32));
                if row.len() > topk {
                    row.remove(0);
                }
            }
        }
    }
    best
}

/// 读 npy：只支持 v1/v2、`<f4`、C 序、2 维。返回 (行, 列, 行优先数据)。
fn load_npy(path: &Path) -> Result<(usize, usize, Vec<f32>), RecognizeError> {
    let bad = |m: String| RecognizeError::ModelLoad(format!("{}: {m}", path.display()));
    let mut f = std::fs::File::open(path)?;
    let mut pre = [0u8; 12];
    f.read_exact(&mut pre).map_err(|e| bad(format!("npy 文件太短: {e}")))?;
    if &pre[..6] != b"\x93NUMPY" {
        return Err(bad("不是 npy 文件".into()));
    }
    let (hlen, dict_off) = match pre[6] {
        1 => (u16::from_le_bytes([pre[8], pre[9]]) as usize, 10usize),
        2 => (u32::from_le_bytes(pre[8..12].try_into().unwrap()) as usize, 12usize),
        v => return Err(bad(format!("不支持的 npy 版本 {v}"))),
    };
    let mut dict = pre[dict_off..12].to_vec();
    dict.resize(hlen, 0);
    f.read_exact(&mut dict[12 - dict_off..]).map_err(|e| bad(format!("npy header 不完整: {e}")))?;
    let header = String::from_utf8_lossy(&dict).into_owned();
    if !header.contains("<f4") {
        return Err(bad(format!("只支持 little-endian float32: {header}")));
    }
    if !header.contains("False") {
        return Err(bad(format!("只支持 C 序: {header}")));
    }
    let dims: Vec<usize> = header
        .split("'shape': (")
        .nth(1)
        .ok_or_else(|| bad("header 里没有 shape".into()))?
        .split(')')
        .next()
        .unwrap_or("")
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    if dims.len() != 2 {
        return Err(bad(format!("只支持 2 维 npy: {header}")));
    }
    let (rows, cols) = (dims[0], dims[1]);
    let n = rows.checked_mul(cols).ok_or_else(|| bad("shape 溢出".into()))?;
    let mut data = vec![0f32; n];
    // SAFETY: f32 对齐 4；'<f4' 在小端平台与本机 f32 布局相同（x86_64/aarch64），
    // 缓冲区 n*4 字节由 vec![0f32; n] 保证。
    unsafe {
        let bytes = std::slice::from_raw_parts_mut(data.as_mut_ptr().cast::<u8>(), n * 4);
        f.read_exact(bytes).map_err(|e| bad(format!("npy 数据段不完整: {e}")))?;
    }
    Ok((rows, cols, data))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn test_species_of_handles_messy_species_field() {
        assert_eq!(
            species_of(&p(&["Animalia", "Chordata", "Aves", "Passeriformes", "Muscicapidae", "Tarsiger", "cyanurus"])),
            "Tarsiger cyanurus"
        );
        // 种加词后跟命名人
        assert_eq!(
            species_of(&p(&["Archaeplastida", "Tracheophyta", "", "Rosales", "Rosaceae", "Prunus", "triloba Lindl."])),
            "Prunus triloba"
        );
        // species 自带属名且与 genus 列冲突 → 用 species 里的
        assert_eq!(
            species_of(&p(&["Animalia", "", "", "", "", "Charis", "Sarota acanthoides"])),
            "Sarota acanthoides"
        );
        // 亚种三名法只取到种
        assert_eq!(
            species_of(&p(&["Animalia", "", "", "", "", "Turdus", "merula merula"])),
            "Turdus merula"
        );
    }

    #[test]
    fn test_zh_of_three_level_fallback() {
        let path = p(&["Animalia", "Chordata", "Aves", "Passeriformes", "Muscicapidae", "Tarsiger", "cyanurus"]);
        let mut zh = HashMap::new();
        zh.insert("Muscicapidae".to_string(), "鸫科".to_string());
        assert_eq!(zh_of(&zh, &path, "Tarsiger cyanurus"), ("鸫科".to_string(), CnLevel::Family));
        zh.insert("Tarsiger".to_string(), "林鸲属".to_string());
        assert_eq!(zh_of(&zh, &path, "Tarsiger cyanurus"), ("林鸲属".to_string(), CnLevel::Genus));
        zh.insert("Tarsiger cyanurus".to_string(), "红胁蓝尾鸲".to_string());
        assert_eq!(zh_of(&zh, &path, "Tarsiger cyanurus"), ("红胁蓝尾鸲".to_string(), CnLevel::Species));
        // 空串不算命中，继续回落
        zh.insert("Tarsiger cyanurus".to_string(), String::new());
        assert_eq!(zh_of(&zh, &path, "Tarsiger cyanurus"), ("林鸲属".to_string(), CnLevel::Genus));
        // 三级都没有 → Missing，展示时会退到学名
        let empty = HashMap::new();
        assert_eq!(zh_of(&empty, &path, "Tarsiger cyanurus"), (String::new(), CnLevel::Missing));
    }

    #[test]
    fn test_rank_picks_argmax_and_orders_ascending() {
        // 2 张图 × 4 类：图 0 命中列 2，图 1 命中列 3
        let dim = DIM;
        let mut feats = vec![0f32; 2 * dim];
        feats[0] = 1.0;
        feats[dim + 1] = 1.0;
        let mut emb = vec![0f32; dim * 4];
        emb[2] = 1.0;
        emb[7] = 1.0;

        let best = rank(&feats, &emb, dim, 4, TOPK);
        assert_eq!(best[0].len(), 4);
        assert_eq!(*best[0].last().unwrap(), (1.0, 2));
        assert_eq!(*best[1].last().unwrap(), (1.0, 3));
        assert!(best[0].windows(2).all(|w| w[0].0 <= w[1].0), "top-k 应升序");
    }

    #[test]
    fn test_rank_keeps_only_topk() {
        let dim = DIM;
        let mut feats = vec![0f32; dim];
        feats[0] = 1.0;
        let cols = TOPK * 3;
        let mut emb = vec![0f32; dim * cols];
        for c in 0..cols {
            emb[c] = 1.0;
        }
        let best = rank(&feats, &emb, dim, cols, TOPK);
        assert_eq!(best[0].len(), TOPK);
    }

    #[test]
    fn test_load_npy_reads_v1_float32() {
        // 手写一个最小的 npy v1（shape (2,3)，'<f4'，C 序），验证解析器
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("t.npy");
        let mut header = b"{'descr': '<f4', 'fortran_order': False, 'shape': (2, 3), }".to_vec();
        while (10 + header.len()) % 64 != 0 {
            header.push(b' ');
        }
        header.push(b'\n');
        let mut bytes = b"\x93NUMPY".to_vec();
        bytes.push(1);
        bytes.push(0);
        bytes.extend_from_slice(&(header.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&header);
        for v in [1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0] {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        std::fs::write(&path, &bytes).unwrap();

        let (rows, cols, data) = load_npy(&path).unwrap();
        assert_eq!((rows, cols), (2, 3));
        assert_eq!(data, vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    }

    #[test]
    fn test_load_npy_rejects_non_npy() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("bad.npy");
        std::fs::write(&path, b"not an npy at all").unwrap();
        assert!(load_npy(&path).is_err());
    }

    #[test]
    fn test_build_region_filter_semantics() {
        let dim = 4;
        let mk = |path: Vec<String>| (path, String::new());
        // 0: 鸟A 有数据且在本省 → 允许；1: 鸟B 有数据但不在本省 → 排除；
        // 2: 鸟C 无数据（盲区）→ 允许；3: 非鸟（植物）→ 恒允许
        let labels = vec![
            mk(p(&["Animalia", "Chordata", "Aves", "Order", "Family", "GenusA", "spA"])),
            mk(p(&["Animalia", "Chordata", "Aves", "Order", "Family", "GenusB", "spB"])),
            mk(p(&["Animalia", "Chordata", "Aves", "Order", "Family", "GenusC", "spC"])),
            mk(p(&["Plantae", "Tracheophyta", "Magnoliopsida", "Order", "Family", "GenusD", "spD"])),
        ];
        let mut regions: HashMap<String, HashMap<String, u32>> = HashMap::new();
        regions.insert(
            "GenusA spA".into(),
            HashMap::from([("浙江省".to_string(), 3u32)]),
        );
        regions.insert(
            "GenusB spB".into(),
            HashMap::from([("云南省".to_string(), 2u32)]),
        );
        // emb 是 [dim, cols] 行优先：emb[d * 4 + c] = (d+1)*10 + c，
        // 每个 (维度,列) 取值唯一，能精确验证列搬运的布局（第一版转置着抄的病根）。
        let mut emb = vec![0f32; dim * 4];
        for d in 0..dim {
            for c in 0..4 {
                emb[d * 4 + c] = ((d + 1) * 10 + c) as f32;
            }
        }

        // 浙江省：允许 0（A 在本省）/ 2（C 无数据）/ 3（非鸟），排除 1（B 不在本省）
        let f = build_region_filter("浙江省", &labels, &regions, &emb, dim, 4).unwrap();
        assert_eq!(f.col_indices, vec![0, 2, 3]);
        assert_eq!(f.active_cols(), 3);
        // 子矩阵是 [dim, 3] 行优先：emb_sub[d * 3 + sub_c] == emb[d * 4 + col_indices[sub_c]]
        assert_eq!(f.emb_sub.len(), dim * 3);
        for (sub_c, &orig_c) in f.col_indices.iter().enumerate() {
            for d in 0..dim {
                assert_eq!(
                    f.emb_sub[d * 3 + sub_c],
                    emb[d * 4 + orig_c as usize],
                    "子矩阵布局错：d={d} sub_c={sub_c} orig_c={orig_c}"
                );
            }
        }

        // 空地区（全国）→ 无过滤
        assert!(build_region_filter("", &labels, &regions, &emb, dim, 4).is_none());
        assert!(build_region_filter("   ", &labels, &regions, &emb, dim, 4).is_none());

        // 地区数据全空（盲区）→ 全部允许 → None（回落无过滤，不给空候选）
        assert!(build_region_filter("浙江省", &labels, &HashMap::new(), &emb, dim, 4).is_none());

        // 有数据但目标省一个都不中，且还有非鸟列 → 仍产生过滤（只留非鸟）
        let f2 = build_region_filter("西藏自治区", &labels, &regions, &emb, dim, 4).unwrap();
        assert_eq!(f2.col_indices, vec![2, 3]); // C（盲区）+ 非鸟
    }

    #[test]
    fn test_build_region_filter_species_of_alignment() {
        // 学名口径必须与 species_of 一致：鸟种键是「属名 种加词」
        let path = p(&["Animalia", "Chordata", "Aves", "Passeriformes", "Corvidae", "Pica", "pica"]);
        let labels = vec![(path, String::new())];
        let mut regions = HashMap::new();
        regions.insert(
            "Pica pica".to_string(),
            HashMap::from([("浙江省".to_string(), 1u32)]),
        );
        let emb = vec![0f32; 768 * 1];
        // 非鸟列不存在，只此一列是鸟且在本省 → 全部允许 → None
        assert!(build_region_filter("浙江省", &labels, &regions, &emb, 768, 1).is_none());
        // 同名键在别省 → 排除唯一一列 → 全排除 → None（保守回退）
        let mut regions2 = HashMap::new();
        regions2.insert(
            "Pica pica".to_string(),
            HashMap::from([("云南省".to_string(), 5u32)]),
        );
        assert!(build_region_filter("浙江省", &labels, &regions2, &emb, 768, 1).is_none());
    }
}
