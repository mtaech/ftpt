//! 手动冒烟：批量操作（复制 / 移动 / 重命名 / 删除到回收站）与 Ctrl+Z 撤销
//! （无头，自建素材，**不需要模型**）。
//!
//! 跑法：
//!   XDG_CONFIG_HOME=/tmp/pt-batch-config xvfb-run -a \
//!     cargo run -p photo-ui --example batch_ops_smoke
//! 全部通过退出码 0；失败退出码 1。支持 PHOTO_SMOKE_HOLD_MS 保持窗口供截图目检。
//!
//! 背景（docs/todo.md #13.1 / #13.2）：GPUI 版里左栏「复制到目录 / 移动到目录」以前
//! 弹完确认框**什么都不做**、批量重命名**连入口都没有**，而 op_journal 全仓从未 record
//! （Ctrl+Z 永远回答「没有可撤销的批量操作」）。引擎侧 batch_ops / ops::rename_captures
//! 早已就绪，本冒烟把接线后的整条链路逐项钉住：
//!
//!   1) 无筛选时批量操作默认锁住；「显式确认」解锁；筛选条件一变自动收回（安全门）
//!   2) Delete 键只弹确认框、不动文件；确认后才进回收站
//!   3) 进回收站时记撤销日志，Ctrl+Z 真的从回收站恢复
//!   4) 批量移动落地 4 个文件、Ctrl+Z 回原位
//!   5) 批量复制落地 4 个副本、Ctrl+Z 删副本且源保留
//!   6) 批量重命名生效、Ctrl+Z 改回原名
//!   7) 空模板被拒（有可读提示，不静默）
//!   8) 确认框 / 重命名弹窗渲染多帧不 panic

use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui_kit::{AppContext as _, Bounds, Point, WindowBounds, WindowOptions, px, size};
use photo_domain::Rating;

use photo_ui::actions::{Delete, Rescan, Undo};
use photo_ui::state::{ActiveDialog, AppState, engine_ops};

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

