//! Material You「墨白」配色方案：从一个 seed 色生成亮/暗两套语义色。
//!
//! 立场（§6.1）：
//! - **表面全部黑白灰**——中性板固定 chroma = 0，取 HCT 的 tone（CIE L*）纯灰阶；
//! - **彩色只给 accent 一族**——主色、焦点环、选中描边、进度条、主按钮；
//! - seed 色相决定 accent 色相；seed 本身近乎无彩（chroma < 8）时退化为**单色**
//!   （M3 SchemeMonochrome 语义），此时 accent 用中性板本身，得到真正的黑/白强调。
//!
//! 本模块只产出 `#rrggbb` 字符串与 tone 值，不认识 gpui；令牌映射在 `theme::mod`。

use super::material::{self, Hct, TonalPalette};

/// 主题色预设：名字给设置页显示，hex 是 HCT 生成的 seed。
pub struct AccentPreset {
    pub name: &'static str,
    pub hex: &'static str,
}

/// 设置页的颜色预设表。顺序按经典 Material 3 动态取色方案排列。
pub const ACCENT_PRESETS: &[AccentPreset] = &[
    AccentPreset {
        name: "天青蓝",
        hex: "#00639b",
    },
    AccentPreset {
        name: "薄荷青",
        hex: "#006874",
    },
    AccentPreset {
        name: "碧玉绿",
        hex: "#386a20",
    },
    AccentPreset {
        name: "琥珀金",
        hex: "#8c5000",
    },
    AccentPreset {
        name: "木槿紫",
        hex: "#6750a4",
    },
    AccentPreset {
        name: "珊瑚红",
        hex: "#ba1a1a",
    },
    AccentPreset {
        name: "朱橙",
        hex: "#f97316",
    },
    AccentPreset {
        name: "靛蓝",
        hex: "#3b82f6",
    },
    AccentPreset {
        name: "紫罗兰",
        hex: "#8b5cf6",
    },
    AccentPreset {
        name: "玫红",
        hex: "#ec4899",
    },
    AccentPreset {
        name: "极简墨",
        hex: "#111111",
    },
];

/// 默认 seed：经典 Material You 天青蓝。
pub const DEFAULT_ACCENT: &str = "#00639b";

/// seed chroma 低于此值视为「无彩」，走单色强调。
const MONO_CHROMA_THRESHOLD: f64 = 8.0;
/// 彩色 accent 的 chroma 区间：太淡看不见，太浓刺眼。
const ACCENT_CHROMA_MIN: f64 = 28.0;
const ACCENT_CHROMA_MAX: f64 = 52.0;

/// accent 一族在某个明暗模式下的 tone 取值。
#[derive(Debug, Clone, Copy)]
struct AccentTones {
    primary: f64,
    on_primary: f64,
    hover: f64,
    active: f64,
    container: f64,
    on_container: f64,
}

/// 单色（墨白）强调：把主色压到高对比的黑/白。
const MONO_LIGHT: AccentTones = AccentTones {
    primary: 12.0,
    on_primary: 100.0,
    hover: 26.0,
    active: 6.0,
    container: 90.0,
    on_container: 12.0,
};
const MONO_DARK: AccentTones = AccentTones {
    primary: 95.0,
    on_primary: 14.0,
    hover: 100.0,
    active: 86.0,
    container: 26.0,
    on_container: 92.0,
};
/// 彩色强调：标准的 Material 3 主色 tone（亮 40 / 暗 80）。
const COLOR_LIGHT: AccentTones = AccentTones {
    primary: 40.0,
    on_primary: 100.0,
    hover: 34.0,
    active: 24.0,
    container: 90.0,
    on_container: 10.0,
};
const COLOR_DARK: AccentTones = AccentTones {
    primary: 80.0,
    on_primary: 20.0,
    hover: 86.0,
    active: 72.0,
    container: 30.0,
    on_container: 90.0,
};

/// 一套解析好的 Material You 配色：中性板 + 中性变体板 + accent 板 + secondary 板 + 明暗模式。
#[derive(Debug, Clone)]
pub struct Scheme {
    neutral: TonalPalette,
    neutral_variant: TonalPalette,
    accent: TonalPalette,
    secondary: TonalPalette,
    tones: AccentTones,
    dark: bool,
    mono: bool,
    seed: u32,
}

