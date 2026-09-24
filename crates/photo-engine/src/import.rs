//! 导入功能（SD 卡 → 按日期建目录 → 去重 → 复制/移动）
//!
//! 全同步实现（与 scanner/ops 一致，core 层禁止 async）。流程：
//!   1. `detect_removable_drives` —— 检测可移动驱动器（Windows 原生 API）
//!   2. `scan_import_source`     —— 递归扫描源，日期取文件修改时间（只 stat，不读 EXIF），
//!      同时标出媒体类别（照片/RAW/视频）与随行 .xmp
//!   3. `plan_import`            —— 过滤（类别 / 跳过与 RAW 同名的 JPEG）+ 按 YYYY-MM-DD 分组
//!      + 三层去重（**源卷账本** → 目标存在性 → 计划内冲突）+ 冲突策略（跳过/改名/覆盖）
//!   4. `execute_import`         —— 逐文件委托 ops 复制/移动，进度回调 + 可取消 + 可选校验，
//!      返回逐文件 `ImportReport`（成功/失败/警告/是否取消），并回写 mtime、随行 .xmp sidecar、
//!      把完成项写进断点日志（`<目标根>/.pt/import-journal.jsonl`，供中断续传）
//!   5. `eject_source`           —— 导入完成后卸载源盘（Linux udisksctl；其余平台明确报不支持）
//!
//! 驱动器检测说明：本模块不引入 `windows` crate，
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

use crate::import_history::{ImportHistory, ImportJournal};
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
    #[error("{0}")]
    Verify(String),
    #[error("{0}")]
    Eject(String),
}

/// 可移动驱动器信息
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveInfo {
    /// 根路径（Windows 如 "E:\\"，Linux 为挂载点如 "/run/media/user/CANON"）
    pub path: String,
    /// 卷标（读取失败/空卷标为 None；Linux 取挂载点末段——udisks 按卷标挂载）
    pub label: Option<String>,
}

/// 媒体类别（导入过滤与分诊的最小粒度）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MediaKind {
    /// 常规图片（JPEG/PNG/TIFF/HEIF/WebP/…）
    Photo,
    /// 相机 RAW
    Raw,
    /// 视频等非图片可查看格式
    Video,
}

impl MediaKind {
    /// 由扩展名推断（无法识别时按 Photo 处理，哨兵不会被静默过滤掉）
    pub fn of(path: &Path) -> Self {
        match path
            .extension()
            .and_then(|e| e.to_str())
            .and_then(ImageFormat::from_extension)
        {
            Some(ImageFormat::Raw(_)) => MediaKind::Raw,
            Some(ImageFormat::Other) => MediaKind::Video,
            _ => MediaKind::Photo,
        }
    }

    /// 界面标签
    pub fn label(self) -> &'static str {
        match self {
            MediaKind::Photo => "照片",
            MediaKind::Raw => "RAW",
            MediaKind::Video => "视频",
        }
    }
}

/// 路径是否为 JPEG（RAW+JPEG 双拍判定只认 JPEG）
fn is_jpeg(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .and_then(ImageFormat::from_extension),
        Some(ImageFormat::Jpeg)
    )
}

/// 导入候选文件（扫描产物）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportCandidate {
    /// 源文件完整路径
    pub path: PathBuf,
    /// 文件日期 YYYY-MM-DD（取文件修改时间；不读 EXIF——相机挂载下读整文件太慢）
    pub date: String,
    /// 文件大小（字节，去重用）
    pub size: u64,
    /// 文件修改时间（纳秒；账本用它识别「源文件被改过」）
    pub mtime_ns: i64,
    /// 媒体类别
    pub kind: MediaKind,
    /// 随行 sidecar（同目录同 stem 的 .xmp，扩展名大小写不敏感）；无则空
    pub sidecars: Vec<PathBuf>,
}

/// 导入计划的单文件目标：源路径 + 目标文件名（可被重命名模板改写）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportFileTarget {
    /// 源文件完整路径
    pub source: PathBuf,
    /// 目标文件名（含扩展名）
    pub target_name: String,
    /// 随行 sidecar：源路径 + 目标文件名（跟随主文件改名，只换扩展名）
    pub sidecars: Vec<(PathBuf, String)>,
    /// 覆盖策略放行（目标已存在同名不同大小且策略 = 覆盖）
    pub overwrite: bool,
    /// 源文件大小（落地校验用）
    pub size: u64,
}

/// 按子目录分组的导入计划组（目标目录 = dest_root/sub_dir/；sub_dir 为空 = 直接放 dest_root）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportGroup {
    /// 目标子目录（相对 dest_root 的正斜杠路径；空串 = 不建子目录）
    pub sub_dir: String,
    /// 该组内的源文件与目标文件名
    pub files: Vec<ImportFileTarget>,
}

