//! 导入功能（SD 卡 → 按日期建目录 → 去重 → 复制/移动）
//!
//! 全同步实现（与 scanner/ops 一致，core 层禁止 async）。流程：
//!   1. `detect_removable_drives` —— 检测可移动驱动器（Windows 原生 API）
//!   2. `scan_import_source`     —— 递归扫描源，EXIF 拍摄日期优先（回退 mtime）
//!   3. `plan_import`            —— 按 YYYY-MM-DD 分组 + 目标去重（同名同大小跳过）
//!   4. `execute_import`         —— 逐文件委托 ops 复制/移动，进度回调
//!
//! 驱动器检测说明：批次约束只允许 photo-tauri 新增 `windows` crate，本模块
//! Windows 用等价的 kernel32 原生 FFI（GetLogicalDrives / GetDriveTypeW /
//! GetVolumeInformationW）；Linux 零依赖解析 /proc/mounts + 查
//! /sys/class/block/<dev>/removable；其余平台返回空（前端退化手动选源）。

use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use thiserror::Error;
use walkdir::WalkDir;

use chrono::Datelike;
use photo_domain::{Capture, ImageFormat, SourceFile};

use crate::exif;
use crate::ops;

/// 导入错误
#[derive(Error, Debug)]
pub enum ImportError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("文件操作错误: {0}")]
    Ops(#[from] ops::OpError),
    #[error("源文件不存在: {0}")]
    SourceNotFound(PathBuf),
    #[error("目标已存在: {0}")]
    TargetExists(PathBuf),
}

/// 可移动驱动器信息
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveInfo {
    /// 根路径（Windows 如 "E:\\"，Linux 为挂载点如 "/run/media/user/CANON"）
    pub path: String,
    /// 卷标（读取失败/空卷标为 None；Linux 取挂载点末段——udisks 按卷标挂载）
    pub label: Option<String>,
}

/// 导入候选文件（扫描产物）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportCandidate {
    /// 源文件完整路径
    pub path: PathBuf,
    /// 拍摄日期 YYYY-MM-DD（EXIF DateTimeOriginal 优先，回退文件修改时间）
    pub date: String,
    /// 文件大小（字节，去重用）
    pub size: u64,
}

/// 按日期分组的导入计划组（目标目录 = dest_root/date_dir/）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportGroup {
    /// 日期目录名（YYYY-MM-DD，相对 dest_root）
    pub date_dir: String,
    /// 该组内的源文件路径
    pub files: Vec<PathBuf>,
}

/// 计划阶段被跳过的文件
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportSkipped {
    /// 源文件完整路径
    pub path: PathBuf,
    /// 跳过原因
    pub reason: String,
}

/// 导入计划（干跑结果，不碰文件）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportPlan {
    /// 按日期分组（保持候选顺序）
    pub groups: Vec<ImportGroup>,
    /// 跳过清单（目标去重 / 源内冲突）
    pub skipped: Vec<ImportSkipped>,
}

/// 导入执行模式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportMode {
    /// 复制（源保留）
    Copy,
    /// 移动（源删除；跨文件系统走 copy + delete 回退）
    Move,
}

/// 导入进度（逐文件回调）
#[derive(Debug, Clone)]
pub struct ImportProgress {
    /// 已处理文件数（从 1 开始）
    pub done: u32,
    /// 总文件数
    pub total: u32,
    /// 当前处理的源文件路径
    pub current: PathBuf,
}

/// 检测可移动驱动器。
///
/// Windows：GetLogicalDrives 位掩码枚举 A–Z → GetDriveTypeW 过滤 DRIVE_REMOVABLE
/// （SD 卡/U 盘）→ GetVolumeInformationW 取卷标。
/// Linux：/proc/mounts 过滤可移动介质常规挂载前缀（/run/media、/media、/mnt）
/// → /sys/class/block/<dev>/removable 判定（分区名回退磁盘名）。
/// 其余平台返回空（前端退化为手动选择源）。
pub fn detect_removable_drives() -> Vec<DriveInfo> {
    #[cfg(windows)]
    {
        return win32::detect_removable_drives();
    }
    #[cfg(target_os = "linux")]
    {
        return linux::detect_removable_drives();
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        Vec::new()
    }
}

