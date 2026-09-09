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
use photo_domain::ImageFormat;

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

/// 导入计划的单文件目标：源路径 + 目标文件名（可被重命名模板改写）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportFileTarget {
    /// 源文件完整路径
    pub source: PathBuf,
    /// 目标文件名（含扩展名）
    pub target_name: String,
}

/// 按子目录分组的导入计划组（目标目录 = dest_root/sub_dir/；sub_dir 为空 = 直接放 dest_root）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportGroup {
    /// 目标子目录（相对 dest_root 的正斜杠路径；空串 = 不建子目录）
    pub sub_dir: String,
    /// 该组内的源文件与目标文件名
    pub files: Vec<ImportFileTarget>,
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
        // 跳过应用自己的缓存目录（源盘被本应用浏览过会生成 <目录>/.pt/）：
        // 否则 .pt/thumbs 下的缓存 JPEG 会被当成待导入的照片
        if path.components().any(|c| c.as_os_str() == ".pt") {
            continue;
        }
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
    // 视频等非图片格式不尝试 EXIF（无后端支持，直接走 mtime）
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

/// 子目录模式（相对 dest_root；None = 不建子目录，文件直接放 dest_root）
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ImportSubfolder {
    /// 不建子目录
    None,
    /// 按日期 2026-09-10（默认，与旧行为一致）
    #[default]
    DateDash,
    /// 按日期 2026/09/10
    DateSlash,
    /// 按日期 20260910
    DateCompact,
}

/// 导入的目录/命名选项
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportOptions {
    /// 子目录模式
    pub subfolder: ImportSubfolder,
    /// 文件重命名模板（None/空 = 保留原名）；占位符复用 template.rs：
    /// {name} 原名(无扩展) / {date} 拍摄日期 YYYYMMDD / {seq} 序号（补零 3 位）
    pub rename_template: Option<String>,
}

/// 按模式把候选日期（YYYY-MM-DD）渲染为目标子目录（正斜杠）。
fn render_subdir(mode: ImportSubfolder, date: &str) -> String {
    let parts: Vec<&str> = date.split('-').collect();
    let ymd = (parts.len() == 3).then(|| (parts[0], parts[1], parts[2]));
    match (mode, ymd) {
        (ImportSubfolder::None, _) => String::new(),
        (ImportSubfolder::DateSlash, Some((y, m, d))) => format!("{y}/{m}/{d}"),
        (ImportSubfolder::DateCompact, Some((y, m, d))) => format!("{y}{m}{d}"),
        // 非法日期原样回退（不会 panic）
        _ => date.to_string(),
    }
}

/// 渲染目标文件名：无模板/模板为空 = 原名；有模板 = 复用 engine::template 渲染
/// （占位符 {name}/{date}/{seq}，结果经文件名清洗，空结果回退原名）+ 保留原扩展名。
fn render_target_name(template: Option<&str>, source: &Path, date: &str, seq: u32) -> String {
    let orig = source
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let Some(template) = template.map(str::trim).filter(|t| !t.is_empty()) else {
        return orig;
    };
    let ext = source
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_default();
    let stem = source
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let ctx = crate::template::NameTemplateContext {
        name: stem,
        species: None,
        date: Some(date.to_string()),
        camera: None,
        seq,
    };
    let base = crate::template::render_name_template(template, &ctx);
    if ext.is_empty() {
        base
    } else {
        format!("{base}.{ext}")
    }
}

