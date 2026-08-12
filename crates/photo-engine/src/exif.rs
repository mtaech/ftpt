use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;
use thiserror::Error;
use photo_domain::{FocusPoint, FocusShape, ImageFormat};
use serde_json::Value as JsonValue;

use photo_domain::ExifMetadata;

/// EXIF 提取错误
#[derive(Error, Debug)]
pub enum ExifError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("EXIF parse error: {0}")]
    Parse(String),
    #[error("RAW EXIF error: {0}")]
    Raw(String),
    #[error("EXIF provider error: {0}")]
    Provider(String),
}

// ============================================================================
// EXIF 后端抽象（2026-08 重构：kamadak-exif 移除，exiftool 为主后端）
//
// 统一接口：photo-tauri 启动时经 [`init_provider`] 注入后端（默认 exiftool，
// 找不到二进制时回退 rawlib）；调用方永远走 [`extract_exif`] 模块函数，
// 不感知具体后端。对焦点等厂商私有字段由各后端自行解析（exiftool 输出
// SubjectArea/AFPointPosition/AFAreaX*，rawlib 输出 FocusPixel/AFInfo blob）。
// ============================================================================

/// EXIF 提取后端抽象
pub trait ExifProvider: Send + Sync {
    /// 提取单个文件 EXIF
    fn extract(&self, path: &Path, format: &ImageFormat) -> Result<ExifMetadata, ExifError>;

    /// 类型擦除访问（shutdown 时 downcast 到具体后端）
    fn as_any(&self) -> &dyn std::any::Any;

    /// 批量提取（后端可一次起多个子进程/单次命令处理，默认退化为逐文件）
    fn extract_batch(
        &self,
        files: &[(PathBuf, ImageFormat)],
    ) -> Vec<(PathBuf, Result<ExifMetadata, ExifError>)> {
        files
            .iter()
            .map(|(p, f)| (p.clone(), self.extract(p, f)))
            .collect()
    }
}

/// 全局 EXIF provider（photo-tauri 启动时设置；未设置时惰性取默认）
static PROVIDER: OnceLock<Arc<dyn ExifProvider>> = OnceLock::new();

/// 注入全局 provider（幂等：重复调用返回 Err 表示已被占用）
pub fn init_provider(p: Arc<dyn ExifProvider>) -> Result<(), Arc<dyn ExifProvider>> {
    PROVIDER.set(p)
}

/// 全局 provider：默认尝试 exiftool，失败回退 rawlib（RAW 可用，常规图无后端）
///
/// 测试环境（cfg(test)）不 spawn 真实 exiftool 进程——避免残留子进程
/// 拖住后续 cargo 命令（Windows 文件锁/句柄问题）；exiftool 的 JSON 映射
/// 由纯函数测试覆盖，进程层协议用 photo-engine example 手动验证。
fn provider() -> &'static dyn ExifProvider {
    PROVIDER
        .get_or_init(|| -> Arc<dyn ExifProvider> {
            #[cfg(test)]
            {
                return Arc::new(RawLibProvider);
            }
            #[cfg(not(test))]
            {
                match ExifToolProvider::spawn() {
                    Ok(p) => Arc::new(p),
                    Err(e) => {
                        tracing::warn!("exiftool 不可用（{e}），回退 rawlib 后端");
                        Arc::new(RawLibProvider)
                    }
                }
            }
        })
        .as_ref()
}

/// 关闭全局 exiftool 长驻进程（photo-tauri 退出时调用；无 exiftool 时 no-op）
pub fn shutdown_provider() {
    if let Some(p) = PROVIDER.get() {
        if let Some(t) = p.as_ref().as_any().downcast_ref::<ExifToolProvider>() {
            t.terminate();
        }
    }
}

/// 统一入口：委托全局 provider 提取，失败时记录 warning
pub fn extract_exif(path: &Path, format: &ImageFormat) -> Result<ExifMetadata, ExifError> {
    let result = provider().extract(path, format);
    if let Err(e) = &result {
        tracing::warn!("EXIF 提取失败 {} (格式 {:?}): {e}", path.display(), format);
    }
    result
}

// ============================================================================
// exiftool 后端（主）：-stay_open 长驻进程 + JSON 输出
// ============================================================================

/// 提取的 exiftool tag 列表（# 后缀 = 数值格式）
const EXIFTOOL_TAGS: &[&str] = &[
    "-Make", "-Model", "-LensModel", "-Lens",
    "-ExposureTime", "-FNumber", "-ISO", "-FocalLength",
    "-ExposureCompensation", "-WhiteBalance", "-DateTimeOriginal",
    "-ImageWidth", "-ImageHeight", "-Orientation#", "-ColorSpace",
    "-GPSLatitude#", "-GPSLongitude#", "-GPSAltitude#",
    "-SubjectArea", "-AFPointPosition", "-AFAreaSize",
    "-AFImageWidth", "-AFAreaXPosition", "-AFAreaYPosition",
    "-AFAreaWidth", "-AFAreaHeight",
];

