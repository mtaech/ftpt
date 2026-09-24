//! 手动冒烟：RAW 调整走 **16-bit 母版**（ADR 0007 的落地点）——需要真实 RAW 素材。
//!
//! 跑法（无头，隔离配置，不碰真实配置）：
//!   XDG_CONFIG_HOME=/tmp/pt-raw-adjust-config xvfb-run -a \
//!     cargo run -p photo-ui --example raw_adjust_smoke -- <RAW 文件或含 RAW 的目录>
//! 参数指向的**第一张 RAW 会被复制到自己的临时目录**再扫描：往用户目录写 .pt/ 缓存
//! 不只是不礼貌——上次运行留下的调整参数会让「初始未就绪」「调整生效」这类断言失去意义
//! （实测踩过：debug 跑过一次后，release 复跑时素材已带 +1.75 EV）。
//! 没给参数时打印跳过说明并退出 0；全部通过退出码 0；失败 1。
//!
//! 为什么单开一个冒烟：adjust_smoke 用自建 JPEG，走不到 RAW 分支；而这一批的全部价值
//! 就在「RAW + 有调整时用 16-bit 母版」这条路上。仓库里没有 RAW 素材，所以它按目录参数运行。
//!
//! 覆盖：
//!   1) 首帧不卡：调整帧先出来（8-bit 兜底），不因为 16-bit 解码而白屏
//!   2) 16-bit 母版后台就绪，同一帧自动升级（adjust_preview_16bit == true）
//!   3) 升级后再改参数直接命中 16-bit 母版（用时明显短于首次解码）
//!   4) 调整真的生效（调整帧像素 != 中性母版像素）

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use gpui_kit::{AppContext as _, Bounds, Point, WindowBounds, WindowOptions, px, size};

use photo_ui::actions::Rescan;
use photo_ui::image::source_file_of;
use photo_ui::model::AdjustField;
use photo_ui::state::{AppState, ViewMode};

/// 在窗口上派发一个动作
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

/// 推进若干毫秒（渲染帧在后台执行器的节拍上跑）
async fn pump(async_cx: &mut gpui_kit::AsyncApp, ms: u64) {
    async_cx
        .background_executor()
        .timer(Duration::from_millis(ms))
        .await;
}

/// 轮询直到条件成立或超时；返回是否成立
async fn wait_until<F>(async_cx: &mut gpui_kit::AsyncApp, timeout_ms: u64, mut pred: F) -> bool
where
    F: FnMut(&mut gpui_kit::AsyncApp) -> bool,
{
    let started = Instant::now();
    loop {
        if pred(async_cx) {
            return true;
        }
        if started.elapsed().as_millis() as u64 >= timeout_ms {
            return false;
        }
        pump(async_cx, 50).await;
    }
}

/// 是否是 RAW 扩展名（与扫描器的判定同源：photo_domain::ImageFormat）
fn is_raw_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .and_then(photo_domain::ImageFormat::from_extension)
        .is_some_and(|f| matches!(f, photo_domain::ImageFormat::Raw(_)))
}

