//! 手动冒烟：重复/相似照片检测整条链路（无头，自建素材，**不需要模型**）。
//!
//! 跑法：
//!   XDG_CONFIG_HOME=/tmp/pt-dup-config xvfb-run -a \
//!     cargo run -p photo-ui --example duplicates_smoke
//! 全部通过退出码 0；失败退出码 1。
//!
//! 背景（docs/todo.md #2 / #1b）：引擎侧 `photo-engine/src/phash.rs` 早已就绪（dHash +
//! 汉明距离 + 贪心聚类，带 9 个单测）但全仓 0 调用方，而左栏入口打开的是一个
//! 「点开始检测只弹状态栏提示就关窗」的占位弹窗。本冒烟覆盖接线后的完整链路：
//!
//!   1) 弹窗入口真的打开（`open_duplicates_dialog`，左栏按钮走的就是它）
//!   2) 检测作用域 = 当前目录全部照片，**剔除同 stem 多格式组**（RAW+JPEG 同画面）
//!   3) 检测跑完真的出分组：2 组 × 2 张（两对内容重复），无关照片不误报
//!   4) 组内首张 = 保留锚点（keeper），多余张数 = summarize
//!   5) 结果落 `folder_db.duplicates`（阈值 + 时间 + keeper 标记）
//!   6) 重开弹窗能从落库结果读回（不用重算）
//!   7) 空文件健壮性：0 字节文件早判（不再让 LibRaw 报 I/O error）、扫描收尾汇总一次
//!   8) 弹窗保持打开时持续渲染不 panic

use std::path::Path;
use std::time::Duration;

use gpui_kit::{AppContext as _, Bounds, Point, WindowBounds, WindowOptions, px, size};

use photo_ui::actions::Rescan;
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

/// 直接构造 dHash 位模式可控的素材：9×8 灰度网格，每行按 ±12 步长累计，使
/// 「左 < 右」严格等于目标 bit；再按 CELL 倍整数放大（结构块远大于滤波核，缩放后位模式不变）。
///
/// 为什么不用「渐变 + 亮块」那类自然感素材：dHash 只看 9×8 网格的水平相邻亮度比较，
/// 渐变图彼此距离只有个位数（实测 24 张不同亮块位置互相 ≤8），根本造不出「无关照片」。
/// 位模式法则可精确控制距离：同 pattern = 0（真重复），不同 pattern 之间 = 32/64 bit。
fn write_pattern(path: &Path, pattern: u8) {
    const CELL: u32 = 24;
    let mut grid = [[0u8; 9]; 8];
    for y in 0..8usize {
        let mut v: i32 = 128;
        grid[y][0] = v as u8;
        for x in 0..8usize {
            let bit = (pattern >> x) & 1 == 1;
            v += if bit { 12 } else { -12 };
            grid[y][x + 1] = v as u8;
        }
    }
    let img = image::RgbImage::from_fn(9 * CELL, 8 * CELL, |x, y| {
        let v = grid[(y / CELL).min(7) as usize][(x / CELL).min(8) as usize];
        image::Rgb([v, v, v])
    });
    img.save(path).expect("写测试素材失败");
}

