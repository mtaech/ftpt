//! 手动冒烟：统计页右栏「选中物种 → 照片网格 → 点击跳转」（无头，自建素材，不碰真实配置）。
//!
//! 跑法：
//!   XDG_CONFIG_HOME=/tmp/pt-stats-config xvfb-run -a \
//!     cargo run -p photo-ui --example stats_smoke
//! 全部通过退出码 0；失败退出码 1。
//!
//! **不需要识别模型**：直接往全局索引（global.db）写测试行，验证统计页数据链路：
//!   1) 扫描素材 + 缩略图管线生成 .pt/thumbs
//!   2) 写入全局索引后 select_stats_species 取到该物种的照片记录
//!   3) 缩略图按**各自照片目录**解析到（跨文件夹缓存目录隔离）
//!   4) 统计视图可渲染（切 view_mode 跑几帧不 panic）
//!   5) 点击照片 → 同目录跳转选中正确
//!   6) 退出前清掉写入的全局索引行（不污染开发库）

use std::path::Path;
use std::time::Duration;

use gpui_kit::{AppContext as _, Bounds, Point, WindowBounds, WindowOptions, point, px, size};
use photo_engine::global_db::SpeciesRow;

use photo_ui::actions::Rescan;
use photo_ui::state::{AppState, ViewMode};
use photo_ui::state::engine_ops::{open_stats_photo, select_stats_species};

const PHOTOS: usize = 2;
const SPECIES: &str = "冒烟测试物种";

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

/// 320x240 的基线 JPEG（够小，缩略图管线毫秒级完成）
fn write_jpeg(path: &Path, rgb: (u8, u8, u8)) {
    let (r, g, b) = rgb;
    let img = image::RgbImage::from_fn(320, 240, |x, y| {
        image::Rgb([
            r.saturating_add((x % 40) as u8),
            g.saturating_add((y % 40) as u8),
            b,
        ])
    });
    img.save(path).expect("写测试 JPEG 失败");
}

