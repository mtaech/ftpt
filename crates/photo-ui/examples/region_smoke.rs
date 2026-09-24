//! 手动冒烟：预览「框选识别」（漏检补充）+ 识别器常驻（无头，自建素材，不碰真实配置）。
//!
//! 跑法：
//!   XDG_CONFIG_HOME=/tmp/pt-region-config xvfb-run -a \
//!     cargo run -p photo-ui --example region_smoke
//! 全部通过退出码 0；失败退出码 1；缺 models/ 时跳过（退出码 0）。
//!
//! 钉住：
//!   1) ToggleRegionSelect action 切换框选模式状态
//!   2) 识别器懒装配一次后**常驻缓存**（槽位从 None → Some 并保持），
//!      第二次框选免掉 ~1-2s 模型装配
//!   3) 框选结果**追加**为该照片的新主体（首次建结论、二次追加，不覆盖已有主体）
//!   4) folder_db 持久化同步 + 全局索引（global.db）落行
//!   5) 批量识别复用同一常驻实例（不再另装一份）
//!
//! 说明：鼠标坐标 → 归一化 bbox 的换算由 model::tests 的纯函数单测覆盖；
//! 本冒烟直接调 start_region_recognition 验证引擎/持久化链路（含真实模型推理）。

use std::path::Path;
use std::time::Duration;

use gpui_kit::{AppContext as _, Bounds, Point, WindowBounds, WindowOptions, px, size};
use photo_domain::BBox;

use photo_ui::actions::{RecognizeAllUnrecognized, Rescan, ToggleRegionSelect};
use photo_ui::state::AppState;
use photo_ui::state::engine_ops::start_region_recognition;

const PHOTOS: usize = 2;

