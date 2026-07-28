use gpui::*;
use gpui_component::{Icon, IconName, Sizable, h_flex};

use crate::action::Action;
use crate::state::app::RootView;
use crate::ui::theme;

/// 底部状态栏：24px 三段 ticker 风格
/// 左：目录路径 | 中：文件计数 | 右：扫描状态 + 进度
pub fn render_status_bar(
    view: &RootView,
    cx: &mut Context<RootView>,
) -> impl IntoElement {
    let vh = cx.entity().downgrade();
    let total = view.captures.len();
    let selected_count = view.selected.len();

    let path_str = view
        .dir_path
        .as_ref()
        .and_then(|p| p.to_str())
        .unwrap_or("无目录");
    let truncated = if path_str.len() > 48 {
        format!("…{}", &path_str[path_str.len() - 45..])
    } else {
        path_str.to_string()
    };
    let file_count = if view.dir_path.is_some() { total } else { 0 };
    let scanning = view.scan_task.is_some();

    let tooltip_text = if view.batch_in_progress {
        if view.batch_progress_msg.is_empty() {
            let (done, total) = view.batch_progress.unwrap_or((0, 1));
            format!("处理中: {done}/{total}")
        } else {
            view.batch_progress_msg.clone()
        }
    } else {
        String::new()
    };

    h_flex()
        .h(px(24.))
        .w_full()
        .bg(theme::colors().background)
        .border_t_1()
        .border_color(theme::colors().border_variant)
        .px(px(12.))
        .gap(px(8.))
        
        // 左段：图标 + 路径
        .child(
            h_flex()
                .flex_1()
                .items_center()
                .gap(px(6.))
                .child(
                    Icon::new(IconName::Folder)
                        .xsmall()
                        .text_color(theme::colors().text_muted),
                )
                .child(
                    div()
                        .text_color(theme::colors().text_muted)
                        .overflow_x_hidden()
                        .text_ellipsis()
                        .child(truncated),
                ),
        )
        // 中段：文件计数
        .child(
            h_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .gap(px(4.))
                .child(
                    div()
                        .font_family(theme::MONO_FONT_FAMILY)
                        .text_color(theme::colors().text_muted)
                        .child(file_count.to_string()),
                )
                .child(div().text_color(theme::colors().text_muted).child("文件"))
                .child(div().text_color(theme::colors().text_muted).child("·"))
                .child(
                    div()
                        .font_family(theme::MONO_FONT_FAMILY)
                        .text_color(if selected_count > 0 {
                            *theme::colors::PICK
                        } else {
                            theme::colors().text_muted
                        })
                        .child(selected_count.to_string()),
                )
                .child(
                    div()
                        .text_color(if selected_count > 0 {
                            *theme::colors::PICK
                        } else {
                            theme::colors().text_muted
                        })
                        .child("已选"),
                ),
        )
        // 右段：扫描状态 + 批量操作进度
        .child({
            h_flex()
                .flex_1()
                .items_center()
                .justify_end()
                .gap(px(6.))
                .child(if let Some((done, total)) = view.sync_progress {
                    h_flex()
                        .gap(px(4.))
                        .child(div().text_color(theme::colors().text_accent).child("⟳ 同步中"))
                        .child(div().font_family(theme::MONO_FONT_FAMILY).text_color(theme::colors().text_accent).child(done.to_string()))
                        .child(div().text_color(theme::colors().text_muted).child("/"))
                        .child(div().font_family(theme::MONO_FONT_FAMILY).text_color(theme::colors().text_muted).child(total.to_string()))
                        .into_any_element()
                } else if view.batch_recognizing {
                    let (done, total) = view.batch_progress_rc;
                    let (confirmed, unrecognized, needs_review) = view.batch_counts;
                    let file = &view.batch_current_file;
                    let vh = vh.clone();
                    h_flex()
                        .gap(px(4.))
                        .child(div().text_color(theme::colors().text_accent).child("⟳ 识别中"))
                        .child(div().font_family(theme::MONO_FONT_FAMILY).text_color(theme::colors().text_accent).child(done.to_string()))
                        .child(div().text_color(theme::colors().text_muted).child("/"))
                        .child(div().font_family(theme::MONO_FONT_FAMILY).text_color(theme::colors().text_muted).child(total.to_string()))
                        .child(div().text_color(theme::colors().text_muted).child("·"))
                        .child(div().text_color(theme::colors().text_muted).max_w(px(160.)).overflow_x_hidden().text_ellipsis().child(file.clone()))
                        .child(div().text_color(theme::colors().text_muted).child("·"))
                        .child(div().font_family(theme::MONO_FONT_FAMILY).text_color(theme::colors().success).child(confirmed.to_string()))
                        .child(div().text_color(theme::colors().text_muted).child("已识别"))
                        .child(div().text_color(theme::colors().text_muted).child("·"))
                        .child(div().font_family(theme::MONO_FONT_FAMILY).text_color(theme::colors().text_muted).child(unrecognized.to_string()))
                        .child(div().text_color(theme::colors().text_muted).child("无鸟"))
                        .child(div().text_color(theme::colors().text_muted).child("·"))
                        .child(div().font_family(theme::MONO_FONT_FAMILY).text_color(theme::colors().warning).child(needs_review.to_string()))
                        .child(div().text_color(theme::colors().text_muted).child("待复核"))
                        .child({
                            let vh = vh.clone();
                            div()
                                .id("cancel-batch-recognize")
                                .cursor_pointer()
                                .text_color(theme::colors().text_muted)
                                .hover(|style| style.text_color(theme::colors().text))
                                .child("✕")
                                .on_click(move |_, _window, cx| {
                                    if let Some(e) = vh.upgrade() {
                                        cx.update_entity(&e, |view, cx| {
                                            view.dispatch_action(Action::CancelBatchRecognize, cx);
                                        });
                                    }
                                })
                        })
                        .into_any_element()
                } else if view.batch_in_progress {
                    let (done, total) = view.batch_progress.unwrap_or((0, 1));
                    let pct = if total > 0 { done as f32 / total as f32 } else { 0.0 };
                    let tt = tooltip_text;
                    h_flex()
                        .gap(px(4.))
                        .child({
                            let vh = vh.clone();
                            div()
                                .id("batch-progress-bar")
                                .w(px(80.))
                                .h(px(6.))
                                .rounded_full()
                                .bg(theme::colors().element_background)
                                .overflow_hidden()
                                .tooltip(move |window, cx| {
                                    gpui_component::tooltip::Tooltip::new(tt.clone()).build(window, cx)
                                })
                                .on_click(move |_, _window, cx| {
                                    if let Some(e) = vh.upgrade() {
                                        cx.update_entity(&e, |view, cx| {
                                            view.batch_show_progress_popup = true;
                                            cx.notify();
                                        });
                                    }
                                })
                                .child(
                                    div()
                                        .h_full()
                                        .bg(theme::colors().text_accent)
                                        .w(px((80.0 * pct).max(2.0).min(80.0)))
                                        .rounded_full(),
                                )
                        })
                        .child(div().text_color(theme::colors().text_accent).child(done.to_string()))
                        .child(div().text_color(theme::colors().text_muted).child("/"))
                        .child(div().text_color(theme::colors().text_muted).child(total.to_string()))
                        .into_any_element()
                } else {
                    div()
                        .text_color(if scanning {
                            theme::colors().text_accent
                        } else {
                            theme::colors().text_muted
                        })
                        .child(if scanning { "扫描中…" } else { "就绪" })
                        .into_any_element()
                })

        })
}
