//! 顶层应用根视图实现（对应 §2 界面结构与 §5 交互契约）。
//!
//! 包含：
//! - 5 栏布局（左活动栏 48px + 左栏 200–480px + 中间主视图 + 右栏 200–480px + 右活动栏 48px）
//! - 自定义标题栏 (36px) 与顶栏 Header (44px)
//! - 底部状态栏 (24px)
//! - 模态弹窗系统（设置、导入、导出、查重、纠错、连拍选优、批量确认）
//! - 全局动作分发与快捷键（§5.4 Esc 优先级链与 §5.6 键位总表）

use gpui_kit::component::dock::DockPlacement;
use gpui_kit::component::searchable_list::{SearchableListDelegate as _, SearchableVec};
use gpui_kit::component::select::{SelectEvent, SelectState};
use gpui_kit::component::{ActiveTheme as _, h_flex, input::InputState, v_flex};
use gpui_kit::{
    App, Context, Decorations, IntoElement, KeyBinding, Render, SharedString, Window, div,
    prelude::*,
};
use photo_domain::{ColorLabel, Flag, Rating};

use crate::actions::*;
use crate::state::engine_ops::{
    defer_entity_action, set_color_label, set_flag, set_rating, start_recognition, start_scan,
};
use crate::state::{ActiveDialog, AppState, ViewMode};
use crate::views::dialogs::*;
use crate::views::*;

pub type AppRoot = AppState;

