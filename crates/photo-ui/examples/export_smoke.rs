//! 手动冒烟：导出整条链路真的产出文件、参数真的生效、失败真的报错（无头，自建素材）。
//!
//! 跑法：
//!   XDG_CONFIG_HOME=/tmp/pt-export-config xvfb-run -a \
//!     cargo run -p photo-ui --example export_smoke
//! 全部通过退出码 0；失败退出码 1。配置目录由本冒烟自己钉进临时目录（PHOTO_CONFIG_DIR），
//! 不读写用户真实配置。
//!
//! 背景（docs/todo.md #1）：导出弹窗以前是「只报成功不产出文件」的假按钮，而且全仓没有
//! 任何地方设 `ActiveDialog::Export`（连入口都没有）。本冒烟覆盖接线后的完整链路：
//!
//!   1) 顶栏 / Ctrl+E 的 action 真的能打开导出弹窗
//!   2) 输入框文本 → 草稿（按钮实际走的就是 `read_export_inputs` 这条桥）
//!   3) 选中口径：扫描后自动选中 1 张 → 只导出这 1 张；取消选择 → 导出筛选结果全部 5 张
//!   4) 命名模板 `{seq}` 补零、无 `{seq}` 时同名自动加 `_1` 后缀
//!   5) 长边真的缩到 600；质量参数真的影响体积（85 < 95）
//!   6) 原文件字节不变；目标目录被记进配置
//!   7) 目标目录不可创建时给真实错误、**不是**报成功

use std::path::Path;
use std::time::Duration;

use gpui_kit::{AppContext as _, Bounds, Point, WindowBounds, WindowOptions, px, size};
use photo_domain::{
    CnLevel, Recognition, RecognitionFailureStage, RecognitionStatus, TaxonMatch,
};

use photo_ui::actions::{Export, Rescan};
use photo_ui::state::{ActiveDialog, AppState, engine_ops};

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

/// 生成一张带方向性纹理的 JPEG（不同 seed 让 DCT 结果不退化）。
fn write_jpeg(path: &Path, w: u32, h: u32, seed: u8) {
    let img = image::RgbImage::from_fn(w, h, |x, y| {
        let v = 40u8
            .saturating_add((x % 97) as u8)
            .saturating_add(((y + seed as u32 * 13) % 61) as u8);
        image::Rgb([v, v.wrapping_add(seed), v])
    });
    img.save(path).expect("写测试 JPEG 失败");
}

/// 目录里的 .jpg 文件名（升序）
fn jpg_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| {
                    e.path()
                        .extension()
                        .is_some_and(|x| x.eq_ignore_ascii_case("jpg"))
                })
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// 目录里所有 .jpg 的字节数之和（质量对比用）
fn jpg_bytes(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| {
                    e.path()
                        .extension()
                        .is_some_and(|x| x.eq_ignore_ascii_case("jpg"))
                })
                .filter_map(|e| std::fs::metadata(e.path()).ok())
                .map(|m| m.len())
                .sum()
        })
        .unwrap_or(0)
}

/// JPEG 长边像素
fn long_edge_of(path: &Path) -> u32 {
    let img = image::open(path).expect("读导出结果失败");
    img.width().max(img.height())
}