/// 跳过类别（分诊条按它计数；与界面上的四分类一一对应）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// 被过滤（类别没勾 / 跳过与 RAW 同名的 JPEG）
    Filtered,
    /// 已导入过（账本命中 / 目标同名同大小 / 断点日志里已完成）
    AlreadyImported,
    /// 目标同名但大小不同，且冲突策略 = 跳过
    Conflict,
    /// 源内重复（两个候选映射同一目标）
    SourceDuplicate,
    /// 其他（文件名解析失败等）
    Other,
}

impl SkipReason {
    /// 界面标签
    pub fn label(self) -> &'static str {
        match self {
            SkipReason::Filtered => "已过滤",
            SkipReason::AlreadyImported => "已导入",
            SkipReason::Conflict => "冲突",
            SkipReason::SourceDuplicate => "源内重复",
            SkipReason::Other => "其他",
        }
    }
}

/// 计划阶段被跳过的文件
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportSkipped {
    /// 源文件完整路径
    pub path: PathBuf,
    /// 跳过原因（中文，直接透传界面）
    pub reason: String,
    /// 跳过类别（分诊计数）
    pub category: SkipReason,
}

/// 导入计划（干跑结果，不碰文件）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportPlan {
    /// 按日期分组（保持候选顺序）
    pub groups: Vec<ImportGroup>,
    /// 跳过清单（过滤 / 去重 / 冲突 / 源内重复）
    pub skipped: Vec<ImportSkipped>,
}

/// 计划分诊统计（「新 / 已导入 / 冲突 / 源内重复 / 已过滤」）
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ImportPlanStats {
    /// 本次会搬的新文件数
    pub new: u32,
    /// 已导入过（账本 / 目标同名同大小 / 断点日志）
    pub already_imported: u32,
    /// 目标同名但大小不同，策略 = 跳过
    pub conflict: u32,
    /// 源内重复
    pub source_duplicate: u32,
    /// 被过滤
    pub filtered: u32,
    /// 其他跳过
    pub other: u32,
}

impl ImportPlan {
    /// 本次计划要搬运的文件总数
    pub fn file_count(&self) -> usize {
        self.groups.iter().map(|g| g.files.len()).sum()
    }

    /// 跳过总数
    pub fn skipped_count(&self) -> usize {
        self.skipped.len()
    }

    /// 分诊统计（界面顶部那条「新 812 · 已导入 24 · …」）
    pub fn stats(&self) -> ImportPlanStats {
        let mut stats = ImportPlanStats {
            new: self.file_count() as u32,
            ..Default::default()
        };
        for entry in &self.skipped {
            match entry.category {
                SkipReason::Filtered => stats.filtered += 1,
                SkipReason::AlreadyImported => stats.already_imported += 1,
                SkipReason::Conflict => stats.conflict += 1,
                SkipReason::SourceDuplicate => stats.source_duplicate += 1,
                SkipReason::Other => stats.other += 1,
            }
        }
        stats
    }
}

/// 执行阶段的一条问题记录（失败 / 警告共用）：源 → 目标 + 中文原因
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportIssue {
    /// 出问题的源文件（时间戳警告时为照片本身）
    pub source: PathBuf,
    /// 对应的目标路径
    pub target: PathBuf,
    /// 中文原因（直接透传到界面）
    pub reason: String,
}

/// 导入执行报告：逐文件结果，取代原先只回传 `Vec<(源, Result)>` 的形态。
///
/// 区分「失败」与「警告」：失败 = 文件没到目标；警告 = 主文件已到目标，
/// 但附带动作（mtime 回写 / sidecar 搬运）没做成——不能把已完成的主文件算作失败。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportReport {
    /// 成功搬运：源 → 目标
    pub ok: Vec<(PathBuf, PathBuf)>,
    /// 失败：文件没到目标
    pub failed: Vec<ImportIssue>,
    /// 警告：主文件成功，附带动作失败
    pub warnings: Vec<ImportIssue>,
    /// 是否被取消（剩余文件未处理，数量由调用方用计划总数推算）
    pub cancelled: bool,
}

impl ImportReport {
    /// 成功搬运的文件数
    pub fn imported(&self) -> usize {
        self.ok.len()
    }