/// 生成导入计划（干跑，不碰文件）：
///   1. 按选项渲染子目录（默认按日期 YYYY-MM-DD）与目标文件名（可选重命名模板）
///   2. 目标去重：目标已存在同名文件 → 大小相同 = 已完成导入（跳过）；
///      大小不同 = 同名冲突防覆盖（跳过）
///   3. 计划内冲突：两个源文件映射同一目标（同子目录 + 同目标名）→ 保留先者，后者跳过
pub fn plan_import(
    candidates: &[ImportCandidate],
    dest_root: &Path,
    options: &ImportOptions,
) -> ImportPlan {
    let mut groups: Vec<ImportGroup> = Vec::new();
    let mut skipped: Vec<ImportSkipped> = Vec::new();
    // sub_dir → 组索引（保持候选首次出现顺序）
    let mut group_index: HashMap<String, usize> = HashMap::new();
    // sub_dir → (目标文件名 → 源大小)：已计划目标，防同名互踩
    let mut planned: HashMap<String, HashMap<String, u64>> = HashMap::new();
    // 重命名序号：只对「被接受」的文件递增（跳过的不占号）
    let mut next_seq: u32 = 1;

    for cand in candidates {
        if cand.path.file_name().is_none() {
            skipped.push(ImportSkipped {
                path: cand.path.clone(),
                reason: "无法解析文件名".to_string(),
            });
            continue;
        }
        let sub_dir = render_subdir(options.subfolder, &cand.date);
        let target_name = render_target_name(
            options.rename_template.as_deref(),
            &cand.path,
            &cand.date,
            next_seq,
        );
        // 目标已存在：同名同大小 = 去重跳过；同名不同大小 = 防覆盖跳过
        let target_path = dest_root.join(&sub_dir).join(&target_name);
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
        // 计划内同名冲突：同一目标目录两个源文件映射到同一目标名
        let planned_for_group = planned.entry(sub_dir.clone()).or_default();
        if let Some(&prev_size) = planned_for_group.get(&target_name) {
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
        // 接受该文件时才创建/复用组（全部被跳过的目标目录不产生空组）
        let group_pos = match group_index.get(&sub_dir) {
            Some(&i) => i,
            None => {
                groups.push(ImportGroup {
                    sub_dir: sub_dir.clone(),
                    files: Vec::new(),
                });
                let i = groups.len() - 1;
                group_index.insert(sub_dir.clone(), i);
                i
            }
        };
        planned_for_group.insert(target_name.clone(), cand.size);
        groups[group_pos].files.push(ImportFileTarget {
            source: cand.path.clone(),
            target_name,
        });
        next_seq += 1;
    }

    ImportPlan { groups, skipped }
}

/// 执行导入计划：逐文件委托 ops 复制/移动到计划里解析好的目标路径
/// （move 跨文件系统走 EXDEV 回退），返回逐文件结果（顺序 = 计划组顺序）。
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
        // 空子目录 = 直接放 dest_root
        let target_dir = if group.sub_dir.is_empty() {
            dest_root.to_path_buf()
        } else {
            dest_root.join(&group.sub_dir)
        };
        for file in &group.files {
            let dest = target_dir.join(&file.target_name);
            let result = import_one_file(&file.source, &dest, mode);
            done += 1;
            if let Some(cb) = &on_progress {
                cb(ImportProgress {
                    done,
                    total,
                    current: file.source.clone(),
                });
            }
            results.push((file.source.clone(), result));
        }
    }
    results
}

