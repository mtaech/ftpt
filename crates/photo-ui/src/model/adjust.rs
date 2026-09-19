//! 调整参数（ADR 0007）的纯逻辑：量化、格式化、相对路径。
//!
//! 不含 IO 与 GPUI 依赖——滑杆交互在 views 层，持久化在 state 层，
//! 这里只放可单测的换算规则。

use std::path::Path;

use photo_domain::AdjustParams;

/// 曝光范围与步进（ADR 0007：±2.0 EV，步进 0.05）
pub const EXPOSURE_MIN: f32 = -2.0;
pub const EXPOSURE_MAX: f32 = 2.0;
pub const EXPOSURE_STEP: f32 = 0.05;

/// 对比度 / 饱和度范围与步进（±100，整数）
pub const TONE_MIN: f32 = -100.0;
pub const TONE_MAX: f32 = 100.0;
pub const TONE_STEP: f32 = 1.0;

/// 滑杆浮点值 → 参数曝光值：按 0.05 量化、钳制到 ±2.0，并抹掉浮点噪声（保留两位小数）。
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

/// 调整 tab 的三类参数的定位标识（滑杆、单项重置按钮共用）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdjustField {
    Exposure,
    Contrast,
    Saturation,
}

impl AdjustField {
    /// 全部字段（装载/同步滑杆时遍历用）
    pub const ALL: [AdjustField; 3] = [
        AdjustField::Exposure,
        AdjustField::Contrast,
        AdjustField::Saturation,
    ];

    /// 用滑杆原始浮点值更新一个字段（量化 + 钳制）
    pub fn apply(self, params: &mut AdjustParams, raw: f32) {
        match self {
            Self::Exposure => params.exposure = quantize_exposure(raw),
            Self::Contrast => params.contrast = quantize_tone(raw),
            Self::Saturation => params.saturation = quantize_tone(raw),
        }
    }

    /// 该字段对应的滑杆值（装载已存参数时同步滑杆用）
    pub fn slider_value(self, params: &AdjustParams) -> f32 {
        match self {
            Self::Exposure => params.exposure,
            Self::Contrast => params.contrast as f32,
            Self::Saturation => params.saturation as f32,
        }
    }

    /// 重置该字段为中性
    pub fn reset(self, params: &mut AdjustParams) {
        match self {
            Self::Exposure => params.exposure = 0.0,
            Self::Contrast => params.contrast = 0,
            Self::Saturation => params.saturation = 0,
        }
    }

    /// 该字段是否已是中性值
    pub fn is_neutral(self, params: &AdjustParams) -> bool {
        match self {
            Self::Exposure => params.exposure == 0.0,
            Self::Contrast => params.contrast == 0,
            Self::Saturation => params.saturation == 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quantize_exposure_snaps_to_step() {
        assert_eq!(quantize_exposure(0.37), 0.35);
        assert_eq!(quantize_exposure(-1.234), -1.25);
        assert_eq!(quantize_exposure(0.0), 0.0);
    }

    #[test]
    fn test_quantize_exposure_clamps_and_guards_non_finite() {
        assert_eq!(quantize_exposure(9.0), 2.0);
        assert_eq!(quantize_exposure(-9.0), -2.0);
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
        AdjustField::Exposure.apply(&mut p, 0.37);
        AdjustField::Contrast.apply(&mut p, 15.6);
        AdjustField::Saturation.apply(&mut p, -300.0);
        assert_eq!(
            p,
            AdjustParams {
                exposure: 0.35,
                contrast: 16,
                saturation: -100,
            }
        );
    }

    #[test]
    fn test_adjust_field_slider_value_round_trips() {
        let p = AdjustParams {
            exposure: -1.25,
            contrast: 42,
            saturation: -7,
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
        };
        for field in AdjustField::ALL {
            assert!(!field.is_neutral(&p));
            field.reset(&mut p);
            assert!(field.is_neutral(&p));
        }
        assert!(p.is_neutral());
    }

    #[test]
    fn test_has_adjustments_matches_neutral() {
        assert!(!has_adjustments(&AdjustParams::default()));
        assert!(has_adjustments(&AdjustParams {
            exposure: 0.05,
            ..Default::default()
        }));
        assert!(has_adjustments(&AdjustParams {
            contrast: -1,
            ..Default::default()
        }));
    }
}
