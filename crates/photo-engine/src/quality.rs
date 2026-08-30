//! 技术质量机筛评分（QualityScore 切片）。
//!
//! 三项技术指标合成 0..1 技术分：
//! - **眼锐度**（主，权重 0.5）：鸟眼锐度分直接反映对焦/防抖质量，鸟类照片可用性的首要决定因素；
//! - **直方图剪切**（权重 0.3）：过曝/欠曝毁片，但可后期部分修复，权重次之；
//! - **检测置信度**（权重 0.2）：识别可信度与画质弱相关（可识别 ≠ 清晰），权重最低。
//!
//! `score` 为纯函数（不碰磁盘）；批量入口 `compute_scores` 读取 folder_db recognition 表
//! 取眼锐度/置信度，直方图剪切复用 `histogram::compute_histogram_from_file`（调用方已缓存的
//! 直方图可经 `clip_override` 注入，命中免重复解码）。

use std::collections::HashMap;
use std::path::Path;

use crate::folder_db::FolderDb;
use crate::histogram;

// ===========================================================================
// 权重常量（初值，待人工标注样片集标定；理由见模块注释）
// ===========================================================================

/// 眼锐度权重：对焦/防抖质量直接决定鸟类照片可用性，权重最高
pub const WEIGHT_EYE_SHARPNESS: f64 = 0.5;
/// 直方图剪切权重：过曝/欠曝毁片，但可后期部分修复，权重次之
pub const WEIGHT_CLIP: f64 = 0.3;
/// 检测置信度权重：识别可信度与画质弱相关，权重最低
pub const WEIGHT_CONFIDENCE: f64 = 0.2;

/// 眼锐度归一化半响应点：`sharpness = 该值` 时眼锐度分量 = 0.5。
/// 眼锐度无绝对标度（仅保证单调性，见 photo-recognize sharpness.rs），映射到 0..1
/// 需人为定标；初值待人工样片集标定（与 sharpness.rs 权重同性质）。
pub const EYE_SHARPNESS_HALF_RESPONSE: f64 = 100.0;

/// 全部分量缺失时返回的中性分：无识别/无直方图的照片按中性 0.5 计，不拔高也不拖累
pub const NEUTRAL_SCORE: f64 = 0.5;

// ===========================================================================
// 纯函数：单张合成
// ===========================================================================

/// 评分输入：各分量 Option，None = 数据缺失。
///
/// 缺失分量不参与评分——权重在可用分量间重归一化（避免缺失证据稀释已知证据）；
/// 全部分量缺失时为权重归一化边界，返回中性 [`NEUTRAL_SCORE`]。
#[derive(Debug, Clone, Default)]
pub struct QualityInputs {
    /// 鸟眼锐度分（photo-recognize 输出，仅保证单调性，无绝对标度）
    pub eye_sharpness: Option<f64>,
    /// 检测置信度（0–100，分类 softmax Top-1）
    pub detect_confidence: Option<f64>,
    /// 高光剪切像素占比（0..1：luma >= 250 像素数 / 总像素数）
    pub clip_high: Option<f64>,
    /// 死黑剪切像素占比（0..1：luma <= 5 像素数 / 总像素数）
    pub clip_low: Option<f64>,
}

