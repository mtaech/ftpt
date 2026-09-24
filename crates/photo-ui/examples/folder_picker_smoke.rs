//! 手动冒烟：系统「选择目录」对话框的接线（**不需要模型，也不会真的弹框**）。
//!
//! 跑法：`XDG_CONFIG_HOME=/tmp/pt-picker-config xvfb-run -a \
//!   cargo run -p photo-ui --example folder_picker_smoke`
//!
//! 为什么要有它：2026-09-25 用户报「打开目录失效了」。根因是 rfd 0.14 在 Linux 上走
//! XDG portal，而 GPUI executor 上那条 future 永远不返回（portal 一次请求都没收到，
//! 也不走 rfd 自带的 zenity 兜底）→ 点了没反应、日志里一条错误都没有。修法是自选
//! kdialog / zenity。这个冒烟用**假的 kdialog / zenity**（PATH 前置一个临时目录）把
//! 整条链路钉住，因此不弹任何对话框、也不需要桌面环境：
//!   1) KDE 桌面 → 优先 kdialog，路径解析正确
//!   2) kdialog 失败（退出码 2）→ 落到 zenity
//!   3) 用户取消（退出码 1）→ Ok(None)，不当成失败
//!   4) 端到端：派发 OpenDirectory 动作 → 真的扫描了假选择器返回的目录

use std::io::Write;
use std::path::Path;
use std::time::Duration;

use gpui_kit::{AppContext as _, Bounds, Point, WindowBounds, WindowOptions, px, size};
use photo_ui::actions::OpenDirectory;
use photo_ui::state::AppState;
use photo_ui::state::folder_picker::pick_folder;

/// 写一个可执行的假选择器：把 `output`（或空 = 取消）打到 stdout
fn write_fake(dir: &Path, name: &str, output: Option<&str>, exit_code: i32) {
    let path = dir.join(name);
    let mut file = std::fs::File::create(&path).expect("写假选择器失败");
    let body = match output {
        Some(line) => format!("#!/bin/sh\necho \"{line}\"\nexit {exit_code}\n"),
        None => format!("#!/bin/sh\nexit {exit_code}\n"),
    };
    file.write_all(body.as_bytes()).expect("写假选择器失败");
    drop(file);
    let mut perms = std::fs::metadata(&path).expect("stat 失败").permissions();
    use std::os::unix::fs::PermissionsExt as _;
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).expect("chmod 失败");
}

fn write_jpeg(path: &Path) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("建素材目录失败");
    }
    let img = image::RgbImage::from_fn(48, 36, |x, y| image::Rgb([x as u8 * 4, y as u8 * 5, 90]));
    img.save(path).expect("写素材失败");
}

