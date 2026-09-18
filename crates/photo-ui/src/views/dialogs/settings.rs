//! 设置弹窗（对应 §11 与 §9.10）。
//!
//! 包含通用设置（主题、网格列数、堆叠模式、递归扫描）、快捷键参考与关于页。

use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::Input,
    separator::Separator,
    tag::Tag,
    v_flex,
};
use gpui_kit::{Context, IntoElement, SharedString, Window, div, prelude::*, px};
use photo_config::StackMode;

use crate::actions::Rescan;
use crate::state::AppState;
pub use crate::state::SettingsTab;
use crate::theme::{ACCENT_PRESETS, AccentPreset, hex_to_rgba, prefers_dark_ink, resolve_seed};

pub fn render_settings_dialog(
    state: &AppState,
    _window: &mut Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    div()
        .id("settings-modal-overlay")
        .occlude()
        .absolute()
        .inset_0()
        .bg(gpui_kit::rgba(0x00000088))
        .flex()
        .items_center()
        .justify_center()
        .child(
            // 弹窗本体 (Material 3 Large Dialog)
            v_flex()
                .w(px(820.))
                .h(px(580.))
                .bg(cx.theme().sidebar)
                .rounded(px(20.))
                .border_1()
                .border_color(cx.theme().border.opacity(0.6))
                .shadow(crate::theme::overlay_shadow())
                .overflow_hidden()
                // ── 弹窗顶栏 ──
                .child(
                    h_flex()
                        .w_full()
                        .h(px(48.))
                        .items_center()
                        .justify_between()
                        .px_4()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .child(
                            div()
                                .font_semibold()
                                .text_sm()
                                .text_color(cx.theme().foreground)
                                .child("应用设置"),
                        )
                        .child(
                            Button::new("close-settings")
                                .ghost()
                                .small()
                                .icon(IconName::Close)
                                .on_click(cx.listener(|state, _, _, cx| {
                                    state.active_dialog = None;
                                    cx.notify();
                                })),
                        ),
                )
                // ── 主体双栏 ──
                .child(
                    h_flex()
                        .w_full()
                        .flex_1()
                        .overflow_hidden()
                        // 左侧竖排 Tab
                        .child(
                            v_flex()
                                .w(px(180.))
                                .h_full()
                                .bg(cx.theme().background)
                                .border_r_1()
                                .border_color(cx.theme().border)
                                .p_2()
                                .gap_1()
                                .child(render_tab_btn(
                                    "通用设置",
                                    SettingsTab::General,
                                    state.settings_tab,
                                    cx,
                                ))
                                .child(render_tab_btn(
                                    "快捷键指南",
                                    SettingsTab::Shortcuts,
                                    state.settings_tab,
                                    cx,
                                ))
                                .child(render_tab_btn(
                                    "关于 Photo Tool",
                                    SettingsTab::About,
                                    state.settings_tab,
                                    cx,
                                )),
                        )
                        // 右侧滚动内容
                        .child(
                            div()
                                .id("settings-scroll")
                                .flex_1()
                                .h_full()
                                .p_6()
                                .overflow_y_scroll()
                                .child(match state.settings_tab {
                                    SettingsTab::General => {
                                        render_general_settings(state, cx).into_any_element()
                                    }
                                    SettingsTab::Shortcuts => {
                                        render_shortcuts_settings(cx).into_any_element()
                                    }
                                    SettingsTab::About => {
                                        render_about_settings(state, cx).into_any_element()
                                    }
                                }),
                        ),
                ),
        )
}

fn render_tab_btn(
    label: &'static str,
    tab: SettingsTab,
    current_tab: SettingsTab,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let is_active = tab == current_tab;
    Button::new(label)
        .small()
        .w_full()
        .when(is_active, |b| b.secondary())
        .when(!is_active, |b| b.ghost())
        .label(label)
        .on_click(cx.listener(move |state, _, _, cx| {
            state.settings_tab = tab;
            cx.notify();
        }))
}

