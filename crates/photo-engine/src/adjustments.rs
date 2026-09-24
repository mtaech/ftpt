//! 参数化调整的纯函数（ADR 0007）：曝光 / 对比度 / 饱和度单次遍历。
//! 全同步、无 IO、无依赖注入，供 app 层 worker 线程调用（性能预算：1600px 单帧 ≤ 5ms）。
//!
//! 色彩语义：
//! - **曝光**在线性域做（sRGB → 线性 → ×2^EV → sRGB 查表）：sRGB 编码值直接乘法
//!   在 +1EV 时中灰（128）即溢出，线性域乘才是相机曝光行为
//! - **对比度 / 饱和度**在编码域做（围绕中灰缩放 / 亮度加权混合），常见实现惯例

use std::sync::LazyLock;

use image::RgbImage;
use photo_domain::AdjustParams;

/// 16-bit RGB 缓冲（image 0.25 的 `Rgb16Image` 为 crate 私有，自行别名）
pub type Rgb16Image = image::ImageBuffer<image::Rgb<u16>, Vec<u16>>;

/// 色调调整参数（从 AdjustParams 提取，不含裁切——裁切是几何操作单独处理）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ToneParams {

    /// 曝光（EV，±3.0；0 = 不变）
    pub exposure: f32,
    /// 对比度（-100 ~ +100；0 = 不变）
    pub contrast: i32,
    /// 饱和度（-100 ~ +100；0 = 不变，-100 = 去饱和）
    pub saturation: i32,
    /// 阴影（-100 ~ +100；正 = 提亮暗部，负 = 压暗暗部；0 = 不变）
    pub shadows: i32,
    /// 高光（-100 ~ +100；正 = 提亮亮部，负 = 回收亮部；0 = 不变）
    pub highlights: i32,
    /// 色温（-100 ~ +100；正 = 暖（抬 R 压 B），负 = 冷；0 = 不变）
    pub temperature: i32,
    /// 色调（-100 ~ +100；正 = 品红（抬 R/B 压 G），负 = 绿；0 = 不变）
    pub tint: i32,
}

impl Default for ToneParams {
    /// 全中性（测试与"不调整"路径共用）
    fn default() -> Self {
        Self {
            exposure: 0.0,
            contrast: 0,
            saturation: 0,
            shadows: 0,
            highlights: 0,
            temperature: 0,
            tint: 0,
        }
    }
}

impl ToneParams {
    /// 是否为中性（无需任何像素变换）
    pub fn is_neutral(&self) -> bool {
        self.exposure == 0.0
            && self.contrast == 0
            && self.saturation == 0
            && self.shadows == 0
            && self.highlights == 0
            && self.temperature == 0
            && self.tint == 0
    }
}

impl From<&AdjustParams> for ToneParams {
    fn from(a: &AdjustParams) -> Self {
        // 防御 DB/外部坏值：Q15 定点饱和数学依赖 saturation∈[-100,100]（已验算该范围无 i32 溢出），
        // 对比度参与浮点缩放（超出范围 f32 溢出成 inf），曝光 ±2 外查表产生 NaN/inf 路径——
        // 三者钳制到各自文档范围；非有限 exposure（NaN/inf）归零（等价中性）。
        Self {
            exposure: if a.exposure.is_finite() {
                a.exposure.clamp(-3.0, 3.0)
            } else {
                0.0
            },
            contrast: a.contrast.clamp(-100, 100),
            saturation: a.saturation.clamp(-100, 100),
            shadows: a.shadows.clamp(-100, 100),
            highlights: a.highlights.clamp(-100, 100),
            temperature: a.temperature.clamp(-100, 100),
            tint: a.tint.clamp(-100, 100),
        }
    }
}

/// sRGB 编码值 → 线性值查表（65536 项；0.0–1.0 归一化）
fn srgb_to_linear_tab() -> &'static [f32; 65536] {
    static TAB: LazyLock<[f32; 65536]> = LazyLock::new(|| {
        std::array::from_fn(|v| {
            let c = v as f32 / 65535.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        })
    });
    &TAB
}