impl AppState {
    /// 从当前上下文构建应用状态；设置页的主题色输入框需要 Window 才能创建。
    pub fn build(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut state = Self::new();
        state.focus_handle = Some(cx.focus_handle());
        // 自定义主题色输入框：初值 = 当前 seed（非法/缺失时回退默认墨白）
        let initial = crate::theme::resolve_seed(state.app_config.accent_color.as_deref());
        let accent_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(initial)
                .placeholder("#RRGGBB")
        });
        state.accent_input = Some(accent_input);
        // 自定义字体输入框：初值 = 当前 font_family
        let initial_font = state.app_config.font_family.clone();
        let font_input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(initial_font)
                .placeholder("输入字体名称，如 Noto Sans SC")
        });
        state.font_input = Some(font_input);
        // 全局界面字体选择器：汇聚推荐预设与操作系统全部安装字体，支持中英文双向实时搜索过滤
        let font_options =
            crate::state::app_state::build_font_options(&state.app_config.font_family, cx);
        let initial_font_sh: SharedString = state.app_config.font_family.clone().into();
        let selected_ix = font_options.position(&initial_font_sh);
        let font_select = cx.new(|cx| {
            SelectState::new(font_options, selected_ix, window, cx).searchable(true)
        });
        let font_select_sub = cx.subscribe(
            &font_select,
            |this,
             _entity,
             event: &SelectEvent<SearchableVec<crate::state::app_state::ChoiceOption>>,
             cx| {
                if let SelectEvent::Confirm(Some(value)) = event {
                    let val_str = value.as_ref();
                    this.set_font_family(val_str, None, cx);
                    this.set_status_message(format!("全局字体已切换为 {val_str}"));
                    cx.notify();
                }
            },
        );
        state.font_select = Some(font_select);
        state._font_select_sub = Some(font_select_sub);

        // 筛选栏下拉：排序方式 / 每行列数（静态选项，确认即生效；换掉了原来的「点击循环」按钮）
        let sort_options = crate::state::app_state::choice_options(&crate::model::SORT_OPTIONS);
        let sort_value: SharedString = crate::model::sort_by_value(state.sort_by).into();
        let sort_ix = sort_options.position(&sort_value);
        let sort_select = cx.new(|cx| SelectState::new(sort_options, sort_ix, window, cx));
        let sort_select_sub = cx.subscribe(
            &sort_select,
            |this,
             _entity,
             event: &SelectEvent<SearchableVec<crate::state::app_state::ChoiceOption>>,
             cx| {
                if let SelectEvent::Confirm(Some(value)) = event {
                    this.set_sort_by(crate::model::sort_by_from_value(value.as_ref()), cx);
                }
            },
        );
        let cols_options = crate::state::app_state::choice_options(&crate::model::GRID_COL_OPTIONS);
        let cols_value: SharedString = state.grid_columns.to_string().into();
        let cols_ix = cols_options.position(&cols_value);
        let cols_select = cx.new(|cx| SelectState::new(cols_options, cols_ix, window, cx));
        let cols_select_sub = cx.subscribe(
            &cols_select,
            |this,
             _entity,
             event: &SelectEvent<SearchableVec<crate::state::app_state::ChoiceOption>>,
             cx| {
                if let SelectEvent::Confirm(Some(value)) = event {
                    this.set_grid_columns(crate::model::grid_columns_from_value(value.as_ref()), cx);
                }
            },
        );
        state.sort_select = Some(sort_select);
        state.grid_cols_select = Some(cols_select);
        state._sort_select_sub = Some(sort_select_sub);
        state._grid_cols_select_sub = Some(cols_select_sub);
        // 导入弹窗输入框：目标根目录 / 重命名模板
        state.import_dest_input =
            Some(cx.new(|cx| {
                InputState::new(window, cx).placeholder("/目标/根目录（不存在时自动创建）")
            }));
        state.import_rename_input = Some(cx.new(|cx| {
            InputState::new(window, cx).placeholder("{name}_{date}_{seq}（留空 = 保留原名）")
        }));
        // 导出弹窗输入框：目标目录 / 命名模板
        state.export_dest_input = Some(cx.new(|cx| {
            InputState::new(window, cx).placeholder("/导出目标目录（不存在时自动创建）")
        }));
        state.export_template_input = Some(cx.new(|cx| {
            InputState::new(window, cx).placeholder("{name}_{species}_{seq}（占位符见弹窗内说明）")
        }));
        // Dock 工作区：左右停靠区 + 中央主视图（建面板实体需要 window）
        state.init_dock(window, cx);
        state
    }

    /// 注册应用全局键位绑定（§5.6）
    pub fn register_keybindings(cx: &mut App) {
        cx.bind_keys(vec![
            // 导航与视图
            KeyBinding::new("escape", Escape, None),
            KeyBinding::new("left", Prev, None),
            KeyBinding::new("right", Next, None),
            KeyBinding::new("home", Home, None),
            KeyBinding::new("end", End, None),
            KeyBinding::new("q", PrevMember, None),
            KeyBinding::new("e", NextMember, None),
            KeyBinding::new("g", ToggleView, None),
            KeyBinding::new("s", Slideshow, None),
            KeyBinding::new("space", TogglePlay, None),
            KeyBinding::new("t", Stats, None),
            KeyBinding::new("m", Map, None),
            KeyBinding::new("=", ZoomIn, None),
            KeyBinding::new("-", ZoomOut, None),
            // 评分 0–5
            KeyBinding::new("0", Rate0, None),
            KeyBinding::new("1", Rate1, None),
            KeyBinding::new("2", Rate2, None),
            KeyBinding::new("3", Rate3, None),
            KeyBinding::new("4", Rate4, None),
            KeyBinding::new("5", Rate5, None),
            // 色标 6–9, ctrl-6
            KeyBinding::new("6", SetColorRed, None),
            KeyBinding::new("7", SetColorYellow, None),
            KeyBinding::new("8", SetColorGreen, None),
            KeyBinding::new("9", SetColorBlue, None),
            KeyBinding::new("ctrl-6", SetColorPurple, None),
            // 旗标 P, X, U
            KeyBinding::new("p", SetFlagPick, None),
            KeyBinding::new("x", SetFlagReject, None),
            KeyBinding::new("u", ClearFlag, None),
            // 连拍选优与删除
            KeyBinding::new("k", BurstCull, None),
            KeyBinding::new("delete", Delete, None),
            KeyBinding::new("backspace", Delete, None),
            // 识别与叠加
            KeyBinding::new("b", RecognizeSelected, None),
            KeyBinding::new("ctrl-b", RecognizeAllUnrecognized, None),
            KeyBinding::new("ctrl-shift-b", ReRecognizeAll, None),
            KeyBinding::new("v", ToggleBbox, None),
            KeyBinding::new("f", ToggleFocus, None),
            KeyBinding::new("o", ToggleClipping, None),
            // 选择与面板与系统
            KeyBinding::new("ctrl-a", SelectAll, None),
            KeyBinding::new("ctrl-d", DeselectAll, None),
            KeyBinding::new("ctrl-z", Undo, None),
            // 剪贴板：复制当前照片（文本框内的 Ctrl+C 由 Input 自己处理，优先于本绑定）
            KeyBinding::new("ctrl-c", CopyImage, None),
            KeyBinding::new("ctrl-[", ToggleLeftPanel, None),
            KeyBinding::new("ctrl-]", ToggleRightPanel, None),
            KeyBinding::new("f5", Rescan, None),
            KeyBinding::new("ctrl-o", OpenDirectory, None),
            KeyBinding::new("ctrl-e", Export, None),
            KeyBinding::new("ctrl-,", OpenSettings, None),
        ]);
    }
}