/// 合成技术质量分（0..1，纯函数）。
///
/// 公式：`Σ(分量权重 × 分量值) / Σ(可用分量权重)`，各分量先归一化到 0..1：
/// - 眼锐度：`s / (s + EYE_SHARPNESS_HALF_RESPONSE)`（单调、有界）
/// - 置信度：`clamp(c / 100, 0, 1)`
/// - 剪切：`clamp(1 - 高光占比 - 死黑占比, 0, 1)`（无剪切 = 满分）
///
/// None 分量不参与（权重重归一化）；全 None 返回中性 0.5。
pub fn score(inputs: &QualityInputs) -> f64 {
    let mut weighted = 0.0f64;
    let mut weight_sum = 0.0f64;

    // 眼锐度：无绝对标度，经半响应点单调映射到 0..1
    if let Some(s) = inputs.eye_sharpness {
        weight_sum += WEIGHT_EYE_SHARPNESS;
        let s = s.max(0.0);
        weighted += WEIGHT_EYE_SHARPNESS * (s / (s + EYE_SHARPNESS_HALF_RESPONSE));
    }
    // 检测置信度：0–100 百分比归一
    if let Some(c) = inputs.detect_confidence {
        weight_sum += WEIGHT_CONFIDENCE;
        weighted += WEIGHT_CONFIDENCE * (c / 100.0).clamp(0.0, 1.0);
    }
    // 剪切：无剪切 = 满分 1.0，剪切像素占比线性扣减（占比越界钳制防脏数据）
    if inputs.clip_high.is_some() || inputs.clip_low.is_some() {
        weight_sum += WEIGHT_CLIP;
        let hi = inputs.clip_high.unwrap_or(0.0).clamp(0.0, 1.0);
        let lo = inputs.clip_low.unwrap_or(0.0).clamp(0.0, 1.0);
        weighted += WEIGHT_CLIP * (1.0 - hi - lo).clamp(0.0, 1.0);
    }

    if weight_sum <= 0.0 {
        return NEUTRAL_SCORE;
    }
    (weighted / weight_sum).clamp(0.0, 1.0)
}

// ===========================================================================
// 批量入口
// ===========================================================================

/// 完整路径 → 相对 dir 的正斜杠相对路径（folder_db recognition 表键约定：`\` → `/`）
fn rel_path_of(dir: &Path, path: &str) -> Option<String> {
    let rel = Path::new(path).strip_prefix(dir).ok()?;
    Some(rel.to_string_lossy().replace('\\', "/"))
}

/// 批量技术质量评分（任务指定的三参入口）。
///
/// 直方图复用 `histogram` 模块现有函数现场计算（engine 层无直方图缓存；调用方缓存的
/// 命中注入走 [`compute_scores_with_clip`]）。逐张回调 `progress_cb(done, total)`。
pub fn compute_scores(
    dir: &Path,
    paths: &[String],
    progress_cb: impl FnMut(usize, usize),
) -> Vec<(String, f64)> {
    compute_scores_with_clip(dir, paths, &HashMap::new(), progress_cb)
}

/// 批量技术质量评分（带直方图缓存覆盖）。
///
/// `clip_override`：完整路径 → (高光剪切占比, 死黑剪切占比)，调用方（Tauri 层
/// AppState.hist_cache）已算好的直方图命中项注入，免重复解码；未命中的路径现场
/// 调 `histogram::compute_histogram_from_file`，解码失败该分量缺失（权重重归一化）。
/// 眼锐度/置信度读 `dir/.pt/data.db` 的 recognition 表（rel 键）。返回与入参
/// `paths` 同序的 (完整路径, 技术分)。单张失败不中止整体。
pub fn compute_scores_with_clip(
    dir: &Path,
    paths: &[String],
    clip_override: &HashMap<String, (f64, f64)>,
    mut progress_cb: impl FnMut(usize, usize),
) -> Vec<(String, f64)> {
    // 识别表（rel 键 → (眼锐度, 置信度)）：只读查询，不改库。
    // 库不存在（未扫描过的目录）时不创建空库，直接按无识别记录处理
    let mut rec_map: HashMap<String, (Option<f64>, Option<f64>)> = HashMap::new();
    if dir.join(".pt").join("data.db").exists()
        && let Ok(db) = FolderDb::open_in_dir(dir)
        && let Ok(all) = db.all_recognitions()
    {
        for (rel, rec) in all {
            rec_map.insert(
                rel,
                (
                    rec.eye_sharpness.map(|v| v as f64),
                    rec.confidence.map(|v| v as f64),
                ),
            );
        }
    }

    let total = paths.len();
    let mut out = Vec::with_capacity(total);
    for (i, path) in paths.iter().enumerate() {
        let (eye, conf) = rec_map
            .get(&rel_path_of(dir, path).unwrap_or_default())
            .copied()
            .unwrap_or((None, None));
        // 剪切占比：优先命中调用方缓存覆盖，未命中现场解码（失败该分量缺失）
        let (clip_high, clip_low) = match clip_override.get(path) {
            Some(&(hi, lo)) => (Some(hi), Some(lo)),
            None => match histogram::compute_histogram_from_file(Path::new(path)) {
                Ok(h) => {
                    let total_px = h.total_pixels().max(1) as f64;
                    (
                        Some(h.clip_high_count as f64 / total_px),
                        Some(h.clip_low_count as f64 / total_px),
                    )
                }
                Err(_) => (None, None),
            },
        };
        let s = score(&QualityInputs {
            eye_sharpness: eye,
            detect_confidence: conf,
            clip_high,
            clip_low,
        });
        out.push((path.clone(), s));
        progress_cb(i + 1, total);
    }
    out
}