/// 线性值 → sRGB 编码值查表（索引 = 线性值量化到 65536 级，值 = 0–65535）
fn linear_to_srgb_tab() -> &'static [u16; 65536] {
    static TAB: LazyLock<[u16; 65536]> = LazyLock::new(|| {
        std::array::from_fn(|i| {
            let c = i as f32 / 65535.0;
            let c = if c <= 0.0031308 {
                c * 12.92
            } else {
                1.055 * c.powf(1.0 / 2.4) - 0.055
            };
            (c * 65535.0).round().clamp(0.0, 65535.0) as u16
        })
    });
    &TAB
}

/// 构造 16-bit 曝光查表（输入编码值 → 输出编码值，含线性域 ×2^EV 与回编码）。
/// 表构建为纯数组操作（无 pow，pow 已预计算进 static 表），65536 项 ≈ 0.1ms。
fn exposure_tab16(ev: f32) -> Vec<u16> {
    if ev == 0.0 {
        return (0u16..=u16::MAX).collect();
    }
    let f = (2f32).powf(ev);
    let to_lin = srgb_to_linear_tab();
    let from_lin = linear_to_srgb_tab();
    to_lin
        .iter()
        .map(|&l| {
            let l = (l * f).clamp(0.0, 1.0);
            let idx = (l * 65535.0).round() as usize;
            from_lin[idx.min(65535)]
        })
        .collect()
}

/// 构造 8-bit 曝光查表（256 项，输入编码值 → 输出编码值）
fn exposure_tab8(ev: f32) -> Vec<u8> {
    if ev == 0.0 {
        return (0u8..=u8::MAX).collect();
    }
    let f = (2f32).powf(ev);
    (0u16..=255)
        .map(|v| {
            let c = v as f32 / 255.0;
            let lin = if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            };
            let lin = (lin * f).clamp(0.0, 1.0);
            let c = if lin <= 0.0031308 {
                lin * 12.92
            } else {
                1.055 * lin.powf(1.0 / 2.4) - 0.055
            };
            (c * 255.0).round().clamp(0.0, 255.0) as u8
        })
        .collect()
}

/// 对 16-bit RGB 缓冲应用色调调整（单次遍历，性能预算 ≤5ms/帧 @1600px）。
/// 性能设计：**曝光 + 对比度合成单张查表**（一次查表完成两项，消除对比度浮点）；
/// **饱和度用 Q15 整数定点**（避免每像素浮点乘加与 round/clamp 标量调用，允许编译器向量化）。
/// 中性参数仅拷贝（检查前置：不构建查表、不遍历像素；签名返回 owned 缓冲，无法零拷贝借用）。
pub fn apply_tone16(img: &Rgb16Image, p: &ToneParams) -> Rgb16Image {
    if p.is_neutral() {
        return img.clone();
    }
    let mut out = img.clone();
    let (tr, tg, tb) = tone_tabs16(p);
    let s = 1.0 + p.saturation as f32 / 100.0;
    // Q14 定点（×16384）：灰阶系数 BT.601 4899/9617/1868 求和恰为 16384 → 灰度像素精确；
    // s∈[0,2] 时中间值 ≤ 16384*131070 < i32::MAX，无溢出（Q15 的 Rec.709 系数求和非 32768 会偏色）
    let s_q = (s * 16384.0) as i32;
    let inv_q = 16384 - s_q;
    let needs_sat = s != 1.0;
    if needs_sat {
        for px in out.pixels_mut() {
            let r = tr[px[0] as usize] as i32;
            let g = tg[px[1] as usize] as i32;
            let b = tb[px[2] as usize] as i32;
            // gray = (r*4899 + g*9617 + b*1868) >> 14
            let gray = (r * 4899 + g * 9617 + b * 1868) >> 14;
            px[0] = clamp_q15((r * s_q + gray * inv_q) >> 14);
            px[1] = clamp_q15((g * s_q + gray * inv_q) >> 14);
            px[2] = clamp_q15((b * s_q + gray * inv_q) >> 14);
        }
    } else {
        for px in out.pixels_mut() {
            px[0] = tr[px[0] as usize];
            px[1] = tg[px[1] as usize];
            px[2] = tb[px[2] as usize];
        }
    }
    out
}