/// 无头冒烟入口：把 GPUI 的后端**钉在 X11**（`xvfb-run` 提供的那个显示）上。
///
/// GPUI 选后端只看环境变量（`platform::guess_compositor()`）：`WAYLAND_DISPLAY` 非空优先
/// Wayland，其次 `DISPLAY`（X11），都没有才 Headless。而 `xvfb-run` 只准备 X 显示——
/// 在 Wayland 会话里跑冒烟（`WAYLAND_DISPLAY` 是继承来的）时窗口会落到**用户真实桌面**，
/// 帧由真实合成器决定（遮挡/最小化时干脆不产帧）：渲染驱动的行为（预览母版、缩略图）
/// 就随环境时好时坏，同一份代码一会儿 13/13 一会儿 12/13。
///
/// 所以冒烟在 `gpui_kit::application()` 之前调本函数：把 `WAYLAND_DISPLAY` 置空
/// （GPUI 判空即视为未设置），强制走 Xvfb。
/// 需要真机 Wayland 目检时设 `PHOTO_SMOKE_ALLOW_WAYLAND=1` 跳过。
pub fn prepare_headless_smoke() {
    if std::env::var_os("PHOTO_SMOKE_ALLOW_WAYLAND").is_some() {
        return;
    }
    // SAFETY: 只在 main() 最开头、GPUI 启动前调用（此时进程还是单线程），
    // 之后不再有任何线程读环境变量。
    unsafe {
        std::env::set_var("WAYLAND_DISPLAY", "");
    }
}

/// 首帧后把焦点交给根视图。
///
/// GPUI 的 `Window::dispatch_action` 是「从当前焦点节点向上冒泡」：窗口里没有任何
/// 焦点节点时动作会被直接丢弃——表现就是所有按钮/快捷键失灵（日志里可能伴随
/// "window not found"。根视图 render 里已经 `track_focus`，这里等首帧元素注册完成
/// 后再聚焦它。
pub fn focus_root(view: &gpui_kit::Entity<AppState>, window: &mut Window, cx: &mut App) {
    let Some(handle) = view.read(cx).focus_handle.clone() else {
        return;
    };
    window.on_next_frame(move |window, cx| handle.focus(window, cx));
}

