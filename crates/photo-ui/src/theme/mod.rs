//! 视觉系统与主题令牌（Material You · 墨白，对应手册 §6）。
//!
//! 三层职责：
//! 1. material 是自包含的 HCT / CAM16 色彩引擎（移植自 material-color-utilities）；
//! 2. scheme 把一个 seed 解析成亮 / 暗两套语义色（中性板 chroma = 0 + accent 一族）；
//! 3. 本模块把语义色映射成 gpui-component 的 ThemeConfig（令牌表见手册 §6.3），
//!    并提供 apply 供启动与设置页切换。
//!
//! 立场：表面全部黑白灰，彩色只给 accent 一族——主按钮、焦点环、选中描边、进度条、
//! 活动图标。领域情景色（色标 / 旗标 / 星级 / 对焦 / 检测框）是本模块的常量，
//! 不随 seed 变，也不参与明暗反转。

pub mod material;
pub mod scheme;

use std::rc::Rc;

use gpui_kit::component::theme::{Theme, ThemeConfig, ThemeConfigColors, ThemeMode};
use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::{App, BoxShadow, Div, Hsla, Rgba, SharedString, Styled, Window, hsla, px};

pub use scheme::{ACCENT_PRESETS, AccentPreset, DEFAULT_ACCENT, Scheme};

// ── 领域色常量（手册 §6.7，亮暗同表，去饱和可辨） ──
pub const COLOR_LABEL_RED: Rgba = const_rgb(0xd9534f);
pub const COLOR_LABEL_YELLOW: Rgba = const_rgb(0xc98a1c);
pub const COLOR_LABEL_GREEN: Rgba = const_rgb(0x4a9f63);
pub const COLOR_LABEL_BLUE: Rgba = const_rgb(0x4f7fd0);
pub const COLOR_LABEL_PURPLE: Rgba = const_rgb(0x8b6fd0);

pub const COLOR_PICK: Rgba = const_rgb(0x3a9a5f);
pub const COLOR_REJECT: Rgba = const_rgb(0xd64c44);
pub const COLOR_RATING: Rgba = const_rgb(0xc98a1c);
pub const COLOR_FOCUS: Rgba = const_rgb(0xc98a1c);

pub const fn const_rgb(hex: u32) -> Rgba {
    let r = ((hex >> 16) & 0xff) as f32 / 255.0;
    let g = ((hex >> 8) & 0xff) as f32 / 255.0;
    let b = (hex & 0xff) as f32 / 255.0;
    Rgba { r, g, b, a: 1.0 }
}

pub fn hex_to_hsla(color: impl Into<Hsla>) -> Hsla {
    color.into()
}

pub fn hex_to_hsla_alpha(color: impl Into<Hsla>, a: f32) -> Hsla {
    let mut hsla = color.into();
    hsla.a = a;
    hsla
}

/// hex（#rrggbb / rrggbb）→ 不透明 Rgba；非法回退默认 seed。
pub fn hex_to_rgba(hex: &str) -> Rgba {
    let argb = material::argb_from_hex(hex)
        .unwrap_or_else(|| material::argb_from_hex(DEFAULT_ACCENT).expect("默认 seed 必须合法"));
    const_rgb(argb & 0x00ff_ffff)
}

/// 该颜色上是否应使用深色前景（Rec.601 亮度阈值）。
/// 设置页色块上的对勾靠它决定画黑勾还是白勾。
pub fn prefers_dark_ink(hex: &str) -> bool {
    let c = hex_to_rgba(hex);
    0.299 * c.r + 0.587 * c.g + 0.114 * c.b > 0.6
}

// ── seed 解析 ──

/// 解析 seed：非法 / 缺失回退默认墨白（#111111），统一小写 #rrggbb。
pub fn resolve_seed(seed_hex: Option<&str>) -> String {
    seed_hex
        .and_then(photo_config::normalize_accent_hex)
        .unwrap_or_else(|| DEFAULT_ACCENT.to_string())
}