    /// 未处理的文件数 = 计划总数 − 成功 − 失败（取消时 > 0）
    pub fn unprocessed(&self, total: usize) -> usize {
        total.saturating_sub(self.ok.len() + self.failed.len())
    }
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
/// 日期 = 文件修改时间（YYYY-MM-DD）：只 stat 不读文件内容——相机 USB 挂载下持续读只有
/// ~1.7MB/s，逐张读 EXIF 会让整卡扫描从瞬时变成十几分钟。元数据读取失败的文件跳过，
/// 返回按完整路径排序。
pub fn scan_import_source(dir: &Path) -> Result<Vec<ImportCandidate>, ImportError> {
    if !dir.is_dir() {
        return Err(ImportError::Io(std::io::Error::new(
            ErrorKind::NotFound,
            format!("源目录不存在: {}", dir.display()),
        )));
    }
    // 1. 收集可查看文件 + 同 stem 的 .xmp sidecar（跳过应用自己的 .pt 缓存目录）
    let mut files: Vec<(PathBuf, ImageFormat)> = Vec::new();
    // (父目录, 小写 stem) → sidecar 列表：与照片配对，随照片一起搬运（照片被跳过时它也不走）
    let mut sidecars: HashMap<(PathBuf, String), Vec<PathBuf>> = HashMap::new();
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
        // .xmp 不是可查看格式，单独收集：导入把评分/关键词留在源上是很常见的用户抱怨
        if ext.eq_ignore_ascii_case("xmp") {
            if let (Some(parent), Some(stem)) = (path.parent(), path.file_stem()) {
                sidecars
                    .entry((parent.to_path_buf(), stem.to_string_lossy().to_lowercase()))
                    .or_default()
                    .push(path.to_path_buf());
            }
            continue;
        }
        let Some(format) = ImageFormat::from_extension(ext) else {
            continue;
        };
        files.push((path.to_path_buf(), format));
    }
    // 确定性顺序（walkdir 目录序不定）：按完整路径排序
    files.sort_by(|a, b| a.0.cmp(&b.0));

    // 2. 组装候选：日期取文件修改时间（相机写入时即拍摄时间），不读 EXIF——
    //    相机挂载下读整文件每张要 2–3 秒，整卡要十几分钟；stat 只取元数据，瞬时完成。
    let mut candidates = Vec::with_capacity(files.len());
    for (path, _) in files {
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        let mut attached = match (path.parent(), path.file_stem()) {
            (Some(parent), Some(stem)) => sidecars
                .remove(&(parent.to_path_buf(), stem.to_string_lossy().to_lowercase()))
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        attached.sort();
        let date = mtime_date(&meta);
        let mtime_ns = mtime_nanos(&meta);
        let kind = MediaKind::of(&path);
        candidates.push(ImportCandidate {
            path,
            date,
            size: meta.len(),
            mtime_ns,
            kind,
            sidecars: attached,
        });
    }
    Ok(candidates)
}

/// 文件修改时间 → Unix 纳秒（读不到按 0）。账本用它识别「源文件被改过」——
/// 大小 + mtime 双双对不上就不算命中，不会把重新导出过的文件静默跳过。
fn mtime_nanos(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

/// 文件修改时间 → YYYY-MM-DD（读不到按纪元日兜底）
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

/// 导入过滤（默认全收）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportFilter {
    /// 收常规图片
    pub photos: bool,
    /// 收 RAW
    pub raw: bool,
    /// 收视频
    pub videos: bool,
    /// RAW+JPEG 双拍时跳过 JPEG（同目录同 stem 且该 stem 有 RAW）
    pub skip_jpeg_with_raw: bool,
}

impl Default for ImportFilter {
    fn default() -> Self {
        Self {
            photos: true,
            raw: true,
            videos: true,
            skip_jpeg_with_raw: false,
        }
    }
}

impl ImportFilter {
    /// 该类别是否收
    pub fn accepts(&self, kind: MediaKind) -> bool {
        match kind {
            MediaKind::Photo => self.photos,
            MediaKind::Raw => self.raw,
            MediaKind::Video => self.videos,
        }
    }
}

/// 目标同名冲突策略（仅对「同名但大小不同」生效；同名同大小一律按已导入跳过）
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ConflictPolicy {
    /// 跳过并记入冲突清单（默认，最安全）
    #[default]
    Skip,
    /// 自动改名 `IMG_1_1.jpg` 后落地
    Rename,
    /// 覆盖目标（危险动作，界面上要显式选）
    Overwrite,
}

impl ConflictPolicy {
    /// 界面标签
    pub fn label(self) -> &'static str {
        match self {
            ConflictPolicy::Skip => "跳过",
            ConflictPolicy::Rename => "自动改名",
            ConflictPolicy::Overwrite => "覆盖",
        }
    }
}

/// 导入的目录/命名选项 + 过滤 + 冲突策略 + 校验
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportOptions {
    /// 子目录模式
    pub subfolder: ImportSubfolder,
    /// 文件重命名模板（None/空 = 保留原名）；占位符复用 template.rs：
    /// {name} 原名(无扩展) / {date} 文件日期 YYYYMMDD / {seq} 序号（补零 3 位）
    pub rename_template: Option<String>,
    /// 媒体类别过滤
    pub filter: ImportFilter,
    /// 目标同名冲突策略
    pub policy: ConflictPolicy,
    /// 复制后校验落地文件大小（默认开：能抓住短写/坏读卡器，代价是一次 stat）
    pub verify: bool,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            subfolder: ImportSubfolder::default(),
            rename_template: None,
            filter: ImportFilter::default(),
            policy: ConflictPolicy::default(),
            verify: true,
        }
    }
}

/// 计划期的额外判据：源卷账本（去重正解）+ 断点日志（中断续传）。
/// 两者都缺省 = 退化成「只按目标存在性去重」，不会误判。
#[derive(Default)]
pub struct PlanExtras<'a> {
    /// 源卷账本
    pub ledger: Option<SourceLedger<'a>>,
    /// 上次中断留下的断点日志
    pub resume: Option<&'a ImportJournal>,
}