fn main() {
    photo_ui::app::prepare_headless_smoke();
    let cfg_dir = std::env::temp_dir().join("pt_duplicates_smoke_config");
    let _ = std::fs::remove_dir_all(&cfg_dir);
    std::fs::create_dir_all(&cfg_dir).expect("建隔离配置目录失败");
    // SAFETY: 在 GPUI 启动前、进程单线程时设置
    unsafe {
        std::env::set_var("PHOTO_CONFIG_DIR", &cfg_dir);
    }

    let dir = std::env::temp_dir().join("pt_duplicates_smoke");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建素材目录失败");

    // 组 A：a1/a2 同 pattern（dHash 距离 0）；组 B：b1/b2 同 pattern；
    // c1..c4 各用一个与彼此、与 A/B 都相距 ≥32 bit 的 pattern（无关照片不误报）
    write_pattern(&dir.join("a1.png"), 0xAA);
    write_pattern(&dir.join("a2.png"), 0xAA);
    write_pattern(&dir.join("b1.png"), 0x55);
    write_pattern(&dir.join("b2.png"), 0x55);
    write_pattern(&dir.join("c1.png"), 0xF0);
    write_pattern(&dir.join("c2.png"), 0x0F);
    write_pattern(&dir.join("c3.png"), 0xCC);
    write_pattern(&dir.join("c4.png"), 0x33);

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

                /// 跑一次检测并等收尾（与弹窗「开始检测」按钮同一条：start_duplicates）
                macro_rules! run_detect {
                    () => {{
                        async_cx.update(|cx| {
                            engine_ops::start_duplicates(task_state.clone(), cx);
                        });
                        let mut waited = 0u64;
                        loop {
                            let running = async_cx.update(|cx| task_state.read(cx).is_detecting_duplicates);
                            if !running || waited > 60_000 {
                                break;
                            }
                            pump(async_cx, 200).await;
                            waited += 200;
                        }
                    }};
                }

                pump(async_cx, 300).await;
                fire(async_cx, handle, Box::new(Rescan));
                pump(async_cx, 2500).await;
                let count = async_cx.update(|cx| task_state.read(cx).items.len());
                check!("扫描到 8 张素材", count == 8);
                if count != 8 {
                    eprintln!("FAIL: 扫描没有结果，后续检查无意义");
                    std::process::exit(1);
                }

                // ── 1) 入口：左栏按钮走的 open_duplicates_dialog 真的开窗 ──
                async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| state.open_duplicates_dialog(cx));
                });
                pump(async_cx, 200).await;
                let dialog = async_cx.update(|cx| task_state.read(cx).active_dialog.clone());
                check!("入口打开重复检测弹窗", dialog == Some(ActiveDialog::Duplicates));

                // ── 2) 阈值档位可设（弹窗 chip 改的就是这个字段）──
                async_cx.update(|cx| {
                    task_state.update(cx, |state, _| state.dup_threshold = 10);
                });

                // ── 3) 检测：作用域 / 分组 / 收尾 ──
                run_detect!();
                let (total, done, groups, summary, status) = async_cx.update(|cx| {
                    let s = task_state.read(cx);
                    (
                        s.dup_total,
                        s.dup_done,
                        s.dup_groups.clone(),
                        photo_ui::model::duplicates::summarize(&s.dup_groups),
                        s.status_message.clone().map(|(m, _)| m),
                    )
                });
                check!("检测作用域 = 8 张（无同 stem 多格式可剔除）", total == 8);
                check!("进度跑完 (done == total)", done == total);
                check!(format!("出 2 组重复（实际 {} 组）", groups.len()), groups.len() == 2);
                check!("分组 = 每组 2 张（keeper + 1）", groups.iter().all(|g| g.len() == 2));
                check!("多余张数 = 2", summary == (2, 2));
                check!(
                    format!("状态栏报真实结果（{status:?}）"),
                    status.as_deref() == Some("重复检测完成：2 组、2 张多余（阈值 10）")
                );
                // 每组首张 = 路径序靠前（keeper 语义）
                let keeper_ok = groups.iter().all(|g| {
                    let first = Path::new(&g[0]).file_name().map(|n| n.to_string_lossy().to_string());
                    matches!(first.as_deref(), Some("a1.png") | Some("b1.png"))
                });
                check!("组内首张 = 保留锚点（a1 / b1）", keeper_ok);
                // 无关照片不误报
                let grouped_paths: Vec<String> = groups.iter().flatten().cloned().collect();
                check!(
                    "无关照片（c1..c4）没有被误报",
                    !grouped_paths.iter().any(|p| Path::new(p)
                        .file_name()
                        .map(|n| n.to_string_lossy().starts_with('c'))
                        .unwrap_or(false))
                );

                // ── 4) 落库：阈值 / keeper / 行数 ──
                let rows = async_cx.update(|cx| {
                    let s = task_state.read(cx);
                    s.folder_db
                        .as_ref()
                        .and_then(|db| db.load_duplicates().ok())
                        .unwrap_or_default()
                });
                check!("结果落 folder_db（4 行入组照片）", rows.len() == 4);
                check!("落库阈值 = 选定阈值 10", rows.iter().all(|r| r.threshold == 10));
                check!(
                    "落库 keeper 每组恰好 1 个",
                    {
                        let mut gs: Vec<i64> = rows.iter().map(|r| r.group_index).collect();
                        gs.sort_unstable();
                        gs.dedup();
                        gs.len() == 2 && rows.iter().filter(|r| r.keeper).count() == 2
                    }
                );
                check!(
                    "落库时间已记录（UI 展示用）",
                    async_cx.update(|cx| task_state.read(cx).dup_computed_at.is_some())
                );

                // ── 4b) 非破坏性路径（手册 §9.10 原始规格）：其余标 Rejected，文件不动 ──
                let extras0: Vec<String> = groups[0][1..].to_vec();
                let bytes_before = std::fs::read(&extras0[0]).expect("读素材失败");
                async_cx.update(|cx| {
                    task_state.update(cx, |state, _| {
                        engine_ops::set_flag_for_paths(state, &extras0, Some(photo_domain::Flag::Reject));
                    });
                });
                let (flagged_mem, flagged_db) = async_cx.update(|cx| {
                    let s = task_state.read(cx);
                    let mem = s
                        .items
                        .iter()
                        .filter(|m| m.flag == Some(photo_domain::Flag::Reject))
                        .count();
                    let db = extras0
                        .iter()
                        .filter(|p| {
                            s.folder_db
                                .as_ref()
                                .and_then(|db| db.get_xmp(Path::new(p)).ok().flatten())
                                .and_then(|x| x.flag())
                                == Some(photo_domain::Flag::Reject)
                        })
                        .count();
                    (mem, db)
                });
                check!("其余标 Rejected：内存旗标生效", flagged_mem == 1);
                check!("其余标 Rejected：folder_db 的 xmp 同步", flagged_db == 1);
                check!(
                    "标 Rejected 不动文件（字节不变）",
                    std::fs::read(&extras0[0]).expect("读素材失败") == bytes_before
                );

                // ── 5) 重开弹窗读回（清空内存分组后仍能从落库恢复）──
                async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| {
                        state.dup_groups.clear();
                        state.dup_computed_at = None;
                        state.open_duplicates_dialog(cx);
                    });
                });
                pump(async_cx, 200).await;
                let reloaded = async_cx.update(|cx| {
                    let s = task_state.read(cx);
                    (s.dup_groups.len(), s.dup_computed_at.is_some(), s.dup_threshold)
                });
                check!(
                    format!("重开弹窗读回落库结果（{reloaded:?}）"),
                    reloaded == (2, true, 10)
                );

                // ── 6) 同 stem 多格式剔除：加 dup.jpg + dup.rw2 后作用域仍是 8 ──
                write_pattern(&dir.join("dup.png"), 0xF0);
                std::fs::write(dir.join("dup.rw2"), b"not a real raw").expect("写 RAW 占位");
                fire(async_cx, handle, Box::new(Rescan));
                pump(async_cx, 2500).await;
                async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| state.open_duplicates_dialog(cx));
                });
                run_detect!();
                let (items_after, total_after) = async_cx.update(|cx| {
                    let s = task_state.read(cx);
                    (s.items.len(), s.dup_total)
                });
                check!("重扫后素材 10 张（含同 stem 的 dup.png/dup.rw2）", items_after == 10);
                check!(
                    format!("同 stem 多格式被排除在检测作用域外（作用域 {total_after}）"),
                    total_after == 8
                );

                // ── 7) 空文件健壮性（真实案例：0 字节 RW2 让 LibRaw 报 -100009 I/O error，
                //        日志里每轮扫描刷 ERROR；现在应早判 + 扫描收尾汇总一次）──
                std::fs::write(dir.join("broken.rw2"), b"").expect("写空 RAW 占位");
                fire(async_cx, handle, Box::new(Rescan));
                pump(async_cx, 3500).await;
                let (items_broken, thumb_running, status_broken) = async_cx.update(|cx| {
                    let s = task_state.read(cx);
                    (
                        s.items.len(),
                        s.thumb_total > 0,
                        s.status_message.clone().map(|(m, _)| m),
                    )
                });
                check!("空文件也进扫描结果（网格里可见、可清理）", items_broken == 11);
                check!(
                    format!("空文件不卡住缩略图管线（thumb_total 仍在跑={thumb_running}）"),
                    !thumb_running
                );
                check!(
                    format!("扫描收尾汇总无法生成缩略图的张数（{status_broken:?}）"),
                    status_broken
                        .as_deref()
                        .is_some_and(|m| m.contains("无法生成缩略图") && m.contains("已跳过"))
                );
                // 空文件仍会进检测作用域（无同 stem 配对）→ 哈希失败被跳过，分组不变
                run_detect!();
                let after_empty = async_cx.update(|cx| {
                    let s = task_state.read(cx);
                    (s.is_detecting_duplicates, s.dup_groups.len())
                });
                check!(
                    format!("空文件存在时检测仍能收尾且分组不变（{after_empty:?}）"),
                    after_empty == (false, 2)
                );

                // ── 8) 弹窗保持打开持续渲染几个节拍（渲染 panic 会直接崩）──
                // PHOTO_SMOKE_HOLD_MS：保持窗口供 Xvfb 外部截图目检（与 grid_scroll_smoke 同约定）
                let hold = std::env::var("PHOTO_SMOKE_HOLD_MS")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(600);
                pump(async_cx, hold).await;
                let still_open = async_cx.update(|cx| {
                    task_state.read(cx).active_dialog == Some(ActiveDialog::Duplicates)
                });
                check!("弹窗渲染多帧不 panic 且仍打开", still_open);

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
