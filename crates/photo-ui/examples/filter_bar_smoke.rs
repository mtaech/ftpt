//! 手动冒烟：筛选栏的下拉（排序方式 / 每行列数 / 镜头 / 物种）。
//!
//! 跑法：XDG_CONFIG_HOME=/tmp/pt-filter-config xvfb-run -a cargo run -p photo-ui --example filter_bar_smoke
//! 全部通过退出码 0；失败退出码 1。
//!
//! 背景：这两个控件原来是「点一下循环到下一个」的按钮（排序 7 档循环、列数 2→5 循环），
//! 按用户要求改成下拉。冒烟验证的是「下拉 → AppState」这条订阅链（emit 真实 SelectEvent，
//! 不是直接改字段），以及设置页改列数后下拉选中项会同步。
//!
//! 不需要素材目录：排序 / 列数是静态选项；镜头 / 物种的候选由注入的 items 驱动
//! （走真实的渲染期 defer_in → sync_filter_option_selects 路径，不直接调同步函数）。

use std::time::Duration;

use gpui_kit::component::combobox::ComboboxEvent;
use gpui_kit::component::searchable_list::SearchableVec;
use gpui_kit::component::select::SelectEvent;
use gpui_kit::{AppContext as _, Bounds, Point, SharedString, WindowBounds, WindowOptions, px, size};
use photo_domain::{CaptureMeta, SortBy};

use photo_ui::model::filter::has_active_filters;
use photo_ui::state::AppState;
use photo_ui::state::app_state::ChoiceOption;

/// 造一张最小摘要（只填筛选器用得到的字段：lens / 物种）
fn meta(base: &str, lens: Option<&str>, taxon: Option<&str>) -> CaptureMeta {
    let mut m = CaptureMeta::from_capture(
        &photo_domain::Capture {
            base_name: base.to_string(),
            source_files: Vec::new(),
            primary_index: 0,
        },
        0,
    );
    m.lens = lens.map(str::to_string);
    m.taxon_name = taxon.map(str::to_string);
    m
}

async fn pump(async_cx: &mut gpui_kit::AsyncApp, ms: u64) {
    async_cx
        .background_executor()
        .timer(Duration::from_millis(ms))
        .await;
}

