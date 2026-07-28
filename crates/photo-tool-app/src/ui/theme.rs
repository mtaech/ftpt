use gpui::{
    BoxShadow, Hsla, Pixels, Point, Rgba, Styled, hsla, px,
};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Mutex;

// ═══════════════════════════════════════════════════════════════
//  Theme Mode
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeMode {
    Light,
    Dark,
}

// ═══════════════════════════════════════════════════════════════
//  Semantic Theme Colors
// ═══════════════════════════════════════════════════════════════

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct ThemeColors {
    pub border: Hsla,
    pub border_variant: Hsla,
    pub border_focused: Hsla,
    pub border_selected: Hsla,
    pub border_transparent: Hsla,
    pub border_disabled: Hsla,
    pub elevated_surface_background: Hsla,
    pub surface_background: Hsla,
    pub background: Hsla,
    pub element_background: Hsla,
    pub element_hover: Hsla,
    pub element_active: Hsla,
    pub element_selected: Hsla,
    pub element_disabled: Hsla,
    pub text: Hsla,
    pub text_muted: Hsla,
    pub text_placeholder: Hsla,
    pub text_disabled: Hsla,
    pub text_accent: Hsla,
    pub icon: Hsla,
    pub icon_muted: Hsla,
    pub icon_disabled: Hsla,
    pub icon_placeholder: Hsla,
    pub icon_accent: Hsla,
    pub error: Hsla,
    pub error_background: Hsla,
    pub error_border: Hsla,
    pub warning: Hsla,
    pub warning_background: Hsla,
    pub warning_border: Hsla,
    pub success: Hsla,
    pub success_background: Hsla,
    pub success_border: Hsla,
    pub info: Hsla,
    pub info_background: Hsla,
    pub info_border: Hsla,
}

// ═══════════════════════════════════════════════════════════════
//  Elevation
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ElevationIndex {
    Background,
    Surface,
    ElevatedSurface,
    ModalSurface,
}

impl ElevationIndex {
    pub fn shadow(self) -> Vec<BoxShadow> {
        match self {
            ElevationIndex::Background => vec![],
            ElevationIndex::Surface => vec![BoxShadow {
                color: hsla(0., 0., 0., 0.06),
                offset: Point { x: px(0.), y: px(1.) },
                blur_radius: px(2.),
                spread_radius: px(0.),
                inset: false,
            }],
            ElevationIndex::ElevatedSurface => vec![BoxShadow {
                color: hsla(0., 0., 0., 0.08),
                offset: Point { x: px(0.), y: px(4.) },
                blur_radius: px(12.),
                spread_radius: px(-1.),
                inset: false,
            }],
            ElevationIndex::ModalSurface => vec![BoxShadow {
                color: hsla(0., 0., 0., 0.10),
                offset: Point { x: px(0.), y: px(8.) },
                blur_radius: px(32.),
                spread_radius: px(-4.),
                inset: false,
            }],
        }
    }
    #[allow(dead_code)]
    pub fn bg(self, colors: &ThemeColors) -> Hsla {
        match self {
            ElevationIndex::Background => colors.background,
            ElevationIndex::Surface => colors.surface_background,
            ElevationIndex::ElevatedSurface => colors.elevated_surface_background,
            ElevationIndex::ModalSurface => colors.elevated_surface_background,
        }
    }
}

// ═══════════════════════════════════════════════════════════════
//  Spacing
// ═══════════════════════════════════════════════════════════════

#[allow(dead_code)]
pub const SPACING: [f32; 8] = [4.0, 8.0, 12.0, 16.0, 24.0, 32.0, 40.0, 48.0];

#[allow(dead_code)]
pub fn spacing(level: usize) -> Pixels {
    px(SPACING[level.min(7)])
}

#[allow(dead_code)]
pub mod sp {
    pub const XS: usize = 0;
    pub const SM: usize = 1;
    pub const MD: usize = 2;
    pub const LG: usize = 3;
    pub const XL: usize = 4;
    pub const XXL: usize = 5;
    pub const XXXL: usize = 6;
    pub const HUGE: usize = 7;
}

// ═══════════════════════════════════════════════════════════════
//  Border Radius
// ═══════════════════════════════════════════════════════════════

