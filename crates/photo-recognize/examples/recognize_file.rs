//! 单文件识别冒烟工具：cargo run -p photo-recognize --example recognize_file -- <图片路径>
//!
//! 模型与名录库默认取 worktree 根的 models/ 与 data/pica_ref.db，
//! 可用第二、三参数覆盖：<图片> [models_dir] [catalog_db]

use std::path::{Path, PathBuf};

use photo_domain::{Capture, ImageFormat, SourceFile};
use photo_recognize::Recognizer;

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(image) = args.next() else {
        eprintln!("用法: recognize_file <图片路径> [models_dir] [catalog_db]");
        std::process::exit(2);
    };
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();
    let models_dir = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace_root.join("models"));
    let catalog_db = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace_root.join("data").join("pica_ref.db"));

    let image_path = PathBuf::from(&image);
    let ext = image_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();
    let format = ImageFormat::from_extension(ext).unwrap_or_else(|| {
        eprintln!("不支持的扩展名: {ext}");
        std::process::exit(2);
    });
    let file_size = std::fs::metadata(&image_path).ok().map(|m| m.len());
    let base_name = image_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    let capture = Capture {
        base_name,
        source_files: vec![SourceFile {
            path: image_path.clone(),
            format,
            file_size,
        }],
        primary_index: 0,
    };

    eprintln!("加载模型: {}", models_dir.display());
    let mut recognizer = Recognizer::new(&models_dir, &catalog_db).unwrap_or_else(|e| {
        eprintln!("初始化识别器失败: {e}");
        std::process::exit(1);
    });

    let started = std::time::Instant::now();
    let progress = |p: photo_recognize::RecognitionProgress| {
        eprintln!("  [{:>5.1}%] {}", p.value * 100.0, p.stage);
    };
    let result = recognizer
        .recognize(&capture, Some(&progress))
        .unwrap_or_else(|e| {
            eprintln!("识别过程系统故障: {e}");
            std::process::exit(1);
        });

    println!("--- 结果（{:?}）---", started.elapsed());
    println!("status:        {:?}", result.status);
    println!("failure_stage: {:?}", result.failure_stage);
    if let Some(bird) = &result.bird {
        println!(
            "bird:          {} ({}) [id={}]",
            bird.cn_name, bird.latin_name, bird.bird_id
        );
    }
    if let Some(conf) = result.confidence {
        println!("confidence:    {conf:.1}%");
    }
    if let Some(bbox) = &result.bbox {
        println!(
            "bbox:          [{:.3}, {:.3}, {:.3}, {:.3}]",
            bbox.x1, bbox.y1, bbox.x2, bbox.y2
        );
    }
    println!("candidates:    {}", result.candidates.len());
    for c in &result.candidates {
        let name = c
            .bird
            .as_ref()
            .map(|b| b.cn_name.as_str())
            .unwrap_or("<未映射>");
        println!("  - cls={} {name} {:.1}%", c.class_index, c.confidence);
    }
}
