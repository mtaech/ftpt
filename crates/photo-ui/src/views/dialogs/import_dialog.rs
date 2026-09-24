//! 导入弹窗（对应手册 §9.10 与 §10.3）——**顶部来源条 + 两列**：
//!
//! - 顶：来源条（可移动盘 / 浏览目录 / 重新扫描 / 仅添加不搬运）横铺在标题下方。
//!   来源是一次性决定（选完就去审阅），不需要常驻竖栏；原先的左栏竖排三张卡片，
//!   卡片下面大半是空白，还白占了 240px 把审阅网格挤窄。
//! - 左（主）：**审阅**（分诊条 + 类别过滤 + 全选/反选 + 缩略图网格，可放大看单张）
//! - 右：目标根目录 + 目录与命名 + 冲突策略 + 文件处理 + 计划统计/跳过明细 + 执行结果
//!
//! 计划**实时重算**：任何选项/勾选变化都排一次 300ms 去抖重算，没有「生成计划」按钮；
//! 点「开始导入」时若计划恰好过期，会先重算再自动续跑（用户不需要多点一次）。
//! 执行完成后不自动关窗：成功/跳过/失败逐条留在右栏，有失败可「重试失败项」。
//!
//! 遮罩点击与 Esc 都不关闭（有状态的扫描/计划流程），只走右上角 × 或「完成/取消」。

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Selectable as _, Sizable as _,
    StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::Input,
    scroll::Scrollbar,
    v_flex,
};
use gpui_kit::{
    AnyElement, Context, CursorStyle, IntoElement, MouseButton, Role, SharedString, Window, div,
    img, prelude::*, px, uniform_list,
};
use photo_engine::import::{ConflictPolicy, ImportMode, ImportSubfolder, SkipReason};

use crate::state::AppState;
use crate::state::engine_ops::defer_entity_action;
use crate::state::import;

/// 右栏固定宽度与主列内边距：审阅网格的可用宽度 = 弹窗宽 − 这些。
/// 与右栏的 `.w(px(..))`、审阅列的 `.p_3()` 一一对应，改一处必须改两处。
/// （来源选择已合并到顶部来源条，不再占横向宽度——见 render_source_bar）
const RIGHT_COL_W: f32 = 320.0;
/// 中列左右 padding（p_3 = 12 × 2）
const MIDDLE_PAD: f32 = 24.0;
/// 审阅格子的目标边长（实际按可用宽度摊平，见 review_grid_metrics）
const GRID_EDGE_HINT: f32 = 120.0;
/// 跳过明细里每个类别最多展示多少条（超出只留一行汇总，避免 5000 条把右栏撑爆；
/// 右栏是**非虚拟化**滚动区，塞几千个元素会实打实拖帧）
const SKIP_PREVIEW_LIMIT: usize = 40;
/// 失败/警告清单的展示上限（同理由；计数仍是真实值，「重试失败项」不受影响）
const ISSUE_PREVIEW_LIMIT: usize = 200;
/// 弹窗最小尺寸（再小，右栏 + 审阅列 + 顶部来源条三段就挤成一团）
const MIN_DIALOG_W: f32 = 900.0;
const MIN_DIALOG_H: f32 = 520.0;
/// 弹窗与窗口边缘的留白（可拖拽上限 = 窗口尺寸 − 这个值）
const DIALOG_MARGIN: f32 = 48.0;

pub fn render_import_dialog(
    state: &AppState,
    window: &mut Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement + use<> {
    // 尺寸来自配置（右下角拖拽可调），这里再按窗口夹一次：
    // 窗口比存下来的尺寸小时必须跟着缩，否则弹窗会被裁掉整条底栏。
    let viewport = window.viewport_size();
    let avail_w = (f32::from(viewport.width) - DIALOG_MARGIN).max(MIN_DIALOG_W);
    let avail_h = (f32::from(viewport.height) - DIALOG_MARGIN).max(MIN_DIALOG_H);
    let width = (state.app_config.import_dialog_width as f32).clamp(MIN_DIALOG_W, avail_w);
    let height = (state.app_config.import_dialog_height as f32).clamp(MIN_DIALOG_H, avail_h);
    let (review_cols, review_edge) = review_grid_metrics(width);

    div()
        .id("import-modal-overlay")
        .occlude()
        .absolute()
        .inset_0()
        .bg(gpui_kit::rgba(0x00000088))
        .flex()
        .items_center()
        .justify_center()
        // 拖拽调整大小：事件挂在**遮罩**上而不是卡片上——遮罩盖住整个窗口，
        // 指针跑出弹窗边界（缩小/放大时的常态）也不会中断拖拽
        .on_mouse_move(cx.listener(
            |state, event: &gpui_kit::MouseMoveEvent, window, cx| {
                let Some((start_x, start_y, start_w, start_h)) = state.import_resize_drag else {
                    return;
                };
                // 自愈：mouse_up 丢事件（指针移出窗口后松手）时左键已经不在按下态，
                // 这里顺手收尾，免得弹窗此后跟着鼠标乱变大小
                if event.pressed_button != Some(MouseButton::Left) {
                    state.import_resize_drag = None;
                    return;
                }
                let viewport = window.viewport_size();
                let max_w = (f32::from(viewport.width) - DIALOG_MARGIN).max(MIN_DIALOG_W);
                let max_h = (f32::from(viewport.height) - DIALOG_MARGIN).max(MIN_DIALOG_H);
                let width =
                    (start_w + (f32::from(event.position.x) - start_x)).clamp(MIN_DIALOG_W, max_w);
                let height =
                    (start_h + (f32::from(event.position.y) - start_y)).clamp(MIN_DIALOG_H, max_h);
                state.app_config.import_dialog_width = width.round() as u32;
                state.app_config.import_dialog_height = height.round() as u32;
                cx.notify();
            },
        ))
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(|state, _event: &gpui_kit::MouseUpEvent, _window, cx| {
                // 松手才落盘：拖拽过程每帧写配置既慢又容易写坏
                if state.import_resize_drag.take().is_some() {
                    state.save_config();
                    cx.notify();
                }
            }),
        )
        .child(
            v_flex()
                .relative()
                .w(px(width))
                .h(px(height))
                .bg(cx.theme().sidebar)
                .rounded(px(20.))
                .border_1()
                .border_color(cx.theme().border.opacity(0.6))
                .shadow(crate::theme::overlay_shadow())
                .overflow_hidden()
                .child(render_header(state, cx))
                .child(render_source_bar(state, cx))
                .child(
                    h_flex()
                        .w_full()
                        .flex_1()
                        .min_h_0()
                        .child(render_review_column(state, review_cols, review_edge, cx))
                        .child(render_options_column(state, cx)),
                )
                .child(render_footer(state, cx))
                .child(render_resize_grip(width, height, cx)),
        )
}