fn main() {
    photo_ui::app::prepare_headless_smoke();

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

                let sort_sel = async_cx.update(|cx| task_state.read(cx).sort_select.clone());
                let cols_sel = async_cx.update(|cx| task_state.read(cx).grid_cols_select.clone());
                check!("排序下拉已创建", sort_sel.is_some());
                check!("列数下拉已创建", cols_sel.is_some());

                // 初始选中项应与状态一致（默认：文件名 / 4 列）
                let initial_sort = async_cx.update(|cx| {
                    task_state
                        .read(cx)
                        .sort_select
                        .as_ref()
                        .and_then(|s| s.read(cx).selected_value().cloned())
                });
                let initial_cols = async_cx.update(|cx| {
                    task_state
                        .read(cx)
                        .grid_cols_select
                        .as_ref()
                        .and_then(|s| s.read(cx).selected_value().cloned())
                });
                check!(
                    format!("排序下拉选中项与状态一致（{initial_sort:?}）"),
                    initial_sort.as_deref() == Some("file_name")
                );
                check!(
                    format!("列数下拉选中项与状态一致（{initial_cols:?}）"),
                    initial_cols.as_deref() == Some("4")
                );

                // 走真实事件路径：排序下拉 Confirm = 星级评分
                if let Some(sel) = sort_sel.clone() {
                    let _ = async_cx.update(|cx| {
                        sel.update(cx, |_s, cx| {
                            let event: SelectEvent<SearchableVec<ChoiceOption>> =
                                SelectEvent::Confirm(Some(SharedString::from("rating")));
                            cx.emit(event);
                        });
                    });
                }
                pump(async_cx, 250).await;
                check!(
                    "排序下拉 Confirm → sort_by 变成星级评分",
                    async_cx.update(|cx| task_state.read(cx).sort_by) == SortBy::Rating
                );

                // 列数下拉 Confirm = 3 列（顺带写回配置）
                if let Some(sel) = cols_sel.clone() {
                    let _ = async_cx.update(|cx| {
                        sel.update(cx, |_s, cx| {
                            let event: SelectEvent<SearchableVec<ChoiceOption>> =
                                SelectEvent::Confirm(Some(SharedString::from("3")));
                            cx.emit(event);
                        });
                    });
                }
                pump(async_cx, 250).await;
                let cols = async_cx.update(|cx| task_state.read(cx).grid_columns);
                let cfg_cols = async_cx.update(|cx| task_state.read(cx).app_config.grid_columns);
                check!(format!("列数下拉 Confirm → grid_columns = {cols}"), cols == 3);
                check!(format!("列数写回配置（{cfg_cols}）"), cfg_cols == 3);

                // 设置页那条路径：set_grid_columns + sync_grid_cols_select → 下拉跟着变
                let _ = async_cx.update(|cx| {
                    let _ = handle.update(cx, |_view, window, cx| {
                        task_state.update(cx, |state, cx| {
                            state.set_grid_columns(5, cx);
                            state.sync_grid_cols_select(window, cx);
                        });
                    });
                });
                pump(async_cx, 250).await;
                let synced = async_cx.update(|cx| {
                    task_state
                        .read(cx)
                        .grid_cols_select
                        .as_ref()
                        .and_then(|s| s.read(cx).selected_value().cloned())
                });
                check!(
                    format!("设置页改列数后下拉同步（{synced:?}）"),
                    synced.as_deref() == Some("5")
                );

                // ── 镜头 / 物种多选筛选器（§13.3）──
                // 注入 3 张照片（两张同镜头、两个物种），候选来自真实 items ——
                // 走渲染期的 defer_in → sync_filter_option_selects，而不是直接调同步函数
                async_cx.update(|cx| {
                    task_state.update(cx, |state, cx| {
                        state.items = vec![
                            meta("a", Some("RF24-70mm F2.8"), Some("大山雀")),
                            meta("b", Some("RF24-70mm F2.8"), Some("远东山雀")),
                            meta("c", Some("EF100-400mm"), Some("大山雀")),
                        ];
                        state.scan_generation = state.scan_generation.wrapping_add(1);
                        state.recompute_pipeline();
                        // 必须 notify：候选同步挂在筛选栏渲染上（defer_in），不重绘就不会跑
                        cx.notify();
                    });
                });
                pump(async_cx, 400).await;

                let (lens_sel, species_sel) = async_cx.update(|cx| {
                    let s = task_state.read(cx);
                    (s.lens_filter_select.clone(), s.species_filter_select.clone())
                });
                check!("镜头多选下拉已创建", lens_sel.is_some());
                check!("物种多选下拉已创建", species_sel.is_some());
                check!(
                    "渲染期自动同步候选项（世代已推进）",
                    async_cx.update(|cx| {
                        let s = task_state.read(cx);
                        s.filter_options_generation == s.scan_generation
                    })
                );

                // 走真实事件路径：Change = 勾选「RF24-70mm F2.8」（3 张里命中 2 张）
                if let Some(sel) = lens_sel.clone() {
                    let _ = async_cx.update(|cx| {
                        sel.update(cx, |_s, cx| {
                            let event: ComboboxEvent<SearchableVec<ChoiceOption>> =
                                ComboboxEvent::Change(vec![SharedString::from("RF24-70mm F2.8")]);
                            cx.emit(event);
                        });
                    });
                }
                pump(async_cx, 250).await;
                let (lens_filter, lens_hits, lens_active) = async_cx.update(|cx| {
                    let s = task_state.read(cx);
                    (
                        s.criteria.lens_filter.clone(),
                        s.display_order.len(),
                        has_active_filters(&s.criteria),
                    )
                });
                check!(
                    format!("镜头多选 Change → criteria 更新（{lens_filter:?}）"),
                    lens_filter == vec!["RF24-70mm F2.8".to_string()]
                );
                check!(
                    format!("镜头筛选真的生效（命中 {lens_hits} / 3 张）"),
                    lens_hits == 2
                );
                check!("多选筛选计入「已筛选」→ 批量操作安全边界随之解锁", lens_active);

                // 物种多选与镜头筛选取交集（b 是唯一同时命中的）
                if let Some(sel) = species_sel.clone() {
                    let _ = async_cx.update(|cx| {
                        sel.update(cx, |_s, cx| {
                            let event: ComboboxEvent<SearchableVec<ChoiceOption>> =
                                ComboboxEvent::Change(vec![SharedString::from("远东山雀")]);
                            cx.emit(event);
                        });
                    });
                }
                pump(async_cx, 250).await;
                let taxon_hits = async_cx.update(|cx| task_state.read(cx).display_order.len());
                check!(
                    format!("物种筛选与镜头取交集（命中 {taxon_hits} / 3 张）"),
                    taxon_hits == 1
                );

                // 清掉物种，单独验证镜头的多值语义
                if let Some(sel) = species_sel.clone() {
                    let _ = async_cx.update(|cx| {
                        sel.update(cx, |_s, cx| {
                            let event: ComboboxEvent<SearchableVec<ChoiceOption>> =
                                ComboboxEvent::Change(vec![]);
                            cx.emit(event);
                        });
                    });
                }
                pump(async_cx, 250).await;

                // 多值：Change 带两个镜头 → 命中 3 张（多选语义 = 命中任一即保留）
                if let Some(sel) = lens_sel.clone() {
                    let _ = async_cx.update(|cx| {
                        sel.update(cx, |_s, cx| {
                            let event: ComboboxEvent<SearchableVec<ChoiceOption>> =
                                ComboboxEvent::Change(vec![
                                    SharedString::from("RF24-70mm F2.8"),
                                    SharedString::from("EF100-400mm"),
                                ]);
                            cx.emit(event);
                        });
                    });
                }
                pump(async_cx, 250).await;
                let multi_hits = async_cx.update(|cx| {
                    let s = task_state.read(cx);
                    (s.criteria.lens_filter.len(), s.display_order.len())
                });
                check!(
                    format!("镜头多值 = 命中任一即保留（{multi_hits:?}）"),
                    multi_hits == (2, 3)
                );

                // 清空镜头（chip 的 × 走的就是这条：清 criteria + 清下拉选中集）
                if let Some(sel) = lens_sel.clone() {
                    let _ = async_cx.update(|cx| {
                        sel.update(cx, |_s, cx| {
                            let event: ComboboxEvent<SearchableVec<ChoiceOption>> =
                                ComboboxEvent::Change(vec![]);
                            cx.emit(event);
                        });
                    });
                }
                pump(async_cx, 250).await;
                let after_clear = async_cx.update(|cx| {
                    let s = task_state.read(cx);
                    (
                        s.criteria.lens_filter.is_empty() && s.criteria.taxon_names.is_empty(),
                        s.display_order.len(),
                    )
                });
                check!(
                    format!("清空两个多选筛选后回到 3 张（{after_clear:?}）"),
                    after_clear == (true, 3)
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
        .unwrap();
    });
}