impl Scheme {
    /// 从 seed（ARGB）与明暗模式生成配色。
    pub fn from_seed(seed_argb: u32, dark: bool) -> Self {
        let hct = Hct::from_argb(seed_argb);
        let mono = hct.chroma() < MONO_CHROMA_THRESHOLD;
        let (neutral_chroma, nv_chroma, secondary_chroma, accent_chroma) = if mono {
            (0.0, 0.0, 0.0, 0.0)
        } else {
            (
                4.0,
                8.0,
                16.0,
                hct.chroma().clamp(ACCENT_CHROMA_MIN, ACCENT_CHROMA_MAX),
            )
        };
        let tones = match (mono, dark) {
            (true, false) => MONO_LIGHT,
            (true, true) => MONO_DARK,
            (false, false) => COLOR_LIGHT,
            (false, true) => COLOR_DARK,
        };
        Self {
            neutral: TonalPalette::from_hue_and_chroma(hct.hue(), neutral_chroma),
            neutral_variant: TonalPalette::from_hue_and_chroma(hct.hue(), nv_chroma),
            accent: TonalPalette::from_hue_and_chroma(hct.hue(), accent_chroma),
            secondary: TonalPalette::from_hue_and_chroma(hct.hue(), secondary_chroma),
            tones,
            dark,
            mono,
            seed: seed_argb,
        }
    }

    /// 是否走单色强调。
    pub fn is_mono(&self) -> bool {
        self.mono
    }

    pub fn is_dark(&self) -> bool {
        self.dark
    }

    pub fn seed_argb(&self) -> u32 {
        self.seed
    }

    /// 中性色调（表面/背景）→ `#rrggbb`
    pub fn n(&self, tone: f64) -> String {
        material::hex_from_argb(self.neutral.tone(tone))
    }

    /// 中性变体色调（边框/描边/低对比文本）→ `#rrggbb`
    pub fn nv(&self, tone: f64) -> String {
        material::hex_from_argb(self.neutral_variant.tone(tone))
    }

    /// accent 主强调色调 → `#rrggbb`（单色时等于中性灰阶）
    pub fn p(&self, tone: f64) -> String {
        material::hex_from_argb(self.accent.tone(tone))
    }

    /// secondary 次强调色调 → `#rrggbb`
    pub fn s(&self, tone: f64) -> String {
        material::hex_from_argb(self.secondary.tone(tone))
    }

    pub fn primary(&self) -> String {
        self.p(self.tones.primary)
    }
    pub fn on_primary(&self) -> String {
        self.p(self.tones.on_primary)
    }
    pub fn primary_hover(&self) -> String {
        self.p(self.tones.hover)
    }
    pub fn primary_active(&self) -> String {
        self.p(self.tones.active)
    }
    /// accent 容器（选中行 / 药丸胶囊 / 高亮背景）
    pub fn primary_container(&self) -> String {
        self.p(self.tones.container)
    }
    pub fn on_primary_container(&self) -> String {
        self.p(self.tones.on_container)
    }

    pub fn secondary(&self) -> String {
        self.s(if self.dark { 80.0 } else { 40.0 })
    }
    pub fn on_secondary(&self) -> String {
        self.s(if self.dark { 20.0 } else { 100.0 })
    }
    pub fn secondary_container(&self) -> String {
        self.s(if self.dark { 30.0 } else { 90.0 })
    }
    pub fn on_secondary_container(&self) -> String {
        self.s(if self.dark { 90.0 } else { 10.0 })
    }

