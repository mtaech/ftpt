//! 临时端到端冒烟：复制图片到系统剪贴板（跑完即删）。
//!
//! 走的是生产同一条通路：焦点根视图 → dispatch CopyImage 动作 → AppState 监听器
//! → engine_ops 解码全尺寸 RGBA → arboard 写系统剪贴板，最后用 arboard 读回校验尺寸。
//!
//! 跑法：xvfb-run -a cargo run -p photo-ui --example clipboard_smoke

use std::time::Duration;

use gpui_kit::{AppContext as _, Bounds, Point, WindowBounds, WindowOptions, px, size};

use photo_ui::actions::CopyImage;
use photo_ui::state::AppState;

async fn pump(async_cx: &mut gpui_kit::AsyncApp, ms: u64) {
    async_cx
        .background_executor()
        .timer(Duration::from_millis(ms))
        .await;
}

/// 等状态栏从「正在复制…」变成结果（成功或失败），返回最终那条文案。
async fn wait_copy_done(
    async_cx: &mut gpui_kit::AsyncApp,
    task_state: gpui_kit::Entity<AppState>,
) -> String {
    let mut msg = String::new();
    for _ in 0..160 {
        pump(async_cx, 200).await;
        msg = async_cx.update(|cx| {
            task_state
                .read(cx)
                .status_message
                .as_ref()
                .map(|(m, _)| m.clone())
                .unwrap_or_default()
        });
        if msg.contains("已复制") || msg.contains("复制图片失败") {
            break;
        }
    }
    msg
}

/// 写一张高熵 JPEG：内容越乱，之后 arboard 的 PNG 编码越慢（这正是要复现的负载）
fn write_noise_jpeg(path: &std::path::Path, w: u32, h: u32) {
    let mut img = image::RgbImage::new(w, h);
    let mut seed = 0x1234_5678u32;
    for p in img.pixels_mut() {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        *p = image::Rgb([
            (seed & 0xff) as u8,
            ((seed >> 8) & 0xff) as u8,
            ((seed >> 16) & 0xff) as u8,
        ]);
    }
    img.save(path).expect("写大图失败");
}