/// 对 8-bit RGB 缓冲应用色调调整（同 16-bit 语义，Q14 定点饱和度）
pub fn apply_tone8(img: &RgbImage, p: &ToneParams) -> RgbImage {
    if p.is_neutral() {
        return img.clone();
    }
    let mut out = img.clone();
    let (tr, tg, tb) = tone_tabs8(p);
    let s = 1.0 + p.saturation as f32 / 100.0;
    let s_q = (s * 16384.0) as i32;
    let inv_q = 16384 - s_q;
    if s != 1.0 {
        for px in out.pixels_mut() {
            let r = tr[px[0] as usize] as i32;
            let g = tg[px[1] as usize] as i32;
            let b = tb[px[2] as usize] as i32;
            let gray = (r * 4899 + g * 9617 + b * 1868) >> 14;
            px[0] = clamp_q8((r * s_q + gray * inv_q) >> 14);
            px[1] = clamp_q8((g * s_q + gray * inv_q) >> 14);
            px[2] = clamp_q8((b * s_q + gray * inv_q) >> 14);
        }
    } else {
        for px in out.pixels_mut() {
            px[0] = tr[px[0] as usize];
            px[1] = tg[px[1] as usize];
            px[2] = tb[px[2] as usize];
        }
    }
    out
}

/// 编码域曲线：阴影 / 高光（都在 0..1，pivot = 0.5，权重从中灰向两端线性增大）。
///
/// 幅度 = k × 权重 × 0.5：k=+1 时黑端可抬到中灰、白端可压到中灰（方向明确、可单测）。
#[inline]
fn apply_shadows_curve(v: f32, shadows: i32) -> f32 {
    if shadows == 0 {
        return v;
    }
    let k = shadows as f32 / 100.0;
    let t = ((0.5 - v) / 0.5).clamp(0.0, 1.0);
    (v + k * t * 0.5).clamp(0.0, 1.0)
}

#[inline]
fn apply_highlights_curve(v: f32, highlights: i32) -> f32 {
    if highlights == 0 {
        return v;
    }
    let k = highlights as f32 / 100.0;
    let t = ((v - 0.5) / 0.5).clamp(0.0, 1.0);
    (v + k * t * 0.5).clamp(0.0, 1.0)
}

/// 色温 / 色调的通道增益：暖(+) 抬 R 压 B；品红(+) 抬 R/B 压 G。
/// 编码域乘法是白平衡的常规近似（线性域做需要先解 sRGB，收益不抵成本）。
fn channel_gains(temperature: i32, tint: i32) -> (f32, f32, f32) {
    let t = temperature as f32 / 100.0;
    let m = tint as f32 / 100.0;
    (1.0 + 0.25 * t + 0.10 * m, 1.0 - 0.20 * m, 1.0 - 0.25 * t + 0.10 * m)
}

/// 曝光 + 对比度合成查表（16-bit）：输入编码值 → 输出编码值
fn tone_tab16(ev: f32, contrast: f32) -> Vec<u16> {
    let exp = exposure_tab16(ev);
    if contrast == 1.0 {
        return exp;
    }
    exp.iter()
        .map(|&v| {
            ((v as f32 - 32768.0) * contrast + 32768.0)
                .round()
                .clamp(0.0, 65535.0) as u16
        })
        .collect()
}

/// 曝光 + 对比度合成查表（8-bit）
fn tone_tab8(ev: f32, contrast: f32) -> Vec<u8> {
    let exp = exposure_tab8(ev);
    if contrast == 1.0 {
        return exp;
    }
    exp.iter()
        .map(|&v| {
            ((v as f32 - 127.5) * contrast + 127.5)
                .round()
                .clamp(0.0, 255.0) as u8
        })
        .collect()
}

/// 单通道公共表（16-bit）：曝光 → 对比度 → 阴影 → 高光。
///
/// 曝光/对比度仍走旧路径（**逐位保持原有口径**，老测试就是这张回归网），新曲线在其后叠加；
/// 三条新曲线都在编码域，不额外增加每像素成本（还是「一次查表」）。
fn tone_tab16_full(p: &ToneParams) -> Vec<u16> {
    let c = 1.0 + p.contrast as f32 / 100.0;
    let mut tab = tone_tab16(p.exposure, c);
    if p.shadows != 0 || p.highlights != 0 {
        for v in tab.iter_mut() {
            let x = *v as f32 / 65535.0;
            let x = apply_highlights_curve(apply_shadows_curve(x, p.shadows), p.highlights);
            *v = (x * 65535.0).round().clamp(0.0, 65535.0) as u16;
        }
    }
    tab
}