/// 审阅网格的列数与格子边长：按中列可用宽度摊平——弹窗拖大就多排几列，
/// 而不是把格子摊在原地留一大片空白；拖窄也不会把格子挤成一条。
fn review_grid_metrics(dialog_width: f32) -> (usize, f32) {
    let avail = (dialog_width - RIGHT_COL_W - MIDDLE_PAD).max(180.0);
    let inner = (avail - 16.0).max(120.0); // 网格容器左右各 8px padding
    let cols = ((inner / (GRID_EDGE_HINT + 8.0)).floor() as usize).clamp(2, 8);
    let gap_total = cols.saturating_sub(1) as f32 * 8.0;
    let edge = ((inner - gap_total) / cols as f32).clamp(72.0, 200.0);
    (cols, edge)
}

/// 右下角的拖拽把手（两条短横杠）：按下开始记录起点，位移在遮罩的 on_mouse_move 里算。
///
/// GPUI 的鼠标事件给的是**窗口坐标**，所以这里存的是窗口绝对坐标、用差值算——
/// 不依赖把手自己的 bounds（弹窗大小一帧一变，拿 bounds 换算反而容易漂）。
fn render_resize_grip(width: f32, height: f32, cx: &mut Context<AppState>) -> AnyElement {
    let color = cx.theme().muted_foreground;
    div()
        .id("import-resize-grip")
        .absolute()
        // 贴角 + 小尺寸：底栏主按钮距右缘 16px，把手只到 17px，几乎不重叠（不抢按钮点击）
        .right(px(3.))
        .bottom(px(3.))
        .w(px(14.))
        .h(px(14.))
        .flex()
        .items_end()
        .justify_end()
        .cursor(CursorStyle::ResizeUpRightDownLeft)
        .child(
            v_flex()
                .gap_1()
                .items_end()
                .child(div().w(px(10.)).h(px(2.)).rounded_full().bg(color))
                .child(div().w(px(6.)).h(px(2.)).rounded_full().bg(color)),
        )
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |state, event: &gpui_kit::MouseDownEvent, _window, cx| {
                // 起点用**当前显示尺寸**而不是配置里的值：窗口比配置小时渲染会夹小，
                // 拿配置值当起点会让弹窗在第一次移动时「跳」一下
                state.import_resize_drag = Some((
                    f32::from(event.position.x),
                    f32::from(event.position.y),
                    width,
                    height,
                ));
                cx.notify();
            }),
        )
        .into_any_element()
}

// ── 顶栏 ──────────────────────────────────────────────────────────────────

fn render_header(state: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let running = state.import.running;
    h_flex()
        .w_full()
        .h(px(48.))
        .flex_shrink_0()
        .items_center()
        .justify_between()
        .px_4()
        .border_b_1()
        .border_color(cx.theme().border)
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .font_semibold()
                        .text_sm()
                        .text_color(cx.theme().foreground)
                        .child("导入照片（SD / CFexpress / 外接盘 / 普通目录）"),
                )
                .when(state.import.planning, |this| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("计划重算中…"),
                    )
                }),
        )
        .child(
            Button::new("close-import")
                .ghost()
                .small()
                .icon(IconName::Close)
                .disabled(running)
                .on_click(cx.listener(|state, _, _, cx| {
                    state.active_dialog = None;
                    cx.notify();
                })),
        )
        .into_any_element()
}

// ── 顶部来源条（原左栏的「来源 / 当前来源 / 不搬运只看」合并成一行）──────────

/// 来源条：标题下方**一整行**（约 40px）——可移动盘 chip / 当前来源 / 三个动作。

