//! 对比 RAW 两条解码路径的白平衡：内嵌 JPEG vs 完整解码（preview 预设）。
//! 输出各自的 R/G、B/G 通道比（尺度无关，直接反映白平衡差异）。
//! 用法: cargo run -p photo-engine --example wb_check -- <file.RW2>

fn channel_ratio_rgb(data: &[u8], channels: usize) -> (f64, f64) {
    let (mut r, mut g, mut b) = (0u64, 0u64, 0u64);
    for px in data.chunks_exact(channels) {
        r += px[0] as u64;
        g += px[1] as u64;
        b += px[2] as u64;
    }
    let g = g.max(1) as f64;
    (r as f64 / g, b as f64 / g)
}

fn main() {
    let path = std::env::args().nth(1).expect("usage: wb_check <raw file>");

    // 基准 0：同名片对 JPG（机内直出，用户认知的正确白平衡）
    let pair = std::path::Path::new(&path).with_extension("jpg");
    if pair.exists() {
        let img = image::open(&pair).unwrap().to_rgb8();
        let (rg, bg) = channel_ratio_rgb(&img, 3);
        println!("片对 JPG:   {}x{}  R/G={rg:.4} B/G={bg:.4}", img.width(), img.height());
    }

    // 路径 1：内嵌 JPEG（相机机内渲染，白平衡基准）
    match rawlib::extract_thumbnail(&path) {
        Ok(jpeg) => {
            let img = image::load_from_memory(&jpeg).unwrap().to_rgb8();
            let (rg, bg) = channel_ratio_rgb(&img, 3);
            println!("内嵌 JPEG:  {}x{}  R/G={rg:.4} B/G={bg:.4}", img.width(), img.height());
        }
        Err(e) => println!("内嵌 JPEG 提取失败: {e}"),
    }

    // 路径 2：完整解码 preview 预设（use_camera_wb=true）
    match rawlib::extract_image_with_options(&path, &rawlib::DecodeOptions::preview()) {
        Ok(img) => {
            println!(
                "完整解码:   {}x{} colors={} bits={}",
                img.width, img.height, img.colors, img.bits
            );
            let (rg, bg) = channel_ratio_rgb(&img.data, img.colors as usize);
            println!("           R/G={rg:.4} B/G={bg:.4}");
        }
        Err(e) => println!("完整解码失败: {e}"),
    }

    // 路径 3：A/B 对照 —— use_camera_wb=false（LibRaw 默认日光 WB）
    let off = rawlib::DecodeOptions { use_camera_wb: false, ..rawlib::DecodeOptions::preview() };
    match rawlib::extract_image_with_options(&path, &off) {
        Ok(img) => {
            let (rg, bg) = channel_ratio_rgb(&img.data, img.colors as usize);
            println!("camera_wb=off: R/G={rg:.4} B/G={bg:.4}（若与 preview 相同说明开关未生效）");
        }
        Err(e) => println!("wb=off 解码失败: {e}"),
    }

    // 路径 4：应用实际路径 —— ThumbnailCache（含磁盘缓存，可能命中陈旧条目）
    let cache_dir = std::env::args().nth(2).unwrap_or_else(|| ".wb_check_cache".into());
    let cache = photo_engine::thumbnail::ThumbnailCache::new(cache_dir.into());
    let source = photo_domain::SourceFile {
        path: path.clone().into(),
        format: photo_domain::ImageFormat::from_extension(
            std::path::Path::new(&path).extension().and_then(|e| e.to_str()).unwrap_or(""),
        ).unwrap_or(photo_domain::ImageFormat::Jpeg),
        file_size: std::fs::metadata(&path).ok().map(|m| m.len()),
    };
    match cache.get_or_generate(&source, 1600, None) {
        Ok(bytes) => {
            let img = image::load_from_memory(&bytes).unwrap().to_rgb8();
            let (rg, bg) = channel_ratio_rgb(&img, 3);
            println!("磁盘缓存路径: {}x{}  R/G={rg:.4} B/G={bg:.4}", img.width(), img.height());
        }
        Err(e) => println!("磁盘缓存路径失败: {e}"),
    }
}
