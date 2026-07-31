//! 并行处理示例：批量提取缩略图 + 批量完整解码
//!
//! 运行：cargo run --release --example parallel_processing -- <RAW 文件目录> [文件数上限]
//!
//! 建议用 --release 运行，debug 模式下 LibRaw 解码速度差异很大。

use rawlib::{DecodeOptions, ParallelConfig, ParallelProcessor};
use std::path::PathBuf;
use std::time::Instant;

fn main() {
    let dir = match std::env::args().nth(1) {
        Some(d) => d,
        None => {
            println!("用法: cargo run --release --example parallel_processing -- <RAW 文件目录> [文件数上限]");
            return;
        }
    };
    let limit: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(16);

    // 收集目录中的 RAW 文件
    let extensions = [
        "cr2", "cr3", "nef", "nrw", "arw", "srf", "sr2", "raf", "orf", "rw2", "dng",
    ];
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("无法读取目录")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .map(|e| extensions.contains(&e.to_string_lossy().to_lowercase().as_str()))
                .unwrap_or(false)
        })
        .collect();
    files.sort();
    files.truncate(limit);

    if files.is_empty() {
        eprintln!("目录中没有找到 RAW 文件: {}", dir);
        std::process::exit(1);
    }
    println!("找到 {} 个 RAW 文件\n", files.len());

    // === 1. 并行提取缩略图 ===
    let start = Instant::now();
    let (results, stats) =
        ParallelProcessor::process_with_stats(&files, &ParallelConfig::default());
    let _ = results;
    println!(
        "缩略图提取: {} 成功 / {} 失败, 总耗时 {:?}, 速度 {:.1} 文件/秒",
        stats.success,
        stats.failed,
        start.elapsed(),
        stats.files_per_second()
    );

    // === 2. 并行完整解码（preview 快速模式） ===
    // process_images 会自动将 LibRaw 内部 OpenMP 限制为单线程，
    // 避免与文件级并行争抢核（实测吞吐差 15-20%）
    let start = Instant::now();
    let results = ParallelProcessor::process_images(
        &files,
        &ParallelConfig::default(),
        &DecodeOptions::preview(),
    );
    let elapsed = start.elapsed();
    let success = results.iter().filter(|r| r.is_success()).count();
    println!(
        "完整解码(preview): {} 成功 / {} 失败, 总耗时 {:?}, 速度 {:.1} 文件/秒",
        success,
        results.len() - success,
        elapsed,
        results.len() as f64 / elapsed.as_secs_f64()
    );

    // 单文件结果示例
    if let Some(first) = results.first() {
        if let Some(img) = first.thumbnail() {
            println!(
                "\n首个文件: {} -> {}x{} {} bit ({} 字节, 耗时 {:?})",
                first.path.display(),
                img.width,
                img.height,
                img.bits,
                img.data.len(),
                first.elapsed
            );
        }
    }
}