/// 解析字体家族：非法 / 缺失回退系统默认 UI 字体（.SystemUIFont）。
pub fn resolve_font(font: Option<&str>) -> SharedString {
    match font.map(str::trim) {
        Some(f) if !f.is_empty() && f != "System" && f != "默认" && f != ".SystemUIFont" => {
            SharedString::from(f.to_string())
        }
        _ => SharedString::from(".SystemUIFont"),
    }
}

// ── 令牌映射 ──

/// 少写一百多遍 Some(SharedString::from(..))；字段名由编译器校验。
macro_rules! theme_colors {
    ($($field:ident: $value:expr),* $(,)?) => {{
        // 不能用结构体更新语法：ThemeConfigColors 有若干私有字段（base red/green/...）
        let mut colors = ThemeConfigColors::default();
        $(colors.$field = Some(SharedString::from($value));)*
        colors
    }};
}

/// 由 seed + 明暗模式构建一份 gpui-component 主题配置。
pub fn theme_config(seed_hex: &str, dark: bool) -> ThemeConfig {
    theme_config_with_font(seed_hex, dark, None)
}

/// 由 seed + 明暗模式 + 可选全局字体构建一份 gpui-component 主题配置。
pub fn theme_config_with_font(
    seed_hex: &str,
    dark: bool,
    font_family: Option<&str>,
) -> ThemeConfig {
    let seed = material::argb_from_hex(seed_hex)
        .unwrap_or_else(|| material::argb_from_hex(DEFAULT_ACCENT).expect("默认 seed 必须合法"));
    let s = Scheme::from_seed(seed, dark);
    let n = |tone: f64| s.n(tone);
    let nv = |tone: f64| s.nv(tone);

    // ── Material 3 色调表面容器系统 ──
    let background = n(if dark { 6.0 } else { 98.0 });
    let chrome = n(if dark { 10.0 } else { 95.0 }); // sidebar / title_bar / status_bar
    let content = n(if dark { 13.0 } else { 100.0 }); // list / table / button / accordion
    let popover = n(if dark { 17.0 } else { 100.0 });
    let surface2 = n(if dark { 15.0 } else { 92.0 }); // muted / secondary / tiles / skeleton
    let surface_hover = n(if dark { 20.0 } else { 90.0 });
    let surface_head = n(if dark { 14.0 } else { 94.0 });
    let tab_track = n(if dark { 12.0 } else { 93.0 });
    let surface_even = n(if dark { 13.0 } else { 97.0 });
    let secondary_hover = n(if dark { 22.0 } else { 88.0 });
    let secondary_active = n(if dark { 26.0 } else { 84.0 });

    let foreground = n(if dark { 90.0 } else { 10.0 });
    let muted_foreground = nv(if dark { 68.0 } else { 45.0 });
    let border = nv(if dark { 28.0 } else { 82.0 });
    let row_border = nv(if dark { 22.0 } else { 88.0 });
    let input = nv(if dark { 30.0 } else { 65.0 });
    let window_border = nv(if dark { 32.0 } else { 75.0 });
    let scrollbar_thumb = nv(if dark { 38.0 } else { 70.0 });
    let scrollbar_thumb_hover = nv(if dark { 48.0 } else { 60.0 });
    let switch_bg = nv(if dark { 32.0 } else { 80.0 });
    let switch_thumb = n(if dark { 90.0 } else { 100.0 });
    let slider_bar = nv(if dark { 30.0 } else { 85.0 });

    // ── accent 与 secondary 一族（Material You 动态色彩） ──
    let primary = s.primary();
    let primary_foreground = s.on_primary();
    let primary_hover = s.primary_hover();
    let primary_active = s.primary_active();
    let container = s.primary_container();

    let secondary = s.secondary_container();
    let secondary_foreground = s.on_secondary_container();

    let (danger, danger_fg) = s.danger();
    let (success, success_fg) = s.success();
    let (warning, warning_fg) = s.warning();
    let (info, info_fg) = s.info();

    let transparent = "#00000000";
    let overlay = if dark { "#00000099" } else { "#00000066" };

    let (chart_1, chart_2, chart_3, chart_4, chart_5) = if dark {
        ("#e0a83f", "#4fb06a", "#6f97dc", "#9d82dd", "#ef5f52")
    } else {
        ("#c98a1c", "#4a9f63", "#4f7fd0", "#8b6fd0", "#d9534f")
    };

    let colors = theme_colors! {
        background: background.clone(),
        foreground: foreground.clone(),
        sidebar: chrome.clone(),
        sidebar_foreground: foreground.clone(),
        sidebar_border: border.clone(),
        sidebar_accent: surface_hover.clone(),
        sidebar_accent_foreground: foreground.clone(),
        sidebar_primary: primary.clone(),
        sidebar_primary_foreground: primary_foreground.clone(),
        title_bar: chrome.clone(),
        title_bar_border: border.clone(),
        status_bar: chrome.clone(),
        status_bar_border: border.clone(),
        popover: popover.clone(),
        popover_foreground: foreground.clone(),
        list: content.clone(),
        list_head: surface_head.clone(),
        list_hover: surface_hover.clone(),
        list_even: surface_even.clone(),
        table: content.clone(),
        table_head: surface_head.clone(),
        table_head_foreground: muted_foreground.clone(),
        table_foot: surface_head.clone(),
        table_foot_foreground: muted_foreground.clone(),
        table_hover: surface_hover.clone(),
        table_even: surface_even.clone(),
        table_row_border: row_border.clone(),
        accordion: content.clone(),
        button: content.clone(),
        button_foreground: foreground.clone(),
        button_hover: surface_hover.clone(),
        button_active: secondary_active.clone(),
        muted: surface2.clone(),
        muted_foreground: muted_foreground.clone(),
        secondary: secondary.clone(),
        secondary_foreground: secondary_foreground.clone(),
        secondary_hover: secondary_hover.clone(),
        secondary_active: secondary_active.clone(),
        accent: surface_hover.clone(),
        accent_foreground: foreground.clone(),
        group_box: surface2.clone(),
        group_box_foreground: foreground.clone(),
        group_box_title_foreground: muted_foreground.clone(),
        description_list_label: surface_head.clone(),
        description_list_label_foreground: foreground.clone(),
        skeleton: surface2.clone(),
        tiles: surface2.clone(),
        border: border.clone(),
        input: input.clone(),
        window_border: window_border.clone(),
        drag_border: primary.clone(),
        tab: transparent,
        tab_foreground: muted_foreground.clone(),
        tab_active: popover.clone(),
        tab_active_foreground: foreground.clone(),
        tab_bar: tab_track,
        tab_bar_segmented: popover.clone(),
        switch: switch_bg,
        switch_thumb: switch_thumb,
        slider_bar: slider_bar,
        slider_thumb: primary.clone(),
        scrollbar: transparent,
        scrollbar_thumb: scrollbar_thumb,
        scrollbar_thumb_hover: scrollbar_thumb_hover,
        caret: primary.clone(),
        link: primary.clone(),
        link_hover: primary_hover.clone(),
        link_active: primary_active.clone(),
        progress_bar: primary.clone(),
        ring: primary.clone(),
        overlay: overlay,
        primary: primary.clone(),
        primary_foreground: primary_foreground.clone(),
        primary_hover: primary_hover.clone(),
        primary_active: primary_active.clone(),
        button_primary: primary.clone(),
        button_primary_foreground: primary_foreground.clone(),
        button_primary_hover: primary_hover.clone(),
        button_primary_active: primary_active.clone(),
        selection: container.clone(),
        list_active: container.clone(),
        list_active_border: primary.clone(),
        table_active: container.clone(),
        table_active_border: primary.clone(),
        drop_target: container.clone(),
        button_secondary: secondary.clone(),
        button_secondary_foreground: secondary_foreground.clone(),
        button_secondary_hover: secondary_hover.clone(),
        button_secondary_active: secondary_active.clone(),
        danger: danger,
        danger_foreground: danger_fg,
        button_danger: danger,
        button_danger_foreground: danger_fg,
        success: success,
        success_foreground: success_fg,
        button_success: success,
        button_success_foreground: success_fg,
        warning: warning,
        warning_foreground: warning_fg,
        button_warning: warning,
        button_warning_foreground: warning_fg,
        info: info,
        info_foreground: info_fg,
        button_info: info,
        button_info_foreground: info_fg,
        chart_1: chart_1,
        chart_2: chart_2,
        chart_3: chart_3,
        chart_4: chart_4,
        chart_5: chart_5,
        chart_bullish: success,
        chart_bearish: danger,
    };

    let font_val = resolve_font(font_family);

    ThemeConfig {
        is_default: false,
        name: SharedString::from(if dark {
            "Material You 深色"
        } else {
            "Material You 浅色"
        }),
        mode: if dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        },
        font_family: Some(font_val),
        font_size: Some(14.0),
        // 现代 Material 3 圆角：标准组件基础圆角 8px，大组件/卡片 16px
        radius: Some(8),
        radius_lg: Some(16),
        // 开启柔和弥散阴影
        shadow: Some(true),
        colors,
        ..Default::default()
    }
}

