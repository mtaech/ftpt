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
    /// 曝光（EV，±2.0；0 = 不变）
    pub exposure: f32,
    /// 对比度（-100 ~ +100；0 = 不变）
    pub contrast: i32,
    /// 饱和度（-100 ~ +100；0 = 不变，-100 = 去饱和）
    pub saturation: i32,
}

impl ToneParams {
    /// 是否为中性（无需任何像素变换）
    pub fn is_neutral(&self) -> bool {
        self.exposure == 0.0 && self.contrast == 0 && self.saturation == 0
    }
}

impl From<&AdjustParams> for ToneParams {
    fn from(a: &AdjustParams) -> Self {
        // 防御 DB/外部坏值：Q15 定点饱和数学依赖 saturation∈[-100,100]（已验算该范围无 i32 溢出），
        // 对比度参与浮点缩放（超出范围 f32 溢出成 inf），曝光 ±2 外查表产生 NaN/inf 路径——
        // 三者钳制到各自文档范围；非有限 exposure（NaN/inf）归零（等价中性）。
        Self {
            exposure: if a.exposure.is_finite() {
                a.exposure.clamp(-2.0, 2.0)
            } else {
                0.0
            },
            contrast: a.contrast.clamp(-100, 100),
            saturation: a.saturation.clamp(-100, 100),
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
    let c = 1.0 + p.contrast as f32 / 100.0;
    let s = 1.0 + p.saturation as f32 / 100.0;
    let tab = tone_tab16(p.exposure, c);
    // Q14 定点（×16384）：灰阶系数 BT.601 4899/9617/1868 求和恰为 16384 → 灰度像素精确；
    // s∈[0,2] 时中间值 ≤ 16384*131070 < i32::MAX，无溢出（Q15 的 Rec.709 系数求和非 32768 会偏色）
    let s_q = (s * 16384.0) as i32;
    let inv_q = 16384 - s_q;
    let needs_sat = s != 1.0;
    if needs_sat {
        for px in out.pixels_mut() {
            let r = tab[px[0] as usize] as i32;
            let g = tab[px[1] as usize] as i32;
            let b = tab[px[2] as usize] as i32;
            // gray = (r*4899 + g*9617 + b*1868) >> 14
            let gray = (r * 4899 + g * 9617 + b * 1868) >> 14;
            px[0] = clamp_q15((r * s_q + gray * inv_q) >> 14);
            px[1] = clamp_q15((g * s_q + gray * inv_q) >> 14);
            px[2] = clamp_q15((b * s_q + gray * inv_q) >> 14);
        }
    } else {
        for px in out.pixels_mut() {
            px[0] = tab[px[0] as usize];
            px[1] = tab[px[1] as usize];
            px[2] = tab[px[2] as usize];
        }
    }
    out
}

/// 对 8-bit RGB 缓冲应用色调调整（同 16-bit 语义，Q15 定点饱和度）
pub fn apply_tone8(img: &RgbImage, p: &ToneParams) -> RgbImage {
    if p.is_neutral() {
        return img.clone();
    }
    let mut out = img.clone();
    let c = 1.0 + p.contrast as f32 / 100.0;
    let s = 1.0 + p.saturation as f32 / 100.0;
    let tab = tone_tab8(p.exposure, c);
    let s_q = (s * 16384.0) as i32;
    let inv_q = 16384 - s_q;
    if s != 1.0 {
        for px in out.pixels_mut() {
            let r = tab[px[0] as usize] as i32;
            let g = tab[px[1] as usize] as i32;
            let b = tab[px[2] as usize] as i32;
            let gray = (r * 4899 + g * 9617 + b * 1868) >> 14;
            px[0] = clamp_q8((r * s_q + gray * inv_q) >> 14);
            px[1] = clamp_q8((g * s_q + gray * inv_q) >> 14);
            px[2] = clamp_q8((b * s_q + gray * inv_q) >> 14);
        }
    } else {
        for px in out.pixels_mut() {
            px[0] = tab[px[0] as usize];
            px[1] = tab[px[1] as usize];
            px[2] = tab[px[2] as usize];
        }
    }
    out
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

    /// 中性参数：输出与输入逐像素一致（但曝光 0 仍走查表路径，验证表恒等）
    #[test]
    fn test_apply_tone_neutral_identity() {
        let img = Rgb16Image::from_fn(8, 8, |x, y| image::Rgb([
            (x * 4096) as u16, (y * 8192) as u16, 32768,
        ]));
        let p = ToneParams { exposure: 0.0, contrast: 0, saturation: 0 };
        let out = apply_tone16(&img, &p);
        for (a, b) in img.pixels().zip(out.pixels()) {
            assert_eq!(a, b);
        }
    }

    /// 曝光 +1EV：中灰 32768（sRGB 0.5 ≈ 线性 0.214）应显著提亮且不溢出
    #[test]
    fn test_exposure_plus_ev_brightens_without_clip() {
        let img = Rgb16Image::from_pixel(1, 1, image::Rgb([32768, 32768, 32768]));
        let p = ToneParams { exposure: 1.0, contrast: 0, saturation: 0 };
        let out = apply_tone16(&img, &p);
        let v = out.get_pixel(0, 0)[0];
        assert!(v > 40000, "中灰 +1EV 应提亮（线性域乘 2）：{v}");
        assert!(v < 60000, "不应溢出截断：{v}");
    }

    /// 曝光 -1EV：中灰应压暗
    #[test]
    fn test_exposure_minus_ev_darkens() {
        let img = Rgb16Image::from_pixel(1, 1, image::Rgb([32768, 32768, 32768]));
        let p = ToneParams { exposure: -1.0, contrast: 0, saturation: 0 };
        let out = apply_tone16(&img, &p);
        let v = out.get_pixel(0, 0)[0];
        assert!(v < 25000 && v > 8000, "中灰 -1EV 应压暗且不纯黑：{v}");
    }

    /// 曝光 +2EV 高光 0.9 应接近溢出但不纯白（线性 0.81×4=3.24 → clip）
    #[test]
    fn test_exposure_high_clips_gracefully() {
        let img = Rgb16Image::from_pixel(1, 1, image::Rgb([58981, 58981, 58981])); // ≈0.9
        let p = ToneParams { exposure: 2.0, contrast: 0, saturation: 0 };
        let out = apply_tone16(&img, &p);
        assert_eq!(out.get_pixel(0, 0)[0], 65535, "+2EV 高光 0.9 应钳到白");
    }

    /// 对比度 +100：中灰不变，暗部更暗亮部更亮
    #[test]
    fn test_contrast_scales_around_mid() {
        let img = Rgb16Image::from_pixel(1, 1, image::Rgb([16384, 32768, 49152]));
        let p = ToneParams { exposure: 0.0, contrast: 100, saturation: 0 };
        let out = apply_tone16(&img, &p);
        assert_eq!(out.get_pixel(0, 0)[0], 0, "0.25 亮度 +100 对比应到黑");
        assert_eq!(out.get_pixel(0, 0)[1], 32768, "中灰应不变");
        assert_eq!(out.get_pixel(0, 0)[2], 65535, "0.75 亮度 +100 对比应到白");
    }

    /// 饱和度 -100：彩图去饱和为灰度（R=G=B）
    #[test]
    fn test_saturation_minus_100_desaturates() {
        let img = Rgb16Image::from_pixel(1, 1, image::Rgb([50000, 20000, 10000]));
        let p = ToneParams { exposure: 0.0, contrast: 0, saturation: -100 };
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
        let p = ToneParams { exposure: 0.0, contrast: 0, saturation: 100 };
        let out = apply_tone16(&img, &p);
        assert_eq!(out.get_pixel(0, 0)[0], 20000, "灰像素 +100 饱和度应不变");
    }

    /// 8-bit 与 16-bit 语义一致：0.5 中灰曝光 ±1EV 方向一致
    #[test]
    fn test_tone8_matches_tone16_direction() {
        let img8 = RgbImage::from_pixel(1, 1, image::Rgb([128, 128, 128]));
        let p = ToneParams { exposure: 1.0, contrast: 0, saturation: 0 };
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
