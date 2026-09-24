//! 手动冒烟：重扫（F5 / 刷新）的正确性——用户报过「源文件失踪了但刷新没剔除」。
//!
//! 跑法（无头，隔离配置，自建素材，不碰真实照片与配置）：
//!   XDG_CONFIG_HOME=/tmp/pt-rescan-config xvfb-run -a \
//!     cargo run -p photo-ui --example rescan_smoke
//! 全部通过退出码 0；失败退出码 1。
//!
//! 覆盖：
//!   1) 默认单层扫描；「仅添加并浏览」式递归视图被记录为当前视图模式
//!   2) F5 重扫**保持当前视图模式**（曾硬编码 false；改读配置后又漏了「递归视图 + 配置单层」）
//!   3) **删除后重扫仍保持递归**：用户报「多选删除完成后所有图片都不显示了」——
//!      删除前的视图是递归打开的（导入弹窗的「仅添加并浏览」），删除后的重扫退回单层，
//!      子目录里的照片一张都匹配不上，列表看起来全空了（文件其实还在盘上）
//!   4) Ctrl+Z 恢复后的重扫同样保持递归
//!   5) 源文件被外部删除后重扫，列表里不再有它
//!   6) 焦点图的源文件失踪后，预览 / 调整槽位一起清掉（别让面板挂着不存在的图）

use std::path::Path;
use std::time::Duration;

use gpui_kit::{AppContext as _, Bounds, Point, WindowBounds, WindowOptions, px, size};

use photo_ui::actions::{Rescan, Undo};
use photo_ui::state::{AppState, ViewMode, engine_ops};

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

                // ── 2) 模拟导入弹窗的「仅添加并浏览」：显式递归扫描（配置仍是单层）──
                let _ = async_cx.update(|cx| {
                    engine_ops::start_scan(task_state.clone(), smoke_dir.clone(), true, cx);
                });
                pump(async_cx, 2500).await;
                let (recursive, mode) = async_cx.update(|cx| {
                    let s = task_state.read(cx);
                    (
                        s.items.iter().map(|m| m.display_name()).collect::<Vec<_>>(),
                        s.scan_recursive,
                    )
                });
                check!(
                    format!("仅添加并浏览 = 递归视图：{recursive:?}"),
                    recursive.len() == 2 && recursive.iter().any(|n| n == "inner.jpg")
                );
                check!("视图模式被记录为递归（配置仍是单层）", mode);

                // ── 3) F5 重扫：保持递归（以前读配置会退回单层、把列表刷空）──
                fire(async_cx, handle, Box::new(Rescan));
                pump(async_cx, 2500).await;
                let after_f5 = items(async_cx);
                check!(
                    format!("F5 保持递归视图：{after_f5:?}"),
                    after_f5.len() == 2 && after_f5.iter().any(|n| n == "inner.jpg")
                );

                // ── 4) 删掉**顶层**那张 → 重扫：列表仍显示子目录里剩下那张（**不是 0**）──
                // 用户报的 bug：多选删除完成后所有图片都不显示了。删除后的重扫退回单层后，
                // 顶层已经没有照片了 → 一张都匹配不上，界面上就是全空（文件其实还在子目录里）。
                // 这里刻意删顶层那张：单层重扫的结果是空列表，断言就能把 bug 抓住。
                let del_idx = async_cx.update(|cx| {
                    task_state
                        .read(cx)
                        .items
                        .iter()
                        .position(|m| m.primary_path.ends_with("top.jpg"))
                        .expect("top.jpg 应在列表里")
                });
                let _ = async_cx.update(|cx| {
                    task_state.update(cx, |s, _| s.select_single(del_idx));
                });
                let _ = async_cx.update(|cx| {
                    engine_ops::delete_selected_to_trash(task_state.clone(), cx);
                });
                let mut after_del: Vec<String> = Vec::new();
                let mut mode_after_del = false;
                for _ in 0..40 {
                    pump(async_cx, 250).await;
                    after_del = items(async_cx);
                    mode_after_del = async_cx.update(|cx| task_state.read(cx).scan_recursive);
                    if after_del == vec!["inner.jpg".to_string()] {
                        break;
                    }
                }
                check!(
                    format!("删除顶层照片后重扫仍显示子目录里那张（{after_del:?}）"),
                    after_del == vec!["inner.jpg".to_string()]
                );
                check!("删除后的重扫没有把视图模式降级成单层", mode_after_del);
                check!("被删的文件确实不在盘上", !smoke_dir.join("top.jpg").exists());

                // ── 5) Ctrl+Z 恢复：撤销后的重扫同样保持递归 ──
                fire(async_cx, handle, Box::new(Undo));
                let mut after_undo: Vec<String> = Vec::new();
                for _ in 0..40 {
                    pump(async_cx, 250).await;
                    after_undo = items(async_cx);
                    if after_undo.len() == 2 {
                        break;
                    }
                }
                check!(
                    format!("Ctrl+Z 后重扫保持递归（{after_undo:?}）"),
                    after_undo.len() == 2 && after_undo.iter().any(|n| n == "top.jpg")
                );

                // ── 6) 设置页关掉「包含递归子目录」→ 按新值重扫（模拟开关动作）──
                let _ = async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| {
                        state.app_config.include_subdirectories = false;
                        cx.notify();
                    });
                });
                let _ = async_cx.update(|cx| {
                    engine_ops::start_scan(task_state.clone(), smoke_dir.clone(), false, cx);
                });
                pump(async_cx, 2500).await;
                let single_again = items(async_cx);
                check!(
                    format!("按新值重扫回到单层：{single_again:?}"),
                    single_again == vec!["top.jpg".to_string()]
                );

                // ── 7) 预览装载顶层那张（准备验证失踪后的清理）──
                let _ = async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| {
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

                // ── 8) 外部删掉 top.jpg → 重扫：列表剔除 + 预览槽位清空 ──
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
