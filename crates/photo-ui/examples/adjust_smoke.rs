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
//!   2) 滑杆事件（真实订阅路径）→ 参数按 0.3 量化（2026-09-24 起的步进）
//!   3) 调整预览被烘焙成新图，且比原图更亮（+0.9 EV = 三档）
//!   4) 350ms 去抖后参数写进 folder_db（原文件字节不变）
//!   5) 重置 → 预览回到未调整母版、滑杆值同步回 0、库里也复位

use std::path::Path;
use std::time::Duration;

use gpui_kit::component::slider::{SliderEvent, SliderValue};
use gpui_kit::{AppContext as _, Bounds, Point, WindowBounds, WindowOptions, px, size};

use photo_ui::actions::{AdjustExposureUp, AdjustExposureUpCoarse, Rescan, ToggleClipping};
use photo_ui::model::adjust::AdjustField;
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

/// 生成一张 1200×800 的中灰渐变 JPEG（亮度可量化，+0.9 EV 的字节差异肉眼可判）
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

/// JPEG 字节 → 最暗像素的亮度：阴影调整的直接判据（均值会被本来就亮的像素稀释）
fn min_luma(bytes: &[u8]) -> f64 {
    let img = image::load_from_memory(bytes)
        .expect("解码预览 JPEG 失败")
        .to_rgb8();
    img.pixels()
        .map(|p| 0.299 * p[0] as f64 + 0.587 * p[1] as f64 + 0.114 * p[2] as f64)
        .fold(f64::MAX, f64::min)
}

/// JPEG 字节 → 指定通道均值（0=R/1=G/2=B），用于色温方向断言
fn mean_channel(bytes: &[u8], ch: usize) -> f64 {
    let img = image::load_from_memory(bytes).expect("解码预览 JPEG 失败").to_rgb8();
    let sum: u64 = img.pixels().map(|p| p[ch] as u64).sum();
    sum as f64 / (img.width() as f64 * img.height() as f64)
}

/// 从 folder_db 读某张图当前的曝光值（None = 无行 / 路径算不出）
fn stored_exposure(
    async_cx: &mut gpui_kit::AsyncApp,
    state: &gpui_kit::Entity<AppState>,
    idx: usize,
) -> Option<f32> {
    async_cx.update(|cx| {
        let s = state.read(cx);
        let db = s.folder_db.as_ref()?;
        let dir = s.current_dir.as_ref()?;
        let meta = s.items.get(idx)?;
        let rel = photo_ui::model::rel_path_of(dir, Path::new(&meta.primary_path))?;
        db.get_adjustments(&rel).ok().flatten().map(|p| p.exposure)
    })
}

