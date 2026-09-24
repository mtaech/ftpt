//! 手动冒烟：照片右键菜单（§13.4）——键盘导航 + 最大高度滚动（无头，自建素材，**不需要模型**）。
//!
//! 跑法：
//!   XDG_CONFIG_HOME=/tmp/pt-ctxmenu-config xvfb-run -a \
//!     cargo run -p photo-ui --example context_menu_smoke
//! 全部通过退出码 0；失败退出码 1。
//!
//! 背景（docs/todo.md #13.4）：GPUI 版**根本没有右键菜单**；而 gpui-component 的
//! PopupMenu 只处理点击（整个 menu 模块没有一处键盘处理），所以菜单是自绘的。
//! 本冒烟钉住：
//!   1) 网格/胶片条/预览三处右键都会打开同一个菜单（open_photo_context_menu）
//!   2) 菜单项随上下文变（预览态里「在预览中打开」置灰）且初始高亮落在可用项上
//!   3) 键盘上下移动：跳过置灰项、到边界不环绕
//!   4) Esc 关菜单（handle_escape 第 0 优先级）
//!   5) 执行：Pick 生效 / 打开预览 / 移至回收站只弹确认框（不动文件）
//!   6) 10 项**一次看全**（高度按内容给，放得下就不滚动）——用户报过菜单太矮；
//!      真正需要滚动时的高度/滚动判据由 model::context_menu 的纯逻辑单测覆盖
//!   7) 目标已不在目录时诚实报错，不默默作用到别的照片
//!   8) 菜单开着渲染多帧不 panic

use std::path::Path;
use std::time::Duration;

use gpui_kit::{AppContext as _, Bounds, Point, WindowBounds, WindowOptions, px, size};
use photo_domain::Flag;

use photo_ui::actions::Rescan;
use photo_ui::model::context_menu::ContextMenuAction;
use photo_ui::state::{ActiveDialog, AppState};
use photo_ui::views::run_context_menu_action;