/// 为什么这么矮：来源是**一次性决定**（选完就去审阅），不需要常驻竖栏、也不需要三张
/// 大卡片；说明性文字（「递归、不动文件」这类）收进按钮 tooltip，横向宽度整幅留给审阅。
fn render_source_bar(state: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let drives = state.import.drives.clone();
    let source_text = state
        .import
        .source
        .as_ref()
        .map(|p| p.to_string_lossy().to_string());
    let scanning = state.import.scanning;

    h_flex()
        .w_full()
        .flex_shrink_0()
        .items_center()
        .gap_2()
        .px_3()
        .py_2()
        .border_b_1()
        .border_color(cx.theme().border)
        // 可移动盘：chip；一个都没有就一句灰字（不再占一整张卡片）
        .child(if drives.is_empty() {
            div()
                .flex_shrink_0()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("未检测到可移动盘")
                .into_any_element()
        } else {
            h_flex()
                .min_w_0()
                .gap_1()
                .children(drives.into_iter().map(|drive| {
                    let path = drive.path.clone();
                    let label = drive.label.clone().unwrap_or_else(|| drive.path.clone());
                    let selected = source_text.as_deref() == Some(path.as_str());
                    let pick = path.clone();
                    let row_id = SharedString::from(format!("import-drive-{path}"));
                    h_flex()
                        .id(row_id)
                        .max_w(px(160.))
                        .px_2()
                        .py_1()
                        .rounded(cx.theme().radius)
                        .bg(if selected {
                            cx.theme().selection
                        } else {
                            cx.theme().background
                        })
                        .border_1()
                        .border_color(cx.theme().border)
                        .items_center()
                        .text_xs()
                        .cursor_pointer()
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_color(cx.theme().foreground)
                                .child(label),
                        )
                        .on_click(cx.listener(move |_state, _, _window, cx| {
                            let entity = cx.entity();
                            import::set_source_and_scan(entity, PathBuf::from(pick.clone()), cx);
                            cx.notify();
                        }))
                }))
                .into_any_element()
        })
        // 当前来源：占满中间，太长省略
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .truncate()
                .child(source_text.unwrap_or_else(|| "未选择来源".to_string())),
        )
        .child(
            Button::new("import-browse-source")
                .small()
                .secondary()
                .label("浏览")
                .tooltip("把某个目录设为导入来源")
                .disabled(scanning)
                .on_click(cx.listener(|_state, _, window, cx| {
                    import::pick_source(window, cx);
                })),
        )
        .child(
            Button::new("import-rescan-source")
                .small()
                .ghost()
                .label(if scanning { "扫描中…" } else { "重新扫描" })
                .tooltip("重新扫描当前来源")
                .disabled(scanning || state.import.source.is_none())
                .on_click(cx.listener(|_state, _, _window, cx| {
                    let entity = cx.entity();
                    import::rescan(entity, cx);
                })),
        )
        .child(
            Button::new("import-add-dir")
                .small()
                .ghost()
                .icon(IconName::FolderOpen)
                .label("仅添加并浏览")
                .tooltip("不搬运，直接把来源目录打开到网格里浏览（递归）")
                .disabled(scanning || state.import.source.is_none())
                .on_click(cx.listener(|_state, _, _window, cx| {
                    let entity = cx.entity();
                    import::open_source_in_grid(entity, cx);
                })),
        )
        .into_any_element()
}

// ── 中：审阅 ─────────────────────────────────────────────────────────────

fn render_review_column(
    state: &AppState,
    cols: usize,
    edge: f32,
    cx: &mut Context<AppState>,
) -> AnyElement {
    v_flex()
        .flex_1()
        .min_w_0()
        .h_full()
        .gap_2()
        .p_3()
        .child(render_triage_bar(state, cx))
        .child(render_review_toolbar(state, cx))
        .child(
            div()
                .flex_1()
                .min_h_0()
                .child(if state.import.review_loupe.is_some() {
                    render_loupe(state, cx)
                } else {
                    render_review_grid(state, cols, edge, cx)
                }),
        )
        .child(render_plan_summary(state, cx))
        .into_any_element()
}

/// 分诊条：新 / 已导入 / 冲突 / 源内重复 / 已过滤
fn render_triage_bar(state: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let stats = state.import.stats();
    let foreground = cx.theme().foreground;
    let warning = cx.theme().warning;
    let primary = cx.theme().primary;
    let muted = cx.theme().muted_foreground;

    v_flex()
        .w_full()
        .gap_1()
        .child(
            h_flex()
                .w_full()
                .gap_2()
                .flex_wrap()
                .child(stat_chip("新", stats.new, primary, cx))
                .child(stat_chip("已导入", stats.already_imported, muted, cx))
                .child(stat_chip(
                    "冲突",
                    stats.conflict,
                    if stats.conflict > 0 { warning } else { muted },
                    cx,
                ))
                .child(stat_chip("源内重复", stats.source_duplicate, muted, cx))
                .child(stat_chip("已过滤", stats.filtered, muted, cx))
                .when(stats.other > 0, |this| {
                    this.child(stat_chip("其他", stats.other, muted, cx))
                }),
        )
        .when_some(state.import.plan_note.clone(), |this, note| {
            this.child(
                div()
                    .text_size(px(10.))
                    .text_color(muted)
                    .child(note),
            )
        })
        .child(
            h_flex()
                .w_full()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(10.))
                        .text_color(foreground.opacity(0.7))
                        .child(match (&state.import.volume_id, state.import.resumed) {
                            (Some(id), 0) => format!("源卷 {id}（账本去重生效）"),
                            (Some(id), n) => {
                                format!("源卷 {id}；上次导入已完成 {n} 项，本次将跳过（续传）")
                            }
                            (None, 0) => "未识别源卷：账本去重关闭".to_string(),
                            (None, n) => format!("未识别源卷；上次导入已完成 {n} 项"),
                        }),
                )
                .when(state.import.resumed > 0, |this| {
                    this.child(
                        Button::new("import-discard-journal")
                            .ghost()
                            .small()
                            .label("忽略断点日志，全部重导")
                            .on_click(cx.listener(|_state, _, _window, cx| {
                                let entity = cx.entity();
                                import::discard_journal(entity, cx);
                            })),
                    )
                }),
        )
        .into_any_element()
}