fn render_general_settings(state: &AppState, cx: &mut Context<AppState>) -> impl IntoElement {
    v_flex()
        .w_full()
        .gap_4()
        // ── 外观：Material You 明暗 + 主题色（手册 §6.6） ──
        .child(
            v_flex()
                .p_4()
                .rounded(px(12.))
                .border_1()
                .border_color(cx.theme().border.opacity(0.6))
                .bg(cx.theme().popover)
                .shadow(crate::theme::panel_card_shadow(cx))
                .gap_3()
                .child(
                    div()
                        .font_medium()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("外观 · Material You"),
                )
                // 明暗模式：立即生效并落盘
                .child(
                    h_flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().foreground)
                                .child("明暗模式"),
                        )
                        .child(
                            h_flex()
                                .gap_1()
                                .child(render_mode_btn("浅色", false, state, cx))
                                .child(render_mode_btn("深色", true, state, cx)),
                        ),
                )
                // 主题色预设：M3 色盘
                .child(
                    v_flex()
                        .gap_2()
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().foreground)
                                .child("主题色 · 动态调色板"),
                        )
                        .child(
                            h_flex().flex_wrap().gap_2().children(
                                ACCENT_PRESETS
                                    .iter()
                                    .map(|preset| render_accent_swatch(preset, state, cx)),
                            ),
                        ),
                )
                // 自定义 seed
                .child(
                    v_flex()
                        .gap_2()
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().foreground)
                                .child("自定义（#RRGGBB）"),
                        )
                        .child(
                            h_flex()
                                .items_center()
                                .gap_2()
                                .when_some(state.accent_input.clone(), |this, input| {
                                    this.child(Input::new(&input).small().w(px(120.)))
                                })
                                .child(
                                    Button::new("btn-apply-accent")
                                        .small()
                                        .secondary()
                                        .label("应用")
                                        .on_click(cx.listener(|state, _, window, cx| {
                                            let hex = state
                                                .accent_input
                                                .as_ref()
                                                .map(|input| input.read(cx).value().to_string())
                                                .unwrap_or_default();
                                            if state.set_accent(&hex, Some(window), cx) {
                                                state.set_status_message(format!(
                                                    "主题色已设为 {hex}"
                                                ));
                                            } else {
                                                state.set_status_message(
                                                    "主题色格式无效，请用 #RRGGBB",
                                                );
                                            }
                                            cx.notify();
                                        })),
                                 ),
                        ),
                ),
        )
        // 网格列数
        .child(
            v_flex()
                .p_4()
                .rounded(px(12.))
                .border_1()
                .border_color(cx.theme().border.opacity(0.6))
                .bg(cx.theme().popover)
                .shadow(crate::theme::panel_card_shadow(cx))
                .gap_3()
                .child(
                    div()
                        .font_medium()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("照片网格"),
                )
                .child(
                    h_flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().foreground)
                                .child("每行显示列数 (2–5)"),
                        )
                        .child(
                            h_flex()
                                .gap_1()
                                .child(render_col_btn(2, state.grid_columns, cx))
                                .child(render_col_btn(3, state.grid_columns, cx))
                                .child(render_col_btn(4, state.grid_columns, cx))
                                .child(render_col_btn(5, state.grid_columns, cx)),
                        ),
                ),
        )
        // 堆叠模式
        .child(
            v_flex()
                .p_4()
                .rounded(px(12.))
                .border_1()
                .border_color(cx.theme().border.opacity(0.6))
                .bg(cx.theme().popover)
                .shadow(crate::theme::panel_card_shadow(cx))
                .gap_3()
                .child(
                    div()
                        .font_medium()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("堆叠与聚类"),
                )
                .child(
                    h_flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().foreground)
                                .child("堆叠模式"),
                        )
                        .child(
                            h_flex()
                                .gap_1()
                                .child(render_stack_mode_btn(
                                    "不堆叠",
                                    StackMode::None,
                                    state.app_config.stack_mode,
                                    cx,
                                ))
                                .child(render_stack_mode_btn(
                                    "同名合并",
                                    StackMode::ByFileName,
                                    state.app_config.stack_mode,
                                    cx,
                                ))
                                .child(render_stack_mode_btn(
                                    "时间连拍",
                                    StackMode::ByTime,
                                    state.app_config.stack_mode,
                                    cx,
                                )),
                        ),
                ),
        )
        // 子目录扫描
        .child(
            v_flex()
                .p_4()
                .rounded(px(12.))
                .border_1()
                .border_color(cx.theme().border.opacity(0.6))
                .bg(cx.theme().popover)
                .shadow(crate::theme::panel_card_shadow(cx))
                .gap_3()
                .child(
                    div()
                        .font_medium()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("扫描设置"),
                )
                .child(
                    h_flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().foreground)
                                .child("包含递归子目录照片"),
                        )
                        .child(
                            Button::new("toggle-subdirs")
                                .small()
                                .when(state.app_config.include_subdirectories, |b| b.secondary())
                                .when(!state.app_config.include_subdirectories, |b| b.ghost())
                                .label(if state.app_config.include_subdirectories {
                                    "已开启"
                                } else {
                                    "已关闭"
                                })
                                .on_click(cx.listener(|state, _, window, cx| {
                                    state.app_config.include_subdirectories =
                                        !state.app_config.include_subdirectories;
                                    window.dispatch_action(Box::new(Rescan), cx);
                                })),
                        ),
                ),
        )
}

