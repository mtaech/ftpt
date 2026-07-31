//! 白平衡偏色诊断工具（临时）
//!
//! 用法：cargo run --release --example decode_debug -- <RAW 文件> <输出目录>
//!
//! 输出三份材料用于对比：
//!   thumb.jpg   - 内嵌缩略图（相机机内处理，颜色基准）
//!   wb_off.ppm  - use_camera_wb=0 解码（LibRaw 默认，当前行为）
//!   wb_on.ppm   - use_camera_wb=1 解码（相机拍摄白平衡）
//! 并打印两种解码的 RGB 通道均值，定量判断偏色。

use rawlib::{DecodeOptions, RawProcessor, ThumbnailData};
use std::io::Write;
use std::path::Path;

fn channel_means(img: &ThumbnailData) -> (f64, f64, f64) {
    // 仅支持 8bit RGB（preview 预设输出）
    assert_eq!(img.bits, 8);
    assert_eq!(img.colors, 3);
    let n = img.data.len() as f64 / 3.0;
    let (mut r, mut g, mut b) = (0u64, 0u64, 0u64);
    for px in img.data.chunks_exact(3) {
        r += px[0] as u64;
        g += px[1] as u64;
        b += px[2] as u64;
    }
    (r as f64 / n, g as f64 / n, b as f64 / n)
}

fn write_ppm<P: AsRef<Path>>(img: &ThumbnailData, path: P) -> std::io::Result<()> {
    let mut f = std::fs::File::create(path)?;
    write!(f, "P6\n{} {}\n255\n", img.width, img.height)?;
    f.write_all(&img.data)?;
    Ok(())
}

fn decode(path: &str, use_camera_wb: bool) -> ThumbnailData {
    let mut p = RawProcessor::new().expect("RawProcessor::new 失败");
    p.open_file(path).expect("open_file 失败");
    p.set_decode_options(&DecodeOptions::preview());
    p.set_use_camera_wb(use_camera_wb);
    p.unpack().expect("unpack 失败");
    p.dcraw_process().expect("dcraw_process 失败");
    p.get_image().expect("get_image 失败")
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args
        .get(1)
        .expect("用法: decode_debug <RAW 文件> <输出目录>");
    let out_dir = args.get(2).map(|s| s.as_str()).unwrap_or(".");
    std::fs::create_dir_all(out_dir).unwrap();

    // 1. 内嵌缩略图（颜色基准）
    let thumb = RawProcessor::extract_thumbnail(path).expect("提取缩略图失败");
    let thumb_path = format!("{}/thumb.jpg", out_dir);
    std::fs::write(&thumb_path, &thumb.data).unwrap();
    println!("已保存: {} ({} 字节)", thumb_path, thumb.data.len());

    // 2. 两种白平衡解码对比
    for (wb, name) in [(false, "wb_off"), (true, "wb_on")] {
        let img = decode(path, wb);
        let (r, g, b) = channel_means(&img);
        let ppm = format!("{}/{}.ppm", out_dir, name);
        write_ppm(&img, &ppm).unwrap();
        println!(
            "已保存: {} | {}x{} | RGB 均值 = ({:.1}, {:.1}, {:.1})",
            ppm, img.width, img.height, r, g, b
        );
    }

    // 3. 修复验证：仅使用 preview 预设（不显式设置白平衡），
    //    预设已默认开启相机白平衡，结果应与 wb_on 一致
    let img = RawProcessor::extract_image_with_options(path, &DecodeOptions::preview())
        .expect("extract_image_with_options 失败");
    let (r, g, b) = channel_means(&img);
    let ppm = format!("{}/preset.ppm", out_dir);
    write_ppm(&img, &ppm).unwrap();
    println!(
        "已保存: {} | {}x{} | RGB 均值 = ({:.1}, {:.1}, {:.1})（preview 预设，应≈wb_on）",
        ppm, img.width, img.height, r, g, b
    );
}