/// 过滤 + 勾选工具条
fn render_review_toolbar(state: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let filter = state.import.filter;
    let selected = state.import.selected_count();
    let visible = state.import.visible_count();
    let preview = (state.import.preview_done, state.import.preview_total);

    v_flex()
        .w_full()
        .gap_2()
        .child(
            h_flex()
                .w_full()
                .gap_1()
                .flex_wrap()
                .child(flip_button(
                    "import-filter-photo",
                    format!("照片"),
                    filter.photos,
                    |import| import.filter.photos = !import.filter.photos,
                    cx,
                ))
                .child(flip_button(
                    "import-filter-raw",
                    format!("RAW"),
                    filter.raw,
                    |import| import.filter.raw = !import.filter.raw,
                    cx,
                ))
                .child(flip_button(
                    "import-filter-video",
                    format!("视频"),
                    filter.videos,
                    |import| import.filter.videos = !import.filter.videos,
                    cx,
                ))
                .child(flip_button(
                    "import-filter-pair",
                    format!("跳过与 RAW 同名的 JPEG"),
                    filter.skip_jpeg_with_raw,
                    |import| import.filter.skip_jpeg_with_raw = !import.filter.skip_jpeg_with_raw,
                    cx,
                )),
        )
        .child(
            h_flex()
                .w_full()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().foreground)
                        .child(format!("已选 {selected} / 可见 {visible}")),
                )
                .child(
                    Button::new("import-select-all")
                        .ghost()
                        .small()
                        .label("全选")
                        .on_click(cx.listener(|_state, _, _window, cx| {
                            let entity = cx.entity();
                            import::update_import(entity, cx, |import| import.select_all());
                        })),
                )
                .child(
                    Button::new("import-deselect-all")
                        .ghost()
                        .small()
                        .label("全不选")
                        .on_click(cx.listener(|_state, _, _window, cx| {
                            let entity = cx.entity();
                            import::update_import(entity, cx, |import| import.deselect_all());
                        })),
                )
                .child(
                    Button::new("import-invert")
                        .ghost()
                        .small()
                        .label("反选")
                        .on_click(cx.listener(|_state, _, _window, cx| {
                            let entity = cx.entity();
                            import::update_import(entity, cx, |import| import.invert_selection());
                        })),
                )
                .when(state.import.stats().already_imported > 0, |this| {
                    this.child(
                        div()
                            .text_size(px(10.))
                            .text_color(cx.theme().muted_foreground)
                            .child("半透明 = 已导入（本次会跳过）"),
                    )
                })
                .when(preview.1 > 0 && preview.0 < preview.1, |this| {
                    this.child(
                        div()
                            .text_size(px(10.))
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("缩略图 {}/{}", preview.0, preview.1)),
                    )
                }),
        )
        .into_any_element()
}

/// 审阅网格（虚拟化按行；缩略图未就绪显示占位并随生成进度自动刷新）
fn render_review_grid(
    state: &AppState,
    cols: usize,
    edge: f32,
    cx: &mut Context<AppState>,
) -> AnyElement {
    if state.import.source.is_none() {
        return hint_block("先选择来源（左栏），扫描完成后在这里审阅", cx);
    }
    if state.import.scanning {
        return hint_block("正在扫描来源…", cx);
    }
    if state.import.candidates.is_empty() {
        return hint_block("来源里没有可导入的照片", cx);
    }
    let indices: Arc<Vec<usize>> = Arc::new(
        state
            .import
            .visible_candidates()
            .into_iter()
            .map(|(i, _)| i)
            .collect(),
    );
    if indices.is_empty() {
        return hint_block("当前过滤条件下没有文件（试试勾上「照片 / RAW / 视频」）", cx);
    }

    // 计划里判为「已导入」的源文件：格子上打标（勾着也不会搬）。
    // 没有这层标记，用户会看到「明明勾了却没导入」——去重是引擎的决定，界面必须说出来。
    let already: std::collections::HashSet<PathBuf> = state
        .import
        .plan
        .as_ref()
        .map(|plan| {
            plan.skipped
                .iter()
                .filter(|entry| entry.category == SkipReason::AlreadyImported)
                .map(|entry| entry.path.clone())
                .collect()
        })
        .unwrap_or_default();

    let row_count = indices.len().div_ceil(cols);
    let scroll_handle = state.import_review_scroll.clone();
    let rows = indices.clone();

    div()
        .id("import-review-viewport")
        .size_full()
        .relative()
        .rounded(px(12.))
        .border_1()
        .border_color(cx.theme().border.opacity(0.6))
        .bg(cx.theme().background)
        .child(
            uniform_list(
                "import-review-rows",
                row_count,
                cx.processor(move |state, range, _window, cx| {
                    let mut out = Vec::new();
                    for row_idx in range {
                        let start = row_idx as usize * cols;
                        let end = (start + cols).min(rows.len());
                        let mut cells = Vec::new();
                        for pos in start..end {
                            let cand_index = rows[pos];
                            if let Some(cand) = state.import.candidates.get(cand_index) {
                                let selected = state.import.is_selected(cand_index);
                                let thumb = state
                                    .import_preview
                                    .as_ref()
                                    .and_then(|manager| import::thumb_path(manager, cand));
                                let name = cand
                                    .path
                                    .file_name()
                                    .map(|n| n.to_string_lossy().to_string())
                                    .unwrap_or_default();
                                let already_imported = already.contains(&cand.path);
                                cells.push(render_review_cell(
                                    cand_index,
                                    name,
                                    thumb,
                                    selected,
                                    already_imported,
                                    edge,
                                    cx,
                                ));
                            }
                        }
                        out.push(
                            h_flex()
                                .id(("import-review-row", row_idx as usize))
                                .role(Role::Row)
                                .w_full()
                                .gap(px(8.))
                                .pb(px(8.))
                                .children(cells),
                        );
                    }
                    out
                }),
            )
            .p(px(8.))
            .h_full()
            .track_scroll(&scroll_handle),
        )
        .child(Scrollbar::vertical(&scroll_handle))
        .into_any_element()
}