/// exiftool 长驻会话（-stay_open True -@ -，stdin 写参数、stdout 读 JSON）
struct ExifToolSession {
    #[allow(dead_code)]
    child: Child,
    stdin: BufWriter<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

impl Drop for ExifToolSession {
    fn drop(&mut self) {
        // 通知 exiftool 退出并回收子进程（防泄漏；ExifToolProvider 销毁时触发）
        let _ = writeln!(self.stdin, "-stay_open");
        let _ = writeln!(self.stdin, "False");
        let _ = writeln!(self.stdin, "-execute");
        let _ = self.stdin.flush();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl ExifToolSession {
    /// 执行一次参数列表：写 stdin + `-execute`，读 stdout 直到 `{ready}` 标记
    fn execute(&mut self, args: &[String]) -> Result<String, ExifError> {
        for a in args {
            writeln!(self.stdin, "{a}").map_err(|e| ExifError::Provider(format!("写 exiftool stdin 失败: {e}")))?;
        }
        writeln!(self.stdin, "-execute")
            .map_err(|e| ExifError::Provider(format!("写 exiftool execute 失败: {e}")))?;
        self.stdin.flush().map_err(|e| ExifError::Provider(format!("flush exiftool stdin 失败: {e}")))?;
        let mut out = String::new();
        let mut line = String::new();
        let mut lines = 0u32;
        loop {
            line.clear();
            let n = self
                .stdout
                .read_line(&mut line)
                .map_err(|e| ExifError::Provider(format!("读 exiftool stdout 失败: {e}")))?;
            if n == 0 {
                return Err(ExifError::Provider("exiftool 提前退出（进程崩溃？）".into()));
            }
            if line.trim() == "{ready}" {
                break;
            }
            out.push_str(&line);
            lines += 1;
            // 防御：异常文件可能让 exiftool 刷出大量错误行，限制读取上限避免挂起
            if lines > 20_000 || out.len() > 8 * 1024 * 1024 {
                return Err(ExifError::Provider("exiftool 输出异常（超出上限）".into()));
            }
        }
        Ok(out)
    }
}

/// exiftool 后端（主）：长驻进程复用，避免每次 spawn 的 Perl 启动开销
pub struct ExifToolProvider {
    inner: Mutex<ExifToolSession>,
}

impl ExifToolProvider {
    /// 定位 exiftool 根目录/二进制：env PHOTO_EXIFTOOL → exe 同级 exiftool/ →
    /// 仓库 local-lib/exiftool/（开发/测试）→ PATH
    #[doc(hidden)]
    pub fn debug_find_root() -> Option<PathBuf> {
        Self::find_root()
    }
    #[doc(hidden)]
    pub fn debug_resolve() -> Option<(PathBuf, Vec<String>)> {
        Self::resolve_cmd()
    }
    /// 定位 exiftool 根目录/二进制：env PHOTO_EXIFTOOL → exe 同级 exiftool/ →
    /// 仓库 local-lib/exiftool/（开发/测试）→ PATH
    fn find_root() -> Option<PathBuf> {
        // env 覆盖：可指向文件或目录
        if let Ok(p) = std::env::var("PHOTO_EXIFTOOL") {
            let p = PathBuf::from(p);
            if p.exists() {
                return Some(p);
            }
        }
        // 打包布局：exe 同级 exiftool/ 目录
        if let Ok(exe) = std::env::current_exe() {
            if let Some(d) = exe.parent() {
                let dir = d.join("exiftool");
                if dir.is_dir() {
                    return Some(dir);
                }
                let f = d.join("exiftool.exe");
                if f.exists() {
                    return Some(f);
                }
            }
            // 开发/测试布局：向上找仓库 local-lib/exiftool/
            let mut dir = exe.parent();
            for _ in 0..5 {
                if let Some(d) = dir {
                    let cand = d.join("local-lib").join("exiftool");
                    if cand.is_dir() {
                        return Some(cand);
                    }
                    dir = d.parent();
                } else {
                    break;
                }
            }
        }
        // PATH
        for dir in std::env::split_paths(&std::env::var("PATH").unwrap_or_default()) {
            for name in ["exiftool.exe", "exiftool"] {
                let p = dir.join(name);
                if p.exists() {
                    return Some(p);
                }
            }
        }
        None
    }

    /// 解析实际执行命令：(program, args 前缀)
    ///
    /// Windows 官方包为 exiftool(-k).exe（内嵌 -k，每个命令后等 ENTER，
    /// 不适合程序化调用）→ 用同目录 perl.exe + exiftool.pl。
    /// Linux 源码包为纯 Perl 脚本 → 用系统 perl 执行。
    fn resolve_cmd() -> Option<(PathBuf, Vec<String>)> {
        let root = Self::find_root()?;
        if root.is_file() {
            // 用户显式指定二进制（如 PATH 里的正式 exiftool）
            return Some((root, Vec::new()));
        }
        // 目录：按平台解析（local-lib/exiftool/{windows,linux} 固定布局，
        // 版本号记录在 VERSION.txt，升级时覆盖对应平台目录即可）
        #[cfg(windows)]
        {
            let win = root.join("windows");
            let perl = win.join("exiftool_files").join("perl.exe");
            let pl = win.join("exiftool_files").join("exiftool.pl");
            if perl.exists() && pl.exists() {
                return Some((perl, vec![pl.to_string_lossy().into_owned()]));
            }
            let exe = win.join("exiftool.exe");
            if exe.exists() {
                return Some((exe, Vec::new()));
            }
        }
        #[cfg(not(windows))]
        {
            // Linux 源码包：local-lib/exiftool/linux/exiftool 脚本（依赖系统 perl）
            let script = root.join("linux").join("exiftool");
            if script.exists() {
                return Some((PathBuf::from("perl"), vec![script.to_string_lossy().into_owned()]));
            }
        }
        None
    }

    /// 启动长驻进程（自动定位 + 平台解析执行方式）
    pub fn spawn() -> Result<Self, ExifError> {
        let (prog, prefix) = Self::resolve_cmd().ok_or_else(|| {
            ExifError::Provider(
                "未找到 exiftool（env PHOTO_EXIFTOOL / exe 同级 exiftool/ / local-lib / PATH）".into(),
            )
        })?;
        Self::spawn_with(&prog, &prefix)
    }

    /// 用解析出的 (program, args 前缀) 启动长驻进程
    pub fn spawn_with(program: &Path, prefix: &[String]) -> Result<Self, ExifError> {
        let mut cmd = Command::new(program);
        cmd.args(prefix);
        let mut child = cmd
            .args(["-stay_open", "True", "-@", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| ExifError::Provider(format!("启动 exiftool 失败: {e}")))?;
        let stdin = BufWriter::new(
            child
                .stdin
                .take()
                .ok_or_else(|| ExifError::Provider("exiftool stdin 不可用".into()))?,
        );
        let stdout = BufReader::new(
            child
                .stdout
                .take()
                .ok_or_else(|| ExifError::Provider("exiftool stdout 不可用".into()))?,
        );
        Ok(Self {
            inner: Mutex::new(ExifToolSession {
                child,
                stdin,
                stdout,
            }),
        })
    }

    /// 终止长驻进程（应用退出时调用；session Drop 也会 kill，这里主动触发）
    pub fn terminate(&self) {
        if let Ok(mut s) = self.inner.lock() {
            // 通知退出并回收（写失败/已死则静默）
            let _ = writeln!(s.stdin, "-stay_open");
            let _ = writeln!(s.stdin, "False");
            let _ = writeln!(s.stdin, "-execute");
            let _ = s.stdin.flush();
            let _ = s.child.kill();
            let _ = s.child.wait();
        }
    }

    /// 构造一次提取命令的参数（JSON + tag 列表 + 路径）
    /// 注意：不能用 -q——它同时抑制 -stay_open 模式的 {ready} 标记，
    /// 导致 execute 读不到结果边界而挂起。
    fn command_args(paths: &[&Path]) -> Vec<String> {
        let mut args = vec![
            "-json".into(),
            "-charset".into(),
            "filename=UTF8".into(),
        ];
        args.extend(EXIFTOOL_TAGS.iter().map(|t| (*t).to_string()));
        args.extend(paths.iter().map(|p| p.to_string_lossy().into_owned()));
        args
    }
}

impl ExifProvider for ExifToolProvider {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn extract(&self, path: &Path, _format: &ImageFormat) -> Result<ExifMetadata, ExifError> {
        let mut session = self
            .inner
            .lock()
            .map_err(|e| ExifError::Provider(format!("exiftool 会话锁中毒: {e}")))?;
        let args = Self::command_args(std::slice::from_ref(&path));
        let out = session.execute(&args)?;
        let arr: Vec<JsonValue> = serde_json::from_str(&out)
            .map_err(|e| ExifError::Parse(format!("exiftool JSON 解析失败: {e}")))?;
        let v = arr
            .first()
            .ok_or_else(|| ExifError::Parse("exiftool 返回空结果".into()))?;
        exif_json_to_meta(v, path)
    }

    fn extract_batch(
        &self,
        files: &[(PathBuf, ImageFormat)],
    ) -> Vec<(PathBuf, Result<ExifMetadata, ExifError>)> {
        // 一次命令处理全部文件（-stay_open 单次 execute 多路径），大幅降低进程往返
        let mut session = match self.inner.lock() {
            Ok(s) => s,
            Err(_) => {
                return files
                    .iter()
                    .map(|(p, _)| (p.clone(), Err(ExifError::Provider("exiftool 会话锁中毒".into()))))
                    .collect()
            }
        };
        let paths: Vec<&Path> = files.iter().map(|(p, _)| p.as_path()).collect();
        let args = Self::command_args(&paths);
        let out = match session.execute(&args) {
            Ok(o) => o,
            Err(e) => {
                return files
                    .iter()
                    .map(|(p, _)| {
                        (
                            p.clone(),
                            Err(ExifError::Provider(format!("exiftool 批量提取失败: {e}"))),
                        )
                    })
                    .collect()
            }
        };
        let arr: Vec<JsonValue> = serde_json::from_str(&out).unwrap_or_default();
        // 按 SourceFile 匹配回每个文件（exiftool 与输入同序）
        let mut map = std::collections::HashMap::new();
        for v in arr {
            if let Some(sf) = v.get("SourceFile").and_then(|x| x.as_str()) {
                map.insert(sf.to_string(), v);
            }
        }
        files
            .iter()
            .map(|(p, _)| {
                let key = p.to_string_lossy().replace('\\', "/");
                let res = map.get(&key).map(|v| exif_json_to_meta(v, p)).unwrap_or_else(|| {
                    Err(ExifError::Parse(format!("exiftool 未返回 {} 的结果", p.display())))
                });
                (p.clone(), res)
            })
            .collect()
    }
}

/// 快门秒数 → 显示字符串（对齐 rawlib 格式：整倒数 → "1/N"，否则秒）
fn format_shutter(secs: f64) -> String {
    if secs <= 0.0 {
        return String::new();
    }
    let inv = (1.0 / secs).round();
    if (inv - (1.0 / secs)).abs() < 0.01 && inv <= 100_000.0 {
        format!("1/{inv}")
    } else {
        format!("{secs:.4}s")
    }
}

/// exiftool JSON 对象 → ExifMetadata
fn exif_json_to_meta(v: &JsonValue, path: &Path) -> Result<ExifMetadata, ExifError> {
    let mut meta = ExifMetadata::default();
    // 字符串 tag（空串视为缺失）
    let str_opt = |k: &str| v.get(k).and_then(|x| x.as_str()).filter(|s| !s.is_empty()).map(str::to_string);
    meta.camera.make = str_opt("Make");
    meta.camera.model = str_opt("Model");
    meta.camera.lens = str_opt("LensModel").or_else(|| str_opt("Lens"));
    meta.date_time_original = str_opt("DateTimeOriginal");
    meta.shooting.white_balance = str_opt("WhiteBalance");
    meta.color_space = str_opt("ColorSpace");

    // 数值/字符串混合 tag
    meta.shooting.exposure_time = str_opt("ExposureTime").or_else(|| {
        v.get("ExposureTime")
            .and_then(|x| x.as_f64())
            .map(format_shutter)
            .filter(|s| !s.is_empty())
    });
    meta.shooting.f_number = v
        .get("FNumber")
        .and_then(|x| x.as_f64())
        .map(|f| format!("{f}"))
        .or_else(|| str_opt("FNumber"));
    meta.shooting.iso = v
        .get("ISO")
        .and_then(|x| x.as_u64())
        .map(|x| x as u32)
        .or_else(|| str_opt("ISO").and_then(|s| s.parse().ok()));
    meta.shooting.focal_length = str_opt("FocalLength");
    meta.shooting.exposure_compensation = str_opt("ExposureCompensation").or_else(|| {
        v.get("ExposureCompensation")
            .and_then(|x| x.as_f64())
            .map(|f| format!("{f}"))
    });

    meta.image_width = v.get("ImageWidth").and_then(|x| x.as_u64()).map(|x| x as u32);
    meta.image_height = v.get("ImageHeight").and_then(|x| x.as_u64()).map(|x| x as u32);
    meta.orientation = v.get("Orientation").and_then(|x| x.as_u64()).map(|x| x as u16);

    // GPS：exiftool `-GPSLatitude#` 输出十进制度（南纬/西经已带符号）
    // 存 (deg, 0, 0)，enrich_with_exif 的 dms_to_decimal(deg,0,0) = deg 原样
    meta.gps.latitude = v.get("GPSLatitude").and_then(|x| x.as_f64()).map(|d| (d, 0.0, 0.0));
    meta.gps.longitude = v.get("GPSLongitude").and_then(|x| x.as_f64()).map(|d| (d, 0.0, 0.0));
    meta.gps.altitude = v.get("GPSAltitude").and_then(|x| x.as_f64());

    // 对焦点：SubjectArea（JPEG）→ Panasonic AFPointPosition+AFAreaSize → Nikon AFAreaX*
    meta.focus_point = focus_from_exif_json(v);

    if let Ok(fs) = std::fs::metadata(path) {
        meta.file_size = Some(fs.len());
    }
    Ok(meta)
}

/// 从 exiftool JSON 构造对焦点（优先级：JPEG SubjectArea > Panasonic > Nikon）
fn focus_from_exif_json(v: &JsonValue) -> Option<FocusPoint> {
    // 1) JPEG 标准 SubjectArea（像素坐标，需尺寸 + orientation 归一化）
    if let Some(s) = v.get("SubjectArea").and_then(|x| x.as_str()) {
        if let (Some(iw), Some(ih)) = (
            v.get("ImageWidth").and_then(|x| x.as_u64()),
            v.get("ImageHeight").and_then(|x| x.as_u64()),
        ) {
            if iw > 0 && ih > 0 {
                if let Some((shape, x, y, w, h)) = subject_area_str(s) {
                    return Some(normalize_focus(
                        shape,
                        x,
                        y,
                        w,
                        h,
                        v.get("Orientation").and_then(|x| x.as_u64()).map(|x| x as u16),
                        iw as f64,
                        ih as f64,
                    ));
                }
            }
        }
    }
    // 2) Panasonic AFPointPosition（中心 0–1）+ AFAreaSize（区域尺寸 0–1）
    if let Some((cx, cy)) = pair_str(v.get("AFPointPosition").and_then(|x| x.as_str()).unwrap_or("")) {
        if let Some((w, h)) = pair_str(v.get("AFAreaSize").and_then(|x| x.as_str()).unwrap_or("")) {
            if w > 0.0 && h > 0.0 && cx >= 0.0 && cx <= 1.0 && cy >= 0.0 && cy <= 1.0 {
                return Some(FocusPoint::rectangle(
                    (cx - w / 2.0) as f32,
                    (cy - h / 2.0) as f32,
                    w as f32,
                    h as f32,
                ));
            }
            return Some(FocusPoint::point(cx as f32, cy as f32));
        }
        return Some(FocusPoint::point(cx as f32, cy as f32));
    }
    // 3) Nikon AFImageWidth + AFAreaX/YPosition（AF 图像坐标系，同本地 parse_af_info）
    if let Some(img_w) = v.get("AFImageWidth").and_then(|x| x.as_f64()) {
        if img_w > 0.0 {
            let get = |k: &str| v.get(k).and_then(|x| x.as_f64());
            if let (Some(xp), Some(yp)) = (get("AFAreaXPosition"), get("AFAreaYPosition")) {
                let img_h = get("AFImageHeight").unwrap_or(img_w);
                if img_h > 0.0 {
                    let (w, h) = (get("AFAreaWidth").unwrap_or(0.0), get("AFAreaHeight").unwrap_or(0.0));
                    return Some(FocusPoint::rectangle(
                        ((xp - w / 2.0) / img_w) as f32,
                        ((yp - h / 2.0) / img_h) as f32,
                        (w / img_w) as f32,
                        (h / img_h) as f32,
                    ));
                }
            }
        }
    }
    None
}

/// 解析 "x y" 对（空白分隔）
fn pair_str(s: &str) -> Option<(f64, f64)> {
    let mut it = s.split_whitespace();
    let x = it.next()?.parse().ok()?;
    let y = it.next()?.parse().ok()?;
    Some((x, y))
}

/// 解析 exiftool SubjectArea 字符串："x y" 点 / "x y d" 圆 / "x y w h" 矩形（像素）
fn subject_area_str(s: &str) -> Option<(FocusShape, f64, f64, f64, f64)> {
    let nums: Vec<f64> = s.split_whitespace().filter_map(|t| t.parse().ok()).collect();
    match nums.as_slice() {
        [x, y] => Some((FocusShape::Point, *x, *y, 0.0, 0.0)),
        [x, y, d] => Some((FocusShape::Circle, *x, *y, *d, *d)),
        [x, y, w, h] => Some((FocusShape::Rectangle, *x, *y, *w, *h)),
        _ => None,
    }
}

/// 像素矩形按 EXIF orientation 旋转（相对原始图像坐标，原点左上）。
/// 返回旋转后的 (左上角 x, 左上角 y, 宽, 高)：
/// - 3 (180°)：(W-x-w, H-y-h)，尺寸不变
/// - 6 (90° CW)：(H-y-h, x)，宽高交换
/// - 8 (270° CW)：(y, W-x-w)，宽高交换
/// 其余（1/2/4/5/7 或未知）按不旋转处理。
fn rotate_focus_rect(
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    orientation: Option<u16>,
    img_w: f64,
    img_h: f64,
) -> (f64, f64, f64, f64) {
    match orientation {
        Some(3) => (img_w - x - w, img_h - y - h, w, h),
        Some(6) => (img_h - y - h, x, h, w),
        Some(8) => (y, img_w - x - w, h, w),
        _ => (x, y, w, h),
    }
}

/// 对焦点像素值 → 归一化 FocusPoint（相对 orientation 修正后的显示方向）。
/// 统一以「左上角 + 宽高」矩形表达旋转，再按 shape 还原语义：
/// Point 取旋转后点坐标；Circle 由旋转后外接矩还原圆心 + 直径（圆旋转后不变形）；
/// Rectangle 直接用旋转后左上角 + 交换后的宽高。
fn normalize_focus(
    shape: FocusShape,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    orientation: Option<u16>,
    img_w: f64,
    img_h: f64,
) -> FocusPoint {
    let (rect_x, rect_y, rect_w, rect_h) = match shape {
        FocusShape::Point => (x, y, 0.0, 0.0),
        // 圆转外接矩参与旋转：圆心 (x,y)、直径 d
        FocusShape::Circle => (x - w / 2.0, y - h / 2.0, w, h),
        FocusShape::Rectangle => (x, y, w, h),
    };
    let (rx, ry, rw, rh) =
        rotate_focus_rect(rect_x, rect_y, rect_w, rect_h, orientation, img_w, img_h);
    // 显示尺寸：90°/270° 旋转后宽高互换
    let (dw, dh) = match orientation {
        Some(6) | Some(8) => (img_h, img_w),
        _ => (img_w, img_h),
    };
    if dw <= 0.0 || dh <= 0.0 {
        return FocusPoint::point(0.0, 0.0);
    }
    match shape {
        FocusShape::Point => FocusPoint::point((rx / dw) as f32, (ry / dh) as f32),
        FocusShape::Circle => FocusPoint::circle(
            ((rx + rw / 2.0) / dw) as f32,
            ((ry + rh / 2.0) / dh) as f32,
            (rw / dw) as f32,
        ),
        FocusShape::Rectangle => FocusPoint::rectangle(
            (rx / dw) as f32,
            (ry / dh) as f32,
            (rw / dw) as f32,
            (rh / dh) as f32,
        ),
    }
}

// ============================================================================
// rawlib 后端（回退）：LibRaw 直接读 EXIF 结构体（Fuji FocusPixel / Nikon
// AFInfo blob 本地解析）。仅 RAW 有数据，常规图无后端（exiftool 不可用时）。
// ============================================================================

pub struct RawLibProvider;

impl ExifProvider for RawLibProvider {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn extract(&self, path: &Path, format: &ImageFormat) -> Result<ExifMetadata, ExifError> {
        match format {
            ImageFormat::Raw(_) => extract_exif_raw(path),
            _ => Err(ExifError::Provider(format!(
                "rawlib 后端不支持常规图 {}（exiftool 不可用）",
                path.display()
            ))),
        }
    }
}

/// 从 RAW 文件读取 EXIF（rawlib 回退后端：LibRaw 直接读 EXIF 结构体，
/// 支持 RW2 等非标准 TIFF 魔数；Fuji FocusPixel / Nikon AFInfo blob 本地解析）
fn extract_exif_raw(path: &Path) -> Result<ExifMetadata, ExifError> {
    let path_str = path.to_string_lossy();
    let raw_exif = rawlib::exif::extract_exif(path_str.as_ref())
        .map_err(|e| ExifError::Raw(e.to_string()))?;
    Ok(raw_exif_to_meta(raw_exif, path))
}

/// 从 rawlib::ExifData 转为 ExifMetadata，共用逻辑
///
/// GPS 与常规图路径同构：rawlib::exif 已支持 GPS DMS 解析，raw_exif.gps_latitude/
/// gps_longitude 为 (度, 分, 秒) 元组（rawlib 侧已按 Ref 施加南纬/西经符号到度分量），
/// 与 `extract_exif_regular` 的输出格式一致——RAW 不需要 fallback None 的特例，
/// 十进制转换统一在 `CaptureMeta::enrich_with_exif`（photo_domain::dms_to_decimal）完成。
fn raw_exif_to_meta(raw_exif: rawlib::ExifData, path: &Path) -> ExifMetadata {
    let mut meta = ExifMetadata::default();
    meta.camera.make = raw_exif.make;
    meta.camera.model = raw_exif.model;
    meta.camera.lens = raw_exif.lens_model;
    meta.date_time_original = raw_exif.date_time_original;
    meta.shooting.exposure_time = raw_exif.exposure_time;
    meta.shooting.f_number = raw_exif.f_number;
    meta.shooting.iso = raw_exif.iso;
    meta.shooting.focal_length = raw_exif.focal_length;
    let (iw, ih) = (raw_exif.image_width, raw_exif.image_height);
    meta.image_width = iw;
    meta.image_height = ih;
    meta.orientation = raw_exif.orientation;
    if let Some(lat) = raw_exif.gps_latitude {
        meta.gps.latitude = Some(lat);
    }
    if let Some(lon) = raw_exif.gps_longitude {
        meta.gps.longitude = Some(lon);
    }
    meta.gps.altitude = raw_exif.gps_altitude;
    // 对焦点：Fuji FocusPixel 优先（点像素坐标）；Nikon AFInfo2 提供矩形区域
    // （AFImage 坐标系中心 + 尺寸）。rawlib 不暴露 orientation，坐标相对
    // 未旋转传感器图，与 RAW 路径现有尺寸/方向一致。
    if let Some((fx, fy)) = raw_exif.focus_pixel {
        if let (Some(w), Some(h)) = (iw, ih) {
            if w > 0 && h > 0 {
                meta.focus_point = Some(normalize_focus(
                    FocusShape::Point,
                    f64::from(fx),
                    f64::from(fy),
                    0.0,
                    0.0,
                    None,
                    f64::from(w),
                    f64::from(h),
                ));
            }
        }
    } else if let Some(af) = raw_exif.af_info.as_ref() {
        if let Some(fp) = parse_af_info(af) {
            meta.focus_point = Some(fp);
        }
    } else if let Some((px, py)) = raw_exif.panasonic_focus {
        // Panasonic AFPointPosition：X/Y 已归一化 0–1（相机侧），直接点焦点
        meta.focus_point = Some(FocusPoint::point(px as f32, py as f32));
    }
    if let Ok(fs) = std::fs::metadata(path) {
        meta.file_size = Some(fs.len());
    }
    meta
}


/// 解析 Nikon AFInfo2（tag 0x00b7）blob 为归一化对焦点矩形。
///
/// 格式依据 ExifTool Nikon.pm（AFInfo2V0300/V0400 布局）。libraw 存储时已跳过
/// 前 4 字节版本号，blob 从 AFDetectionMethod 开始；坐标前提是
/// AFCoordinatesAvailable（blob[3]）== 1，否则只有对焦点编号（需机型布局表，不支持）。
/// 坐标语义：AFAreaX/YPosition 为区域中心、AFAreaWidth/Height 为尺寸，均位于
/// AFImageWidth/Height 坐标系（Z 系列 = 全图分辨率），归一化即相对全图。
///
/// 布局（blob 偏移 = ExifTool 偏移 - 4）：
/// - V0300（Expeed 6：Z5/Z6/Z7 等）：AFImageWidth 0x26、Height 0x28、
///   XPosition 0x2a、YPosition 0x2c、Width 0x2e、Height 0x30
/// - V0400+（Expeed 7：Z8/Z9/Zf 等）：AFImageWidth 0x3a、Height 0x3c、
///   XPosition 0x3e、YPosition 0x40、Width 0x42、Height 0x44
/// 字节序由 af.order 决定（0x4949 小端 / 0x4d4d 大端）。
fn parse_af_info(af: &rawlib::AfInfoData) -> Option<FocusPoint> {
    // 仅处理 AFInfo2 且坐标可用（blob[3] == 1）；AFInfo 老版（0x0088）无坐标
    if af.tag != 0x00b7 {
        return None;
    }
    if af.data.len() < 4 || af.data[3] != 1 {
        return None;
    }
    // 版本决定坐标偏移（V0300/V0400 两套布局）
    let off = match af.version {
        300 => (0x26usize, 0x28, 0x2a, 0x2c, 0x2e, 0x30), // V0300
        v if v >= 400 && v < 500 => (0x3a, 0x3c, 0x3e, 0x40, 0x42, 0x44), // V0400/V0401/V0402
        _ => return None,
    };
    let (iw, ih, xp, yp, w, h) = off;
    if af.data.len() < h + 2 {
        return None;
    }
    let u16_at = |i: usize| -> u16 {
        if af.order == 0x4d4d {
            u16::from_be_bytes([af.data[i], af.data[i + 1]])
        } else {
            u16::from_le_bytes([af.data[i], af.data[i + 1]])
        }
    };
    let img_w = u16_at(iw);
    let img_h = u16_at(ih);
    if img_w == 0 || img_h == 0 {
        return None;
    }
    let cx = f64::from(u16_at(xp));
    let cy = f64::from(u16_at(yp));
    let cw = f64::from(u16_at(w));
    let ch = f64::from(u16_at(h));
    // 中心 + 尺寸 → 归一化左上角矩形；坐标越界由 FocusPoint::rectangle 夹紧
    Some(FocusPoint::rectangle(
        ((cx - cw / 2.0) / f64::from(img_w)) as f32,
        ((cy - ch / 2.0) / f64::from(img_h)) as f32,
        (cw / f64::from(img_w)) as f32,
        (ch / f64::from(img_h)) as f32,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// 创建一个包含基本 EXIF 的测试 JPEG 文件
    /// 使用 image crate 写一个简单图片，然后附加 EXIF 头
    fn create_test_jpeg_with_exif(dir: &TempDir, name: &str) -> std::path::PathBuf {
        let path = dir.path().join(name);
        // 使用 image crate 生成 160x120 的 JPEG
        let img = image::RgbImage::new(160, 120);
        img.save(&path).unwrap();
        path
    }

    #[test]
    fn test_rawlib_provider_rejects_regular_images() {
        // 无 exiftool 环境（测试/CI）：rawlib 后端不支持常规图。
        // 直接构造 RawLibProvider 验证，避免触发全局 provider 启动真实 exiftool。
        let dir = TempDir::new().unwrap();
        let path = create_test_jpeg_with_exif(&dir, "test.jpg");
        let provider = RawLibProvider;
        match provider.extract(&path, &ImageFormat::Jpeg) {
            Err(ExifError::Provider(_)) => {}
            other => panic!("expected Provider error, got {other:?}"),
        }
    }

    #[test]
    fn test_extract_exif_nonexistent_file() {
        let path = std::path::Path::new("/nonexistent/photo.jpg");
        let result = extract_exif(path, &ImageFormat::Jpeg);
        assert!(result.is_err());
    }

    #[test]
    fn test_default_exif_metadata_is_empty() {
        let meta = ExifMetadata::default();
        assert!(meta.camera.make.is_none());
        assert!(meta.camera.model.is_none());
        assert!(meta.shooting.iso.is_none());
        assert!(meta.image_width.is_none());
        assert!(meta.image_height.is_none());
        assert!(meta.file_size.is_none());
    }

    #[test]
    fn test_extract_exif_raw_handles_nonexistent() {
        let path = std::path::Path::new("/nonexistent/photo.nef");
        let result = extract_exif(path, &ImageFormat::Raw("NEF".into()));
        // RAW 文件不存在：可能是 IO 错误或 Raw 错误
        assert!(result.is_err());
    }

    #[test]
    fn test_focus_from_exif_json_panasonic_and_nikon() {
        // Panasonic：AFPointPosition（中心 0–1）+ AFAreaSize
        let pan = serde_json::json!({
            "AFPointPosition": "0.5 0.5",
            "AFAreaSize": "0.1 0.15",
        });
        let fp = focus_from_exif_json(&pan).expect("Panasonic 应有对焦点");
        assert_eq!(fp.shape, FocusShape::Rectangle);
        assert!((fp.x - 0.45).abs() < 1e-4);
        assert!((fp.y - 0.425).abs() < 1e-4);
        assert!((fp.width - 0.1).abs() < 1e-4);
        assert!((fp.height - 0.15).abs() < 1e-4);

        // Nikon：AFImageWidth + AFAreaX/YPosition（AF 图像坐标系）
        let nikon = serde_json::json!({
            "AFImageWidth": 8256,
            "AFImageHeight": 5504,
            "AFAreaXPosition": 5423,
            "AFAreaYPosition": 4127,
            "AFAreaWidth": 807,
            "AFAreaHeight": 857,
        });
        let fp = focus_from_exif_json(&nikon).expect("Nikon 应有对焦点");
        assert_eq!(fp.shape, FocusShape::Rectangle);
        assert!((fp.x - (5423.0 / 8256.0 - 807.0 / 8256.0 / 2.0) as f32).abs() < 1e-4);
        assert!((fp.width - (807.0 / 8256.0) as f32).abs() < 1e-4);

        // JPEG SubjectArea 字符串（像素 + 尺寸 + orientation）
        let jpeg = serde_json::json!({
            "SubjectArea": "100 50 200 100",
            "ImageWidth": 400,
            "ImageHeight": 200,
            "Orientation": 6,
        });
        let fp = focus_from_exif_json(&jpeg).expect("JPEG SubjectArea 应有对焦点");
        // orientation 6（90° CW）矩形旋转后左上角 (H-y-h, x, h, w) = (50, 100, 100, 200)
        // 显示 200×400 → (0.25, 0.25, 0.5, 0.5)
        assert!((fp.x - 0.25).abs() < 1e-4);
        assert!((fp.y - 0.25).abs() < 1e-4);

        // 无任何对焦数据 → None
        assert!(focus_from_exif_json(&serde_json::json!({})).is_none());
    }

    #[test]
    fn test_exif_json_to_meta_basic_fields() {
        let v = serde_json::json!({
            "Make": "NIKON CORPORATION",
            "Model": "NIKON Z 7",
            "ExposureTime": "1/400",
            "FNumber": 7.1,
            "ISO": 400,
            "FocalLength": "500.0 mm",
            "WhiteBalance": "Auto",
            "DateTimeOriginal": "2026:06:19 14:40:12",
            "ImageWidth": 8256,
            "ImageHeight": 5504,
            "Orientation": 1,
            "ColorSpace": "sRGB",
            "GPSLatitude": 39.9,
            "GPSLongitude": -116.4,
        });
        let path = std::path::Path::new("/nonexistent");
        let meta = exif_json_to_meta(&v, path).expect("映射不应失败");
        assert_eq!(meta.camera.make.as_deref(), Some("NIKON CORPORATION"));
        assert_eq!(meta.shooting.exposure_time.as_deref(), Some("1/400"));
        assert_eq!(meta.shooting.f_number.as_deref(), Some("7.1"));
        assert_eq!(meta.shooting.iso, Some(400));
        assert_eq!(meta.image_width, Some(8256));
        assert_eq!(meta.orientation, Some(1));
        assert_eq!(meta.gps.latitude, Some((39.9, 0.0, 0.0)));
        assert_eq!(meta.gps.longitude, Some((-116.4, 0.0, 0.0)));
    }

    // ── 对焦点解析（SubjectArea 字符串）──────────────────────────

    #[test]
    fn test_subject_area_str_point_circle_rectangle() {
        // 2 个数 = 点
        let (shape, x, y, w, h) = subject_area_str("100 200").unwrap();
        assert_eq!(shape, FocusShape::Point);
        assert_eq!((x, y, w, h), (100.0, 200.0, 0.0, 0.0));

        // 3 个数 = 圆（圆心 + 直径）
        let (shape, x, y, w, h) = subject_area_str("100 200 50").unwrap();
        assert_eq!(shape, FocusShape::Circle);
        assert_eq!((x, y, w, h), (100.0, 200.0, 50.0, 50.0));

        // 4 个数 = 矩形（左上角 + 宽高）
        let (shape, x, y, w, h) = subject_area_str("100 200 60 40").unwrap();
        assert_eq!(shape, FocusShape::Rectangle);
        assert_eq!((x, y, w, h), (100.0, 200.0, 60.0, 40.0));

        // 空/不足 → None
        assert!(subject_area_str("").is_none());
        assert!(subject_area_str("100").is_none());
    }

    #[test]
    fn test_normalize_focus_orientation_identity() {
        // 无旋转：点 (100, 50) 在 400×200 图上 → (0.25, 0.25)
        let p = normalize_focus(FocusShape::Point, 100.0, 50.0, 0.0, 0.0, None, 400.0, 200.0);
        assert_eq!(p, FocusPoint::point(0.25, 0.25));

        // 矩形：左上角 (100, 50) 宽 200 高 100 → 归一化 (0.25, 0.25, 0.5, 0.5)
        let r = normalize_focus(FocusShape::Rectangle, 100.0, 50.0, 200.0, 100.0, None, 400.0, 200.0);
        assert_eq!(r, FocusPoint::rectangle(0.25, 0.25, 0.5, 0.5));

        // 圆：圆心 (100, 50) 直径 40 → 圆心 (0.25, 0.25) 直径 0.1
        let c = normalize_focus(FocusShape::Circle, 100.0, 50.0, 40.0, 40.0, None, 400.0, 200.0);
        assert_eq!(c, FocusPoint::circle(0.25, 0.25, 0.1));
    }

    #[test]
    fn test_normalize_focus_orientation_rotations() {
        // 180°（orientation 3）：点 (100, 50) 在 400×200 → (300, 150) → (0.75, 0.75)
        let p = normalize_focus(FocusShape::Point, 100.0, 50.0, 0.0, 0.0, Some(3), 400.0, 200.0);
        assert_eq!(p, FocusPoint::point(0.75, 0.75));

        // 90° CW（orientation 6）：点 (100, 50) 在 400×200 → 显示 200×400
        // 旋转后坐标 (H-y, x) = (200-50, 100) = (150, 100) → (0.75, 0.25)
        let p = normalize_focus(FocusShape::Point, 100.0, 50.0, 0.0, 0.0, Some(6), 400.0, 200.0);
        assert_eq!(p, FocusPoint::point(0.75, 0.25));

        // 270° CW（orientation 8）：点 (100, 50) → (y, W-x) = (50, 300) → (0.25, 0.75)
        let p = normalize_focus(FocusShape::Point, 100.0, 50.0, 0.0, 0.0, Some(8), 400.0, 200.0);
        assert_eq!(p, FocusPoint::point(0.25, 0.75));

        // 矩形旋转 90°：左上角 (100, 50) 宽 200 高 100 →
        // 外接矩旋转 (H-y-h, x, h, w) = (200-50-100, 100, 100, 200) = (50, 100, 100, 200)
        // 显示 200×400 → (0.25, 0.25, 0.5, 0.5)
        let r = normalize_focus(FocusShape::Rectangle, 100.0, 50.0, 200.0, 100.0, Some(6), 400.0, 200.0);
        assert_eq!(r, FocusPoint::rectangle(0.25, 0.25, 0.5, 0.5));

        // 圆旋转 90°：圆心 (100, 50) 直径 40 → 旋转后圆心 (H-cy, cx) = (150, 100)
        // 显示 200×400 → (0.75, 0.25)，直径 40/200 = 0.2
        let c = normalize_focus(FocusShape::Circle, 100.0, 50.0, 40.0, 40.0, Some(6), 400.0, 200.0);
        assert_eq!(c, FocusPoint::circle(0.75, 0.25, 0.2));
    }

    // ── Nikon AFInfo2 解析（ExifTool 布局；用真实 Z7 样本 blob）─────────

    /// Z7（8256×5504）AFInfo2 V0300 blob：AFAreaMode=Wide(S)、坐标可用、
    /// AFImage=8256×5504、X=5423、Y=4127、W=807、H=857（小端）
    fn z7_af_info_v300() -> rawlib::AfInfoData {
        let mut data = vec![0u8; 56];
        // blob[0]=AFDetectionMethod=2, blob[1]=AFAreaMode=0xc3, blob[3]=AFCoordinatesAvailable=1
        data[0] = 0x02;
        data[1] = 0xc3;
        data[3] = 0x01;
        // AFImageWidth=8256 (0x2040) @ 0x26, AFImageHeight=5504 (0x1580) @ 0x28
        data[0x26..0x28].copy_from_slice(&0x2040u16.to_le_bytes());
        data[0x28..0x2a].copy_from_slice(&0x1580u16.to_le_bytes());
        // X=5423 (0x152f) @ 0x2a, Y=4127 (0x101f) @ 0x2c, W=807 (0x0327) @ 0x2e, H=857 (0x0359) @ 0x30
        data[0x2a..0x2c].copy_from_slice(&0x152fu16.to_le_bytes());
        data[0x2c..0x2e].copy_from_slice(&0x101fu16.to_le_bytes());
        data[0x2e..0x30].copy_from_slice(&0x0327u16.to_le_bytes());
        data[0x30..0x32].copy_from_slice(&0x0359u16.to_le_bytes());
        rawlib::AfInfoData {
            tag: 0x00b7,
            order: 0x4949,
            version: 300,
            data,
        }
    }

    #[test]
    fn test_parse_af_info_v300_z7() {
        let fp = parse_af_info(&z7_af_info_v300()).expect("应有对焦点");
        assert_eq!(fp.shape, FocusShape::Rectangle);
        // 中心 (5423/8256, 4127/5504) = (0.6569, 0.7498)；宽 807/8256=0.0977 高 857/5504=0.1557
        // 左上角 = 中心 - 半尺寸
        let expect_x = 5423.0 / 8256.0 - 807.0 / 8256.0 / 2.0;
        let expect_y = 4127.0 / 5504.0 - 857.0 / 5504.0 / 2.0;
        assert!((fp.x - expect_x as f32).abs() < 1e-4, "x={} expect={}", fp.x, expect_x);
        assert!((fp.y - expect_y as f32).abs() < 1e-4, "y={} expect={}", fp.y, expect_y);
        assert!((fp.width - (807.0 / 8256.0) as f32).abs() < 1e-4);
        assert!((fp.height - (857.0 / 5504.0) as f32).abs() < 1e-4);
    }

    #[test]
    fn test_parse_af_info_v400_layout() {
        // V0400（Z8/Z9）：坐标偏移后移 16 字节（0x26 → 0x3a）
        let mut af = z7_af_info_v300();
        af.version = 400;
        af.data = vec![0u8; 0x48 + 2];
        af.data[3] = 0x01;
        af.data[0x3a..0x3c].copy_from_slice(&0x2040u16.to_le_bytes());
        af.data[0x3c..0x3e].copy_from_slice(&0x1580u16.to_le_bytes());
        af.data[0x3e..0x40].copy_from_slice(&0x152fu16.to_le_bytes());
        af.data[0x40..0x42].copy_from_slice(&0x101fu16.to_le_bytes());
        af.data[0x42..0x44].copy_from_slice(&0x0327u16.to_le_bytes());
        af.data[0x44..0x46].copy_from_slice(&0x0359u16.to_le_bytes());
        let fp = parse_af_info(&af).expect("应有对焦点");
        assert_eq!(fp.shape, FocusShape::Rectangle);
        assert!((fp.x - (5423.0 / 8256.0 - 807.0 / 8256.0 / 2.0) as f32).abs() < 1e-4);
    }

    #[test]
    fn test_parse_af_info_guards() {
        // 老版 AFInfo（0x0088）无坐标 → None
        let mut af = z7_af_info_v300();
        af.tag = 0x0088;
        assert!(parse_af_info(&af).is_none());
        // 坐标不可用（AFCoordinatesAvailable = 0）→ None
        let mut af = z7_af_info_v300();
        af.data[3] = 0;
        assert!(parse_af_info(&af).is_none());
        // 不支持版本 → None
        let mut af = z7_af_info_v300();
        af.version = 100;
        assert!(parse_af_info(&af).is_none());
        // blob 过短 → None
        let mut af = z7_af_info_v300();
        af.data.truncate(4);
        assert!(parse_af_info(&af).is_none());
        // 大端字节序正确读取
        let mut af = z7_af_info_v300();
        af.order = 0x4d4d;
        for i in [0x26usize, 0x28, 0x2a, 0x2c, 0x2e, 0x30] {
            af.data[i..i + 2].swap(0, 1);
        }
        let fp = parse_af_info(&af).expect("大端应可解析");
        assert_eq!(fp.shape, FocusShape::Rectangle);
    }
}
