//! RawLib 库 API 基本用法示例
//!
//! 运行：cargo run --example usage -- <RAW 文件路径>
//! 未提供文件路径时仅打印 API 说明，不会失败。

use rawlib::{
    extract_exif, extract_image_with_options, extract_thumbnail_with_info, DecodeOptions,
};
use std::path::Path;

fn main() {
    let path = match std::env::args().nth(1) {
        Some(p) => p,
        None => {
            println!("用法: cargo run --example usage -- <RAW 文件路径>");
            println!();
            println!("本示例演示：");
            println!("  1. 提取内嵌缩略图（extract_thumbnail_with_info）");
            println!("  2. 提取 EXIF 元数据（extract_exif）");
            println!("  3. 完整解码 RAW 图像（extract_image_with_options + DecodeOptions）");
            return;
        }
    };

    if !Path::new(&path).exists() {
        eprintln!("文件不存在: {}", path);
        std::process::exit(1);
    }

    // === 1. 提取内嵌缩略图（不解码 RAW，最快） ===
    println!("--- 缩略图 ---");
    match extract_thumbnail_with_info(&path) {
        Ok(thumb) => {
            println!(
                "格式: {:?}, 尺寸: {}x{}, 大小: {} 字节",
                thumb.format,
                thumb.width,
                thumb.height,
                thumb.data.len()
            );
        }
        Err(e) => println!("提取缩略图失败: {}", e),
    }

    // === 2. 提取 EXIF 元数据 ===
    println!("\n--- EXIF ---");
    match extract_exif(&path) {
        Ok(exif) => print!("{}", exif.summary()),
        Err(e) => println!("提取 EXIF 失败: {}", e),
    }

    // === 3. 完整解码 RAW 图像 ===
    println!("\n--- 完整解码（preview 快速模式） ---");
    let start = std::time::Instant::now();
    match extract_image_with_options(&path, &DecodeOptions::preview()) {
        Ok(img) => {
            println!(
                "尺寸: {}x{}, {} 通道, {} bit, 像素数据 {} 字节, 耗时 {:?}",
                img.width,
                img.height,
                img.colors,
                img.bits,
                img.data.len(),
                start.elapsed()
            );
        }
        Err(e) => println!("完整解码失败: {}", e),
    }

    println!("\n--- 完整解码（quality 画质模式） ---");
    let start = std::time::Instant::now();
    match extract_image_with_options(&path, &DecodeOptions::quality()) {
        Ok(img) => {
            println!(
                "尺寸: {}x{}, {} 通道, {} bit, 像素数据 {} 字节, 耗时 {:?}",
                img.width,
                img.height,
                img.colors,
                img.bits,
                img.data.len(),
                start.elapsed()
            );
        }
        Err(e) => println!("完整解码失败: {}", e),
    }
}
