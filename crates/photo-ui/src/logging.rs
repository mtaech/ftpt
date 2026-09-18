//! 应用日志：全局 tracing 订阅者初始化 + 日志路径查询。
//!
//! 现状：Rust 侧（photo-engine / photo-recognize / 本 crate）已大量使用 `tracing::*` 埋点，
//! 但此前没有任何订阅者初始化——所有日志被静默丢弃（排查缩略图管线时踩过这个坑：
//! `tracing::debug!` 什么也不输出，只能临时插 `eprintln!`）。
//!
//! `main()` 启动时安装全局订阅者，把日志落到磁盘滚动文件：
//!
//! - **落盘**：滚动日切文件 `<配置目录>/logs/ftpt.YYYY-MM-DD.log`
//!   （`tracing-appender` 非阻塞 worker，最多保留 14 份，超限自动清理）
//! - **stderr**：同步输出到终端（`cargo run -p photo-ui` 时在控制台可见）
//! - **log crate 桥接**：`tracing_log::LogTracer` 把 rawlib 等第三方依赖的 `log::*`
//!   接入同一管道
//! - **级别过滤**：debug 构建默认 `DEBUG`，release 默认 `INFO`；
//!   可用环境变量 `PHOTO_LOG_LEVEL`（如 `debug` / `info` / `warn` / `error`）覆盖
//!
//! 返回的 `WorkerGuard` 必须存活到应用退出（由 `main` 持有），否则非阻塞
//! writer 的缓冲日志会在退出时丢失。

use std::path::PathBuf;
use std::sync::OnceLock;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;

/// 当前日志目录（`init` 后确定；供设置「关于」页与打开日志入口读取）
static LOG_DIR: OnceLock<PathBuf> = OnceLock::new();

/// 日志目录：`~/.config/pt/logs/`（Windows 为 `%USERPROFILE%\.config\pt\logs\`），
/// 与配置文件（`photo_config::determine_config_path` → `~/.config/pt/config.toml`）同级。
/// 必须在 `init()` 之后调用。
pub fn log_dir() -> &'static PathBuf {
    LOG_DIR
        .get()
        .expect("logging::init 必须先于 log_dir() 调用")
}

/// 今日日志文件路径（滚动日切文件名 `ftpt.YYYY-MM-DD.log`）。
pub fn today_log_file() -> PathBuf {
    let stamp = chrono::Local::now().format("%Y-%m-%d");
    log_dir().join(format!("ftpt.{stamp}.log"))
}

/// 安装全局 tracing 订阅者（进程生命周期内只能调用一次；重复调用 panic）。
/// 返回 `WorkerGuard`，须存活到应用退出。
pub fn init() -> WorkerGuard {
    // 日志目录与配置同级（全平台统一 ~/.config/pt/logs/）
    let base = photo_config::determine_config_path()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    let dir = base.join("logs");
    let _ = std::fs::create_dir_all(&dir);
    let _ = LOG_DIR.set(dir);
    init_into(LOG_DIR.get().expect("LOG_DIR 已设置"))
}

/// 以指定目录初始化订阅者（`init` 的落地实现，便于将来扩展自定义目录）。
fn init_into(dir: &PathBuf) -> WorkerGuard {
    // 滚动日切文件 appender（文件名 `ftpt.YYYY-MM-DD.log`，保留 14 份）
    let file_appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("ftpt")
        // 不设 suffix 时日切文件名只有 ftpt.YYYY-MM-DD（无扩展名），显式加 .log
        .filename_suffix("log")
        .max_log_files(14)
        .build(dir)
        .expect("创建日志文件 appender 失败（日志目录不可写？）");
    // 非阻塞 writer：文件 IO 走独立线程，不阻塞业务线程
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);

    // 级别过滤：PHOTO_LOG_LEVEL 环境变量优先；否则 debug 构建 DEBUG、release INFO。
    //
    // **第三方噪声必须默认压制**——GPUI 直接跑在 wgpu 上，底下这层在 DEBUG 级是刷屏级的：
    // 实测 14 秒 **88,348 行 / 5.6 MB**（cosmic_text 字体枚举、wgpu_hal Vulkan loader
    // 逐调用打印、naga 校验），照这个速率一天能写数 GB。原 Tauri 版用 WebView，没这个问题；
    // GPUI 直接跑在 wgpu 上，所以过滤器必须补这一课。
    // 需要查渲染/输入问题时按目标显式打开：PHOTO_LOG_LEVEL=debug,wgpu_hal=debug
    const NOISY: &str = "ort=warn,wgpu=warn,wgpu_core=warn,wgpu_hal=warn,wgpu_types=warn,\
naga=warn,cosmic_text=warn,swash=warn,fontdb=warn,ash=warn,\
gpui_linux=warn,gpui_wgpu=warn,zbus=warn,calloop=warn";
    let default_level = if cfg!(debug_assertions) {
        "debug"
    } else {
        "info"
    };
    let base = std::env::var("PHOTO_LOG_LEVEL")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| default_level.to_string());
    let filter = EnvFilter::new(format!("{base},{NOISY}"));

    // 时间戳：读系统本地时区（chrono::Local），默认 SystemTime 为 UTC 会偏离本地时间
    let timer = tracing_subscriber::fmt::time::ChronoLocal::rfc_3339();

    tracing_subscriber::registry()
        .with(filter)
        // 文件输出：无 ANSI 颜色
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(file_writer)
                .with_ansi(false)
                .with_timer(timer.clone()),
        )
        // 终端输出：debug 构建带颜色（cargo run 控制台可读），release 不带
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr)
                .with_ansi(cfg!(debug_assertions))
                .with_timer(timer),
        )
        .init();

    // 桥接 log crate（rawlib 等第三方依赖的 log::* 进同一日志管道）
    let _ = tracing_log::LogTracer::init();

    guard
}