fn main() {
    photo_ui::app::prepare_headless_smoke();

    let Some(arg) = std::env::args().nth(1) else {
        println!("跳过：未提供 RAW 文件或含 RAW 的目录（用法见文件头）");
        std::process::exit(0);
    };
    let arg_path = PathBuf::from(arg);
    let src_raw = if arg_path.is_dir() {
        std::fs::read_dir(&arg_path).ok().and_then(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .find(|p| is_raw_file(p))
        })
    } else if arg_path.is_file() {
        Some(arg_path.clone())
    } else {
        None
    };
    let Some(src_raw) = src_raw else {
        eprintln!("FAIL: 没找到 RAW 素材：{}", arg_path.display());
        std::process::exit(1);
    };

    // 复制进自己的临时目录：不往用户目录写缓存，也保证每次运行的前置状态一致
    let dir = std::env::temp_dir().join(format!("pt_raw_adjust_smoke_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建临时素材目录失败");
    let file_name = src_raw.file_name().expect("RAW 文件名").to_os_string();
    std::fs::copy(&src_raw, dir.join(&file_name)).expect("复制 RAW 素材失败");
    println!("素材：{} → {}", src_raw.display(), dir.display());

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
                check!("扫描到 RAW 素材", count >= 1);
                if count == 0 {
                    eprintln!("FAIL: 扫描没有结果，后续检查无意义");
                    std::process::exit(1);
                }

                // 选中第 0 张并进预览（预览渲染负责装载调整参数）
                let _ = async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| {
                        state.view_mode = ViewMode::Preview;
                        state.selected_indices = vec![0];
                        state.anchor_index = Some(0);
                        cx.notify();
                    });
                });
                pump(async_cx, 2000).await;

                let path = async_cx.update(|cx| {
                    task_state
                        .read(cx)
                        .primary_selected_meta()
                        .map(|m| m.primary_path.clone())
                });
                check!(
                    "调整参数已装载且指向焦点图",
                    async_cx.update(|cx| task_state.read(cx).adjust_path.clone()) == path
                );

                let is_raw = async_cx.update(|cx| {
                    task_state
                        .read(cx)
                        .primary_selected_meta()
                        .and_then(source_file_of)
                        .is_some_and(|s| matches!(s.format, photo_domain::ImageFormat::Raw(_)))
                });
                check!("焦点图确实是 RAW", is_raw);
                if !is_raw {
                    eprintln!("FAIL: 素材不是 RAW，本冒烟无意义");
                    std::process::exit(1);
                }

                let source = async_cx.update(|cx| {
                    task_state
                        .read(cx)
                        .primary_selected_meta()
                        .and_then(source_file_of)
                });
                check!(
                    "16-bit 母版初始未就绪（懒解码）",
                    !async_cx.update(|cx| {
                        let s = task_state.read(cx);
                        source
                            .as_ref()
                            .is_some_and(|src| s.image_manager.has_raw16_master(src))
                    })
                );

                let neutral_bytes = async_cx.update(|cx| {
                    task_state
                        .read(cx)
                        .preview_image
                        .as_ref()
                        .map(|(_, img)| img.bytes.clone())
                });
                check!("中性母版已就绪", neutral_bytes.is_some());

                // 滑杆实体（与 adjust_smoke 同一入口）
                let _ = async_cx.update(|cx| {
                    let _ = handle.update(cx, |_view, window, cx| {
                        task_state.update(cx, |state, cx| state.ensure_adjust_sliders(window, cx));
                    });
                });

                // +1.5 EV：首帧应尽快出来（8-bit 兜底），16-bit 母版随后后台升级
                let t0 = Instant::now();
                let _ = async_cx.update(|cx| {
                    let _ = handle.update(cx, |_view, window, cx| {
                        task_state.update(cx, |state, cx| {
                            state.nudge_adjust(AdjustField::Exposure, 1.5, window, cx);
                        });
                    });
                });
                let first_frame = wait_until(async_cx, 10000, |cx| {
                    cx.update(|cx| task_state.read(cx).adjust_preview_toned)
                })
                .await;
                let first_ms = t0.elapsed().as_millis();
                check!(format!("调整帧已出（首帧 {first_ms}ms）"), first_frame);
                let first_16bit = async_cx.update(|cx| task_state.read(cx).adjust_preview_16bit);
                println!(
                    "diag: 首帧 {}（16-bit: {first_16bit}）",
                    if first_16bit { "16-bit" } else { "8-bit 兜底" }
                );

                // 16-bit 升级：后台解码完成后自动重出同一帧
                let upgraded = wait_until(async_cx, 30000, |cx| {
                    cx.update(|cx| task_state.read(cx).adjust_preview_16bit)
                })
                .await;
                let upgrade_ms = t0.elapsed().as_millis();
                check!(
                    format!("调整帧升级为 16-bit（累计 {upgrade_ms}ms）"),
                    upgraded
                );
                check!(
                    "16-bit 母版已进缓存",
                    async_cx.update(|cx| {
                        let s = task_state.read(cx);
                        source
                            .as_ref()
                            .is_some_and(|src| s.image_manager.has_raw16_master(src))
                    })
                );

                // 调整真的生效：升级后的像素必须与中性母版不同
                let toned_bytes = async_cx.update(|cx| {
                    task_state
                        .read(cx)
                        .preview_image
                        .as_ref()
                        .map(|(_, img)| img.bytes.clone())
                });
                check!(
                    "调整生效（16-bit 帧 != 中性母版）",
                    toned_bytes
                        .as_ref()
                        .zip(neutral_bytes.as_ref())
                        .is_some_and(|(a, b)| a != b)
                );

                // 第二次改参数：16-bit 母版已在内存，应明显快于首次（不含解码）
                let t1 = Instant::now();
                let before = async_cx.update(|cx| {
                    task_state
                        .read(cx)
                        .preview_image
                        .as_ref()
                        .map(|(_, img)| img.bytes.clone())
                });
                let _ = async_cx.update(|cx| {
                    let _ = handle.update(cx, |_view, window, cx| {
                        task_state.update(cx, |state, cx| {
                            state.nudge_adjust(AdjustField::Exposure, 0.3, window, cx);
                        });
                    });
                });
                let changed = wait_until(async_cx, 10000, |cx| {
                    cx.update(|cx| {
                        let s = task_state.read(cx);
                        s.adjust_preview_16bit
                            && s.preview_image
                                .as_ref()
                                .map(|(_, img)| img.bytes.clone())
                                .zip(before.as_ref())
                                .is_some_and(|(a, b)| &a != b)
                    })
                })
                .await;
                let second_ms = t1.elapsed().as_millis();
                check!(
                    format!("再改参数仍走 16-bit 且无需重新解码（{second_ms}ms）"),
                    changed && second_ms < upgrade_ms.max(1)
                );
                println!(
                    "diag: 首帧 {first_ms}ms / 首次升级 {upgrade_ms}ms / 复用 16-bit {second_ms}ms"
                );

                // ── 3.4 全分辨率重算：切到 1:1（显示尺寸远超 1600 链路）→ 停手后异步重算 ──
                let t2 = Instant::now();
                let _ = async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| {
                        state.preview_zoom = 0.0;
                        cx.notify();
                    });
                });
                let full_ok = wait_until(async_cx, 90000, |cx| {
                    cx.update(|cx| task_state.read(cx).preview_full_toned_params.is_some())
                })
                .await;
                let full_ms = t2.elapsed().as_millis();
                check!(
                    format!("1:1 全分辨率调整帧就绪（{full_ms}ms，含全尺寸解码 + tone + 编码）"),
                    full_ok
                );
                let dims = async_cx.update(|cx| {
                    task_state
                        .read(cx)
                        .preview_full
                        .as_ref()
                        .and_then(|(_, img)| image::load_from_memory(&img.bytes).ok())
                        .map(|d| {
                            use image::GenericImageView as _;
                            d.dimensions()
                        })
                });
                check!(
                    format!("全分辨率帧尺寸远超 1600 链路（{dims:?}）"),
                    dims.is_some_and(|(w, h)| w.max(h) > 2600)
                );

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