/// 源卷账本视图：账本按「源内相对路径」记账，所以必须带上源根
pub struct SourceLedger<'a> {
    /// 导入源根（账本相对路径的基准）
    pub source_root: &'a Path,
    /// 源卷标识（见 `import_history::volume_id_for_path`）
    pub volume_id: &'a str,
    /// 账本
    pub history: &'a ImportHistory,
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

/// 生成导入计划（干跑，不碰文件）。判定顺序（先便宜后昂贵、先确定性后启发式）：
///   1. **过滤**：媒体类别没勾 → 已过滤；勾了「跳过与 RAW 同名的 JPEG」且该 stem 有 RAW → 已过滤
///   2. **账本**（Layer 1）：源卷 + 源内相对路径 + 大小 + mtime 命中 → 已导入
///   3. **断点日志**：上次中断的导入里已完成 → 已导入
///   4. **目标存在性**（Layer 2）：同名同大小 → 已导入（内容一致，任何策略都不必再搬）；
///      同名不同大小 → 按冲突策略 跳过 / 自动改名 / 覆盖
///   5. **计划内冲突**（Layer 3）：两个源映射同一目标 → 保留先者，后者记源内重复
pub fn plan_import(
    candidates: &[ImportCandidate],
    dest_root: &Path,
    options: &ImportOptions,
    extras: &PlanExtras<'_>,
) -> ImportPlan {
    let mut groups: Vec<ImportGroup> = Vec::new();
    let mut skipped: Vec<ImportSkipped> = Vec::new();
    // sub_dir → 组索引（保持候选首次出现顺序）
    let mut group_index: HashMap<String, usize> = HashMap::new();
    // sub_dir → (目标文件名 → 源大小)：已计划目标，防同名互踩
    let mut planned: HashMap<String, HashMap<String, u64>> = HashMap::new();
    // 重命名序号：**每个候选都占号**（= 候选列表里的 1-based 位置）。
    //
    // 曾经是「只对被接受的文件递增」，但那样续传/重算会漂移：第一次导 A B C 得 001/002/003
    // 后中断，第二次 A B C 被断点日志跳过、D 变成 001 → 撞上已存在的 20240501_001.jpg
    // （策略=跳过就整批被误判成冲突）。按候选位置编号则任何一次重算都给出同一个名字。
    let mut next_seq: u32 = 1;

    // 「同目录同 stem 存在 RAW」的集合：勾了「跳过与 RAW 同名的 JPEG」时才算
    let raw_stems: std::collections::HashSet<(PathBuf, String)> =
        if options.filter.skip_jpeg_with_raw {
            candidates
                .iter()
                .filter(|c| c.kind == MediaKind::Raw)
                .filter_map(|c| {
                    Some((
                        c.path.parent()?.to_path_buf(),
                        c.path.file_stem()?.to_string_lossy().to_lowercase(),
                    ))
                })
                .collect()
        } else {
            std::collections::HashSet::new()
        };

    for cand in candidates {
        if cand.path.file_name().is_none() {
            skipped.push(ImportSkipped {
                path: cand.path.clone(),
                reason: "无法解析文件名".to_string(),
                category: SkipReason::Other,
            });
            next_seq += 1;
            continue;
        }
        // 序号在任何判定之前就定下来：跳过与否都不影响这张照片的编号
        let seq = next_seq;
        next_seq += 1;

        // ── 1. 过滤 ──
        if !options.filter.accepts(cand.kind) {
            skipped.push(skip_entry(
                cand,
                format!("按设置跳过{}", cand.kind.label()),
                SkipReason::Filtered,
            ));
            continue;
        }
        if options.filter.skip_jpeg_with_raw && is_jpeg(&cand.path) {
            let key = (
                cand.path
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_default(),
                cand.path
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_lowercase())
                    .unwrap_or_default(),
            );
            if raw_stems.contains(&key) {
                skipped.push(skip_entry(
                    cand,
                    "与 RAW 同名（已按设置跳过 JPEG）".to_string(),
                    SkipReason::Filtered,
                ));
                continue;
            }
        }

        // ── 2. 源卷账本 ──
        // 账本命中还要求「当时的目标文件仍在」：用户删掉目标后再插卡，必须能重新导进来，
        // 不能让一条历史记录永久挡住这张照片
        if let Some(ledger) = &extras.ledger
            && let Ok(rel) = cand.path.strip_prefix(ledger.source_root)
            && let Ok(Some(previous)) = ledger.history.lookup(
                ledger.volume_id,
                &rel.to_string_lossy(),
                cand.size,
                cand.mtime_ns,
            )
            && Path::new(&previous).exists()
        {
            skipped.push(skip_entry(
                cand,
                format!("导入账本：已导入到 {previous}"),
                SkipReason::AlreadyImported,
            ));
            continue;
        }

        // ── 3. 断点日志（上次中断已完成） ──
        if let Some(resume) = &extras.resume
            && resume.completed().contains(&cand.path)
        {
            skipped.push(skip_entry(
                cand,
                "上次导入已完成".to_string(),
                SkipReason::AlreadyImported,
            ));
            continue;
        }

        let sub_dir = render_subdir(options.subfolder, &cand.date);
        let mut target_name = render_target_name(
            options.rename_template.as_deref(),
            &cand.path,
            &cand.date,
            seq,
        );
        let target_dir = dest_root.join(&sub_dir);
        let mut overwrite = false;

        // ── 4. 目标存在性 ──
        if let Ok(meta) = std::fs::metadata(target_dir.join(&target_name)) {
            if meta.len() == cand.size {
                // 同名同大小 = 内容一致：任何策略下都没必要再搬一遍
                skipped.push(skip_entry(
                    cand,
                    "目标已存在且大小相同".to_string(),
                    SkipReason::AlreadyImported,
                ));
                continue;
            }
            match options.policy {
                ConflictPolicy::Skip => {
                    skipped.push(skip_entry(
                        cand,
                        "目标已存在同名文件（大小不同）".to_string(),
                        SkipReason::Conflict,
                    ));
                    continue;
                }
                ConflictPolicy::Rename => {
                    target_name =
                        unique_target_name(&target_dir, &target_name, planned.get(&sub_dir));
                }
                ConflictPolicy::Overwrite => overwrite = true,
            }
        }

        // ── 5. 计划内同名冲突 ──
        let planned_for_group = planned.entry(sub_dir.clone()).or_default();
        if let Some(&prev_size) = planned_for_group.get(&target_name) {
            if options.policy == ConflictPolicy::Rename {
                // 源内同名也按改名处理：两个源目录下的 IMG_1 是两张不同的照片，不该丢一张
                target_name =
                    unique_target_name(&target_dir, &target_name, Some(&*planned_for_group));
            } else {
                let reason = match options.policy {
                    ConflictPolicy::Overwrite => {
                        // 覆盖在这里**不能**照搬：被覆盖的是本批里还没搬的先者，等于丢文件
                        "源内同名（覆盖不适用，保留先者）".to_string()
                    }
                    _ if prev_size == cand.size => "源内重复（另一候选同名同大小）".to_string(),
                    _ => "源内同名冲突（另一候选同名不同大小）".to_string(),
                };
                skipped.push(skip_entry(cand, reason, SkipReason::SourceDuplicate));
                continue;
            }
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
        // sidecar 跟随主文件改名：只换扩展名，stem 取主文件的目标 stem
        let target_stem = Path::new(&target_name)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| target_name.clone());
        let sidecar_targets: Vec<(PathBuf, String)> = cand
            .sidecars
            .iter()
            .map(|sc| {
                let ext = sc
                    .extension()
                    .map(|e| e.to_string_lossy().to_string())
                    .unwrap_or_else(|| "xmp".to_string());
                (sc.clone(), format!("{target_stem}.{ext}"))
            })
            .collect();
        planned_for_group.insert(target_name.clone(), cand.size);
        groups[group_pos].files.push(ImportFileTarget {
            source: cand.path.clone(),
            target_name,
            sidecars: sidecar_targets,
            overwrite,
            size: cand.size,
        });
    }

    ImportPlan { groups, skipped }
}