/// 递归扫描导入源目录（整棵子树，不限 DCIM），收集可查看媒体文件（图片/RAW/视频）。
///
/// 日期 = EXIF DateTimeOriginal 优先（解析为 YYYY-MM-DD），回退文件修改时间；
/// 单文件提取失败不影响整体（跳过该文件并记 warning）。返回按完整路径排序。
pub fn scan_import_source(dir: &Path) -> Result<Vec<ImportCandidate>, ImportError> {
    if !dir.is_dir() {
        return Err(ImportError::Io(std::io::Error::new(
            ErrorKind::NotFound,
            format!("源目录不存在: {}", dir.display()),
        )));
    }
    let mut candidates = Vec::new();
    for entry in WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        if !ImageFormat::is_viewable(ext) {
            continue;
        }
        if let Some(candidate) = build_candidate(path) {
            candidates.push(candidate);
        }
    }
    // 确定性顺序（walkdir 目录序不定）：按完整路径排序
    candidates.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(candidates)
}

/// 单个文件 → 候选（EXIF 提取/元数据失败返回 None，由调用方跳过）
fn build_candidate(path: &Path) -> Option<ImportCandidate> {
    let meta = std::fs::metadata(path).ok()?;
    let format = path.extension().and_then(|e| e.to_str()).and_then(ImageFormat::from_extension);
    // 视频等非图片格式不尝试 EXIF（kamadak 无法解析，直接走 mtime）
    let date = match format {
        Some(f) if !f.is_other() => match exif::extract_exif(path, &f) {
            Ok(m) => m
                .date_time_original
                .as_deref()
                .and_then(parse_exif_date)
                .unwrap_or_else(|| mtime_date(&meta)),
            Err(_) => mtime_date(&meta),
        },
        _ => mtime_date(&meta),
    };
    Some(ImportCandidate {
        path: path.to_path_buf(),
        date,
        size: meta.len(),
    })
}

/// EXIF 日期串 → YYYY-MM-DD。
/// 支持标准 EXIF 形态 "2024:01:02 10:30:00" 与 "-" 分隔形态（部分相机/手机）。
fn parse_exif_date(raw: &str) -> Option<String> {
    let raw = raw.trim();
    for fmt in ["%Y:%m:%d %H:%M:%S", "%Y-%m-%d %H:%M:%S"] {
        if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(raw, fmt) {
            return Some(format!("{:04}-{:02}-{:02}", dt.year(), dt.month(), dt.day()));
        }
    }
    // 仅日期形态（部分机型无时间字段）
    for fmt in ["%Y:%m:%d", "%Y-%m-%d"] {
        if let Ok(d) = chrono::NaiveDate::parse_from_str(raw, fmt) {
            return Some(format!("{:04}-{:02}-{:02}", d.year(), d.month(), d.day()));
        }
    }
    None
}

/// 文件修改时间 → YYYY-MM-DD（mtime 回退；读不到按纪元日兜底）
fn mtime_date(meta: &std::fs::Metadata) -> String {
    meta.modified()
        .map(|t| {
            let dt: chrono::DateTime<chrono::Local> = t.into();
            format!("{:04}-{:02}-{:02}", dt.year(), dt.month(), dt.day())
        })
        .unwrap_or_else(|_| "1970-01-01".to_string())
}

/// 生成导入计划（干跑，不碰文件）：
///   1. 按候选日期（YYYY-MM-DD）分组，目标目录 = dest_root/YYYY-MM-DD/
///   2. 目标去重：目标已存在同名文件 → 大小相同 = 已完成导入（跳过）；
///      大小不同 = 同名冲突防覆盖（跳过）
///   3. 组内冲突：两个源文件映射同一目标名 → 保留先者，后者跳过
pub fn plan_import(candidates: &[ImportCandidate], dest_root: &Path) -> ImportPlan {
    let mut groups: Vec<ImportGroup> = Vec::new();
    let mut skipped: Vec<ImportSkipped> = Vec::new();
    // date_dir → 组索引（保持候选首次出现顺序）
    let mut group_index: HashMap<&str, usize> = HashMap::new();
    // date_dir → (目标文件名 → 源大小)：组内已计划目标，防组内同名互踩
    let mut planned: HashMap<String, HashMap<String, u64>> = HashMap::new();

    for cand in candidates {
        let Some(file_name) = cand.path.file_name().map(|n| n.to_string_lossy().to_string()) else {
            skipped.push(ImportSkipped {
                path: cand.path.clone(),
                reason: "无法解析文件名".to_string(),
            });
            continue;
        };
        // 目标已存在：同名同大小 = 去重跳过；同名不同大小 = 防覆盖跳过
        let target_path = dest_root.join(&cand.date).join(&file_name);
        if let Ok(meta) = std::fs::metadata(&target_path) {
            let reason = if meta.len() == cand.size {
                "目标已存在且大小相同".to_string()
            } else {
                "目标已存在同名文件（大小不同）".to_string()
            };
            skipped.push(ImportSkipped {
                path: cand.path.clone(),
                reason,
            });
            continue;
        }
        // 组内同名冲突：同一日期组两个源文件映射到同一目标名
        let planned_for_group = planned.entry(cand.date.clone()).or_default();
        if let Some(&prev_size) = planned_for_group.get(&file_name) {
            let reason = if prev_size == cand.size {
                "源内重复（另一候选同名同大小）".to_string()
            } else {
                "源内同名冲突（另一候选同名不同大小）".to_string()
            };
            skipped.push(ImportSkipped {
                path: cand.path.clone(),
                reason,
            });
            continue;
        }
        // 接受该文件时才创建/复用日期组（全部被跳过的日期不产生空组）
        let group_pos = match group_index.get(cand.date.as_str()) {
            Some(&i) => i,
            None => {
                groups.push(ImportGroup {
                    date_dir: cand.date.clone(),
                    files: Vec::new(),
                });
                let i = groups.len() - 1;
                group_index.insert(cand.date.as_str(), i);
                i
            }
        };
        planned_for_group.insert(file_name, cand.size);
        groups[group_pos].files.push(cand.path.clone());
    }

    ImportPlan { groups, skipped }
}

