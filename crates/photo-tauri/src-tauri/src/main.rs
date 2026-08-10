// 桌面端入口：全部逻辑在 lib.rs（Tauri 移动端模板约定）
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    photo_tauri_lib::run()
}