/// 自建素材：小尺寸 JPEG（扫描/缩略图管线都吃得下，不依赖外部图片）
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
    let cfg_dir = std::env::temp_dir().join("pt_batch_ops_smoke_config");
    let _ = std::fs::remove_dir_all(&cfg_dir);
    std::fs::create_dir_all(&cfg_dir).expect("建隔离配置目录失败");
    // SAFETY: 在 GPUI 启动前、进程单线程时设置
    unsafe {
        std::env::set_var("PHOTO_CONFIG_DIR", &cfg_dir);
    }

    let dir = std::env::temp_dir().join("pt_batch_ops_smoke");
    let dest = std::env::temp_dir().join("pt_batch_ops_smoke_dest");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&dest);
    std::fs::create_dir_all(&dir).expect("建素材目录失败");
    for (i, name) in ["img_a.jpg", "img_b.jpg", "img_c.jpg", "img_d.jpg"]
        .iter()
        .enumerate()
    {
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
            let dir = dir.clone();
            let dest = dest.clone();
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
                /// 轮询等待条件成立（最多 $ms）
                macro_rules! wait_for {
                    ($cond:expr, $ms:expr) => {{
                        let mut waited = 0u64;
                        loop {
                            if $cond || waited > $ms {
                                break $cond;
                            }
                            pump(async_cx, 200).await;
                            waited += 200;
                        }
                    }};
                }
                /// 等到「扫描结束 + 可见集恢复到 n 张」：撤销/批量操作后紧接着的自动重扫是
                /// defer 出去的，只等 is_scanning 会赶在重扫开始之前通过（display_order 还是空的）
                macro_rules! wait_ready {
                    ($n:expr) => {
                        wait_for!(
                            async_cx.update(|cx| {
                                let s = task_state.read(cx);
                                !s.is_scanning && s.display_order.len() == $n
                            }),
                            25_000
                        )
                    };
                }
                macro_rules! batch_result {
                    () => {
                        async_cx.update(|cx| {
                            task_state.read(cx).batch_result.clone().unwrap_or_default()
                        })
                    };
                }
                macro_rules! files_in {
                    ($p:expr) => {{
                        let mut v: Vec<String> = std::fs::read_dir(&$p)
                            .map(|it| {
                                it.filter_map(|e| e.ok())
                                    .map(|e| e.file_name().to_string_lossy().to_string())
                                    .filter(|n| n.ends_with(".jpg"))
                                    .collect::<Vec<String>>()
                            })
                            .unwrap_or_default();
                        v.sort();
                        v
                    }};
                }

                pump(async_cx, 300).await;
                fire(async_cx, handle, Box::new(Rescan));
                pump(async_cx, 2500).await;
                let count = async_cx.update(|cx| task_state.read(cx).items.len());
                check!("扫描到 4 张素材", count == 4);
                if count != 4 {
                    eprintln!("FAIL: 扫描没有结果，后续检查无意义");
                    std::process::exit(1);
                }

                // ── 1) 安全门（#13.1）：默认锁定 → 显式解锁 → 筛选一变自动收回 ──
                check!(
                    "无筛选时默认不解锁「对全目录执行」",
                    async_cx.update(|cx| !task_state.read(cx).batch_ignore_filter)
                );
                async_cx.update(|cx| task_state.update(cx, |s, _| s.batch_ignore_filter = true));
                check!(
                    "显式确认后解锁",
                    async_cx.update(|cx| task_state.read(cx).batch_ignore_filter)
                );
                async_cx.update(|cx| {
                    task_state.update(cx, |s, _| {
                        s.criteria.min_rating = Some(Rating::One);
                        s.recompute_pipeline();
                    })
                });
                check!(
                    "筛选条件一变自动收回解锁（解锁状态不跨筛选存活）",
                    async_cx.update(|cx| !task_state.read(cx).batch_ignore_filter)
                );
                async_cx.update(|cx| {
                    task_state.update(cx, |s, _| {
                        s.criteria.min_rating = None;
                        s.recompute_pipeline();
                        // 上面那次筛选把选中集清空了（筛选结果为空时保留选中没有意义），
                        // 这里重新选第一张，后面的 Delete 才有作用域
                        if let Some(&i) = s.display_order.first() {
                            s.select_single(i);
                        }
                    })
                });
                check!(
                    "重新选中 1 张（Delete 的作用域）",
                    async_cx.update(|cx| task_state.read(cx).mark_indices().len() == 1)
                );

                // ── 2) Delete 键：只弹确认框，不动文件 ──
                fire(async_cx, handle, Box::new(Delete));
                pump(async_cx, 200).await;
                let dlg = async_cx.update(|cx| task_state.read(cx).active_dialog.clone());
                check!("Delete 键先弹确认框（不再直接删）", dlg == Some(ActiveDialog::DeleteConfirm));
                check!("确认前文件一个没动", files_in!(dir).len() == 4);

                // ── 3) 确认删除（弹窗按钮走的就是 delete_selected_to_trash）──
                let selected_path = async_cx
                    .update(|cx| {
                        let s = task_state.read(cx);
                        s.mark_indices()
                            .first()
                            .and_then(|&i| s.items.get(i))
                            .map(|m| PathBuf::from(&m.primary_path))
                            .unwrap_or_default()
                    });
                async_cx.update(|cx| {
                    task_state.update(cx, |s, _| s.active_dialog = None);
                    engine_ops::delete_selected_to_trash(task_state.clone(), cx);
                });
                check!("Delete 有选中作用域", !selected_path.as_os_str().is_empty());
                let gone = wait_for!(!selected_path.exists(), 20_000);
                check!(format!("确认后进回收站（{}）", selected_path.display()), gone);
                check!(
                    "撤销日志有记录（以前永远是空的）",
                    async_cx.update(|cx| task_state.read(cx).op_journal.has_pending())
                );
                let del_msg = batch_result!();
                check!(
                    format!("左栏常驻结果提示可从回收站恢复（{del_msg:?}）"),
                    del_msg.contains("Ctrl+Z")
                );
                wait_ready!(4);

                // ── 4) Ctrl+Z：从回收站恢复 ──
                fire(async_cx, handle, Box::new(Undo));
                let back = wait_for!(selected_path.exists(), 20_000);
                check!("Ctrl+Z 从回收站恢复文件", back);
                check!(
                    "撤销后日志清空（单槽语义）",
                    async_cx.update(|cx| !task_state.read(cx).op_journal.has_pending())
                );
                wait_ready!(4);

                // ── 5) 批量移动 4 张 → 撤销回原位 ──
                async_cx.update(|cx| {
                    engine_ops::start_batch_op(
                        task_state.clone(),
                        photo_domain::BatchOpType::Move,
                        Some(dest.clone()),
                        cx,
                    );
                });
                let moved = wait_for!(
                    files_in!(dest).len() == 4 && files_in!(dir).is_empty(),
                    30_000
                );
                check!(
                    format!("批量移动到目录真的落地 4 个文件（dest={}）", files_in!(dest).len()),
                    moved
                );
                check!(
                    "移动后记了撤销日志",
                    wait_for!(
                        async_cx.update(|cx| task_state.read(cx).op_journal.has_pending()),
                        10_000
                    )
                );
                let move_msg = batch_result!();
                check!(
                    format!("左栏常驻结果报移动完成（{move_msg:?}）"),
                    move_msg.contains("批量移动完成")
                );
                // 移动后目录为空：等这次自动重扫把可见集清空，再撤销
                wait_for!(
                    async_cx.update(|cx| {
                        let s = task_state.read(cx);
                        !s.is_scanning && s.display_order.is_empty()
                    }),
                    25_000
                );

                fire(async_cx, handle, Box::new(Undo));
                let restored = wait_for!(
                    files_in!(dir).len() == 4 && files_in!(dest).is_empty(),
                    20_000
                );
                check!("Ctrl+Z 撤销移动：文件回原位", restored);
                wait_ready!(4);

                // ── 6) 批量复制 → 撤销删副本 ──
                async_cx.update(|cx| {
                    engine_ops::start_batch_op(
                        task_state.clone(),
                        photo_domain::BatchOpType::Copy,
                        Some(dest.clone()),
                        cx,
                    );
                });
                let copied = wait_for!(
                    files_in!(dest).len() == 4 && files_in!(dir).len() == 4,
                    30_000
                );
                check!("批量复制：目标 4 个副本、源仍在", copied);
                wait_ready!(4);
                fire(async_cx, handle, Box::new(Undo));
                let copy_undone = wait_for!(
                    files_in!(dest).is_empty() && files_in!(dir).len() == 4,
                    20_000
                );
                check!("Ctrl+Z 撤销复制：删副本、源保留", copy_undone);
                wait_ready!(4);

                // ── 7) 批量重命名 → 撤销改回 ──
                async_cx.update(|cx| {
                    engine_ops::start_batch_rename(task_state.clone(), "{name}_renamed".into(), cx);
                });
                let renamed = wait_for!(
                    files_in!(dir).len() == 4
                        && files_in!(dir).iter().all(|n| n.contains("_renamed")),
                    30_000
                );
                check!(format!("批量重命名生效（{:?}）", files_in!(dir)), renamed);
                wait_ready!(4);
                fire(async_cx, handle, Box::new(Undo));
                let rename_undone = wait_for!(
                    files_in!(dir).len() == 4
                        && files_in!(dir).iter().all(|n| !n.contains("_renamed")),
                    20_000
                );
                check!("Ctrl+Z 撤销重命名：改回原名", rename_undone);
                wait_ready!(4);

                // ── 8) 空模板被拒（不静默、不产生任何改名）──
                async_cx.update(|cx| {
                    engine_ops::start_batch_rename(task_state.clone(), "   ".into(), cx);
                });
                pump(async_cx, 300).await;
                // 状态栏那行可能被随后收尾的重扫覆盖，所以断言常驻结果（左栏批处理面板显示它）
                let empty_msg = batch_result!();
                check!(
                    format!("空模板被拒并给出提示（{empty_msg:?}）"),
                    empty_msg.contains("模板为空")
                );

                // ── 9) 确认框 / 重命名弹窗渲染多帧不 panic ──
                async_cx.update(|cx| {
                    task_state.update(cx, |s, _| {
                        s.batch_target_dir = Some(dest.clone());
                        s.active_dialog =
                            Some(ActiveDialog::BatchConfirm(photo_domain::BatchOpType::Move));
                    })
                });
                pump(async_cx, 500).await;
                check!("移动确认框渲染多帧不 panic", true);
                async_cx.update(|cx| {
                    task_state.update(cx, |s, _| {
                        s.batch_target_dir = None;
                        s.active_dialog = Some(ActiveDialog::Rename);
                    })
                });
                pump(async_cx, 500).await;
                check!("重命名弹窗渲染多帧不 panic", true);

                let hold = std::env::var("PHOTO_SMOKE_HOLD_MS")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(0);
                if hold > 0 {
                    pump(async_cx, hold).await;
                }

                if failures == 0 {
                    println!("ALL PASS");
                } else {
                    eprintln!("{failures} 项失败");
                    std::process::exit(1);
                }
                std::process::exit(0);
            })
            .detach();

            state
        })
        .expect("开窗失败");
    });
}