fn main() {
    // 无头冒烟固定走 Xvfb 的 X11 后端：Wayland 会话下窗口会落到真实桌面，渲染帧不可控
    photo_ui::app::prepare_headless_smoke();
    let app = gpui_kit::application().with_assets(gpui_kit::assets::AllAssets);
    app.run(|cx| {
        gpui_kit::component::init(cx);
        photo_ui::theme::apply(None, false, None, None, cx);
        AppState::register_keybindings(cx);

        // 一张 64×48 的测试 JPEG
        let dir = std::env::temp_dir().join("pt_clipboard_smoke");
        let _ = std::fs::remove_dir_all(&dir); // 清掉上次跑剩的坏文件
        let _ = std::fs::create_dir_all(&dir);
        // 用 PNG：JPEG 编码器不支持 RGBA
        let png = dir.join("clip.png");
        image::RgbaImage::from_fn(64, 48, |x, y| image::Rgba([x as u8, y as u8, 128, 255]))
            .save(&png)
            .expect("写测试 PNG");
        // 大图（3000×2000 噪声）：arboard 的 set_image 在 Linux 上会**同步把 RGBA 编码成
        // PNG**，噪声图压不动——修前这一步跑在主线程，正是「复制图片卡死」的来源。
        let big = dir.join("big.jpg");
        write_noise_jpeg(&big, 3000, 2000);

        let window_options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point {
                    x: px(0.),
                    y: px(0.),
                },
                size: size(px(900.), px(640.)),
            })),
            titlebar: None,
            ..Default::default()
        };

        cx.open_window(window_options, |window, cx| {
            let state = cx.new(|cx| AppState::build(window, cx));
            let handle = window.window_handle();
            photo_ui::app::focus_root(&state, window, cx);
            photo_ui::state::engine_ops::start_scan(state.clone(), dir.clone(), false, cx);

            // 与生产接线一致：根视图是 gpui-component 的 Root（tooltip / 原生菜单浮层挂它）
            let task_state = state.clone();
            let root_view = state.clone();
            let root = cx.new(|cx| {
                gpui_kit::component::Root::new(root_view, window, cx).bordered(false)
            });

            // 心跳：前台协程每 20ms 打一次点。「两次打点的最大间隔」= UI 线程被独占的时长。
            // 复制大图时 set_image（同步 PNG 编码）必须在后台线程跑，否则这个间隔会飙到 1s+。
            let heart = std::sync::Arc::new(std::sync::Mutex::new((0u128, std::time::Instant::now())));
            {
                let heart = heart.clone();
                cx.spawn(async move |async_cx: &mut gpui_kit::AsyncApp| loop {
                    async_cx
                        .background_executor()
                        .timer(Duration::from_millis(20))
                        .await;
                    if let Ok(mut h) = heart.lock() {
                        let now = std::time::Instant::now();
                        let gap = now.duration_since(h.1).as_millis();
                        if gap > h.0 {
                            h.0 = gap;
                        }
                        h.1 = now;
                    }
                })
                .detach();
            }

            cx.spawn(async move |async_cx: &mut gpui_kit::AsyncApp| {
                // tooltip 的前提：根视图必须是 Root（Root::tooltip_overlay 靠 window.root::<Root>() 查找）
                pump(async_cx, 300).await;
                let root_ok = async_cx
                    .update(|cx| {
                        handle
                            .update(cx, |_root, window, _cx| {
                                window
                                    .root::<gpui_kit::component::Root>()
                                    .flatten()
                                    .is_some()
                            })
                            .unwrap_or(false)
                    });
                if root_ok {
                    println!("ROOT_OK 根视图是 gpui-component Root（tooltip 前提成立）");
                } else {
                    eprintln!("FAIL 根视图不是 Root，tooltip 会被静默丢弃");
                    std::process::exit(1);
                }

                // 等扫出小图（clip.png）并选中它；目录里还有第二阶段的 big.jpg，
                // 所以按路径找，不能假设它是第 0 项（文件名排序 big < clip）。
                let mut clip_idx = None;
                for _ in 0..40 {
                    pump(async_cx, 200).await;
                    clip_idx = async_cx.update(|cx| {
                        task_state
                            .read(cx)
                            .items
                            .iter()
                            .position(|m| m.primary_path.ends_with("clip.png"))
                    });
                    if clip_idx.is_some() {
                        break;
                    }
                }
                let Some(clip_idx) = clip_idx else {
                    eprintln!("FAIL 小图 clip.png 没扫进列表");
                    std::process::exit(1);
                };
                async_cx.update(|cx| {
                    task_state.update(cx, |state, _| {
                        state.selected_indices = vec![clip_idx];
                        state.anchor_index = Some(clip_idx);
                    });
                });
                pump(async_cx, 300).await;

                // 与预览工具条按钮完全同一条通路
                let _ = async_cx.update(|cx| {
                    let _ = handle.update(cx, |_view, window, cx| {
                        window.dispatch_action(Box::new(CopyImage), cx);
                    });
                });
                pump(async_cx, 3000).await;

                let got = async_cx
                    .background_executor()
                    .spawn(async move {
                        arboard::Clipboard::new()
                            .ok()
                            .and_then(|mut c| c.get_image().ok())
                    })
                    .await;

                match got {
                    Some(img) if (img.width, img.height) == (64, 48) => {
                        println!("CLIPBOARD_E2E_OK {}x{}", img.width, img.height);
                        println!(
                            "状态栏：{:?}",
                            async_cx.update(|cx| task_state.read(cx).status_message.clone())
                        );
                    }
                    Some(img) => {
                        eprintln!("FAIL 尺寸不符：{}x{}", img.width, img.height);
                        std::process::exit(1);
                    }
                    None => {
                        eprintln!(
                            "FAIL 剪贴板没有图片；状态栏：{:?}",
                            async_cx.update(|cx| task_state.read(cx).status_message.clone())
                        );
                        std::process::exit(1);
                    }
                }
                // ── 大图复制：写剪贴板必须不在 UI 线程（用户报「复制图片会导致卡死」）──
                // 心跳已在开窗时启动（见上面的 heart），这里重置「复制前」的统计。

                // 扫描出大图并选中它
                async_cx.update(|cx| {
                    photo_ui::state::engine_ops::start_scan(task_state.clone(), dir.clone(), false, cx);
                });
                let mut big_idx = None;
                for _ in 0..60 {
                    pump(async_cx, 250).await;
                    big_idx = async_cx.update(|cx| {
                        task_state
                            .read(cx)
                            .items
                            .iter()
                            .position(|m| m.primary_path.ends_with("big.jpg"))
                    });
                    if big_idx.is_some() {
                        break;
                    }
                }
                let Some(big_idx) = big_idx else {
                    eprintln!("FAIL 大图没扫进列表");
                    std::process::exit(1);
                };
                async_cx.update(|cx| {
                    task_state.update(cx, |state, _| {
                        state.selected_indices = vec![big_idx];
                        state.anchor_index = Some(big_idx);
                    });
                });
                pump(async_cx, 300).await;

                // 重置心跳，再走生产同一条通路（dispatch CopyImage）
                if let Ok(mut h) = heart.lock() {
                    *h = (0, std::time::Instant::now());
                }
                let _ = async_cx.update(|cx| {
                    let _ = handle.update(cx, |_view, window, cx| {
                        window.dispatch_action(Box::new(CopyImage), cx);
                    });
                });

                // 等状态栏从「正在复制…」变成结果
                let raw_msg = wait_copy_done(async_cx, task_state.clone()).await;
                let raw_gap = heart.lock().map(|h| h.0).unwrap_or(u128::MAX);
                println!("RAW_MSG {raw_msg:?}（UI 最大停顿 {raw_gap}ms）");
                if raw_msg.contains("复制图片失败") {
                    eprintln!("FAIL 原字节路径复制失败：{raw_msg}");
                    std::process::exit(1);
                }

                // ① 状态栏说明走的是「原文件」而不是 PNG
                if !raw_msg.contains("image/jpeg 原文件") {
                    eprintln!("FAIL 状态栏没说是原文件路径：{raw_msg:?}");
                    std::process::exit(1);
                }

                // ② 剪贴板上真的挂着源文件原字节 + 文件路径（不是重编码的 PNG）
                let big_bytes = std::fs::read(&big).expect("读大图");
                let targets = x11_target_names();
                println!("CLIPBOARD_TARGETS {targets:?}");
                if !targets.iter().any(|t| t == "image/jpeg")
                    || !targets.iter().any(|t| t == "text/uri-list")
                {
                    eprintln!("FAIL TARGETS 里没有 image/jpeg + text/uri-list：{targets:?}");
                    std::process::exit(1);
                }
                match x11_read_bytes("image/jpeg") {
                    Some(bytes) if bytes == big_bytes => {
                        println!("RAW_BYTES_OK image/jpeg 与源文件逐字节相同（{} 字节）", bytes.len());
                    }
                    Some(bytes) => {
                        eprintln!("FAIL image/jpeg 字节与源文件不同：{} vs {}", bytes.len(), big_bytes.len());
                        std::process::exit(1);
                    }
                    None => {
                        eprintln!("FAIL 读不到 image/jpeg target");
                        std::process::exit(1);
                    }
                }
                let want_uri = photo_ui::state::clipboard_file::uri_list(&big);
                match x11_read_bytes("text/uri-list") {
                    Some(bytes) if bytes == want_uri.as_bytes() => {
                        println!("URI_LIST_OK text/uri-list = {}", want_uri.trim_end());
                    }
                    other => {
                        eprintln!("FAIL text/uri-list 不符：{other:?} != {want_uri:?}");
                        std::process::exit(1);
                    }
                }

                // ── 回退路径（RAW / 平台不支持时走它）：解码 → PNG 也必须不卡 UI ──
                photo_ui::state::clipboard_file::set_legacy_png(true);
                if let Ok(mut h) = heart.lock() {
                    *h = (0, std::time::Instant::now());
                }
                let _ = async_cx.update(|cx| {
                    let _ = handle.update(cx, |_view, window, cx| {
                        window.dispatch_action(Box::new(CopyImage), cx);
                    });
                });
                let png_msg = wait_copy_done(async_cx, task_state.clone()).await;
                let png_gap = heart.lock().map(|h| h.0).unwrap_or(u128::MAX);
                photo_ui::state::clipboard_file::set_legacy_png(false);
                println!("PNG_MSG {png_msg:?}（UI 最大停顿 {png_gap}ms）");
                if !png_msg.contains("已复制图片到剪贴板") {
                    eprintln!("FAIL 回退 PNG 路径没生效：{png_msg:?}");
                    std::process::exit(1);
                }
                let png_back = async_cx
                    .background_executor()
                    .spawn(async move {
                        arboard::Clipboard::new().ok().and_then(|mut c| c.get_image().ok())
                    })
                    .await;
                match png_back {
                    Some(img) if (img.width, img.height) == (3000, 2000) => {
                        println!("FALLBACK_PNG_OK 回退路径把 3000×2000 编码进剪贴板");
                    }
                    other => {
                        eprintln!("FAIL 回退路径剪贴板图片不对：{other:?}");
                        std::process::exit(1);
                    }
                }
                // 阈值 400ms：健康时是几十毫秒，修前（主线程 PNG 编码）是 1000ms 以上
                if png_gap > 400 {
                    eprintln!("FAIL 回退路径把 UI 线程独占 {png_gap}ms（应 < 400ms）");
                    std::process::exit(1);
                }
                println!("UI_RESPONSIVE_OK 两条复制路径的 UI 最大停顿：原字节 {raw_gap}ms / PNG {png_gap}ms");

                let _ = async_cx.update(|cx| cx.quit());
                println!("全部通过");
                // 同 lease_smoke：examples 带 test-support（含 leak-detection），
                // 而 detached 任务仍持有 task_state，App 正常析构会被判成泄漏；
                // 冒烟自己就是「报完结果即退出」的抛头进程。
                std::process::exit(0);
            })
            .detach();

            root
        })
        .expect("开窗失败");
    });
}

