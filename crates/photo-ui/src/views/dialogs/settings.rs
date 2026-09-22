//! 设置弹窗（对应 §11 与 §9.10）。
//!
//! 基于 gpui-component 的 Settings 系列组件（Settings / SettingPage / SettingGroup / SettingItem / SettingField）
//! 构建的现代化偏好设置界面，支持全分类搜索、明暗主题与调色板、网格列数与堆叠策略、
//! 识别引擎并发与对焦点优先级配置、快捷键速查表以及系统运行诊断。

use gpui_kit::assets::IconName as SharedIconName;
use gpui_kit::component::group_box::GroupBoxVariant;
use gpui_kit::component::setting::{
    RenderOptions, SelectIndex, SettingField, SettingGroup, SettingItem, SettingPage, Settings,
};
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::Input,
    select::Select,
    tag::Tag,
    v_flex,
};
use gpui_kit::{
    App, Axis, Context, Entity, IntoElement, SharedString, Window, div, prelude::*, px,
};
use photo_config::{DetectionSource, StackMode};

use crate::state::AppState;
pub use crate::state::SettingsTab;
use crate::theme::{ACCENT_PRESETS, hex_to_rgba, prefers_dark_ink, resolve_seed};

/// 渲染设置弹窗（居中模态对话框 + gpui-component 设置架构）
pub fn render_settings_dialog(
    state: &AppState,
    _window: &mut Window,
    cx: &mut Context<AppState>,
) -> impl IntoElement {
    let app = cx.entity().clone();

    // 映射外部初始 Tab 到 Settings 的 Page 索引
    let default_page_ix = match state.settings_tab {
        SettingsTab::General => 0,
        SettingsTab::Typography => 1,
        SettingsTab::Recognition => 2,
        SettingsTab::Shortcuts => 3,
        SettingsTab::About => 4,
    };

    let settings = Settings::new("photo-tool-settings")
        .sidebar_width(px(185.))
        .sidebar_size_range(px(160.)..px(240.))
        .with_group_variant(GroupBoxVariant::Outline)
        .with_size(gpui_kit::component::Size::Medium)
        .default_selected_index(SelectIndex {
            page_ix: default_page_ix,
            group_ix: None,
        })
        .pages(vec![
            build_general_page(&app, state, cx),
            build_typography_page(&app, state, cx),
            build_recognition_page(&app),
            build_shortcuts_page(),
            build_about_page(),
        ]);

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
            // 弹窗本体 (现代化圆角卡片)
            v_flex()
                .w(px(960.))
                .h(px(660.))
                .bg(cx.theme().sidebar)
                .rounded(px(16.))
                .border_1()
                .border_color(cx.theme().border.opacity(0.8))
                .shadow(crate::theme::overlay_shadow())
                .overflow_hidden()
                // ── 弹窗顶栏 ──
                .child(
                    h_flex()
                        .w_full()
                        .h(px(46.))
                        .items_center()
                        .justify_between()
                        .px_4()
                        .bg(cx.theme().sidebar)
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .child(
                            h_flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    Icon::new(IconName::Settings)
                                        .size(px(17.))
                                        .text_color(cx.theme().primary),
                                )
                                .child(
                                    div()
                                        .font_semibold()
                                        .text_sm()
                                        .text_color(cx.theme().foreground)
                                        .child("偏好设置"),
                                )
                                .child(
                                    Tag::secondary()
                                        .small()
                                        .child("Preferences"),
                                ),
                        )
                        .child(
                            Button::new("close-settings")
                                .ghost()
                                .small()
                                .icon(IconName::Close)
                                .tooltip("关闭 (Esc)")
                                .on_click(cx.listener(|state, _, _, cx| {
                                    state.active_dialog = None;
                                    cx.notify();
                                })),
                        ),
                )
                // ── gpui-component Settings 主体容器 ──
                .child(
                    div()
                        .w_full()
                        .flex_1()
                        .overflow_hidden()
                        .bg(cx.theme().background)
                        .child(settings),
                ),
        )
}

// ─────────────────────────────────────────────────────────────────────────────
// 页面 1：通用与外观
// ─────────────────────────────────────────────────────────────────────────────

