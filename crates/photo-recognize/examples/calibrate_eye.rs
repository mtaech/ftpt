//! 眼锐度标定工具（临时）：批量跑识别，输出分数分布并导出眼部裁剪供目检。
//!
//! cargo run -p photo-recognize --example calibrate_eye -- <图片目录> [采样间隔]

use std::path::{Path, PathBuf};

use photo_domain::{Capture, ImageFormat, SourceFile};
use photo_recognize::Recognizer;

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(dir) = args.next() else {
        eprintln!("用法: calibrate_eye <图片目录> [采样间隔]");
        std::process::exit(2);
    };
    let step: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(10).max(1);

    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();
    let models_dir = workspace_root.join("models");
    let catalog_db = workspace_root.join("data").join("pica_ref.db");

    let mut recognizer = Recognizer::new(&models_dir, &catalog_db).unwrap();

    let mut jpgs: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| {
            let p = e.unwrap().path();
            (p.extension().and_then(|x| x.to_str()) == Some("jpg")).then_some(p)
        })
        .collect();
    jpgs.sort();

    let out_dir = std::env::temp_dir().join("eye_calibrate");
    std::fs::create_dir_all(&out_dir).unwrap();

    let mut scores: Vec<(String, f32)> = Vec::new();
    for (i, path) in jpgs.iter().step_by(step).enumerate() {
        eprintln!("[{i}] {}", path.file_name().unwrap().to_string_lossy());
        let base = path.file_stem().unwrap().to_string_lossy().to_string();
        let capture = Capture {
            base_name: base.clone(),
            source_files: vec![SourceFile {
                path: path.clone(),
                format: ImageFormat::Jpeg,
                file_size: std::fs::metadata(path).ok().map(|m| m.len()),
            }],
            primary_index: 0,
        };
        let Ok(rec) = recognizer.recognize(&capture, None) else { continue };
        let (Some(score), Some(eye)) = (rec.eye_sharpness, rec.eye_bbox) else {
            eprintln!("  无眼 (status={:?})", rec.status);
            continue;
        };
        eprintln!("  分数 {score:.2}");

        // 导出眼部裁剪（放大 3 倍上下文）供目检
        if let Ok(img) = image::open(path) {
            let (fw, fh) = (img.width() as f32, img.height() as f32);
            let cx = (eye.x1 + eye.x2) / 2.0;
            let cy = (eye.y1 + eye.y2) / 2.0;
            let half_w = (eye.x2 - eye.x1) * 1.5;
            let half_h = (eye.y2 - eye.y1) * 1.5;
            let x1 = ((cx - half_w).max(0.0) * fw) as u32;
            let y1 = ((cy - half_h).max(0.0) * fh) as u32;
            let w = (((cx + half_w).min(1.0) * fw) as u32 - x1).max(1);
            let h = (((cy + half_h).min(1.0) * fh) as u32 - y1).max(1);
            let crop = image::imageops::crop_imm(&img.to_rgb8(), x1, y1, w, h).to_image();
            let name = format!("{score:.2}_{base}.png");
            crop.save(out_dir.join(&name)).unwrap();
        }
        scores.push((base, score));
    }

    scores.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    println!("\n=== 分数分布（{} 张有眼） ===", scores.len());
    for (name, s) in &scores {
        println!("{s:.2}  {name}");
    }
    println!("裁剪目录: {}", out_dir.display());
}
