use gpui::*;
use gpui_component::{Icon, IconName, Sizable, h_flex};

use crate::state::app::RootView;
use crate::ui::theme;

/// 底部状态栏：24px 三段 ticker 风格
/// 左：目录路径 | 中：文件计数 | 右：扫描状态
pub fn render_status_bar(
    view: &RootView,
    _cx: &mut Context<RootView>,
) -> impl IntoElement {
    let total = view.captures.len();
    let selected_count = view.selected.len();

    // 左段：目录路径（截断）
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

    // 中段：文件计数（等宽数字）
    let file_count = if view.dir_path.is_some() {
        total
    } else {
        0
    };

    // 右段：扫描状态
    let scanning = view.scan_task.is_some();

    h_flex()
        .h(px(24.))
        .w_full()
        .bg(theme::colors().background)
        .border_t_1()
        .border_color(theme::colors().border_variant)
        .px(px(12.))
        .gap(px(8.))
        .text_xs()
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
        // 右段：扫描状态
        .child(
            h_flex()
                .flex_1()
                .items_center()
                .justify_end()
                .child(
                    div()
                        .text_color(if scanning {
                            theme::colors().text_accent
                        } else {
                            theme::colors().text_muted
                        })
                        .child(if scanning { "扫描中…" } else { "就绪" }),
                ),
        )
}