/// 执行导入计划：逐文件委托 ops 复制/移动（move 跨文件系统走 EXDEV 回退），
/// 返回逐文件结果（顺序 = 计划组顺序）。
///
/// `on_progress` — 每个文件处理后回调（done 从 1 开始；total = 计划文件总数）。
pub fn execute_import(
    plan: &ImportPlan,
    dest_root: &Path,
    mode: ImportMode,
    on_progress: Option<Box<dyn Fn(ImportProgress) + Send>>,
) -> Vec<(PathBuf, Result<(), ImportError>)> {
    let total = plan.groups.iter().map(|g| g.files.len() as u32).sum::<u32>();
    let mut results = Vec::new();
    let mut done: u32 = 0;

    for group in &plan.groups {
        let target_dir = dest_root.join(&group.date_dir);
        for src in &group.files {
            let result = import_one_file(src, &target_dir, mode);
            done += 1;
            if let Some(cb) = &on_progress {
                cb(ImportProgress {
                    done,
                    total,
                    current: src.clone(),
                });
            }
            results.push((src.clone(), result));
        }
    }
    results
}

/// 单个文件导入：源存在性 + 目标存在性防御检查后，委托 ops 复制/移动。
fn import_one_file(src: &Path, target_dir: &Path, mode: ImportMode) -> Result<(), ImportError> {
    if !src.exists() {
        return Err(ImportError::SourceNotFound(src.to_path_buf()));
    }
    let Some(name) = src.file_name() else {
        return Err(ImportError::SourceNotFound(src.to_path_buf()));
    };
    let dest = target_dir.join(name);
    // 计划与执行之间目标可能被并发写入：不静默覆盖（ops 层跨设备回退同样报错）
    if dest.exists() {
        return Err(ImportError::TargetExists(dest));
    }
    let capture = single_file_capture(src);
    match mode {
        ImportMode::Copy => ops::copy_capture(&capture, target_dir, false).map_err(ImportError::from)?,
        ImportMode::Move => ops::move_capture(&capture, target_dir).map_err(ImportError::from)?,
    }
    Ok(())
}

/// 构造单文件 Capture（导入按文件粒度操作，ops 层需要 Capture 形态）
fn single_file_capture(path: &Path) -> Capture {
    let format = path
        .extension()
        .and_then(|e| e.to_str())
        .and_then(ImageFormat::from_extension)
        .unwrap_or(ImageFormat::Other);
    Capture {
        base_name: path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default(),
        source_files: vec![SourceFile {
            path: path.to_path_buf(),
            format,
            file_size: None,
        }],
        primary_index: 0,
    }
}

// ============================================================================
// Windows 驱动器检测（kernel32 原生 FFI；cfg(windows) 门控）
// ============================================================================

#[cfg(windows)]
mod win32 {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    use super::DriveInfo;

