use std::path::PathBuf;

use gpui::*;
use gpui::prelude::*;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::progress::Progress;
use gpui_component::spinner::Spinner;
use gpui_component::input::{InputState, Input};
use gpui_component::{Icon, IconName, h_flex, v_flex};
use photo_tool_core::import;

use crate::state::app::RootView;
use crate::ui::theme;

/// Steps of the import wizard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportStep {
    DeviceDetection,
    ImportOptions,
    Execution,
}

/// State for the import wizard.
pub struct ImportWizardState {
    pub step: ImportStep,
    /// Detected removable devices.
    pub devices: Vec<(PathBuf, usize)>,
    /// Scanning in progress.
    pub scanning: bool,
    /// Target directory for import.
    pub target_dir: String,
    /// Subdirectory format: "year_month_day", "iso_date", "year_iso"
    pub date_format: String,
    /// "copy" or "move"
    pub behavior: String,
    /// "skip", "overwrite", "rename"
    pub overwrite_strategy: String,
    /// Import progress: (current, total)
    pub progress: Option<(usize, usize)>,
    /// Current file being imported.
    pub current_file: Option<String>,
    /// Import completed with results.
    pub completed: bool,
    pub success_count: usize,
    pub error_count: usize,
}

impl Default for ImportWizardState {
    fn default() -> Self {
        Self {
            step: ImportStep::DeviceDetection,
            devices: Vec::new(),
            scanning: false,
            target_dir: String::new(),
            date_format: "year_month_day".to_string(),
            behavior: "copy".to_string(),
            overwrite_strategy: "skip".to_string(),
            progress: None,
            current_file: None,
            completed: false,
            success_count: 0,
            error_count: 0,
        }
    }
}

/// Render the import wizard as a modal overlay.
pub fn render_import_wizard(
    view: &RootView,
    cx: &mut Context<RootView>,
    state: &mut ImportWizardState,
) -> impl IntoElement {
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .bg(rgba(0x00000088))
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .w(px(560.))
                .max_h(px(520.))
                .bg(theme::colors().elevated_surface_background)
                .rounded_md()
                .border_1()
                .border_color(theme::colors().border)
                .shadow_lg()
                .flex()
                .flex_col()
                .p_4()
                .gap_2()
                // Header
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .pb_2()
                        .border_b_1()
                        .border_color(theme::colors().border_variant)
                        .child(
                            div()
                                .text_lg()
                                .font_weight(FontWeight::BOLD)
                                .child("导入照片"),
                        )
                        .child(
                            Button::new("import-close")
                                .ghost()
                                .label("\u{2715}")
                                .on_click({
                                    let vh = cx.entity().downgrade();
                                    move |_, _window, cx| {
                                        if let Some(view) = vh.upgrade() {
                                            cx.update_entity(&view, |view, cx| {
                                                view.show_import_wizard = false;
                                                cx.notify();
                                            })
                                        }
                                    }
                                }),
                        ),
                )
                // Step indicator
                .child(render_step_indicator(state.step))
                // Content based on step
                .child(match state.step {
                    ImportStep::DeviceDetection => {
                        render_device_detection(view, cx, state).into_any_element()
                    }
                    ImportStep::ImportOptions => {
                        render_import_options(cx, state).into_any_element()
                    }
                    ImportStep::Execution => {
                        render_execution(state).into_any_element()
                    }
                }),
        )
}

fn render_step_indicator(current: ImportStep) -> impl IntoElement {
    let steps = ["设备检测", "导入选项", "执行中"];
    div()
        .flex()
        .flex_row()
        .gap_1()
        .pb_2()
        .children(steps.iter().enumerate().map(|(i, label)| {
            let step_idx = match current {
                ImportStep::DeviceDetection => 0,
                ImportStep::ImportOptions => 1,
                ImportStep::Execution => 2,
            };
            let is_active = i == step_idx;
            let is_done = i < step_idx;
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap_1()
                .px_2()
                .py_1()
                .rounded_sm()
                .when(is_active, |s| {
                    s.bg(theme::colors().text_accent).text_color(theme::colors().surface_background)
                })
                .when(is_done, |s| {
                    s.text_color(theme::colors().text_muted)
                })
                .when(!is_active && !is_done, |s| {
                    s.text_color(theme::colors().text_muted)
                })
                .child(if is_done {
                    "\u{2713} ".to_string()
                } else {
                    format!("{}. ", i + 1)
                })
                .child(*label)
        }))
}