fn main() {
    photo_ui::app::prepare_headless_smoke();

    let dir = std::env::temp_dir().join("pt_stats_smoke");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建素材目录失败");
    write_jpeg(&dir.join("st_0.jpg"), (200, 60, 60));
    write_jpeg(&dir.join("st_1.jpg"), (60, 60, 200));

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
                size: size(px(1280.), px(820.)),
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
                pump(async_cx, 1500).await;

                let count = read_state!(|s: &AppState| s.items.len());
                check!(format!("扫描到 {PHOTOS} 张素材"), count == PHOTOS);
                if count != PHOTOS {
                    eprintln!("FAIL: 素材不足，后续检查无意义");
                    std::process::exit(1);
                }

                // 全局索引不可用（缺 models/ 导致 data_root() 拿不到）就没法验证
                if read_state!(|s: &AppState| s.global_db.is_none()) {
                    eprintln!("跳过：global.db 未打开（需仓库根有 models/ 与 data/bird_catalog.db）");
                    std::process::exit(0);
                }

                // ── 1) 等缩略图管线把 .pt/thumbs 生成出来（按统计页同口径检查）──
                let mut thumbs_ready = false;
                for _ in 0..200 {
                    pump(async_cx, 50).await;
                    let hit = read_state!(|s: &AppState| {
                        let folder = s
                            .current_dir
                            .as_ref()
                            .map(|d| d.to_string_lossy().to_string())
                            .unwrap_or_default();
                        let mut cache = std::collections::HashMap::new();
                        s.items.iter().all(|m| {
                            let rel = std::path::Path::new(&m.primary_path)
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_default();
                            photo_ui::image::cached_thumb_path_in_folder(&mut cache, &folder, &rel)
                                .is_some()
                        })
                    });
                    if hit {
                        thumbs_ready = true;
                        break;
                    }
                }
                check!("缩略图管线已生成 .pt/thumbs（2/2 命中）", thumbs_ready);

                // ── 2) 往全局索引写该文件夹的两条记录 ──
                let folder_s = dir.to_string_lossy().to_string();
                let mk_row = |rel: &str, conf: f64| SpeciesRow {
                    folder: folder_s.clone(),
                    rel_path: rel.to_string(),
                    subject_index: 0,
                    species_name: SPECIES.to_string(),
                    confidence: Some(conf),
                    status: "confirmed".to_string(),
                    date_taken: None,
                    updated_at: "2026-09-22T10:00:00Z".to_string(),
                };
                let rows = vec![mk_row("st_0.jpg", 90.0), mk_row("st_1.jpg", 80.0)];
                let wrote = read_state!(|s: &AppState| s
                    .global_db
                    .as_ref()
                    .map(|g| g.upsert_rows(&rows).is_ok())
                    .unwrap_or(false));
                check!("全局索引写入 2 条记录", wrote);

                // ── 3) 选中物种 → 照片记录 + 缩略图解析 ──
                async_cx.update(|cx| {
                    task_state.update(cx, |s, cx| {
                        select_stats_species(s, SPECIES);
                        cx.notify();
                    });
                });
                let (sel, n_photos, total, n_thumb) = read_state!(|s: &AppState| {
                    (
                        s.stats_selected_species.clone(),
                        s.stats_photos.len(),
                        s.stats_photo_total,
                        s.stats_photos
                            .iter()
                            .filter(|p| p.thumb_path.is_some())
                            .count(),
                    )
                });
                check!(format!("选中物种 = {SPECIES}"), sel.as_deref() == Some(SPECIES));
                check!(
                    format!("照片记录 {n_photos} 条 / 总数 {total}"),
                    n_photos == PHOTOS && total == PHOTOS
                );
                check!(format!("缩略图解析到 {n_thumb}/{PHOTOS} 条"), n_thumb == PHOTOS);

                // ── 4) 统计视图可渲染（切过去跑几帧，不 panic 即过）──
                async_cx.update(|cx| {
                    task_state.update(cx, |s, cx| {
                        s.view_mode = ViewMode::Stats;
                        cx.notify();
                    });
                });
                pump(async_cx, 600).await;
                check!("统计视图渲染无 panic", true);

                // ── 5) 点击照片 → 同目录跳转选中 ──
                let target = dir.join("st_1.jpg").to_string_lossy().to_string();
                async_cx.update(|cx| {
                    open_stats_photo(
                        task_state.clone(),
                        folder_s.clone(),
                        "st_1.jpg".to_string(),
                        cx,
                    );
                });
                pump(async_cx, 300).await;
                let selected_path = read_state!(|s: &AppState| {
                    s.mark_indices()
                        .first()
                        .and_then(|&i| s.items.get(i))
                        .map(|m| m.primary_path.clone())
                });
                check!(
                    format!("点击缩略图跳转选中 st_1.jpg（实际 {selected_path:?}）"),
                    selected_path.as_deref() == Some(target.as_str())
                );

                // ── 5b) 滚动条（docs/todo.md #4）：统计页两处列表真的有滚动范围 ──
                // 右栏照片网格：补 34 条记录（文件不存在 → 占位 tile）撑出纵向溢出；
                // 左栏排行榜：补 30 个物种（各 1 条）。两条列表此前都是裸 overflow_y_scroll。
                let extra_photos: Vec<SpeciesRow> = (0..34)
                    .map(|i| mk_row(&format!("syn_{i}.jpg"), 70.0))
                    .collect();
                let extra_species: Vec<SpeciesRow> = (0..30)
                    .map(|i| {
                        let mut row = mk_row(&format!("sp_{i}.jpg"), 60.0);
                        row.species_name = format!("冒烟物种{i:02}");
                        row
                    })
                    .collect();
                let wrote_extra = read_state!(|s: &AppState| s
                    .global_db
                    .as_ref()
                    .map(|g| g.upsert_rows(&extra_photos).is_ok()
                        && g.upsert_rows(&extra_species).is_ok())
                    .unwrap_or(false));
                check!("补写 34 张照片 + 30 个物种行", wrote_extra);
                async_cx.update(|cx| {
                    task_state.update(cx, |s, cx| {
                        select_stats_species(s, SPECIES);
                        // 上一步点击缩略图会切回网格（open_stats_photo 的语义），
                        // 统计页的滚动句柄只在该视图渲染时更新——断言前切回来
                        s.view_mode = ViewMode::Stats;
                        cx.notify();
                    });
                });
                pump(async_cx, 800).await;
                let (n_extra, species_max, species_vp, photos_max, photos_vp) =
                    read_state!(|s: &AppState| {
                        (
                            s.stats_photos.len(),
                            f32::from(s.stats_species_scroll.max_offset().y),
                            f32::from(s.stats_species_scroll.bounds().size.height),
                            f32::from(s.stats_photos_scroll.max_offset().y),
                            f32::from(s.stats_photos_scroll.bounds().size.height),
                        )
                    });
                check!(format!("右栏照片记录扩到 {n_extra} 条"), n_extra == PHOTOS + 34);
                check!(
                    format!("物种榜滚动区：视口高 {species_vp}、可滚 {species_max}"),
                    species_vp > 0.0 && species_max > 0.0
                );
                check!(
                    format!("照片网格滚动区：视口高 {photos_vp}、可滚 {photos_max}"),
                    photos_vp > 0.0 && photos_max > 0.0
                );
                // 写偏移 → 跨帧保留（滚动位置与滚动条同源，网格视图同款做法）
                async_cx.update(|cx| {
                    task_state.update(cx, |s, _cx| {
                        s.stats_photos_scroll.set_offset(point(px(0.), px(-120.)));
                    });
                });
                pump(async_cx, 300).await;
                let kept = read_state!(|s: &AppState| s.stats_photos_scroll.offset().y);
                check!(
                    format!("照片网格滚动偏移跨帧保留（{kept:?}）"),
                    kept <= px(-100.)
                );

                // ── 5c) 胶片条横向滚动条：补足素材后进预览，内容宽 > 视口宽 ──
                for i in 2..16 {
                    write_jpeg(&dir.join(format!("st_{i}.jpg")), (80, 160, 80));
                }
                fire(async_cx, handle, Box::new(Rescan));
                pump(async_cx, 1500).await;
                let item_count = read_state!(|s: &AppState| s.items.len());
                async_cx.update(|cx| {
                    task_state.update(cx, |s, cx| {
                        s.view_mode = ViewMode::Preview;
                        s.selected_indices = vec![0];
                        s.anchor_index = Some(0);
                        cx.notify();
                    });
                });
                pump(async_cx, 800).await;
                let (film_max, film_vp) = read_state!(|s: &AppState| {
                    (
                        f32::from(s.filmstrip_scroll.max_offset().x),
                        f32::from(s.filmstrip_scroll.bounds().size.width),
                    )
                });
                check!(
                    format!("胶片条横向可滚（{item_count} 张，视口宽 {film_vp}、可滚 {film_max}）"),
                    item_count >= 16 && film_vp > 0.0 && film_max > 0.0
                );

                // ── 5d) 导入弹窗内容滚动区接上了句柄 ──
                async_cx.update(|cx| {
                    let _ = handle.update(cx, |_view, window, cx| {
                        task_state.update(cx, |s, cx| s.open_import_dialog(window, cx));
                    });
                });
                pump(async_cx, 500).await;
                let import_vp = read_state!(|s: &AppState| {
                    f32::from(s.import_scroll.bounds().size.height)
                });
                check!(format!("导入弹窗内容滚动区拿到视口（{import_vp}）"), import_vp > 0.0);
                async_cx.update(|cx| {
                    task_state.update(cx, |s, cx| {
                        s.active_dialog = None;
                        cx.notify();
                    });
                });
                pump(async_cx, 200).await;

                // ── 6) 清理：清掉写入的全局索引行 ──
                let _ = async_cx.update(|cx| {
                    let _ = task_state
                        .read(cx)
                        .global_db
                        .as_ref()
                        .map(|g| g.delete_folder_rows(&folder_s));
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