    /// DRIVE_REMOVABLE（winbase.h：可移动介质，如软驱/读卡器/U 盘）
    const DRIVE_REMOVABLE: u32 = 2;
    /// 卷标缓冲长度（MAX_PATH + 1，Windows 惯例）
    const MAX_VOLUME_NAME: u32 = 261;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        /// 返回 A: 到 Z: 的位掩码（bit 0 = A:）
        fn GetLogicalDrives() -> u32;
        /// 返回驱动器类型（DRIVE_REMOVABLE = 2）
        fn GetDriveTypeW(lp_root_path_name: *const u16) -> u32;
        /// 读取卷标（lp_volume_name_buffer 以 NUL 结尾的宽字符串）
        #[allow(non_snake_case)]
        fn GetVolumeInformationW(
            lp_root_path_name: *const u16,
            lp_volume_name_buffer: *mut u16,
            n_volume_name_size: u32,
            lp_volume_serial_number: *mut u32,
            lp_maximum_component_length: *mut u32,
            lp_file_system_flags: *mut u32,
            lp_file_system_name_buffer: *mut u16,
            n_file_system_name_size: u32,
        ) -> i32;
    }

    /// 枚举可移动驱动器（GetLogicalDrives + GetDriveTypeW + GetVolumeInformationW）
    pub(super) fn detect_removable_drives() -> Vec<DriveInfo> {
        let mut drives = Vec::new();
        let mask = unsafe { GetLogicalDrives() };
        for i in 0..26u32 {
            if mask & (1 << i) == 0 {
                continue;
            }
            let letter = char::from(b'A' + i as u8);
            let root: Vec<u16> = format!("{}:\\", letter)
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            if unsafe { GetDriveTypeW(root.as_ptr()) } != DRIVE_REMOVABLE {
                continue;
            }
            drives.push(DriveInfo {
                path: format!("{}:\\", letter),
                label: volume_label(&root),
            });
        }
        drives
    }

    /// 读取卷标（GetVolumeInformationW；失败/空卷标返回 None）
    fn volume_label(root: &[u16]) -> Option<String> {
        let mut buf = vec![0u16; MAX_VOLUME_NAME as usize];
        let ok = unsafe {
            GetVolumeInformationW(
                root.as_ptr(),
                buf.as_mut_ptr(),
                buf.len() as u32,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0,
            )
        };
        if ok == 0 {
            return None;
        }
        // 卷标以 NUL 结尾：截断到第一个 NUL 再转 String
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        let label = OsString::from_wide(&buf[..len]).to_string_lossy().trim().to_string();
        if label.is_empty() {
            None
        } else {
            Some(label)
        }
    }
}

// ============================================================================
// Linux 驱动器检测（/proc/mounts + /sys/class/block，零依赖）
// 纯解析函数跨平台编译（cfg(test) 可达），sys/proc 探测仅 cfg(linux)
// ============================================================================

/// Linux 可移动介质常规挂载前缀（/run/media/<user>/ 与 /media/<user>/ 为 udisks
/// 自动挂载点，/mnt/ 为手动挂载惯例；真正的判定闸门是 /sys removable 属性）
#[cfg(any(target_os = "linux", test))]
const LINUX_MOUNT_PREFIXES: [&str; 3] = ["/run/media/", "/media/", "/mnt/"];

/// 解析 /proc/mounts 内容 → (设备路径, 挂载点) 列表。
/// 行格式：dev mount fstype options dump pass；挂载点内空格转义为 \040（八进制）
#[cfg(any(target_os = "linux", test))]
fn parse_proc_mounts(content: &str) -> Vec<(String, String)> {
    content
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let dev = fields.next()?;
            let mount = fields.next()?.replace("\\040", " ");
            Some((dev.to_string(), mount))
        })
        .collect()
}

/// 挂载点是否位于可移动介质常规前缀下（排除前缀目录本身，如 /mnt 根）
#[cfg(any(target_os = "linux", test))]
fn is_removable_mount_point(mount: &str) -> bool {
    LINUX_MOUNT_PREFIXES
        .iter()
        .any(|p| mount.len() > p.len() && mount.starts_with(p))
}

/// /dev/ 设备路径 → 块设备名（/dev/sdb1 → sdb1；/dev/mapper/xxx 等嵌套形态返回 None）
#[cfg(any(target_os = "linux", test))]
fn block_device_name(dev: &str) -> Option<&str> {
    dev.strip_prefix("/dev/")
        .filter(|n| !n.is_empty() && !n.contains('/'))
}

