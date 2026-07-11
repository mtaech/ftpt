use std::path::PathBuf;
use photo_tool_core::domain::ImageFormat;
use photo_tool_core::convert::{self, ConvertOptions};

#[derive(serde::Deserialize)]
pub struct ConvertInput {
    pub output_dir: String,
    pub output_format: String,
    pub jpeg_quality: u8,
    pub max_dimension: u32,
}

#[tauri::command]
pub async fn convert_images(paths: Vec<String>, options: ConvertInput) -> Result<Vec<String>, String> {
    let opts = ConvertOptions {
        output_dir: PathBuf::from(&options.output_dir),
        output_format: options.output_format,
        jpeg_quality: options.jpeg_quality,
        max_dimension: options.max_dimension,
        overwrite: false,
    };
    let mut results = Vec::new();
    for path_str in &paths {
        let path = PathBuf::from(path_str);
        let ext = path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let format = ImageFormat::from_extension(&ext).unwrap_or(ImageFormat::Jpeg);
        match convert::convert_image(&path, &format, &opts) {
            Ok(out) => results.push(out.to_string_lossy().to_string()),
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(results)
}