// ===========================================================================
// 测试
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(
        eye: Option<f64>,
        conf: Option<f64>,
        hi: Option<f64>,
        lo: Option<f64>,
    ) -> QualityInputs {
        QualityInputs {
            eye_sharpness: eye,
            detect_confidence: conf,
            clip_high: hi,
            clip_low: lo,
        }
    }

    #[test]
    fn test_score_eye_sharpness_monotonic() {
        // 其他分量固定，眼锐度升高 → 技术分严格升高（单调性契约）
        let low = score(&inputs(Some(50.0), Some(90.0), Some(0.01), Some(0.01)));
        let high = score(&inputs(Some(200.0), Some(90.0), Some(0.01), Some(0.01)));
        assert!(high > low, "眼锐度升高应提升技术分: {high} <= {low}");
        // 眼锐度为 0 时眼分量 = 0（边界最低）
        assert_eq!(score(&inputs(Some(0.0), None, None, None)), 0.0);
        // 仅眼锐度分量时分数 = 归一化后的眼分量（权重 0.5 重归一化为 1.0）
        let eye = 90.0;
        let only = score(&inputs(Some(eye), None, None, None));
        assert!((only - (eye / (eye + EYE_SHARPNESS_HALF_RESPONSE))).abs() < 1e-9);
    }

    #[test]
    fn test_score_none_neutral() {
        // None 分量中性：不稀释已知分量——剪切满分会抬升加权均值，缺失剪切时分数
        // 仅由可用分量（眼锐度）决定
        let no_clip = score(&inputs(Some(90.0), None, None, None));
        let perfect_clip = score(&inputs(Some(90.0), None, Some(0.0), Some(0.0)));
        assert!(perfect_clip > no_clip);
        // 缺失置信度与缺失剪切互不影响可用分量的权重归一（均按重归一化公式计算）
        let conf_only = score(&inputs(None, Some(50.0), None, None));
        assert!((conf_only - 0.5).abs() < 1e-9, "置信度 50 归一 = 0.5: {conf_only}");
    }

    #[test]
    fn test_score_all_none_neutral_boundary() {
        // 权重归一化边界：全部分量缺失 = 中性 0.5（不拔高也不拖累）
        assert_eq!(score(&QualityInputs::default()), NEUTRAL_SCORE);
        assert!((score(&QualityInputs::default()) - 0.5).abs() < 1e-12);
    }

    #[test]
    fn test_score_clip_and_confidence_bounds() {
        // 无剪切 + 满置信度 = 满分 1.0
        let perfect = score(&inputs(None, Some(100.0), Some(0.0), Some(0.0)));
        assert_eq!(perfect, 1.0);
        // 全剪切 = 剪切分量 0
        let clipped = score(&inputs(None, None, Some(1.0), Some(1.0)));
        assert_eq!(clipped, 0.0);
        // 置信度越界（脏数据）钳制为满分，不越界
        let conf_overflow = score(&inputs(None, Some(250.0), Some(0.0), Some(0.0)));
        assert_eq!(conf_overflow, 1.0);
        // 剪切占比总和超过 1 钳制为 0，不为负
        let overshoot = score(&inputs(None, None, Some(0.7), Some(0.7)));
        assert_eq!(overshoot, 0.0);
    }
}