/// 应用主题：重建亮 / 暗两套配置并切到指定模式，应用全局主题色与字体。
///
/// window 为 None 时（启动早期）仍会 refresh_windows，保证无窗口路径也能生效。
pub fn apply(
    seed_hex: Option<&str>,
    dark: bool,
    font_family: Option<&str>,
    window: Option<&mut Window>,
    cx: &mut App,
) {
    let seed = resolve_seed(seed_hex);
    let light = theme_config_with_font(&seed, false, font_family);
    let dark_config = theme_config_with_font(&seed, true, font_family);
    let font_val = resolve_font(font_family);
    {
        let theme = Theme::global_mut(cx);
        theme.light_theme = Rc::new(light);
        theme.dark_theme = Rc::new(dark_config);
        theme.font_family = font_val;
    }
    let mode = if dark {
        ThemeMode::Dark
    } else {
        ThemeMode::Light
    };
    Theme::change(mode, window, cx);
    cx.refresh_windows();
}

/// 当前是否为暗色（读 gpui 全局主题，避免与配置不同步时误判）。
pub fn is_dark(cx: &App) -> bool {
    Theme::global(cx).mode.is_dark()
}

// ── 共享样式 helper（视图只认这几个入口） ──

/// 面板卡片底：内容面白 / 深灰。
pub fn card_surface(cx: &App) -> Hsla {
    cx.theme().popover
}

