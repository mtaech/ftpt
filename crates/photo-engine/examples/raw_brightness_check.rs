//! RAW 预览亮度诊断：对比 机内 JPG / 内嵌 JPEG / 应用母版路径 / auto_bright 口径。
//! 用法: cargo run --release -p photo-engine --example raw_brightness_check -- <file.RW2>
//!
//! 判读：内嵌 JPEG 与机内 JPG 亮度应接近（相机口径）；「应用母版」若明显更暗，
//! 说明 RAW 显示解码掉了自动亮度（`display_preview_options` 见 thumbnail.rs）。
use photo_domain::{ImageFormat, SourceFile};
use photo_engine::thumbnail::ThumbnailCache;
use std::path::Path;

fn pct(mut v: Vec<u8>, p: f64) -> u8 {
    v.sort_unstable();
    v[((v.len() as f64 - 1.0) * p) as usize]
}

fn stats(name: &str, rgb: &image::RgbImage) {
    let mut lumas: Vec<u8> = Vec::with_capacity((rgb.width() * rgb.height()) as usize);
    for p in rgb.pixels() {
        lumas.push((0.2126 * p[0] as f64 + 0.7152 * p[1] as f64 + 0.0722 * p[2] as f64).round() as u8);
    }
    let mean = lumas.iter().map(|&x| x as f64).sum::<f64>() / lumas.len() as f64;
    println!(
        "{name}: {}x{} luma mean={mean:.1} p10={} p50={} p90={} p99={}",
        rgb.width(), rgb.height(),
        pct(lumas.clone(), 0.10), pct(lumas.clone(), 0.50), pct(lumas.clone(), 0.90), pct(lumas, 0.99)
    );
}

fn main() {
    let path = std::env::args().nth(1).expect("用法: raw_brightness_check <RAW>");
    // 基准：机内直出 JPG（缩到 1920 与内嵌图同尺度）与内嵌 JPEG
    let pair = Path::new(&path).with_extension("JPG");
    if pair.exists() {
        let img = image::open(&pair).unwrap().to_rgb8();
        let small = image::imageops::resize(&img, 1920, 1280, image::imageops::FilterType::Lanczos3);
        stats("机内直出 JPG (缩1920)", &small);
    }
    let thumb = rawlib::extract_thumbnail_with_info(&path).unwrap();
    match image::load_from_memory(&thumb.data) {
        Ok(i) => stats("内嵌 JPEG (相机机内)", &i.to_rgb8()),
        Err(e) => println!("内嵌 JPEG 解码失败: {e}"),
    }

    // 应用真实母版路径（预览视图走的就是这条，含磁盘缓存）
    let source = SourceFile {
        path: path.clone().into(),
        format: ImageFormat::from_extension(
            Path::new(&path).extension().and_then(|e| e.to_str()).unwrap_or(""),
        ).unwrap_or(ImageFormat::Jpeg),
        file_size: std::fs::metadata(&path).ok().map(|m| m.len()),
    };
    let cache = ThumbnailCache::new(std::env::temp_dir().join("raw_brightness_check_cache"));
    let master = cache.get_or_generate(&source, 2560, None).unwrap();
    match image::load_from_memory(&master) {
        Ok(i) => stats("应用母版(2560) 真实路径", &i.to_rgb8()),
        Err(e) => println!("母版解码失败: {e}"),
    }

    // 对照：1:1 全尺寸口径（full()）
    let img = rawlib::extract_image_with_options(&path, &rawlib::DecodeOptions::full()).unwrap();
    let rgb = image::RgbImage::from_raw(img.width as u32, img.height as u32, img.data.clone()).unwrap();
    let small = image::imageops::resize(&rgb, 1920, 1280, image::imageops::FilterType::Lanczos3);
    stats("1:1 full() 口径", &small);
}
