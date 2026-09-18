//! 预览锐度诊断：同一张 RAW 走各条图源，统一缩到显示尺寸后比锐度（拉普拉斯方差 / 平均梯度）。
//! 用法: cargo run --release -p photo-engine --example raw_sharpness_check -- <file.RW2>
//!
//! 判读：显示尺寸下「派生 2560 母版」应与 AHD 全尺寸接近（说明 fit 预览不糊）；
//! 「网格缩略图 440」是母版未就绪时的占位，明显更糊；
//! 「1:1」两行差的倍数 = 当前 RAW 1:1 把母版放大的代价（photo-ui 未接 full 全分辨率路径）。
use photo_domain::{ImageFormat, SourceFile};
use photo_engine::thumbnail::ThumbnailCache;
use std::path::Path;

fn to_luma(img: &image::RgbImage) -> image::GrayImage {
    image::DynamicImage::ImageRgb8(img.clone()).to_luma8()
}

fn lap_var(g: &image::GrayImage) -> f64 {
    let (w, h) = (g.width() as i32, g.height() as i32);
    let mut vals = Vec::new();
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let p = |dx: i32, dy: i32| g.get_pixel((x + dx) as u32, (y + dy) as u32)[0] as f64;
            vals.push(4.0 * p(0, 0) - p(-1, 0) - p(1, 0) - p(0, -1) - p(0, 1));
        }
    }
    let m = vals.iter().sum::<f64>() / vals.len() as f64;
    vals.iter().map(|v| (v - m).powi(2)).sum::<f64>() / vals.len() as f64
}

fn grad(g: &image::GrayImage) -> f64 {
    let (w, h) = (g.width() as i32, g.height() as i32);
    let mut s = 0f64;
    let mut n = 0f64;
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let p = |dx: i32, dy: i32| g.get_pixel((x + dx) as u32, (y + dy) as u32)[0] as f64;
            s += ((p(1, 0) - p(-1, 0)).powi(2) + (p(0, 1) - p(0, -1)).powi(2)).sqrt();
            n += 1.0;
        }
    }
    s / n
}

/// 统一缩到显示宽度再比较（同尺寸才有可比性）
fn report(name: &str, rgb: &image::RgbImage) {
    let l = to_luma(rgb);
    let disp = image::imageops::resize(
        &l,
        812,
        (812.0 * l.height() as f64 / l.width() as f64).round() as u32,
        image::imageops::FilterType::Lanczos3,
    );
    println!(
        "{name}: 源 {}x{} → 显示 812x{} | lapVar={:.1} 平均梯度={:.2}",
        rgb.width(), rgb.height(), disp.height(), lap_var(&disp), grad(&disp)
    );
}

fn bytes_to_rgb(b: &[u8]) -> image::RgbImage {
    image::load_from_memory(b).unwrap().to_rgb8()
}

fn main() {
    let path = std::env::args().nth(1).expect("usage: raw_sharpness_check <RAW>");
    let source = SourceFile {
        path: path.clone().into(),
        format: ImageFormat::from_extension(
            Path::new(&path).extension().and_then(|e| e.to_str()).unwrap_or(""),
        )
        .unwrap_or(ImageFormat::Jpeg),
        file_size: std::fs::metadata(&path).ok().map(|m| m.len()),
    };

    // AHD 全尺寸（1:1 应有的口径）
    let img = rawlib::extract_image_with_options(&path, &rawlib::DecodeOptions::full()).unwrap();
    let full = image::RgbImage::from_raw(img.width as u32, img.height as u32, img.data).unwrap();
    report("AHD 全尺寸 full()      ", &full);

    // 相机内嵌 JPEG
    let thumb = rawlib::extract_thumbnail_with_info(&path).unwrap();
    report("相机内嵌 JPEG          ", &bytes_to_rgb(&thumb.data));

    let cache = ThumbnailCache::new(std::env::temp_dir().join("raw_sharpness_check_cache"));
    report("母版 u32::MAX (4096)   ", &bytes_to_rgb(&cache.get_or_generate(&source, u32::MAX, None).unwrap()));
    report("派生母版 2560（预览用） ", &bytes_to_rgb(&cache.get_or_generate(&source, 2560, None).unwrap()));
    report("网格缩略图 440（占位）  ", &bytes_to_rgb(&cache.get_or_generate(&source, 440, None).unwrap()));

    // 1:1：AHD 中心裁切 vs 2560 母版放大到自然尺寸后中心裁切
    let crop = |src: &image::RgbImage| {
        let (w, h) = (src.width(), src.height());
        image::imageops::crop_imm(src, (w - 812) / 2, (h - 541) / 2, 812, 541).to_image()
    };
    let l = to_luma(&crop(&full));
    println!("真 1:1 (AHD 全尺寸中心裁切): lapVar={:.1} 平均梯度={:.2}", lap_var(&l), grad(&l));
    let master2560 = bytes_to_rgb(&cache.get_or_generate(&source, 2560, None).unwrap());
    let up = image::imageops::resize(
        &master2560,
        full.width(),
        full.height(),
        image::imageops::FilterType::Triangle,
    );
    let l = to_luma(&crop(&up));
    println!("当前 RAW 1:1 (2560 母版放大): lapVar={:.1} 平均梯度={:.2}", lap_var(&l), grad(&l));
}
