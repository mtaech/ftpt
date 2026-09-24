//! 把源文件**原字节**挂上系统剪贴板（不解码、不重编码）。
//!
//! 为什么不用 arboard：它的公开 API 只有 `set_image`（RGBA → **同步 PNG 编码**）、
//! `set_file_list`、文本 / HTML，没有「按任意 MIME 塞原始字节」的入口。于是「复制图片」
//! 在 Linux 上必然要解码全尺寸 RGBA、再把 RGBA 编码成 PNG——39MP 就是秒级 CPU + 几百 MB
//! 内存（虽然已经挪到后台线程、不再卡 UI，但纯属白干）。用户要求「直接把源图片复制进
//! 剪贴板」，于是这里自己挂：
//!
//! - **Wayland**：`wl-clipboard-rs`（arboard 的底层依赖）的 `copy_multi`，一次挂
//!   `image/jpeg`（**源文件原字节**）+ `text/uri-list`（文件路径）；它内部起线程服务，
//!   立即返回、出错同步返回，不 fork。
//! - **X11**：自己写一个最小 selection 服务器（`x11rb`，同样已在依赖树里），对
//!   `TARGETS` / `TIMESTAMP` / 图片 MIME / `text/uri-list` / `UTF8_STRING` 逐个应答。
//!   X11 的剪贴板本来就是「进程应答请求才有数据」的语义，所以数据由一个常驻线程持有；
//!   新一次复制会顶掉旧的（旧线程收到 `SelectionClear` 自行退出）。
//!
//! 任一后端不支持 / 失败都返回 `Err`，调用方回退「解码 → PNG」（RAW 只能走那条路，
//! 它本来就不是能直接粘贴的图片格式）。

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use photo_domain::ImageFormat;

/// `text/uri-list`（freedesktop 的文件列表 MIME）
pub const URI_LIST_MIME: &str = "text/uri-list";

/// 强制走旧的「解码 → PNG」路径（冒烟与诊断用；见 [`legacy_png_forced`]）。
static LEGACY_PNG: AtomicBool = AtomicBool::new(false);

/// 冒烟 / 诊断：把「原字节挂剪贴板」关掉，验证回退路径（解码 → PNG）仍然可用。
pub fn set_legacy_png(forced: bool) {
    LEGACY_PNG.store(forced, Ordering::Relaxed);
}

/// 是否走旧的 PNG 路径：程序内开关，或 `PHOTO_CLIPBOARD_LEGACY_PNG=1`。
pub fn legacy_png_forced() -> bool {
    LEGACY_PNG.load(Ordering::Relaxed)
        || std::env::var_os("PHOTO_CLIPBOARD_LEGACY_PNG").is_some_and(|v| v == "1")
}

/// 该格式能否原样放进剪贴板（返回它自己的 MIME）。
///
/// RAW 与视频没有可直接粘贴的 MIME（RAW 需要解码成位图），返回 `None` 让调用方回退。
pub fn image_mime_for(format: &ImageFormat) -> Option<&'static str> {
    match format {
        ImageFormat::Jpeg => Some("image/jpeg"),
        ImageFormat::Png => Some("image/png"),
        ImageFormat::WebP => Some("image/webp"),
        ImageFormat::Gif => Some("image/gif"),
        ImageFormat::Tiff => Some("image/tiff"),
        ImageFormat::Bmp => Some("image/bmp"),
        ImageFormat::Heif => Some("image/heif"),
        ImageFormat::Raw(_) | ImageFormat::Other => None,
    }
}

/// `text/uri-list` 的一行：RFC 8089 的 `file://` URI（CRLF 结尾）。
///
/// 编码规则与 arboard 的 `set_file_list` 对齐（非 ASCII 一并按 UTF-8 百分号编码），
/// 这样「把路径粘进文件管理器 / 文本框」的行为和以前一致。
pub fn uri_list(path: &Path) -> String {
    let raw = path.to_string_lossy();
    let absolute = if raw.starts_with('/') {
        raw.to_string()
    } else {
        format!("/{raw}")
    };
    format!("file://{}\r\n", percent_encode_uri(&absolute))
}