/// 块设备名 → 磁盘名（去分区后缀）：sdb1→sdb、mmcblk0p1→mmcblk0、nvme0n1p2→nvme0n1。
/// 规则：先剥 `p<数字>` 后缀（mmcblk/nvme 形态，基底以数字结尾才剥），再剥纯数字
/// 后缀（sdX 形态，基底全字母才剥）。
/// 注意：磁盘名本身以数字结尾时（mmcblk0）两种规则都可能误剥，故本函数仅作兜底
/// 候选——调用方（is_removable_device）必须先查原名节点，查不到才用本结果
#[cfg(any(target_os = "linux", test))]
fn block_disk_name(dev_name: &str) -> &str {
    // mmcblk0p1 / nvme0n1p2 形态：'p'+数字后缀，且磁盘名本身以数字结尾
    if let Some(pidx) = dev_name.rfind('p') {
        let (base, part) = dev_name.split_at(pidx);
        if base.ends_with(|c: char| c.is_ascii_digit())
            && part.len() > 1
            && part[1..].chars().all(|c| c.is_ascii_digit())
        {
            return base;
        }
    }
    // sdb1 形态：纯数字后缀，且磁盘名全字母（mmcblk0 这类以数字结尾的整体名不动）
    let stripped = dev_name.trim_end_matches(|c: char| c.is_ascii_digit());
    if stripped.len() < dev_name.len()
        && !stripped.is_empty()
        && stripped.chars().all(|c| c.is_ascii_alphabetic())
    {
        return stripped;
    }
    dev_name
}

#[cfg(target_os = "linux")]
mod linux {
    use super::{
        DriveInfo, block_device_name, block_disk_name, is_removable_mount_point, parse_proc_mounts,
    };
    use std::path::Path;

    /// 判定块设备是否可移动介质：/sys/class/block/<name>/removable == "1"；
    /// 分区节点无该属性时回退磁盘名（sdb1→sdb、mmcblk0p1→mmcblk0）
    fn is_removable_device(dev_name: &str) -> bool {
        for name in [dev_name, block_disk_name(dev_name)] {
            if let Ok(v) = std::fs::read_to_string(format!("/sys/class/block/{name}/removable")) {
                return v.trim() == "1";
            }
        }
        false
    }

    /// 卷标：udisks 按卷标挂载（/run/media/<user>/<LABEL>），挂载点末段即卷标
    fn label_of(mount: &str) -> Option<String> {
        Path::new(mount)
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .filter(|s| !s.is_empty())
    }