#[allow(dead_code)]
pub const BORDER_RADIUS: f32 = 6.0;
#[allow(dead_code)]
pub const BORDER_RADIUS_LG: f32 = 10.0;
#[allow(dead_code)]
pub const BORDER_RADIUS_SM: f32 = 4.0;
#[allow(dead_code)]
pub const FOCUS_RING_WIDTH: f32 = 2.0;

// ═══════════════════════════════════════════════════════════════
//  Font
// ═══════════════════════════════════════════════════════════════

#[allow(dead_code)]
pub const DEFAULT_FONT_FAMILY: &str = "Microsoft YaHei UI";

/// 等宽字体：EXIF 数值、文件大小、状态栏计数等数字场景统一使用。
/// Cascadia Mono 随 Windows 11 预装；缺失时 GPUI 自动回落系统默认。
pub const MONO_FONT_FAMILY: &str = "Cascadia Mono";

// ═══════════════════════════════════════════════════════════════
//  Color helpers
// ═══════════════════════════════════════════════════════════════

fn hsla_from_rgba(r: f32, g: f32, b: f32, a: f32) -> Hsla {
    Hsla::from(Rgba { r, g, b, a })
}

fn hex_rgb(hex: u32) -> Hsla {
    hsla_from_rgba(
        ((hex >> 16) & 0xff) as f32 / 255.0,
        ((hex >> 8) & 0xff) as f32 / 255.0,
        (hex & 0xff) as f32 / 255.0,
        1.0,
    )
}

fn hex_rgba(hex: u32) -> Hsla {
    hsla_from_rgba(
        ((hex >> 24) & 0xff) as f32 / 255.0,
        ((hex >> 16) & 0xff) as f32 / 255.0,
        ((hex >> 8) & 0xff) as f32 / 255.0,
        (hex & 0xff) as f32 / 255.0,
    )
}

// ═══════════════════════════════════════════════════════════════
//  Domain-specific colors
// ═══════════════════════════════════════════════════════════════

use std::sync::LazyLock;

pub mod colors {
    use super::*;

    pub static RATING: LazyLock<[Hsla; 5]> = LazyLock::new(|| [
        hex_rgb(0xef4444),
        hex_rgb(0xf97316),
        hex_rgb(0xe8ab07),
        hex_rgb(0x22c55e),
        hex_rgb(0x3b82f6),
    ]);

    pub static LABEL_RED: LazyLock<Hsla> = LazyLock::new(|| hex_rgb(0xef4444));
    pub static LABEL_YELLOW: LazyLock<Hsla> = LazyLock::new(|| hex_rgb(0xe8ab07));
    pub static LABEL_GREEN: LazyLock<Hsla> = LazyLock::new(|| hex_rgb(0x22c55e));
    pub static LABEL_BLUE: LazyLock<Hsla> = LazyLock::new(|| hex_rgb(0x3b82f6));
    pub static LABEL_PURPLE: LazyLock<Hsla> = LazyLock::new(|| hex_rgb(0x8b5cf6));
    pub static PICK: LazyLock<Hsla> = LazyLock::new(|| hex_rgb(0x22c55e));
    pub static REJECT: LazyLock<Hsla> = LazyLock::new(|| hex_rgb(0xef4444));

    /// 徽标底：缩略图角标（RAW/JPG、旗标）的半透明黑底，两种模式通用
    pub static BADGE_BG: LazyLock<Hsla> = LazyLock::new(|| hex_rgba(0x000000b3));
}

/// accent 色的低透明度版本：选中行底色等大面积弱化场景。
/// 由 text_accent 派生，跟随当前模式。
pub fn accent_dim() -> Hsla {
    let mut c = colors().text_accent;
    c.a = 0.10;
    c
}

/// 卡片容器规范（设计系统约定）：element_background 底色 + border_variant 细边框
/// + rounded_md 圆角。用于需要与背景拉开层次的内容卡片（侧栏文件夹卡片、目录行等），
/// 选中/高亮态在此基础上覆盖 bg（如 accent_dim）。
pub fn card<E: Styled>(el: E) -> E {
    el.bg(colors().element_background)
        .border_1()
        .border_color(colors().border_variant)
        .rounded_md()
}

/// accent 色的悬浮版本：比 solid 稍亮/稍暗，取决于模式。
pub fn accent_hover() -> Hsla {
    let mut c = colors().text_accent;
    c.a = 0.80;
    c
}