/// 单个审阅格子：缩略图 + 勾选角标 + 放大按钮
#[allow(clippy::too_many_arguments)]
fn render_review_cell(
    index: usize,
    name: String,
    thumb: Option<PathBuf>,
    selected: bool,
    already_imported: bool,
    edge: f32,
    cx: &mut Context<AppState>,
) -> AnyElement {
    let has_thumb = thumb.is_some();
    let border = if selected {
        cx.theme().primary
    } else {
        cx.theme().border
    };
    let check_bg = if selected {
        cx.theme().primary
    } else {
        cx.theme().popover
    };
    let primary_fg = cx.theme().primary_foreground;
    let selected_note = if selected { "（已勾选）" } else { "" };
    let already_note = if already_imported {
        "（已导入，本次计划会跳过）"
    } else {
        ""
    };
    let name_text = if already_imported {
        format!("{name} · 已导入")
    } else {
        name.clone()
    };

    v_flex()
        .id(SharedString::from(format!("import-cell-{index}")))
        .role(Role::GridCell)
        .aria_label(SharedString::from(format!("{name}{selected_note}{already_note}")))
        .aria_selected(selected)
        .w(px(edge))
        .flex_shrink_0()
        .gap_1()
        .child(
            div()
                .id(SharedString::from(format!("import-cell-image-{index}")))
                .relative()
                .w(px(edge))
                .h(px(edge))
                .rounded(px(8.))
                .border_2()
                .border_color(border)
                .overflow_hidden()
                .cursor_pointer()
                .bg(cx.theme().popover)
                // 已导入的半透明：不是禁用（可以取消勾选/放大看），只是别让用户以为它会再搬一遍
                .when(already_imported, |this| this.opacity(0.45))
                .when_some(thumb, |this, path| {
                    this.child(img(path).size_full().object_fit(gpui_kit::ObjectFit::Cover))
                })
                .when(!has_thumb, |this| {
                    this.flex().items_center().justify_center().child(
                        Icon::new(IconName::Frame)
                            .size(px(20.))
                            .text_color(cx.theme().muted_foreground),
                    )
                })
                // 勾选角标
                .child(
                    div()
                        .absolute()
                        .top(px(4.))
                        .left(px(4.))
                        .w(px(16.))
                        .h(px(16.))
                        .rounded_full()
                        .bg(check_bg)
                        .border_1()
                        .border_color(cx.theme().border)
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(selected, |this| {
                            this.child(Icon::new(IconName::Check).size(px(11.)).text_color(primary_fg))
                        }),
                )
                // 放大按钮（角标）
                .child(
                    div()
                        .id(SharedString::from(format!("import-cell-loupe-{index}")))
                        .absolute()
                        .top(px(4.))
                        .right(px(4.))
                        .w(px(16.))
                        .h(px(16.))
                        .rounded_full()
                        .bg(cx.theme().popover)
                        .border_1()
                        .border_color(cx.theme().border)
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .child(
                            Icon::new(IconName::Eye)
                                .size(px(11.))
                                .text_color(cx.theme().muted_foreground),
                        )
                        .on_click(cx.listener(move |_state, _, _window, cx| {
                            let entity = cx.entity();
                            defer_entity_action(cx, entity, move |entity, cx| {
                                import::open_loupe(entity, index, cx)
                            });
                        })),
                )
                .on_click(cx.listener(move |_state, _, _window, cx| {
                    let entity = cx.entity();
                    import::update_import(entity, cx, move |import| import.toggle_selection(index));
                })),
        )
        .child(
            div()
                .w_full()
                .truncate()
                .text_size(px(10.))
                .text_color(cx.theme().muted_foreground)
                .child(name_text),
        )
        .into_any_element()
}

/// 放大单张（2560 母版；生成中先显示占位）
fn render_loupe(state: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let index = state.import.review_loupe.unwrap_or(0);
    let name = state
        .import
        .candidates
        .get(index)
        .and_then(|c| c.path.file_name())
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let selected = state.import.is_selected(index);
    let thumb = state.import.loupe_thumb.clone();

    v_flex()
        .size_full()
        .gap_2()
        .child(
            h_flex()
                .w_full()
                .gap_2()
                .items_center()
                .child(
                    Button::new("import-loupe-back")
                        .ghost()
                        .small()
                        .label("返回网格")
                        .on_click(cx.listener(|_state, _, _window, cx| {
                            let entity = cx.entity();
                            import::close_loupe(entity, cx);
                        })),
                )
                .child(
                    Button::new("import-loupe-toggle")
                        .small()
                        .selected(selected)
                        .label(if selected { "取消勾选" } else { "勾选" })
                        .on_click(cx.listener(move |_state, _, _window, cx| {
                            let entity = cx.entity();
                            import::update_import(entity, cx, move |import| {
                                import.toggle_selection(index)
                            });
                        })),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .truncate()
                        .child(name),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_h_0()
                .w_full()
                .rounded(px(12.))
                .border_1()
                .border_color(cx.theme().border.opacity(0.6))
                .bg(cx.theme().background)
                .flex()
                .items_center()
                .justify_center()
                .when_some(thumb, |this, path| {
                    this.child(img(path).max_w_full().max_h_full().object_fit(gpui_kit::ObjectFit::Contain))
                })
                .when(state.import.loupe_thumb.is_none(), |this| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("正在生成放大预览…"),
                    )
                }),
        )
        .into_any_element()
}

/// 网格下方一行：本次会搬多少张、落到几个目录
fn render_plan_summary(state: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let text = match &state.import.plan {
        None => {
            if state.import.source.is_none() {
                "选好来源后自动生成计划".to_string()
            } else if state.import.dest_root.trim().is_empty() {
                "填好目标根目录后自动生成计划".to_string()
            } else {
                "计划计算中…".to_string()
            }
        }
        Some(plan) => format!(
            "本次将导入 {} 张 → {} 个目标目录；跳过 {} 个",
            plan.file_count(),
            plan.groups.len(),
            plan.skipped_count()
        ),
    };
    div()
        .w_full()
        .text_xs()
        .text_color(cx.theme().foreground)
        .child(text)
        .into_any_element()
}

fn hint_block(text: &str, cx: &mut Context<AppState>) -> AnyElement {
    div()
        .size_full()
        .rounded(px(12.))
        .border_1()
        .border_color(cx.theme().border.opacity(0.6))
        .bg(cx.theme().background)
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(text.to_string()),
        )
        .into_any_element()
}

