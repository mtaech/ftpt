//! 对焦点提取验证：真实 RAW 文件 → ExifMetadata.focus_point。
//! 用法：cargo run -p photo-engine --example focus_check -- <raw文件>

use photo_domain::ImageFormat;

fn main() {
    let path = std::env::args().nth(1).expect("usage: focus_check <image>");
    let p = std::path::Path::new(&path);
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let format = ImageFormat::from_extension(&ext).unwrap_or(ImageFormat::Jpeg);
    match photo_engine::exif::extract_exif(p, &format) {
        Ok(meta) => {
            println!("make={:?} model={:?}", meta.camera.make, meta.camera.model);
            println!(
                "size={}x{} focus={:?}",
                meta.image_width.unwrap_or(0),
                meta.image_height.unwrap_or(0),
                meta.focus_point
            );
        }
        Err(e) => println!("提取失败: {e}"),
    }
}