/// 明暗模式分段按钮：选中态用 secondary 底，未选中 ghost。
fn render_mode_btn(
    label: &'static str,
    dark: bool,
    state: &AppState,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let selected = matches!(state.app_config.theme, photo_config::Theme::Dark) == dark;
    Button::new(label)
        .small()
        .when(selected, |b| b.secondary())
        .when(!selected, |b| b.ghost())
        .label(label)
        .on_click(cx.listener(move |state, _, window, cx| {
            let want = if dark {
                photo_config::Theme::Dark
            } else {
                photo_config::Theme::Light
            };
            if state.app_config.theme != want {
                state.app_config.theme = want;
                state.apply_theme(Some(window), cx);
            }
            cx.notify();
        }))
}

/// 主题色预设色块：圆形色片 + 名字；选中时色片描边用前景色并画对勾
/// （勾色按色片明度自动取黑/白）。
///
/// 这里刻意不用 Button：gpui-component 的 Button 会用变体样式覆盖自定义
/// bg / border_color，色片就画不出来。改用可交互 div + aria_label 保证可达性。
/// 返回的元素不借用 cx（use<> 精确捕获），否则它无法在 map 闭包里逐项构造。
fn render_accent_swatch(
    preset: &'static AccentPreset,
    state: &AppState,
    cx: &mut Context<AppState>,
) -> impl IntoElement + use<> {
    let current = resolve_seed(state.app_config.accent_color.as_deref());
    let selected = current.eq_ignore_ascii_case(preset.hex);
    let swatch = hex_to_rgba(preset.hex);
    let mark = if prefers_dark_ink(preset.hex) {
        gpui_kit::black()
    } else {
        gpui_kit::white()
    };
    let border = cx.theme().border;
    let foreground = cx.theme().foreground;
    let muted = cx.theme().muted_foreground;
    let primary = cx.theme().primary;
    let card_shadow = crate::theme::panel_card_shadow(cx);

    v_flex()
        .id(SharedString::from(preset.hex))
        .items_center()
        .gap_1()
        .cursor_pointer()
        .aria_label(SharedString::from(preset.name))
        .on_click(cx.listener(move |state, _, window, cx| {
            state.set_accent(preset.hex, Some(window), cx);
            state.sync_accent_input(window, cx);
            cx.notify();
        }))
        .child(
            div()
                .size(px(32.))
                .rounded_full()
                .bg(swatch)
                .border_2()
                .border_color(if selected { primary } else { border.opacity(0.4) })
                .flex()
                .items_center()
                .justify_center()
                .hover(move |this| this.border_color(primary))
                .when(selected, move |this| {
                    this.shadow(card_shadow)
                        .child(Icon::new(IconName::Check).size(px(16.)).text_color(mark))
                }),
        )
        .child(
            div()
                .text_size(px(10.))
                .font_medium()
                .text_color(if selected { foreground } else { muted })
                .child(preset.name),
        )
}

fn render_col_btn(cols: usize, current: usize, cx: &mut Context<AppState>) -> impl IntoElement {
    let is_selected = cols == current;
    Button::new(format!("col-{cols}"))
        .xsmall()
        .when(is_selected, |b| b.secondary())
        .when(!is_selected, |b| b.ghost())
        .label(format!("{cols} 列"))
        .on_click(cx.listener(move |state, _, _, cx| {
            state.grid_columns = cols;
            cx.notify();
        }))
}

fn render_stack_mode_btn(
    label: &'static str,
    mode: StackMode,
    current: StackMode,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let is_selected = mode == current;
    Button::new(label)
        .xsmall()
        .when(is_selected, |b| b.secondary())
        .when(!is_selected, |b| b.ghost())
        .label(label)
        .on_click(cx.listener(move |state, _, _, cx| {
            state.app_config.stack_mode = mode;
            state.recompute_pipeline();
            cx.notify();
        }))
}