/// 单个文件导入：源存在性 + 目标存在性防御检查后，委托 ops 复制/移动到显式目标。
fn import_one_file(src: &Path, dest: &Path, mode: ImportMode) -> Result<(), ImportError> {
    if !src.exists() {
        return Err(ImportError::SourceNotFound(src.to_path_buf()));
    }
    // 计划与执行之间目标可能被并发写入：不静默覆盖（ops 层跨设备回退同样报错）
    if dest.exists() {
        return Err(ImportError::TargetExists(dest.to_path_buf()));
    }
    match mode {
        ImportMode::Copy => ops::copy_file_to(src, dest, false).map_err(ImportError::from)?,
        ImportMode::Move => ops::move_file_to(src, dest).map_err(ImportError::from)?,
    }
    Ok(())
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

/// sysfs 路径是否位于 USB 总线下（组件名以 "usb" 开头，如 usb8）。
/// 部分读卡器/移动硬盘把 removable 报成 0，但 sysfs 路径仍带 usbN 组件。
#[cfg(any(target_os = "linux", test))]
fn is_usb_sysfs_path(path: &Path) -> bool {
    path.components()
        .any(|c| c.as_os_str().to_string_lossy().starts_with("usb"))
}

#[cfg(target_os = "linux")]
mod linux {
    use super::{
        DriveInfo, block_device_name, block_disk_name, is_removable_mount_point,
        is_usb_sysfs_path, parse_proc_mounts,
    };
    use std::path::Path;

    /// 判定块设备是否可移动介质：
    /// 1. /sys/class/block/<name>/removable == "1"；
    /// 2. 否则看设备是否挂在 USB 总线下（读卡器常把 removable 报成 0，
    ///    如 sda removable=0 但 sysfs 路径含 usb8）；
    /// 分区节点无该属性时回退磁盘名（sdb1→sdb、mmcblk0p1→mmcblk0）。
    fn is_removable_device(dev_name: &str) -> bool {
        for name in [dev_name, block_disk_name(dev_name)] {
            if let Ok(v) = std::fs::read_to_string(format!("/sys/class/block/{name}/removable"))
                && v.trim() == "1"
            {
                return true;
            }
            if let Ok(real) = std::fs::canonicalize(format!("/sys/class/block/{name}"))
                && is_usb_sysfs_path(&real)
            {
                return true;
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
    fn test_is_usb_sysfs_path() {
        // 真实读卡器：sda removable=0，但 sysfs 路径含 usb8
        assert!(is_usb_sysfs_path(Path::new(
            "/sys/devices/pci0000:00/0000:00:08.3/0000:09:00.4/usb8/8-1/8-1.4/8-1.4:1.0/host0/target0:0:0/0:0:0:0/block/sda"
        )));
        // 内置 NVMe：无 usb 组件
        assert!(!is_usb_sysfs_path(Path::new(
            "/sys/devices/pci0000:00/0000:00:01.2/0000:02:00.0/nvme/nvme1/nvme1n1"
        )));
    }

    #[test]
    fn test_scan_import_source_skips_pt_cache_dir() {
        let dir = TempDir::new().unwrap();
        make_file(&dir, "IMG_1.jpg", b"photo");
        // 应用浏览过源盘后生成的缓存目录：其中的缩略图不能被当成本次导入的照片
        make_file(&dir, ".pt/thumbs/abc.jpg", b"cached-thumb");

        let cands = scan_import_source(dir.path()).unwrap();
        assert_eq!(cands.len(), 1, "候选: {:?}", cands.iter().map(|c| &c.path).collect::<Vec<_>>());
        assert!(cands[0].path.ends_with("IMG_1.jpg"));
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
        let plan = plan_import(&cands, &dest, &ImportOptions::default());
        assert_eq!(plan.skipped.len(), 0);
        assert_eq!(plan.groups.len(), 2, "应按日期分两组");
        assert_eq!(plan.groups[0].sub_dir, "2024-05-01");
        assert_eq!(plan.groups[0].files.len(), 2);
        assert_eq!(plan.groups[1].sub_dir, "2024-06-02");
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
        let plan = plan_import(&cands, &dest, &ImportOptions::default());
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
        let plan = plan_import(&cands, &dest, &ImportOptions::default());
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
        let plan = plan_import(&cands, &dest, &ImportOptions::default());
        assert_eq!(plan.groups.len(), 1);
        assert_eq!(plan.groups[0].files.len(), 1, "保留先者");
        assert_eq!(plan.skipped.len(), 1);
        assert!(plan.skipped[0].reason.contains("重复") || plan.skipped[0].reason.contains("冲突"));
    }

    #[test]
    fn test_plan_import_subfolder_modes_and_rename() {
        let dir = TempDir::new().unwrap();
        let s1 = make_file(&dir, "src/d1/IMG_1.JPG", b"11111");
        let s2 = make_file(&dir, "src/d2/IMG_2.JPG", b"22222");
        let dest = dir.path().join("dest");
        let cands = vec![cand(s1, "2024-05-01", 5), cand(s2, "2024-05-01", 5)];

        // 不建子目录：直接放 dest_root，保留原名
        let flat = plan_import(
            &cands,
            &dest,
            &ImportOptions {
                subfolder: ImportSubfolder::None,
                rename_template: None,
            },
        );
        assert_eq!(flat.groups.len(), 1);
        assert_eq!(flat.groups[0].sub_dir, "");
        assert_eq!(flat.groups[0].files[0].target_name, "IMG_1.JPG");

        // 斜杠日期 + 重命名模板（{date} 归一为 YYYYMMDD，{seq} 补零 3 位）
        let opts = ImportOptions {
            subfolder: ImportSubfolder::DateSlash,
            rename_template: Some("{date}_{seq}".to_string()),
        };
        let plan = plan_import(&cands, &dest, &opts);
        assert_eq!(plan.groups[0].sub_dir, "2024/05/01");
        assert_eq!(plan.groups[0].files[0].target_name, "20240501_001.JPG");
        assert_eq!(plan.groups[0].files[1].target_name, "20240501_002.JPG");

        // 紧凑日期
        let compact = plan_import(
            &cands,
            &dest,
            &ImportOptions {
                subfolder: ImportSubfolder::DateCompact,
                rename_template: None,
            },
        );
        assert_eq!(compact.groups[0].sub_dir, "20240501");
    }

    #[test]
    fn test_execute_import_flat_renamed_creates_dest_root() {
        let dir = TempDir::new().unwrap();
        let src = make_file(&dir, "src/a.jpg", b"aaa");
        // 目标根目录不存在：执行时自动创建（LR 式「新建文件夹」）
        let dest = dir.path().join("new/root");
        let plan = plan_import(
            &[cand(src.clone(), "2024-05-01", 3)],
            &dest,
            &ImportOptions {
                subfolder: ImportSubfolder::None,
                rename_template: Some("{seq}_{name}".to_string()),
            },
        );
        assert_eq!(plan.groups[0].files[0].target_name, "001_a.jpg");
        let results = execute_import(&plan, &dest, ImportMode::Copy, None);
        assert!(results[0].1.is_ok(), "{:?}", results[0].1);
        assert!(dest.join("001_a.jpg").is_file(), "目标根目录应自动创建");
    }

    #[test]
    fn test_plan_import_empty() {
        let dir = TempDir::new().unwrap();
        let plan = plan_import(&[], &dir.path().join("dest"), &ImportOptions::default());
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
            &ImportOptions::default(),
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
        let plan = plan_import(&[cand(s1.clone(), "2024-05-01", 3)], &dest, &ImportOptions::default());
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
        let plan = plan_import(&[cand(missing.clone(), "2024-05-01", 3)], &dest, &ImportOptions::default());
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
        let plan = plan_import(&[cand(s1.clone(), "2024-05-01", 3)], &dest, &ImportOptions::default());
        // 计划后目标被并发写入 → 执行时 TargetExists 报错（不覆盖）
        make_file(&dir, "dest/2024-05-01/a.jpg", b"concurrent");
        let results = execute_import(&plan, &dest, ImportMode::Copy, None);
        assert!(matches!(results[0].1, Err(ImportError::TargetExists(_))), "{:?}", results[0].1);
        // 目标内容未被覆盖
        assert_eq!(std::fs::read(dest.join("2024-05-01/a.jpg")).unwrap(), b"concurrent");
        assert!(s1.is_file());
    }
}