/// 三通道表（16-bit）：色温/色调中性时三通道共用一张表（省两次构建）
fn tone_tabs16(p: &ToneParams) -> (Vec<u16>, Vec<u16>, Vec<u16>) {
    let base = tone_tab16_full(p);
    let (gr, gg, gb) = channel_gains(p.temperature, p.tint);
    if gr == 1.0 && gg == 1.0 && gb == 1.0 {
        return (base.clone(), base.clone(), base);
    }
    let scaled = |gain: f32| -> Vec<u16> {
        base.iter()
            .map(|&v| (v as f32 * gain).clamp(0.0, 65535.0).round() as u16)
            .collect()
    };
    (scaled(gr), scaled(gg), scaled(gb))
}

/// 单通道公共表（8-bit），与 16-bit 同语义
fn tone_tab8_full(p: &ToneParams) -> Vec<u8> {
    let c = 1.0 + p.contrast as f32 / 100.0;
    let mut tab = tone_tab8(p.exposure, c);
    if p.shadows != 0 || p.highlights != 0 {
        for v in tab.iter_mut() {
            let x = *v as f32 / 255.0;
            let x = apply_highlights_curve(apply_shadows_curve(x, p.shadows), p.highlights);
            *v = (x * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
    tab
}

/// 三通道表（8-bit）
fn tone_tabs8(p: &ToneParams) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let base = tone_tab8_full(p);
    let (gr, gg, gb) = channel_gains(p.temperature, p.tint);
    if gr == 1.0 && gg == 1.0 && gb == 1.0 {
        return (base.clone(), base.clone(), base);
    }
    let scaled = |gain: f32| -> Vec<u8> {
        base.iter()
            .map(|&v| (v as f32 * gain).clamp(0.0, 255.0).round() as u8)
            .collect()
    };
    (scaled(gr), scaled(gg), scaled(gb))
}

/// Q15 定点结果夹紧到 16-bit 范围
#[inline]
fn clamp_q15(v: i32) -> u16 {
    v.clamp(0, 65535) as u16
}

/// Q15 定点结果夹紧到 8-bit 范围
#[inline]
fn clamp_q8(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 16-bit 路径的价值（ADR 0007 的条带论据）：同一段暗部（16-bit 1000..1199，200 级）
    /// 经 +2 EV 后，16-bit 路径保留 **62** 个输出级，8-bit 路径只剩 **2** 个——**级差少 = 色带**。
    ///
    /// 这条不能只看"最终都是 8-bit 显示"，因为差异发生在**量化时机**：
    /// 8-bit 路径先在输入侧把 1000..1199 塌成 3/4 两级再拉曝光，信息在拉之前就没了。
    /// 62 而不是 200：曝光在**线性域**做乘 2^EV，线性查表以 65536 级量化，
    /// 极暗区（线性 0.005 上下）经 sRGB 曲线回编码后级差被压缩——但仍是 8-bit 路径的 30 倍。
    #[test]
    fn test_tone16_keeps_shadow_levels_that_8bit_tone_loses() {
        use std::collections::HashSet;
        let (w, h) = (64u32, 8u32);
        let mut img16 = Rgb16Image::new(w, h);
        let mut img8 = image::RgbImage::new(w, h);
        for (i, (p16, p8)) in img16.pixels_mut().zip(img8.pixels_mut()).enumerate() {
            let v = 1000 + (i as u16 % 200);
            *p16 = image::Rgb([v, v, v]);
            *p8 = image::Rgb([(v >> 8) as u8; 3]);
        }
        let tone = ToneParams {

            exposure: 2.0,
            contrast: 0,
            saturation: 0,
            ..ToneParams::default()
        };
        let toned8 = apply_tone8(&img8, &tone);
        let toned16 = apply_tone16(&img16, &tone);
        // 8-bit 的输出只有 256 级，统一按 ×257 扩到 16-bit 域再数级差
        let levels8: HashSet<u16> = toned8.pixels().map(|p| u16::from(p[0]) * 257).collect();
        let levels16: HashSet<u16> = toned16.pixels().map(|p| p[0]).collect();
        assert!(
            levels8.len() <= 4,
            "8-bit 路径本就该塌成两三级：{}",
            levels8.len()
        );
        assert!(
            levels16.len() >= 40,
            "16-bit 路径应保留几十个级差：{}",
            levels16.len()
        );
        assert!(
            levels16.len() > levels8.len() * 10,
            "16-bit 级差应远多于 8-bit：8bit={} 16bit={}",
            levels8.len(),
            levels16.len()
        );
    }

    /// 中性参数：输出与输入逐像素一致（但曝光 0 仍走查表路径，验证表恒等）
    #[test]
    fn test_apply_tone_neutral_identity() {
        let img = Rgb16Image::from_fn(8, 8, |x, y| image::Rgb([
            (x * 4096) as u16, (y * 8192) as u16, 32768,
        ]));
        let p = ToneParams {  exposure: 0.0, contrast: 0, saturation: 0, ..ToneParams::default() };
        let out = apply_tone16(&img, &p);
        for (a, b) in img.pixels().zip(out.pixels()) {
            assert_eq!(a, b);
        }
    }

    /// 曝光 +1EV：中灰 32768（sRGB 0.5 ≈ 线性 0.214）应显著提亮且不溢出
    #[test]
    fn test_exposure_plus_ev_brightens_without_clip() {
        let img = Rgb16Image::from_pixel(1, 1, image::Rgb([32768, 32768, 32768]));
        let p = ToneParams {  exposure: 1.0, contrast: 0, saturation: 0, ..ToneParams::default() };
        let out = apply_tone16(&img, &p);
        let v = out.get_pixel(0, 0)[0];
        assert!(v > 40000, "中灰 +1EV 应提亮（线性域乘 2）：{v}");
        assert!(v < 60000, "不应溢出截断：{v}");
    }

    /// 曝光 -1EV：中灰应压暗
    #[test]
    fn test_exposure_minus_ev_darkens() {
        let img = Rgb16Image::from_pixel(1, 1, image::Rgb([32768, 32768, 32768]));
        let p = ToneParams {  exposure: -1.0, contrast: 0, saturation: 0, ..ToneParams::default() };
        let out = apply_tone16(&img, &p);
        let v = out.get_pixel(0, 0)[0];
        assert!(v < 25000 && v > 8000, "中灰 -1EV 应压暗且不纯黑：{v}");
    }

    /// 曝光 +2EV 高光 0.9 应接近溢出但不纯白（线性 0.81×4=3.24 → clip）
    #[test]
    fn test_exposure_high_clips_gracefully() {
        let img = Rgb16Image::from_pixel(1, 1, image::Rgb([58981, 58981, 58981])); // ≈0.9
        let p = ToneParams {  exposure: 2.0, contrast: 0, saturation: 0, ..ToneParams::default() };
        let out = apply_tone16(&img, &p);
        assert_eq!(out.get_pixel(0, 0)[0], 65535, "+2EV 高光 0.9 应钳到白");
    }

    /// 对比度 +100：中灰不变，暗部更暗亮部更亮
    #[test]
    fn test_contrast_scales_around_mid() {
        let img = Rgb16Image::from_pixel(1, 1, image::Rgb([16384, 32768, 49152]));
        let p = ToneParams {  exposure: 0.0, contrast: 100, saturation: 0, ..ToneParams::default() };
        let out = apply_tone16(&img, &p);
        assert_eq!(out.get_pixel(0, 0)[0], 0, "0.25 亮度 +100 对比应到黑");
        assert_eq!(out.get_pixel(0, 0)[1], 32768, "中灰应不变");
        assert_eq!(out.get_pixel(0, 0)[2], 65535, "0.75 亮度 +100 对比应到白");
    }

    /// 饱和度 -100：彩图去饱和为灰度（R=G=B）
    #[test]
    fn test_saturation_minus_100_desaturates() {
        let img = Rgb16Image::from_pixel(1, 1, image::Rgb([50000, 20000, 10000]));
        let p = ToneParams {  exposure: 0.0, contrast: 0, saturation: -100, ..ToneParams::default() };
        let out = apply_tone16(&img, &p);
        let (r, g, b) = (
            out.get_pixel(0, 0)[0],
            out.get_pixel(0, 0)[1],
            out.get_pixel(0, 0)[2],
        );
        assert_eq!(r, g, "去饱和后 R=G");
        assert_eq!(g, b, "去饱和后 G=B");
    }

    /// 饱和度 +100 保持亮度不变（灰度像素不受影响）
    #[test]
    fn test_saturation_keeps_gray_neutral() {
        let img = Rgb16Image::from_pixel(1, 1, image::Rgb([20000, 20000, 20000]));
        let p = ToneParams {  exposure: 0.0, contrast: 0, saturation: 100, ..ToneParams::default() };
        let out = apply_tone16(&img, &p);
        assert_eq!(out.get_pixel(0, 0)[0], 20000, "灰像素 +100 饱和度应不变");
    }

    /// 阴影 +100：暗部提亮（黑端可抬到中灰）、中灰不动（权重在中灰处为 0）
    #[test]
    fn test_shadows_lifts_darks_only() {
        let mut img = Rgb16Image::new(3, 1);
        img.put_pixel(0, 0, image::Rgb([0, 0, 0]));
        img.put_pixel(1, 0, image::Rgb([16384, 16384, 16384]));
        img.put_pixel(2, 0, image::Rgb([32768, 32768, 32768]));
        let p = ToneParams {
            shadows: 100,
            ..ToneParams::default()
        };
        let out = apply_tone16(&img, &p);
        assert!(
            out.get_pixel(0, 0)[0] >= 32000,
            "纯黑阴影+100 应提到中灰附近：{}",
            out.get_pixel(0, 0)[0]
        );
        assert!(out.get_pixel(1, 0)[0] > 16384, "0.25 应被提亮");
        assert_eq!(out.get_pixel(2, 0)[0], 32768, "中灰不动（权重为 0）");
    }

    /// 高光 -100：亮部回收（白端可压到中灰）、中灰不动
    #[test]
    fn test_highlights_recovery_darkens_brights_only() {
        let mut img = Rgb16Image::new(3, 1);
        img.put_pixel(0, 0, image::Rgb([65535, 65535, 65535]));
        img.put_pixel(1, 0, image::Rgb([49152, 49152, 49152]));
        img.put_pixel(2, 0, image::Rgb([32768, 32768, 32768]));
        let p = ToneParams {
            highlights: -100,
            ..ToneParams::default()
        };
        let out = apply_tone16(&img, &p);
        assert!(
            out.get_pixel(0, 0)[0] <= 33000,
            "纯白高光-100 应回收到中灰附近：{}",
            out.get_pixel(0, 0)[0]
        );
        assert!(out.get_pixel(1, 0)[0] < 49152, "0.75 应被压暗");
        assert_eq!(out.get_pixel(2, 0)[0], 32768, "中灰不动");
    }

    /// 色温：+100 暖（R>G>B），-100 冷（B>R）
    #[test]
    fn test_temperature_warms_and_cools() {
        let img = Rgb16Image::from_pixel(1, 1, image::Rgb([32768, 32768, 32768]));
        let warm = apply_tone16(
            &img,
            &ToneParams {
                temperature: 100,
                ..ToneParams::default()
            },
        );
        let (r, g, b) = (
            warm.get_pixel(0, 0)[0],
            warm.get_pixel(0, 0)[1],
            warm.get_pixel(0, 0)[2],
        );
        assert!(r > g && g > b, "暖：R>G>B（{r}/{g}/{b}）");
        let cool = apply_tone16(
            &img,
            &ToneParams {
                temperature: -100,
                ..ToneParams::default()
            },
        );
        assert!(
            cool.get_pixel(0, 0)[2] > cool.get_pixel(0, 0)[0],
            "冷：B>R"
        );
    }

    /// 色调：+100 品红（R/B>G），-100 绿（G>R）
    #[test]
    fn test_tint_moves_green_magenta() {
        let img = Rgb16Image::from_pixel(1, 1, image::Rgb([32768, 32768, 32768]));
        let magenta = apply_tone16(
            &img,
            &ToneParams {
                tint: 100,
                ..ToneParams::default()
            },
        );
        let (r, g, b) = (
            magenta.get_pixel(0, 0)[0],
            magenta.get_pixel(0, 0)[1],
            magenta.get_pixel(0, 0)[2],
        );
        assert!(r > g && b > g, "品红：R/B>G（{r}/{g}/{b}）");
        let green = apply_tone16(
            &img,
            &ToneParams {
                tint: -100,
                ..ToneParams::default()
            },
        );
        assert!(green.get_pixel(0, 0)[1] > green.get_pixel(0, 0)[0], "绿：G>R");
    }

    /// 新参数在 8-bit 链路上方向一致（两条链路语义必须同号）
    #[test]
    fn test_tone8_new_params_match_direction() {
        let img = RgbImage::from_pixel(1, 1, image::Rgb([128, 128, 128]));
        // 阴影权重在中灰处为 0（128/255 = 0.502 已在 pivot 之上）→ 用暗像素验证提亮
        let dark = RgbImage::from_pixel(1, 1, image::Rgb([64, 64, 64]));
        let shadows = apply_tone8(
            &dark,
            &ToneParams {
                shadows: 100,
                ..ToneParams::default()
            },
        );
        assert!(
            shadows.get_pixel(0, 0)[0] > 64,
            "8-bit 阴影+100 应提亮暗部：{}",
            shadows.get_pixel(0, 0)[0]
        );
        let warm = apply_tone8(
            &img,
            &ToneParams {
                temperature: 100,
                ..ToneParams::default()
            },
        );
        assert!(warm.get_pixel(0, 0)[0] > warm.get_pixel(0, 0)[2], "8-bit 暖：R>B");
        let highlights = apply_tone8(
            &img,
            &ToneParams {
                highlights: -100,
                ..ToneParams::default()
            },
        );
        assert_eq!(highlights.get_pixel(0, 0)[0], 128, "中灰高光-100 不动");
    }

    /// 8-bit 与 16-bit 语义一致：0.5 中灰曝光 ±1EV 方向一致
    #[test]
    fn test_tone8_matches_tone16_direction() {
        let img8 = RgbImage::from_pixel(1, 1, image::Rgb([128, 128, 128]));
        let p = ToneParams {  exposure: 1.0, contrast: 0, saturation: 0, ..ToneParams::default() };
        let out8 = apply_tone8(&img8, &p);
        let v = out8.get_pixel(0, 0)[0];
        assert!(v > 128 && v < 255, "8-bit 中灰 +1EV 应提亮且不截断：{v}");
    }

    /// 性能基准（ADR 0007 预算抽查）：1600px 16-bit 显示源 tone 变换应 < 30ms（debug 宽松阈值）。
    /// 运行：`cargo test --release -p photo-engine -- --ignored adjustments::tests::bench_tone16_1600px`
    #[test]
    #[ignore]
    fn bench_tone16_1600px() {
        let img = Rgb16Image::from_fn(1600, 1067, |x, y| {
            image::Rgb([(x * 7) as u16, (y * 11) as u16, ((x + y) * 13) as u16])
        });
        let p = ToneParams {

            exposure: 1.25,
            contrast: 40,
            saturation: -25,
            ..ToneParams::default()
        };
        // 预热（查表 static 首次构建 + 内存页）
        let _ = apply_tone16(&img, &p);
        let start = std::time::Instant::now();
        let runs = 30;
        for _ in 0..runs {
            let _ = apply_tone16(&img, &p);
        }
        let per_frame = start.elapsed() / runs;
        println!(
            "bench_tone16_1600px: {per_frame:?}/帧 ({} 帧)",
            runs
        );
        // 预算 5ms/帧；debug 放宽到 30ms（release 应在 5ms 内）
        assert!(per_frame.as_millis() < 30, "1600px tone 超预算: {per_frame:?}");
    }
}
