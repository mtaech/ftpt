use gpui::*;
use gpui_component::status_bar::StatusBar;

use crate::state::app::RootView;
use crate::ui::theme;

pub fn render_status_bar(view: &RootView, _cx: &mut Context<RootView>) -> impl IntoElement {
    let total = view.captures.len();
    let selected_count = view.selected.len();
    let path_display: String = view
        .dir_path
        .as_ref()
        .and_then(|p| p.to_str())
        .unwrap_or("无目录")
        .to_string();

    let file_count = if view.dir_path.is_some() {
        format!("{} 文件 · {} 已选", total, selected_count)
    } else {
        "未打开目录".to_string()
    };

    StatusBar::new()
        .left(
            div()
                .text_color(theme::colors().text_muted)
                .child(file_count),
        )
        .child(
            div()
                .text_color(theme::colors().text_muted)
                .child(path_display),
        )
        .right(
            div()
                .text_color(theme::colors().success)
                .child("就绪"),
        )
}
