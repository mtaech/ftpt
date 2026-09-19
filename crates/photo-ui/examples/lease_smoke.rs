//! 手动冒烟：验证按钮派发的动作真的能到达 AppState 根监听器，且不会双重租借。
//!
//! 背景（两个真实故障）：
//! 1. GPUI 的 `Window::dispatch_action` 从「当前焦点节点」向上冒泡。窗口里没有任何
//!    焦点节点时动作会被直接丢弃 —— 表现就是所有按钮/快捷键失灵。生产接线里
//!    `photo_ui::app::focus_root` 在首帧后把焦点交给根视图。
//! 2. `Context::listener` 运行期间实体已被租借；在回调里再对同一实体
//!    `Entity::update` 会 panic（entity_map 的 double_lease_panic）。
//!
//! 本示例把用户报过的那几个按钮依次走一遍，并对效果做断言。
//!
//! 跑法（无头，用 XDG_CONFIG_HOME 隔离配置，不碰真实配置）：
//!   XDG_CONFIG_HOME=/tmp/ptlease-config xvfb-run -a cargo run -p photo-ui --example lease_smoke
//! 全部通过退出码 0；任何一项失败退出码 1。

use std::time::Duration;

use gpui_kit::component::dock::DockPlacement;
use gpui_kit::{AppContext as _, Bounds, Point, WindowBounds, WindowOptions, px, size};

use photo_engine::import::ImportSubfolder;
use photo_ui::actions::{Escape, OpenSettings, Rescan, Stats, ToggleLeftPanel, ToggleRightPanel};
use photo_ui::state::import;
use photo_ui::state::{ActiveDialog, AppState, ViewMode};

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