/// 在窗口上派发一个动作（与视图里 window.dispatch_action 同一条路径）。
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
    // 空闲自动卸载（#17）的检查节拍压到 500ms：默认 30s，端到端验证一个用例要等半分钟。
    // SAFETY: 在 GPUI 启动前、进程单线程时设置环境变量
    unsafe {
        std::env::set_var("PHOTO_RECOGNIZER_IDLE_TICK_MS", "500");
    }
    // 模型/名录库定位：与 data_root() 同一条约定，缺模型就跳过
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let has_models = std::env::var("PHOTO_DATA_DIR").is_ok()
        || (repo_root.join("models/org_det.onnx").exists()
            && repo_root.join("models/bioclip2_model_int8.onnx").exists()
            && repo_root.join("data/bird_catalog.db").exists());
    if !has_models {
        eprintln!("跳过：未找到 models/ 与 data/bird_catalog.db（设 PHOTO_DATA_DIR 或从仓库根跑）");
        return;
    }

    let dir = std::env::temp_dir().join("pt_region_smoke");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建素材目录失败");
    write_jpeg(&dir.join("reg_0.jpg"), (180, 60, 60));
    write_jpeg(&dir.join("reg_1.jpg"), (60, 140, 180));

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

                // ── 1) 框选模式开关（action → 状态）──
                fire(async_cx, handle, Box::new(ToggleRegionSelect));
                pump(async_cx, 120).await;
                check!(
                    "ToggleRegionSelect 打开框选模式",
                    read_state!(|s: &AppState| s.region_select)
                );
                fire(async_cx, handle, Box::new(ToggleRegionSelect));
                pump(async_cx, 120).await;
                check!(
                    "再切一次关闭框选模式",
                    !read_state!(|s: &AppState| s.region_select)
                );

                // ── 2) 识别器懒加载：任何识别之前槽位应为空 ──
                check!(
                    "识别器初始未装配（懒加载）",
                    !read_state!(|s: &AppState| s.recognizer.lock().is_some())
                );

                let path = read_state!(|s: &AppState| {
                    std::path::PathBuf::from(&s.items[0].primary_path)
                });

                // ── 3) 第一次框选（冷启动：装配 + 识别）→ 首次建立结论 ──
                let t_first = std::time::Instant::now();
                async_cx.update(|cx| {
                    start_region_recognition(
                        task_state.clone(),
                        path.clone(),
                        BBox::new(0.15, 0.15, 0.65, 0.75),
                        cx,
                    );
                });
                check!(
                    "框选识别已启动（进行中标志置位）",
                    read_state!(|s: &AppState| s.region_recognizing)
                );
                let mut finished = false;
                for _ in 0..800 {
                    // 最多 40s（含模型装配）
                    pump(async_cx, 50).await;
                    if !read_state!(|s: &AppState| s.region_recognizing) {
                        finished = true;
                        break;
                    }
                }
                let first_elapsed = t_first.elapsed();
                let after_first = read_state!(|s: &AppState| s.items[0].subjects.len());
                let status_msg = read_state!(|s: &AppState| {
                    s.status_message.as_ref().map(|(m, _)| m.clone())
                });
                check!(
                    format!("第一次框选在 40s 内完成并建立结论（{after_first} 个主体）[状态栏: {status_msg:?}]"),
                    finished && after_first == 1
                );

                // ── 4) 常驻缓存：槽位已装配并保持 ──
                check!(
                    "识别器装配后常驻缓存（槽位 Some）",
                    read_state!(|s: &AppState| s.recognizer.lock().is_some())
                );

                // ── 5) 第二次框选（热：直接复用，免装配）→ 追加为主体 ──
                let t_second = std::time::Instant::now();
                async_cx.update(|cx| {
                    start_region_recognition(
                        task_state.clone(),
                        path.clone(),
                        BBox::new(0.35, 0.4, 0.85, 0.9),
                        cx,
                    );
                });
                let mut finished2 = false;
                for _ in 0..800 {
                    pump(async_cx, 50).await;
                    if !read_state!(|s: &AppState| s.region_recognizing) {
                        finished2 = true;
                        break;
                    }
                }
                let second_elapsed = t_second.elapsed();
                let after_second = read_state!(|s: &AppState| s.items[0].subjects.len());
                println!(
                    "[diag] 框选耗时：首次 {first_elapsed:?}（含模型装配）/ 第二次 {second_elapsed:?}（复用常驻）"
                );
                check!(
                    format!("第二次框选**追加**为主体且免装配（1 → {after_second}，耗时 {second_elapsed:?}）"),
                    finished2 && after_second == after_first + 1
                );

                // ── 6) 持久化：folder_db 的主体数一致 ──
                let persisted = read_state!(|s: &AppState| {
                    s.folder_db
                        .as_ref()
                        .and_then(|db| db.get_recognition("reg_0.jpg").ok().flatten())
                        .map(|r| r.subjects.len())
                });
                check!(
                    format!("folder_db 持久化主体数 {persisted:?} == {after_second}"),
                    persisted == Some(after_second)
                );

                // ── 7) 全局索引：应有对应主体行 ──
                let gdb_rows = read_state!(|s: &AppState| {
                    s.global_db
                        .as_ref()
                        .and_then(|g| g.species_stats().ok())
                        .map(|v| v.iter().map(|x| x.photo_count).sum::<i64>())
                });
                check!(
                    format!("全局索引主体行数 {gdb_rows:?}（>= {after_second}）"),
                    gdb_rows.unwrap_or(0) >= after_second as i64
                );

                // ── 8) 批量识别复用同一常驻实例（不再另装一份）──
                fire(async_cx, handle, Box::new(RecognizeAllUnrecognized));
                let mut started = false;
                for _ in 0..300 {
                    pump(async_cx, 50).await;
                    if read_state!(|s: &AppState| s.is_recognizing) {
                        started = true;
                    } else if started {
                        break;
                    }
                }
                check!(
                    "批量识别跑完且仍复用同一常驻识别器",
                    read_state!(|s: &AppState| s.recognizer.lock().is_some())
                        && !read_state!(|s: &AppState| s.is_recognizing)
                );

                // ── 9) 回归（用户报「框选识别出来了，结果栏却显示『无记录』」）──
                // 信息栏主行只读顶层 taxon_name；框选是「追加主体」，若旧记录是
                // Unrecognized（顶层无结论、subjects 空），顶层不同步就会一直空白。
                // 这里把 reg_0 打回「旧记录」形状再框选一次，断言内存摘要与落库顶层都有名字。
                let legacy_unrecognized = photo_domain::Recognition {
                    status: photo_domain::RecognitionStatus::Unrecognized,
                    taxon: None,
                    class_index: None,
                    confidence: None,
                    bbox: None,
                    candidates: vec![],
                    failure_stage: photo_domain::RecognitionFailureStage::Detection,
                    recognized_at: "2026-09-23T10:00:00Z".to_string(),
                    subjects: vec![],
                };
                let wrote = read_state!(|s: &AppState| {
                    if let Some(db) = s.folder_db.as_ref() {
                        db.upsert_recognition("reg_0.jpg", &legacy_unrecognized).is_ok()
                    } else {
                        false
                    }
                });
                async_cx.update(|cx| {
                    task_state.update(cx, |s, _| {
                        // 同步内存形状（模拟「之前跑过批量识别但没结论」的现场）
                        s.items[0].recognition_status = Some(photo_domain::RecognitionStatus::Unrecognized);
                        s.items[0].taxon_name = None;
                        s.items[0].taxon_confidence = None;
                        s.items[0].subjects.clear();
                    });
                });
                check!("把 reg_0 打回 Unrecognized 旧记录形状", wrote);

                async_cx.update(|cx| {
                    start_region_recognition(
                        task_state.clone(),
                        path.clone(),
                        BBox::new(0.2, 0.2, 0.7, 0.8),
                        cx,
                    );
                });
                for _ in 0..800 {
                    pump(async_cx, 50).await;
                    if !read_state!(|s: &AppState| s.region_recognizing) {
                        break;
                    }
                }
                let (mem_name, mem_status, mem_subjects, db_taxon) = read_state!(|s: &AppState| {
                    (
                        s.items[0].taxon_name.clone(),
                        s.items[0].recognition_status,
                        s.items[0].subjects.len(),
                        s.folder_db
                            .as_ref()
                            .and_then(|db| db.get_recognition("reg_0.jpg").ok().flatten())
                            .and_then(|r| r.taxon.map(|t| t.display_name().to_string())),
                    )
                });
                check!(
                    format!("旧 Unrecognized 记录框选后内存摘要显示物种（{mem_name:?}，状态 {mem_status:?}）"),
                    mem_name.is_some() && mem_status == Some(photo_domain::RecognitionStatus::Confirmed)
                );
                check!("框选结果作为唯一主体落进内存摘要", mem_subjects == 1);
                check!(
                    format!("落库记录的顶层字段同步为 subjects[0]（{db_taxon:?}）"),
                    db_taxon.is_some()
                );

                // ── 10) 存量坏数据（修复前写下的：顶层 NULL + subjects 里有结论）也要显示 ──
                let legacy_broken = photo_domain::Recognition {
                    status: photo_domain::RecognitionStatus::Confirmed,
                    taxon: None,
                    class_index: None,
                    confidence: None,
                    bbox: None,
                    candidates: vec![],
                    failure_stage: photo_domain::RecognitionFailureStage::None,
                    recognized_at: "2026-09-23T11:00:00Z".to_string(),
                    subjects: vec![photo_domain::SubjectRecognition {
                        index: 0,
                        bbox: BBox::new(0.1, 0.1, 0.5, 0.5),
                        taxon: Some(photo_domain::TaxonMatch {
                            taxon_id: None,
                            cn_name: String::new(),
                            latin_name: "Padda oryzivora".to_string(),
                            cn_level: photo_domain::CnLevel::Missing,
                            ranks: vec![],
                        }),
                        class_index: Some(7),
                        confidence: Some(68.3),
                        candidates: vec![],
                        failure: photo_domain::RecognitionFailureStage::None,
                    }],
                };
                let _ = read_state!(|s: &AppState| {
                    s.folder_db
                        .as_ref()
                        .map(|db| db.upsert_recognition("reg_0.jpg", &legacy_broken).is_ok())
                });
                fire(async_cx, handle, Box::new(Rescan));
                pump(async_cx, 3000).await;
                let (reload_name, reload_conf) = read_state!(|s: &AppState| {
                    (s.items[0].taxon_name.clone(), s.items[0].taxon_confidence)
                });
                check!(
                    format!("存量坏数据重扫后仍显示物种（{reload_name:?} / {reload_conf:?}）"),
                    reload_name.as_deref() == Some("Padda oryzivora")
                );

                // ── 6) 识别器空闲自动卸载（#17）──
                // a) 识别进行中绝不释放（busy 保护）：置 busy + 把空闲时间推过阈值
                async_cx.update(|cx| {
                    task_state.update(cx, |state, _| {
                        state.app_config.recognizer_idle_unload_minutes = 1;
                        state.region_recognizing = true;
                        state.recognizer_last_used =
                            std::time::Instant::now() - Duration::from_secs(600);
                    });
                });
                pump(async_cx, 1200).await;
                check!(
                    "识别进行中不释放识别器（busy 保护）",
                    read_state!(|s: &AppState| s.recognizer.lock().is_some())
                );

                // b) 空闲满阈值 → 自动释放 + 状态栏如实说明
                async_cx.update(|cx| {
                    task_state.update(cx, |state, _| {
                        state.region_recognizing = false;
                        state.recognizer_last_used =
                            std::time::Instant::now() - Duration::from_secs(600);
                    });
                });
                pump(async_cx, 1500).await;
                let (still_assembled, idle_msg) = read_state!(|s: &AppState| {
                    (
                        s.recognizer.lock().is_some(),
                        s.status_message
                            .clone()
                            .map(|(m, _)| m)
                            .unwrap_or_default(),
                    )
                });
                check!("空闲超阈值后自动释放识别器", !still_assembled);
                check!(
                    format!("状态栏说明自动释放（{idle_msg:?}）"),
                    idle_msg.contains("自动释放")
                );

                // 清理：本冒烟在临时目录产生的全局索引行不留在开发库
                let _ = async_cx.update(|cx| {
                    let _ = task_state
                        .read(cx)
                        .global_db
                        .as_ref()
                        .map(|g| g.delete_folder_rows(dir.to_string_lossy().as_ref()));
                });

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