fn main() {
    // 无头冒烟固定走 Xvfb 的 X11 后端：Wayland 会话下窗口会落到真实桌面，渲染帧不可控
    photo_ui::app::prepare_headless_smoke();
    let dir = std::env::temp_dir().join("pt_adjust_smoke");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建素材目录失败");
    let photo = dir.join("adjust.jpg");
    write_jpeg(&photo);
    // 批量套用 / 粘贴需要多张素材（内容是同一张渐变，够断言参数落库即可）
    write_jpeg(&dir.join("adjust_b.jpg"));
    write_jpeg(&dir.join("adjust_c.jpg"));
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

                // ── 直方图（§7.4）：装载母版时同一帧算好；无调整时当前 = 基线 ──
                let baseline_hist =
                    async_cx.update(|cx| task_state.read(cx).preview_histogram.clone());
                check!(
                    "直方图已就绪且非空",
                    baseline_hist.as_ref().is_some_and(|h| h.total_pixels() > 0)
                );
                check!(
                    "无调整时当前直方图 = 基线直方图",
                    async_cx.update(|cx| {
                        let s = task_state.read(cx);
                        s.preview_histogram.is_some()
                            && s.preview_histogram == s.preview_base_histogram
                    })
                );
                let baseline_clip_high = baseline_hist
                    .as_ref()
                    .map(|h| h.clip_high_count)
                    .unwrap_or(0);
                check!(
                    format!("素材本身无高光溢出（clip_high={baseline_clip_high}）"),
                    baseline_clip_high == 0
                );

                // 调整 tab 首次渲染才懒创建滑杆；这里直接建好，再用**真实订阅路径**驱动一次
                let _ = async_cx.update(|cx| {
                    let _ = handle.update(cx, |_view, window, cx| {
                        task_state.update(cx, |state, cx| state.ensure_adjust_sliders(window, cx));
                    });
                });
                check!(
                    "七条滑杆实体已就绪（与 AdjustField::ALL 同序）",
                    async_cx.update(|cx| {
                        task_state
                            .read(cx)
                            .adjust_sliders
                            .as_ref()
                            .map(|s| s.all_entities().len())
                            == Some(AdjustField::ALL.len())
                    })
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

                // 滑杆 → 参数：0.37 应量化到 0.3（0.3 EV 步进）
                let _ = async_cx.update(|cx| {
                    let slider = task_state
                        .read(cx)
                        .adjust_sliders
                        .as_ref()
                        .map(|s| s.all_entities()[AdjustField::Exposure.index()].clone());
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
                    (exposure - 0.3).abs() < 1e-6
                );

                // 拉一档（滑杆 1.0 会量化成 +0.9 EV）：预览应被重新烘焙且更亮
                let _ = async_cx.update(|cx| {
                    let slider = task_state
                        .read(cx)
                        .adjust_sliders
                        .as_ref()
                        .map(|s| s.all_entities()[AdjustField::Exposure.index()].clone());
                    if let Some(slider) = slider {
                        slider.update(cx, |_slider, cx| {
                            cx.emit(SliderEvent::Change(SliderValue::Single(1.0))); // → 0.9 EV
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
                            format!("+0.9 EV 后更亮（{baseline_luma:.1} → {luma:.1}）"),
                            luma > baseline_luma + 20.0
                        );
                    }
                    None => check!("调整预览已生成", false),
                }

                // 直方图必须跟着调整走：+0.9 EV 把亮部推到高光端，clip_high 应从 0 变正
                let toned_hist = async_cx.update(|cx| task_state.read(cx).preview_histogram.clone());
                let toned_clip_high = toned_hist
                    .as_ref()
                    .map(|h| h.clip_high_count)
                    .unwrap_or(0);
                check!(
                    format!("+0.9 EV 后直方图高光溢出变多（{baseline_clip_high} → {toned_clip_high}）"),
                    toned_clip_high > baseline_clip_high
                );
                check!(
                    "基线直方图不随调整变化（原图口径）",
                    async_cx.update(|cx| {
                        task_state
                            .read(cx)
                            .preview_base_histogram
                            .as_ref()
                            .map(|h| h.clip_high_count)
                            == Some(baseline_clip_high)
                    })
                );

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
                    "调整参数已落库（+0.9 EV）",
                    stored.is_some_and(|p| (p.exposure - 0.9).abs() < 1e-6)
                );
                check!(
                    "网格「已调整」标识立即点亮（不等重扫）",
                    async_cx.update(|cx| {
                        task_state
                            .read(cx)
                            .items
                            .iter()
                            .find(|m| Some(m.primary_path.as_str()) == path.as_deref())
                            .is_some_and(|m| m.has_adjustments)
                    })
                );
                check!(
                    "原文件从未被改写",
                    std::fs::read(&photo).unwrap_or_default() == original_bytes
                );

                // ── 键盘微调（[ / ] 与 Shift 粗调）：走真实动作派发路径 ──
                let before_nudge = async_cx.update(|cx| task_state.read(cx).adjust.exposure);
                fire(async_cx, handle, Box::new(AdjustExposureUp));
                pump(async_cx, 400).await;
                let after_nudge = async_cx.update(|cx| task_state.read(cx).adjust.exposure);
                check!(
                    format!("] 微调 +0.3 EV（{before_nudge} → {after_nudge}）"),
                    (after_nudge - before_nudge - 0.3).abs() < 1e-6
                );
                fire(async_cx, handle, Box::new(AdjustExposureUpCoarse));
                pump(async_cx, 400).await;
                let after_coarse = async_cx.update(|cx| task_state.read(cx).adjust.exposure);
                check!(
                    format!("Shift+] 粗调 +0.9 EV（{after_nudge} → {after_coarse}）"),
                    (after_coarse - after_nudge - 0.9).abs() < 1e-6
                );
                check!(
                    "微调同步回滑杆（不出现参数与滑杆不一致）",
                    async_cx.update(|cx| {
                        task_state
                            .read(cx)
                            .adjust_sliders
                            .as_ref()
                            .map(|s| s.all_entities()[AdjustField::Exposure.index()].read(cx).value().start())
                            == Some(after_coarse)
                    })
                );

                // ── 剪切警告（O 键）：默认关；打开后掩码按当前参数烘焙，与主图同尺寸 ──
                check!(
                    "默认没有剪切掩码",
                    async_cx.update(|cx| task_state.read(cx).preview_clip_mask.is_none())
                );
                fire(async_cx, handle, Box::new(ToggleClipping));
                pump(async_cx, 2500).await;
                let mask_bytes = async_cx.update(|cx| {
                    task_state
                        .read(cx)
                        .preview_clip_mask
                        .as_ref()
                        .map(|(_, img)| img.bytes.clone())
                });
                match mask_bytes {
                    Some(bytes) => {
                        let mask = image::load_from_memory(&bytes)
                            .expect("解码剪切掩码失败")
                            .to_rgba8();
                        let master = image::load_from_memory(&baseline_bytes)
                            .expect("解码母版失败")
                            .to_rgba8();
                        check!(
                            "剪切掩码与主图同尺寸",
                            mask.dimensions() == master.dimensions()
                        );
                        let red = mask
                            .pixels()
                            .filter(|p| p.0 == [255, 0, 0, 255])
                            .count();
                        let blue = mask
                            .pixels()
                            .filter(|p| p.0 == [0, 0, 255, 255])
                            .count();
                        // 此时曝光已被微调推到 +2.1 EV：红色（高光）必须出现；素材没有死黑区
                        check!(format!("+2.1 EV 后高光掩码非空（红 {red} 像素）"), red > 0);
                        check!(format!("素材无死黑（蓝 {blue} 像素）"), blue == 0);
                    }
                    None => check!("O 键后剪切掩码已生成", false),
                }
                // 目检钩子：保持「+2.1 EV + 剪切叠加」这一帧供外部截图（PHOTO_SMOKE_HOLD_MS）
                if let Ok(ms) = std::env::var("PHOTO_SMOKE_HOLD_MS")
                    && let Ok(ms) = ms.parse::<u64>()
                {
                    println!("保持窗口 {ms}ms 供截图目检");
                    pump(async_cx, ms).await;
                }

                fire(async_cx, handle, Box::new(ToggleClipping));
                pump(async_cx, 400).await;
                check!(
                    "再按 O 掩码被撤掉",
                    async_cx.update(|cx| task_state.read(cx).preview_clip_mask.is_none())
                );

                // ── before/after（按住反斜杠看原图）：未调整母版进独立槽位 ──
                let _ = async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| state.set_before_after(true, cx));
                });
                pump(async_cx, 2000).await;
                check!(
                    "按住后未调整母版已就绪（独立槽位）",
                    async_cx.update(|cx| {
                        let s = task_state.read(cx);
                        s.preview_base_image
                            .as_ref()
                            .is_some_and(|(p, _)| Some(p.as_str()) == s.adjust_path.as_deref())
                    })
                );
                let held_luma = async_cx.update(|cx| {
                    task_state
                        .read(cx)
                        .preview_base_image
                        .as_ref()
                        .map(|(_, img)| mean_luma(&img.bytes))
                });
                check!(
                    format!("before/after 用的是原图（{held_luma:?} ≈ {baseline_luma:.1}）"),
                    held_luma.is_some_and(|l| (l - baseline_luma).abs() < 2.0)
                );
                check!(
                    "调整帧仍留在 preview_image（两槽位互不覆盖）",
                    async_cx.update(|cx| {
                        task_state
                            .read(cx)
                            .preview_image
                            .as_ref()
                            .map(|(_, img)| mean_luma(&img.bytes))
                            .is_some_and(|l| l > baseline_luma + 20.0)
                    })
                );
                // 第二个目检钩子：保持「按住看原图」这一帧（与第一帧对比画面明暗）
                if let Ok(ms) = std::env::var("PHOTO_SMOKE_HOLD_MS")
                    && let Ok(ms) = ms.parse::<u64>()
                {
                    println!("保持 before/after 帧 {ms}ms 供截图对比");
                    pump(async_cx, ms).await;
                }

                let _ = async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| state.set_before_after(false, cx));
                });
                pump(async_cx, 200).await;
                check!(
                    "松开后标志复位",
                    !async_cx.update(|cx| task_state.read(cx).before_after_held)
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
                        .map(|s| s.all_entities()[AdjustField::Exposure.index()].read(cx).value().start())
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
                check!(
                    "重置后「已调整」标识熄灭",
                    async_cx.update(|cx| {
                        task_state
                            .read(cx)
                            .items
                            .iter()
                            .find(|m| Some(m.primary_path.as_str()) == path.as_deref())
                            .is_some_and(|m| !m.has_adjustments)
                    })
                );

                // ── 重扫回填路径：写一笔 → 重扫 → 标识从库里带回来（不是内存残留） ──
                fire(async_cx, handle, Box::new(AdjustExposureUpCoarse));
                pump(async_cx, 800).await;
                fire(async_cx, handle, Box::new(Rescan));
                pump(async_cx, 3000).await;
                check!(
                    "重扫后从 folder_db 回填「已调整」",
                    async_cx.update(|cx| {
                        task_state
                            .read(cx)
                            .items
                            .iter()
                            .find(|m| Some(m.primary_path.as_str()) == path.as_deref())
                            .is_some_and(|m| m.has_adjustments)
                    })
                );

                // 第三个目检钩子：网格视图下的「已调整」徽标（写一笔参数后保持一帧）
                let _ = async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| {
                        state.view_mode = ViewMode::Grid;
                        cx.notify();
                    });
                });
                pump(async_cx, 800).await;
                if let Ok(ms) = std::env::var("PHOTO_SMOKE_HOLD_MS")
                    && let Ok(ms) = ms.parse::<u64>()
                {
                    println!("保持网格帧 {ms}ms 供截图目检");
                    pump(async_cx, ms).await;
                }

                // ── 3.13 新参数：预览像素真的跟着变（不只是参数落库）──
                // 先整体清成中性：上面「重扫回填」那段会把曝光留在 +0.9 EV，
                // 基线必须是原图，否则阴影/高光的幅度判据会被既有的提亮吃掉
                let _ = async_cx.update(|cx| {
                    let _ = handle.update(cx, |_view, window, cx| {
                        task_state.update(cx, |state, cx| {
                            state.reset_all_adjustments(window, cx);
                        });
                    });
                });
                pump(async_cx, 1500).await;

                let neutral_bytes_for_params = async_cx.update(|cx| {
                    task_state
                        .read(cx)
                        .preview_image
                        .as_ref()
                        .map(|(_, img)| img.bytes.clone())
                });
                let neutral_luma = neutral_bytes_for_params
                    .as_ref()
                    .map(|b| mean_luma(b))
                    .unwrap_or(0.0);
                let neutral_min = neutral_bytes_for_params
                    .as_ref()
                    .map(|b| min_luma(b))
                    .unwrap_or(0.0);
                let neutral_r = neutral_bytes_for_params
                    .as_ref()
                    .map(|b| mean_channel(b, 0))
                    .unwrap_or(0.0);
                let neutral_b = neutral_bytes_for_params
                    .as_ref()
                    .map(|b| mean_channel(b, 2))
                    .unwrap_or(0.0);

                // 阴影 +100 → 提亮
                let _ = async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| {
                        state.set_adjust_field(AdjustField::Shadows, 100.0, cx);
                        cx.notify();
                    });
                });
                pump(async_cx, 1200).await;
                let shadows_probe = async_cx.update(|cx| {
                    task_state
                        .read(cx)
                        .preview_image
                        .as_ref()
                        .map(|(_, img)| (mean_luma(&img.bytes), min_luma(&img.bytes)))
                });
                let (shadows_luma, shadows_min) = shadows_probe.unwrap_or((0.0, 0.0));
                check!(
                    format!(
                        "阴影 +100 提亮暗部（最暗 {neutral_min:.1}→{shadows_min:.1}，均值 {neutral_luma:.1}→{shadows_luma:.1}）"
                    ),
                    // 均值会被本来就亮的像素稀释（素材偏亮），所以以**最暗像素**为判据
                    shadows_min > neutral_min + 20.0
                );

                // 高光 -100（阴影归零）→ 压暗
                let _ = async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| {
                        state.set_adjust_field(AdjustField::Shadows, 0.0, cx);
                        state.set_adjust_field(AdjustField::Highlights, -100.0, cx);
                        cx.notify();
                    });
                });
                pump(async_cx, 1200).await;
                let highlights_luma = async_cx.update(|cx| {
                    task_state
                        .read(cx)
                        .preview_image
                        .as_ref()
                        .map(|(_, img)| mean_luma(&img.bytes))
                });
                check!(
                    format!("高光 -100 回收亮部（{neutral_luma:.1} → {highlights_luma:?}）"),
                    highlights_luma.is_some_and(|l| l < neutral_luma - 2.0)
                );

                // 色温 +100（高光归零）→ R 抬 B 压
                let _ = async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| {
                        state.set_adjust_field(AdjustField::Highlights, 0.0, cx);
                        state.set_adjust_field(AdjustField::Temperature, 100.0, cx);
                        cx.notify();
                    });
                });
                pump(async_cx, 1200).await;
                let warm = async_cx.update(|cx| {
                    task_state
                        .read(cx)
                        .preview_image
                        .as_ref()
                        .map(|(_, img)| (mean_channel(&img.bytes, 0), mean_channel(&img.bytes, 2)))
                });
                let (warm_r, warm_b) = warm.unwrap_or((0.0, 0.0));
                check!(
                    format!(
                        "色温 +100 让预览偏暖（R {neutral_r:.1}→{warm_r:.1} / B {neutral_b:.1}→{warm_b:.1}）"
                    ),
                    warm_r > warm_b
                );

                // 单张滑杆改动的撤销（3.11 的另一半）：
                // 先把色温落回 0 并等落盘（否则这次的"改前值"还是上一段的 +100），
                // 再改成 -100，Ctrl+Z 应回到 0
                let _ = async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| {
                        state.set_adjust_field(AdjustField::Temperature, 0.0, cx);
                        cx.notify();
                    });
                });
                pump(async_cx, 900).await;
                let _ = async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| {
                        state.set_adjust_field(AdjustField::Temperature, -100.0, cx);
                        cx.notify();
                    });
                });
                pump(async_cx, 800).await;
                let temp_before_undo = async_cx.update(|cx| {
                    let s = task_state.read(cx);
                    let idx = s.primary_selected_index()?;
                    let dir = s.current_dir.as_ref()?;
                    let meta = s.items.get(idx)?;
                    let rel = photo_ui::model::rel_path_of(dir, Path::new(&meta.primary_path))?;
                    s.folder_db
                        .as_ref()?
                        .get_adjustments(&rel)
                        .ok()
                        .flatten()
                        .map(|p| p.temperature)
                });
                let undone_single = async_cx.update(|cx| {
                    let outcomes = task_state.update(cx, |state, _cx| state.undo_last_op());
                    outcomes.iter().filter(|o| o.result.is_ok()).count()
                });
                pump(async_cx, 400).await;
                let temp_after_undo = async_cx.update(|cx| {
                    let s = task_state.read(cx);
                    let idx = s.primary_selected_index()?;
                    let dir = s.current_dir.as_ref()?;
                    let meta = s.items.get(idx)?;
                    let rel = photo_ui::model::rel_path_of(dir, Path::new(&meta.primary_path))?;
                    s.folder_db
                        .as_ref()?
                        .get_adjustments(&rel)
                        .ok()
                        .flatten()
                        .map(|p| p.temperature)
                });
                check!(
                    format!(
                        "单张滑杆改动可 Ctrl+Z（色温 {temp_before_undo:?} → {temp_after_undo:?}，成功 {undone_single} 项）"
                    ),
                    temp_before_undo == Some(-100) && temp_after_undo == Some(0)
                );
                let _ = async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| {
                        state.adjust = photo_domain::AdjustParams::default();
                        cx.notify();
                    });
                });
                pump(async_cx, 300).await;

                // 收回中性，后面的批量 / 预设检查从同一基线出发
                let _ = async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| {
                        state.set_adjust_field(AdjustField::Temperature, 0.0, cx);
                        cx.notify();
                    });
                });
                pump(async_cx, 1200).await;

                let total = async_cx.update(|cx| task_state.read(cx).items.len());
                check!(format!("素材是 3 张（{total}）"), total >= 3);

                // 当前这张设成一档（0.9 EV）后套用到选中集
                let _ = async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| {
                        state.set_adjust_field(AdjustField::Exposure, 1.0, cx);
                        state.selected_indices = (0..state.items.len()).collect();
                        cx.notify();
                    });
                });
                pump(async_cx, 600).await;
                let _ = async_cx.update(|cx| {
                    let _ = handle.update(cx, |_view, window, cx| {
                        task_state.update(cx, |state, cx| {
                            state.apply_adjustments_to_selection(window, cx);
                        });
                    });
                });
                pump(async_cx, 500).await;
                let applied: Vec<Option<f32>> = (0..total)
                    .map(|i| stored_exposure(async_cx, &task_state, i))
                    .collect();
                check!(
                    format!("批量套用写进 folder_db（3 张都是 +0.9 EV：{applied:?}）"),
                    applied.iter().all(|e| e.is_some_and(|v| (v - 0.9).abs() < 1e-6))
                );
                check!(
                    "批量套用后三张的「已调整」标识都点亮",
                    async_cx.update(|cx| {
                        task_state.read(cx).items.iter().all(|m| m.has_adjustments)
                    })
                );

                // 复制 → 改当前 → 粘贴回选中集
                let _ = async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| {
                        state.copy_adjustments();
                        state.set_adjust_field(AdjustField::Exposure, 0.0, cx);
                        cx.notify();
                    });
                });
                pump(async_cx, 600).await;
                check!(
                    "复制进进程内剪贴板（+0.9 EV）",
                    async_cx.update(|cx| {
                        task_state
                            .read(cx)
                            .adjust_clipboard
                            .is_some_and(|p| (p.exposure - 0.9).abs() < 1e-6)
                    })
                );
                let _ = async_cx.update(|cx| {
                    let _ = handle.update(cx, |_view, window, cx| {
                        task_state.update(cx, |state, cx| state.paste_adjustments(window, cx));
                    });
                });
                pump(async_cx, 500).await;
                let pasted: Vec<Option<f32>> = (0..total)
                    .map(|i| stored_exposure(async_cx, &task_state, i))
                    .collect();
                check!(
                    format!("粘贴把三张写回 +0.9 EV（{pasted:?}）"),
                    pasted.iter().all(|e| e.is_some_and(|v| (v - 0.9).abs() < 1e-6))
                );

                // 沿用上一张：第 0 张设 0.5（量化成 0.6），选中第 1 张 → 应变成 0.6
                let _ = async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| {
                        state.selected_indices = vec![0];
                        cx.notify();
                    });
                });
                pump(async_cx, 300).await;
                let _ = async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| {
                        state.set_adjust_field(AdjustField::Exposure, 0.5, cx); // → 0.6
                        cx.notify();
                    });
                });
                pump(async_cx, 600).await;
                let _ = async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| {
                        state.select_single(1);
                        cx.notify();
                    });
                });
                pump(async_cx, 800).await;
                let _ = async_cx.update(|cx| {
                    let _ = handle.update(cx, |_view, window, cx| {
                        task_state.update(cx, |state, cx| {
                            state.reuse_previous_adjustments(window, cx);
                        });
                    });
                });
                pump(async_cx, 500).await;
                let reused = stored_exposure(async_cx, &task_state, 1);
                check!(
                    format!("沿用上一张（第 1 张 → 0.6 EV：{reused:?}）"),
                    reused.is_some_and(|v| (v - 0.6).abs() < 1e-6)
                );

                // Ctrl+Z：带 folder_db 的撤销把第 1 张写回沿用之前的 +1.0
                let undone = async_cx.update(|cx| {
                    let outcomes = task_state.update(cx, |state, _cx| state.undo_last_op());
                    outcomes.iter().filter(|o| o.result.is_ok()).count()
                });
                pump(async_cx, 400).await;
                let restored = stored_exposure(async_cx, &task_state, 1);
                check!(
                    format!("Ctrl+Z 撤销沿用（成功 {undone} 项；第 1 张回到 0.9：{restored:?}）"),
                    undone >= 1 && restored.is_some_and(|v| (v - 0.9).abs() < 1e-6)
                );

                // ── 调整预设（3.12）：保存 / 套用 / 删除 ──
                let _ = async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| {
                        state.select_single(0);
                        cx.notify();
                    });
                });
                pump(async_cx, 500).await;
                let _ = async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| {
                        state.set_adjust_field(AdjustField::Exposure, 0.85, cx); // → 0.9
                        cx.notify();
                    });
                });
                pump(async_cx, 500).await;
                let _ = async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| state.save_adjust_preset(cx));
                });
                pump(async_cx, 300).await;
                check!(
                    format!(
                        "保存预设（{} 条，曝光 {:?}）",
                        async_cx.update(|cx| task_state.read(cx).app_config.adjust_presets.len()),
                        async_cx.update(|cx| task_state
                            .read(cx)
                            .app_config
                            .adjust_presets
                            .first()
                            .map(|p| p.exposure))
                    ),
                    async_cx.update(|cx| {
                        let s = task_state.read(cx);
                        s.app_config.adjust_presets.len() == 1
                            && s.app_config.adjust_presets[0].exposure == 0.9
                    })
                );

                // 套用到第 2 张：只动它，且 chip 选中态跟上
                let _ = async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| {
                        state.select_single(2);
                        cx.notify();
                    });
                });
                pump(async_cx, 800).await;
                let _ = async_cx.update(|cx| {
                    let _ = handle.update(cx, |_view, window, cx| {
                        task_state.update(cx, |state, cx| {
                            state.apply_adjust_preset(0, window, cx);
                        });
                    });
                });
                pump(async_cx, 500).await;
                let item0_after = stored_exposure(async_cx, &task_state, 0);
                let item1_after = stored_exposure(async_cx, &task_state, 1);
                let item2_after = stored_exposure(async_cx, &task_state, 2);
                check!(
                    format!(
                        "套用预设只动目标张（0:{item0_after:?} / 1:{item1_after:?} 不动 / 2:{item2_after:?}）"
                    ),
                    item2_after.is_some_and(|v| (v - 0.9).abs() < 1e-6)
                        && item1_after.is_some_and(|v| (v - 0.9).abs() < 1e-6)
                );
                check!(
                    "套用后 chip 选中态命中该预设",
                    async_cx.update(|cx| task_state.read(cx).current_adjust_preset()) == Some(0)
                );

                // 删除
                let _ = async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| state.delete_adjust_preset(0, cx));
                });
                pump(async_cx, 300).await;
                check!(
                    "删除预设后列表清空且选中态归零",
                    async_cx.update(|cx| {
                        let s = task_state.read(cx);
                        s.app_config.adjust_presets.is_empty()
                            && s.current_adjust_preset().is_none()
                    })
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
