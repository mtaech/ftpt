//! AFInfo blob 调试工具：dump libraw makernotes.common.afdata 原始字节。
//! 用法：cargo run -p rawlib --example dump_afdata -- <raw文件>
//! 用于逆向 Nikon AFInfo(0x0088)/AFInfo2(0x00b7)、Panasonic 等私有对焦点格式。

fn main() {
    let path = std::env::args().nth(1).expect("usage: dump_afdata <rawfile>");

    unsafe {
        let data = rawlib::ffi::libraw_init(rawlib::ffi::LIBRAW_OPTIONS_NONE);
        assert!(!data.is_null(), "libraw_init failed");

        #[cfg(windows)]
        let open_result = {
            use std::os::windows::ffi::OsStrExt;
            let wide: Vec<u16> = std::ffi::OsStr::new(&path)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            rawlib::ffi::libraw_open_wfile(data, wide.as_ptr())
        };
        #[cfg(not(windows))]
        let open_result = {
            let cpath = std::ffi::CString::new(path.clone()).unwrap();
            rawlib::ffi::libraw_open_file(data, cpath.as_ptr())
        };

        if open_result != rawlib::ffi::LIBRAW_SUCCESS {
            let err = std::ffi::CStr::from_ptr(rawlib::ffi::libraw_strerror(open_result))
                .to_string_lossy()
                .into_owned();
            eprintln!("open failed: {err}");
            rawlib::ffi::libraw_close(data);
            return;
        }

        let mut tag: u32 = 0;
        let mut order: i16 = 0;
        let mut version: u32 = 0;
        let mut len: u32 = 0;
        let mut buf = vec![0u8; 1 << 20]; // 1MB 上限
        let got = rawlib::ffi::rawlib_get_afinfo(
            data,
            &mut tag,
            &mut order,
            &mut version,
            &mut len,
            buf.as_mut_ptr(),
            buf.len() as u32,
        );
        if got != 0 {
            println!(
                "AFInfo tag=0x{tag:04x} order=0x{order:04x} version={version} len={len}"
            );
            let n = (len as usize).min(buf.len());
            for (i, chunk) in buf[..n].chunks(16).enumerate() {
                let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02x}")).collect();
                println!("{:04x}: {}", i * 16, hex.join(" "));
            }
        } else {
            println!("no afinfo blob");
        }
        rawlib::ffi::libraw_close(data);
    }
}
