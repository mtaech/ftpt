//! 系统「选择目录」对话框（打开目录 / 导入源 / 导入目标 / 批量目标 / 导出目标共用）。
//!
//! **为什么 Linux 上不用 rfd 的 portal 后端**（实测 2026-09-25，本机 KDE Wayland）：
//! rfd 0.14 在 Linux 默认走 XDG portal（ashpd + zbus）。在 GPUI 的 executor 上
//! `rfd::AsyncFileDialog::pick_folder()` **永远不返回**——portal 一次请求都没收到
//! （`xdg-desktop-portal-kde` 的 journal 里没有 `parent_window` 记录），也不会走 rfd 自带的
//! zenity 兜底（那条只在 portal **报错**时才走，而这里是**卡住**）。用户看到的就是
//! 「点打开目录什么都没发生」，且日志里一条错误都没有（是静默失败，最难查的那种）。
//! 同一个调用换成 zenity（死总线触发兜底）立刻出框——所以这里自己按桌面挑一个**会真的弹出来**
//! 的选择器：KDE 优先 kdialog（原生），其余优先 zenity（GTK），都没有才退回 rfd
//! （Windows / macOS 仍然走 rfd 的原生对话框）。
//!
//! 对话框是**阻塞**的：必须经 `pick_folder_async` 放到独立线程上跑，别占 UI 线程，
//! 也别占后台执行器的线程池（扫描/缩略图还要用它，用户慢慢挑目录时不该被parked 一个线程）。

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use gpui_kit::{App, Context, Entity, Window};

use super::app_state::AppState;

/// 运行时挑中的命令行选择器
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerKind {
    Kdialog,
    Zenity,
}

/// 优先顺序：KDE 桌面优先 kdialog（原生 Qt 对话框），其它桌面优先 zenity。
/// 纯函数，便于单测。
pub fn picker_order(kde: bool) -> [PickerKind; 2] {
    if kde {
        [PickerKind::Kdialog, PickerKind::Zenity]
    } else {
        [PickerKind::Zenity, PickerKind::Kdialog]
    }
}

/// 从选择器的 stdout 取路径：第一行非空内容、去掉首尾空白。取消/空输出 → None。
pub fn parse_picker_output(stdout: &[u8]) -> Option<PathBuf> {
    let text = String::from_utf8_lossy(stdout);
    let first = text.lines().map(str::trim).find(|line| !line.is_empty())?;
    Some(PathBuf::from(first))
}

enum RunErr {
    /// 命令本身不存在（没装）→ 试下一个
    NotFound,
    /// 命令跑了但失败（非 0/1 退出码）→ 记下来，最后报给状态栏
    Failed(String),
}

fn run_picker(kind: PickerKind, start: Option<&Path>) -> Result<Option<PathBuf>, RunErr> {
    let mut cmd = match kind {
        // kdialog --getexistingdirectory [起始目录]
        PickerKind::Kdialog => {
            let mut c = Command::new("kdialog");
            c.arg("--getexistingdirectory");
            if let Some(dir) = start {
                c.arg(dir);
            }
            c
        }
        // zenity --file-selection --directory [--filename=起始目录/]
        PickerKind::Zenity => {
            let mut c = Command::new("zenity");
            c.args(["--file-selection", "--directory"]);
            if let Some(dir) = start {
                c.arg(format!("--filename={}/", dir.display()));
            }
            c
        }
    };

    let out = cmd.output().map_err(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            RunErr::NotFound
        } else {
            RunErr::Failed(e.to_string())
        }
    })?;

    if out.status.success() {
        return Ok(parse_picker_output(&out.stdout));
    }
    // kdialog / zenity 都用退出码 1 表示「用户取消」
    if out.status.code() == Some(1) {
        return Ok(None);
    }
    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
    Err(RunErr::Failed(if err.is_empty() {
        format!("选择目录失败（退出码 {:?}）", out.status.code())
    } else {
        err
    }))
}

