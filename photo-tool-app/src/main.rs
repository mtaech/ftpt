mod action;
mod state;
mod worker;
mod ui;

use gpui::*;
use photo_tool_core::config::{self, AppConfig};

use crate::state::app::RootView;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let config_path = config::determine_config_path().unwrap_or_else(|e| {
        tracing::warn!("Failed to determine config path: {e}, using default");
        std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join("PT.toml")
    });
    let app_config = config::load_config(&config_path).unwrap_or_else(|e| {
        tracing::warn!("Failed to load config: {e}, using defaults");
        AppConfig::default()
    });

    // 加载 gpui-component 内置 SVG 资源（图标等），缺失时所有 IconName 渲染为空白
    let app = gpui_platform::application().with_assets(gpui_component_assets::Assets);
    app.run(move |cx| {
        cx.activate(true);
        let cp = config_path.clone();
        gpui_component::init(cx);
        gpui_component::theme::Theme::change(gpui_component::theme::ThemeMode::Light, None, cx);
        // 初始化 photo-tool 主题系统
        crate::ui::theme::set_mode(crate::ui::theme::ThemeMode::Light);
        let ac = app_config.clone();
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1400.), px(900.)),
                    cx,
                ))),
                ..Default::default()
            },
            move |window, cx| {
                let view = cx.new(|cx| RootView::new(window, cx, cp.clone(), ac.clone()));
                cx.new(|cx| gpui_component::Root::new(view, window, cx))
            },
        )
        .unwrap();
    });
}