// ── 右：目标与选项 ────────────────────────────────────────────────────────

fn render_options_column(state: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    crate::views::scroll_area::scroll_area_v(
        "import-options-scroll",
        &state.import_scroll,
        v_flex()
            .w_full()
            .gap_3()
            .child(render_target_card(state, cx))
            .child(render_naming_card(state, cx))
            .child(render_handling_card(state, cx))
            .child(render_plan_detail_card(state, cx))
            .child(render_result_card(state, cx))
            .when_some(state.import.error.clone(), |this, (message, _)| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().warning)
                        .child(message),
                )
            }),
    )
    .w(px(RIGHT_COL_W))
    .h_full()
    .flex_shrink_0()
    .border_l_1()
    .border_color(cx.theme().border)
    .p_3()
    .into_any_element()
}

fn render_target_card(state: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    card(cx)
        .child(card_title("目标根目录", cx))
        .child(
            h_flex()
                .w_full()
                .items_center()
                .gap_2()
                .when_some(state.import_dest_input.clone(), |this, input| {
                    this.child(div().flex_1().min_w_0().child(Input::new(&input).small().w_full()))
                })
                .child(
                    Button::new("import-pick-dest")
                        .small()
                        .secondary()
                        .label("选择…")
                        .on_click(cx.listener(|_state, _, window, cx| {
                            import::pick_dest(window, cx);
                        })),
                ),
        )
        .into_any_element()
}

/// 子目录模式 + 重命名模板 + 冲突策略 + 导入方式
fn render_naming_card(state: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let subfolder = state.import.subfolder;
    let policy = state.import.policy;
    let mode = state.import.mode;

    card(cx)
        .child(card_title("目录与命名", cx))
        .child(
            h_flex()
                .w_full()
                .gap_1()
                .flex_wrap()
                .child(flip_button(
                    "import-subfolder-none",
                    "不建".into(),
                    subfolder == ImportSubfolder::None,
                    |import| import.subfolder = ImportSubfolder::None,
                    cx,
                ))
                .child(flip_button(
                    "import-subfolder-dash",
                    "YYYY-MM-DD".into(),
                    subfolder == ImportSubfolder::DateDash,
                    |import| import.subfolder = ImportSubfolder::DateDash,
                    cx,
                ))
                .child(flip_button(
                    "import-subfolder-slash",
                    "YYYY/MM/DD".into(),
                    subfolder == ImportSubfolder::DateSlash,
                    |import| import.subfolder = ImportSubfolder::DateSlash,
                    cx,
                ))
                .child(flip_button(
                    "import-subfolder-compact",
                    "YYYYMMDD".into(),
                    subfolder == ImportSubfolder::DateCompact,
                    |import| import.subfolder = ImportSubfolder::DateCompact,
                    cx,
                )),
        )
        .child(
            h_flex()
                .w_full()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .w(px(48.))
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("重命名"),
                )
                .when_some(state.import_rename_input.clone(), |this, input| {
                    this.child(div().flex_1().min_w_0().child(Input::new(&input).small().w_full()))
                }),
        )
        .child(
            div()
                .text_size(px(10.))
                .text_color(cx.theme().muted_foreground)
                .child(rename_hint(state)),
        )
        .child(
            h_flex()
                .w_full()
                .gap_1()
                .items_center()
                .child(
                    div()
                        .w(px(48.))
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("冲突"),
                )
                .child(flip_button(
                    "import-policy-skip",
                    ConflictPolicy::Skip.label().into(),
                    policy == ConflictPolicy::Skip,
                    |import| import.policy = ConflictPolicy::Skip,
                    cx,
                ))
                .child(flip_button(
                    "import-policy-rename",
                    ConflictPolicy::Rename.label().into(),
                    policy == ConflictPolicy::Rename,
                    |import| import.policy = ConflictPolicy::Rename,
                    cx,
                ))
                .child(flip_button(
                    "import-policy-overwrite",
                    ConflictPolicy::Overwrite.label().into(),
                    policy == ConflictPolicy::Overwrite,
                    |import| import.policy = ConflictPolicy::Overwrite,
                    cx,
                )),
        )
        .child(
            h_flex()
                .w_full()
                .gap_1()
                .items_center()
                .child(
                    div()
                        .w(px(48.))
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("方式"),
                )
                .child(flip_button(
                    "import-mode-copy",
                    "复制（源保留）".into(),
                    mode == ImportMode::Copy,
                    |import| import.mode = ImportMode::Copy,
                    cx,
                ))
                .child(flip_button(
                    "import-mode-move",
                    "移动（源删除）".into(),
                    mode == ImportMode::Move,
                    |import| import.mode = ImportMode::Move,
                    cx,
                )),
        )
        .into_any_element()
}

/// 重命名模板的实时示例（拿第一张候选渲染一遍，别再让用户自己猜占位符）
fn rename_hint(state: &AppState) -> String {
    let template = state.import.rename_template.trim();
    if template.is_empty() {
        return "留空 = 保留原名；占位符 {name} {date} {seq}".to_string();
    }
    match state.import.candidates.first() {
        Some(cand) => {
            // 与引擎侧 render_target_name 同一套上下文：{date} 用的是「YYYY-MM-DD」原样
            // （导入侧不归一为 YYYYMMDD，示例必须和实际落地名字一致）
            let ctx = photo_engine::template::NameTemplateContext {
                name: cand
                    .path
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default(),
                species: None,
                date: Some(cand.date.clone()),
                camera: None,
                seq: 1,
            };
            let base = photo_engine::template::render_name_template(template, &ctx);
            let ext = cand
                .path
                .extension()
                .map(|e| e.to_string_lossy().to_string())
                .unwrap_or_default();
            format!("示例：{base}.{ext}")
        }
        None => format!("模板：{template}"),
    }
}

