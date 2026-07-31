//! 完整解码管线的性能基准测试
//!
//! 用法：
//!   cargo run --release --example bench_decode -- <目录> [阶段计时文件数] [并行文件数]
//!
//! 输出：
//!   1. 单文件各阶段耗时（open / unpack / dcraw_process / make_mem_image+拷贝）
//!   2. 批量并行吞吐（files/s）
//!
//! 用 OMP_NUM_THREADS 环境变量控制 LibRaw 内部 OpenMP 线程数做对比：
//!   OMP_NUM_THREADS=1 cargo run --release --example bench_decode -- <目录>

use rawlib::{DecodeOptions, ParallelConfig, ParallelProcessor, RawProcessor};
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn collect_rw2(dir: &str, limit: usize) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("无法读取目录")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .map(|e| e.to_string_lossy().eq_ignore_ascii_case("rw2"))
                .unwrap_or(false)
        })
        .collect();
    files.sort();
    files.truncate(limit);
    files
}

fn fmt_ms(d: Duration) -> String {
    format!("{:7.1}ms", d.as_secs_f64() * 1000.0)
}

/// 单文件逐阶段计时
fn bench_stages(files: &[PathBuf], opts: &DecodeOptions, label: &str) {
    let mut t_open = Duration::ZERO;
    let mut t_unpack = Duration::ZERO;
    let mut t_process = Duration::ZERO;
    let mut t_mem = Duration::ZERO;
    let mut out_bytes = 0u64;
    let mut dims = String::new();

    for f in files {
        let mut p = RawProcessor::new().expect("RawProcessor::new 失败");

        let t = Instant::now();
        p.open_file(f).expect("open_file 失败");
        t_open += t.elapsed();

        p.set_decode_options(opts);

        let t = Instant::now();
        p.unpack().expect("unpack 失败");
        t_unpack += t.elapsed();

        let t = Instant::now();
        p.dcraw_process().expect("dcraw_process 失败");
        t_process += t.elapsed();

        let t = Instant::now();
        let img = p.get_image().expect("get_image 失败");
        t_mem += t.elapsed();

        out_bytes += img.data.len() as u64;
        dims = format!("{}x{} {}bit", img.width, img.height, img.bits);
    }

    let n = files.len() as u32;
    let total = t_open + t_unpack + t_process + t_mem;
    println!(
        "[{}] {} 个文件均值 | open {} | unpack {} | process {} | mem+拷贝 {} | 合计 {} | 输出 {} ({:.0}MB/张)",
        label,
        files.len(),
        fmt_ms(t_open / n),
        fmt_ms(t_unpack / n),
        fmt_ms(t_process / n),
        fmt_ms(t_mem / n),
        fmt_ms(total / n),
        dims,
        out_bytes as f64 / files.len() as f64 / 1e6,
    );
}

/// 批量并行吞吐
fn bench_parallel(files: &[PathBuf], opts: &DecodeOptions, label: &str) {
    let t = Instant::now();
    let results = ParallelProcessor::process_images(files, &ParallelConfig::default(), opts);
    let elapsed = t.elapsed();
    let ok = results.iter().filter(|r| r.is_success()).count();
    let out_mb: f64 = results
        .iter()
        .filter_map(|r| r.thumbnail())
        .map(|t| t.data.len() as f64)
        .sum::<f64>()
        / 1e6;
    println!(
        "[{}] 并行 {} 个文件: {:?} | {:.2} 文件/秒 | {:.0} MB 输出 | 成功 {}/{}",
        label,
        files.len(),
        elapsed,
        files.len() as f64 / elapsed.as_secs_f64(),
        out_mb,
        ok,
        files.len(),
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = args.get(1).map(|s| s.as_str()).unwrap_or(".");
    let stage_n: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(3);
    let par_n: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(8);

    println!("LibRaw 版本: {}", RawProcessor::version());
    println!("CPU 核心数: {}", num_cpus::get());
    println!(
        "OMP_NUM_THREADS: {}",
        std::env::var("OMP_NUM_THREADS").unwrap_or_else(|_| "(未设置)".to_string())
    );
    println!();

    let quality = DecodeOptions::quality();
    let bilinear_16 = DecodeOptions {
        half_size: false,
        demosaic_quality: 0,
        output_bps: 16,
        no_auto_bright: true,
        output_color: 1,
        linear_gamma: false,
        use_camera_wb: true,
    };
    let raw_pipeline = DecodeOptions {
        half_size: false,
        demosaic_quality: 0,
        output_bps: 16,
        no_auto_bright: true,
        output_color: 0,
        linear_gamma: true,
        use_camera_wb: true,
    };
    let preview = DecodeOptions::preview();

    // === 单文件逐阶段计时 ===
    let files = collect_rw2(dir, stage_n);
    if !files.is_empty() {
        println!("--- 单文件逐阶段（{} 个文件，串行） ---", files.len());
        bench_stages(&files, &quality, "quality: AHD+16bit+自动亮度");
        bench_stages(&files, &bilinear_16, "bilinear+16bit+无自动亮度");
        bench_stages(&files, &raw_pipeline, "raw管线: bilinear+无色彩转换+线性");
        bench_stages(&files, &preview, "preview: half+bilinear+8bit");
        println!();
    }

    // === 批量并行吞吐 ===
    let files = collect_rw2(dir, par_n);
    println!("--- 批量并行（{} 个文件） ---", files.len());
    bench_parallel(&files, &preview, "preview");
    bench_parallel(&files, &quality, "quality");
}
