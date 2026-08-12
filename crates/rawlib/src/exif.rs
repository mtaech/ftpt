//! EXIF metadata extraction via LibRaw
//!
//! This module provides EXIF metadata extraction from RAW files
//! using LibRaw's internal data structures.

use std::ffi::CStr;
use std::path::Path;

use crate::ffi;

/// EXIF data container
#[derive(Debug, Clone, Default)]
pub struct ExifData {
    /// Camera make (e.g., "Panasonic")
    pub make: Option<String>,
    /// Camera model (e.g., "DC-G9")
    pub model: Option<String>,
    /// Lens model
    pub lens_model: Option<String>,
    /// Date and time original
    pub date_time_original: Option<String>,
    /// Exposure time (e.g., "1/250")
    pub exposure_time: Option<String>,
    /// F-number (e.g., "f/2.8")
    pub f_number: Option<String>,
    /// ISO speed rating
    pub iso: Option<u32>,
    /// Focal length (e.g., "50.0 mm")
    pub focal_length: Option<String>,
    /// Image width
    pub image_width: Option<u32>,
    /// Image height
    pub image_height: Option<u32>,
    /// Orientation
    pub orientation: Option<u16>,
    /// GPS latitude (degrees, minutes, seconds)
    pub gps_latitude: Option<(f64, f64, f64)>,
    /// GPS longitude (degrees, minutes, seconds)
    pub gps_longitude: Option<(f64, f64, f64)>,
    /// GPS altitude
    pub gps_altitude: Option<f64>,
    /// 对焦点像素坐标（Fuji makernotes FocusPixel，相对未旋转传感器图；无记录为 None）
    pub focus_pixel: Option<(u16, u16)>,
    /// AFInfo/AFInfo2 原始 blob（Nikon 0x0088/0x00b7、Panasonic 等；libraw 只存不解析）
    pub af_info: Option<AfInfoData>,
    /// Panasonic AFPointPosition 归一化坐标（0–1；现代机型数据在预览 EXIF 则 None）
    pub panasonic_focus: Option<(f64, f64)>,
}

/// libraw makernotes 的 AFInfo 原始数据（应用层按 tag/version 解析坐标）。
#[derive(Debug, Clone, Default)]
pub struct AfInfoData {
    /// makernotes tag：0x0088 = AFInfo（旧版，无坐标）、0x00b7 = AFInfo2
    pub tag: u32,
    /// TIFF 字节序：0x4949 小端 / 0x4d4d 大端
    pub order: u16,
    /// AFInfo2 版本号（300 = 0300 等；AFInfo 老版未设置为 0）
    pub version: u32,
    /// blob 数据（AFInfo2 已跳过前 4 字节版本号，从 AFDetectionMethod 开始）
    pub data: Vec<u8>,
}

impl ExifData {
    /// Returns a formatted summary of key EXIF data
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();

        if let Some(ref make) = self.make {
            parts.push(format!("相机: {}", make));
        }
        if let Some(ref model) = self.model {
            parts.push(format!("型号: {}", model));
        }
        if let Some(ref lens) = self.lens_model {
            parts.push(format!("镜头: {}", lens));
        }
        if let Some(ref date) = self.date_time_original {
            parts.push(format!("拍摄时间: {}", date));
        }
        if let Some(ref exp) = self.exposure_time {
            parts.push(format!("快门: {}", exp));
        }
        if let Some(ref fnum) = self.f_number {
            parts.push(format!("光圈: {}", fnum));
        }
        if let Some(iso) = self.iso {
            parts.push(format!("ISO: {}", iso));
        }
        if let Some(ref focal) = self.focal_length {
            parts.push(format!("焦距: {}", focal));
        }
        if let (Some(w), Some(h)) = (self.image_width, self.image_height) {
            parts.push(format!("尺寸: {}x{}", w, h));
        }

        parts.join(" | ")
    }

    /// Check if GPS data is available
    pub fn has_gps(&self) -> bool {
        self.gps_latitude.is_some() && self.gps_longitude.is_some()
    }

    /// Get GPS coordinates as (latitude, longitude) tuple in decimal degrees
    pub fn gps_coordinates(&self) -> Option<(f64, f64)> {
        match (self.gps_latitude, self.gps_longitude) {
            (Some((d1, m1, s1)), Some((d2, m2, s2))) => {
                let lat = d1 + m1 / 60.0 + s1 / 3600.0;
                let lon = d2 + m2 / 60.0 + s2 / 3600.0;
                Some((lat, lon))
            }
            _ => None,
        }
    }
}

/// EXIF extraction errors
#[derive(Debug, thiserror::Error)]
pub enum ExifError {
    #[error("File not found: {0}")]
    FileNotFound(std::path::PathBuf),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("LibRaw error: {0}")]
    LibRaw(String),
    #[error("Parse error: {0}")]
    ParseError(String),
}

