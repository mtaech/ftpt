//! 手动冒烟：预览图源选择——1:1（显示尺寸超过母版像素）应后台换成全分辨率图源。
//!
//! 跑法（无头，隔离配置，不碰真实配置）：
//!   XDG_CONFIG_HOME=/tmp/pt-full-preview-config xvfb-run -a \
//!     cargo run --release -p photo-ui --example preview_full_smoke -- <含 RAW 的目录>
//! 目录里只放一张 RAW 最稳妥（多张时取第一张）。全部通过退出码 0；失败退出码 1。
//!
//! 验证三点：
//!   1) 适应（fit）时 preview_full 保持 None（不该为 fit 付全尺寸代价）
//!   2) 按 1:1 后 preview_full 被填上，且字节量是母版量级之上的真原图
//!   3) 路径与当前选中图一致（切换图不会串图源）

use std::path::PathBuf;
use std::time::Duration;

use gpui_kit::{AppContext as _, Bounds, Point, WindowBounds, WindowOptions, px, size};

use photo_ui::actions::Rescan;
use photo_ui::state::{AppState, ViewMode};

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
    // 无头冒烟固定走 Xvfb 的 X11 后端：Wayland 会话下窗口会落到真实桌面，渲染帧不可控
    photo_ui::app::prepare_headless_smoke();
    let Some(dir) = std::env::args().nth(1).map(PathBuf::from) else {
        eprintln!("跳过：未提供含 RAW 的目录（用法见文件头）");
        return;
    };
    if !dir.is_dir() {
        eprintln!("FAIL: 目录不存在 {}", dir.display());
        std::process::exit(1);
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
                pump(async_cx, 2500).await;

                let count = async_cx.update(|cx| task_state.read(cx).items.len());
                check!("扫描到照片", count >= 1);
                if count == 0 {
                    eprintln!("FAIL: 扫描没有结果，后续检查无意义");
                    std::process::exit(1);
                }

                // 进预览 + 选中第 0 张，保持「适应」
                let _ = async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| {
                        state.view_mode = ViewMode::Preview;
                        state.selected_indices = vec![0];
                        state.anchor_index = Some(0);
                        state.preview_zoom = 1.0;
                        cx.notify();
                    });
                });
                pump(async_cx, 800).await;
                check!(
                    "适应窗口时不加载全分辨率",
                    async_cx.update(|cx| task_state.read(cx).preview_full.is_none())
                );

                // 切 1:1：显示尺寸 = EXIF 自然尺寸 > 2560 母版 → 应后台换全分辨率
                let path = async_cx.update(|cx| {
                    task_state
                        .read(cx)
                        .primary_selected_meta()
                        .map(|m| m.primary_path.clone())
                });
                let _ = async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| {
                        state.preview_zoom = 0.0;
                        cx.notify();
                    });
                });
                pump(async_cx, 6000).await;

                let loaded = async_cx.update(|cx| {
                    task_state
                        .read(cx)
                        .preview_full
                        .as_ref()
                        .map(|(p, img)| (p.clone(), img.bytes.len()))
                });
                match loaded {
                    Some((p, bytes)) => {
                        check!("1:1 已加载全分辨率图源", true);
                        check!("图源路径与选中图一致", Some(p) == path);
                        // 母版 2560 只有几百 KB；全尺寸 RAW 导出 JPEG 量级在 MB 以上
                        check!(
                            format!("字节量是全尺寸量级（{} KB）", bytes / 1024),
                            bytes > 2_000_000
                        );
                    }
                    None => check!("1:1 已加载全分辨率图源", false),
                }

                if failures == 0 {
                    println!("全部通过");
                    // 必须显式退出：examples 带上了 gpui 的 leak-detection（dev-dependencies 的
                    // test-support 打开），正常析构会被判成 leaked handles；而且 GPUI 事件循环
                    // 自己不会结束（这条以前漏了，成功路径会一直挂着不返回）。
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
