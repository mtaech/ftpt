//! 调整预览烘焙计时（开发工具，3.5 性能实测）。
//!
//! 用法：cargo run --release -p photo-ui --example adjust_bench [图片路径]
//! 不给路径时用自建 2560 中灰渐变图测纯变换/编码成本；给路径时额外测端到端
//! （母版加载 + 烘焙；RAW 还会测 16-bit 母版解码与命中后的帧时）。
//!
//! 目的：回答「2560 + JPEG 编码这条链路到底贵在哪」——调色、降位深、编码三段分开量，
//! 免得凭感觉去改提交方式（ADR 0007 的预算是按 1600px 写的，这里把 1600/2560 两档都印出来）。

use std::io::Cursor;
use std::path::PathBuf;
use std::time::Instant;

use photo_domain::{AdjustParams, ImageFormat, SourceFile};
use photo_engine::adjustments::{Rgb16Image, ToneParams, apply_tone8, apply_tone16};
use photo_engine::convert::rgb16_to_rgb8;
use photo_ui::image::{ImageManager, MASTER_SIZE};

/// 重复取中位数（第一次当预热丢掉）
fn median_ms(runs: usize, mut f: impl FnMut()) -> f64 {
    let mut samples = Vec::new();
    for i in 0..=runs {
        let t = Instant::now();
        f();
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        if i > 0 {
            samples.push(ms);
        }
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    samples[samples.len() / 2]
}

/// 渐变 RGB8（长边 size）
fn gradient8(size: u32) -> image::RgbImage {
    let (w, h) = (size, (size as f64 * 0.667) as u32);
    image::RgbImage::from_fn(w, h, |x, y| {
        let v = ((x % 256) as u8).saturating_add(((y % 64) / 2) as u8).saturating_add(80);
        image::Rgb([v, v.saturating_add(4), v.saturating_sub(4)])
    })
}

/// 渐变 16-bit（长边 size）
fn gradient16(size: u32) -> Rgb16Image {
    let (w, h) = (size, (size as f64 * 0.667) as u32);
    Rgb16Image::from_fn(w, h, |x, y| {
        let v = ((x % 256) as u16) * 250 + ((y % 64) as u16) * 8 + 3000;
        image::Rgb([v, v + 64, v.saturating_sub(64)])
    })
}

fn encode_jpeg(img: &image::RgbImage, quality: u8) -> usize {
    let mut buf = Cursor::new(Vec::new());
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, quality);
    enc.encode(
        img.as_raw(),
        img.width(),
        img.height(),
        image::ExtendedColorType::Rgb8,
    )
    .expect("JPEG 编码失败");
    buf.into_inner().len()
}

fn main() {
    let arg = std::env::args().nth(1).map(PathBuf::from);
    let tone = ToneParams {
        exposure: 1.0,
        contrast: 20,
        saturation: 10,
        ..ToneParams::default()
    };

    println!("== 纯变换 / 编码成本（中位数，各跑 6 次）==");
    for size in [1600u32, MASTER_SIZE] {
        let img8 = gradient8(size);
        let img16 = gradient16(size);
        let px = (img8.width() as f64 * img8.height() as f64) as u64;
        let t8 = median_ms(6, || {
            let _ = apply_tone8(&img8, &tone);
        });
        let t16 = median_ms(6, || {
            let _ = apply_tone16(&img16, &tone);
        });
        let down = median_ms(6, || {
            let _ = rgb16_to_rgb8(&img16);
        });
        let enc = median_ms(6, || {
            let _ = encode_jpeg(&img8, 90);
        });
        let full = median_ms(6, || {
            let toned = apply_tone8(&img8, &tone);
            let _ = encode_jpeg(&toned, 90);
        });
        println!(
            "{size}px ({:.1}MP): tone8 {t8:.1}ms | tone16 {t16:.1}ms | 16→8 降位深 {down:.1}ms | JPEG 编码 {enc:.1}ms | 合计(8bit) {full:.1}ms",
            px as f64 / 1e6
        );
        println!(
            "        其中编码占合计的 {:.0}%",
            enc / full * 100.0
        );
    }

    let Some(path) = arg else {
        println!();
        println!("（未给图片路径：以上只是合成图上的变换/编码成本）");
        return;
    };

    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default();
    let Some(format) = ImageFormat::from_extension(ext) else {
        eprintln!("无法识别扩展名: {ext}");
        std::process::exit(2);
    };
    let source = SourceFile {
        path: path.clone(),
        format: format.clone(),
        file_size: std::fs::metadata(&path).ok().map(|m| m.len()),
    };
    let cache_dir = path
        .parent()
        .unwrap_or(&path)
        .join(".pt")
        .join("thumbs");
    let mgr = ImageManager::new(Some(cache_dir));
    let params = AdjustParams {

        exposure: 1.0,
        contrast: 20,
        saturation: 10,
            ..AdjustParams::default()
        };

    println!();
    println!("== 端到端 build_preview_frame（{}）==", path.display());
    let t = Instant::now();
    let cold = mgr.build_preview_frame(&source, &params, false);
    println!("冷（母版未落盘）: {:?} -> {}", t.elapsed(), cold.is_ok());
    let hot = median_ms(6, || {
        let _ = mgr.build_preview_frame(&source, &params, false);
    });
    println!("热（母版 + 像素缓存命中）: {hot:.1}ms");
    let clip = median_ms(3, || {
        let _ = mgr.build_preview_frame(&source, &params, true);
    });
    println!("热 + 剪切掩码（O 键打开时）: {clip:.1}ms");

    if matches!(format, ImageFormat::Raw(_)) {
        println!();
        println!("== RAW 16-bit 母版 ==");
        let t = Instant::now();
        let ok = mgr.ensure_raw16_master(&source).is_ok();
        println!(
            "16-bit 母版解码（half_size+16bit，一次性）: {:?} -> {ok}",
            t.elapsed()
        );
        let hot16 = median_ms(6, || {
            let _ = mgr.build_preview_frame(&source, &params, false);
        });
        println!("命中 16-bit 母版后的帧时: {hot16:.1}ms");
    }
}
