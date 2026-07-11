use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use gpui::{RenderImage, ImageSource};
use image::Frame;
use smallvec::smallvec;
use photo_tool_core::domain::{ImageFormat, SourceFile};
use photo_tool_core::thumbnail::ThumbnailCache;

pub struct TextureManager {
    cache: ThumbnailCache,
    images: HashMap<String, Arc<RenderImage>>,
    max_images: usize,
}

impl TextureManager {
    pub fn new(cache_dir: PathBuf) -> Self {
        Self {
            cache: ThumbnailCache::new(cache_dir),
            images: HashMap::new(),
            max_images: 200,
        }
    }

    pub fn get_or_load(&mut self, path: &str, size: u32) -> Option<ImageSource> {
        let cache_key = format!("{}:{}", path, size);
        if let Some(ri) = self.images.get(&cache_key) {
            return Some(ImageSource::Render(ri.clone()));
        }

        let path_buf = PathBuf::from(path);
        let ext = path_buf.extension()
            .and_then(|e| e.to_str()).unwrap_or("jpg").to_lowercase();
        let format = ImageFormat::from_extension(&ext).unwrap_or(ImageFormat::Jpeg);
        let source = SourceFile { path: path_buf, format, is_sidecar: false, file_size: None };

        let jpeg_bytes = self.cache.get_or_generate(&source, size).ok()?;
        let img = image::load_from_memory(&jpeg_bytes).ok()?;
        let rgba = img.to_rgba8();

        let frame = Frame::new(rgba);
        let render_image = Arc::new(RenderImage::new(smallvec![frame]));

        if self.images.len() >= self.max_images
            && let Some(k) = self.images.keys().next().cloned() {
                self.images.remove(&k);
            }

        self.images.insert(cache_key, render_image.clone());
        Some(ImageSource::Render(render_image))
    }
}
