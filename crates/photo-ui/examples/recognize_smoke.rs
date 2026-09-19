//! 手动冒烟：批量识别的进度上报与取消（无头，自建素材，不碰真实配置）。
//!
//! 跑法：
//!   XDG_CONFIG_HOME=/tmp/pt-recognize-config xvfb-run -a \
//!     cargo run -p photo-ui --example recognize_smoke
//! 全部通过退出码 0；失败退出码 1；缺 models/ 或 data/bird_catalog.db 时跳过（退出码 0）。
//!
//! 背景：识别是 CPU 密集的同步推理，此前直接写在 `cx.spawn` 的循环里且循环内没有任何
//! `.await`——GPUI 前台执行器（渲染 + 事件循环）被整批识别占住，状态栏的「识别中 n/m」
//! 要等全部跑完才闪一下（用户报的「全部识别没有进度」），「取消识别」也点不动。
//! 本冒烟把三件事钉住：
//!
//!   1) 识别途中能观察到 0 < done < total —— 证明前台还在跑（旧实现下本项必挂）
//!   2) 跑完后 done == total，且每张照片都拿到了识别状态
//!   3) 置上取消标志后识别在数秒内停下（is_recognizing 复位）

use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::Duration;

use gpui_kit::{AppContext as _, Bounds, Point, WindowBounds, WindowOptions, px, size};

use photo_ui::actions::{ReRecognizeAll, RecognizeAllUnrecognized, Rescan};
use photo_ui::state::AppState;

const PHOTOS: usize = 6;

/// 在窗口上派发一个动作（与视图里 `window.dispatch_action(...)` 同一条路径）。
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

/// 1600x1200 的基线 JPEG（带色阶，便于人眼分辨；尺寸够大让单张推理不至于瞬时完成）
fn write_jpeg(path: &Path, rgb: (u8, u8, u8)) {
    let (r, g, b) = rgb;
    let img = image::RgbImage::from_fn(1600, 1200, |x, y| {
        image::Rgb([
            r.saturating_add((x % 70) as u8),
            g.saturating_add((y % 70) as u8),
            b,
        ])
    });
    img.save(path).expect("写测试 JPEG 失败");
}

fn main() {
    // 无头冒烟固定走 Xvfb 的 X11 后端：Wayland 会话下窗口会落到真实桌面，渲染帧不可控
    photo_ui::app::prepare_headless_smoke();
    // 模型/名录库定位：photo-ui 的 data_root() 会从 CARGO_MANIFEST_DIR 向上找，
    // 这里按同一条约定先探一下，缺模型就跳过（真机识别冒烟本来就是可选的）。
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let has_models = std::env::var("PHOTO_DATA_DIR").is_ok()
        || (repo_root.join("models/detect.onnx").exists()
            && repo_root.join("data/bird_catalog.db").exists());
    if !has_models {
        eprintln!("跳过：未找到 models/ 与 data/bird_catalog.db（设 PHOTO_DATA_DIR 或从仓库根跑）");
        return;
    }

    let dir = std::env::temp_dir().join("pt_recognize_smoke");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建素材目录失败");
    let palette = [
        (200u8, 40u8, 40u8),
        (40, 200, 40),
        (40, 40, 200),
        (200, 200, 40),
        (200, 40, 200),
        (40, 200, 200),
    ];
    for (i, rgb) in palette.iter().enumerate() {
        write_jpeg(&dir.join(format!("rec_{i}.jpg")), *rgb);
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
                macro_rules! read_state {
                    ($f:expr) => {
                        async_cx.update(|cx| $f(task_state.read(cx)))
                    };
                }

                pump(async_cx, 300).await;
                fire(async_cx, handle, Box::new(Rescan));
                pump(async_cx, 3000).await;

                let count = read_state!(|s: &AppState| s.items.len());
                check!(format!("扫描到 {PHOTOS} 张素材"), count == PHOTOS);
                if count != PHOTOS {
                    eprintln!("FAIL: 素材不足，后续检查无意义");
                    std::process::exit(1);
                }

                // ── 1) 全部识别：采样看能不能在途中抓到中间进度 ──
                fire(async_cx, handle, Box::new(RecognizeAllUnrecognized));
                let mut saw_partial = false;
                let mut started = false;
                let mut last_done = 0u32;
                let mut last_total = 0u32;
                for _ in 0..800 {
                    // 最多 40s
                    pump(async_cx, 50).await;
                    let (recognizing, done, total) = read_state!(|s: &AppState| {
                        (s.is_recognizing, s.recognize_done, s.recognize_total)
                    });
                    last_done = done;
                    last_total = total;
                    if recognizing {
                        started = true;
                        if done > 0 && done < total {
                            saw_partial = true;
                        }
                    } else if started {
                        break;
                    }
                }

                check!(
                    "识别途中能观察到中间进度（前台没被推理占住）",
                    saw_partial
                );
                check!(
                    format!("识别跑完全部（{last_done}/{last_total}）"),
                    last_done == last_total && last_total as usize == PHOTOS
                );
                let unresolved = read_state!(|s: &AppState| {
                    s.items
                        .iter()
                        .filter(|m| m.recognition_status.is_none())
                        .count()
                });
                check!("每张都拿到了识别状态", unresolved == 0);

                // ── 2) 取消：强制重跑一批，250ms 后置标志，必须很快停下 ──
                fire(async_cx, handle, Box::new(ReRecognizeAll));
                pump(async_cx, 250).await;
                let _ = async_cx.update(|cx| {
                    task_state.update(cx, |s, _cx| {
                        s.recognize_cancel.store(true, Ordering::Relaxed);
                    });
                });
                let mut stopped = false;
                for _ in 0..100 {
                    // 最多 5s
                    pump(async_cx, 50).await;
                    if !read_state!(|s: &AppState| s.is_recognizing) {
                        stopped = true;
                        break;
                    }
                }
                check!("置取消标志后识别在 5s 内停下", stopped);

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
