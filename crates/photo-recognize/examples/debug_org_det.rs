//! org_det.onnx 输出布局探针（临时调试工具）。
//!
//! 用法：cargo run -p photo-recognize --example debug_org_det -- <图片路径> [models_dir]
//!
//! 打印两个输出的形状、逐通道统计，以及分数最高的若干候选的原始 38 通道值，
//! 用于确定 [1,300,38] 的通道语义（框 / 类别分 / 掩码系数）。

use std::path::{Path, PathBuf};

use image::DynamicImage;
use ort::value::Tensor;

const SIZE: usize = 640;

fn build_input(img: &DynamicImage) -> Vec<f32> {
    let plane = SIZE * SIZE;
    let mut out = vec![0.0f32; 3 * plane];
    let rgb = img.resize_exact(SIZE as u32, SIZE as u32, image::imageops::FilterType::CatmullRom).to_rgb8();
    for (i, px) in rgb.as_raw().chunks_exact(3).enumerate() {
        out[i] = px[0] as f32 / 255.0;
        out[plane + i] = px[1] as f32 / 255.0;
        out[2 * plane + i] = px[2] as f32 / 255.0;
    }
    out
}

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(image) = args.next() else {
        eprintln!("用法: debug_org_det <图片路径> [models_dir]");
        std::process::exit(2);
    };
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest.parent().unwrap().parent().unwrap();
    let models_dir = args.next().map(PathBuf::from).unwrap_or_else(|| root.join("models"));
    let model = models_dir.join("org_det.onnx");

    let img = image::open(&image).expect("图片解码失败");
    let mut session = ort::session::Session::builder()
        .unwrap()
        .commit_from_file(&model)
        .expect("org_det.onnx 加载失败");
    let data = build_input(&img);
    let tensor = Tensor::<f32>::from_array(([1usize, 3, SIZE, SIZE], data.into_boxed_slice())).unwrap();
    let outputs = session.run(ort::inputs![tensor]).expect("推理失败");
    println!("输出个数: {}", outputs.len());

    for i in 0..outputs.len() {
        let (shape, flat) = outputs[i].try_extract_tensor::<f32>().unwrap();
        println!("\n输出[{i}] shape={shape:?} len={}", flat.len());
        if i == 0 {
            // 假定 [1, 300, 38]：逐通道统计（跨 300 行）
            let rows = shape[1] as usize;
            let cols = shape[2] as usize;
            println!("  逐通道 min / max / mean（跨 {rows} 行）：");
            for c in 0..cols {
                let mut mn = f32::INFINITY; let mut mx = f32::NEG_INFINITY; let mut sum = 0.0f32;
                for r in 0..rows { let v = flat[r * cols + c]; mn = mn.min(v); mx = mx.max(v); sum += v; }
                println!("    ch{c:>2}: min={mn:>10.4} max={mx:>10.4} mean={:>9.4}", sum / rows as f32);
            }
            // 假设布局 [x1,y1,x2,y2, conf, cls, 32×mask_coeff]：列出所有 conf>0.05 的行
            const NAMES: [&str; 19] = ["animal","bird","mammal","reptile","amphibian","fish","insect","spider","crustacean","mollusk","worm","plant","flower","tree","leaf","fruit","grass","mushroom","lichen"];
            println!("\n  conf>0.05 的候选（布局假设 4=conf, 5=cls, 6..38=掩码系数）：");
            let mut n = 0;
            for r in 0..rows {
                let conf = flat[r * cols + 4];
                if conf <= 0.05 { continue; }
                let cls = flat[r * cols + 5] as i32;
                let name = NAMES.get(cls as usize).copied().unwrap_or("?");
                let b: Vec<f32> = (0..4).map(|c| (flat[r * cols + c] * 100.0).round() / 100.0).collect();
                println!("    row {r:>3} conf={conf:.3} cls={cls}({name}) box_px={b:?}");
                n += 1;
                if n >= 12 { println!("    ...(截断)"); break; }
            }
            if n == 0 { println!("    (无)"); }
        } else {
            println!("  min={:?} max={:?}", flat.iter().cloned().fold(f32::INFINITY, f32::min), flat.iter().cloned().fold(f32::NEG_INFINITY, f32::max));
        }
    }
}