/// 跳过记录构造（统一 category + 中文原因）
fn skip_entry(cand: &ImportCandidate, reason: String, category: SkipReason) -> ImportSkipped {
    ImportSkipped {
        path: cand.path.clone(),
        reason,
        category,
    }
}

/// 冲突自动改名：`IMG_1.jpg` → `IMG_1_1.jpg`，一直试到目标目录与计划里都没占用。
/// 试满 10000 次就退回原名——执行阶段会按 TargetExists 报错，绝不静默覆盖。
fn unique_target_name(dir: &Path, desired: &str, planned: Option<&HashMap<String, u64>>) -> String {
    let path = Path::new(desired);
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| desired.to_string());
    let ext = path.extension().map(|e| e.to_string_lossy().to_string());
    for i in 1..10_000u32 {
        let candidate = match &ext {
            Some(ext) => format!("{stem}_{i}.{ext}"),
            None => format!("{stem}_{i}"),
        };
        let taken =
            dir.join(&candidate).exists() || planned.is_some_and(|map| map.contains_key(&candidate));
        if !taken {
            return candidate;
        }
    }
    desired.to_string()
}

/// 执行导入计划：逐文件委托 ops 复制/移动到计划里解析好的目标路径
/// （move 跨文件系统走 EXDEV 回退），返回逐文件执行报告（顺序 = 计划组顺序）。
///
/// `should_cancel` — 每个文件**开始前**询问一次；为真则立即停止：已处理的文件保留在报告里
/// （`ImportReport::cancelled = true`），剩余文件一个都不碰。取消粒度是「一个文件」——
/// `std::fs::copy` 本身不可中断，正在写的大视频不会被腰斩。
///
/// `on_progress` — 每个文件处理后回调（done 从 1 开始；total = 计划文件总数）。
///
/// 两个附带动作（失败只记 `warnings`，不影响主文件成败）：
/// - 搬运后把 mtime 回写成源的时间戳——`std::fs::copy` 不保留时间戳，
///   跨设备 move 的 EXDEV 回退同样会丢，而卡/外接盘导入正是跨设备最常见的一档
/// - 同 stem 的 `.xmp` sidecar 跟随主文件改名一起搬运
pub fn execute_import(
    plan: &ImportPlan,
    dest_root: &Path,
    mode: ImportMode,
    should_cancel: impl Fn() -> bool,
    verify: bool,
    mut journal: Option<ImportJournal>,
    on_progress: Option<Box<dyn Fn(ImportProgress) + Send>>,
) -> ImportReport {
    let total = plan.groups.iter().map(|g| g.files.len() as u32).sum::<u32>();
    let mut report = ImportReport::default();
    let mut done: u32 = 0;

    'groups: for group in &plan.groups {
        // 空子目录 = 直接放 dest_root
        let target_dir = if group.sub_dir.is_empty() {
            dest_root.to_path_buf()
        } else {
            dest_root.join(&group.sub_dir)
        };
        for file in &group.files {
            if should_cancel() {
                report.cancelled = true;
                break 'groups;
            }
            let dest = target_dir.join(&file.target_name);
            // mtime 必须在搬运前取：move 成功后源已经不存在了
            let src_mtime = std::fs::metadata(&file.source)
                .ok()
                .and_then(|m| m.modified().ok());

            match import_one_file(&file.source, &dest, mode, file.overwrite, verify) {
                Ok(()) => {
                    // 断点日志：逐条 flush。写失败不影响导入本身（只是丢了续传能力），
                    // 所以不做逐条 warning——否则坏盘上会刷满一屏。
                    if let Some(j) = journal.as_mut() {
                        let _ = j.record(&file.source, &dest, file.size);
                    }
                    if let Some(t) = src_mtime
                        && let Err(reason) = preserve_mtime(&dest, t)
                    {
                        report.warnings.push(ImportIssue {
                            source: file.source.clone(),
                            target: dest.clone(),
                            reason: format!("时间戳未保留：{reason}"),
                        });
                    }
                    for (sc_src, sc_name) in &file.sidecars {
                        let sc_dest = target_dir.join(sc_name);
                        // 覆盖策略与主文件同一口径：用户选「覆盖」时 sidecar 也覆盖
                        if let Err(err) =
                            import_one_file(sc_src, &sc_dest, mode, file.overwrite, verify)
                        {
                            report.warnings.push(ImportIssue {
                                source: sc_src.clone(),
                                target: sc_dest,
                                reason: format!("sidecar 未搬运：{err}"),
                            });
                        }
                    }
                    report.ok.push((file.source.clone(), dest));
                }
                Err(err) => report.failed.push(ImportIssue {
                    source: file.source.clone(),
                    target: dest,
                    reason: err.to_string(),
                }),
            }

            done += 1;
            if let Some(cb) = &on_progress {
                cb(ImportProgress {
                    done,
                    total,
                    current: file.source.clone(),
                });
            }
        }
    }
    report
}

