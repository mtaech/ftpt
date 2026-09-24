//! 手动冒烟：无障碍语义（§13.4「网格无表格语义」）——网格 / 胶片条 / 右键菜单。
//!
//! 跑法：
//!   XDG_CONFIG_HOME=/tmp/pt-a11y-config xvfb-run -a cargo run -p photo-ui --example a11y_smoke
//! 全部通过退出码 0；失败退出码 1。不需要模型。
//!
//! 手段：gpui-base 的 **ElementSnapshot**（gpui_kit::test::TestWindowExt::find）在元素
//! prepaint 时读**原生 a11y 属性**（role / aria_label / selected），与 pixels 无关、
//! 也与「有没有屏幕阅读器」无关——无头环境里 AccessKit 树不会真正下发（X11/Wayland
//! 后端要 AT-SPI 客户端接入才激活），所以这是能在 CI 里钉住语义的方式。
//!
//! 钉住：
//!   1) 网格容器 = Grid（带行列数与「N 张 / 已选 M 张」名称），行 = Row，格子 = GridCell
//!   2) 格子名称含文件名，且随状态变化（选中 / N 星 / Pick）实时更新
//!   3) 容器名称里的张数与选中数跟着状态走（不是写死的首帧快照）
//!   4) 胶片条 = List + ListItem（当前照片报 selected）
//!   5) 右键菜单 = Menu + MenuItem；置灰项名称明说「不可用」；键盘高亮项报 selected
//!   6) 文案组合的边界（缺项不补占位词）由 model::a11y 的单测覆盖，这里只钉接线

use std::path::Path;
use std::time::Duration;

use gpui_kit::{AppContext as _, Bounds, ElementId, Point, Role, WindowBounds, WindowOptions, px, size};
use photo_domain::{Flag, Rating};

use photo_ui::actions::Rescan;
use photo_ui::model::{filmstrip_label, grid_cell_label, grid_container_label};
use photo_ui::state::{AppState, ViewMode};

/// 从 ElementSnapshot 读回来的原生 a11y 事实（角色用 Debug 串比较，读起来直观）
#[derive(Debug, Clone)]
struct Probe {
    role: String,
    label: String,
    selected: Option<bool>,
}

fn probe(
    async_cx: &mut gpui_kit::AsyncApp,
    handle: gpui_kit::AnyWindowHandle,
    id: ElementId,
) -> Option<Probe> {
    async_cx.update(|cx| {
        handle
            .update(cx, |_view, window, _cx| {
                use gpui_kit::test::TestWindowExt as _;
                window.try_find(id).map(|s| Probe {
                    role: s.role().map(|r| format!("{r:?}")).unwrap_or_default(),
                    label: s.label().unwrap_or_default().to_string(),
                    selected: s.selected(),
                })
            })
            .ok()
            .flatten()
    })
}

fn role_of(probe: &Option<Probe>) -> String {
    probe.as_ref().map(|p| p.role.clone()).unwrap_or_default()
}

fn label_of(probe: &Option<Probe>) -> String {
    probe.as_ref().map(|p| p.label.clone()).unwrap_or_default()
}

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

const PHOTOS: usize = 3;
const NAMES: [&str; PHOTOS] = ["a11y_a.jpg", "a11y_b.jpg", "a11y_c.jpg"];

fn write_jpeg(path: &Path, seed: u8) {
    let img = image::RgbImage::from_fn(48, 36, |x, y| {
        image::Rgb([
            (x * 5 + seed as u32 * 19) as u8,
            (y * 7 + seed as u32 * 5) as u8,
            (seed as u32 * 41) as u8,
        ])
    });
    img.save(path).expect("写测试素材失败");
}

