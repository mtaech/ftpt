use gpui::{AssetSource, Result, SharedString};
use std::borrow::Cow;

/// 照片工具的Asset源。先查自己的assets，找不到时回退到gpui-component的。
pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        // 先查自己的 assets
        if let Ok(Some(data)) = OwnAssets.load(path) {
            return Ok(Some(data));
        }
        // 回退到 gpui-component 的图标
        gpui_component_assets::Assets.load(path)
    }
}

/// 编译时嵌入的 app assets（从 assets/ 目录）
#[derive(rust_embed::RustEmbed)]
#[folder = "assets"]
struct OwnAssets;