    /// 状态色（不随 seed 变，保持语义可辨）。返回 (背景, 其上的字)。
    pub fn danger(&self) -> (&'static str, &'static str) {
        if self.dark {
            ("#f2b8b5", "#601410")
        } else {
            ("#b3261e", "#ffffff")
        }
    }
    pub fn success(&self) -> (&'static str, &'static str) {
        if self.dark {
            ("#6dd58c", "#00391d")
        } else {
            ("#2e7d4f", "#ffffff")
        }
    }
    pub fn warning(&self) -> (&'static str, &'static str) {
        if self.dark {
            ("#f5b944", "#2b1700")
        } else {
            ("#8a5300", "#ffffff")
        }
    }
    pub fn info(&self) -> (&'static str, &'static str) {
        if self.dark {
            ("#a8c7fa", "#04264d")
        } else {
            ("#1f5fa8", "#ffffff")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed(hex: &str) -> u32 {
        material::argb_from_hex(hex).expect("测试 seed 必须是合法 hex")
    }

    #[test]
    fn test_monochrome_seed_is_monochrome() {
        let s = Scheme::from_seed(seed("#111111"), false);
        assert!(s.is_mono(), "近黑 seed 必须走单色强调");
        // 单色 accent 的各个 tone 都是纯灰：R=G=B
        for c in [s.primary(), s.primary_container(), s.on_primary()] {
            let r = u8::from_str_radix(&c[1..3], 16).unwrap();
            let g = u8::from_str_radix(&c[3..5], 16).unwrap();
            let b = u8::from_str_radix(&c[5..7], 16).unwrap();
            assert!(r == g && g == b, "单色 accent 应无彩: {c}");
        }
    }

    #[test]
    fn test_colorful_seed_keeps_hue_as_accent() {
        // 靛蓝 seed：accent 主色应仍是蓝（B 通道最大），且不是灰
        let s = Scheme::from_seed(seed("#3b82f6"), false);
        assert!(!s.is_mono());
        let p = s.primary();
        let r = u8::from_str_radix(&p[1..3], 16).unwrap();
        let b = u8::from_str_radix(&p[5..7], 16).unwrap();
        assert!(b > r, "蓝色 seed 的主色应偏蓝: {p}");
    }

    #[test]
    fn test_neutral_ramp_is_monotonic() {
        // 单色种子下 tone 越大越亮：R 通道单调不减（纯灰保证三个通道一致）
        let s = Scheme::from_seed(seed("#111111"), false);
        let mut prev = 0u32;
        for tone in [0.0, 10.0, 20.0, 40.0, 60.0, 80.0, 92.0, 100.0] {
            let c = s.n(tone);
            let r = u8::from_str_radix(&c[1..3], 16).unwrap() as u32;
            assert!(r >= prev, "tone {tone} 亮度回退: {c}");
            assert!(r == u8::from_str_radix(&c[3..5], 16).unwrap() as u32);
            assert!(r == u8::from_str_radix(&c[5..7], 16).unwrap() as u32);
            prev = r;
        }
        assert_eq!(s.n(100.0), "#ffffff");
        assert_eq!(s.n(0.0), "#000000");
    }

    #[test]
    fn test_light_and_dark_use_different_neutral_ends() {
        let light = Scheme::from_seed(seed("#111111"), false);
        let dark = Scheme::from_seed(seed("#111111"), true);
        // 亮色主色接近黑，暗色主色接近白
        let lum = |hex: &str| u8::from_str_radix(&hex[1..3], 16).unwrap();
        assert!(
            lum(&light.primary()) < 60,
            "亮色墨色应偏深: {}",
            light.primary()
        );
        assert!(
            lum(&dark.primary()) > 200,
            "暗色墨色应偏浅: {}",
            dark.primary()
        );
        assert!(light.is_mono() && dark.is_mono());
    }

    #[test]
    fn test_presets_are_valid_hex_and_unique() {
        let mut seen = std::collections::HashSet::new();
        for p in ACCENT_PRESETS {
            assert!(
                material::argb_from_hex(p.hex).is_some(),
                "预设色非法: {} {}",
                p.name,
                p.hex
            );
            assert!(seen.insert(p.hex), "预设色重复: {}", p.hex);
        }
        assert_eq!(
            ACCENT_PRESETS[0].hex, DEFAULT_ACCENT,
            "第一个预设必须是默认墨白"
        );
    }

    #[test]
    fn test_status_colors_differ_by_mode() {
        let light = Scheme::from_seed(seed(DEFAULT_ACCENT), false);
        let dark = Scheme::from_seed(seed(DEFAULT_ACCENT), true);
        assert_ne!(light.danger().0, dark.danger().0);
        assert_ne!(light.success().0, dark.success().0);
    }
}
