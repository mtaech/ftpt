use std::path::PathBuf;

use gpui_kit::component::Colorize as _;
use gpui_kit::{AppContext as _, Bounds, Point, WindowBounds, WindowOptions, px, size};

use photo_ui::state::AppState;
use photo_ui::state::engine_ops::start_scan;

fn main() {
    // 日志管道必须最先装：Rust 侧（engine / recognize / 本 crate）已有大量 tracing 埋点，
    // 没有订阅者时全部被静默丢弃。WorkerGuard 存活到进程退出，保证退出时缓冲日志落盘。
    let _log_guard = photo_ui::logging::init();

    // 第一行日志先把"环境"钉下来：版本、日志落点、数据根——排查问题时先看这一行
    tracing::info!(
        "photo-ui 启动 v{} · 日志目录 {} · 数据根 {:?}",
        env!("CARGO_PKG_VERSION"),
        photo_ui::logging::log_dir().display(),
        photo_ui::state::app_state::data_root()
    );

    // 图标资产：默认 Assets 只打包 101 个组件图标，这里注册 AllAssets 用完整
    // Lucide / GPUI Kit 目录（1,830 个，二进制约 +1 MiB），避免"这个图标不在内置集里"。
    let app = gpui_kit::application().with_assets(gpui_kit::assets::AllAssets);
    app.run(|cx| {
        gpui_kit::component::init(cx);

        // 按持久化配置激活主题（Material You · 墨白；seed 非法 / 缺失回退默认）
        let app_config = photo_ui::state::app_state::load_app_config();
        let seed = photo_ui::theme::resolve_seed(app_config.accent_color.as_deref());
        let dark = matches!(app_config.theme, photo_config::Theme::Dark);
        photo_ui::theme::apply(Some(&seed), dark, None, cx);
        // 主题生效证据：把解析后的实际令牌写进日志（排查"主题没上"的第一锚点）
        {
            let theme = gpui_kit::component::theme::Theme::global(cx);
            tracing::info!(
                "主题生效：seed={} mode={:?} radius={}px radius_lg={}px background={} foreground={} primary={} border={}",
                seed,
                theme.mode,
                f32::from(theme.radius),
                f32::from(theme.radius_lg),
                theme.background.to_hex(),
                theme.foreground.to_hex(),
                theme.primary.to_hex(),
                theme.border.to_hex(),
            );
        }

        // 注册全局键位绑定
        AppState::register_keybindings(cx);

        // 窗口配置
        let window_options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point {
                    x: px(100.0),
                    y: px(100.0),
                },
                size: size(px(1440.0), px(900.0)),
            })),
            // 用系统标题栏（服务端装饰）：合成器支持时不再自绘，省掉 36px 自定义顶栏。
            // 合成器不支持时 gpui 会回落到客户端装饰，app.rs 会照旧画自绘标题栏。
            titlebar: Some(gpui_kit::TitlebarOptions {
                title: Some("Photo Tool".into()),
                ..Default::default()
            }),
            window_decorations: Some(gpui_kit::WindowDecorations::Server),
            window_min_size: Some(size(px(1280.0), px(800.0))),
            ..Default::default()
        };

        cx.open_window(window_options, |window, cx| {
            // 显式请求一次：部分合成器只在收到客户端请求后才给系统标题栏
            window.request_decorations(gpui_kit::WindowDecorations::Server);
            let app_state = cx.new(|cx| AppState::build(window, cx));

            // 首帧后聚焦根视图：没有焦点节点时 window.dispatch_action 会丢动作，
            // 所有按钮与快捷键都会失灵。
            photo_ui::app::focus_root(&app_state, window, cx);

            // 启动自愈：检查上次打开的目录
            let last_dir = app_state.read(cx).app_config.last_directory.clone();
            if let Some(dir_str) = last_dir {
                let path = PathBuf::from(&dir_str);
                if path.is_dir() {
                    start_scan(app_state.clone(), path, false, cx);
                }
            }

            app_state
        })
        .expect("打开主窗口失败");
    });
}
