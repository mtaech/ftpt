//! 临时端到端冒烟：复制图片到系统剪贴板（跑完即删）。
//!
//! 走的是生产同一条通路：焦点根视图 → dispatch CopyImage 动作 → AppState 监听器
//! → engine_ops 解码全尺寸 RGBA → arboard 写系统剪贴板，最后用 arboard 读回校验尺寸。
//!
//! 跑法：xvfb-run -a cargo run -p photo-ui --example clipboard_smoke

use std::time::Duration;

use gpui_kit::{AppContext as _, Bounds, Point, WindowBounds, WindowOptions, px, size};

use photo_ui::actions::CopyImage;
use photo_ui::state::AppState;

async fn pump(async_cx: &mut gpui_kit::AsyncApp, ms: u64) {
    async_cx
        .background_executor()
        .timer(Duration::from_millis(ms))
        .await;
}

fn main() {
    // 无头冒烟固定走 Xvfb 的 X11 后端：Wayland 会话下窗口会落到真实桌面，渲染帧不可控
    photo_ui::app::prepare_headless_smoke();
    let app = gpui_kit::application().with_assets(gpui_kit::assets::AllAssets);
    app.run(|cx| {
        gpui_kit::component::init(cx);
        photo_ui::theme::apply(None, false, None, None, cx);
        AppState::register_keybindings(cx);

        // 一张 64×48 的测试 JPEG
        let dir = std::env::temp_dir().join("pt_clipboard_smoke");
        let _ = std::fs::remove_dir_all(&dir); // 清掉上次跑剩的坏文件
        let _ = std::fs::create_dir_all(&dir);
        // 用 PNG：JPEG 编码器不支持 RGBA
        let png = dir.join("clip.png");
        image::RgbaImage::from_fn(64, 48, |x, y| image::Rgba([x as u8, y as u8, 128, 255]))
            .save(&png)
            .expect("写测试 PNG");

        let window_options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point {
                    x: px(0.),
                    y: px(0.),
                },
                size: size(px(900.), px(640.)),
            })),
            titlebar: None,
            ..Default::default()
        };

        cx.open_window(window_options, |window, cx| {
            let state = cx.new(|cx| AppState::build(window, cx));
            let handle = window.window_handle();
            photo_ui::app::focus_root(&state, window, cx);
            photo_ui::state::engine_ops::start_scan(state.clone(), dir.clone(), false, cx);

            // 与生产接线一致：根视图是 gpui-component 的 Root（tooltip / 原生菜单浮层挂它）
            let task_state = state.clone();
            let root_view = state.clone();
            let root = cx.new(|cx| {
                gpui_kit::component::Root::new(root_view, window, cx).bordered(false)
            });

            cx.spawn(async move |async_cx: &mut gpui_kit::AsyncApp| {
                // tooltip 的前提：根视图必须是 Root（Root::tooltip_overlay 靠 window.root::<Root>() 查找）
                pump(async_cx, 300).await;
                let root_ok = async_cx
                    .update(|cx| {
                        handle
                            .update(cx, |_root, window, _cx| {
                                window
                                    .root::<gpui_kit::component::Root>()
                                    .flatten()
                                    .is_some()
                            })
                            .unwrap_or(false)
                    });
                if root_ok {
                    println!("ROOT_OK 根视图是 gpui-component Root（tooltip 前提成立）");
                } else {
                    eprintln!("FAIL 根视图不是 Root，tooltip 会被静默丢弃");
                    std::process::exit(1);
                }

                // 等扫描出 1 项
                let mut items = 0usize;
                for _ in 0..40 {
                    pump(async_cx, 200).await;
                    items = async_cx.update(|cx| task_state.read(cx).items.len());
                    if items > 0 {
                        break;
                    }
                }
                assert!(items > 0, "扫描没出条目");
                async_cx.update(|cx| {
                    task_state.update(cx, |state, _| {
                        state.selected_indices = vec![0];
                        state.anchor_index = Some(0);
                    });
                });
                pump(async_cx, 300).await;

                // 与预览工具条按钮完全同一条通路
                let _ = async_cx.update(|cx| {
                    let _ = handle.update(cx, |_view, window, cx| {
                        window.dispatch_action(Box::new(CopyImage), cx);
                    });
                });
                pump(async_cx, 3000).await;

                let got = async_cx
                    .background_executor()
                    .spawn(async move {
                        arboard::Clipboard::new()
                            .ok()
                            .and_then(|mut c| c.get_image().ok())
                    })
                    .await;

                match got {
                    Some(img) if (img.width, img.height) == (64, 48) => {
                        println!("CLIPBOARD_E2E_OK {}x{}", img.width, img.height);
                        println!(
                            "状态栏：{:?}",
                            async_cx.update(|cx| task_state.read(cx).status_message.clone())
                        );
                    }
                    Some(img) => {
                        eprintln!("FAIL 尺寸不符：{}x{}", img.width, img.height);
                        std::process::exit(1);
                    }
                    None => {
                        eprintln!(
                            "FAIL 剪贴板没有图片；状态栏：{:?}",
                            async_cx.update(|cx| task_state.read(cx).status_message.clone())
                        );
                        std::process::exit(1);
                    }
                }
                let _ = async_cx.update(|cx| cx.quit());
            })
            .detach();

            root
        })
        .expect("开窗失败");
    });
}
