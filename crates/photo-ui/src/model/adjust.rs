//! 调整参数（ADR 0007）的纯逻辑：量化、格式化、相对路径。
//!
//! 不含 IO 与 GPUI 依赖——滑杆交互在 views 层，持久化在 state 层，
//! 这里只放可单测的换算规则。

use std::path::Path;

use photo_domain::AdjustParams;

/// 曝光范围与步进（2026-09-24 用户拍板：±3.0 EV，步进 0.3 EV）。
///
/// 原规格是 ADR 0007 的 ±2.0 / 0.05——用户反馈「太保守」：culling 里救欠曝/过曝常常要拉更多，
/// 而 0.05 的细档在挑选场景没有意义。0.3 EV ≈ 1/3 档，是相机上的常见最小档位。
pub const EXPOSURE_MIN: f32 = -3.0;
pub const EXPOSURE_MAX: f32 = 3.0;
pub const EXPOSURE_STEP: f32 = 0.3;

/// 对比度 / 饱和度范围与步进（±100，整数）
pub const TONE_MIN: f32 = -100.0;
pub const TONE_MAX: f32 = 100.0;
pub const TONE_STEP: f32 = 1.0;

/// 滑杆浮点值 → 参数曝光值：按 0.3 量化、钳制到 ±3.0，并抹掉浮点噪声（保留两位小数）。
/// 非有限值（NaN/inf）归零，等价中性。
pub fn quantize_exposure(value: f32) -> f32 {
    if !value.is_finite() {
        return 0.0;
    }
    let stepped = (value / EXPOSURE_STEP).round() * EXPOSURE_STEP;
    let clamped = stepped.clamp(EXPOSURE_MIN, EXPOSURE_MAX);
    (clamped * 100.0).round() / 100.0
}

/// 滑杆浮点值 → 参数对比度/饱和度：四舍五入到整数并钳制到 ±100。
pub fn quantize_tone(value: f32) -> i32 {
    if !value.is_finite() {
        return 0;
    }
    value.round().clamp(TONE_MIN, TONE_MAX) as i32
}

/// 曝光数值 chip 文案：`+0.35 EV` / `0.00 EV` / `-1.20 EV`
pub fn format_exposure(value: f32) -> String {
    if value == 0.0 {
        "0.00 EV".to_string()
    } else {
        format!("{value:+.2} EV")
    }
}

/// 对比度 / 饱和度数值 chip 文案：`+15` / `0` / `-40`
pub fn format_tone(value: i32) -> String {
    if value == 0 {
        "0".to_string()
    } else {
        format!("{value:+}")
    }
}

