//! 预览母版加载计时（开发工具，对应 §7.2）。
//!
//! 用法：cargo run -p photo-ui --example master_bench -- <图片路径> [缓存目录]
//! 不传缓存目录时默认用图片所在目录的 .pt/thumbs（与 app 行为一致）。

use std::path::PathBuf;
use std::time::Instant;

use photo_domain::{ImageFormat, SourceFile};
use photo_ui::image::ImageManager;

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("用法: master_bench <图片路径> [缓存目录]");
        std::process::exit(2);
    };
    let p = PathBuf::from(&path);
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or_default();
    let source = SourceFile {
        path: p.clone(),
        format: ImageFormat::from_extension(ext).expect("无法识别扩展名"),
        file_size: std::fs::metadata(&p).ok().map(|m| m.len()),
    };
    let cache_dir = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| p.parent().unwrap_or(&p).join(".pt").join("thumbs"));

    let mgr = ImageManager::new(Some(cache_dir.clone()));
    // 打印两个档位的缓存键与命中状态：便于精确判断"这次是冷还是热"、以及定点清理
    {
        use photo_engine::thumbnail::ThumbnailCache;
        use photo_ui::image::{MASTER_SIZE, THUMB_SIZE_GRID};
        let cache = ThumbnailCache::new(cache_dir.clone());
        for (label, size) in [
            ("网格缩略图 440", THUMB_SIZE_GRID),
            ("预览母版 2560", MASTER_SIZE),
        ] {
            let key = cache.cache_key(&source, size, "std");
            println!("{label}: {key}  已缓存={}", cache_dir.join(&key).exists());
        }
        // 1:1 全分辨率母版：独立 full 变体，只有放大到超过母版像素才生成
        let full_key = cache.cache_key(&source, u32::MAX, "full");
        println!("1:1 全分辨率: {full_key}  已缓存={}", cache_dir.join(&full_key).exists());
    }
    println!(
        "文件: {} ({:.1} MB, {:?})",
        p.display(),
        source.file_size.unwrap_or(0) as f64 / 1_048_576.0,
        source.format
    );
    println!("缓存: {}", cache_dir.display());

    let t = Instant::now();
    let cold = mgr.load_master_image(&source, None);
    println!(
        "冷启动（母版未落盘）: {:?} -> {}",
        t.elapsed(),
        match &cold {
            Ok(_) => "ok".to_string(),
            Err(e) => e.clone(),
        }
    );

    let t = Instant::now();
    let warm = mgr.load_master_image(&source, None);
    println!(
        "内存缓存命中        : {:?} -> {}",
        t.elapsed(),
        match &warm {
            Ok(_) => "ok".to_string(),
            Err(e) => e.clone(),
        }
    );

    // 1:1 全分辨率（RAW：AHD 全尺寸解码）。放大到超过母版像素时才走这条路。
    let describe = |r: &Result<std::sync::Arc<gpui_kit::Image>, String>| match r {
        Ok(img) => match image::load_from_memory(&img.bytes) {
            Ok(decoded) => format!("ok {}x{} {}", decoded.width(), decoded.height(), "px"),
            Err(e) => format!("ok（解码尺寸失败: {e}）"),
        },
        Err(e) => e.clone(),
    };
    let t = Instant::now();
    let full = mgr.load_full_image(&source, None);
    println!("1:1 冷启动（全尺寸） : {:?} -> {}", t.elapsed(), describe(&full));

    let t = Instant::now();
    let full_warm = mgr.load_full_image(&source, None);
    println!("1:1 内存命中        : {:?} -> {}", t.elapsed(), describe(&full_warm));
}