fn main() {
    photo_ui::app::prepare_headless_smoke();
    let dir = std::env::temp_dir().join("pt_a11y_smoke");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("建素材目录失败");
    for (i, name) in NAMES.iter().enumerate() {
        write_jpeg(&dir.join(name), i as u8 + 1);
    }

    let app = gpui_kit::application().with_assets(gpui_kit::assets::AllAssets);
    app.run(move |cx| {
        gpui_kit::component::init(cx);
        photo_ui::theme::apply(None, false, None, None, cx);
        AppState::register_keybindings(cx);

        let window_options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point { x: px(0.), y: px(0.) },
                size: size(px(1200.), px(800.)),
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
                macro_rules! probe {
                    ($id:expr) => {
                        probe(async_cx, handle, $id.into())
                    };
                }

                pump(async_cx, 300).await;
                fire(async_cx, handle, Box::new(Rescan));
                pump(async_cx, 2500).await;
                let count = read_state!(|s: &AppState| s.items.len());
                check!(format!("扫描到 {PHOTOS} 张素材"), count == PHOTOS);
                if count != PHOTOS {
                    eprintln!("FAIL: 素材不足，后续检查无意义");
                    std::process::exit(1);
                }

                // ── 1) 网格容器 / 行 / 格子：表格语义 ──
                pump(async_cx, 400).await;
                let grid = probe!("photo-grid-a11y");
                check!(
                    format!("网格容器 role = Grid（实测 {}）", role_of(&grid)),
                    role_of(&grid) == format!("{:?}", Role::Grid)
                );
                let (cols_cfg, sel_count) = read_state!(|s: &AppState| {
                    (s.grid_columns.clamp(2, 5) as usize, s.selected_indices.len())
                });
                check!(
                    format!("容器名称报张数/列数/选中数（{}）", label_of(&grid)),
                    label_of(&grid) == grid_container_label(PHOTOS, cols_cfg, sel_count)
                );

                let row0 = probe!(("grid-row", 0usize));
                check!(
                    format!("首行 role = Row（实测 {}）", role_of(&row0)),
                    role_of(&row0) == format!("{:?}", Role::Row)
                );

                let cell0 = probe!(("grid-cell", 0usize));
                let name0 = read_state!(|s: &AppState| s
                    .items
                    .first()
                    .map(|m| m.display_name())
                    .unwrap_or_default());
                check!(
                    format!("首格 role = GridCell（实测 {}）", role_of(&cell0)),
                    role_of(&cell0) == format!("{:?}", Role::GridCell)
                );
                // 扫描完成后首张是**默认选中**的（与视图里的当前照片一致）
                let default_selected = read_state!(|s: &AppState| s.selected_indices.contains(&0));
                check!(
                    format!("首格名称与状态一致（{}）", label_of(&cell0)),
                    label_of(&cell0)
                        == grid_cell_label(&name0, default_selected, Rating::None, None, None)
                );
                check!(
                    format!("扫描后默认选中首张（selected = {default_selected:?}）"),
                    default_selected
                        && cell0.as_ref().and_then(|p| p.selected) == Some(true)
                );

                // ── 2) 选中态迁移：第二张被选后，首格必须报 false ──
                async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| {
                        state.select_single(1);
                        cx.notify();
                    });
                });
                pump(async_cx, 400).await;
                let cell0 = probe!(("grid-cell", 0usize));
                let cell1 = probe!(("grid-cell", 1usize));
                check!(
                    "选中第二张后首格报 selected = false",
                    cell0.as_ref().and_then(|p| p.selected) == Some(false)
                );
                check!(
                    "第二格报 selected = true",
                    cell1.as_ref().and_then(|p| p.selected) == Some(true)
                );

                // ── 2) 选中 + 评分 + 旗标：名称与状态实时跟着走 ──
                async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| {
                        state.select_single(0);
                        if let Some(item) = state.items.first_mut() {
                            item.rating = Rating::Three;
                            item.flag = Some(Flag::Pick);
                        }
                        state.recompute_pipeline();
                        cx.notify();
                    });
                });
                pump(async_cx, 500).await;

                let cell0 = probe!(("grid-cell", 0usize));
                let label0 = label_of(&cell0);
                check!(
                    format!("选中后首格报 selected = true"),
                    cell0.as_ref().and_then(|p| p.selected) == Some(true)
                );
                check!(
                    format!("首格名称带 3 星 + Pick（{label0}）"),
                    label0.contains("已选中") && label0.contains("3 星") && label0.contains("标识为 Pick")
                );
                let grid = probe!("photo-grid-a11y");
                check!(
                    format!("容器名称的选中数跟着变（{}）", label_of(&grid)),
                    label_of(&grid) == grid_container_label(
                        PHOTOS,
                        read_state!(|s: &AppState| s.grid_columns.clamp(2, 5) as usize),
                        1
                    )
                );

                // ── 3) 胶片条（预览态）：List + ListItem ──
                async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| {
                        state.view_mode = ViewMode::Preview;
                        cx.notify();
                    });
                });
                pump(async_cx, 600).await;
                let strip = probe!("filmstrip-list");
                check!(
                    format!("胶片条 role = List（实测 {}）", role_of(&strip)),
                    role_of(&strip) == format!("{:?}", Role::List)
                );
                check!(
                    format!("胶片条名称报张数（{}）", label_of(&strip)),
                    label_of(&strip) == filmstrip_label(PHOTOS)
                );
                let idx0 = read_state!(|s: &AppState| s.display_order.first().copied().unwrap_or(0));
                let thumb = probe!(("filmstrip-thumb", idx0));
                check!(
                    format!("胶片条首项 role = ListItem（实测 {}）", role_of(&thumb)),
                    role_of(&thumb) == format!("{:?}", Role::ListItem)
                );
                check!(
                    "胶片条当前照片报 selected = true",
                    thumb.as_ref().and_then(|p| p.selected) == Some(true)
                );

                // ── 4) 右键菜单：Menu + MenuItem + 置灰项名称 ──
                let target = read_state!(|s: &AppState| s
                    .items
                    .first()
                    .map(|m| m.primary_path.clone())
                    .unwrap_or_default());
                async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| {
                        // 预览态下第一项「在预览中打开」应置灰
                        state.open_photo_context_menu(320.0, 320.0, target.clone(), cx);
                    });
                });
                pump(async_cx, 400).await;
                let card = probe!("context-menu-card");
                check!(
                    format!("菜单 role = Menu（实测 {}）", role_of(&card)),
                    role_of(&card) == format!("{:?}", Role::Menu)
                );
                let item0 = probe!("context-menu-item-0");
                check!(
                    format!("菜单项 role = MenuItem（实测 {}）", role_of(&item0)),
                    role_of(&item0) == format!("{:?}", Role::MenuItem)
                );
                check!(
                    format!("置灰项名称明说不可用（{}）", label_of(&item0)),
                    label_of(&item0) == "在预览中打开（不可用）"
                );

                // 网格态：键盘高亮要能被读屏感知（aria_selected）
                async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| {
                        state.view_mode = ViewMode::Grid;
                        state.open_photo_context_menu(320.0, 320.0, target.clone(), cx);
                        state.move_context_menu_selection(1);
                        cx.notify();
                    });
                });
                pump(async_cx, 400).await;
                let item0 = probe!("context-menu-item-0");
                let item1 = probe!("context-menu-item-1");
                check!(
                    format!("网格态首项可用（{}）", label_of(&item0)),
                    label_of(&item0) == "在预览中打开"
                );
                check!(
                    "键盘高亮项报 selected = true、未高亮项为 false",
                    item1.as_ref().and_then(|p| p.selected) == Some(true)
                        && item0.as_ref().and_then(|p| p.selected) == Some(false)
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
