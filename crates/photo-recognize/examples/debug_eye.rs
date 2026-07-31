//! 眼模型调试工具：dump eye.onnx 输入形状 + 原始输出头部关键点。
//!
//! cargo run -p photo-recognize --example debug_eye -- <图片路径> [x1 y1 x2 y2]
//! 可选框为全图归一化鸟框；缺省用整幅图。

use std::path::{Path, PathBuf};

use image::{DynamicImage, GenericImageView};
use ort::value::Tensor;
use photo_recognize::Recognizer;

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(image) = args.next() else {
        eprintln!("用法: debug_eye <图片> [x1 y1 x2 y2]");
        std::process::exit(2);
    };
    let mut positional = Vec::new();
    let mut letterbox = false;
    for a in args {
        if a == "--letterbox" {
            letterbox = true;
        } else {
            positional.push(a);
        }
    }
    let region: Option<[f32; 4]> = if positional.len() >= 4 {
        Some([
            positional[0].parse().unwrap(),
            positional[1].parse().unwrap(),
            positional[2].parse().unwrap(),
            positional[3].parse().unwrap(),
        ])
    } else {
        None
    };

    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();

    let img = image::open(PathBuf::from(&image)).expect("图片解码失败");
    let (full_w, full_h) = img.dimensions();
    println!("图像尺寸: {full_w}x{full_h}");

    let catalog_db = workspace_root.join("data").join("pica_ref.db");
    let mut recognizer = Recognizer::new(&workspace_root.join("models"), &catalog_db)
        .expect("识别器初始化失败");
    println!("推理后端: {:?}", recognizer.backend());
    let session = recognizer.eye_session();

    for (i, input) in session.inputs().iter().enumerate() {
        println!("输入[{i}]: {} shape={:?}", input.name(), input.dtype());
    }
    for (i, output) in session.outputs().iter().enumerate() {
        println!("输出[{i}]: {} shape={:?}", output.name(), output.dtype());
    }

    // 裁剪区域：先按轴 min/max 归一化再 clamp 到 [0,1]，反向/越界坐标在像素换算前消除，
    // 避免 u32 下溢（debug panic）与 crop_imm 越界。
    let [rx1, ry1, rx2, ry2] = region.unwrap_or([0.0, 0.0, 1.0, 1.0]);
    let x1 = rx1.min(rx2).clamp(0.0, 1.0);
    let y1 = ry1.min(ry2).clamp(0.0, 1.0);
    let x2 = rx1.max(rx2).clamp(0.0, 1.0);
    let y2 = ry1.max(ry2).clamp(0.0, 1.0);
    let px1 = ((x1 * full_w as f32).floor() as u32).min(full_w.saturating_sub(1));
    let py1 = ((y1 * full_h as f32).floor() as u32).min(full_h.saturating_sub(1));
    let px2 = (x2 * full_w as f32).ceil() as u32;
    let py2 = (y2 * full_h as f32).ceil() as u32;
    let crop_w = (px2 - px1).clamp(1, full_w);
    let crop_h = (py2 - py1).clamp(1, full_h);
    println!("裁剪: origin=({px1},{py1}) size={crop_w}x{crop_h}");
    let crop = DynamicImage::ImageRgba8(
        image::imageops::crop_imm(&img, px1, py1, crop_w, crop_h).to_image(),
    );

    // 固定 640x640（与 eye.rs 缺省一致；若模型静态形状不同会在 run 时报错）
    let (input_w, input_h) = (640usize, 640usize);
    let resized = if letterbox {
        // letterbox：等比缩放放进 640x640，灰边填充（YOLO 训练标准预处理）
        let scale = (input_w as f32 / crop_w as f32).min(input_h as f32 / crop_h as f32);
        let new_w = ((crop_w as f32 * scale).round() as u32).max(1);
        let new_h = ((crop_h as f32 * scale).round() as u32).max(1);
        let scaled = crop.resize_exact(new_w, new_h, image::imageops::FilterType::CatmullRom);
        let mut canvas = image::RgbaImage::from_pixel(
            input_w as u32,
            input_h as u32,
            image::Rgba([114, 114, 114, 255]),
        );
        let dx = (input_w as u32 - new_w) / 2;
        let dy = (input_h as u32 - new_h) / 2;
        image::imageops::overlay(&mut canvas, &scaled, dx as i64, dy as i64);
        println!("letterbox: scale={scale:.4} new={new_w}x{new_h} pad=({dx},{dy})");
        DynamicImage::ImageRgba8(canvas)
    } else {
        crop.resize_exact(
            input_w as u32,
            input_h as u32,
            image::imageops::FilterType::CatmullRom,
        )
    };

    let mut input_data = Vec::with_capacity(3 * input_w * input_h);
    for c in 0..3 {
        for y in 0..input_h {
            for x in 0..input_w {
                let pixel = resized.get_pixel(x as u32, y as u32);
                input_data.push(pixel[c] as f32 / 255.0);
            }
        }
    }

    let tensor =
        Tensor::<f32>::from_array(([1usize, 3, input_h, input_w], input_data.into_boxed_slice()))
            .unwrap();
    let outputs = session.run(ort::inputs![tensor]).unwrap();
    let (shape, flat) = outputs[0].try_extract_tensor::<f32>().unwrap();
    println!("输出形状: {shape:?} 长度: {}", flat.len());

    // 按 12 值一行解析，打印置信度最高的 10 个关键点
    let mut kpts: Vec<(usize, usize, f32, f32, f32)> = Vec::new(); // (row, kpt_idx, x, y, conf)
    for (ri, row) in flat.chunks_exact(12).enumerate() {
        println!(
            "row{ri:3}: box=[{:.1},{:.1},{:.1},{:.1}] conf={:.3} cls={:.1} k1=({:.1},{:.1},{:.3}) k2=({:.1},{:.1},{:.3})",
            row[0], row[1], row[2], row[3], row[4], row[5],
            row[6], row[7], row[8], row[9], row[10], row[11],
        );
        if ri >= 4 {
            break;
        }
        for ki in 0..2 {
            let off = 6 + ki * 3;
            kpts.push((ri, ki, row[off], row[off + 1], row[off + 2]));
        }
    }
    // 全量扫描 top-10
    let mut all: Vec<(usize, usize, f32, f32, f32)> = Vec::new();
    for (ri, row) in flat.chunks_exact(12).enumerate() {
        for ki in 0..2 {
            let off = 6 + ki * 3;
            all.push((ri, ki, row[off], row[off + 1], row[off + 2]));
        }
    }
    all.sort_by(|a, b| b.4.partial_cmp(&a.4).unwrap());
    println!("--- 每个关键点槽位最优 ---");
    for ki in 0..2 {
        if let Some((ri, _, x, y, c)) = all.iter().find(|(_, k, _, _, _)| *k == ki) {
            let row = &flat[ri * 12..ri * 12 + 12];
            println!(
                "kpt{ki}: row{ri} raw=({x:.2}, {y:.2}) conf={c:.3} box=[{:.1},{:.1},{:.1},{:.1}]",
                row[0], row[1], row[2], row[3]
            );
        }
    }
    println!("--- Top-10 关键点（原始值） ---");
    for (ri, ki, x, y, c) in all.iter().take(10) {
        let row = &flat[ri * 12..ri * 12 + 12];
        println!(
            "row{ri:3} kpt{ki}: raw=({x:.2}, {y:.2}) conf={c:.3}  box=[{:.1},{:.1},{:.1},{:.1}] boxconf={:.3}",
            row[0], row[1], row[2], row[3], row[4]
        );
        // 假设 A：关键点为输入图绝对像素（当前实现）
        println!("    A 绝对像素: ({:.1}, {:.1})", x, y);
        // 假设 B：关键点为框内归一化坐标
        let bw = row[2] - row[0];
        let bh = row[3] - row[1];
        println!(
            "    B 框归一化: ({:.1}, {:.1})",
            row[0] + x * bw,
            row[1] + y * bh
        );
        // 假设 C：关键点为框内像素（框缩放到 640）
        println!(
            "    C 框内640: ({:.1}, {:.1})",
            row[0] + x / 640.0 * bw,
            row[1] + y / 640.0 * bh
        );
    }
}