impl Render for AppState {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .font_family(cx.theme().font_family.clone())
            .when_some(self.focus_handle.clone(), |this, handle| {
                this.track_focus(&handle)
            })
            // ── 键位动作绑定 ──
            .on_action(cx.listener(|this, _: &Escape, _window, cx| {
                if !this.handle_escape() {
                    if this.selected_indices.len() > 1 {
                        if let Some(&first) = this.selected_indices.first() {
                            this.select_single(first);
                        }
                    }
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Prev, _window, cx| {
                match this.view_mode {
                    ViewMode::Slideshow => {
                        if !this.display_order.is_empty() {
                            if this.slideshow_pos > 0 {
                                this.slideshow_pos -= 1;
                            } else {
                                this.slideshow_pos = this.display_order.len().saturating_sub(1);
                            }
                        }
                    }
                    _ => {
                        this.navigate_stack_group(-1);
                    }
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Next, _window, cx| {
                match this.view_mode {
                    ViewMode::Slideshow => {
                        if !this.display_order.is_empty() {
                            this.slideshow_pos =
                                (this.slideshow_pos + 1) % this.display_order.len();
                        }
                    }
                    _ => {
                        this.navigate_stack_group(1);
                    }
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Home, _window, cx| {
                if let Some(&first) = this.display_order.first() {
                    this.select_single(first);
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &End, _window, cx| {
                if let Some(&last) = this.display_order.last() {
                    this.select_single(last);
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &PrevMember, _window, cx| {
                if this.view_mode == ViewMode::Grid {
                    this.navigate_intra_stack(-1);
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &NextMember, _window, cx| {
                if this.view_mode == ViewMode::Grid {
                    this.navigate_intra_stack(1);
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleView, _window, cx| {
                match this.view_mode {
                    ViewMode::Grid => {
                        if this.primary_selected_meta().is_some() {
                            this.view_mode = ViewMode::Preview;
                            this.preview_zoom = 1.0;
                            this.preview_pan = (0.0, 0.0);
                            this.preview_drag_start = None;
                        }
                    }
                    ViewMode::Preview => {
                        this.view_mode = ViewMode::Grid;
                    }
                    _ => {
                        this.view_mode = ViewMode::Grid;
                    }
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Slideshow, _window, cx| {
                if this.view_mode == ViewMode::Slideshow {
                    this.view_mode = this.view_before_slideshow;
                } else if !this.display_order.is_empty() {
                    this.view_before_slideshow = this.view_mode;
                    let cur = this.primary_selected_index().unwrap_or(0);
                    let pos = this
                        .display_order
                        .iter()
                        .position(|&x| x == cur)
                        .unwrap_or(0);
                    this.slideshow_pos = pos;
                    this.slideshow_paused = false;
                    this.view_mode = ViewMode::Slideshow;
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &TogglePlay, _window, cx| {
                if this.view_mode == ViewMode::Slideshow {
                    this.slideshow_paused = !this.slideshow_paused;
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &Stats, _window, cx| {
                if this.view_mode == ViewMode::Stats {
                    this.view_mode = ViewMode::Grid;
                } else {
                    this.view_mode = ViewMode::Stats;
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Map, _window, cx| {
                this.map_overlay = !this.map_overlay;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ZoomIn, _window, cx| {
                if this.view_mode == ViewMode::Preview {
                    let cur_zoom = if this.preview_zoom == 0.0 { 1.0 } else { this.preview_zoom };
                    this.preview_zoom = (cur_zoom * 1.25).clamp(0.1, 10.0);
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &ZoomOut, _window, cx| {
                if this.view_mode == ViewMode::Preview {
                    let cur_zoom = if this.preview_zoom == 0.0 { 1.0 } else { this.preview_zoom };
                    this.preview_zoom = (cur_zoom / 1.25).clamp(0.1, 10.0);
                    cx.notify();
                }
            }))
            // ── 评分 ──
            .on_action(cx.listener(|this, _: &Rate0, _window, cx| {
                set_rating(this, Rating::None);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Rate1, _window, cx| {
                set_rating(this, Rating::One);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Rate2, _window, cx| {
                set_rating(this, Rating::Two);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Rate3, _window, cx| {
                set_rating(this, Rating::Three);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Rate4, _window, cx| {
                set_rating(this, Rating::Four);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Rate5, _window, cx| {
                set_rating(this, Rating::Five);
                cx.notify();
            }))
            // ── 色标 ──
            .on_action(cx.listener(|this, _: &SetColorRed, _window, cx| {
                set_color_label(this, ColorLabel::Red);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &SetColorYellow, _window, cx| {
                set_color_label(this, ColorLabel::Yellow);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &SetColorGreen, _window, cx| {
                set_color_label(this, ColorLabel::Green);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &SetColorBlue, _window, cx| {
                set_color_label(this, ColorLabel::Blue);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &SetColorPurple, _window, cx| {
                set_color_label(this, ColorLabel::Purple);
                cx.notify();
            }))
            // ── 旗标 ──
            .on_action(cx.listener(|this, _: &SetFlagPick, _window, cx| {
                set_flag(this, Some(Flag::Pick));
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &SetFlagReject, _window, cx| {
                set_flag(this, Some(Flag::Reject));
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ClearFlag, _window, cx| {
                set_flag(this, None);
                cx.notify();
            }))
            // ── 连拍选优与删除 ──
            .on_action(cx.listener(|this, _: &BurstCull, _window, cx| {
                this.active_dialog = Some(ActiveDialog::BurstConfirm);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Delete, _window, cx| {
                // 与批量删除同一套口径：先确认再进回收站（§13：以前 Delete 键无确认直接删）
                if this.mark_indices().is_empty() {
                    return;
                }
                this.active_dialog = Some(ActiveDialog::DeleteConfirm);
                cx.notify();
            }))
            // ── 识别 ──
            .on_action(cx.listener(|_this, _: &RecognizeSelected, _window, cx| {
                let entity = cx.entity().clone();
                defer_entity_action(cx, entity, |entity, cx| {
                    start_recognition(entity, false, false, cx);
                });
            }))
            .on_action(
                cx.listener(|_this, _: &RecognizeAllUnrecognized, _window, cx| {
                    let entity = cx.entity().clone();
                    defer_entity_action(cx, entity, |entity, cx| {
                        start_recognition(entity, true, false, cx);
                    });
                }),
            )
            .on_action(cx.listener(|_this, _: &ReRecognizeAll, _window, cx| {
                let entity = cx.entity().clone();
                defer_entity_action(cx, entity, |entity, cx| {
                    start_recognition(entity, false, true, cx);
                });
            }))
            // ── 叠加与面板 ──
            .on_action(cx.listener(|this, _: &ToggleBbox, _window, cx| {
                this.show_bbox = !this.show_bbox;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleFocus, _window, cx| {
                this.show_focus = !this.show_focus;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleClipping, _window, cx| {
                this.show_clipping = !this.show_clipping;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleRegionSelect, _window, cx| {
                // 框选模式：关时清掉框与拖拽状态（开时保留进行中的框，继续画）
                this.region_select = !this.region_select;
                this.region_drag_start = None;
                if !this.region_select {
                    this.region_bbox = None;
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleLeftPanel, window, cx| {
                this.toggle_dock(DockPlacement::Left, window, cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleRightPanel, window, cx| {
                this.toggle_dock(DockPlacement::Right, window, cx);
                cx.notify();
            }))
            // ── 选择与系统 ──
            .on_action(cx.listener(|this, _: &SelectAll, _window, cx| {
                this.select_all();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &DeselectAll, _window, cx| {
                this.deselect_all();
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Rescan, _window, cx| {
                if let Some(dir) = this.current_dir.clone() {
                    let entity = cx.entity().clone();
                    defer_entity_action(cx, entity, move |entity, cx| {
                        start_scan(entity, dir, false, cx);
                    });
                }
            }))
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| {
                this.sync_accent_input(window, cx);
                this.sync_font_input(window, cx);
                this.sync_font_select(window, cx);
                this.active_dialog = Some(ActiveDialog::Settings);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Export, window, cx| {
                this.open_export_dialog(window, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleThemeMode, window, cx| {
                this.toggle_theme(Some(window), cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &Undo, _window, cx| {
                let outcomes = this.op_journal.undo_last();
                if !outcomes.is_empty() {
                    let success = outcomes.iter().filter(|o| o.result.is_ok()).count();
                    let failed = outcomes.len() - success;
                    let from_trash = outcomes
                        .iter()
                        .any(|o| matches!(o.op, photo_engine::undo::UndoOp::Trash { .. }));
                    let msg = if failed == 0 {
                        if from_trash {
                            format!("已从回收站恢复 {success} 项")
                        } else {
                            format!("撤销完成：{success} 项")
                        }
                    } else {
                        // 逐条原因只报第一条：状态栏只有一行，全部原因在日志里
                        let reason = outcomes
                            .iter()
                            .find_map(|o| match &o.result {
                                Err(e) => Some(e.to_string()),
                                Ok(()) => None,
                            })
                            .unwrap_or_default();
                        tracing::warn!("撤销部分失败: {reason}");
                        format!("撤销：成功 {success} 项、失败/跳过 {failed} 项（{reason}）")
                    };
                    this.set_status_message(msg);
                    if let Some(dir) = this.current_dir.clone() {
                        let recursive = this.app_config.include_subdirectories;
                        let entity = cx.entity().clone();
                        defer_entity_action(cx, entity, move |entity, cx| {
                            start_scan(entity, dir, recursive, cx);
                        });
                    }
                } else {
                    this.set_status_message("没有可撤销的批量操作");
                }
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &CopyImage, _window, cx| {
                this.copy_current_image_to_clipboard(cx);
            }))
            .on_action(cx.listener(|_this, _: &OpenDirectory, _window, cx| {
                cx.spawn(
                    |weak_entity: gpui_kit::WeakEntity<AppState>,
                     async_cx: &mut gpui_kit::AsyncApp| {
                        let async_app = async_cx.clone();
                        async move {
                            if let Some(folder) = rfd::AsyncFileDialog::new().pick_folder().await {
                                let path = folder.path().to_path_buf();
                                let _ = async_app.update(|cx| {
                                    if let Some(entity) = weak_entity.upgrade() {
                                        start_scan(entity, path, false, cx);
                                    }
                                });
                            }
                        }
                    },
                )
                .detach();
            }))
            // ── 1. 自绘标题栏（36px）：仅当窗口是客户端装饰时。
            // 系统已经画了标题栏（服务端装饰）就不要再画一条，避免两条标题栏。
            .when(
                matches!(window.window_decorations(), Decorations::Client { .. }),
                |this| this.child(render_title_bar(self, window, cx)),
            )
            // ── 2. 顶栏 Header (44px) ──
            .child(render_header(self, window, cx))
            // ── 3. 主体工作区（左活动栏 + Dock 工作区 + 右活动栏） ──
            .child(
                h_flex()
                    .w_full()
                    .flex_1()
                    .overflow_hidden()
                    // 左活动栏 48px（常驻，§9.2）
                    .child(render_left_activity_bar(self, window, cx))
                    // Dock 工作区：左停靠区（文件树/批量操作）+ 中央主视图 + 右停靠区（信息/调整）
                    // 边缘可拖宽、Ctrl+[ / Ctrl+] 折叠显隐，宽度持久化到配置
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .overflow_hidden()
                            .when_some(self.dock_area.clone(), |this, area| this.child(area)),
                    )
                    // 右活动栏 48px（常驻，§9.2）
                    .child(render_right_activity_bar(self, window, cx)),
            )
            // ── 4. 底部状态栏 (24px) ──
            .child(render_status_bar(self, window, cx))
            // ── 5. 活动弹窗遮罩 ──
            .when_some(self.active_dialog.clone(), |this, dialog| {
                this.child(match dialog {
                    ActiveDialog::Settings => {
                        render_settings_dialog(self, window, cx).into_any_element()
                    }
                    ActiveDialog::Import => {
                        render_import_dialog(self, window, cx).into_any_element()
                    }
                    ActiveDialog::Export => {
                        render_export_dialog(self, window, cx).into_any_element()
                    }
                    ActiveDialog::Duplicates => {
                        render_duplicates_dialog(self, window, cx).into_any_element()
                    }
                    ActiveDialog::BurstConfirm => {
                        render_burst_confirm_dialog(self, window, cx).into_any_element()
                    }
                    ActiveDialog::BatchConfirm(op) => {
                        render_batch_confirm_dialog(self, op, window, cx).into_any_element()
                    }
                    ActiveDialog::DeleteConfirm => {
                        render_delete_confirm_dialog(self, window, cx).into_any_element()
                    }
                    ActiveDialog::Rename => {
                        render_rename_dialog(self, window, cx).into_any_element()
                    }
                })
            })
    }
}
