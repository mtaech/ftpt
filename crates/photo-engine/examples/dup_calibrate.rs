//! 近重复检测阈值标定：真实照片目录 → dHash 最近邻距离分布 + 各阈值分组质量。
//! 用法：cargo run -p photo-engine --release --example dup_calibrate -- <目录> [阈值...]
//!
//! 输出（选阈值只看这两块）：
//! 1. **最近邻距离直方图**：每张图到「除自己外最像的那张」的汉明距离。真重复应聚在
//!    低分段（0-6），互不相关照片应在高分段（≥16）；两峰之间的低谷就是阈值的安全区。
//! 2. **各阈值分组统计**：组数 / 入组照片数 / 最大组，外加距离最小的 8 对样例
//!    （人工确认这些对是不是真重复——这是标定唯一可靠依据，单看数字会被
//!    「连拍」这类真实近似照片骗到）。

use std::path::PathBuf;

use photo_engine::phash::{compute_hashes, group_duplicates, hamming};
use photo_engine::thumbnail::ThumbnailCache;
use photo_domain::ImageFormat;

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().expect("usage: dup_calibrate <dir> [thresholds...]"));
    let thresholds: Vec<u32> = {
        let v: Vec<u32> = args.filter_map(|a| a.parse().ok()).collect();
        if v.is_empty() { vec![4, 6, 8, 10, 12, 14] } else { v }
    };

    // 单层扫描：与 app 的 scanner 同口径（默认非递归）
    let mut paths: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("读目录失败") {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if !path.is_file() { continue; }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        if ImageFormat::from_extension(&ext).is_some() {
            paths.push(path.to_string_lossy().to_string());
        }
    }
    paths.sort();
    println!("目录: {}", dir.display());
    println!("图片: {} 张", paths.len());

    let cache = ThumbnailCache::new(dir.join(".pt").join("thumbs"));
    let t0 = std::time::Instant::now();
    let pairs = compute_hashes(&cache, &paths, |_| {}, || false)
        .expect("哈希计算失败")
        .expect("未取消");
    println!("哈希: {} 张, 用时 {:?}", pairs.len(), t0.elapsed());

    // ── 1. 最近邻距离直方图 ──
    let mut nn: Vec<u32> = Vec::with_capacity(pairs.len());
    let mut best_pairs: Vec<(u32, String, String)> = Vec::new();
    for i in 0..pairs.len() {
        let mut min = 64u32;
        for j in 0..pairs.len() {
            if i == j { continue; }
            min = min.min(hamming(pairs[i].1, pairs[j].1));
        }
        nn.push(min);
        if let Some(j) = (0..pairs.len())
            .filter(|&j| j != i)
            .min_by_key(|&j| hamming(pairs[i].1, pairs[j].1))
        {
            let d = hamming(pairs[i].1, pairs[j].1);
            if d <= 16 {
                best_pairs.push((d, pairs[i].0.clone(), pairs[j].0.clone()));
            }
        }
    }
    let mut buckets = [0usize; 33];
    for &d in &nn {
        buckets[(d / 2) as usize] += 1;
    }
    println!("\n── 最近邻距离直方图（每 2 bit 一档）──");
    for (b, &c) in buckets.iter().enumerate() {
        if c == 0 { continue; }
        println!("{:>2}-{:>2}: {:>4} {}", b * 2, b * 2 + 1, c, "#".repeat((c / 2).min(60)));
    }

    // ── 2. 各阈值分组 ──
    println!("\n── 各阈值分组 ──");
    for &th in &thresholds {
        let groups = group_duplicates(pairs.clone(), th);
        let members: usize = groups.iter().map(|g| g.len()).sum();
        let largest = groups.iter().map(|g| g.len()).max().unwrap_or(0);
        println!(
            "阈值 {:>2}: 组数 {:>4}, 入组照片 {:>4}/{}, 最大组 {}, 多余张数 {}",
            th,
            groups.len(),
            members,
            pairs.len(),
            largest,
            members - groups.len()
        );
    }

    // ── 3. 最近对样例（人工确认是否真重复）──
    best_pairs.sort();
    best_pairs.dedup_by(|a, b| (a.1 == b.2 && a.2 == b.1) || (a.1 == b.1 && a.2 == b.2));
    println!("\n── 距离最小的 {} 对（人工确认）──", best_pairs.len().min(12));
    for (d, a, b) in best_pairs.iter().take(12) {
        let name = |p: &str| PathBuf::from(p).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        println!("  {:>2}  {}  ⇄  {}", d, name(a), name(b));
    }

    // ── 4. 可选：打印指定阈值的分组明细（`--groups <阈值>`），人工核对组内容 ──
    if let Some(pos) = std::env::args().position(|a| a == "--groups") {
        let th: u32 = std::env::args().nth(pos + 1).and_then(|v| v.parse().ok()).unwrap_or(10);
        let groups = group_duplicates(pairs.clone(), th);
        println!("\n── 阈值 {} 的分组明细（{} 组）──", th, groups.len());
        for (i, g) in groups.iter().enumerate() {
            let names: Vec<String> = g
                .iter()
                .map(|p| PathBuf::from(p).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())
                .collect();
            println!("  组 {:>2} ({}): {}", i + 1, g.len(), names.join(", "));
        }
    }
}