fn render_shortcuts_settings(cx: &mut Context<AppState>) -> impl IntoElement {
    v_flex()
        .w_full()
        .p_4()
        .rounded(px(12.))
        .border_1()
        .border_color(cx.theme().border.opacity(0.6))
        .bg(cx.theme().popover)
        .shadow(crate::theme::panel_card_shadow(cx))
        .gap_2p5()
        .child(render_shortcut_row(
            "视图切换（网格 / 预览）",
            "G / 双击",
            cx,
        ))
        .child(Separator::horizontal())
        .child(render_shortcut_row("连拍对比模式", "C", cx))
        .child(Separator::horizontal())
        .child(render_shortcut_row("幻灯片全屏放映", "S", cx))
        .child(Separator::horizontal())
        .child(render_shortcut_row("鸟种全局统计", "T", cx))
        .child(Separator::horizontal())
        .child(render_shortcut_row("1–5 星评分 / 清除", "1–5 / 0", cx))
        .child(Separator::horizontal())
        .child(render_shortcut_row(
            "红 / 黄 / 绿 / 蓝 / 紫 色标",
            "6 / 7 / 8 / 9 / Ctrl+6",
            cx,
        ))
        .child(Separator::horizontal())
        .child(render_shortcut_row(
            "入选 / 淘汰 / 清除旗标",
            "P / X / U",
            cx,
        ))
        .child(Separator::horizontal())
        .child(render_shortcut_row("连拍选优：淘汰非最优帧", "K", cx))
        .child(Separator::horizontal())
        .child(render_shortcut_row("识别所选照片", "B", cx))
        .child(Separator::horizontal())
        .child(render_shortcut_row("检测框与鸟眼角标叠加", "V", cx))
        .child(Separator::horizontal())
        .child(render_shortcut_row("对焦点叠加", "F", cx))
        .child(Separator::horizontal())
        .child(render_shortcut_row("剪切溢出警告叠加", "O", cx))
        .child(Separator::horizontal())
        .child(render_shortcut_row(
            "全选 / 取消全选",
            "Ctrl+A / Ctrl+D",
            cx,
        ))
        .child(Separator::horizontal())
        .child(render_shortcut_row("删除到回收站", "Delete", cx))
        .child(Separator::horizontal())
        .child(render_shortcut_row("优先级撤销与退出", "Esc", cx))
}

fn render_shortcut_row(
    action: &'static str,
    key: &'static str,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    h_flex()
        .w_full()
        .items_center()
        .justify_between()
        .text_xs()
        .child(div().text_color(cx.theme().foreground).child(action))
        .child(Tag::secondary().small().child(key))
}

fn render_about_settings(_state: &AppState, cx: &mut Context<AppState>) -> impl IntoElement {
    v_flex()
        .w_full()
        .gap_4()
        .text_xs()
        .child(
            h_flex()
                .p_4()
                .rounded(px(12.))
                .border_1()
                .border_color(cx.theme().border.opacity(0.6))
                .bg(cx.theme().popover)
                .shadow(crate::theme::panel_card_shadow(cx))
                .items_center()
                .gap_3()
                .child(Icon::new(IconName::Frame).size(px(32.)).text_color(cx.theme().foreground))
                .child(
                    v_flex()
                        .child(div().font_semibold().text_base().text_color(cx.theme().foreground).child("Photo Tool (ftpt)"))
                        .child(div().text_color(cx.theme().muted_foreground).child("版本 0.6.1 (GPUI-Kit 原生架构)")),
                ),
        )
        .child(
            v_flex()
                .p_4()
                .rounded(px(12.))
                .border_1()
                .border_color(cx.theme().border.opacity(0.6))
                .bg(cx.theme().popover)
                .shadow(crate::theme::panel_card_shadow(cx))
                .gap_2()
                .child(div().font_medium().text_color(cx.theme().foreground).child("运行与技术栈"))
                .child(div().text_color(cx.theme().muted_foreground).child("• 界面框架：GPUI (Zed) + gpui-kit"))
                .child(div().text_color(cx.theme().muted_foreground).child("• 核心引擎：photo-engine (全同步 Rust)"))
                .child(div().text_color(cx.theme().muted_foreground).child("• 识别管线：YOLOv8 + ONNX Runtime (CPU)"))
                .child(div().text_color(cx.theme().muted_foreground).child("• 元数据后端：ExifTool / rawlib")),
        )
        .child(
            v_flex()
                .p_4()
                .rounded(px(12.))
                .border_1()
                .border_color(cx.theme().border.opacity(0.6))
                .bg(cx.theme().popover)
                .shadow(crate::theme::panel_card_shadow(cx))
                .gap_2()
                .child(div().font_medium().text_color(cx.theme().foreground).child("诊断"))
                .child(
                    div()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("日志目录：{}", crate::logging::log_dir().display())),
                )
                .child(
                    div()
                        .text_color(cx.theme().muted_foreground)
                        .child("滚动日切，保留 14 份；级别可用 PHOTO_LOG_LEVEL 覆盖（默认 debug 构建 DEBUG / release INFO）"),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("open-log-dir")
                                .secondary()
                                .xsmall()
                                .icon(IconName::FolderOpen)
                                .label("打开日志目录")
                                .on_click(cx.listener(|_, _, _, cx| {
                                    let dir = crate::logging::log_dir().clone();
                                    cx.background_executor()
                                        .spawn(async move {
                                            let _ = open::that(&dir);
                                        })
                                        .detach();
                                })),
                        )
                        .child(
                            Button::new("open-log-file")
                                .secondary()
                                .xsmall()
                                .icon(IconName::ExternalLink)
                                .label("打开今日日志")
                                .on_click(cx.listener(|_, _, _, cx| {
                                    let file = crate::logging::today_log_file();
                                    cx.background_executor()
                                        .spawn(async move {
                                            let _ = open::that(&file);
                                        })
                                        .detach();
                                })),
                        ),
                ),
        )
}
