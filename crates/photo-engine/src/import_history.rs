//! 导入账本（跨会话去重）与断点日志（中断续传）
//!
//! **账本为什么要存在**：原来那套去重只看「目标目录 + 同名 + 同大小」。只要目标布局
//! 或命名模板一变（`2026-09-10` → `2026/09/10`、加了 `{seq}` 前缀），同一张卡插第二次
//! 就会被整批再搬一遍。账本按 **源卷 + 源内相对路径 + 大小 + mtime** 记账，与目标怎么
//! 排布完全解耦——这才是「重复导入」的正解。
//!
//! 卷标识（`volume_id_for_path`）：Linux 取 `/dev/disk/by-uuid`（拿不到就退设备路径），
//! Windows 取卷序列号；其余平台返回 `None` → 账本整体退化为「按目标存在性去重」，
//! **不会误判**（宁可少一层保护，也不能凭猜测的卷号乱跳文件）。
//!
//! **断点日志**是 append-only JSONL：`<目标根>/.pt/import-journal.jsonl`，每搬完一个
//! 文件追加一行并 flush。下次对同一目标目录导入时，已完成的源文件在计划阶段直接跳过；
//! 全部成功后由调用方 `ImportJournal::discard` 丢弃，失败/取消时保留以便续传。

use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, params};
use thiserror::Error;

/// 账本 / 断点日志错误
#[derive(Error, Debug)]
pub enum ImportHistoryError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("账本数据库错误: {0}")]
    Db(#[from] rusqlite::Error),
}

/// 账本里的一行
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportRecord {
    /// 源卷标识（见 `volume_id_for_path`）
    pub volume_id: String,
    /// 源内相对路径（相对导入源根；卡换挂载点也不影响）
    pub rel_path: String,
    /// 文件大小（字节，识别文件被改动）
    pub size: u64,
    /// 源文件 mtime（纳秒，识别文件被改动）
    pub mtime_ns: i64,
    /// 当时落地的目标路径
    pub dest_path: String,
    /// 导入时间（Unix 秒）
    pub imported_at: i64,
}

/// 导入账本（SQLite；表只有一张，按 (volume_id, rel_path) 主键）
pub struct ImportHistory {
    conn: Connection,
}