/// 等若干个渲染帧（动作经 `cx.defer` 在 effect 末尾执行，需要推进帧）。
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

        // 空目录：扫描立即结束，不会碰到真实照片
        let dir = std::env::temp_dir().join("pt_lease_smoke");
        let _ = std::fs::create_dir_all(&dir);

        // 「左停靠区宽度来自配置」的期望值直接读配置文件，不硬编码默认 200——
        // 开发机上左栏被拖宽过（配置里就不是 200）时，硬编码会让冒烟假失败。
        // 配置目录现在认 XDG_CONFIG_HOME / PHOTO_CONFIG_DIR，所以隔离跑法拿到的是默认值。
        let expected_left = photo_config::determine_config_path()
            .ok()
            .and_then(|path| photo_config::load_config(&path).ok())
            .map(|cfg| cfg.left_panel_width.clamp(200, 480) as f32)
            .unwrap_or(200.0);

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
            // 与生产接线一致：首帧后聚焦根视图（否则动作派发会丢）
            photo_ui::app::focus_root(&state, window, cx);
            state.update(cx, |state, _| {
                state.current_dir = Some(dir.clone());
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

                pump(async_cx, 400).await;

                // Dock 工作区已建立
                check!(
                    "Dock 工作区已建立",
                    async_cx.update(|cx| task_state.read(cx).dock_area.is_some())
                );

                // 左右停靠区显隐（Ctrl+[ / Ctrl+] 走 Dock 的 toggle_dock）
                let left = async_cx.update(|cx| {
                    task_state
                        .read(cx)
                        .dock_area
                        .as_ref()
                        .map(|area| area.read(cx).is_dock_open(DockPlacement::Left))
                });
                fire(async_cx, handle, Box::new(ToggleLeftPanel));
                pump(async_cx, 250).await;
                check!(
                    "左侧栏显隐",
                    async_cx.update(|cx| {
                        task_state
                            .read(cx)
                            .dock_area
                            .as_ref()
                            .map(|area| area.read(cx).is_dock_open(DockPlacement::Left))
                    }) != left
                );

                let right = async_cx.update(|cx| {
                    task_state
                        .read(cx)
                        .dock_area
                        .as_ref()
                        .map(|area| area.read(cx).is_dock_open(DockPlacement::Right))
                });
                fire(async_cx, handle, Box::new(ToggleRightPanel));
                pump(async_cx, 250).await;
                check!(
                    "右侧栏显隐",
                    async_cx.update(|cx| {
                        task_state
                            .read(cx)
                            .dock_area
                            .as_ref()
                            .map(|area| area.read(cx).is_dock_open(DockPlacement::Right))
                    }) != right
                );

                // 左停靠区宽度来自配置（期望值见 main 里的 expected_left），拖宽走同一条 set_dock_size 通路
                check!(
                    format!("左停靠区宽度来自配置 ({}px)", expected_left),
                    async_cx.update(|cx| {
                        task_state
                            .read(cx)
                            .dock_area
                            .as_ref()
                            .and_then(|area| area.read(cx).dock_size(DockPlacement::Left))
                            == Some(px(expected_left))
                    })
                );

                // 导入弹窗：open_import_dialog 建 Input、检测可移动盘、预填目标目录
                let _ = async_cx.update(|cx| {
                    let _ = handle.update(cx, |_view, window, cx| {
                        task_state.update(cx, |state, cx| {
                            state.open_import_dialog(window, cx);
                        });
                    });
                });
                pump(async_cx, 250).await;
                check!(
                    "导入弹窗打开",
                    async_cx.update(|cx| matches!(
                        task_state.read(cx).active_dialog,
                        Some(ActiveDialog::Import)
                    ))
                );
                check!(
                    "导入弹窗目标目录预填",
                    async_cx.update(|cx| !task_state.read(cx).import.dest_root.trim().is_empty())
                );
                fire(async_cx, handle, Box::new(Escape));
                pump(async_cx, 250).await;
                check!(
                    "Esc 不关导入弹窗",
                    async_cx.update(|cx| matches!(
                        task_state.read(cx).active_dialog,
                        Some(ActiveDialog::Import)
                    ))
                );
                let _ = async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| {
                        state.active_dialog = None;
                        cx.notify();
                    });
                });
                pump(async_cx, 120).await;

                // 设置弹窗（用户报 "window not found" 的那个按钮）
                fire(async_cx, handle, Box::new(OpenSettings));
                pump(async_cx, 250).await;
                check!(
                    "打开设置",
                    async_cx.update(|cx| matches!(
                        task_state.read(cx).active_dialog,
                        Some(ActiveDialog::Settings)
                    ))
                );
                fire(async_cx, handle, Box::new(Escape));
                pump(async_cx, 250).await;
                check!(
                    "Esc 关闭设置",
                    async_cx.update(|cx| task_state.read(cx).active_dialog.is_none())
                );

                // 观鸟统计
                fire(async_cx, handle, Box::new(Stats));
                pump(async_cx, 250).await;
                check!(
                    "观鸟统计",
                    async_cx.update(|cx| task_state.read(cx).view_mode == ViewMode::Stats)
                );
                fire(async_cx, handle, Box::new(Escape));
                pump(async_cx, 250).await;

                // 导入端到端：扫描 → 计划 → 执行（临时目录，不碰真实照片）
                let src = std::env::temp_dir().join("pt_lease_smoke_src");
                let dst = std::env::temp_dir().join("pt_lease_smoke_dst");
                let _ = std::fs::remove_dir_all(&src);
                let _ = std::fs::remove_dir_all(&dst);
                let _ = std::fs::create_dir_all(&src);
                for name in ["a.jpg", "b.jpg"] {
                    let _ = std::fs::write(src.join(name), b"not-a-real-jpeg");
                }
                let dst_text = dst.to_string_lossy().to_string();
                let _ = async_cx.update(|cx| {
                    let _ = handle.update(cx, |_view, window, cx| {
                        task_state.update(cx, |state, cx| {
                            state.import.source = Some(src.clone());
                            state.import.dest_root = dst_text.clone();
                            state.import.subfolder = ImportSubfolder::None;
                            state.import.candidates.clear();
                            state.import.plan = None;
                            state.import.result = None;
                            if let Some(input) = state.import_dest_input.clone() {
                                input.update(cx, |input_state, cx| {
                                    input_state.set_value(dst_text.clone(), window, cx);
                                });
                            }
                            cx.notify();
                        });
                    });
                });
                async_cx.update(|cx| import::scan_source(task_state.clone(), cx));
                pump(async_cx, 500).await;
                check!(
                    "导入源扫描",
                    async_cx.update(|cx| task_state.read(cx).import.candidates.len() == 2)
                );

                async_cx.update(|cx| import::generate_plan(task_state.clone(), cx));
                pump(async_cx, 500).await;
                check!(
                    "导入计划生成",
                    async_cx.update(|cx| task_state.read(cx).import.plan_count() == 2)
                );

                async_cx.update(|cx| import::execute(task_state.clone(), cx));
                pump(async_cx, 1800).await;
                check!(
                    "导入执行完成",
                    async_cx.update(|cx| task_state
                        .read(cx)
                        .import
                        .result
                        .map(|r| (r.imported, r.failed))
                        == Some((2, 0)))
                );
                check!(
                    "导入文件落盘",
                    dst.join("a.jpg").exists() && dst.join("b.jpg").exists()
                );
                // 导入成功后会自动重扫结果目录，等它收敛
                pump(async_cx, 500).await;

                // 刷新：既验证按钮通路，也验证不会双重租借
                let gen_before = async_cx.update(|cx| task_state.read(cx).scan_generation);
                fire(async_cx, handle, Box::new(Rescan));
                pump(async_cx, 700).await;
                let gen_after = async_cx.update(|cx| task_state.read(cx).scan_generation);
                check!("刷新触发扫描（且无双重租借）", gen_after > gen_before);

                let _ = async_cx.update(|cx| cx.quit());
                if failures > 0 {
                    eprintln!("{failures} 项失败");
                    std::process::exit(1);
                }
                println!("全部通过");
            })
            .detach();

            state
        })
        .expect("打开窗口失败");
    });
}