/// 文件处理：校验 + 完成后弹出源盘
fn render_handling_card(state: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    card(cx)
        .child(card_title("文件处理", cx))
        .child(flip_button(
            "import-verify",
            format!(
                "复制后校验大小（{}）",
                if state.import.verify { "开" } else { "关" }
            ),
            state.import.verify,
            |import| import.verify = !import.verify,
            cx,
        ))
        .child(flip_button(
            "import-eject",
            format!(
                "完成后弹出源盘（{}）",
                if state.import.eject_after { "开" } else { "关" }
            ),
            state.import.eject_after,
            |import| import.eject_after = !import.eject_after,
            cx,
        ))
        .when(!photo_engine::import::eject_supported(), |this| {
            this.child(
                div()
                    .text_size(px(10.))
                    .text_color(cx.theme().muted_foreground)
                    .child("当前平台不支持自动弹出，请手动安全移除"),
            )
        })
        .into_any_element()
}

/// 计划明细：跳过清单按类别分组（不截断到 20 条；单类别只预览前 100 条）
fn render_plan_detail_card(state: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let Some(plan) = state.import.plan.as_ref() else {
        return card(cx)
            .child(card_title("计划明细", cx))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("计划生成后，这里按「已导入 / 冲突 / 源内重复 / 已过滤」分组列出每个文件"),
            )
            .into_any_element();
    };

    let mut body = card(cx).child(card_title("计划明细", cx));
    let categories = [
        SkipReason::AlreadyImported,
        SkipReason::Conflict,
        SkipReason::SourceDuplicate,
        SkipReason::Filtered,
        SkipReason::Other,
    ];
    for category in categories {
        let entries: Vec<&photo_engine::import::ImportSkipped> = plan
            .skipped
            .iter()
            .filter(|entry| entry.category == category)
            .collect();
        if entries.is_empty() {
            continue;
        }
        body = body.child(
            div()
                .pt_1()
                .text_size(px(10.))
                .text_color(if category == SkipReason::Conflict {
                    cx.theme().warning
                } else {
                    cx.theme().muted_foreground
                })
                .child(format!("{} {} 个", category.label(), entries.len())),
        );
        body = body.child(v_flex().w_full().gap_1().children(
            entries.iter().take(SKIP_PREVIEW_LIMIT).map(|entry| {
                let name = entry
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| entry.path.to_string_lossy().to_string());
                v_flex()
                    .w_full()
                    .gap_1()
                    .child(
                        div()
                            .w_full()
                            .truncate()
                            .text_size(px(10.))
                            .text_color(cx.theme().foreground)
                            .child(name),
                    )
                    .child(
                        div()
                            .w_full()
                            .truncate()
                            .text_size(px(10.))
                            .text_color(cx.theme().muted_foreground)
                            .child(entry.reason.clone()),
                    )
            }),
        ));
        if entries.len() > SKIP_PREVIEW_LIMIT {
            body = body.child(
                div()
                    .text_size(px(10.))
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("…等共 {} 个", entries.len())),
            );
        }
    }
    body.into_any_element()
}

/// 执行结果：汇总 + 失败逐条 + 警告逐条
fn render_result_card(state: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let Some(result) = state.import.result.clone() else {
        return v_flex().into_any_element();
    };
    let summary = if result.cancelled {
        format!(
            "已取消：成功 {} · 跳过 {} · 失败 {} · 未处理 {}",
            result.imported,
            result.skipped,
            result.failed.len(),
            result.unprocessed
        )
    } else {
        format!(
            "完成：成功 {} · 跳过 {} · 失败 {}",
            result.imported,
            result.skipped,
            result.failed.len()
        )
    };
    let mut body = card(cx)
        .child(card_title("导入结果", cx))
        .child(div().text_xs().text_color(cx.theme().foreground).child(summary));

    if !result.failed.is_empty() {
        body = body.child(
            div()
                .text_size(px(10.))
                .text_color(cx.theme().warning)
                .child(format!("失败 {} 个：", result.failed.len())),
        );
        body = body.child(v_flex().w_full().gap_1().children(
            result.failed.iter().take(ISSUE_PREVIEW_LIMIT).map(|issue| {
                v_flex()
                    .w_full()
                    .gap_1()
                    .child(
                        div()
                            .w_full()
                            .truncate()
                            .text_size(px(10.))
                            .text_color(cx.theme().foreground)
                            .child(issue.source.to_string_lossy().to_string()),
                    )
                    .child(
                        div()
                            .w_full()
                            .text_size(px(10.))
                            .text_color(cx.theme().warning)
                            .child(issue.reason.clone()),
                    )
            }),
        ));
    }
    if !result.warnings.is_empty() {
        body = body.child(
            div()
                .text_size(px(10.))
                .text_color(cx.theme().muted_foreground)
                .child(format!("警告 {} 条（文件已导入）：", result.warnings.len())),
        );
        body = body.child(v_flex().w_full().gap_1().children(
            result.warnings.iter().take(ISSUE_PREVIEW_LIMIT).map(|issue| {
                v_flex()
                    .w_full()
                    .gap_1()
                    .child(
                        div()
                            .w_full()
                            .truncate()
                            .text_size(px(10.))
                            .text_color(cx.theme().muted_foreground)
                            .child(issue.target.to_string_lossy().to_string()),
                    )
                    .child(
                        div()
                            .w_full()
                            .text_size(px(10.))
                            .text_color(cx.theme().muted_foreground)
                            .child(issue.reason.clone()),
                    )
            }),
        ));
    }
    body.into_any_element()
}

// ── 底栏 ─────────────────────────────────────────────────────────────────