impl ImportHistory {
    /// 打开（或创建）账本文件；父目录自动创建。
    pub fn open(path: &Path) -> Result<Self, ImportHistoryError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Self::init(Connection::open(path)?)
    }

    /// 内存账本（调用方自己管生命周期；主要给一次性批量脚本用）
    pub fn open_in_memory() -> Result<Self, ImportHistoryError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self, ImportHistoryError> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS import_history (
                 volume_id   TEXT    NOT NULL,
                 rel_path    TEXT    NOT NULL,
                 size        INTEGER NOT NULL,
                 mtime_ns    INTEGER NOT NULL,
                 dest_path   TEXT    NOT NULL,
                 imported_at INTEGER NOT NULL,
                 PRIMARY KEY (volume_id, rel_path)
             );",
        )?;
        Ok(Self { conn })
    }

    /// 命中时返回当时的目标路径。
    ///
    /// 大小或 mtime 对不上说明源文件被改过（重新导出/编辑后写回卡）→ 不算命中，
    /// 该文件会正常重导，不会因为路径相同就被静默跳过。
    pub fn lookup(
        &self,
        volume_id: &str,
        rel_path: &str,
        size: u64,
        mtime_ns: i64,
    ) -> Result<Option<String>, ImportHistoryError> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT dest_path FROM import_history
             WHERE volume_id = ?1 AND rel_path = ?2 AND size = ?3 AND mtime_ns = ?4",
        )?;
        let mut rows = stmt.query(params![volume_id, rel_path, size as i64, mtime_ns])?;
        match rows.next()? {
            Some(row) => Ok(Some(row.get(0)?)),
            None => Ok(None),
        }
    }

    /// 批量写入（一次导入收尾时调用；同一键覆盖旧值）
    pub fn record(&self, records: &[ImportRecord]) -> Result<(), ImportHistoryError> {
        if records.is_empty() {
            return Ok(());
        }
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO import_history
                     (volume_id, rel_path, size, mtime_ns, dest_path, imported_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(volume_id, rel_path) DO UPDATE SET
                     size = excluded.size,
                     mtime_ns = excluded.mtime_ns,
                     dest_path = excluded.dest_path,
                     imported_at = excluded.imported_at",
            )?;
            for r in records {
                stmt.execute(params![
                    r.volume_id,
                    r.rel_path,
                    r.size as i64,
                    r.mtime_ns,
                    r.dest_path,
                    r.imported_at
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// 清空某个卷的账本（「这张卡我从头再导一遍」）
    pub fn clear_volume(&self, volume_id: &str) -> Result<usize, ImportHistoryError> {
        Ok(self
            .conn
            .execute("DELETE FROM import_history WHERE volume_id = ?1", params![volume_id])?)
    }

    /// 账本总行数（设置页展示用）
    pub fn count(&self) -> Result<i64, ImportHistoryError> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM import_history", [], |r| r.get(0))?)
    }
}

/// 断点日志：一行一个「已搬运完成」的文件（append-only JSONL）
pub struct ImportJournal {
    path: PathBuf,
    file: Option<std::fs::File>,
    completed: HashSet<PathBuf>,
}

impl ImportJournal {
    /// 日志文件路径（`<目标根>/.pt/import-journal.jsonl`）
    pub fn path(dest_root: &Path) -> PathBuf {
        dest_root.join(".pt").join("import-journal.jsonl")
    }

    /// 打开（不存在 = 全新导入，completed 为空）
    pub fn open(dest_root: &Path) -> Result<Self, ImportHistoryError> {
        let path = Self::path(dest_root);
        let mut completed = HashSet::new();
        if let Ok(text) = std::fs::read_to_string(&path) {
            for line in text.lines() {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(line)
                    && let Some(source) = value.get("source").and_then(|v| v.as_str())
                {
                    completed.insert(PathBuf::from(source));
                }
            }
        }
        Ok(Self {
            path,
            file: None,
            completed,
        })
    }

    /// 日志里已完成的源文件（计划阶段据此跳过）
    pub fn completed(&self) -> &HashSet<PathBuf> {
        &self.completed
    }

    /// 已完成条数（界面提示「上次导入还剩 N 张」）
    pub fn completed_count(&self) -> usize {
        self.completed.len()
    }

    /// 追加一条已完成记录（首次写入时创建文件；逐条 flush，崩溃也只丢最后一条）
    pub fn record(
        &mut self,
        source: &Path,
        dest: &Path,
        size: u64,
    ) -> Result<(), ImportHistoryError> {
        if self.file.is_none() {
            if let Some(parent) = self.path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            self.file = Some(
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&self.path)?,
            );
        }
        let line = serde_json::json!({
            "source": source.to_string_lossy(),
            "dest": dest.to_string_lossy(),
            "size": size,
        })
        .to_string();
        if let Some(file) = self.file.as_mut() {
            file.write_all(line.as_bytes())?;
            file.write_all(b"\n")?;
            file.flush()?;
        }
        self.completed.insert(source.to_path_buf());
        Ok(())
    }

    /// 全部成功后丢弃日志（下次不再提示续传）；文件本来就不存在也算成功
    pub fn discard(dest_root: &Path) -> Result<(), ImportHistoryError> {
        let path = Self::path(dest_root);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err.into()),
        }
    }
}

/// 源卷标识：整个「重复导入」判定的根。
///
/// - Linux：/proc/mounts 里取**最长前缀**挂载点 → 设备 → /dev/disk/by-uuid 反查
///   （拿不到 UUID 就退设备路径，如 `/dev/sdb1`；tmpfs 之类无常驻设备则 None）
/// - Windows：卷序列号（GetVolumeInformationW）
/// - 其余平台：None（账本关闭，只按目标存在性去重）
pub fn volume_id_for_path(path: &Path) -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        return linux_volume_id(path);
    }
    #[cfg(windows)]
    {
        return win32_volume_id(path);
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        let _ = path;
        None
    }
}

