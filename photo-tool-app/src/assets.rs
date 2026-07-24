use gpui::{AssetSource, Result};
use std::borrow::Cow;

/// 照片工具的Asset源。先查自己的assets，找不到时回退到gpui-component的。
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(data) = OwnAssets::get(path) {
            return Ok(Some(Cow::Owned(data.data.to_vec())));
        }
        gpui_component_assets::Assets.load(path)
    }

    fn list(&self, _path: &str) -> Result<Vec<gpui::SharedString>> {
        Ok(Vec::new())
    }
}

/// 编译时嵌入的 app assets（从 assets/ 目录）
#[derive(rust_embed::RustEmbed)]
#[folder = "assets"]
struct OwnAssets;