fn render_footer(state: &AppState, cx: &mut Context<AppState>) -> AnyElement {
    let running = state.import.running;
    let has_result = state.import.result.is_some() && !running;
    let has_failures = state
        .import
        .result
        .as_ref()
        .is_some_and(|r| !r.failed.is_empty());

    let hint = if running {
        match state.import.progress.clone() {
            Some((done, total, current)) => {
                let name = PathBuf::from(&current)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or(current);
                format!("{done}/{total} · {name}")
            }
            None => "导入中…".to_string(),
        }
    } else if state.import.scanning {
        "正在扫描来源…".to_string()
    } else {
        "选源 → 审阅勾选 → 选目标 → 导入".to_string()
    };

    h_flex()
        .w_full()
        .h(px(56.))
        .flex_shrink_0()
        .items_center()
        .justify_between()
        .px_4()
        .border_t_1()
        .border_color(cx.theme().border)
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(hint),
        )
        .child(
            h_flex()
                .gap_2()
                // 执行中：取消 = 请求停止（剩余文件一个都不动）；空闲时：取消 = 关窗
                .child(
                    Button::new("import-cancel")
                        .ghost()
                        .small()
                        .label(if running {
                            if state.import.is_cancelling() {
                                "正在取消…"
                            } else {
                                "取消导入"
                            }
                        } else {
                            "取消"
                        })
                        .on_click(cx.listener(|state, _, _, cx| {
                            if state.import.running {
                                state.import.cancel.store(true, Ordering::Relaxed);
                            } else {
                                state.active_dialog = None;
                            }
                            cx.notify();
                        })),
                )
                .when(has_failures, |this| {
                    this.child(
                        Button::new("import-retry-failed")
                            .ghost()
                            .small()
                            .label("重试失败项")
                            .on_click(cx.listener(|_state, _, _window, cx| {
                                let entity = cx.entity();
                                defer_entity_action(cx, entity, |entity, cx| {
                                    import::retry_failed(entity, cx)
                                });
                            })),
                    )
                })
                .child(
                    Button::new("import-execute")
                        .small()
                        .primary()
                        .label(if running {
                            "导入中…".to_string()
                        } else if has_result {
                            "完成".to_string()
                        } else {
                            format!("开始导入（{} 张）", state.import.plan_count())
                        })
                        .disabled(running || (!has_result && !state.import.can_execute()))
                        .on_click(cx.listener(|state, _, _, cx| {
                            if state.import.result.is_some() && !state.import.running {
                                state.active_dialog = None;
                                cx.notify();
                                return;
                            }
                            let entity = cx.entity();
                            defer_entity_action(cx, entity, |entity, cx| {
                                import::execute(entity, cx)
                            });
                        })),
                ),
        )
        .into_any_element()
}

// ── 小部件 ───────────────────────────────────────────────────────────────

/// 小号卡片容器
fn card(cx: &mut Context<AppState>) -> gpui_kit::Div {
    let shadow = crate::theme::panel_card_shadow(cx);
    v_flex()
        .w_full()
        .p_3()
        .gap_2()
        .rounded(px(12.))
        .border_1()
        .border_color(cx.theme().border.opacity(0.6))
        .bg(cx.theme().popover)
        .shadow(shadow)
}

/// 卡片小标题
fn card_title(text: &'static str, cx: &mut Context<AppState>) -> gpui_kit::Div {
    div()
        .font_medium()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(text)
}

/// 分诊条上的一个小计数胶囊
fn stat_chip(label: &str, value: u32, color: gpui_kit::Hsla, cx: &mut Context<AppState>) -> AnyElement {
    h_flex()
        .gap_1()
        .px_2()
        .py_1()
        .rounded(px(8.))
        .bg(cx.theme().background)
        .border_1()
        .border_color(cx.theme().border)
        .text_xs()
        .child(
            div()
                .text_color(cx.theme().muted_foreground)
                .child(label.to_string()),
        )
        .child(div().text_color(color).child(value.to_string()))
        .into_any_element()
}

/// 开关型选项按钮（点击即取反 + 立即重算计划）
fn flip_button(
    id: &'static str,
    label: String,
    selected: bool,
    toggle: impl Fn(&mut import::ImportState) + 'static,
    cx: &mut Context<AppState>,
) -> Button {
    // Arc 包一层：listener 是 Fn，可能被多次调用，不能把 toggle 直接 move 出去
    let toggle = std::sync::Arc::new(toggle);
    Button::new(id)
        .small()
        .selected(selected)
        .label(label)
        .on_click(cx.listener(move |_state, _, _window, cx| {
            let entity = cx.entity();
            let toggle = toggle.clone();
            import::update_import(entity, cx, move |import| toggle(import));
        }))
}

#[cfg(test)]
mod tests {
    use super::review_grid_metrics;

    /// 审阅网格必须**正好填满**可用宽度（列数 × 边长 + 间距 == 可用宽 − 容器 padding）。
    ///
    /// 钉的是「可用宽度公式」与「列数/边长公式」同步：2026-09-25 把左栏来源合并成顶部
    /// 来源条时，available 里就得去掉 LEFT_COL_W——两边任一处忘了改，网格要么露一大片白、
    /// 要么溢出被裁。以后再加/减一列（右栏改宽、来源挪回左栏）同样先撞这条。
    #[test]
    fn test_review_grid_fills_available_width() {
        for dialog_width in [900.0f32, 1000.0, 1120.0, 1240.0, 1352.0, 1600.0] {
            let (cols, edge) = review_grid_metrics(dialog_width);
            let inner = dialog_width - super::RIGHT_COL_W - super::MIDDLE_PAD - 16.0;
            let gaps = cols.saturating_sub(1) as f32 * 8.0;
            let used = cols as f32 * edge + gaps;
            assert!(
                (used - inner).abs() < 0.51,
                "弹窗宽 {dialog_width}: {cols} 列 × {edge} + {gaps} 间距 = {used}，可用 {inner}"
            );
            assert!(cols >= 2, "两列是最小可读密度（宽 {dialog_width}）");
        }
    }
}