fn build_general_page(
    app: &Entity<AppState>,
    state: &AppState,
    _cx: &mut Context<AppState>,
) -> SettingPage {
    let accent_input = state.accent_input.clone();

    SettingPage::new("通用设置")
        .icon(Icon::new(SharedIconName::SlidersHorizontal))
        .description("自定义应用外观主题、照片网格布局与文件扫描方式")
        .default_open(true)
        .resettable(false)
        // ── 分组 1：界面风格与主题色 ──
        .group(
            SettingGroup::new()
                .title("外观与主题")
                .description("界面明暗对比度与 Material You 动态调色板")
                .items(vec![
                    SettingItem::new(
                        "深色模式",
                        SettingField::switch(
                            |cx: &App| cx.theme().mode.is_dark(),
                            {
                                let app = app.clone();
                                move |val: bool, cx: &mut App| {
                                    app.update(cx, |state, cx| {
                                        state.app_config.theme = if val {
                                            photo_config::Theme::Dark
                                        } else {
                                            photo_config::Theme::Light
                                        };
                                        state.apply_theme(None, cx);
                                        cx.notify();
                                    });
                                }
                            },
                        ),
                    )
                    .description("切换浅色与深色界面模式。深色模式更突出照片明暗色彩并降低视觉疲劳")
                    .keywords(["深色", "浅色", "明暗", "主题", "暗黑", "亮色", "dark", "light", "theme"]),

                    SettingItem::new(
                        "调色板强调色",
                        SettingField::render({
                            let app = app.clone();
                            move |_options: &RenderOptions, _window: &mut Window, cx: &mut App| {
                                let current = {
                                    let state = app.read(cx);
                                    resolve_seed(state.app_config.accent_color.as_deref())
                                };
                                let primary = cx.theme().primary;
                                let border = cx.theme().border;
                                let foreground = cx.theme().foreground;
                                let muted = cx.theme().muted_foreground;

                                h_flex()
                                    .w_full()
                                    .flex_wrap()
                                    .gap_3()
                                    .children(ACCENT_PRESETS.iter().map(|preset| {
                                        let selected = current.eq_ignore_ascii_case(preset.hex);
                                        let swatch = hex_to_rgba(preset.hex);
                                        let mark = if prefers_dark_ink(preset.hex) {
                                            gpui_kit::black()
                                        } else {
                                            gpui_kit::white()
                                        };
                                        let app = app.clone();
                                        let hex = preset.hex;

                                        v_flex()
                                            .id(SharedString::from(preset.hex))
                                            .items_center()
                                            .gap_1()
                                            .cursor_pointer()
                                            .on_click(move |_, window, cx| {
                                                app.update(cx, |state, cx| {
                                                    state.set_accent(hex, Some(window), cx);
                                                    let hex_val = resolve_seed(state.app_config.accent_color.as_deref());
                                                    if let Some(input) = state.accent_input.clone() {
                                                        input.update(cx, |input_state, cx| {
                                                            input_state.set_value(hex_val, window, cx);
                                                        });
                                                    }
                                                    cx.notify();
                                                });
                                            })
                                            .child(
                                                div()
                                                    .size(px(28.))
                                                    .rounded_full()
                                                    .bg(swatch)
                                                    .border_2()
                                                    .border_color(if selected { primary } else { border.opacity(0.4) })
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .hover(move |this| this.border_color(primary))
                                                    .when(selected, move |this| {
                                                        this.child(
                                                            Icon::new(IconName::Check)
                                                                .size(px(14.))
                                                                .text_color(mark),
                                                        )
                                                    }),
                                            )
                                            .child(
                                                div()
                                                    .text_size(px(10.))
                                                    .font_medium()
                                                    .text_color(if selected { foreground } else { muted })
                                                    .child(preset.name),
                                            )
                                    }))
                            }
                        }),
                    )
                    .layout(Axis::Vertical)
                    .description("基于 Material You 色彩算法预设的协调色彩种子")
                    .keywords(["主题色", "调色板", "颜色", "色彩", "palette", "accent", "color"]),

                    SettingItem::new(
                        "自定义色彩代码",
                        SettingField::render({
                            let app = app.clone();
                            let accent_input = accent_input.clone();
                            move |_options: &RenderOptions, _window: &mut Window, cx: &mut App| {
                                let app = app.clone();
                                let current_hex = resolve_seed(app.read(cx).app_config.accent_color.as_deref());
                                let swatch = hex_to_rgba(&current_hex);
                                h_flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .size(px(22.))
                                            .rounded_md()
                                            .bg(swatch)
                                            .border_1()
                                            .border_color(cx.theme().border),
                                    )
                                    .when_some(accent_input.clone(), |this, input| {
                                        this.child(Input::new(&input).small().w(px(110.)))
                                    })
                                    .child(
                                        Button::new("btn-apply-accent")
                                            .small()
                                            .secondary()
                                            .label("应用")
                                            .on_click(move |_, window, cx| {
                                                app.update(cx, |state, cx| {
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
                                                });
                                            }),
                                    )
                            }
                        }),
                    )
                    .layout(Axis::Vertical)
                    .description("输入 #RRGGBB 格式十六进制代码并点击应用，算法将实时推导全套界面色阶")
                    .keywords(["自定义", "颜色代码", "十六进制", "hex", "seed", "custom"]),
                ]),
        )
        // ── 分组 2：照片网格与堆叠 ──
        .group(
            SettingGroup::new()
                .title("照片网格与堆叠")
                .description("缩略图卡片尺寸、排列密度与连拍照片归拢模式")
                .items(vec![
                    SettingItem::new(
                        "网格每行列数",
                        SettingField::render({
                            let app = app.clone();
                            move |options: &RenderOptions, _window: &mut Window, cx: &mut App| {
                                let current_cols = app.read(cx).grid_columns;
                                h_flex()
                                    .gap_1()
                                    .p_0p5()
                                    .rounded_lg()
                                    .bg(cx.theme().muted.opacity(0.35))
                                    .border_1()
                                    .border_color(cx.theme().border.opacity(0.5))
                                    .children([2, 3, 4, 5].map(|cols| {
                                        let is_active = cols == current_cols;
                                        let app = app.clone();
                                        Button::new(format!("col-{cols}"))
                                            .small()
                                            .with_size(options.size())
                                            .when(is_active, |b| b.secondary())
                                            .when(!is_active, |b| b.ghost())
                                            .label(format!("{cols} 列"))
                                            .on_click(move |_, window, cx| {
                                                app.update(cx, |state, cx| {
                                                    state.set_grid_columns(cols, cx);
                                                    // 同步筛选栏「每行列数」下拉的选中项
                                                    state.sync_grid_cols_select(window, cx);
                                                });
                                            })
                                    }))
                            }
                        }),
                    )
                    .layout(Axis::Vertical)
                    .description("每行显示列数（2–5 列）。列数少利于放大对比细节，列数多利于纵览海量素材")
                    .keywords(["列数", "网格", "缩略图", "尺寸", "grid", "column"]),

                    SettingItem::new(
                        "照片堆叠模式",
                        SettingField::render({
                            let app = app.clone();
                            move |options: &RenderOptions, _window: &mut Window, cx: &mut App| {
                                let current_mode = app.read(cx).app_config.stack_mode;
                                let modes = [
                                    (StackMode::None, "不堆叠"),
                                    (StackMode::ByFileName, "同名合并 (RAW+JPG)"),
                                    (StackMode::ByTime, "时间连拍 (≤2s)"),
                                ];
                                h_flex()
                                    .gap_1()
                                    .p_0p5()
                                    .rounded_lg()
                                    .bg(cx.theme().muted.opacity(0.35))
                                    .border_1()
                                    .border_color(cx.theme().border.opacity(0.5))
                                    .children(modes.map(|(mode, label)| {
                                        let is_active = mode == current_mode;
                                        let app = app.clone();
                                        Button::new(format!("stack-{mode:?}"))
                                            .small()
                                            .with_size(options.size())
                                            .when(is_active, |b| b.secondary())
                                            .when(!is_active, |b| b.ghost())
                                            .label(label)
                                            .on_click(move |_, _, cx| {
                                                app.update(cx, |state, cx| {
                                                    state.app_config.stack_mode = mode;
                                                    state.recompute_pipeline();
                                                    state.save_config();
                                                    cx.notify();
                                                });
                                            })
                                    }))
                            }
                        }),
                    )
                    .layout(Axis::Vertical)
                    .description("聚类模式下堆叠组底部提供格式徽标，可用 Q / E 键或点击直达激活特定文件")
                    .keywords(["堆叠", "连拍", "同名合并", "RAW", "JPG", "聚类", "stack", "cluster"]),
                ]),
        )
        // ── 分组 3：目录扫描 ──
        .group(
            SettingGroup::new()
                .title("文件扫描")
                .description("控制照片目录遍历与检索深度")
                .items(vec![
                    SettingItem::new(
                        "包含递归子目录",
                        SettingField::switch(
                            {
                                let app = app.clone();
                                move |cx: &App| app.read(cx).app_config.include_subdirectories
                            },
                            {
                                let app = app.clone();
                                move |val: bool, cx: &mut App| {
                                    app.update(cx, |state, cx| {
                                        state.app_config.include_subdirectories = val;
                                        state.save_config();
                                        if let Some(dir) = state.current_dir.clone() {
                                            let entity = cx.entity().clone();
                                            crate::state::engine_ops::defer_entity_action(
                                                cx,
                                                entity,
                                                move |entity, cx| {
                                                    crate::state::engine_ops::start_scan(
                                                        entity, dir, true, cx,
                                                    );
                                                },
                                            );
                                        }
                                        cx.notify();
                                    });
                                }
                            },
                        )
                        .default_value(false),
                    )
                    .description("开启后递归扫描当前文件夹及其所有嵌套子文件夹中的照片，关闭时仅扫描当前单层")
                    .keywords(["子目录", "递归", "扫描", "文件夹", "subdirectories", "scan"]),
                ]),
        )
}