fn main() {
    photo_ui::app::prepare_headless_smoke();
    let base = std::env::temp_dir().join(format!("pt_picker_smoke_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let fakes = base.join("bin");
    let photos = base.join("photos");
    let cfg = base.join("cfg");
    for dir in [&fakes, &photos, &cfg] {
        std::fs::create_dir_all(dir).expect("建测试目录失败");
    }
    write_jpeg(&photos.join("p1.jpg"));
    write_jpeg(&photos.join("p2.jpg"));

    // PATH 前置假选择器目录（保底带上原 PATH：EXIF 等还会用系统工具）
    let old_path = std::env::var("PATH").unwrap_or_default();
    let fake_path = format!("{}:{old_path}", fakes.display());
    let photos_str = photos.to_string_lossy().to_string();
    // SAFETY: 进程启动早期、单线程（GPUI 还没起来）
    unsafe {
        std::env::set_var("PHOTO_CONFIG_DIR", &cfg);
        std::env::set_var("PATH", &fake_path);
    }

    let mut failures = 0usize;
    macro_rules! check {
        ($name:expr, $cond:expr) => {
            if $cond {
                println!("OK: {}", $name);
            } else {
                eprintln!("FAIL: {}", $name);
                failures += 1;
            }
        };
    }

    // ── 1) KDE：优先 kdialog ──
    write_fake(&fakes, "kdialog", Some(&photos_str), 0);
    write_fake(&fakes, "zenity", Some("/nonexistent/zenity-should-not-be-used"), 0);
    // SAFETY: 同上
    unsafe { std::env::set_var("XDG_CURRENT_DESKTOP", "KDE"); }
    let picked = pick_folder(None).expect("KDE 下不该失败");
    check!(
        format!("KDE 桌面优先 kdialog（选到 {picked:?}）"),
        picked.as_deref() == Some(photos.as_path())
    );

    // ── 2) kdialog 失败（退出码 2）→ 落到 zenity ──
    write_fake(&fakes, "kdialog", None, 2);
    write_fake(&fakes, "zenity", Some(&photos_str), 0);
    let picked = pick_folder(None).expect("kdialog 失败应回退 zenity，而不是报错");
    check!(
        format!("kdialog 失败后回退 zenity（选到 {picked:?}）"),
        picked.as_deref() == Some(photos.as_path())
    );

    // ── 3) 用户取消（退出码 1）→ Ok(None) ──
    write_fake(&fakes, "kdialog", None, 1);
    let picked = pick_folder(None).expect("取消不是错误");
    check!("取消（退出码 1）返回 None 而不是报错", picked.is_none());

    // ── 4) 端到端：OpenDirectory 动作 → 假选择器 → 真的扫描该目录 ──
    // 非 KDE + 只有 zenity 能出路径（kdialog 退出码 2 会被跳过）
    // SAFETY: 仍是启动前
    unsafe { std::env::set_var("XDG_CURRENT_DESKTOP", "GNOME"); }
    write_fake(&fakes, "kdialog", None, 2);
    write_fake(&fakes, "zenity", Some(&photos_str), 0);

    let app = gpui_kit::application().with_assets(gpui_kit::assets::AllAssets);
    app.run(move |cx| {
        gpui_kit::component::init(cx);
        photo_ui::theme::apply(None, false, None, None, cx);
        AppState::register_keybindings(cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point { x: px(0.), y: px(0.) },
                size: size(px(1100.), px(760.)),
            })),
            titlebar: None,
            ..Default::default()
        };
        cx.open_window(options, |window, cx| {
            let state = cx.new(|cx| AppState::build(window, cx));
            let handle = window.window_handle();
            photo_ui::app::focus_root(&state, window, cx);
            let task_state = state.clone();
            let expect_photos = photos.clone();
            cx.spawn(async move |async_cx: &mut gpui_kit::AsyncApp| {
                async_cx
                    .background_executor()
                    .timer(Duration::from_millis(600))
                    .await;
                let _ = async_cx.update(|cx| {
                    let _ = handle.update(cx, |_view, window, cx| {
                        window.dispatch_action(Box::new(OpenDirectory), cx);
                    });
                });
                // 选择器线程 + 扫描：给足时间，轮询到目录与素材都对上
                let mut ok = false;
                for _ in 0..40 {
                    async_cx
                        .background_executor()
                        .timer(Duration::from_millis(250))
                        .await;
                    let (dir_ok, n) = async_cx.update(|cx| {
                        let s = task_state.read(cx);
                        (
                            s.current_dir.as_deref() == Some(expect_photos.as_path()),
                            s.items.len(),
                        )
                    });
                    if dir_ok && n == 2 {
                        ok = true;
                        break;
                    }
                }
                check!(
                    format!("派发 OpenDirectory 后扫描了假选择器返回的目录（{expect_photos:?}）"),
                    ok
                );

                // ── 5) 最近打开 / 上次目录：运行时 + AppConfig + 真的落盘 ──
                // 用户报「最近打开记录里没有我浏览打开的目录」：以前只改运行时 Vec，
                // app_config.recent_directories 从不同步；last_directory 更是没人写过。
                let picked_text = expect_photos.to_string_lossy().to_string();
                let (recent_first, cfg_recent, last_dir) = async_cx.update(|cx| {
                    let s = task_state.read(cx);
                    (
                        s.recent_dirs.first().cloned(),
                        s.app_config.recent_directories.first().cloned(),
                        s.app_config.last_directory.clone(),
                    )
                });
                check!(
                    format!("写进「最近打开」运行时列表（{recent_first:?}）"),
                    recent_first.as_deref() == Some(expect_photos.as_path())
                );
                check!(
                    format!("最近打开 / 上次目录同步进 AppConfig（{cfg_recent:?} / {last_dir:?}）"),
                    cfg_recent.as_deref() == Some(picked_text.as_str())
                        && last_dir.as_deref() == Some(picked_text.as_str())
                );
                // 「重启还在不在」的唯一证据是把配置文件重新读一遍
                let disk_ok = photo_config::determine_config_path()
                    .ok()
                    .and_then(|path| photo_config::load_config(&path).ok())
                    .is_some_and(|cfg| {
                        cfg.recent_directories.first().map(String::as_str)
                            == Some(picked_text.as_str())
                            && cfg.last_directory.as_deref() == Some(picked_text.as_str())
                    });
                check!("配置真的落盘（重读配置文件仍在）", disk_ok);

                // ── 6) 收藏星标 / 从「最近打开」移除（用户报「最近打开不能移除和收藏」）──
                let fav_on = async_cx.update(|cx| {
                    task_state.update(cx, |s, _| s.toggle_favorite_dir(&expect_photos))
                });
                let (fav_state, fav_cfg) = async_cx.update(|cx| {
                    let s = task_state.read(cx);
                    (
                        s.favorite_dirs.first().cloned(),
                        s.app_config.favorite_dirs.first().cloned(),
                    )
                });
                check!(
                    format!("星标加入收藏（运行时 {fav_state:?} / 配置 {fav_cfg:?}）"),
                    fav_on
                        && fav_state.as_deref() == Some(expect_photos.as_path())
                        && fav_cfg.as_deref() == Some(picked_text.as_str())
                );
                let fav_off = async_cx.update(|cx| {
                    task_state.update(cx, |s, _| s.toggle_favorite_dir(&expect_photos))
                });
                check!(
                    "再点一次取消收藏（星标是开关，不是只增不减）",
                    !fav_off && async_cx.update(|cx| task_state.read(cx).favorite_dirs.is_empty())
                );

                // 移除最近：运行时 / AppConfig / 磁盘三处都要掉，且不动收藏
                async_cx.update(|cx| {
                    task_state.update(cx, |s, _| {
                        s.toggle_favorite_dir(&expect_photos);
                    })
                });
                async_cx.update(|cx| {
                    task_state.update(cx, |s, _| s.remove_recent_dir(&expect_photos))
                });
                let (recent_len, cfg_recent_len, last_after, fav_left) = async_cx.update(|cx| {
                    let s = task_state.read(cx);
                    (
                        s.recent_dirs.len(),
                        s.app_config.recent_directories.len(),
                        s.app_config.last_directory.clone(),
                        s.favorite_dirs.len(),
                    )
                });
                check!(
                    format!("× 从「最近打开」移除（剩 {recent_len} 条 / 配置 {cfg_recent_len} 条 / 上次目录 {last_after:?}）"),
                    recent_len == 0 && cfg_recent_len == 0 && last_after.is_none()
                );
                check!(format!("移除最近不影响收藏（favorite_dirs = {fav_left}）"), fav_left == 1);
                let disk_after = photo_config::determine_config_path()
                    .ok()
                    .and_then(|path| photo_config::load_config(&path).ok());
                check!(
                    "收藏与移除都落盘（重读配置：1 个收藏 / 0 条最近 / 无上次目录）",
                    disk_after.is_some_and(|cfg| {
                        cfg.favorite_dirs.len() == 1
                            && cfg.recent_directories.is_empty()
                            && cfg.last_directory.is_none()
                    })
                );

                if failures == 0 {
                    println!("全部通过");
                    std::process::exit(0);
                } else {
                    eprintln!("{failures} 项失败");
                    std::process::exit(1);
                }
            })
            .detach();
            state
        })
        .expect("开窗失败");
    });
}