// ═══════════════════════════════════════════════════════════════
//  Theme Colors constructors
// ═══════════════════════════════════════════════════════════════

impl ThemeColors {
    pub fn light() -> Self {
        Self {
            border: hex_rgb(0xd1d5db),
            border_variant: hex_rgb(0xe5e7eb),
            border_focused: hex_rgb(0x3b82f6),
            border_selected: hex_rgb(0x93c5fd),
            border_transparent: hsla(0., 0., 0., 0.),
            border_disabled: hex_rgb(0xe5e7eb),
            elevated_surface_background: hex_rgb(0xffffff),
            surface_background: hex_rgb(0xffffff),
            background: hex_rgb(0xf3f4f6),
            element_background: hex_rgb(0xf9fafb),
            element_hover: hex_rgb(0xf3f4f6),
            element_active: hex_rgb(0xe5e7eb),
            element_selected: hex_rgb(0xeff6ff),
            element_disabled: hex_rgb(0xf9fafb),
            text: hex_rgb(0x111827),
            text_muted: hex_rgb(0x6b7280),
            text_placeholder: hex_rgb(0x9ca3af),
            text_disabled: hex_rgb(0xd1d5db),
            text_accent: hex_rgb(0x3b82f6),
            icon: hex_rgb(0x111827),
            icon_muted: hex_rgb(0x6b7280),
            icon_disabled: hex_rgb(0xd1d5db),
            icon_placeholder: hex_rgb(0x9ca3af),
            icon_accent: hex_rgb(0x3b82f6),
            error: hex_rgb(0xef4444),
            error_background: hex_rgba(0xfef2f2ff),
            error_border: hex_rgb(0xfecaca),
            warning: hex_rgb(0xf59e0b),
            warning_background: hex_rgba(0xfffbebff),
            warning_border: hex_rgb(0xfde68a),
            success: hex_rgb(0x10b981),
            success_background: hex_rgba(0xecfdf5ff),
            success_border: hex_rgb(0xa7f3d0),
            info: hex_rgb(0x3b82f6),
            info_background: hex_rgba(0xeff6ffff),
            info_border: hex_rgb(0xbfdbfe),
        }
    }

    pub fn dark() -> Self {
        Self {
            // 交易终端式近黑层级：面板靠微差亮度区分，边框近乎不可见
            border: hex_rgb(0x23272f),
            border_variant: hex_rgb(0x1b1f27),
            border_focused: hex_rgb(0x22d3ee),
            border_selected: hex_rgb(0x155e75),
            border_transparent: hsla(0., 0., 0., 0.),
            border_disabled: hex_rgb(0x1b1f27),
            elevated_surface_background: hex_rgb(0x1a1e26),
            surface_background: hex_rgb(0x12151b),
            background: hex_rgb(0x0b0d11),
            element_background: hex_rgb(0x171b22),
            element_hover: hex_rgb(0x1c212b),
            element_active: hex_rgb(0x262c38),
            element_selected: hex_rgb(0x14202c),
            element_disabled: hex_rgb(0x171b22),
            text: hex_rgb(0xf9fafb),
            text_muted: hex_rgb(0x9ca3af),
            text_placeholder: hex_rgb(0x6b7280),
            text_disabled: hex_rgb(0x4b5563),
            text_accent: hex_rgb(0x22d3ee),
            icon: hex_rgb(0xf9fafb),
            icon_muted: hex_rgb(0x9ca3af),
            icon_disabled: hex_rgb(0x4b5563),
            icon_placeholder: hex_rgb(0x6b7280),
            icon_accent: hex_rgb(0x22d3ee),
            error: hex_rgb(0xf87171),
            error_background: hex_rgba(0x7f1d1d80),
            error_border: hex_rgb(0x7f1d1d),
            warning: hex_rgb(0xfbbf24),
            warning_background: hex_rgba(0x78350f80),
            warning_border: hex_rgb(0x78350f),
            success: hex_rgb(0x34d399),
            success_background: hex_rgba(0x064e3b80),
            success_border: hex_rgb(0x064e3b),
            info: hex_rgb(0x60a5fa),
            info_background: hex_rgba(0x1e3a5f80),
            info_border: hex_rgb(0x1e3a5f),
        }
    }
}

// ═══════════════════════════════════════════════════════════════
//  Global theme state
// ═══════════════════════════════════════════════════════════════