// ─────────────────────────────────────────────────────────────────────────────
// 页面 2：界面字体与排印
// ─────────────────────────────────────────────────────────────────────────────

fn build_typography_page(
    app: &Entity<AppState>,
    state: &AppState,
    _cx: &mut Context<AppState>,
) -> SettingPage {
    let font_input = state.font_input.clone();
    let font_select = state.font_select.clone();

    SettingPage::new("界面字体")
        .icon(Icon::new(SharedIconName::Type))
        .description("全局文本与标签渲染字体、自定义字体应用与排印样张")
        .default_open(true)
        .resettable(false)
        .group(
            SettingGroup::new()
                .title("字体配置")
                .description("选择全局渲染字体或输入本机已安装字体名称")
                .items(vec![
                    SettingItem::new(
                        "全局界面字体",
                        SettingField::render({
                            let font_select = font_select.clone();
                            move |_options: &RenderOptions, _window: &mut Window, _cx: &mut App| {
                                div()
                                    .w_full()
                                    .when_some(font_select.clone(), |this, select| {
                                        this.child(
                                            Select::new(&select)
                                                .w_full()
                                                .max_w(px(520.))
                                                .search_placeholder("搜索已安装字体名称 (支持中英文)...")
                                                .placeholder("点击选择或搜索本机已安装字体...")
                                                .menu_max_h(px(320.)),
                                        )
                                    })
                            }
                        }),
                    )
                    .layout(Axis::Vertical)
                    .description("基于 gpui-component 的 Select 组件构建，下拉菜单垂直精准对齐。汇集常用推荐预设与操作系统全部已安装字体，内置实时中英文搜索过滤，选择后全窗口即刻生效")
                    .keywords(["字体", "字号", "font", "family", "typeface", "typography", "渲染", "搜索", "select"]),

                    SettingItem::new(
                        "自定义字体名称",
                        SettingField::render({
                            let app = app.clone();
                            let font_input = font_input.clone();
                            move |_options: &RenderOptions, _window: &mut Window, _cx: &mut App| {
                                let app_apply = app.clone();
                                let app_sync = app.clone();
                                h_flex()
                                    .w_full()
                                    .items_center()
                                    .gap_2()
                                    .when_some(font_input.clone(), |this, input| {
                                        this.child(Input::new(&input).small().w(px(280.)))
                                    })
                                    .child(
                                        Button::new("btn-apply-font")
                                            .small()
                                            .primary()
                                            .label("应用字体")
                                            .on_click(move |_, window, cx| {
                                                app_apply.update(cx, |state, cx| {
                                                    let custom_font = state
                                                        .font_input
                                                        .as_ref()
                                                        .map(|input| input.read(cx).value().to_string())
                                                        .unwrap_or_default();
                                                    let trimmed = custom_font.trim();
                                                    if !trimmed.is_empty() {
                                                        state.set_font_family(trimmed, Some(window), cx);
                                                        state.sync_font_select(window, cx);
                                                        state.set_status_message(format!(
                                                            "全局字体已设为 {trimmed}"
                                                        ));
                                                    } else {
                                                        state.set_status_message("字体名称不能为空");
                                                    }
                                                    cx.notify();
                                                });
                                            }),
                                    )
                                    .child(
                                        Button::new("btn-sync-font")
                                            .small()
                                            .ghost()
                                            .label("填入当前字体")
                                            .tooltip("将当前生效的字体名称填入输入框")
                                            .on_click(move |_, window, cx| {
                                                app_sync.update(cx, |state, cx| {
                                                    let current = state.app_config.font_family.clone();
                                                    if let Some(input) = state.font_input.clone() {
                                                        input.update(cx, |s, cx| {
                                                            s.set_value(current, window, cx);
                                                        });
                                                    }
                                                    cx.notify();
                                                });
                                            }),
                                    )
                            }
                        }),
                    )
                    .layout(Axis::Vertical)
                    .description("输入本机操作系统已安装的任意字体家族名称（如 Source Han Sans CN、Fira Code、Cascadia Code、霞鹜文楷等）并点击应用")
                    .keywords(["自定义字体", "字体名", "已安装字体", "custom font", "typeface"]),
                ]),
        )
        .group(
            SettingGroup::new()
                .title("排印效果实时样张")
                .description("当前字体的中文、西文标点及相机 EXIF 数字排印效果（随字体设定实时更新）")
                .items(vec![
                    SettingItem::render({
                        let app = app.clone();
                        move |_options, _window, cx| {
                            let current_font = app.read(cx).app_config.font_family.clone();
                            let font_family_sh = crate::theme::resolve_font(Some(&current_font));
                            let primary = cx.theme().primary;
                            let border = cx.theme().border;
                            let muted = cx.theme().muted_foreground;
                            let fg = cx.theme().foreground;
                            let bg = cx.theme().muted.opacity(0.25);

                            v_flex()
                                .w_full()
                                .p_4()
                                .gap_3()
                                .bg(bg)
                                .rounded(px(10.))
                                .border_1()
                                .border_color(border.opacity(0.6))
                                .font_family(font_family_sh)
                                .child(
                                    h_flex()
                                        .items_center()
                                        .justify_between()
                                        .child(
                                            h_flex()
                                                .items_center()
                                                .gap_2()
                                                .child(Tag::primary().small().child("实时渲染"))
                                                .child(
                                                    div()
                                                        .text_sm()
                                                        .font_semibold()
                                                        .text_color(primary)
                                                        .child(format!("生效字体：{current_font}")),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(muted)
                                                .child("Material 3 字体排印标尺"),
                                        ),
                                )
                                .child(
                                    h_flex()
                                        .items_baseline()
                                        .gap_3()
                                        .child(
                                            div()
                                                .text_size(px(28.))
                                                .font_bold()
                                                .text_color(fg)
                                                .child("永"),
                                        )
                                        .child(
                                            div()
                                                .text_base()
                                                .font_semibold()
                                                .text_color(fg)
                                                .child("红嘴蓝鹊在苍翠枝头欢快啼鸣 · 鸟羽光泽明暗分明"),
                                        ),
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(fg.opacity(0.85))
                                        .child("The quick brown fox jumps over the lazy dog. Pack my box with five dozen liquor jugs."),
                                )
                                .child(
                                    h_flex()
                                        .items_center()
                                        .gap_3()
                                        .py_1p5()
                                        .px_2p5()
                                        .rounded(px(6.))
                                        .bg(cx.theme().background.opacity(0.6))
                                        .child(
                                            div()
                                                .text_xs()
                                                .font_medium()
                                                .text_color(muted)
                                                .child("参数样张："),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .font_medium()
                                                .text_color(fg)
                                                .child("1/2000s  ·  f/2.8  ·  ISO 400  ·  400mm  ·  +0.7 EV"),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .font_medium()
                                                .text_color(primary)
                                                .child("98.5% 物种匹配 (Confirmed)"),
                                        ),
                                )
                        }
                    })
                    .keywords(["预览", "排印", "效果", "样张", "preview", "typography", "永"]),
                ]),
        )
}

// ─────────────────────────────────────────────────────────────────────────────
// 页面 2：识别与性能
// ─────────────────────────────────────────────────────────────────────────────

fn build_recognition_page(app: &Entity<AppState>) -> SettingPage {
    SettingPage::new("识别与性能")
        .icon(Icon::new(IconName::Cpu))
        .description("AI 物种识别算法、相机硬件对焦点联动与推理并发控制")
        .resettable(false)
        .group(
            SettingGroup::new()
                .title("目标检测与定位策略")
                .description("物种识别管线提取主体的 ROI 方式")
                .items(vec![
                    SettingItem::new(
                        "主体定位来源",
                        SettingField::render({
                            let app = app.clone();
                            move |options: &RenderOptions, _window: &mut Window, cx: &mut App| {
                                let current_src = app.read(cx).app_config.detection_source;
                                let sources = [
                                    (DetectionSource::Yolo, "全图 YOLO 检测"),
                                    (DetectionSource::Focus, "相机对焦点 ROI"),
                                ];
                                h_flex()
                                    .gap_1()
                                    .p_0p5()
                                    .rounded_lg()
                                    .bg(cx.theme().muted.opacity(0.35))
                                    .border_1()
                                    .border_color(cx.theme().border.opacity(0.5))
                                    .children(sources.map(|(src, label)| {
                                        let is_active = src == current_src;
                                        let app = app.clone();
                                        Button::new(format!("det-src-{src:?}"))
                                            .small()
                                            .with_size(options.size())
                                            .when(is_active, |b| b.secondary())
                                            .when(!is_active, |b| b.ghost())
                                            .label(label)
                                            .on_click(move |_, _, cx| {
                                                app.update(cx, |state, cx| {
                                                    state.app_config.detection_source = src;
                                                    state.save_config();
                                                    cx.notify();
                                                });
                                            })
                                    }))
                            }
                        }),
                    )
                    .layout(Axis::Vertical)
                    .description("Focus 模式优先从 EXIF 提取相机硬件对焦区域直接送入分类器，无对焦点时回退全图 YOLO")
                    .keywords(["识别", "YOLO", "对焦点", "AI", "定位", "模型", "detection", "focus"]),

                ]),
        )
        .group(
            SettingGroup::new()
                .title("推理模型与资产状态")
                .description("内置 ONNX Runtime 本地模型权重与离线物种分类库")
                .items(vec![
                    SettingItem::new(
                        "主体检测定位网络",
                        SettingField::render(|_options, _window, _cx| {
                            Tag::secondary().small().child("YOLOE-26s (org_det.onnx)")
                        }),
                    )
                    .description("轻量高效卷积神经网络，毫秒级快速提取画面主体边界框")
                    .keywords(["YOLO", "detect", "检测", "模型"]),

                    SettingItem::new(
                        "物种分类网络",
                        SettingField::render(|_options, _window, _cx| {
                            Tag::secondary().small().child("BioCLIP 2 (bioclip2_model_int8.onnx)")
                        }),
                    )
                    .description("全物种零样本分类网络：图像塔 embedding 与离线文本向量包做余弦检索，输出 Top-5 候选物种与相似度")
                    .keywords(["BioCLIP", "分类", "物种", "全物种", "零样本", "模型"]),

                    SettingItem::new(
                        "离线物种标签包",
                        SettingField::render(|_options, _window, _cx| {
                            Tag::secondary().small().child("data/taxon/ (93,452 类)")
                        }),
                    )
                    .description("TreeOfLife 子集文本向量 + 七级分类路径 + 中文名映射，是 BioCLIP 的标签空间；缺失时识别器启动即报错")
                    .keywords(["taxon", "标签", "向量", "embedding", "名录子集", "资产"]),

                    SettingItem::new(
                        "离线物种名录库",
                        SettingField::render(|_options, _window, _cx| {
                            Tag::secondary().small().child("bird_catalog.db (SQLite)")
                        }),
                    )
                    .description("内置中英文正名、拼音首字母索引与拉丁学名名录库，支持极速检索与别名解析")
                    .keywords(["名录", "catalog", "SQLite", "物种"]),
                ]),
        )
}

// ─────────────────────────────────────────────────────────────────────────────
// 页面 3：快捷键指南
// ─────────────────────────────────────────────────────────────────────────────

fn build_shortcuts_page() -> SettingPage {
    SettingPage::new("快捷键指南")
        .icon(Icon::new(SharedIconName::Keyboard))
        .description("支持全键盘操作。可在左侧搜索框输入动作名称或按键快速定位")
        .resettable(false)
        .group(
            SettingGroup::new()
                .title("浏览与视图导航")
                .description("视图模式切换与辅助信息显示")
                .items(vec![
                    shortcut_item(
                        "视图切换（网格 / 预览）",
                        "G / 双击",
                        "在缩略图网格视图与单张大图全屏预览模式之间无缝切换",
                        &["视图", "网格", "预览", "大图", "双击", "G", "grid", "preview"],
                    ),
                    shortcut_item(
                        "幻灯片全屏放映",
                        "S",
                        "全屏自动轮播放映当前目录照片，空格键暂停/继续",
                        &["幻灯片", "放映", "全屏", "slideshow", "S"],
                    ),
                    shortcut_item(
                        "物种全局统计看板",
                        "T",
                        "查看跨文件夹全局物种识别统计、名录与分布图",
                        &["统计", "鸟种", "鸟类", "名录", "stats", "T"],
                    ),
                    shortcut_item(
                        "主体检测框叠加",
                        "V",
                        "在大图预览中开关主体检测框显示",
                        &["检测框", "YOLO", "box", "V"],
                    ),
                    shortcut_item(
                        "相机硬件对焦点叠加",
                        "F",
                        "在大图预览中叠加显示拍摄时相机使用的硬件对焦点框",
                        &["对焦点", "对焦", "相机", "focus", "F"],
                    ),
                    shortcut_item(
                        "剪切溢出高光/暗部警告",
                        "O",
                        "高光过曝以红色斑马纹高亮，暗部欠曝以蓝色高亮",
                        &["剪切", "溢出", "过曝", "欠曝", "警告", "斑马纹", "clipping", "O"],
                    ),
                    shortcut_item(
                        "堆叠内切换激活照片",
                        "Q / E",
                        "在当前选中的堆叠照片组中前后切换激活的文件（RAW / JPEG）",
                        &["堆叠", "切换", "RAW", "JPEG", "Q", "E"],
                    ),
                ]),
        )
        .group(
            SettingGroup::new()
                .title("评级、色标与旗标")
                .description("星级评分、五色标签、旗标及 AI 识别触发")
                .items(vec![
                    shortcut_item(
                        "1–5 星评级 / 清除评级",
                        "1–5 / 0",
                        "数字键 1 到 5 快速设定星级，数字键 0 清除星级",
                        &["评级", "评分", "星级", "星", "rate", "star", "1", "2", "3", "4", "5", "0"],
                    ),
                    shortcut_item(
                        "五色彩色标签",
                        "6 / 7 / 8 / 9 / Ctrl+6",
                        "6 红、7 黄、8 绿、9 蓝、Ctrl+6 紫色标签",
                        &["色标", "标签", "红", "黄", "绿", "蓝", "紫", "color", "label"],
                    ),
                    shortcut_item(
                        "旗标标记（入选 / 淘汰 / 清除）",
                        "P / X / U",
                        "P 入选 (Pick)、X 淘汰 (Reject)、U 清除旗标 (Unflag)",
                        &["旗标", "入选", "淘汰", "清除", "pick", "reject", "unflag", "P", "X", "U"],
                    ),
                    shortcut_item(
                        "连拍选优：淘汰非最优帧",
                        "K",
                        "自动将当前连拍组中非最优帧标为淘汰（按文件尺寸 + 路径确定性选优）",
                        &["选优", "连拍", "淘汰", "cull", "K"],
                    ),
                    shortcut_item(
                        "AI 识别所选照片",
                        "B",
                        "对当前选中的单张或批量照片运行 AI 识别管线",
                        &["识别", "鸟种", "AI", "recognize", "B"],
                    ),
                ]),
        )
        .group(
            SettingGroup::new()
                .title("选择、操作与全局快捷键")
                .description("多选、文件删除与界面导航")
                .items(vec![
                    shortcut_item(
                        "全选 / 取消全选",
                        "Ctrl+A / Ctrl+D",
                        "选中当前筛选结果中的全部照片或清除选中",
                        &["全选", "取消全选", "选择", "select", "Ctrl+A", "Ctrl+D"],
                    ),
                    shortcut_item(
                        "删除到系统回收站",
                        "Delete",
                        "将所选照片安全移动至系统回收站（同名 XMP / RAW 伴侣文件联动）",
                        &["删除", "回收站", "delete", "trash"],
                    ),
                    shortcut_item(
                        "撤销批量操作",
                        "Ctrl+Z",
                        "撤销上一次执行的批量重命名、移动或复制操作",
                        &["撤销", "undo", "Ctrl+Z"],
                    ),
                    shortcut_item(
                        "侧边栏折叠显隐",
                        "Ctrl+[ / Ctrl+]",
                        "Ctrl+[ 切换左侧导航面板，Ctrl+] 切换右侧元数据面板",
                        &["侧边栏", "折叠", "展开", "面板", "dock", "sidebar"],
                    ),
                    shortcut_item(
                        "重新扫描目录",
                        "F5",
                        "刷新并重新扫描当前照片目录与元数据缓存",
                        &["刷新", "重扫", "重新扫描", "rescan", "refresh", "F5"],
                    ),
                    shortcut_item(
                        "关闭弹窗 / 优先级取消",
                        "Esc",
                        "关闭当前设置弹窗、取消框选或退出全屏模式",
                        &["退出", "取消", "关闭", "esc", "escape"],
                    ),
                ]),
        )
}

fn shortcut_item(
    title: &'static str,
    key: &'static str,
    description: &'static str,
    keywords: &[&'static str],
) -> SettingItem {
    SettingItem::new(
        title,
        SettingField::render(move |_options, _window, cx| {
            div()
                .px_2()
                .py_0p5()
                .rounded(px(5.))
                .bg(cx.theme().muted.opacity(0.35))
                .border_1()
                .border_color(cx.theme().border)
                .font_family("monospace")
                .text_xs()
                .font_semibold()
                .text_color(cx.theme().foreground)
                .child(key)
        }),
    )
    .description(description)
    .keywords(keywords.iter().copied())
}

// ─────────────────────────────────────────────────────────────────────────────
// 页面 4：关于与诊断
// ─────────────────────────────────────────────────────────────────────────────

fn build_about_page() -> SettingPage {
    SettingPage::new("关于与诊断")
        .icon(Icon::new(IconName::Info))
        .description("Photo Tool 应用版本、架构设计与系统诊断日志")
        .resettable(false)
        .group(
            SettingGroup::new().item(SettingItem::render(|_options, _window, cx| {
                h_flex()
                    .w_full()
                    .p_4()
                    .gap_4()
                    .items_center()
                    .child(
                        Icon::new(IconName::Frame)
                            .size(px(36.))
                            .text_color(cx.theme().primary),
                    )
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                h_flex()
                                    .items_baseline()
                                    .gap_2()
                                    .child(
                                        div()
                                            .font_bold()
                                            .text_lg()
                                            .text_color(cx.theme().foreground)
                                            .child("Photo Tool (ftpt)"),
                                    )
                                    .child(
                                        Tag::primary()
                                            .small()
                                            .child(format!("v{}", env!("CARGO_PKG_VERSION"))),
                                    ),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(
                                        "高性能鸟类照片快速筛选与 AI 智能识别工具 · 基于 GPUI 原生架构",
                                    ),
                            ),
                    )
                    .into_any_element()
            })),
        )
        .group(
            SettingGroup::new()
                .title("核心架构与技术栈")
                .description("纯 Rust 原生高性能技术选型")
                .items(vec![
                    SettingItem::new(
                        "界面系统",
                        SettingField::render(|_options, _window, _cx| {
                            Tag::secondary().small().child("GPUI (Zed) + gpui-kit")
                        }),
                    )
                    .description("基于 GPU 硬件加速的声明式原生渲染管道，高刷低延迟，丝滑流畅")
                    .keywords(["GPUI", "Zed", "渲染", "GPU", "界面"]),

                    SettingItem::new(
                        "核心文件引擎",
                        SettingField::render(|_options, _window, _cx| {
                            Tag::secondary().small().child("photo-engine (全同步)")
                        }),
                    )
                    .description("多线程并行扫描、磁盘紧凑缓存、SQLite 文件夹元数据索引与安全回收站")
                    .keywords(["photo-engine", "引擎", "SQLite", "缓存"]),

                    SettingItem::new(
                        "AI 物种识别管线",
                        SettingField::render(|_options, _window, _cx| {
                            Tag::secondary().small().child("YOLO26 + ONNX Runtime")
                        }),
                    )
                    .description("主体检测 + BioCLIP 全物种零样本分类 + 名录补充")
                    .keywords(["ONNX", "YOLO", "AI", "识别", "模型"]),

                    SettingItem::new(
                        "元数据解析后端",
                        SettingField::render(|_options, _window, _cx| {
                            Tag::secondary().small().child("ExifTool + rawlib")
                        }),
                    )
                    .description("ExifTool -stay_open 进程池提取厂商私有对焦点；rawlib 本地解析 RAW")
                    .keywords(["ExifTool", "rawlib", "EXIF", "元数据"]),
                ]),
        )
        .group(
            SettingGroup::new()
                .title("系统诊断与日志")
                .description("本地运行日志排查与问题追踪")
                .items(vec![
                    SettingItem::new(
                        "日志存储目录",
                        SettingField::render(|_options, _window, cx| {
                            let path = crate::logging::log_dir();
                            div()
                                .text_xs()
                                .font_family("monospace")
                                .text_color(cx.theme().muted_foreground)
                                .child(path.display().to_string())
                        }),
                    )
                    .layout(Axis::Vertical)
                    .description("系统自动滚动日切，保留最近 14 天日志；可通过 PHOTO_LOG_LEVEL 环境变量调整详细级别")
                    .keywords(["日志", "log", "目录", "排错", "诊断"]),

                    SettingItem::render(|options, _window, _cx| {
                        h_flex()
                            .w_full()
                            .justify_between()
                            .items_center()
                            .child(
                                div()
                                    .text_sm()
                                    .child("快捷查看日志排查问题"),
                            )
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(
                                        Button::new("open-log-dir")
                                            .secondary()
                                            .with_size(options.size())
                                            .icon(IconName::FolderOpen)
                                            .label("打开日志目录")
                                            .on_click(|_, _, cx| {
                                                let dir = crate::logging::log_dir().clone();
                                                cx.background_executor()
                                                    .spawn(async move {
                                                        let _ = open::that(&dir);
                                                    })
                                                    .detach();
                                            }),
                                    )
                                    .child(
                                        Button::new("open-log-file")
                                            .secondary()
                                            .with_size(options.size())
                                            .icon(IconName::ExternalLink)
                                            .label("查看今日日志")
                                            .on_click(|_, _, cx| {
                                                let file = crate::logging::today_log_file();
                                                cx.background_executor()
                                                    .spawn(async move {
                                                        let _ = open::that(&file);
                                                    })
                                                    .detach();
                                            }),
                                    ),
                            )
                            .into_any_element()
                    })
                    .keywords(["打开日志", "日志文件", "today log", "log file"]),
                ]),
        )
}
