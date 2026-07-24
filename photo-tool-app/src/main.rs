mod action;
mod assets;
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

    // 加载照片工具和 gpui-component 的内置 SVG 资源
    let app = gpui_platform::application().with_assets(crate::assets::AppAssets);
    app.run(move |cx| {
        cx.activate(true);
        let cp = config_path.clone();
        gpui_component::init(cx);
        // 默认亮色主题，从配置文件恢复用户保存的主题
        let gc_mode = match app_config.theme {
            config::Theme::Light => gpui_component::theme::ThemeMode::Light,
            config::Theme::Dark => gpui_component::theme::ThemeMode::Dark,
        };
        gpui_component::theme::Theme::change(gc_mode, None, cx);
        // 滚动条始终显示（默认 Scrolling 闲置时透明，看不到也无法拖动）
        gpui_component::theme::Theme::global_mut(cx).scrollbar_show =
            gpui_component::scroll::ScrollbarShow::Always;
        // 从配置文件恢复用户保存的字体
        gpui_component::theme::Theme::global_mut(cx).font_family = app_config.font_family.clone().into();
        // 初始化 photo-tool 主题系统
        let pt_mode = match app_config.theme {
            config::Theme::Light => crate::ui::theme::ThemeMode::Light,
            config::Theme::Dark => crate::ui::theme::ThemeMode::Dark,
        };
        crate::ui::theme::set_mode(pt_mode);
        let ac = app_config.clone();

        // 窗口图标通过 build.rs 嵌入 Windows .ico 资源，不在此处设置
        // （GPUI WindowOptions::icon 仅 X11 有效）

        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Maximized(Bounds {
                    origin: Default::default(),
                    size: size(px(1400.), px(900.)),
                })),
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