/// Step 1: Device detection
fn render_device_detection(
    _view: &RootView,
    cx: &mut Context<RootView>,
    state: &mut ImportWizardState,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .flex_grow(1.0)
        .gap_3()
        .child(
            div()
                .text_sm()
                .text_color(theme::colors().text_muted)
                .child("连接相机或存储卡，然后点击「扫描设备」。"),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .gap_2()
                .child(
                    Button::new("scan-devices")
                        .primary()
                        .label("扫描设备")
                        .on_click({
                            let vh = cx.entity().downgrade();
                            move |_, _window, cx| {
                                if let Some(view) = vh.upgrade() {
                                    cx.update_entity(&view, |view, cx| {
                                        view.worker.spawn(
                                            cx,
                                            || import::detect_removable_drives(),
                                            |_view, drives, cx| {
                                                tracing::info!("Found {} removable drives", drives.len());
                                                cx.notify();
                                            },
                                        );
                                    })
                                }
                            }
                        }),
                ),
        )
        .child(
            // Device list
            div()
                .flex()
                .flex_col()
                .flex_grow(1.0)
                .gap_1()
                .children(state.devices.iter().enumerate().map(|(i, (path, count))| {
                    let path_display = path.display().to_string();
                    let path_clone = path.clone();
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .justify_between()
                        .px_3()
                        .py_2()
                        .rounded_sm()
                        .hover(|s| s.bg(theme::colors().element_hover))
                        .border_1()
                        .border_color(theme::colors().border_variant)
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_0()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(path_display.clone()),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(theme::colors().text_muted)
                                        .child(format!("{} 个文件", count)),
                                ),
                        )
                        .child(
                            Button::new(format!("select-device-{i}"))
                                .primary()
                                .label("选择")
                                .on_click({
                                    let vh = cx.entity().downgrade();
                                    let device = path_clone.clone();
                                    move |_, _window, cx| {
                                        let device = device.clone();
                                        if let Some(view) = vh.upgrade() {
                                            cx.update_entity(&view, |view, cx| {
                                                view.worker.spawn(
                                                    cx,
                                                    move || import::scan_device_for_photos(&device),
                                                    |_view, result, cx| {
                                                        match result {
                                                            Ok(_photos) => tracing::info!("Scanned device successfully"),
                                                            Err(e) => tracing::error!("Scan failed: {e}"),
                                                        }
                                                        cx.notify();
                                                    },
                                                );
                                            })
                                        }
                                    }
                                }),
                        )
                })),
        )
        .child(
            // Cancel button
            div()
                .flex()
                .flex_row()
                .justify_end()
                .gap_2()
                .pt_2()
                .border_t_1()
                .border_color(theme::colors().border_variant)
                .child(
                    Button::new("cancel-step1")
                        .ghost()
                        .label("取消")
                        .on_click({
                            let vh = cx.entity().downgrade();
                            move |_, _window, cx| {
                                if let Some(view) = vh.upgrade() {
                                    cx.update_entity(&view, |view, cx| {
                                        view.show_import_wizard = false;
                                        cx.notify();
                                    })
                                }
                            }
                        }),
                ),
        )
}

