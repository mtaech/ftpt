// release 打包产物为 GUI 子系统（不附带控制台窗口）；
// debug 保留控制台便于查看日志输出（release 日志走文件 appender，无需 stdout）
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod action;
mod assets;
mod state;
mod worker;
mod ui;

use gpui::*;
use photo_config::AppConfig;

use crate::state::app::RootView;

fn main() {
    // 日志框架初始化（返回值是文件写入的 WorkerGuard，必须存活到进程结束）
    let _log_guard = init_logging();

    let config_path = photo_config::determine_config_path().unwrap_or_else(|e| {
        tracing::warn!("Failed to determine config path: {e}, using default");
        std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join("PT.toml")
    });
    let app_config = photo_config::load_config(&config_path).unwrap_or_else(|e| {
        tracing::warn!("Failed to load config: {e}, using defaults");
        AppConfig::default()
    });

    // 加载照片工具和 gpui-component 的内置 SVG 资源
    let app = gpui_platform::application().with_assets(crate::assets::AppAssets);
    app.run(move |cx| {
        cx.activate(true);
        gpui_component::init(cx);
        // 默认亮色主题，从配置文件恢复用户保存的主题
        let gc_mode = match app_config.theme {
            photo_config::Theme::Light => gpui_component::theme::ThemeMode::Light,
            photo_config::Theme::Dark => gpui_component::theme::ThemeMode::Dark,
        };
        gpui_component::theme::Theme::change(gc_mode, None, cx);
        // 滚动条始终显示（默认 Scrolling 闲置时透明，看不到也无法拖动）
        gpui_component::theme::Theme::global_mut(cx).scrollbar_show =
            gpui_component::scroll::ScrollbarShow::Always;
        // 从配置文件恢复用户保存的字体
        gpui_component::theme::Theme::global_mut(cx).font_family = app_config.font_family.clone().into();
        // 初始化 photo-tool 主题系统
        let pt_mode = match app_config.theme {
            photo_config::Theme::Light => crate::ui::theme::ThemeMode::Light,
            photo_config::Theme::Dark => crate::ui::theme::ThemeMode::Dark,
        };
        crate::ui::theme::set_mode(pt_mode);
        let cp = config_path.clone();
        let ac = app_config.clone();

        cx.spawn(async move |cx| {
            cx.update(|cx| {
                cx.open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Maximized(Bounds {
                            origin: Default::default(),
                            size: size(px(1400.), px(900.)),
                        })),
                        titlebar: Some(TitlebarOptions {
                            title: Some("ftpt".into()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    move |window, cx| {
                        let view =
                            cx.new(|cx| RootView::new(window, cx, cp.clone(), ac.clone()));
                        cx.new(|cx| gpui_component::Root::new(view, window, cx))
                    },
                )
                .unwrap();
            });
        })
        .detach();
    });
}

/// 初始化日志框架：控制台 + 文件双通道。
/// - 控制台层：RUST_LOG 环境变量控制，默认 info（开发调试用）
/// - 文件层：exe 同级 logs/ 目录，按天滚动（ftpt.log.YYYY-MM-DD），固定 info+，
///   无终端的发布环境下也能排查问题
/// - panic hook：GPUI 会静默吞掉事件处理器中的 panic，hook 把 panic 写入日志
fn init_logging() -> tracing_appender::non_blocking::WorkerGuard {
    use tracing_subscriber::filter::LevelFilter;
    use tracing_subscriber::prelude::*;

    let stdout_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stdout)
        .with_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        );

    // 日志目录随 exe 走（便携约定，与 models/、data/、PT.toml 同级）
    let log_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("logs")))
        .unwrap_or_else(|| std::path::PathBuf::from("logs"));
    let file_appender = tracing_appender::rolling::daily(&log_dir, "ftpt.log");
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);
    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(file_writer)
        .with_ansi(false)
        .with_filter(LevelFilter::INFO);

    tracing_subscriber::registry()
        .with(stdout_layer)
        .with(file_layer)
        .init();

    // 桥接 log crate → tracing：ort 等依赖打日志走 log，需转发到 tracing
    let _ = tracing_log::LogTracer::init();

    std::panic::set_hook(Box::new(|info| {
        tracing::error!(target: "panic", "{info}");
    }));

    tracing::info!("日志框架已初始化，日志目录: {}", log_dir.display());
    guard
}