/// 面板卡片投影：现代 Material 3 Elevation 1 层次。
pub fn panel_card_shadow(cx: &App) -> Vec<BoxShadow> {
    let dark = is_dark(cx);
    vec![
        BoxShadow::new(px(0.), px(1.), hsla(0., 0., 0., if dark { 0.25 } else { 0.05 })).blur_radius(px(3.)),
        BoxShadow::new(px(0.), px(1.), hsla(0., 0., 0., if dark { 0.15 } else { 0.03 })).blur_radius(px(2.)),
    ]
}

/// 侧栏边缘投影。
pub fn panel_edge_shadow(cx: &App, _towards_left: bool) -> Vec<BoxShadow> {
    let dark = is_dark(cx);
    vec![
        BoxShadow::new(px(0.), px(0.), hsla(0., 0., 0., if dark { 0.18 } else { 0.04 })).blur_radius(px(6.)),
    ]
}

/// 浮动胶囊条阴影（M3 Elevation 3）：预览浮动工具条专用。
pub fn floating_toolbar_shadow(cx: &App) -> Vec<BoxShadow> {
    let dark = is_dark(cx);
    vec![
        BoxShadow::new(px(0.), px(6.), hsla(0., 0., 0., if dark { 0.35 } else { 0.12 })).blur_radius(px(16.)),
        BoxShadow::new(px(0.), px(2.), hsla(0., 0., 0., if dark { 0.20 } else { 0.06 })).blur_radius(px(6.)),
    ]
}