/// 百分号编码：字母数字与 `-._~/!$&()*+,=:@` 原样保留（含 UTF-8 多字节按字节编码）。
///
/// 采用白名单而不是黑名单：路径里任何奇怪的字节都被编码，不会破坏 URI 语法。
fn percent_encode_uri(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.as_bytes() {
        let keep = byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'-'
                    | b'.'
                    | b'_'
                    | b'~'
                    | b'/'
                    | b'!'
                    | b'$'
                    | b'&'
                    | b'('
                    | b')'
                    | b'*'
                    | b'+'
                    | b','
                    | b'='
                    | b':'
                    | b'@'
            );
        if keep {
            out.push(*byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// 把文件原字节 + 文件路径挂上剪贴板。
///
/// 失败 / 平台不支持时返回 `Err`，调用方回退到「解码 → PNG」。
pub fn copy_file_as_image(path: &Path, mime: &str) -> Result<(), String> {
    // 应用里的 `source.path` 一定是绝对路径，但探针 / 冒烟可能给相对路径：
    // `std::path::absolute` 只做拼接（不碰文件系统），拼出来的 file URI 才是对的。
    let path = &std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    #[cfg(all(unix, not(any(target_os = "macos", target_os = "android", target_os = "emscripten"))))]
    {
        platform::copy(path, mime)
    }
    #[cfg(not(all(unix, not(any(target_os = "macos", target_os = "android", target_os = "emscripten")))))]
    {
        let _ = (path, mime);
        Err("此平台没有「原字节挂剪贴板」后端".to_string())
    }
}

#[cfg(all(unix, not(any(target_os = "macos", target_os = "android", target_os = "emscripten"))))]
mod platform {
    use super::{URI_LIST_MIME, uri_list};
    use std::path::Path;
    use std::sync::{Mutex, OnceLock};

    /// 会话里用哪种后端。判定口径与 GPUI 一致：`WAYLAND_DISPLAY` 非空即 Wayland。
    enum Backend {
        Wayland,
        X11,
    }

    fn backend() -> Option<Backend> {
        if std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty()) {
            Some(Backend::Wayland)
        } else if std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty()) {
            Some(Backend::X11)
        } else {
            None
        }
    }

    pub(super) fn copy(path: &Path, mime: &str) -> Result<(), String> {
        let bytes = std::fs::read(path).map_err(|e| format!("读取源文件失败：{e}"))?;
        let uri = uri_list(path);
        match backend() {
            Some(Backend::Wayland) => wayland_copy(bytes, mime, uri),
            Some(Backend::X11) => x11_copy(bytes, mime, uri),
            None => Err("没有可用的剪贴板后端（WAYLAND_DISPLAY / DISPLAY 都没设）".to_string()),
        }
    }

    // ── Wayland：wl-clipboard-rs 的 data-control ──

    fn wayland_copy(bytes: Vec<u8>, mime: &str, uri: String) -> Result<(), String> {
        use wl_clipboard_rs::copy::{ClipboardType, MimeSource, MimeType, Options, Source};

        let mut opts = Options::new();
        opts.clipboard(ClipboardType::Regular);
        // 默认 foreground(false)：库自己起线程服务，这里立即返回；拿不到所有权会同步报错。
        opts.copy_multi(vec![
            MimeSource {
                source: Source::Bytes(bytes.into_boxed_slice()),
                mime_type: MimeType::Specific(mime.to_string()),
            },
            MimeSource {
                source: Source::Bytes(uri.into_bytes().into_boxed_slice()),
                mime_type: MimeType::Specific(URI_LIST_MIME.to_string()),
            },
        ])
        .map_err(|e| format!("Wayland 挂剪贴板失败：{e}"))
    }

    // ── X11：最小 selection 服务器 ──

    /// 服务线程句柄。**不能 drop 掉连接**（drop 即失去所有权），所以句柄留到进程退出；
    /// 新一次复制时旧线程会收到 `SelectionClear` 自行退出。
    static X11_OWNER: OnceLock<Mutex<Option<std::thread::JoinHandle<()>>>> = OnceLock::new();

    fn x11_copy(bytes: Vec<u8>, mime: &str, uri: String) -> Result<(), String> {
        use std::collections::HashMap;
        use x11rb::connection::Connection;
        use x11rb::protocol::xproto::{
            Atom, ConnectionExt as _, CreateWindowAux, EventMask, WindowClass,
        };
        use x11rb::{COPY_DEPTH_FROM_PARENT, CURRENT_TIME};

        let (conn, screen_num) =
            x11rb::connect(None).map_err(|e| format!("连不上 X 服务器：{e}"))?;
        let screen = conn.setup().roots[screen_num].clone();
        let win = conn.generate_id().map_err(|e| e.to_string())?;
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
            &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )
        .map_err(|e| e.to_string())?
        .check()
        .map_err(|e| e.to_string())?;

        let intern = |name: &str| -> Result<Atom, String> {
            conn.intern_atom(false, name.as_bytes())
                .map_err(|e| e.to_string())?
                .reply()
                .map_err(|e| e.to_string())
                .map(|reply| reply.atom)
        };
        let clipboard = intern("CLIPBOARD")?;
        let targets = intern("TARGETS")?;
        let timestamp = intern("TIMESTAMP")?;
        let mime_atom = intern(mime)?;
        let uri_atom = intern(URI_LIST_MIME)?;
        let utf8_atom = intern("UTF8_STRING")?;

        conn.set_selection_owner(win, clipboard, CURRENT_TIME)
            .map_err(|e| e.to_string())?
            .check()
            .map_err(|e| e.to_string())?;
        let owner = conn
            .get_selection_owner(clipboard)
            .map_err(|e| e.to_string())?
            .reply()
            .map_err(|e| e.to_string())?
            .owner;
        if owner != win {
            return Err("X11 剪贴板所有权没拿到（被别的客户端占着）".to_string());
        }

        let mut data: HashMap<Atom, Vec<u8>> = HashMap::new();
        data.insert(mime_atom, bytes);
        data.insert(uri_atom, uri.clone().into_bytes());
        // 顺带挂 UTF8 文本：路径可以直接粘进文本框 / 终端
        data.insert(utf8_atom, uri.into_bytes());

        let handle = std::thread::Builder::new()
            .name("pt-clipboard-x11".to_string())
            .spawn(move || serve(conn, targets, timestamp, data))
            .map_err(|e| format!("启动剪贴板服务线程失败：{e}"))?;
        if let Ok(mut guard) = X11_OWNER.get_or_init(|| Mutex::new(None)).lock() {
            *guard = Some(handle);
        }
        Ok(())
    }

    /// 应答 selection 请求，直到所有权被别处拿走（`SelectionClear`）。
    fn serve<C: x11rb::connection::Connection>(
        conn: C,
        targets: u32,
        timestamp: u32,
        data: std::collections::HashMap<u32, Vec<u8>>,
    ) {
        use x11rb::NONE;
        use x11rb::protocol::Event;
        use x11rb::protocol::xproto::{
            AtomEnum, ConnectionExt as _, EventMask, PropMode, SELECTION_NOTIFY_EVENT,
            SelectionNotifyEvent,
        };
        // change_property8/32 在 wrapper 扩展 trait 上（xproto 只提供裸 change_property）
        use x11rb::wrapper::ConnectionExt as _;

        loop {
            let Ok(event) = conn.wait_for_event() else {
                break;
            };
            if let Event::SelectionRequest(request) = event {
                // ICCCM：请求方把 property 留空时，用 target 当属性名
                let property = if request.property == NONE {
                    request.target
                } else {
                    request.property
                };
                let served = if request.target == targets {
                    let mut list: Vec<u32> = vec![targets, timestamp];
                    list.extend(data.keys().copied());
                    conn.change_property32(
                        PropMode::REPLACE,
                        request.requestor,
                        property,
                        AtomEnum::ATOM,
                        &list,
                    )
                    .is_ok()
                } else if request.target == timestamp {
                    conn.change_property32(
                        PropMode::REPLACE,
                        request.requestor,
                        property,
                        AtomEnum::INTEGER,
                        &[request.time],
                    )
                    .is_ok()
                } else if let Some(bytes) = data.get(&request.target) {
                    conn.change_property8(
                        PropMode::REPLACE,
                        request.requestor,
                        property,
                        request.target,
                        bytes,
                    )
                    .is_ok()
                } else {
                    false
                };
                let reply = SelectionNotifyEvent {
                    response_type: SELECTION_NOTIFY_EVENT,
                    sequence: request.sequence,
                    time: request.time,
                    requestor: request.requestor,
                    selection: request.selection,
                    target: request.target,
                    property: if served { property } else { NONE },
                };
                let _ = conn.send_event(false, request.requestor, EventMask::NO_EVENT, reply);
                let _ = conn.flush();
            } else if matches!(event, Event::SelectionClear(_)) {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_image_mime_for_maps_viewable_formats_and_rejects_raw() {
        assert_eq!(image_mime_for(&ImageFormat::Jpeg), Some("image/jpeg"));
        assert_eq!(image_mime_for(&ImageFormat::Png), Some("image/png"));
        assert_eq!(image_mime_for(&ImageFormat::WebP), Some("image/webp"));
        assert_eq!(image_mime_for(&ImageFormat::Raw("NEF".into())), None);
        assert_eq!(image_mime_for(&ImageFormat::Other), None);
    }

    #[test]
    fn test_uri_list_percent_encodes_spaces_and_cjk_but_keeps_slashes() {
        let uri = uri_list(&PathBuf::from("/home/huang/图片/我的 照片.jpg"));
        assert_eq!(
            uri,
            "file:///home/huang/%E5%9B%BE%E7%89%87/%E6%88%91%E7%9A%84%20%E7%85%A7%E7%89%87.jpg\r\n"
        );
        assert!(uri.ends_with("\r\n"));
    }

    #[test]
    fn test_uri_list_encodes_hash_and_keeps_plain_path() {
        // `#` 是 URI 片段分隔符，必须编码，否则文件管理器会截断路径
        assert_eq!(
            uri_list(&PathBuf::from("/tmp/a#b/c.jpg")),
            "file:///tmp/a%23b/c.jpg\r\n"
        );
        assert_eq!(uri_list(&PathBuf::from("/tmp/x.jpg")), "file:///tmp/x.jpg\r\n");
    }
}