/// 阻塞式弹目录选择框（**必须在后台线程调用**）。
///
/// 返回 `Ok(None)` = 用户取消；`Err(msg)` = 所有选择器都不可用/失败（调用方应报到状态栏，
/// 别再静默失败——「打开目录失效了」那次就是什么都看不到）。
pub fn pick_folder(start: Option<&Path>) -> Result<Option<PathBuf>, String> {
    #[cfg(target_os = "linux")]
    {
        let kde = std::env::var("XDG_CURRENT_DESKTOP")
            .map(|v| v.to_ascii_uppercase().contains("KDE"))
            .unwrap_or(false);
        let mut failure: Option<String> = None;
        for kind in picker_order(kde) {
            match run_picker(kind, start) {
                Ok(Some(path)) => return Ok(Some(path)),
                Ok(None) => return Ok(None),
                Err(RunErr::NotFound) => continue,
                Err(RunErr::Failed(msg)) => failure = Some(msg),
            }
        }
        // 两个命令行选择器都没装：退回 rfd（本平台已知会静默卡住，只能算最后一搏）
        tracing::warn!(
            "kdialog / zenity 都不可用（{:?}），退回 rfd 目录对话框",
            failure
        );
        let mut dialog = rfd::FileDialog::new();
        if let Some(dir) = start {
            dialog = dialog.set_directory(dir);
        }
        Ok(dialog.pick_folder())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let mut dialog = rfd::FileDialog::new();
        if let Some(dir) = start {
            dialog = dialog.set_directory(dir);
        }
        Ok(dialog.pick_folder())
    }
}

/// 在独立线程弹对话框，回到前台把结果交给回调。
///
/// `on_picked` 只在用户真的选了目录时调用（取消什么都不做）；失败会写状态栏，不再静默。
pub fn pick_folder_async(
    window: &mut Window,
    cx: &mut Context<AppState>,
    on_picked: impl FnOnce(Entity<AppState>, &mut Window, &mut App, PathBuf) + 'static,
) {
    cx.spawn_in(window, async move |weak, async_cx| {
        let rx: Receiver<Result<Option<PathBuf>, String>> = {
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::Builder::new()
                .name("folder-picker".into())
                .spawn(move || {
                    let _ = tx.send(pick_folder(None));
                })
                .ok();
            rx
        };
        let picked = loop {
            match rx.try_recv() {
                Ok(result) => break result,
                Err(TryRecvError::Empty) => {
                    async_cx
                        .background_executor()
                        .timer(Duration::from_millis(80))
                        .await;
                }
                Err(TryRecvError::Disconnected) => {
                    break Err("目录对话框线程异常退出".to_string());
                }
            }
        };
        let _ = async_cx.update(|window, cx| {
            let Some(entity) = weak.upgrade() else {
                return;
            };
            match picked {
                Ok(Some(path)) => on_picked(entity, window, cx, path),
                Ok(None) => {}
                Err(msg) => {
                    entity.update(cx, |state, cx| {
                        state.set_status_message(format!("打开目录对话框失败：{msg}"));
                        cx.notify();
                    });
                }
            }
        });
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::{PickerKind, parse_picker_output, picker_order};
    use std::path::PathBuf;

    #[test]
    fn test_picker_order_prefers_native_on_kde() {
        // KDE 上优先原生 kdialog；其它桌面优先 zenity
        assert_eq!(picker_order(true), [PickerKind::Kdialog, PickerKind::Zenity]);
        assert_eq!(picker_order(false), [PickerKind::Zenity, PickerKind::Kdialog]);
    }

    #[test]
    fn test_parse_picker_output_takes_first_non_empty_line() {
        assert_eq!(
            parse_picker_output(b"/home/huang/Photos\n"),
            Some(PathBuf::from("/home/huang/Photos"))
        );
        assert_eq!(parse_picker_output(b"  /tmp/a b  \n"), Some(PathBuf::from("/tmp/a b")));
        assert_eq!(parse_picker_output(b"\n  /tmp/x\n"), Some(PathBuf::from("/tmp/x")));
        assert_eq!(parse_picker_output(b""), None, "取消时没有输出");
        assert_eq!(parse_picker_output(b"   \n\n"), None);
    }
}