    /// 枚举可移动驱动器（/proc/mounts 过滤挂载前缀 + /sys removable 判定 + 挂载点去重）
    pub(super) fn detect_removable_drives() -> Vec<DriveInfo> {
        let Ok(mounts) = std::fs::read_to_string("/proc/mounts") else {
            return Vec::new();
        };
        let mut seen = std::collections::HashSet::new();
        parse_proc_mounts(&mounts)
            .into_iter()
            .filter(|(_, m)| is_removable_mount_point(m))
            .filter(|(d, _)| block_device_name(d).is_some_and(is_removable_device))
            .filter(|(_, m)| seen.insert(m.clone())) // bind mount 等同点去重
            .map(|(_, m)| DriveInfo {
                label: label_of(&m),
                path: m,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// 在临时目录写一个文件，返回路径（内容确定 → 大小确定，便于去重断言）
    fn make_file(dir: &TempDir, rel: &str, content: &[u8]) -> PathBuf {
        let path = dir.path().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, content).unwrap();
        path
    }

    /// 构造候选（date 显式指定）
    fn cand(path: PathBuf, date: &str, size: u64) -> ImportCandidate {
        ImportCandidate {
            path,
            date: date.to_string(),
            size,
        }
    }

    // ── detect_removable_drives（形状断言；无可移动盘时为空的合法结果）──

    #[test]
    fn test_detect_removable_drives_shape() {
        let drives = detect_removable_drives();
        for d in &drives {
            assert!(!d.path.is_empty(), "根路径不应为空");
            #[cfg(windows)]
            assert!(d.path.ends_with(":\\"), "Windows 根路径应为盘符根: {:?}", d.path);
            #[cfg(target_os = "linux")]
            assert!(d.path.starts_with('/'), "Linux 根路径应为挂载点: {:?}", d.path);
            // 卷标要么 None 要么非空
            if let Some(label) = &d.label {
                assert!(!label.is_empty());
            }
        }
    }

    // ── Linux 挂载解析（纯函数，跨平台可测）──

    #[test]
    fn test_parse_proc_mounts_basic_and_escape() {
        let content = "/dev/sda1 / ext4 rw 0 0\n\
                       /dev/sdb1 /run/media/user/CANON\\040EOS exfat rw,nosuid 0 0\n\
                       tmpfs /run/user/1000 tmpfs rw 0 0\n\
                       /dev/mmcblk0p1 /media/user/SD128 vfat rw 0 0\n";
        let mounts = parse_proc_mounts(content);
        assert_eq!(mounts.len(), 4);
        assert_eq!(mounts[0], ("/dev/sda1".to_string(), "/".to_string()));
        // \040 转义还原为空格
        assert_eq!(mounts[1].1, "/run/media/user/CANON EOS");
        assert_eq!(mounts[3].0, "/dev/mmcblk0p1");
    }

    #[test]
    fn test_is_removable_mount_point_prefixes() {
        assert!(is_removable_mount_point("/run/media/user/CANON"));
        assert!(is_removable_mount_point("/media/user/SD128"));
        assert!(is_removable_mount_point("/mnt/sdcard"));
        // 前缀目录本身与常规系统挂载点不算
        assert!(!is_removable_mount_point("/mnt"));
        assert!(!is_removable_mount_point("/mnt/"));
        assert!(!is_removable_mount_point("/"));
        assert!(!is_removable_mount_point("/home/user/photos"));
        assert!(!is_removable_mount_point("/boot/efi"));
    }

    #[test]
    fn test_block_device_name_filter() {
        assert_eq!(block_device_name("/dev/sdb1"), Some("sdb1"));
        assert_eq!(block_device_name("/dev/mmcblk0p1"), Some("mmcblk0p1"));
        // 嵌套路径（mapper/dm）与非 /dev 前缀不直接对应 /sys/class/block 节点
        assert_eq!(block_device_name("/dev/mapper/luks-1"), None);
        assert_eq!(block_device_name("tmpfs"), None);
        assert_eq!(block_device_name("/dev/"), None);
    }

    #[test]
    fn test_block_disk_name_partition_suffix() {
        assert_eq!(block_disk_name("sdb1"), "sdb");
        assert_eq!(block_disk_name("sda"), "sda"); // 无分区后缀不动
        assert_eq!(block_disk_name("mmcblk0p1"), "mmcblk0");
        assert_eq!(block_disk_name("nvme0n1p2"), "nvme0n1");
        assert_eq!(block_disk_name("loop0"), "loop");
        // mmcblk0 这类整体名以数字结尾的会被误剥（返回 mmcblk）——仅作兜底候选，
        // is_removable_device 先查原名 /sys 节点，原名命中时不会用到本结果
    }

    // ── scan_import_source ──

    #[test]
    fn test_scan_import_source_collects_viewable_only() {
        let dir = TempDir::new().unwrap();
        make_file(&dir, "a.jpg", b"jpeg-bytes");
        make_file(&dir, "b.PNG", b"png-bytes");
        make_file(&dir, "sub/c.NEF", b"raw-bytes");
        make_file(&dir, "sub/video.mp4", b"mp4-bytes");
        make_file(&dir, "notes.txt", b"not-a-photo");

        let cands = scan_import_source(dir.path()).unwrap();
        // 4 个可查看文件（jpg/png/nef/mp4），notes.txt 排除；顺序按路径排序
        assert_eq!(cands.len(), 4, "候选: {:?}", cands.iter().map(|c| &c.path).collect::<Vec<_>>());
        let names: Vec<String> = cands.iter().map(|c| c.path.to_string_lossy().to_string()).collect();
        assert!(names.windows(2).all(|w| w[0] < w[1]), "应按路径排序");
        // 全部走 mtime 回退：日期 = 文件修改日期
        for c in &cands {
            let meta = std::fs::metadata(&c.path).unwrap();
            let expect = mtime_date(&meta);
            assert_eq!(c.date, expect, "mtime 回退日期不匹配: {:?}", c.path);
            assert_eq!(c.size, meta.len());
        }
    }

    #[test]
    fn test_scan_import_source_missing_dir() {
        let dir = TempDir::new().unwrap();
        let missing = dir.path().join("nope");
        let err = scan_import_source(&missing).unwrap_err();
        assert!(matches!(err, ImportError::Io(_)), "缺失目录应报 Io(NotFound): {err:?}");
    }

    // ── 日期解析（EXIF 路径的解析机械）──

    #[test]
    fn test_parse_exif_date_variants() {
        assert_eq!(parse_exif_date("2024:01:02 10:30:00"), Some("2024-01-02".to_string()));
        assert_eq!(parse_exif_date("2024-01-02 10:30:00"), Some("2024-01-02".to_string()));
        assert_eq!(parse_exif_date("2024:01:02"), Some("2024-01-02".to_string()));
        assert_eq!(parse_exif_date("2024-01-02"), Some("2024-01-02".to_string()));
        assert_eq!(parse_exif_date(" 2024:01:02 10:30:00 "), Some("2024-01-02".to_string()));
        assert_eq!(parse_exif_date("garbage"), None);
        assert_eq!(parse_exif_date(""), None);
    }

    #[test]
    fn test_mtime_fallback_plain_file() {
        // 无 EXIF 的普通文件：build_candidate 走 mtime 回退
        let dir = TempDir::new().unwrap();
        let path = make_file(&dir, "plain.jpg", b"no-exif");
        let meta = std::fs::metadata(&path).unwrap();
        let c = build_candidate(&path).unwrap();
        assert_eq!(c.date, mtime_date(&meta));
        assert_eq!(c.size, meta.len());
    }

    // ── plan_import：分组 / 去重 / 冲突 ──

    #[test]
    fn test_plan_import_groups_by_date() {
        let dir = TempDir::new().unwrap();
        let a1 = make_file(&dir, "src/a1.jpg", b"x");
        let a2 = make_file(&dir, "src/a2.jpg", b"y");
        let b1 = make_file(&dir, "src/b1.png", b"z");
        let dest = dir.path().join("dest");
        let cands = vec![
            cand(a1, "2024-05-01", 1),
            cand(a2, "2024-05-01", 1),
            cand(b1, "2024-06-02", 1),
        ];
        let plan = plan_import(&cands, &dest);
        assert_eq!(plan.skipped.len(), 0);
        assert_eq!(plan.groups.len(), 2, "应按日期分两组");
        assert_eq!(plan.groups[0].date_dir, "2024-05-01");
        assert_eq!(plan.groups[0].files.len(), 2);
        assert_eq!(plan.groups[1].date_dir, "2024-06-02");
        assert_eq!(plan.groups[1].files.len(), 1);
    }

    #[test]
    fn test_plan_import_dedup_existing_same_size() {
        let dir = TempDir::new().unwrap();
        let src = make_file(&dir, "src/photo.jpg", b"12345");
        let dest = dir.path().join("dest");
        // 目标已存在同名同大小文件 → 跳过
        let existing = make_file(&dir, "dest/2024-05-01/photo.jpg", b"12345");
        assert_eq!(existing.file_name().unwrap().to_str().unwrap(), "photo.jpg");
        let cands = vec![cand(src, "2024-05-01", 5)];
        let plan = plan_import(&cands, &dest);
        assert_eq!(plan.groups.len(), 0, "全部跳过 → 无组");
        assert_eq!(plan.skipped.len(), 1);
        assert!(plan.skipped[0].reason.contains("大小相同"), "原因: {}", plan.skipped[0].reason);
    }

    #[test]
    fn test_plan_import_existing_diff_size_skipped() {
        let dir = TempDir::new().unwrap();
        let src = make_file(&dir, "src/photo.jpg", b"12345");
        let dest = dir.path().join("dest");
        // 目标已存在同名但大小不同 → 防覆盖跳过
        make_file(&dir, "dest/2024-05-01/photo.jpg", b"different-size");
        let cands = vec![cand(src, "2024-05-01", 5)];
        let plan = plan_import(&cands, &dest);
        assert_eq!(plan.skipped.len(), 1);
        assert!(plan.skipped[0].reason.contains("大小不同"), "原因: {}", plan.skipped[0].reason);
    }

    #[test]
    fn test_plan_import_duplicate_within_source() {
        let dir = TempDir::new().unwrap();
        // 同一日期组内两个源文件映射同一目标名（不同目录下的同名文件）
        let s1 = make_file(&dir, "src/d1/IMG_1.jpg", b"11111");
        let s2 = make_file(&dir, "src/d2/IMG_1.jpg", b"22222");
        let dest = dir.path().join("dest");
        let cands = vec![cand(s1, "2024-05-01", 5), cand(s2, "2024-05-01", 5)];
        let plan = plan_import(&cands, &dest);
        assert_eq!(plan.groups.len(), 1);
        assert_eq!(plan.groups[0].files.len(), 1, "保留先者");
        assert_eq!(plan.skipped.len(), 1);
        assert!(plan.skipped[0].reason.contains("重复") || plan.skipped[0].reason.contains("冲突"));
    }

    #[test]
    fn test_plan_import_empty() {
        let dir = TempDir::new().unwrap();
        let plan = plan_import(&[], &dir.path().join("dest"));
        assert_eq!(plan.groups.len(), 0);
        assert_eq!(plan.skipped.len(), 0);
    }

    // ── execute_import：复制 / 移动语义 ──

    #[test]
    fn test_execute_import_copy_keeps_source() {
        let dir = TempDir::new().unwrap();
        let s1 = make_file(&dir, "src/a.jpg", b"aaa");
        let s2 = make_file(&dir, "src/sub/b.png", b"bbbb");
        let dest = dir.path().join("dest");
        let plan = plan_import(
            &[
                cand(s1.clone(), "2024-05-01", 3),
                cand(s2.clone(), "2024-05-02", 4),
            ],
            &dest,
        );
        assert_eq!(plan.skipped.len(), 0);

        // 进度回调：done 1..=2，total 2，current 顺序 = 计划顺序
        // （Box<dyn Fn + Send> 需 'static：用 Arc<Mutex> 共享收集容器，调用后取回）
        let seen = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
        let seen_cb = seen.clone();
        let results = execute_import(
            &plan,
            &dest,
            ImportMode::Copy,
            Some(Box::new(move |p| {
                seen_cb.lock().push((p.done, p.total, p.current.to_string_lossy().to_string()));
            })),
        );
        let seen = std::sync::Arc::try_unwrap(seen).unwrap().into_inner();
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|(_, r)| r.is_ok()), "复制应全部成功: {results:?}");
        // 目标存在、源保留
        assert!(dest.join("2024-05-01/a.jpg").is_file());
        assert!(dest.join("2024-05-02/b.png").is_file());
        assert!(s1.is_file() && s2.is_file(), "复制不删除源");
        // 内容一致
        assert_eq!(std::fs::read(dest.join("2024-05-01/a.jpg")).unwrap(), b"aaa");
        // 进度断言
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0], (1, 2, s1.to_string_lossy().to_string()));
        assert_eq!(seen[1], (2, 2, s2.to_string_lossy().to_string()));
    }

    #[test]
    fn test_execute_import_move_removes_source() {
        let dir = TempDir::new().unwrap();
        let s1 = make_file(&dir, "src/a.jpg", b"aaa");
        let dest = dir.path().join("dest");
        let plan = plan_import(&[cand(s1.clone(), "2024-05-01", 3)], &dest);
        let results = execute_import(&plan, &dest, ImportMode::Move, None);
        assert_eq!(results.len(), 1);
        assert!(results[0].1.is_ok());
        assert!(dest.join("2024-05-01/a.jpg").is_file(), "目标应有文件");
        assert!(!s1.exists(), "移动后源应删除");
        assert_eq!(std::fs::read(dest.join("2024-05-01/a.jpg")).unwrap(), b"aaa");
    }

    #[test]
    fn test_execute_import_missing_source() {
        let dir = TempDir::new().unwrap();
        let missing = dir.path().join("src/ghost.jpg");
        let dest = dir.path().join("dest");
        let plan = plan_import(&[cand(missing.clone(), "2024-05-01", 3)], &dest);
        // plan 阶段不查源存在性 → 计划通过；执行时报 SourceNotFound
        assert_eq!(plan.skipped.len(), 0);
        let results = execute_import(&plan, &dest, ImportMode::Copy, None);
        assert_eq!(results.len(), 1);
        assert!(matches!(results[0].1, Err(ImportError::SourceNotFound(_))), "{:?}", results[0].1);
    }

    #[test]
    fn test_execute_import_target_exists_race() {
        let dir = TempDir::new().unwrap();
        let s1 = make_file(&dir, "src/a.jpg", b"aaa");
        let dest = dir.path().join("dest");
        let plan = plan_import(&[cand(s1.clone(), "2024-05-01", 3)], &dest);
        // 计划后目标被并发写入 → 执行时 TargetExists 报错（不覆盖）
        make_file(&dir, "dest/2024-05-01/a.jpg", b"concurrent");
        let results = execute_import(&plan, &dest, ImportMode::Copy, None);
        assert!(matches!(results[0].1, Err(ImportError::TargetExists(_))), "{:?}", results[0].1);
        // 目标内容未被覆盖
        assert_eq!(std::fs::read(dest.join("2024-05-01/a.jpg")).unwrap(), b"concurrent");
        assert!(s1.is_file());
    }
}