/// 把目标文件的修改时间设回源的时间戳（`std::fs::copy` 不保留时间戳）。
/// 目标只读 / 文件系统不支持时报错，由调用方降级成 warning。
fn preserve_mtime(dest: &Path, mtime: std::time::SystemTime) -> Result<(), String> {
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(dest)
        .map_err(|e| e.to_string())?;
    file.set_modified(mtime).map_err(|e| e.to_string())
}

/// 单个文件导入：源存在性 + 目标存在性防御检查后，委托 ops 复制/移动到显式目标。
///
/// - `overwrite`：冲突策略放行时才为 true。显式删掉目标再搬（而不是让底层覆盖），
///   因为 move 的 rename 撞上已存在文件在部分平台行为不一致
/// - `verify`：搬完比对大小。**Move + verify 必须走「复制 → 校验 → 删源」**，
///   绝不能先删源——坏读卡器会让源没了、目标还是残的
fn import_one_file(
    src: &Path,
    dest: &Path,
    mode: ImportMode,
    overwrite: bool,
    verify: bool,
) -> Result<(), ImportError> {
    if !src.exists() {
        return Err(ImportError::SourceNotFound(src.to_path_buf()));
    }
    if dest.exists() {
        // 计划与执行之间目标可能被并发写入：没放行覆盖就报错（ops 层同样不静默覆盖）
        if !overwrite {
            return Err(ImportError::TargetExists(dest.to_path_buf()));
        }
        // 覆盖走「同目录临时文件 + rename 原子替换」：**绝不先删旧文件**——
        // 先删再拷的话，满盘/坏卡让拷贝失败时，用户既丢了新数据也丢了目标上原来那份。
        let tmp = temp_sibling(dest);
        let _ = std::fs::remove_file(&tmp); // 清掉上次中断留下的残留
        if let Err(err) = transfer(src, &tmp, mode, verify) {
            let _ = std::fs::remove_file(&tmp);
            return Err(err);
        }
        return std::fs::rename(&tmp, dest).map_err(ImportError::from);
    }
    transfer(src, dest, mode, verify)
}

