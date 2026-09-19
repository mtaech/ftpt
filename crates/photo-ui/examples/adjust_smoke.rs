//! 手动冒烟：右栏「调整」tab 真的能改预览、落库、重置（无头，自建素材，不碰真实照片与配置）。
//!
//! 跑法：
//!   XDG_CONFIG_HOME=/tmp/pt-adjust-config xvfb-run -a \
//!     cargo run -p photo-ui --example adjust_smoke
//! 全部通过退出码 0；失败退出码 1。
//!
//! 背景：用户报「调整面板没用」——GPUI 重写时调整 tab 只移植了三根静态灰条
//! （既没有滑杆实体，也没有 folder_db 读写与预览重算）。本冒烟覆盖修复后的完整链路：
//!
//!   1) 预览渲染会装载焦点图的调整参数（无记录 = 中性）
//!   2) 滑杆事件（真实订阅路径）→ 参数按 0.05 量化
//!   3) 调整预览被烘焙成新图，且比原图更亮（+1 EV）
//!   4) 350ms 去抖后参数写进 folder_db（原文件字节不变）
//!   5) 重置 → 预览回到未调整母版、滑杆值同步回 0、库里也复位

use std::path::Path;
use std::time::Duration;

use gpui_kit::component::slider::{SliderEvent, SliderValue};
use gpui_kit::{AppContext as _, Bounds, Point, WindowBounds, WindowOptions, px, size};

use photo_ui::actions::Rescan;
use photo_ui::model::rel_path_of;
use photo_ui::state::{AppState, ViewMode};

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

/// 等若干个渲染帧（滑杆事件经 pending effects 在帧末派发，需要推进帧）。
async fn pump(async_cx: &mut gpui_kit::AsyncApp, ms: u64) {
    async_cx
        .background_executor()
        .timer(Duration::from_millis(ms))
        .await;
}

/// 生成一张 1200×800 的中灰渐变 JPEG（亮度可量化，+1 EV 的字节差异肉眼可判）
fn write_jpeg(path: &Path) {
    let img = image::RgbImage::from_fn(1200, 800, |x, y| {
        let v = 90u8
            .saturating_add((x % 80) as u8)
            .saturating_add((y % 40) as u8);
        image::Rgb([v, v, v])
    });
    img.save(path).expect("写测试 JPEG 失败");
}

/// JPEG 字节 → 平均亮度（0–255），用来判断曝光调整是否真的生效
fn mean_luma(bytes: &[u8]) -> f64 {
    let img = image::load_from_memory(bytes).expect("解码预览 JPEG 失败").to_rgb8();
    let sum: u64 = img.pixels().map(|p| p[0] as u64).sum();
    sum as f64 / (img.width() as f64 * img.height() as f64)
}