fn main() {
    photo_ui::app::prepare_headless_smoke();
    // 配置目录隔离：本冒烟会写 export_dir，绝不能碰用户真实配置
    let cfg_dir = std::env::temp_dir().join("pt_export_smoke_config");
    let _ = std::fs::remove_dir_all(&cfg_dir);
    std::fs::create_dir_all(&cfg_dir).expect("建隔离配置目录失败");
    // SAFETY: 在 GPUI 启动前、进程单线程时设置
    unsafe {
        std::env::set_var("PHOTO_CONFIG_DIR", &cfg_dir);
    }

    let dir = std::env::temp_dir().join("pt_export_smoke");
    let out1 = std::env::temp_dir().join("pt_export_smoke_out1");
    let out_all = std::env::temp_dir().join("pt_export_smoke_out_all");
    let out95 = std::env::temp_dir().join("pt_export_smoke_out95");
    let fail_dest = std::env::temp_dir().join("pt_export_smoke_fail_dest");
    for d in [&dir, &out1, &out_all, &out95] {
        let _ = std::fs::remove_dir_all(d);
        std::fs::create_dir_all(d).expect("建素材目录失败");
    }
    // 失败路径：目标是「一个文件」而不是目录 → create_dir_all 必失败
    let _ = std::fs::remove_dir_all(&fail_dest);
    std::fs::write(&fail_dest, b"not a dir").expect("写失败路径占位文件");

    let a = dir.join("alpha.jpg");
    let b = dir.join("beta.jpg");
    let c = dir.join("gamma.jpg");
    // 同 stem 不同扩展 → 模板不含 {seq} 时渲染出同名，靠 _1 后缀去重
    let dup_jpg = dir.join("dup.jpg");
    let dup_png = dir.join("dup.png");
    write_jpeg(&a, 1200, 800, 1);
    write_jpeg(&b, 1200, 800, 2);
    write_jpeg(&c, 900, 700, 3);
    write_jpeg(&dup_jpg, 1200, 800, 4);
    write_jpeg(&dup_png, 1200, 800, 5);
    let originals: Vec<(std::path::PathBuf, Vec<u8>)> = [&a, &b, &c, &dup_jpg, &dup_png]
        .iter()
        .map(|p| (p.to_path_buf(), std::fs::read(p).expect("读素材失败")))
        .collect();

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

                /// 把参数同步进真实输入框 + 草稿（点「开始导出」时走的就是这条桥）
                macro_rules! set_draft {
                    ($dest:expr, $template:expr, $edge:expr, $quality:expr) => {{
                        let dest_text = $dest.to_string();
                        let tpl_text = $template.to_string();
                        let _ = async_cx.update(|cx| {
                            let _ = handle.update(cx, |_view, window, cx| {
                                task_state.update(cx, |state, cx| {
                                    // 上一批结果清掉（回到参数态）
                                    state.export_results.clear();
                                    state.export.long_edge = $edge;
                                    state.export.quality = $quality;
                                    if let Some(input) = state.export_dest_input.clone() {
                                        let text = dest_text.clone();
                                        input.update(cx, |input_state, cx| {
                                            input_state.set_value(text, window, cx);
                                        });
                                    }
                                    if let Some(input) = state.export_template_input.clone() {
                                        let text = tpl_text.clone();
                                        input.update(cx, |input_state, cx| {
                                            input_state.set_value(text, window, cx);
                                        });
                                    }
                                    state.read_export_inputs(cx);
                                });
                            });
                        });
                    }};
                }

                /// 执行导出并等到收尾（与按钮同一条：start_export）
                macro_rules! run_export {
                    () => {{
                        async_cx.update(|cx| {
                            engine_ops::start_export(task_state.clone(), cx);
                        });
                        let mut waited = 0u64;
                        loop {
                            let running = async_cx.update(|cx| task_state.read(cx).is_exporting);
                            if !running || waited > 30_000 {
                                break;
                            }
                            pump(async_cx, 250).await;
                            waited += 250;
                        }
                    }};
                }

                pump(async_cx, 300).await;
                fire(async_cx, handle, Box::new(Rescan));
                pump(async_cx, 2500).await;
                let count = async_cx.update(|cx| task_state.read(cx).items.len());
                check!("扫描到 5 张素材", count == 5);
                if count == 0 {
                    eprintln!("FAIL: 扫描没有结果，后续检查无意义");
                    std::process::exit(1);
                }

                // ── 1) 导出入口：action 真的打开弹窗（以前全仓没人设 ActiveDialog::Export）──
                fire(async_cx, handle, Box::new(Export));
                pump(async_cx, 300).await;
                let dialog = async_cx.update(|cx| task_state.read(cx).active_dialog.clone());
                check!("顶栏 / Ctrl+E 的 action 打开导出弹窗", dialog == Some(ActiveDialog::Export));

                // ── 2) 真实输入框 → 草稿 ──
                set_draft!(out1.to_string_lossy(), "{name}_{seq}", Some(600), 85);
                let draft = async_cx.update(|cx| task_state.read(cx).export.clone());
                check!("目标目录输入框同步进草稿", draft.dest_dir == out1.to_string_lossy());
                check!("命名模板输入框同步进草稿", draft.template == "{name}_{seq}");
                check!("长边/质量进了草稿", draft.long_edge == Some(600) && draft.quality == 85);

                // ── 3) 已选口径：扫描完自动选中第 1 张 → 只导出这 1 张 ──
                let selected = async_cx.update(|cx| task_state.read(cx).selected_indices.len());
                check!("扫描后自动选中 1 张（导出只覆盖它）", selected == 1);
                run_export!();
                let (len1, ok1, status1) = async_cx.update(|cx| {
                    let s = task_state.read(cx);
                    (
                        s.export_results.len(),
                        s.export_results.iter().filter(|(_, r)| r.is_ok()).count(),
                        s.status_message.clone().map(|(m, _)| m),
                    )
                });
                let names1 = jpg_names(&out1);
                check!(
                    format!("已选口径只导出 1 张（结果 {len1} 条 / 文件 {names1:?}）"),
                    len1 == 1 && ok1 == 1 && jpg_names(&out1).len() == 1
                );
                check!(
                    "状态栏报真实张数（不是「已开始导出」）",
                    status1.as_deref() == Some("导出完成：1 张")
                );
                check!(
                    format!("模板 {{seq}} 补零渲染（{names1:?}）"),
                    names1.first().is_some_and(|n| n.ends_with("_001.jpg"))
                );
                check!(
                    "长边 600 真的生效（原 1200）",
                    names1
                        .iter()
                        .all(|n| long_edge_of(&out1.join(n)) == 600)
                );

                // ── 4) 未选口径：取消选择 → 导出筛选结果全部 5 张；模板无 {seq} 时同名去重 ──
                async_cx.update(|cx| {
                    task_state.update(cx, |state, _cx| state.deselect_all());
                });
                set_draft!(out_all.to_string_lossy(), "{name}", Some(600), 85);
                let scope = async_cx.update(|cx| {
                    let s = task_state.read(cx);
                    (s.selected_indices.len(), s.display_order.len())
                });
                check!("取消选择后目标 = 筛选结果全部", scope == (0, 5));
                run_export!();
                let (len_all, ok_all, status_all, still_open, remembered) = async_cx.update(|cx| {
                    let s = task_state.read(cx);
                    (
                        s.export_results.len(),
                        s.export_results.iter().filter(|(_, r)| r.is_ok()).count(),
                        s.status_message.clone().map(|(m, _)| m),
                        s.active_dialog.clone(),
                        s.app_config.export_dir.clone(),
                    )
                });
                let names_all = jpg_names(&out_all);
                check!(
                    format!("未选口径导出全部 5 张（{names_all:?}）"),
                    len_all == 5 && ok_all == 5 && names_all.len() == 5
                );
                check!(
                    "状态栏报 5 张",
                    status_all.as_deref() == Some("导出完成：5 张")
                );
                check!("导出完成后弹窗仍开着（结果可见）", still_open == Some(ActiveDialog::Export));
                check!(
                    "同 stem 不同扩展 → _1 去重后缀",
                    names_all.iter().any(|n| n == "dup.jpg")
                        && names_all.iter().any(|n| n == "dup_1.jpg")
                );
                check!(
                    "长边 600 对全量也生效",
                    names_all
                        .iter()
                        .all(|n| long_edge_of(&out_all.join(n)) == 600)
                );
                check!(
                    "目标目录被记进配置（#14 导出侧）",
                    remembered.as_deref() == Some(out_all.to_string_lossy().as_ref())
                );

                // ── 5) 质量参数真的影响体积（85 < 95） ──
                set_draft!(out95.to_string_lossy(), "{name}", Some(600), 95);
                run_export!();
                let (bytes85, bytes95) = (jpg_bytes(&out_all), jpg_bytes(&out95));
                check!(
                    format!("质量 95 的体积大于 85（{bytes95} vs {bytes85}）"),
                    bytes95 > bytes85
                );

                // ── 6) 原文件字节不变 ──
                let untouched = originals.iter().all(|(p, before)| {
                    std::fs::read(p).map(|now| now == *before).unwrap_or(false)
                });
                check!("原文件字节完全不变", untouched);

                // ── 7) 失败路径给真实错误（不能报成功） ──
                set_draft!(fail_dest.to_string_lossy(), "{name}", Some(600), 90);
                async_cx.update(|cx| {
                    engine_ops::start_export(task_state.clone(), cx);
                });
                pump(async_cx, 500).await;
                let (exporting, status_fail) = async_cx.update(|cx| {
                    let s = task_state.read(cx);
                    (s.is_exporting, s.status_message.clone().map(|(m, _)| m))
                });
                check!("目标目录不可创建时不启动导出", !exporting);
                check!(
                    format!("失败给真实错误文案（{status_fail:?}）"),
                    status_fail
                        .as_deref()
                        .is_some_and(|m| m.contains("导出失败") && m.contains("目标目录"))
                );

                // ── 8) eBird 记录导出（#9：统计页「导出记录 (CSV)」走同一条）──
                // 门控看内存摘要、导出读 folder_db，两处都要造
                let idx = async_cx.update(|cx| {
                    task_state
                        .read(cx)
                        .items
                        .iter()
                        .position(|m| m.base_name == "alpha")
                        .unwrap_or(0)
                });
                let rec = Recognition {
                    status: RecognitionStatus::Confirmed,
                    taxon: Some(TaxonMatch {
                        taxon_id: Some(42),
                        cn_name: "大嘴乌鸦".to_string(),
                        latin_name: "Corvus macrorhynchos".to_string(),
                        cn_level: CnLevel::Species,
                        ranks: vec![],
                    }),
                    class_index: Some(7),
                    confidence: Some(88.5),
                    bbox: None,
                    candidates: Vec::new(),
                    failure_stage: RecognitionFailureStage::None,
                    recognized_at: "2026-01-01T00:00:00Z".to_string(),
                    subjects: Vec::new(),
                };
                let staged = async_cx.update(|cx| {
                    task_state.update(cx, |s, _cx| {
                        let mem_ok = {
                            let m = &mut s.items[idx];
                            m.recognition_status = Some(RecognitionStatus::Confirmed);
                            m.taxon_name = Some("大嘴乌鸦".to_string());
                            true
                        };
                        let db_ok = s
                            .folder_db
                            .as_ref()
                            .map(|db| db.upsert_recognition("alpha.jpg", &rec).is_ok())
                            .unwrap_or(false);
                        mem_ok && db_ok
                    })
                });
                check!("造出一条已确认记录（内存摘要 + folder_db）", staged);
                let gate = async_cx.update(|cx| {
                    photo_ui::model::ebird::ebird_candidates(&task_state.read(cx).items)
                });
                check!(format!("eBird 门控放行（可导出 {gate} 张）"), gate >= 1);

                // 与统计页按钮背后同一条函数（无头下不点鼠标）
                async_cx.update(|cx| {
                    engine_ops::start_ebird_export(task_state.clone(), cx);
                });
                let mut waited = 0u64;
                loop {
                    let running = async_cx.update(|cx| task_state.read(cx).is_ebird_exporting);
                    if !running || waited > 15_000 {
                        break;
                    }
                    pump(async_cx, 100).await;
                    waited += 100;
                }
                let (ebird_status, csv_dir) = async_cx.update(|cx| {
                    let s = task_state.read(cx);
                    (
                        s.status_message.clone().map(|(m, _)| m),
                        s.app_config.export_dir.clone(),
                    )
                });
                let csv = csv_dir
                    .map(std::path::PathBuf::from)
                    .and_then(|d| {
                        std::fs::read_dir(&d).ok()?.flatten().find_map(|e| {
                            let p = e.path();
                            (p.extension().is_some_and(|x| x == "csv")).then_some(p)
                        })
                    });
                let csv_text = csv
                    .as_ref()
                    .and_then(|p| std::fs::read_to_string(p).ok())
                    .unwrap_or_default();
                check!("CSV 落盘在导出目录", csv.is_some());
                check!(
                    format!("状态栏报 CSV 结果（{ebird_status:?}）"),
                    ebird_status
                        .as_deref()
                        .is_some_and(|m| m.contains("已导出观鸟记录") && m.contains("条"))
                );
                check!(
                    "CSV 带 BOM + 表头（RFC 4180）",
                    csv_text.starts_with('\u{feff}')
                        && csv_text.contains("中文名,学名,数量,日期,纬度,经度,备注")
                );
                check!(
                    format!("CSV 含物种聚合行（{}）", csv_text.lines().nth(1).unwrap_or("")),
                    csv_text.contains("大嘴乌鸦") && csv_text.contains("Corvus macrorhynchos")
                );

                if failures == 0 {
                    println!("EXPORT_SMOKE_OK");
                    std::process::exit(0);
                } else {
                    eprintln!("EXPORT_SMOKE_FAILED: {failures} 项");
                    std::process::exit(1);
                }
            })
            .detach();
            state
        })
        .unwrap();
    });
}
