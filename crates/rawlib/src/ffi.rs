//! Foreign Function Interface (FFI) bindings to LibRaw C API
//!
//! This module provides safe Rust bindings to the LibRaw C library functions.
//! LibRaw is a library for reading RAW files from digital cameras.
//!
//! The bindings include:
//! - Core initialization and cleanup functions
//! - File opening and processing operations
//! - Thumbnail extraction functionality
//! - Memory management helpers
//! - Error code constants and utilities

use libc::{c_char, c_int, c_uchar, c_ushort};

// Windows 平台需要使用宽字符 API
#[cfg(windows)]
use std::os::raw::c_ushort as wchar_t;

// libraw_data_t 是一个不透明指针类型，用于表示 LibRaw 数据结构
// 我们使用空枚举来创建类型安全的指针，而不暴露内部结构
pub enum libraw_data_t {}

// LibRaw 处理后的图像数据结构
// 这个结构体表示 LibRaw 解码后的图像数据，包括缩略图和完整图像
#[repr(C)] // 确保 C 内存布局兼容性
pub struct libraw_processed_image_t {
    /// 图像格式类型 (JPEG = 1, Bitmap = 2)
    pub image_type: c_int,
    /// 图像高度（像素）
    pub height: c_ushort,
    /// 图像宽度（像素）
    pub width: c_ushort,
    /// 颜色通道数
    pub colors: c_ushort,
    /// 每像素位数
    pub bits: c_ushort,
    /// 图像数据大小（字节）
    pub data_size: u32,
    /// 图像数据（柔性数组成员，实际大小由 data_size 决定）
    /// 注意：在 Rust 中我们使用长度为 1 的数组来表示柔性数组成员
    pub data: [c_uchar; 1],
}

// === LibRaw EXIF/图像信息结构体（最小定义，仅包含所需字段） ===

/// 对应 libraw_iparams_t — 相机厂商/型号等
#[repr(C)]
pub struct LibRawIparams {
    pub guard: [u8; 4],
    pub make: [u8; 64],
    pub model: [u8; 64],
}

/// 对应 libraw_gps_info_t
#[repr(C)]
pub struct LibRawGpsInfo {
    pub latitude: [f32; 3],
    pub longitude: [f32; 3],
    pub gpstimestamp: [f32; 3],
    pub altitude: f32,
    pub altref: u8,
    pub latref: u8,
    pub longref: u8,
    pub gpsstatus: u8,
    pub gpsparsed: u8,
}

/// 对应 libraw_imgother_t — ISO、快门、光圈、焦距、时间戳、GPS
#[repr(C)]
pub struct LibRawImgother {
    pub iso_speed: f32,
    pub shutter: f32,
    pub aperture: f32,
    pub focal_len: f32,
    pub timestamp: i64,
    pub shot_order: u32,
    pub gpsdata: [u32; 32],
    pub parsed_gps: LibRawGpsInfo,
}

/// 对应 libraw_lensinfo_t — 镜头信息
#[repr(C)]
pub struct LibRawLensinfo {
    pub MinFocal: f32,
    pub MaxFocal: f32,
    pub MaxAp4MinFocal: f32,
    pub MaxAp4MaxFocal: f32,
    pub EXIF_MaxAp: f32,
    pub LensMake: [u8; 128],
    pub Lens: [u8; 128],
}