/// 源卷的挂载根：账本的相对路径以**挂载点**为基准，而不是用户随手选的那个子目录。
///
/// 为什么重要：用户第一次可能只导 `<卡>/DCIM/100CANON`，第二次导整张卡。若相对路径以
/// 用户选的目录为基准，两次的 rel_path 不同 → 账本全部落空 → 同一批照片被再搬一遍。
/// 锚在挂载点则两种情况都命中同一行（也顺带让「弹出源盘」拿到真正能卸载的挂载点）。
pub fn volume_root_for_path(path: &Path) -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let mounts = std::fs::read_to_string("/proc/mounts").ok()?;
        let (_, mount) = longest_mount(&parse_mounts(&mounts), &canonical)?;
        return Some(PathBuf::from(mount));
    }
    #[cfg(windows)]
    {
        return win32::root_of(path).map(PathBuf::from);
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        let _ = path;
        None
    }
}

/// 挂载表解析（纯函数，跨平台可测）：返回 (设备, 挂载点)，挂载点内的 \040 还原为空格
#[cfg(any(target_os = "linux", test))]
fn parse_mounts(content: &str) -> Vec<(String, String)> {
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

/// 最长前缀挂载点查找（纯函数）：`/run/media/u/CARD/DCIM/x.jpg` → `/run/media/u/CARD`
///
/// 根挂载 `/` 单独处理：它的「剩余部分」不以 `/` 开头（`/home/x` 去掉长度 1 后是
/// `home/x`），按常规判据会永远匹配不上 → 根文件系统上的目录拿不到卷身份。
#[cfg(any(target_os = "linux", test))]
fn longest_mount(mounts: &[(String, String)], target: &Path) -> Option<(String, String)> {
    let target = target.to_string_lossy();
    mounts
        .iter()
        .filter(|(_, mount)| {
            if mount == "/" {
                return target.starts_with('/');
            }
            target == *mount
                || (target.starts_with(mount.as_str())
                    && target[mount.len()..].starts_with('/'))
        })
        .max_by_key(|(_, mount)| mount.len())
        .cloned()
}

/// Linux：挂载点 → 设备 → by-uuid 里的 uuid；查不到退设备路径
#[cfg(target_os = "linux")]
fn linux_volume_id(path: &Path) -> Option<String> {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mounts = std::fs::read_to_string("/proc/mounts").ok()?;
    let (device, _mount) = longest_mount(&parse_mounts(&mounts), &canonical)?;
    if !device.starts_with("/dev/") {
        return None; // tmpfs / overlay / 网络盘：没有稳定的卷身份
    }
    if let Some(uuid) = uuid_of_device(&device) {
        return Some(format!("uuid:{uuid}"));
    }
    Some(device)
}

/// /dev/disk/by-uuid/<uuid> → 解析到同一设备则命中
#[cfg(target_os = "linux")]
fn uuid_of_device(device: &str) -> Option<String> {
    let want = std::fs::canonicalize(device).ok()?;
    let dir = std::fs::read_dir("/dev/disk/by-uuid").ok()?;
    for entry in dir.flatten() {
        let link = entry.path();
        if std::fs::canonicalize(&link).ok().as_deref() == Some(want.as_path()) {
            return Some(entry.file_name().to_string_lossy().to_string());
        }
    }
    None
}

/// Windows：取路径所在盘的卷序列号
#[cfg(windows)]
mod win32 {
    use std::path::Path;

    #[link(name = "kernel32")]
    unsafe extern "system" {
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

    /// 路径 → 盘符根（"E:\\"）
    pub(super) fn root_of(path: &Path) -> Option<String> {
        let text = path.to_string_lossy();
        let mut chars = text.chars();
        let letter = chars.next()?;
        if !letter.is_ascii_alphabetic() || chars.next()? != ':' {
            return None;
        }
        Some(format!("{letter}:\\"))
    }

    /// 卷序列号 → "vol:XXXXXXXX"
    pub(super) fn volume_id(path: &Path) -> Option<String> {
        let root = root_of(path)?;
        let wide: Vec<u16> = root.encode_utf16().chain(std::iter::once(0)).collect();
        let mut serial: u32 = 0;
        let ok = unsafe {
            GetVolumeInformationW(
                wide.as_ptr(),
                std::ptr::null_mut(),
                0,
                &mut serial,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0,
            )
        };
        (ok != 0).then(|| format!("vol:{serial:08X}"))
    }
}

#[cfg(windows)]
fn win32_volume_id(path: &Path) -> Option<String> {
    win32::volume_id(path)
}
