//! 手动冒烟：重扫（F5 / 刷新）的正确性——用户报过「源文件失踪了但刷新没剔除」。
//!
//! 跑法（无头，隔离配置，自建素材，不碰真实照片与配置）：
//!   XDG_CONFIG_HOME=/tmp/pt-rescan-config xvfb-run -a \
//!     cargo run -p photo-ui --example rescan_smoke
//! 全部通过退出码 0；失败退出码 1。
//!
//! 覆盖：
//!   1) 重扫按配置决定递归（曾硬编码 false：开了「包含子目录」的目录一按 F5 就退回单层）
//!   2) 源文件被外部删除后重扫，列表里不再有它
//!   3) 焦点图的源文件失踪后，预览 / 调整槽位一起清掉（别让面板挂着不存在的图）

use std::path::Path;
use std::time::Duration;

use gpui_kit::{AppContext as _, Bounds, Point, WindowBounds, WindowOptions, px, size};

use photo_ui::actions::Rescan;
use photo_ui::state::{AppState, ViewMode};

fn write_jpeg(path: &Path) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("建素材目录失败");
    }
    let img = image::RgbImage::from_fn(64, 48, |x, y| {
        image::Rgb([(x * 3) as u8, (y * 5) as u8, 120])
    });
    img.save(path).expect("写素材失败");
}

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

async fn pump(async_cx: &mut gpui_kit::AsyncApp, ms: u64) {
    async_cx
        .background_executor()
        .timer(Duration::from_millis(ms))
        .await;
}

fn main() {
    photo_ui::app::prepare_headless_smoke();
    let dir = std::env::temp_dir().join(format!("pt_rescan_smoke_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    write_jpeg(&dir.join("top.jpg"));
    write_jpeg(&dir.join("sub").join("inner.jpg"));

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
            let smoke_dir = dir.clone();
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
                let items = |cx: &mut gpui_kit::AsyncApp| -> Vec<String> {
                    cx.update(|cx| {
                        task_state
                            .read(cx)
                            .items
                            .iter()
                            .map(|m| m.display_name())
                            .collect()
                    })
                };

                // ── 1) 默认（不递归）：只扫到顶层那张 ──
                pump(async_cx, 300).await;
                fire(async_cx, handle, Box::new(Rescan));
                pump(async_cx, 2500).await;
                let top_only = items(async_cx);
                check!(
                    format!("默认单层扫描：{top_only:?}"),
                    top_only == vec!["top.jpg".to_string()]
                );

                // ── 2) 打开「包含子目录」后重扫：必须递归（曾硬编码 false，F5 会退回单层）──
                let _ = async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| {
                        state.app_config.include_subdirectories = true;
                        cx.notify();
                    });
                });
                fire(async_cx, handle, Box::new(Rescan));
                pump(async_cx, 2500).await;
                let recursive = items(async_cx);
                check!(
                    format!("重扫按配置递归：{recursive:?}"),
                    recursive.len() == 2 && recursive.iter().any(|n| n == "inner.jpg")
                );

                // ── 3) 关掉配置再重扫：回到单层；顺便把预览停到 top.jpg 上 ──
                let _ = async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| {
                        state.app_config.include_subdirectories = false;
                        state.view_mode = ViewMode::Preview;
                        state.selected_indices = vec![0];
                        state.anchor_index = Some(0);
                        cx.notify();
                    });
                });
                fire(async_cx, handle, Box::new(Rescan));
                pump(async_cx, 2600).await;
                let preview_ready = async_cx.update(|cx| {
                    task_state.read(cx).items.len() == 1
                        && task_state
                            .read(cx)
                            .preview_image
                            .as_ref()
                            .is_some_and(|(p, _)| p.ends_with("top.jpg"))
                });
                check!("预览已装载顶层那张（准备验证失踪后的清理）", preview_ready);

                // ── 4) 外部删掉 top.jpg → 重扫：列表剔除 + 预览槽位清空 ──
                std::fs::remove_file(smoke_dir.join("top.jpg")).expect("删素材失败");
                fire(async_cx, handle, Box::new(Rescan));
                pump(async_cx, 2500).await;
                let (left, has_preview) = async_cx.update(|cx| {
                    let s = task_state.read(cx);
                    (s.items.len(), s.preview_image.is_some())
                });
                check!(format!("失踪文件被剔除（剩 {left} 项）"), left == 0);
                check!("焦点图失踪后预览槽位被清掉", !has_preview);

                let _ = std::fs::remove_dir_all(&smoke_dir);
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