/// 建一条一次性 X 连接 + 一个请求窗口（读剪贴板用）。
fn x11_conn() -> Option<(x11rb::rust_connection::RustConnection, x11rb::protocol::xproto::Window)> {
    use x11rb::COPY_DEPTH_FROM_PARENT;
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{ConnectionExt as _, CreateWindowAux, WindowClass};

    let (conn, screen_num) = x11rb::connect(None).ok()?;
    let screen = conn.setup().roots[screen_num].clone();
    let win = conn.generate_id().ok()?;
    conn.create_window(
        COPY_DEPTH_FROM_PARENT,
        win,
        screen.root,
        0,
        0,
        1,
        1,
        0,
        WindowClass::INPUT_OUTPUT,
        screen.root_visual,
        &CreateWindowAux::new(),
    )
    .ok()?
    .check()
    .ok()?;
    Some((conn, win))
}

/// 向剪贴板所有者要一个 target 的字节（Xvfb 下由 `clipboard_file` 的 selection 服务器应答）。
fn x11_request(
    conn: &x11rb::rust_connection::RustConnection,
    win: x11rb::protocol::xproto::Window,
    target_name: &str,
) -> Option<Vec<u8>> {
    use x11rb::CURRENT_TIME;
    use x11rb::connection::Connection;
    use x11rb::protocol::Event;
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _};

    let intern = |name: &str| {
        conn.intern_atom(false, name.as_bytes())
            .ok()?
            .reply()
            .ok()
            .map(|reply| reply.atom)
    };
    let clipboard = intern("CLIPBOARD")?;
    let target = intern(target_name)?;
    let property = intern("PT_SMOKE_READ")?;
    conn.convert_selection(win, clipboard, target, property, CURRENT_TIME)
        .ok()?
        .check()
        .ok()?;
    conn.flush().ok()?;
    for _ in 0..100 {
        match conn.poll_for_event().ok()? {
            Some(Event::SelectionNotify(event)) if event.requestor == win => {
                if event.property == x11rb::NONE {
                    return None;
                }
                let reply = conn
                    .get_property(true, win, event.property, AtomEnum::ANY, 0, u32::MAX)
                    .ok()?
                    .reply()
                    .ok()?;
                return Some(reply.value);
            }
            Some(_) => continue,
            None => std::thread::sleep(std::time::Duration::from_millis(20)),
        }
    }
    None
}

fn x11_read_bytes(target_name: &str) -> Option<Vec<u8>> {
    let (conn, win) = x11_conn()?;
    x11_request(&conn, win, target_name)
}

/// `TARGETS` 里的原子名列表（用来断言剪贴板挂了哪些 MIME）。
fn x11_target_names() -> Vec<String> {
    use x11rb::protocol::xproto::ConnectionExt as _;

    let Some((conn, win)) = x11_conn() else {
        return Vec::new();
    };
    let Some(bytes) = x11_request(&conn, win, "TARGETS") else {
        return Vec::new();
    };
    bytes
        .chunks_exact(4)
        .filter_map(|chunk| u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]).into())
        .filter_map(|atom| {
            conn.get_atom_name(atom)
                .ok()?
                .reply()
                .ok()
                .map(|reply| String::from_utf8_lossy(&reply.name).to_string())
        })
        .collect()
}