/// Extract EXIF data from a RAW file using LibRaw
///
/// Opens the file via LibRaw (identify parses TIFF header),
/// then reads make/model/ISO/shutter/aperture/etc.
/// No `unpack()` call — avoids decoding the full RAW pixel data.
pub fn extract_exif<P: AsRef<Path>>(path: P) -> Result<ExifData, ExifError> {
    let path = path.as_ref();

    if !path.exists() {
        return Err(ExifError::FileNotFound(path.to_path_buf()));
    }

    unsafe {
        let data = ffi::libraw_init(ffi::LIBRAW_OPTIONS_NONE);
        if data.is_null() {
            return Err(ExifError::LibRaw("libraw_init returned NULL".into()));
        }

        // Panasonic AFPointPosition 捕获：注册 makernotes 回调（必须早于 open）
        let pan_ctx = ffi::rawlib_pan_ctx_alloc();
        if pan_ctx.is_null() {
            ffi::libraw_close(data);
            return Err(ExifError::LibRaw("pan_ctx_alloc returned NULL".into()));
        }
        ffi::rawlib_panasonic_af_init(data, pan_ctx);

        // Open file
        #[cfg(windows)]
        let open_result = {
            use std::os::windows::ffi::OsStrExt;
            let wide: Vec<u16> = std::ffi::OsStr::new(path)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            ffi::libraw_open_wfile(data, wide.as_ptr())
        };
        #[cfg(not(windows))]
        let open_result = {
            let cpath = std::ffi::CString::new(path.to_string_lossy().as_ref())
                .map_err(|_| ExifError::ParseError("path contains null byte".into()))?;
            ffi::libraw_open_file(data, cpath.as_ptr())
        };

        if open_result != ffi::LIBRAW_SUCCESS {
            let err_msg = read_cstr(ffi::libraw_strerror(open_result));
            ffi::libraw_close(data);
            return Err(ExifError::LibRaw(err_msg));
        }

        // Read EXIF fields — open_file() already called identify(),
        // which parses the TIFF header and populates iparams/imgother/lensinfo.
        let ip = &*ffi::libraw_get_iparams(data);
        let img = &*ffi::libraw_get_imgother(data);
        let lens = &*ffi::libraw_get_lensinfo(data);

        let make = cstr_from_bytes(&ip.make);
        let model = cstr_from_bytes(&ip.model);
        let lens_model = cstr_from_bytes(&lens.Lens);

        // ISO
        let iso = if img.iso_speed > 0.0 {
            Some(img.iso_speed as u32)
        } else {
            None
        };

        // Shutter: LibRaw stores as seconds (e.g. 0.008 = 1/125)
        let exposure_time = if img.shutter > 0.0 {
            let inv = (1.0 / img.shutter).round();
            if (inv - (1.0 / img.shutter)).abs() < 0.01 && inv <= 100000.0 {
                Some(format!("1/{}", inv as u64))
            } else {
                Some(format!("{:.4}s", img.shutter))
            }
        } else {
            None
        };

        // Aperture
        let f_number = if img.aperture > 0.0 {
            Some(format!("f/{:.1}", img.aperture))
        } else {
            None
        };

        // Focal length
        let focal_length = if img.focal_len > 0.0 {
            Some(format!("{:.1} mm", img.focal_len))
        } else {
            None
        };

        // Timestamp → date string
        let date_time_original = if img.timestamp > 0 {
            Some(format_timestamp(img.timestamp))
        } else {
            None
        };

        // Image dimensions
        let w = ffi::libraw_get_iwidth(data);
        let h = ffi::libraw_get_iheight(data);
        let image_width = if w > 0 { Some(w as u32) } else { None };
        let image_height = if h > 0 { Some(h as u32) } else { None };

        // GPS
        let gps = &img.parsed_gps;
        let gps_latitude = if gps.gpsparsed != 0 {
            Some((
                gps.latitude[0] as f64,
                gps.latitude[1] as f64,
                gps.latitude[2] as f64,
            ))
        } else {
            None
        };
        let gps_longitude = if gps.gpsparsed != 0 {
            Some((
                gps.longitude[0] as f64,
                gps.longitude[1] as f64,
                gps.longitude[2] as f64,
            ))
        } else {
            None
        };
        let gps_altitude = if gps.gpsparsed != 0 {
            Some(gps.altitude as f64)
        } else {
            None
        };

        // 对焦点（Fuji makernotes FocusPixel，C shim 读取）
        let mut fx: u16 = 0;
        let mut fy: u16 = 0;
        let has_focus = ffi::rawlib_get_focus_pixel(data, &mut fx, &mut fy) != 0;
        let focus_pixel = if has_focus { Some((fx, fy)) } else { None };

        // AFInfo blob（Nikon AFInfo/AFInfo2、Panasonic 等；C shim 读取原始字节）
        let mut af_tag: u32 = 0;
        let mut af_order: i16 = 0;
        let mut af_version: u32 = 0;
        let mut af_len: u32 = 0;
        let mut af_buf = vec![0u8; 1 << 20]; // 1MB 上限
        let has_af = ffi::rawlib_get_afinfo(
            data,
            &mut af_tag,
            &mut af_order,
            &mut af_version,
            &mut af_len,
            af_buf.as_mut_ptr(),
            af_buf.len() as u32,
        ) != 0;
        let af_info = if has_af {
            af_buf.truncate((af_len as usize).min(af_buf.len()));
            Some(AfInfoData {
                tag: af_tag,
                order: af_order as u16,
                version: af_version,
                data: af_buf,
            })
        } else {
            None
        };

        // Panasonic AFPointPosition（回调捕获的归一化坐标）
        let mut pan_x: f64 = 0.0;
        let mut pan_y: f64 = 0.0;
        let has_pan = ffi::rawlib_panasonic_af_get(pan_ctx, &mut pan_x, &mut pan_y) != 0;
        ffi::rawlib_pan_ctx_free(pan_ctx);
        let panasonic_focus = if has_pan { Some((pan_x, pan_y)) } else { None };

        ffi::libraw_close(data);

        Ok(ExifData {
            make,
            model,
            lens_model,
            date_time_original,
            exposure_time,
            f_number,
            iso,
            focal_length,
            image_width,
            image_height,
            orientation: None, // LibRaw doesn't expose orientation via C getters
            gps_latitude,
            gps_longitude,
            gps_altitude,
            focus_pixel,
            af_info,
            panasonic_focus,
        })
    }
}