/// Step 2: Import options
fn render_import_options(
    cx: &mut Context<RootView>,
    state: &mut ImportWizardState,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .flex_grow(1.0)
        .gap_3()
        // Target directory
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .child("目标目录"),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .child(
                            div()
                                .flex_grow(1.0)
                                .px_2()
                                .py_1()
                                .rounded_sm()
                                .bg(theme::colors().element_background)
                                .border_1()
                                .border_color(theme::colors().border_variant)
                                .text_sm()
                                .child(if state.target_dir.is_empty() {
                                    SharedString::from("选择目标文件夹...")
                                } else {
                                    SharedString::from(state.target_dir.as_str())
                                }),
                        )
                        .child(
                            Button::new("browse")
                                .outline()
                                .label("浏览")
                                .on_click({
                                    let vh = cx.entity().downgrade();
                                    move |_, _window, cx| {
                                        if let Some(view) = vh.upgrade() {
                                            cx.update_entity(&view, |_view, cx| {
                                                tracing::info!("Browse for target directory");
                                                cx.notify();
                                            })
                                        }
                                    }
                                }),
                        ),
                ),
        )
        // Subdirectory format
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .child("子目录格式"),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .children(["year_month_day", "iso_date", "year_iso"]
                            .iter()
                            .enumerate()
                            .map(|(i, &format)| {
                                let is_active = state.date_format == format;
                                let format_label = match format {
                                    "year_month_day" => "2025/01/15",
                                    "iso_date" => "2025-01-15",
                                    "year_iso" => "2025/2025-01-15",
                                    _ => format,
                                };
                                let vh = cx.entity().downgrade();
                                Button::new(format!("date-format-{i}"))
                                    .when(is_active, |b| b.primary())
                                    .when(!is_active, |b| b.ghost())
                                    .label(format_label)
                                    .on_click({
                                        let format = format.to_string();
                                        let vh = vh.clone();
                                        move |_, _window, cx| {
                                            if let Some(view) = vh.upgrade() {
                                                cx.update_entity(&view, |_view, cx| {
                                                    tracing::info!("Date format: {}", format);
                                                    cx.notify();
                                                })
                                            }
                                        }
                                    })
                            })),
                ),
        )
        // Behavior: Copy / Move
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .child("导入方式"),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .children(["copy", "move"].iter().enumerate().map(|(i, &mode)| {
                            let is_active = state.behavior == mode;
                            let vh = cx.entity().downgrade();
                                Button::new(format!("behavior-{i}"))
                                    .when(is_active, |b| b.primary())
                                    .when(!is_active, |b| b.ghost())
                                    .label(if mode == "copy" { "复制" } else { "移动" })
                                    .on_click({
                                        let mode = mode.to_string();
                                        let vh = vh.clone();
                                        move |_, _window, cx| {
                                            if let Some(view) = vh.upgrade() {
                                                cx.update_entity(&view, |_view, cx| {
                                                    tracing::info!("Behavior: {}", mode);
                                                    cx.notify();
                                                })
                                            }
                                        }
                                    })
                        })),
                ),
        )
        // Overwrite strategy
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .child("同名处理"),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .children(["skip", "overwrite", "rename"].iter().enumerate().map(|(i, &strategy)| {
                            let is_active = state.overwrite_strategy == strategy;
                            let vh = cx.entity().downgrade();
                                Button::new(format!("overwrite-{i}"))
                                    .when(is_active, |b| b.primary())
                                    .when(!is_active, |b| b.ghost())
                                    .label(match strategy {
                                        "skip" => "跳过",
                                        "overwrite" => "覆盖",
                                        "rename" => "重命名",
                                        _ => strategy,
                                    })
                                    .on_click({
                                        let strategy = strategy.to_string();
                                        let vh = vh.clone();
                                        move |_, _window, cx| {
                                            if let Some(view) = vh.upgrade() {
                                                cx.update_entity(&view, |_view, cx| {
                                                    tracing::info!("Overwrite: {}", strategy);
                                                    cx.notify();
                                                })
                                            }
                                        }
                                    })
                        })),
                ),
        )
        // Navigation buttons
        .child(
            div()
                .flex()
                .flex_row()
                .justify_between()
                .pt_2()
                .border_t_1()
                .border_color(theme::colors().border_variant)
                .child(
                    Button::new("back")
                        .ghost()
                        .label("← 返回")
                        .on_click({
                            let vh = cx.entity().downgrade();
                            move |_, _window, cx| {
                                if let Some(view) = vh.upgrade() {
                                    cx.update_entity(&view, |_view, cx| {
                                        tracing::info!("Back to device detection");
                                        cx.notify();
                                    })
                                }
                            }
                        }),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_2()
                        .child(
                            Button::new("cancel-step2")
                                .ghost()
                                .label("取消")
                                .on_click({
                                    let vh = cx.entity().downgrade();
                                    move |_, _window, cx| {
                                        if let Some(view) = vh.upgrade() {
                                            cx.update_entity(&view, |view, cx| {
                                                view.show_import_wizard = false;
                                                cx.notify();
                                            })
                                        }
                                    }
                                }),
                        )
                        .child(
                            Button::new("start")
                                .primary()
                                .label("开始导入")
                                .on_click({
                                    let vh = cx.entity().downgrade();
                                    move |_, _window, cx| {
                                        if let Some(view) = vh.upgrade() {
                                            cx.update_entity(&view, |_view, cx| {
                                                tracing::info!("Start import");
                                                cx.notify();
                                            })
                                        }
                                    }
                                }),
                        ),
                ),
        )
}

/// Step 3: Execution
fn render_execution(state: &ImportWizardState) -> impl IntoElement {
    let current_file_display = state.current_file.clone().unwrap_or_else(|| "正在启动...".to_string());

    div()
        .flex()
        .flex_col()
        .flex_grow(1.0)
        .gap_3()
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .child("正在导入照片..."),
        )
        .child(
            // Progress bar
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child({
                    let (current, total) = state.progress.unwrap_or((0, 1));
                    Progress::new("import-progress")
                        .value((current as f32 / total as f32 * 100.0).min(100.0))
                })
                .child(
                    div()
                        .text_xs()
                        .text_color(theme::colors().text_muted)
                        .child(match state.progress {
                            Some((current, total)) => {
                                format!("{} / {} 个文件", current, total)
                            }
                            None => "准备中...".to_string(),
                        }),
                ),
        )
        .child(
            // Current file
            div()
                .text_xs()
                .text_color(theme::colors().text_muted)
                .truncate()
                .child(current_file_display),
        )
        // Completion summary
        .when(state.completed, |d| {
            d.child(
                div()
                    .mt_2()
                    .p_3()
                    .rounded_sm()
                    .bg(theme::colors().element_background)
                    .border_1()
                    .border_color(theme::colors().border_variant)
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::BOLD)
                            .child("导入完成"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme::colors().text_muted)
                            .child(format!(
                                "成功 {}，失败 {}",
                                state.success_count, state.error_count
                            )),
                    ),
            )
        })
        .child(
            // Action buttons
            div()
                .flex()
                .flex_row()
                .justify_end()
                .gap_2()
                .pt_2()
                .border_t_1()
                .border_color(theme::colors().border_variant)
                .when(!state.completed, |d| {
                    d.child(
                        Button::new("cancel-import")
                            .ghost()
                            .label("取消"),
                    )
                })
                .when(state.completed, |d| {
                    d.child(
                        Button::new("close")
                            .ghost()
                            .label("关闭"),
                    )
                }),
        )
}