/// 顶层浮层投影（M3 elevation 4/5）：弹窗专用。
pub fn overlay_shadow() -> Vec<BoxShadow> {
    vec![
        BoxShadow::new(px(0.), px(12.), hsla(0., 0., 0., 0.22)).blur_radius(px(28.)),
        BoxShadow::new(px(0.), px(4.), hsla(0., 0., 0., 0.12)).blur_radius(px(10.)),
    ]
}

/// 面板卡片容器：全宽纵列 + 现代圆角 (12px) + 柔和边框与微阴影。
pub fn panel_card(cx: &App) -> Div {
    let theme = Theme::global(cx);
    v_flex()
        .w_full()
        .rounded(px(12.))
        .bg(card_surface(cx))
        .border_1()
        .border_color(theme.border.opacity(0.65))
        .shadow(panel_card_shadow(cx))
}

/// 面板分区：柔和微细线 + 上间距。
pub fn section(cx: &App) -> Div {
    let theme = Theme::global(cx);
    v_flex()
        .w_full()
        .border_t_1()
        .border_color(theme.border.opacity(0.6))
        .pt_3()
}

/// 面板第一段分区：与 section 同规格但不带顶线。
pub fn section_first(_cx: &App) -> Div {
    v_flex().w_full()
}

/// 面板内悬浮底。
pub fn element_hover(cx: &App) -> Hsla {
    cx.theme().accent
}

/// 面板内激活底。
pub fn element_active(cx: &App) -> Hsla {
    cx.theme().secondary_active
}

/// 选中底（accent 容器）。
pub fn element_selected(cx: &App) -> Hsla {
    cx.theme().selection
}