/// Extract EXIF data from multiple files in parallel
pub fn extract_exif_parallel<P: AsRef<Path> + Send + Sync>(
    paths: &[P],
    jobs: Option<usize>,
) -> Vec<(std::path::PathBuf, Result<ExifData, ExifError>)> {
    use rayon::prelude::*;

    let pool = if let Some(n) = jobs {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build()
            .expect("Failed to build thread pool")
    } else {
        rayon::ThreadPoolBuilder::new()
            .num_threads(num_cpus::get())
            .build()
            .expect("Failed to build thread pool")
    };

    pool.install(|| {
        paths
            .par_iter()
            .map(|p| (p.as_ref().to_path_buf(), extract_exif(p)))
            .collect()
    })
}

// ---- helpers ----

/// Read a null-terminated C string from a pointer (with 8KB safety limit)
unsafe fn read_cstr(ptr: *const std::os::raw::c_char) -> String {
    if ptr.is_null() {
        return String::new();
    }
    CStr::from_ptr(ptr).to_string_lossy().into_owned()
}

/// Read a null-terminated string from a fixed-size byte array
fn cstr_from_bytes(bytes: &[u8]) -> Option<String> {
    CStr::from_bytes_until_nul(bytes)
        .ok()
        .map(|s| s.to_string_lossy().trim_end().to_string())
        .filter(|s| !s.is_empty())
}

/// Format a Unix timestamp as "YYYY-MM-DD HH:MM:SS"
fn format_timestamp(ts: i64) -> String {
    if ts <= 0 {
        return String::new();
    }
    let secs_per_day: i64 = 86400;
    let days = ts / secs_per_day;

    let mut y = 1970i64;
    let mut remaining = days;
    loop {
        let year_days = if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
            366
        } else {
            365
        };
        if remaining < year_days {
            break;
        }
        remaining -= year_days;
        y += 1;
    }
    let months_days = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let leap = if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
        1
    } else {
        0
    };
    let mut m = 0usize;
    while m < 12 {
        let md = months_days[m] + if m == 1 { leap } else { 0 };
        if remaining < md {
            break;
        }
        remaining -= md;
        m += 1;
    }
    let day = remaining + 1;
    let secs = ts % secs_per_day;
    let h = secs / 3600;
    let min = (secs % 3600) / 60;
    let s = secs % 60;
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", y, m + 1, day, h, min, s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exif_data_default() {
        let exif = ExifData::default();
        assert!(exif.make.is_none());
        assert!(exif.model.is_none());
        assert!(!exif.has_gps());
    }

    #[test]
    fn test_exif_data_summary() {
        let exif = ExifData {
            make: Some("NIKON".to_string()),
            model: Some("D850".to_string()),
            iso: Some(100),
            ..Default::default()
        };
        let summary = exif.summary();
        assert!(summary.contains("NIKON"));
        assert!(summary.contains("D850"));
        assert!(summary.contains("ISO"));
    }
}