fn main() {
    // 无头冒烟固定走 Xvfb 的 X11 后端：Wayland 会话下窗口会落到真实桌面，渲染帧不可控
    photo_ui::app::prepare_headless_smoke();
    let dir = std::env::temp_dir().join("pt_adjust_smoke");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建素材目录失败");
    let photo = dir.join("adjust.jpg");
    write_jpeg(&photo);
    let original_bytes = std::fs::read(&photo).expect("读素材失败");

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

                // 进预览并选中第 0 张：预览渲染负责装载调整参数（与面板同一入口）
                let _ = async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| {
                        state.view_mode = ViewMode::Preview;
                        state.selected_indices = vec![0];
                        state.anchor_index = Some(0);
                        cx.notify();
                    });
                });
                pump(async_cx, 1500).await;

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
                check!(
                    "初始为无调整（库中无记录）",
                    async_cx.update(|cx| task_state.read(cx).adjust.is_neutral())
                );

                let baseline = async_cx.update(|cx| {
                    task_state
                        .read(cx)
                        .preview_image
                        .as_ref()
                        .map(|(_, img)| img.bytes.clone())
                });
                check!("预览母版已就绪", baseline.is_some());
                let baseline_bytes = baseline.unwrap_or_default();
                let baseline_luma = mean_luma(&baseline_bytes);

                // 调整 tab 首次渲染才懒创建滑杆；这里直接建好，再用**真实订阅路径**驱动一次
                let _ = async_cx.update(|cx| {
                    let _ = handle.update(cx, |_view, window, cx| {
                        task_state.update(cx, |state, cx| state.ensure_adjust_sliders(window, cx));
                    });
                });
                check!(
                    "三条滑杆实体已就绪",
                    async_cx.update(|cx| task_state.read(cx).adjust_sliders.is_some())
                );

                // 面板函数本身可无 panic 地渲染一整棵元素树（旧的静态占位没有这些分支）
                let _ = async_cx.update(|cx| {
                    let _ = handle.update(cx, |_view, window, cx| {
                        task_state.update(cx, |state, cx| {
                            let meta = state.primary_selected_meta().cloned();
                            let _el = photo_ui::views::render_adjustments_tab(
                                state,
                                meta.as_ref(),
                                window,
                                cx,
                            );
                        });
                    });
                });
                println!("OK: 调整面板可渲染");

                // 滑杆 → 参数：0.37 应量化到 0.35
                let _ = async_cx.update(|cx| {
                    let slider = task_state
                        .read(cx)
                        .adjust_sliders
                        .as_ref()
                        .map(|s| s.exposure.clone());
                    if let Some(slider) = slider {
                        slider.update(cx, |_slider, cx| {
                            cx.emit(SliderEvent::Change(SliderValue::Single(0.37)));
                        });
                    }
                });
                pump(async_cx, 200).await;
                let exposure = async_cx.update(|cx| task_state.read(cx).adjust.exposure);
                check!(
                    format!("滑杆事件 → 参数（0.37 量化到 {exposure}）"),
                    (exposure - 0.35).abs() < 1e-6
                );

                // 拉 +1 EV：预览应被重新烘焙且更亮
                let _ = async_cx.update(|cx| {
                    let slider = task_state
                        .read(cx)
                        .adjust_sliders
                        .as_ref()
                        .map(|s| s.exposure.clone());
                    if let Some(slider) = slider {
                        slider.update(cx, |_slider, cx| {
                            cx.emit(SliderEvent::Change(SliderValue::Single(1.0)));
                        });
                    }
                });
                pump(async_cx, 2000).await;

                check!(
                    "调整预览已烘焙（toned 标记）",
                    async_cx.update(|cx| task_state.read(cx).adjust_preview_toned)
                );
                let toned = async_cx.update(|cx| {
                    task_state
                        .read(cx)
                        .preview_image
                        .as_ref()
                        .map(|(_, img)| img.bytes.clone())
                });
                match toned {
                    Some(bytes) => {
                        check!("调整预览与原图字节不同", bytes != baseline_bytes);
                        let luma = mean_luma(&bytes);
                        check!(
                            format!("+1 EV 后更亮（{baseline_luma:.1} → {luma:.1}）"),
                            luma > baseline_luma + 20.0
                        );
                    }
                    None => check!("调整预览已生成", false),
                }

                // 落库：350ms 去抖后应写进 folder_db
                let rel = rel_path_of(&dir, Path::new(&photo.to_string_lossy().to_string()))
                    .expect("素材相对路径");
                let stored = async_cx.update(|cx| {
                    task_state
                        .read(cx)
                        .folder_db
                        .as_ref()
                        .and_then(|db| db.get_adjustments(&rel).ok().flatten())
                });
                check!(
                    "调整参数已落库（+1.0 EV）",
                    stored.is_some_and(|p| (p.exposure - 1.0).abs() < 1e-6)
                );
                check!(
                    "原文件从未被改写",
                    std::fs::read(&photo).unwrap_or_default() == original_bytes
                );

                // 全部重置：预览回原图、滑杆同步、库中复位
                let _ = async_cx.update(|cx| {
                    let _ = handle.update(cx, |_view, window, cx| {
                        task_state.update(cx, |state, cx| {
                            state.reset_all_adjustments(window, cx);
                        });
                    });
                });
                pump(async_cx, 2000).await;

                check!(
                    "重置后回到未调整母版",
                    !async_cx.update(|cx| task_state.read(cx).adjust_preview_toned)
                );
                let restored = async_cx.update(|cx| {
                    task_state
                        .read(cx)
                        .preview_image
                        .as_ref()
                        .map(|(_, img)| img.bytes.clone())
                });
                if let Some(bytes) = restored {
                    let luma = mean_luma(&bytes);
                    check!(
                        format!("重置后亮度回到基线（{luma:.1} vs {baseline_luma:.1}）"),
                        (luma - baseline_luma).abs() < 1.0
                    );
                } else {
                    check!("重置后有预览图", false);
                }
                let slider_value = async_cx.update(|cx| {
                    task_state
                        .read(cx)
                        .adjust_sliders
                        .as_ref()
                        .map(|s| s.exposure.read(cx).value().start())
                });
                check!(
                    "滑杆值同步回中性",
                    slider_value.is_some_and(|v| v.abs() < 1e-6)
                );
                let stored_after = async_cx.update(|cx| {
                    task_state
                        .read(cx)
                        .folder_db
                        .as_ref()
                        .and_then(|db| db.get_adjustments(&rel).ok().flatten())
                });
                check!(
                    "重置已落库（参数全零）",
                    stored_after.is_some_and(|p| p.is_neutral())
                );

                if failures == 0 {
                    println!("全部通过");
                    // 检查跑完就退出：GPUI 事件循环不会自己结束，留着会挂住 CI/脚本
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