/// 文件的「相对照片目录」键（folder_db 的 adjustments / recognition 表口径）：
/// 去掉目录前缀并把分隔符统一成 `/`。路径不在目录下时返回 None。
pub fn rel_path_of(dir: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(dir).ok()?;
    Some(rel.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/"))
}

/// 是否存在非中性参数（UI 用来决定要不要走调整渲染链路）
pub fn has_adjustments(params: &AdjustParams) -> bool {
    !params.is_neutral()
}

/// 批量调整的目标集 = **选中集 ∩ 当前筛选结果**（display_order），按可见顺序返回。
///
/// 不做「无选中 = 全目录」兜底：批量改参数套错一整目录的代价远大于收益，
/// 没有选区就返回空（调用方据此禁用/提示）。
pub fn adjust_targets(selected: &[usize], display_order: &[usize]) -> Vec<usize> {
    if selected.is_empty() {
        return Vec::new();
    }
    let picked: std::collections::HashSet<usize> = selected.iter().copied().collect();
    display_order
        .iter()
        .copied()
        .filter(|i| picked.contains(i))
        .collect()
}

/// 「沿用上一张」的目标：display_order 里 current 的前一项。
/// current 为 None 或不在可见集里 → None（没有"上一张"可沿用）。
pub fn previous_in_order(current: Option<usize>, display_order: &[usize]) -> Option<usize> {
    let cur = current?;
    let pos = display_order.iter().position(|&i| i == cur)?;
    pos.checked_sub(1).map(|p| display_order[p])
}

/// 参数的一句话描述（状态栏 / 按钮提示用）：中性项省略，全中性给「无调整」。
pub fn describe_adjust(params: &AdjustParams) -> String {
    let mut parts: Vec<String> = Vec::new();
    if params.exposure != 0.0 {
        parts.push(format_exposure(params.exposure));
    }
    if params.contrast != 0 {
        parts.push(format!("对比度 {}", format_tone(params.contrast)));
    }
    if params.saturation != 0 {
        parts.push(format!("饱和度 {}", format_tone(params.saturation)));
    }
    if params.shadows != 0 {
        parts.push(format!("阴影 {}", format_tone(params.shadows)));
    }
    if params.highlights != 0 {
        parts.push(format!("高光 {}", format_tone(params.highlights)));
    }
    if params.temperature != 0 {
        parts.push(format!("色温 {}", format_tone(params.temperature)));
    }
    if params.tint != 0 {
        parts.push(format!("色调 {}", format_tone(params.tint)));
    }
    if parts.is_empty() {
        "无调整".to_string()
    } else {
        parts.join("、")
    }
}

/// 用当前参数新建一个调整预设（按 UI 同口径钳制，存进配置后与 chip 套用路径一致）。
pub fn preset_from_params(
    name: impl Into<String>,
    params: &AdjustParams,
) -> photo_config::AdjustPreset {
    photo_config::AdjustPreset {

        name: name.into(),
        exposure: params.exposure,
        contrast: params.contrast,
        saturation: params.saturation,
        shadows: params.shadows,
        highlights: params.highlights,
        temperature: params.temperature,
        tint: params.tint,
            ..photo_config::AdjustPreset::default()
        }
    .clamped()
}

/// 当前参数恰好等于哪个已存预设（chip 选中态 / 「删除当前预设」的判定口径）。
/// None = 当前参数不是任何预设 → 删除按钮应禁用，避免删错。
pub fn preset_index_for_params(
    presets: &[photo_config::AdjustPreset],
    params: &AdjustParams,
) -> Option<usize> {
    // 与导出预设的 preset_index_for_draft 同口径：**比较前先把两边都钳制**，
    // 否则库里一个 0.37 的旧参数永远对不上按 0.3 存的预设（chip 选不中、删不掉）。
    let target = preset_from_params(String::new(), params);
    presets.iter().position(|p| {
        let p = p.clone().clamped();
        p.exposure == target.exposure
            && p.contrast == target.contrast
            && p.saturation == target.saturation
            && p.shadows == target.shadows
            && p.highlights == target.highlights
            && p.temperature == target.temperature
            && p.tint == target.tint
    })
}

/// 预设 chip 文案：名字 + 参数摘要（如「夜景（+1.00 EV、对比度 +20）」）
pub fn preset_chip_label(preset: &photo_config::AdjustPreset) -> String {
    let params = AdjustParams {
        exposure: preset.exposure,
        contrast: preset.contrast,
        saturation: preset.saturation,
        shadows: preset.shadows,
        highlights: preset.highlights,
        temperature: preset.temperature,
        tint: preset.tint,
    };
    format!("{}（{}）", preset.name, describe_adjust(&params))
}

/// 调整 tab 的七类参数的定位标识（滑杆、单项重置、键盘微调共用）。
///
/// 顺序 = UI 里的行序 = 滑杆实体数组下标（index()），新增参数只要加进 ALL 即可。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdjustField {
    Exposure,
    Contrast,
    Saturation,
    Shadows,
    Highlights,
    Temperature,
    Tint,
}