extern "C" {
    // === 库版本信息 ===
    /// 获取 LibRaw 版本字符串
    pub fn libraw_version() -> *const c_char;

    /// 获取 LibRaw 版本号（整数格式）
    pub fn libraw_versionNumber() -> c_int;

    // === 构造函数和析构函数 ===
    /// 初始化 LibRaw 实例
    /// flags: 初始化标志，通常使用 LIBRAW_OPTIONS_NONE
    /// 返回: 指向 libraw_data_t 的指针，失败时返回 NULL
    pub fn libraw_init(flags: c_int) -> *mut libraw_data_t;

    /// 关闭 LibRaw 实例并释放所有资源
    pub fn libraw_close(data: *mut libraw_data_t);

    // === 文件操作 ===
    /// 打开 RAW 文件
    /// data: LibRaw 实例指针
    /// filename: 文件名（UTF-8 字符串）
    /// 返回: LIBRAW_SUCCESS 表示成功，其他值表示错误
    pub fn libraw_open_file(data: *mut libraw_data_t, filename: *const c_char) -> c_int;

    /// Windows 平台：打开宽字符文件名
    #[cfg(windows)]
    pub fn libraw_open_wfile(data: *mut libraw_data_t, filename: *const wchar_t) -> c_int;

    /// 解包 RAW 文件数据（解析文件头和基本信息）
    pub fn libraw_unpack(data: *mut libraw_data_t) -> c_int;

    /// 处理 RAW 数据（去马赛克、色彩转换等）
    pub fn libraw_dcraw_process(data: *mut libraw_data_t) -> c_int;

    // === 缩略图操作 ===
    /// 解包缩略图数据
    pub fn libraw_unpack_thumb(data: *mut libraw_data_t) -> c_int;

    /// 从缩略图数据创建内存中的图像
    /// errc: 输出参数，接收错误代码
    /// 返回: 指向处理后图像的指针，失败时返回 NULL
    pub fn libraw_dcraw_make_mem_thumb(
        data: *mut libraw_data_t,
        errc: *mut c_int,
    ) -> *mut libraw_processed_image_t;

    /// 释放由 libraw_dcraw_make_mem_* 分配的内存

    // === 完整图像操作 ===
    /// 从完整 RAW 数据创建内存中的图像（经过 demosaic、白平衡等处理）
    /// errc: 输出参数，接收错误代码
    /// 返回: 指向处理后图像的指针，失败时返回 NULL
    pub fn libraw_dcraw_make_mem_image(
        data: *mut libraw_data_t,
        errc: *mut c_int,
    ) -> *mut libraw_processed_image_t;
    pub fn libraw_dcraw_clear_mem(img: *mut libraw_processed_image_t);

    // === 错误处理 ===
    /// 获取错误代码的描述字符串
    pub fn libraw_strerror(error_code: c_int) -> *const c_char;

    // === 内存管理 ===
    /// 回收 LibRaw 实例的数据流，准备处理新文件
    /// 这比 libraw_close 更轻量级，不会释放所有内存
    pub fn libraw_recycle(data: *mut libraw_data_t);
    /// 设置 half_size 输出参数（1 = 半尺寸去马赛克，4x 加速）
    pub fn libraw_set_half_size(data: *mut libraw_data_t, value: c_int);

    /// 设置是否使用相机白平衡（1 = 使用 RAW 文件中记录的拍摄白平衡）
    pub fn libraw_set_use_camera_wb(data: *mut libraw_data_t, value: c_int);

    // === 解码参数设置（LibRaw 官方 C API） ===
    /// 设置去马赛克算法（user_qual）：0=bilinear（最快）1=VNG 2=PPG 3=AHD（默认）
    pub fn libraw_set_demosaic(data: *mut libraw_data_t, value: c_int);

    /// 设置输出位深：8 或 16（8 位内存减半）
    pub fn libraw_set_output_bps(data: *mut libraw_data_t, value: c_int);

    /// 设置是否跳过自动亮度调整（1 = 跳过直方图扫描）
    pub fn libraw_set_no_auto_bright(data: *mut libraw_data_t, value: c_int);

    /// 设置输出色彩空间：0=RAW、1=sRGB（默认）、2=Adobe RGB、4=WideGamut、5=ProPhoto
    pub fn libraw_set_output_color(data: *mut libraw_data_t, value: c_int);

    /// 设置 gamma 曲线参数（index 0 = 幂次，index 1 = 斜率；1.0/1.0 = 线性输出）
    pub fn libraw_set_gamma(data: *mut libraw_data_t, index: c_int, value: f32);

    // === EXIF / 图像信息访问器 ===
    /// 获取 iparams 指针（make, model, 等）
    pub fn libraw_get_iparams(data: *mut libraw_data_t) -> *const LibRawIparams;
    /// 获取 imgother 指针（iso, shutter, aperture, focal_len, timestamp）
    pub fn libraw_get_imgother(data: *mut libraw_data_t) -> *const LibRawImgother;
    /// 获取 lensinfo 指针（镜头型号等）
    pub fn libraw_get_lensinfo(data: *mut libraw_data_t) -> *const LibRawLensinfo;
    /// 获取图像宽度
    pub fn libraw_get_iwidth(data: *mut libraw_data_t) -> c_int;
    /// 获取图像高度
    pub fn libraw_get_iheight(data: *mut libraw_data_t) -> c_int;

    // === 对焦点（C shim：focus_point.c） ===
    /// 读取 Fuji makernotes 对焦点像素坐标（相对未旋转传感器图；无记录返回 0）
    pub fn rawlib_get_focus_pixel(
        data: *mut libraw_data_t,
        out_x: *mut c_ushort,
        out_y: *mut c_ushort,
    ) -> c_int;
    /// 读取 makernotes AFInfo/AFInfo2 原始 blob（Nikon/Panasonic；无记录返回 0）
    pub fn rawlib_get_afinfo(
        data: *mut libraw_data_t,
        out_tag: *mut libc::c_uint,
        out_order: *mut libc::c_short,
        out_version: *mut libc::c_uint,
        out_len: *mut libc::c_uint,
        out_buf: *mut c_uchar,
        buf_cap: libc::c_uint,
    ) -> c_int;

    // === Panasonic AFPointPosition（C shim：focus_point.c 回调捕获） ===
    /// 分配 per-call 回调 context（无全局状态，支持并行提取）
    pub fn rawlib_pan_ctx_alloc() -> *mut libc::c_void;
    /// 释放回调 context
    pub fn rawlib_pan_ctx_free(ctx: *mut libc::c_void);
    /// 注册 Panasonic makernotes 回调（必须在 open 之前调用）
    pub fn rawlib_panasonic_af_init(data: *mut libraw_data_t, ctx: *mut libc::c_void);
    /// open 之后取归一化坐标（0–1）；无记录返回 0
    pub fn rawlib_panasonic_af_get(ctx: *mut libc::c_void, out_x: *mut libc::c_double, out_y: *mut libc::c_double) -> c_int;
}

// === LibRaw 初始化标志常量 ===
/// 无特殊选项
pub const LIBRAW_OPTIONS_NONE: c_int = 0;

// === LibRaw 返回代码常量 ===
/// 操作成功
pub const LIBRAW_SUCCESS: c_int = 0;

// === 图像格式常量 ===
/// JPEG 格式图像
pub const LIBRAW_IMAGE_JPEG: c_int = 1;

/// 位图格式图像（未压缩的 RGB 数据）
pub const LIBRAW_IMAGE_BITMAP: c_int = 2;
