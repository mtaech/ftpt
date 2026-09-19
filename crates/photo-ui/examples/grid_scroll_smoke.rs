//! 手动冒烟：网格视图的滚动条与滚动位置（无头，自建素材，不碰真实照片与配置）。
//!
//! 跑法：
//!   XDG_CONFIG_HOME=/tmp/pt-grid-scroll-config xvfb-run -a cargo run -p photo-ui --example grid_scroll_smoke
//! 全部通过退出码 0；失败退出码 1。
//!
//! 想目检滚动条本体（外部截图，可选）：
//!   XDG_CONFIG_HOME=/tmp/pt-grid-scroll-config PHOTO_SMOKE_HOLD_MS=12000 \
//!     xvfb-run -a -n 99 --server-args="-screen 0 1440x900x24" cargo run -p photo-ui --example grid_scroll_smoke
//!   另开终端：DISPLAY=:99 import -window root /tmp/grid.png
//!
//! 背景：用户报「网格视图没有滚动条」。GPUI 的溢出滚动容器（uniform_list 自带
//! overflow.y = Scroll）只负责滚动，**不会自己画滚动条**——必须先 track_scroll 绑定
//! 一个跨帧持有的句柄，再把 Scrollbar 组件叠在同一视口上。本冒烟验证这条链：
//!
//!   1) 扫描出足量素材（少素材时滚动条没有存在意义）
//!   2) 句柄拿到了真实视口（说明 uniform_list 确实按句柄布局）
//!   3) 内容高于视口（滚动条有 thumb 可画）
//!   4) 写句柄偏移后跨帧保留（说明句柄就是列表的滚动状态，Scrollbar 与列表同源）

use std::path::Path;
use std::time::Duration;

use gpui_kit::component::scroll::ScrollbarHandle;
use gpui_kit::{AppContext as _, Bounds, Point, WindowBounds, WindowOptions, point, px, size};

use photo_ui::actions::Rescan;
use photo_ui::state::{AppState, ViewMode};

/// 在窗口上派发一个动作（与视图里 window.dispatch_action(...) 同一条路径）。
fn fire(
    async_cx: &mut gpui_kit::AsyncApp,
    handle: gpui_kit::AnyWindowHandle,
    action: Box<dyn gpui_kit::Action>,
) {
    let _ = async_cx.update(|cx| {
        let _ = handle.update(cx, |_view, window, cx| {
            window.dispatch_action(action, cx);
        });
    });
}

/// 等若干个渲染帧 / 后台任务
async fn pump(async_cx: &mut gpui_kit::AsyncApp, ms: u64) {
    async_cx
        .background_executor()
        .timer(Duration::from_millis(ms))
        .await;
}

/// 生成一张 160×120 的基线 JPEG（只要够小、能进网格即可）
fn write_jpeg(path: &Path, seed: u8) {
    let img = image::RgbImage::from_fn(160, 120, |x, y| {
        image::Rgb([
            seed.wrapping_add((x % 40) as u8),
            seed.wrapping_add((y % 40) as u8),
            120,
        ])
    });
    img.save(path).expect("写测试 JPEG 失败");
}

fn main() {
    // 无头冒烟固定走 Xvfb 的 X11 后端：Wayland 会话下窗口会落到真实桌面，渲染帧不可控
    photo_ui::app::prepare_headless_smoke();
    let dir = std::env::temp_dir().join("pt_grid_scroll_smoke");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建素材目录失败");
    for i in 0..48u8 {
        write_jpeg(&dir.join(format!("gs_{i:02}.jpg")), i.wrapping_mul(5));
    }

    let app = gpui_kit::application().with_assets(gpui_kit::assets::AllAssets);
    app.run(move |cx| {
        gpui_kit::component::init(cx);
        photo_ui::theme::apply(None, false, None, None, cx);
        AppState::register_keybindings(cx);

        let window_options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point {
                    x: px(0.),
                    y: px(0.),
                },
                size: size(px(1100.), px(760.)),
            })),
            titlebar: None,
            ..Default::default()
        };

        cx.open_window(window_options, |window, cx| {
            let state = cx.new(|cx| AppState::build(window, cx));
            let handle = window.window_handle();
            photo_ui::app::focus_root(&state, window, cx);
            state.update(cx, |state, cx| {
                state.current_dir = Some(dir.clone());
                cx.notify();
            });

            let task_state = state.clone();
            cx.spawn(async move |async_cx: &mut gpui_kit::AsyncApp| {
                let mut failures = 0usize;
                macro_rules! check {
                    ($name:expr, $cond:expr) => {
                        if $cond {
                            println!("OK: {}", $name);
                        } else {
                            eprintln!("FAIL: {}", $name);
                            failures += 1;
                        }
                    };
                }

                pump(async_cx, 300).await;
                fire(async_cx, handle, Box::new(Rescan));
                pump(async_cx, 3000).await;

                let count = async_cx.update(|cx| task_state.read(cx).items.len());
                check!(format!("扫描到足量素材（{count} 张）"), count >= 40);
                if count == 0 {
                    eprintln!("FAIL: 扫描没有结果，后续检查无意义");
                    std::process::exit(1);
                }

                // 网格态、3 列（行数够多才有滚动条可言）
                let _ = async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| {
                        state.view_mode = ViewMode::Grid;
                        state.grid_columns = 3;
                        cx.notify();
                    });
                });
                pump(async_cx, 1200).await;

                let viewport_h = async_cx.update(|cx| {
                    f32::from(task_state.read(cx).grid_scroll.viewport_bounds().size.height)
                });
                let content_h = async_cx.update(|cx| {
                    f32::from(task_state.read(cx).grid_scroll.content_size().height)
                });
                check!(
                    format!("滚动句柄拿到真实视口（{viewport_h:.0}px）"),
                    viewport_h > 0.0
                );
                check!(
                    format!("内容高于视口（{content_h:.0}px vs {viewport_h:.0}px）"),
                    content_h > viewport_h + 20.0
                );

                // 写句柄 → 隔一帧读回：偏移被保留才说明句柄就是 uniform_list 的滚动状态。
                // 记着 notify：真实滚轮事件会顺手请求重绘，纯写句柄不会——不重绘就看不到 thumb。
                let _ = async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| {
                        state.grid_scroll.set_offset(point(px(0.), px(-200.)));
                        cx.notify();
                    });
                });
                pump(async_cx, 500).await;
                let offset_after = async_cx.update(|cx| {
                    f32::from(task_state.read(cx).grid_scroll.offset().y)
                });
                check!(
                    format!("句柄偏移跨帧保留（{offset_after:.0}px）"),
                    offset_after < -50.0
                );

                // 目检用：保持窗口存活，并持续轻推偏移让 thumb 停在可见态
                if let Ok(hold_ms) = std::env::var("PHOTO_SMOKE_HOLD_MS")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
                    .ok_or(())
                {
                    eprintln!("保持窗口 {hold_ms}ms 供外部截图…");
                    let steps = (hold_ms / 250).max(1);
                    for i in 0..steps {
                        let y = -200.0 - (i % 5) as f32 * 60.0;
                        let _ = async_cx.update(|cx| {
                            task_state.update(cx, |state, cx| {
                                state.grid_scroll.set_offset(point(px(0.), px(y)));
                                cx.notify();
                            });
                        });
                        pump(async_cx, 250).await;
                    }
                }

                if failures == 0 {
                    println!("全部通过");
                    std::process::exit(0);
                } else {
                    eprintln!("{failures} 项失败");
                    std::process::exit(1);
                }
            })
            .detach();

            state
        })
        .unwrap();
    });
}