async fn pump(async_cx: &mut gpui_kit::AsyncApp, ms: u64) {
    async_cx
        .background_executor()
        .timer(Duration::from_millis(ms))
        .await;
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

fn write_jpeg(path: &Path, seed: u8) {
    let img = image::RgbImage::from_fn(48, 36, |x, y| {
        image::Rgb([
            (x * 5 + seed as u32 * 17) as u8,
            (y * 7 + seed as u32 * 3) as u8,
            std::cmp::min(255, seed as u32 * 31) as u8,
        ])
    });
    img.save(path).expect("写测试素材失败");
}

fn main() {
    photo_ui::app::prepare_headless_smoke();
    let cfg_dir = std::env::temp_dir().join("pt_ctxmenu_smoke_config");
    let _ = std::fs::remove_dir_all(&cfg_dir);
    std::fs::create_dir_all(&cfg_dir).expect("建隔离配置目录失败");
    // SAFETY: 在 GPUI 启动前、进程单线程时设置
    unsafe {
        std::env::set_var("PHOTO_CONFIG_DIR", &cfg_dir);
    }

    let dir = std::env::temp_dir().join("pt_ctxmenu_smoke");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建素材目录失败");
    for (i, name) in ["m_a.jpg", "m_b.jpg", "m_c.jpg"].iter().enumerate() {
        write_jpeg(&dir.join(name), i as u8);
    }

    let app = gpui_kit::application().with_assets(gpui_kit::assets::AllAssets);
    app.run(move |cx| {
        gpui_kit::component::init(cx);
        photo_ui::theme::apply(None, false, None, None, cx);
        AppState::register_keybindings(cx);

        let window_options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point { x: px(0.), y: px(0.) },
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
                pump(async_cx, 2500).await;
                let count = read_state!(|s: &AppState| s.items.len());
                check!("扫描到 3 张素材", count == 3);
                if count != 3 {
                    eprintln!("FAIL: 素材不足，后续检查无意义");
                    std::process::exit(1);
                }

                // ── 1) 打开菜单（三处右键走的都是 open_photo_context_menu）──
                let target = read_state!(|s: &AppState| s.items[1].primary_path.clone());
                async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| {
                        state.open_photo_context_menu(640.0, 380.0, target.clone(), cx);
                    });
                });
                pump(async_cx, 300).await;
                let (item_count, selected, opened) = read_state!(|s: &AppState| {
                    let m = s.context_menu.as_ref();
                    (m.map(|m| m.items.len()), m.map(|m| m.selected), m.is_some())
                });
                check!("右键打开菜单（10 项）", opened && item_count == Some(10));
                check!("初始高亮落在第一个可用项", selected == Some(0));
                check!(
                    "菜单目标是命中的那张照片",
                    read_state!(|s: &AppState| s
                        .context_menu
                        .as_ref()
                        .map(|m| m.target.clone()))
                        == Some(target.clone())
                );

                // ── 2) 高度按内容给：10 项（288px）在 760px 窗口里一次看全，不需要滚动 ──
                let max_off = read_state!(|s: &AppState| s.context_menu_scroll.max_offset());
                check!(
                    format!("10 项一次看全（max_offset {:?}，应为 0）", max_off.y),
                    f32::from(max_off.y) == 0.0
                );
                check!(
                    "菜单项数仍是 10（高度按内容给，不再是 240px 上限）",
                    read_state!(|s: &AppState| s.context_menu.as_ref().map(|m| m.items.len()))
                        == Some(10)
                );

                // ── 3) 键盘移动：跳过置灰项、边界不环绕 ──
                async_cx.update(|cx| {
                    task_state.update(cx, |state, _| state.move_context_menu_selection(1));
                });
                check!("↓ 移到下一项", read_state!(|s: &AppState| s.context_menu.as_ref().map(|m| m.selected)) == Some(1));
                async_cx.update(|cx| {
                    task_state.update(cx, |state, _| {
                        // 一路按到底：不应环绕回第一项
                        for _ in 0..20 {
                            state.move_context_menu_selection(1);
                        }
                    });
                });
                check!(
                    "连按到底停在最后一项（不环绕）",
                    read_state!(|s: &AppState| s.context_menu.as_ref().map(|m| m.selected)) == Some(9)
                );
                async_cx.update(|cx| {
                    task_state.update(cx, |state, _| {
                        for _ in 0..20 {
                            state.move_context_menu_selection(-1);
                        }
                    });
                });
                check!(
                    "连按到顶停在第一项",
                    read_state!(|s: &AppState| s.context_menu.as_ref().map(|m| m.selected)) == Some(0)
                );

                // ── 4) 预览态：第一项置灰，初始高亮落到下一项 ──
                async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| {
                        state.view_mode = photo_ui::state::ViewMode::Preview;
                        state.open_photo_context_menu(200.0, 200.0, target.clone(), cx);
                    });
                });
                pump(async_cx, 200).await;
                let (first_disabled, preview_selected) = read_state!(|s: &AppState| {
                    let m = s.context_menu.as_ref().unwrap();
                    (m.items[0].disabled, m.selected)
                });
                check!("预览态里「在预览中打开」置灰", first_disabled);
                check!("置灰项不会被初始高亮选中", preview_selected == 1);
                async_cx.update(|cx| {
                    task_state.update(cx, |state, _| state.move_context_menu_selection(-1));
                });
                check!(
                    "↑ 撞上置灰项时停在原地（不跳到禁用项）",
                    read_state!(|s: &AppState| s.context_menu.as_ref().map(|m| m.selected)) == Some(1)
                );

                // ── 5) Esc 关菜单（handle_escape 第 0 优先级）──
                let handled = async_cx.update(|cx| {
                    task_state.update(cx, |state, _| state.handle_escape())
                });
                check!(
                    format!("Esc 关菜单（handle_escape={handled}）"),
                    handled && read_state!(|s: &AppState| s.context_menu.is_none())
                );

                // ── 6) 执行动作（菜单点击与回车都走 run_context_menu_action）──
                async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| {
                        state.open_photo_context_menu(300.0, 300.0, target.clone(), cx);
                        run_context_menu_action(
                            state,
                            ContextMenuAction::SetPick,
                            target.clone(),
                            cx,
                        );
                    });
                });
                pump(async_cx, 400).await;
                let (picked, menu_closed) = read_state!(|s: &AppState| {
                    (
                        s.items
                            .iter()
                            .find(|m| m.primary_path == target)
                            .and_then(|m| m.flag),
                        s.context_menu.is_none(),
                    )
                });
                check!(
                    format!("「标识为 Pick」真的写进摘要（{picked:?}）"),
                    picked == Some(Flag::Pick)
                );
                check!("执行后菜单自动关闭", menu_closed);

                // 打开预览
                async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| {
                        state.open_photo_context_menu(300.0, 300.0, target.clone(), cx);
                        run_context_menu_action(
                            state,
                            ContextMenuAction::OpenPreview,
                            target.clone(),
                            cx,
                        );
                    });
                });
                pump(async_cx, 300).await;
                check!(
                    "「在预览中打开」真的切到预览态",
                    read_state!(|s: &AppState| s.view_mode) == photo_ui::state::ViewMode::Preview
                );

                // 移至回收站：只弹确认框，绝不直接删
                async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| {
                        state.open_photo_context_menu(300.0, 300.0, target.clone(), cx);
                        run_context_menu_action(
                            state,
                            ContextMenuAction::MoveToTrash,
                            target.clone(),
                            cx,
                        );
                    });
                });
                pump(async_cx, 300).await;
                let (dialog, still_there) = read_state!(|s: &AppState| {
                    (
                        s.active_dialog.clone(),
                        s.items.iter().any(|m| m.primary_path == target),
                    )
                });
                check!(
                    format!("「移至回收站…」只弹确认框（{dialog:?}）"),
                    dialog == Some(ActiveDialog::DeleteConfirm)
                );
                check!(
                    "确认之前文件仍在磁盘上",
                    still_there && Path::new(&target).exists()
                );

                // ── 7) 目标已不在目录：诚实报错，不作用到别的照片 ──
                async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| {
                        state.active_dialog = None;
                        state.open_photo_context_menu(
                            300.0,
                            300.0,
                            "/nowhere/gone.jpg".to_string(),
                            cx,
                        );
                        run_context_menu_action(
                            state,
                            ContextMenuAction::SetReject,
                            "/nowhere/gone.jpg".to_string(),
                            cx,
                        );
                    });
                });
                pump(async_cx, 300).await;
                let (status, any_reject) = read_state!(|s: &AppState| {
                    (
                        s.status_message
                            .clone()
                            .map(|(m, _)| m)
                            .unwrap_or_default(),
                        s.items.iter().any(|m| m.flag == Some(Flag::Reject)),
                    )
                });
                check!(
                    format!("目标不在目录时诚实报错（{status:?}）"),
                    status.contains("已不在当前目录")
                );
                check!("不会把动作作用到别的照片", !any_reject);

                // ── 8) 菜单开着渲染多帧不 panic ──
                async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| {
                        state.open_photo_context_menu(600.0, 420.0, target.clone(), cx);
                    });
                });
                pump(async_cx, 600).await;
                pump(async_cx, 600).await;
                check!(
                    "菜单渲染多帧不 panic 且仍打开",
                    read_state!(|s: &AppState| s.context_menu.is_some())
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
        .expect("开窗失败");
    });
}