/// 当前目录行 accent 容器底。
pub fn accent_dim(cx: &App) -> Hsla {
    cx.theme().selection
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb(hex: &str) -> (u8, u8, u8) {
        let r = u8::from_str_radix(&hex[1..3], 16).unwrap();
        let g = u8::from_str_radix(&hex[3..5], 16).unwrap();
        let b = u8::from_str_radix(&hex[5..7], 16).unwrap();
        (r, g, b)
    }

    /// 取一个已显式映射的令牌，用于断言颜色本身。
    fn field(config: &ThemeConfig, name: &str) -> String {
        let c = &config.colors;
        let value = match name {
            "background" => &c.background,
            "popover" => &c.popover,
            "list" => &c.list,
            "border" => &c.border,
            "primary" => &c.primary,
            "primary_foreground" => &c.primary_foreground,
            "selection" => &c.selection,
            other => panic!("未知字段: {other}"),
        };
        value
            .clone()
            .expect("映射表里的令牌都应已显式设置")
            .to_string()
    }

    #[test]
    fn test_monochrome_light_surfaces_are_neutral_greys() {
        let c = theme_config("#111111", false);
        // 单色模式下中性面没有任何色偏
        for name in ["background", "popover", "list", "border"] {
            let (r, g, b) = rgb(&field(&c, name));
            assert!(r == g && g == b, "{name} 应无彩: {}", field(&c, name));
        }
        // 现代 Material 3 圆角
        assert_eq!(c.radius, Some(8));
        assert_eq!(c.radius_lg, Some(16));
        assert_eq!(c.shadow, Some(true));
        assert_eq!(c.mode, ThemeMode::Light);
    }

    #[test]
    fn test_default_mono_primary_contrasts_on_surface() {
        let light = theme_config("#111111", false);
        // 亮色单色主色接近黑，其上的字接近白
        assert!(rgb(&field(&light, "primary")).0 < 60);
        assert!(rgb(&field(&light, "primary_foreground")).0 > 200);
        let dark = theme_config("#111111", true);
        assert!(rgb(&field(&dark, "primary")).0 > 200);
        assert!(rgb(&field(&dark, "primary_foreground")).0 < 60);
    }

    #[test]
    fn test_colorful_seed_tints_accent_and_surfaces() {
        let c = theme_config("#3b82f6", false);
        // accent 带上 seed 色相（蓝：B 通道最大）
        let (r, _, b) = rgb(&field(&c, "primary"));
        assert!(b > r, "蓝色 seed 的主色应偏蓝");
    }

    #[test]
    fn test_dark_config_uses_dark_surfaces() {
        let c = theme_config("#3b82f6", true);
        assert_eq!(c.mode, ThemeMode::Dark);
        assert!(rgb(&field(&c, "background")).0 < 40, "暗色画布应近黑");
        assert!(rgb(&field(&c, "list")).0 < 60, "暗色内容面应偏深");
    }

    #[test]
    fn test_hex_to_rgba_and_ink_choice() {
        let white = hex_to_rgba("#ffffff");
        assert!((white.r - 1.0).abs() < 1e-6 && (white.g - 1.0).abs() < 1e-6);
        assert!(prefers_dark_ink("#ffffff"), "白底应画黑勾");
        assert!(!prefers_dark_ink("#111111"), "黑底应画白勾");
        // 非法输入回退默认 seed，不 panic
        let fallback = hex_to_rgba("not-a-color");
        let default = hex_to_rgba(DEFAULT_ACCENT);
        assert!((fallback.r - default.r).abs() < 1e-6);
    }

    #[test]
    fn test_resolve_seed_falls_back_to_default() {
        assert_eq!(resolve_seed(None), DEFAULT_ACCENT);
        assert_eq!(resolve_seed(Some("not-a-color")), DEFAULT_ACCENT);
        assert_eq!(resolve_seed(Some("#3B82F6")), "#3b82f6");
    }

    #[test]
    fn test_all_accent_presets_build_valid_theme() {
        for p in ACCENT_PRESETS {
            for dark in [false, true] {
                let c = theme_config(p.hex, dark);
                for name in ["background", "popover", "list", "border", "primary"] {
                    let val = field(&c, name);
                    assert!(val.starts_with('#'), "预设 {} 的 {name} 应为合法 hex: {val}", p.name);
                }
            }
        }
    }

    #[test]
    fn test_resolve_font_and_theme_config() {
        assert_eq!(resolve_font(None).as_ref(), ".SystemUIFont");
        assert_eq!(resolve_font(Some("")).as_ref(), ".SystemUIFont");
        assert_eq!(resolve_font(Some("   ")).as_ref(), ".SystemUIFont");
        assert_eq!(resolve_font(Some("System")).as_ref(), ".SystemUIFont");
        assert_eq!(resolve_font(Some("默认")).as_ref(), ".SystemUIFont");
        assert_eq!(resolve_font(Some(".SystemUIFont")).as_ref(), ".SystemUIFont");
        assert_eq!(resolve_font(Some("Microsoft YaHei UI")).as_ref(), "Microsoft YaHei UI");
        assert_eq!(resolve_font(Some("  PingFang SC  ")).as_ref(), "PingFang SC");

        let cfg = theme_config_with_font("#3B82F6", false, Some("Noto Sans CJK SC"));
        assert_eq!(cfg.font_family.as_deref().map(|s| s.as_ref()), Some("Noto Sans CJK SC"));

        let cfg_default = theme_config_with_font("#3B82F6", false, None);
        assert_eq!(cfg_default.font_family.as_deref().map(|s| s.as_ref()), Some(".SystemUIFont"));
    }
}