/// 同目录临时名（`photo.jpg.pt-tmp`）：与目标同文件系统，rename 过去是原子的；
/// `.pt-tmp` 不在可查看扩展名白名单里，后续扫描不会把它当照片。
fn temp_sibling(dest: &Path) -> PathBuf {
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    dest.with_file_name(format!("{name}.pt-tmp"))
}

/// 搬运主体（调用方保证 dest 可写、不存在）：复制 / 移动，按需校验。
///
/// Move + verify 必须是「复制 → 校验 → 删源」，且任一步失败都要把半成品目标删掉——
/// 保持「文件要么在源、要么在目标」的唯一性。
fn transfer(src: &Path, dest: &Path, mode: ImportMode, verify: bool) -> Result<(), ImportError> {
    match (mode, verify) {
        (ImportMode::Move, false) => ops::move_file_to(src, dest).map_err(ImportError::from),
        (ImportMode::Move, true) => {
            ops::copy_file_to(src, dest, false).map_err(ImportError::from)?;
            if let Err(reason) = verify_size(src, dest) {
                let _ = std::fs::remove_file(dest);
                return Err(ImportError::Verify(reason));
            }
            if let Err(err) = std::fs::remove_file(src) {
                // 删源失败：把刚落地的目标删掉，保持上面那条唯一性
                let _ = std::fs::remove_file(dest);
                return Err(ImportError::Io(err));
            }
            Ok(())
        }
        (ImportMode::Copy, _) => {
            ops::copy_file_to(src, dest, false).map_err(ImportError::from)?;
            if verify
                && let Err(reason) = verify_size(src, dest)
            {
                let _ = std::fs::remove_file(dest);
                return Err(ImportError::Verify(reason));
            }
            Ok(())
        }
    }
}

/// 落地校验：目标大小必须等于源（短写 / 坏读卡器 / 满盘会在这一步暴露）
fn verify_size(src: &Path, dest: &Path) -> Result<(), String> {
    let want = std::fs::metadata(src).map(|m| m.len()).map_err(|e| e.to_string())?;
    let got = std::fs::metadata(dest).map(|m| m.len()).map_err(|e| e.to_string())?;
    if want == got {
        Ok(())
    } else {
        Err(format!("落地校验失败：期望 {want} 字节，实际 {got} 字节"))
    }
}

/// 导入完成后卸载源盘：Linux 走 `udisksctl unmount -b <设备>`（桌面标配、不需要 root）。
/// Windows / macOS 没有零依赖的等价能力，明确报「不支持」而不是静默什么都不做。
pub fn eject_source(mount_point: &Path) -> Result<(), ImportError> {
    #[cfg(target_os = "linux")]
    {
        let device = linux::device_of_mount(mount_point).ok_or_else(|| {
            ImportError::Eject(format!(
                "找不到挂载点对应的设备：{}",
                mount_point.display()
            ))
        })?;
        let output = std::process::Command::new("udisksctl")
            .args(["unmount", "-b"])
            .arg(&device)
            .output()
            .map_err(|e| ImportError::Eject(format!("调用 udisksctl 失败：{e}")))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ImportError::Eject(format!(
                "卸载失败：{}",
                stderr.trim()
            )));
        }
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = mount_point;
        Err(ImportError::Eject(
            "当前平台暂不支持自动弹出，请手动安全移除".to_string(),
        ))
    }
}