static CURRENT_MODE: AtomicU8 = AtomicU8::new(0);
static CURRENT_COLORS: Mutex<Option<ThemeColors>> = Mutex::new(None);

pub fn current_mode() -> ThemeMode {
    match CURRENT_MODE.load(Ordering::Relaxed) {
        0 => ThemeMode::Light,
        _ => ThemeMode::Dark,
    }
}

pub fn set_mode(mode: ThemeMode) {
    CURRENT_MODE.store(match mode {
        ThemeMode::Light => 0,
        ThemeMode::Dark => 1,
    }, Ordering::Relaxed);
    *CURRENT_COLORS.lock().unwrap() = None;
}

pub fn colors() -> ThemeColors {
    let mut guard = CURRENT_COLORS.lock().unwrap();
    guard.get_or_insert_with(|| match current_mode() {
        ThemeMode::Light => ThemeColors::light(),
        ThemeMode::Dark => ThemeColors::dark(),
    }).clone()
}

pub fn toggle_mode() {
    set_mode(match current_mode() {
        ThemeMode::Light => ThemeMode::Dark,
        ThemeMode::Dark => ThemeMode::Light,
    });
}

// ═══════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    fn luma(hsla: &Hsla) -> f32 {
        let rgba: Rgba = (*hsla).into();
        0.299 * rgba.r + 0.587 * rgba.g + 0.114 * rgba.b
    }

    #[test]
    fn test_light_colors_non_zero() {
        let c = ThemeColors::light();
        assert_ne!(c.text, hsla(0., 0., 0., 0.));
        assert_ne!(c.text_muted, hsla(0., 0., 0., 0.));
        assert_ne!(c.border, hsla(0., 0., 0., 0.));
        assert_ne!(c.error, hsla(0., 0., 0., 0.));
    }

    #[test]
    fn test_dark_colors_non_zero() {
        let c = ThemeColors::dark();
        assert_ne!(c.text, hsla(0., 0., 0., 0.));
        assert_ne!(c.text_muted, hsla(0., 0., 0., 0.));
        assert_ne!(c.border, hsla(0., 0., 0., 0.));
    }

    #[test]
    fn test_light_vs_dark_contrast() {
        let light = ThemeColors::light();
        let dark = ThemeColors::dark();
        assert!(luma(&light.text) < 0.5, "light text should be dark");
        assert!(luma(&dark.text) > 0.5, "dark text should be light");
    }

    #[test]
    fn test_elevation_shadow_sizes() {
        assert!(ElevationIndex::Background.shadow().is_empty());
        assert!(!ElevationIndex::Surface.shadow().is_empty());
        assert!(!ElevationIndex::ModalSurface.shadow().is_empty());
        let s = ElevationIndex::Surface.shadow()[0].blur_radius;
        let m = ElevationIndex::ModalSurface.shadow()[0].blur_radius;
        assert!(m > s);
    }

    #[test]
    fn test_spacing_values() {
        assert_eq!(SPACING[0], 4.0);
        assert_eq!(SPACING[4], 24.0);
        assert_eq!(spacing(0), px(4.0));
        assert_eq!(spacing(10), px(48.0));
    }

    #[test]
    fn test_text_luminance_hierarchy() {
        let c = ThemeColors::light();
        assert!(luma(&c.text) < luma(&c.text_muted));
        assert!(luma(&c.text_muted) < luma(&c.text_placeholder));
    }

    #[test]
    fn test_global_theme_state() {
        assert_eq!(current_mode(), ThemeMode::Light);
        set_mode(ThemeMode::Dark);
        assert_eq!(current_mode(), ThemeMode::Dark);
        let dark = colors();
        assert!(luma(&dark.text) > 0.5);
        set_mode(ThemeMode::Light);
        assert_eq!(current_mode(), ThemeMode::Light);
    }

    #[test]
    fn test_status_colors_three_tier() {
        let c = ThemeColors::light();
        assert_ne!(c.error, c.error_background);
        assert_ne!(c.error_background, c.error_border);
        assert_ne!(c.success, c.success_background);
        assert_ne!(c.warning, c.warning_background);
        assert_ne!(c.info, c.info_background);
    }

    #[test]
    fn test_background_layers_distinct() {
        let c = ThemeColors::light();
        // background (grid area) must differ from surface (panels/cards)
        assert_ne!(c.background, c.surface_background);
    }
}
