//! 手动冒烟：筛选栏右侧两个下拉（排序方式 / 每行列数）。
//!
//! 跑法：XDG_CONFIG_HOME=/tmp/pt-filter-config xvfb-run -a cargo run -p photo-ui --example filter_bar_smoke
//! 全部通过退出码 0；失败退出码 1。
//!
//! 背景：这两个控件原来是「点一下循环到下一个」的按钮（排序 7 档循环、列数 2→5 循环），
//! 按用户要求改成下拉。冒烟验证的是「下拉 → AppState」这条订阅链（emit 真实 SelectEvent，
//! 不是直接改字段），以及设置页改列数后下拉选中项会同步。
//!
//! 不需要素材目录：筛选栏在网格态就会渲染，下拉是纯静态选项。

use std::time::Duration;

use gpui_kit::component::searchable_list::SearchableVec;
use gpui_kit::component::select::SelectEvent;
use gpui_kit::{AppContext as _, Bounds, Point, SharedString, WindowBounds, WindowOptions, px, size};
use photo_domain::SortBy;

use photo_ui::state::AppState;
use photo_ui::state::app_state::ChoiceOption;

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