/// 是否支持自动弹出（界面据此决定要不要显示这个开关）
pub fn eject_supported() -> bool {
    cfg!(target_os = "linux")
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

    /// 挂载点 → 设备（精确匹配 /proc/mounts；弹出源盘时用）
    pub(super) fn device_of_mount(mount: &Path) -> Option<String> {
        let mounts = std::fs::read_to_string("/proc/mounts").ok()?;
        let want = mount.to_string_lossy();
        parse_proc_mounts(&mounts)
            .into_iter()
            .find(|(_, m)| *m == want)
            .map(|(dev, _)| dev)
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
            mtime_ns: 0,
            kind: MediaKind::Photo,
            sidecars: Vec::new(),
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

    #[test]
    fn test_mtime_fallback_plain_file() {
        // 无 EXIF 的普通文件：scan_import_source 走 mtime 回退
        let dir = TempDir::new().unwrap();
        let path = make_file(&dir, "plain.jpg", b"no-exif");
        let meta = std::fs::metadata(&path).unwrap();
        let cands = scan_import_source(dir.path()).unwrap();
        let c = cands.iter().find(|c| c.path == path).unwrap();
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
        let plan = plan_import(&cands, &dest, &ImportOptions::default(), &PlanExtras::default());
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
        let plan = plan_import(&cands, &dest, &ImportOptions::default(), &PlanExtras::default());
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
        let plan = plan_import(&cands, &dest, &ImportOptions::default(), &PlanExtras::default());
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
        let plan = plan_import(&cands, &dest, &ImportOptions::default(), &PlanExtras::default());
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
                ..ImportOptions::default()
            },
            &PlanExtras::default(),
        );
        assert_eq!(flat.groups.len(), 1);
        assert_eq!(flat.groups[0].sub_dir, "");
        assert_eq!(flat.groups[0].files[0].target_name, "IMG_1.JPG");

        // 斜杠日期 + 重命名模板（{date} 归一为 YYYYMMDD，{seq} 补零 3 位）
        let opts = ImportOptions {
            subfolder: ImportSubfolder::DateSlash,
            rename_template: Some("{date}_{seq}".to_string()),
            ..ImportOptions::default()
        };
        let plan = plan_import(&cands, &dest, &opts, &PlanExtras::default());
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
                ..ImportOptions::default()
            },
            &PlanExtras::default(),
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
                ..ImportOptions::default()
            },
            &PlanExtras::default(),
        );
        assert_eq!(plan.groups[0].files[0].target_name, "001_a.jpg");
        let report = execute_import(&plan, &dest, ImportMode::Copy, || false, false, None, None);
        assert_eq!(report.ok.len(), 1, "{:?}", report.failed);
        assert!(dest.join("001_a.jpg").is_file(), "目标根目录应自动创建");
    }

    #[test]
    fn test_plan_import_empty() {
        let dir = TempDir::new().unwrap();
        let plan = plan_import(&[], &dir.path().join("dest"), &ImportOptions::default(), &PlanExtras::default());
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
            &PlanExtras::default(),
        );
        assert_eq!(plan.skipped.len(), 0);

        // 进度回调：done 1..=2，total 2，current 顺序 = 计划顺序
        // （Box<dyn Fn + Send> 需 'static：用 Arc<Mutex> 共享收集容器，调用后取回）
        let seen = std::sync::Arc::new(parking_lot::Mutex::new(Vec::new()));
        let seen_cb = seen.clone();
        let report = execute_import(
            &plan,
            &dest,
            ImportMode::Copy,
            || false,
            false,
            None,
            Some(Box::new(move |p| {
                seen_cb.lock().push((p.done, p.total, p.current.to_string_lossy().to_string()));
            })),
        );
        let seen = std::sync::Arc::try_unwrap(seen).unwrap().into_inner();
        assert_eq!(report.ok.len(), 2, "复制应全部成功: {:?}", report.failed);
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
        let plan = plan_import(&[cand(s1.clone(), "2024-05-01", 3)], &dest, &ImportOptions::default(), &PlanExtras::default());
        let report = execute_import(&plan, &dest, ImportMode::Move, || false, false, None, None);
        assert_eq!(report.ok.len(), 1, "{:?}", report.failed);
        assert!(dest.join("2024-05-01/a.jpg").is_file(), "目标应有文件");
        assert!(!s1.exists(), "移动后源应删除");
        assert_eq!(std::fs::read(dest.join("2024-05-01/a.jpg")).unwrap(), b"aaa");
    }

    #[test]
    fn test_execute_import_missing_source() {
        let dir = TempDir::new().unwrap();
        let missing = dir.path().join("src/ghost.jpg");
        let dest = dir.path().join("dest");
        let plan = plan_import(&[cand(missing.clone(), "2024-05-01", 3)], &dest, &ImportOptions::default(), &PlanExtras::default());
        // plan 阶段不查源存在性 → 计划通过；执行时报 SourceNotFound
        assert_eq!(plan.skipped.len(), 0);
        let report = execute_import(&plan, &dest, ImportMode::Copy, || false, false, None, None);
        assert_eq!(report.failed.len(), 1);
        assert!(report.failed[0].reason.contains("源文件不存在"), "{:?}", report.failed);
    }

    #[test]
    fn test_execute_import_target_exists_race() {
        let dir = TempDir::new().unwrap();
        let s1 = make_file(&dir, "src/a.jpg", b"aaa");
        let dest = dir.path().join("dest");
        let plan = plan_import(&[cand(s1.clone(), "2024-05-01", 3)], &dest, &ImportOptions::default(), &PlanExtras::default());
        // 计划后目标被并发写入 → 执行时 TargetExists 报错（不覆盖）
        make_file(&dir, "dest/2024-05-01/a.jpg", b"concurrent");
        let report = execute_import(&plan, &dest, ImportMode::Copy, || false, false, None, None);
        assert_eq!(report.failed.len(), 1);
        assert!(report.failed[0].reason.contains("目标已存在"), "{:?}", report.failed);
        // 目标内容未被覆盖
        assert_eq!(std::fs::read(dest.join("2024-05-01/a.jpg")).unwrap(), b"concurrent");
        assert!(s1.is_file());
    }
}