impl AdjustField {
    /// 全部字段（装载/同步滑杆、渲染行、键盘微调遍历用）
    pub const ALL: [AdjustField; 7] = [
        AdjustField::Exposure,
        AdjustField::Contrast,
        AdjustField::Saturation,
        AdjustField::Shadows,
        AdjustField::Highlights,
        AdjustField::Temperature,
        AdjustField::Tint,
    ];

    /// 在 ALL 里的下标（滑杆实体数组的下标）
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|f| *f == self).unwrap_or(0)
    }

    /// 滑杆范围与步进：曝光 ±3.0 EV / 0.3，其余 ±100 / 1
    pub fn range(self) -> (f32, f32, f32) {
        match self {
            Self::Exposure => (EXPOSURE_MIN, EXPOSURE_MAX, EXPOSURE_STEP),
            _ => (TONE_MIN, TONE_MAX, TONE_STEP),
        }
    }

    /// 行标签（UI 与提示文案共用）
    pub fn label(self) -> &'static str {
        match self {
            Self::Exposure => "曝光 (EV)",
            Self::Contrast => "对比度",
            Self::Saturation => "饱和度",
            Self::Shadows => "阴影",
            Self::Highlights => "高光",
            Self::Temperature => "色温",
            Self::Tint => "色调",
        }
    }

    /// 数值 chip 文案（与滑杆量化口径一致）
    pub fn format_value(self, params: &AdjustParams) -> String {
        match self {
            Self::Exposure => format_exposure(params.exposure),
            Self::Contrast => format_tone(params.contrast),
            Self::Saturation => format_tone(params.saturation),
            Self::Shadows => format_tone(params.shadows),
            Self::Highlights => format_tone(params.highlights),
            Self::Temperature => format_tone(params.temperature),
            Self::Tint => format_tone(params.tint),
        }
    }

    /// 用滑杆原始浮点值更新一个字段（量化 + 钳制）
    pub fn apply(self, params: &mut AdjustParams, raw: f32) {
        match self {
            Self::Exposure => params.exposure = quantize_exposure(raw),
            Self::Contrast => params.contrast = quantize_tone(raw),
            Self::Saturation => params.saturation = quantize_tone(raw),
            Self::Shadows => params.shadows = quantize_tone(raw),
            Self::Highlights => params.highlights = quantize_tone(raw),
            Self::Temperature => params.temperature = quantize_tone(raw),
            Self::Tint => params.tint = quantize_tone(raw),
        }
    }

    /// 该字段对应的滑杆值（装载已存参数时同步滑杆用）
    pub fn slider_value(self, params: &AdjustParams) -> f32 {
        match self {
            Self::Exposure => params.exposure,
            Self::Contrast => params.contrast as f32,
            Self::Saturation => params.saturation as f32,
            Self::Shadows => params.shadows as f32,
            Self::Highlights => params.highlights as f32,
            Self::Temperature => params.temperature as f32,
            Self::Tint => params.tint as f32,
        }
    }

    /// 重置该字段为中性
    pub fn reset(self, params: &mut AdjustParams) {
        match self {
            Self::Exposure => params.exposure = 0.0,
            Self::Contrast => params.contrast = 0,
            Self::Saturation => params.saturation = 0,
            Self::Shadows => params.shadows = 0,
            Self::Highlights => params.highlights = 0,
            Self::Temperature => params.temperature = 0,
            Self::Tint => params.tint = 0,
        }
    }

    /// 该字段是否已是中性值
    pub fn is_neutral(self, params: &AdjustParams) -> bool {
        match self {
            Self::Exposure => params.exposure == 0.0,
            Self::Contrast => params.contrast == 0,
            Self::Saturation => params.saturation == 0,
            Self::Shadows => params.shadows == 0,
            Self::Highlights => params.highlights == 0,
            Self::Temperature => params.temperature == 0,
            Self::Tint => params.tint == 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_preset_from_params_clamps_and_matches_index() {
        // 0.37 存成 0.3（与滑杆量化同口径），否则 chip 永远对不上、删不掉
        let params = AdjustParams {

            exposure: 0.37,
            contrast: 15,
            saturation: 0,
            ..AdjustParams::default()
        };
        let preset = preset_from_params("提亮", &params);
        assert_eq!(preset.exposure, 0.3);
        assert_eq!(preset.contrast, 15);

        // 量化后的参数能找回这个预设；没量化过的原值也能（比较前先钳制）
        let list = vec![preset.clone()];
        assert_eq!(preset_index_for_params(&list, &params), Some(0));
        let quantized = AdjustParams {

            exposure: 0.3,
            contrast: 15,
            saturation: 0,
            ..AdjustParams::default()
        };
        assert_eq!(preset_index_for_params(&list, &quantized), Some(0));
        // 参数不同 → None（删除按钮按这个判据禁用）
        assert_eq!(
            preset_index_for_params(
                &list,
                &AdjustParams {

                    exposure: 0.3,
                    contrast: 16,
                    saturation: 0,
            ..AdjustParams::default()
        }
            ),
            None
        );
        assert_eq!(preset_index_for_params(&[], &quantized), None);
    }

    #[test]
    fn test_preset_chip_label_uses_param_summary() {
        let preset = photo_config::AdjustPreset {

            name: "夜景".into(),
            exposure: 1.0,
            contrast: 20,
            saturation: 0,
            ..photo_config::AdjustPreset::default()
        };
        assert_eq!(preset_chip_label(&preset), "夜景（+1.00 EV、对比度 +20）");
        let neutral = photo_config::AdjustPreset {
            name: "无".into(),
            ..Default::default()
        };
        assert_eq!(preset_chip_label(&neutral), "无（无调整）");
    }

    #[test]
    fn test_adjust_targets_intersects_selection_with_filter() {
        let order = [3usize, 1, 7, 2];
        // 无选区 → 空（绝不退化成"全目录"）
        assert!(adjust_targets(&[], &order).is_empty());
        // 选区 ∩ 筛选结果，顺序跟 display_order（可见顺序）
        assert_eq!(adjust_targets(&[7, 3], &order), vec![3, 7]);
        // 被筛掉的选中项不参与
        assert_eq!(adjust_targets(&[3, 9, 2], &order), vec![3, 2]);
        // 全选中 = 全部可见项
        assert_eq!(adjust_targets(&order, &order), order.to_vec());
    }

    #[test]
    fn test_previous_in_order_walks_visible_order() {
        let order = [5usize, 2, 9];
        assert_eq!(previous_in_order(Some(2), &order), Some(5));
        assert_eq!(previous_in_order(Some(9), &order), Some(2));
        // 第一张没有"上一张"
        assert_eq!(previous_in_order(Some(5), &order), None);
        // 不在可见集 / 无选中
        assert_eq!(previous_in_order(Some(42), &order), None);
        assert_eq!(previous_in_order(None, &order), None);
    }

    #[test]
    fn test_describe_adjust_omits_neutral_fields() {
        assert_eq!(describe_adjust(&AdjustParams::default()), "无调整");
        assert_eq!(
            describe_adjust(&AdjustParams {

                exposure: 1.0,
                contrast: 15,
                saturation: 0,
            ..AdjustParams::default()
        }),
            "+1.00 EV、对比度 +15"
        );
        assert_eq!(
            describe_adjust(&AdjustParams {

                exposure: 0.0,
                contrast: 0,
                saturation: -40,
            ..AdjustParams::default()
        }),
            "饱和度 -40"
        );
    }

    #[test]
    fn test_quantize_exposure_snaps_to_step() {
        assert_eq!(quantize_exposure(0.37), 0.3);
        assert_eq!(quantize_exposure(-1.234), -1.2);
        assert_eq!(quantize_exposure(0.0), 0.0);
        // 0.3 的整数倍逐点对齐（相机上常见的 1/3 档）
        assert_eq!(quantize_exposure(0.9), 0.9);
        assert_eq!(quantize_exposure(2.7), 2.7);
    }

    #[test]
    fn test_quantize_exposure_clamps_and_guards_non_finite() {
        assert_eq!(quantize_exposure(9.0), 3.0);
        assert_eq!(quantize_exposure(-9.0), -3.0);
        assert_eq!(quantize_exposure(f32::NAN), 0.0);
        assert_eq!(quantize_exposure(f32::INFINITY), 0.0);
    }

    #[test]
    fn test_quantize_tone_rounds_and_clamps() {
        assert_eq!(quantize_tone(15.4), 15);
        assert_eq!(quantize_tone(-15.6), -16);
        assert_eq!(quantize_tone(300.0), 100);
        assert_eq!(quantize_tone(-300.0), -100);
        assert_eq!(quantize_tone(f32::NAN), 0);
    }

    #[test]
    fn test_format_exposure_keeps_sign_and_unit() {
        assert_eq!(format_exposure(0.0), "0.00 EV");
        assert_eq!(format_exposure(0.35), "+0.35 EV");
        assert_eq!(format_exposure(-1.2), "-1.20 EV");
    }

    #[test]
    fn test_format_tone_omits_plus_on_zero() {
        assert_eq!(format_tone(0), "0");
        assert_eq!(format_tone(15), "+15");
        assert_eq!(format_tone(-40), "-40");
    }

    #[test]
    fn test_rel_path_of_normalizes_outside_and_inside() {
        let dir = Path::new("/photos/trip");
        assert_eq!(
            rel_path_of(dir, Path::new("/photos/trip/sub/a.NEF")).as_deref(),
            Some("sub/a.NEF")
        );
        assert_eq!(
            rel_path_of(dir, Path::new("/photos/trip/b.jpg")).as_deref(),
            Some("b.jpg")
        );
        // 目录外（例如另一个文件夹的路径）不产生键，避免写错 DB 行
        assert_eq!(rel_path_of(dir, Path::new("/elsewhere/c.jpg")), None);
    }

    #[test]
    fn test_adjust_field_apply_quantizes_each_field() {
        let mut p = AdjustParams::default();
        AdjustField::Exposure.apply(&mut p, 0.37); // → 0.3（0.3 EV 步进）
        AdjustField::Contrast.apply(&mut p, 15.6);
        AdjustField::Saturation.apply(&mut p, -300.0);
        assert_eq!(
            p,
            AdjustParams {

                exposure: 0.3,
                contrast: 16,
                saturation: -100,
            ..AdjustParams::default()
        }
        );
    }

    #[test]
    fn test_adjust_field_slider_value_round_trips() {
        let p = AdjustParams {

            exposure: -1.2,
            contrast: 42,
            saturation: -7,
            ..AdjustParams::default()
        };
        for field in AdjustField::ALL {
            let mut round = AdjustParams::default();
            field.apply(&mut round, field.slider_value(&p));
            assert_eq!(field.slider_value(&round), field.slider_value(&p));
        }
    }

    #[test]
    fn test_adjust_field_reset_and_neutral_check() {
        let mut p = AdjustParams {
            exposure: 1.0,
            contrast: 20,
            saturation: -20,
            shadows: 20,
            highlights: -20,
            temperature: 15,
            tint: -15,
        };
        for field in AdjustField::ALL {
            assert!(!field.is_neutral(&p), "{} 初始应为非中性", field.label());
            field.reset(&mut p);
            assert!(field.is_neutral(&p));
        }
        assert!(p.is_neutral());
    }

    #[test]
    fn test_has_adjustments_matches_neutral() {
        assert!(!has_adjustments(&AdjustParams::default()));
        assert!(has_adjustments(&AdjustParams {
            exposure: 0.3,
            ..Default::default()
        }));
        assert!(has_adjustments(&AdjustParams {
            contrast: -1,
            ..Default::default()
        }));
    }
}
