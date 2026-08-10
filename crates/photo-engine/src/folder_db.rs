//! 照片文件夹级中心数据库（`.pt/data.db`），管理三类表：
//!
//! - **exif_cache**：缓存表，EXIF 元数据的 LRU 风格缓存。
//!   缓存表**可被清除**（清空释放空间）——丢失后只会触发重新提取。
//!
//! - **xmp_meta** / **recognition**：真相表，存储 XMP 元数据与鸟类识别结果。
//!   **任何清理缓存的操作都不得触碰 xmp_meta 与 recognition 表**——
//!   识别结果不可重新计算（需要 YOLO + 模型推理），XMP 元数据为用户手动编辑。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use parking_lot::Mutex;
use thiserror::Error;
use rusqlite_migration::{Migrations, M};

use photo_domain::{
    AdjustParams, BBox, BirdCandidate, ImageFormat, Recognition, RecognitionFailureStage,
    RecognitionStatus,
};
use photo_domain::ExifMetadata;
use photo_domain::XmpMetadata;

/// 缓存表 + 识别真相表迁移
fn folder_migrations() -> Migrations<'static> {
    Migrations::new(vec![
        M::up(
            "CREATE TABLE IF NOT EXISTS exif_cache (
                path        TEXT PRIMARY KEY,
                file_size   INTEGER NOT NULL,
                mtime_ns    INTEGER NOT NULL,
                make        TEXT,
                model       TEXT,
                lens        TEXT,
                exposure_time TEXT,
                f_number    TEXT,
                iso         INTEGER,
                focal_length TEXT,
                exposure_compensation TEXT,
                white_balance TEXT,
                date_time_original TEXT,
                image_width  INTEGER,
                image_height INTEGER,
                file_size_cache INTEGER,
                color_space  TEXT,
                orientation  INTEGER,
                gps_lat_deg  REAL,
                gps_lat_min  REAL,
                gps_lat_sec  REAL,
                gps_lon_deg  REAL,
                gps_lon_min  REAL,
                gps_lon_sec  REAL,
                gps_altitude REAL
            );",
        ),
        M::up(
            "CREATE TABLE IF NOT EXISTS xmp_meta (
                path        TEXT PRIMARY KEY,
                rating      INTEGER NOT NULL DEFAULT 0,
                color_label TEXT NOT NULL DEFAULT '',
                flag        TEXT NOT NULL DEFAULT ''
            );",
        ),
        M::up(
            "CREATE TABLE IF NOT EXISTS recognition (
                rel_path    TEXT PRIMARY KEY,
                status      TEXT NOT NULL,
                bird_id     INTEGER,
                bird_name   TEXT,
                class_index INTEGER,
                confidence  REAL,
                bbox        TEXT,
                candidates  TEXT,
                failure_stage TEXT,
                recognized_at TEXT NOT NULL
            );",
        ),
        M::up(
            "ALTER TABLE recognition ADD COLUMN eye_sharpness REAL;
             ALTER TABLE recognition ADD COLUMN eye_bbox TEXT;",
        ),
        M::up(
            "CREATE TABLE IF NOT EXISTS adjustments (
                rel_path    TEXT PRIMARY KEY,
                exposure    REAL NOT NULL DEFAULT 0,
                contrast    INTEGER NOT NULL DEFAULT 0,
                saturation  INTEGER NOT NULL DEFAULT 0,
                crop        TEXT
            );",
        ),
    ])
}

#[derive(Error, Debug)]
pub enum FolderDbError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Migration error: {0}")]
    Migration(#[from] rusqlite_migration::Error),
    #[error("Serde error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("数据库值超出可表示范围: {0}")]
    ValueOutOfRange(&'static str),
}

#[derive(Clone)]
pub struct FolderDb {
    conn: Arc<Mutex<rusqlite::Connection>>,
}

impl FolderDb {
    /// 在指定目录中打开或创建 `.pt/data.db`，自动迁移表结构。
    /// 自动处理从旧版 `.pt-cache.db` 的迁移。
    pub fn open_in_dir(dir: &Path) -> Result<Self, FolderDbError> {
        let pt_dir = dir.join(".pt");
        std::fs::create_dir_all(&pt_dir)?;
        let db_path = pt_dir.join("data.db");

        // 检测遗留 .pt-cache.db，迁移到新位置
        let legacy_path = dir.join(".pt-cache.db");
        if legacy_path.exists() && !db_path.exists() {
            // 遗留文件存在且新库不存在：执行迁移
            for legacy_ext in &["", "-wal", "-shm"] {
                let legacy_file = dir.join(format!(".pt-cache.db{}", legacy_ext));
                if legacy_file.exists() {
                    let new_file = pt_dir.join(format!("data.db{}", legacy_ext));
                    std::fs::rename(&legacy_file, &new_file)?;
                }
            }
        }

        // 迁移完成后删除遗留文件（可能因为 -wal/-shm 残留）
        for legacy_ext in &["", "-wal", "-shm"] {
            let legacy_file = dir.join(format!(".pt-cache.db{}", legacy_ext));
            if legacy_file.exists() {
                std::fs::remove_file(&legacy_file).ok();
            }
        }

        let mut conn = rusqlite::Connection::open(&db_path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
        folder_migrations().to_latest(&mut conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    // ── EXIF 缓存 ──

    /// 查询缓存。返回 `None` 表示未命中或已失效。
    pub fn get_exif(&self, path: &Path) -> Result<Option<ExifMetadata>, FolderDbError> {
        let path_str = path.to_string_lossy();
        let (size, mtime) = match file_fingerprint(path) {
            Ok(f) => f,
            Err(_) => return Ok(None),
        };

        let conn = self.conn.lock();
        let mut stmt = conn.prepare_cached(
            "SELECT make, model, lens, exposure_time, f_number, iso, focal_length,
                    exposure_compensation, white_balance, date_time_original,
                    image_width, image_height, file_size_cache, color_space, orientation,
                    gps_lat_deg, gps_lat_min, gps_lat_sec,
                    gps_lon_deg, gps_lon_min, gps_lon_sec, gps_altitude
             FROM exif_cache
             WHERE path = ?1 AND file_size = ?2 AND mtime_ns = ?3",
        )?;

        match stmt.query_row(
            rusqlite::params![path_str.as_ref(), size_to_i64(size), mtime as i64],
            |row| row_to_exif(row),
        ) {
            Ok(exif) => Ok(Some(exif)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// 写入缓存。如果路径已存在则更新（UPSERT）。
    pub fn put_exif(&self, path: &Path, exif: &ExifMetadata) -> Result<(), FolderDbError> {
        let path_str = path.to_string_lossy();
        let (size, mtime) = match file_fingerprint(path) {
            Ok(f) => f,
            Err(_) => return Ok(()),
        };

        let conn = self.conn.lock();
        let params = exif_to_params(path_str.as_ref(), size, mtime, exif);
        conn.execute(
            "INSERT OR REPLACE INTO exif_cache
             (path, file_size, mtime_ns, make, model, lens, exposure_time, f_number, iso,
              focal_length, exposure_compensation, white_balance, date_time_original,
              image_width, image_height, file_size_cache, color_space, orientation,
              gps_lat_deg, gps_lat_min, gps_lat_sec,
              gps_lon_deg, gps_lon_min, gps_lon_sec, gps_altitude)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25)",
            rusqlite::params_from_iter(params.iter().map(|b| b.as_ref())),
        )?;
        Ok(())
    }

    /// 获取或提取：缓存命中直接返回；未命中则提取、写入缓存、返回。
    pub fn get_or_extract_exif(
        &self,
        path: &Path,
        format: &ImageFormat,
    ) -> Result<ExifMetadata, FolderDbError> {
        if let Some(cached) = self.get_exif(path)? {
            return Ok(cached);
        }
        let exif = crate::exif::extract_exif(path, format)
            .unwrap_or_default();
        let _ = self.put_exif(path, &exif);
        Ok(exif)
    }

    /// 全量读取 exif_cache（扫描闭包一次性载入内存，替代逐文件 1 查询 + 1 stat）。
    /// 键为写入时的完整路径字符串（与 get_exif 一致，Windows 下为反斜杠）。
    /// 指纹校验由调用方用自己已 stat 的 (file_size, mtime_ns) 完成，本方法不做磁盘 I/O。
    pub fn all_exif(&self) -> Result<HashMap<String, ExifCacheRow>, FolderDbError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare_cached(
            "SELECT make, model, lens, exposure_time, f_number, iso, focal_length,
                    exposure_compensation, white_balance, date_time_original,
                    image_width, image_height, file_size_cache, color_space, orientation,
                    gps_lat_deg, gps_lat_min, gps_lat_sec,
                    gps_lon_deg, gps_lon_min, gps_lon_sec, gps_altitude,
                    path, file_size, mtime_ns
             FROM exif_cache",
        )?;
        let rows = stmt.query_map([], |row| {
            // 列 0-21 与 row_to_exif 对齐；22-24 追加 path/指纹
            let exif = row_to_exif(row)?;
            Ok((
                row.get::<_, String>(22)?,
                ExifCacheRow {
                    file_size: row.get(23)?,
                    mtime_ns: row.get(24)?,
                    exif,
                },
            ))
        })?;
        let mut map = HashMap::new();
        for row in rows {
            let (path, cache_row) = row?;
            map.insert(path, cache_row);
        }
        Ok(map)
    }

    /// 获取 XMP 缓存。返回 `None` 表示未缓存。
    pub fn get_xmp(&self, path: &Path) -> Result<Option<XmpMetadata>, FolderDbError> {
        let path_str = path.to_string_lossy();
        let conn = self.conn.lock();
        match conn.query_row(
            "SELECT rating, color_label, flag FROM xmp_meta WHERE path = ?1",
            rusqlite::params![path_str.as_ref()],
            |row| {
                Ok(XmpMetadata {
                    rating: row.get::<_, i32>(0)? as u8,
                    color_label: row.get(1)?,
                    flag: row.get(2)?,
                })
            },
        ) {
            Ok(m) => Ok(Some(m)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// 写入 XMP 缓存。
    pub fn put_xmp(&self, path: &Path, meta: &XmpMetadata) -> Result<(), FolderDbError> {
        let path_str = path.to_string_lossy();
        let conn = self.conn.lock();
        conn.execute(
            "INSERT OR REPLACE INTO xmp_meta (path, rating, color_label, flag) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![path_str.as_ref(), meta.rating as i32, meta.color_label, meta.flag],
        )?;
        Ok(())
    }

    /// 全量读取 xmp_meta 真相表（扫描闭包一次性载入内存，替代逐文件点查询）。
    /// 键为写入时的完整路径字符串（与 get_xmp 一致），无指纹校验（与 get_xmp 同语义）。
    pub fn all_xmp_meta(&self) -> Result<HashMap<String, XmpMetadata>, FolderDbError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare_cached(
            "SELECT path, rating, color_label, flag FROM xmp_meta",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                XmpMetadata {
                    rating: row.get::<_, i32>(1)? as u8,
                    color_label: row.get(2)?,
                    flag: row.get(3)?,
                },
            ))
        })?;
        let mut map = HashMap::new();
        for row in rows {
            let (path, meta) = row?;
            map.insert(path, meta);
        }
        Ok(map)
    }

    // ── 识别真相表 ──

    /// UPSERT 一条识别结果。rel_path 使用正斜杠归一化。
    pub fn upsert_recognition(&self, rel_path: &str, rec: &Recognition) -> Result<(), FolderDbError> {
        let conn = self.conn.lock();
        let normalized = rel_path.replace('\\', "/");
        let bbox_str = rec.bbox.as_ref().map(|b| b.to_db_string());
        let candidates_str = serde_json::to_string(&rec.candidates)?;
        let eye_bbox_str = rec.eye_bbox.as_ref().map(|b| b.to_db_string());
        conn.execute(
            "INSERT OR REPLACE INTO recognition
             (rel_path, status, bird_id, bird_name, class_index, confidence,
              bbox, candidates, failure_stage, recognized_at,
              eye_sharpness, eye_bbox)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                normalized,
                rec.status.as_str(),
                rec.bird.as_ref().map(|b| b.bird_id),
                rec.bird.as_ref().map(|b| b.cn_name.as_str()),
                rec.class_index.map(|v| v as i64),
                rec.confidence,
                bbox_str,
                candidates_str,
                rec.failure_stage.as_str(),
                rec.recognized_at,
                rec.eye_sharpness,
                eye_bbox_str,
            ],
        )?;
        Ok(())
    }

    /// 查询单条识别结果。
    pub fn get_recognition(&self, rel_path: &str) -> Result<Option<Recognition>, FolderDbError> {
        let conn = self.conn.lock();
        let normalized = rel_path.replace('\\', "/");
        let mut stmt = conn.prepare_cached(
            "SELECT rel_path, status, bird_id, bird_name, class_index, confidence,
                    bbox, candidates, failure_stage, recognized_at,
                    eye_sharpness, eye_bbox
             FROM recognition WHERE rel_path = ?1",
        )?;
        match stmt.query_row(rusqlite::params![normalized], |row| row_to_recognition(row)) {
            Ok(rec) => Ok(Some(rec?)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// 查询所有识别结果（用于网格全量填充）。
    pub fn all_recognitions(&self) -> Result<Vec<(String, Recognition)>, FolderDbError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare_cached(
            "SELECT rel_path, status, bird_id, bird_name, class_index, confidence,
                    bbox, candidates, failure_stage, recognized_at,
                    eye_sharpness, eye_bbox
             FROM recognition",
        )?;
        let rows = stmt.query_map([], |row| {
            let rel_path: String = row.get(0)?;
            Ok((rel_path, row_to_recognition(row)?))
        })?;
        let mut results = Vec::new();
        for row in rows {
            let (rel_path, rec) = row?;
            results.push((rel_path, rec?));
        }
        Ok(results)
    }

    /// 批量删除识别行。
    pub fn delete_recognitions(&self, rel_paths: &[String]) -> Result<(), FolderDbError> {
        let conn = self.conn.lock();
        for rp in rel_paths {
            let normalized = rp.replace('\\', "/");
            conn.execute("DELETE FROM recognition WHERE rel_path = ?1", rusqlite::params![normalized])?;
        }
        Ok(())
    }

    /// 重命名识别行的键（文件重命名后同步）。
    pub fn rename_recognition(&self, old_rel: &str, new_rel: &str) -> Result<(), FolderDbError> {
        let conn = self.conn.lock();
        let old_norm = old_rel.replace('\\', "/");
        let new_norm = new_rel.replace('\\', "/");
        conn.execute(
            "UPDATE recognition SET rel_path = ?1 WHERE rel_path = ?2",
            rusqlite::params![new_norm, old_norm],
        )?;
        Ok(())
    }

    /// 将一批识别行复制到目标库（跨文件夹移动/复制用）。
    /// entries: (源 rel_path, 目标 rel_path)
    pub fn copy_recognitions_to(
        &self,
        target_db: &mut FolderDb,
        entries: &[(String, String)],
    ) -> Result<(), FolderDbError> {
        // 先批量读取本库数据：逐条查询并成对收集（目标 rel_path, Recognition），
        // 避免按 batch 序号索引 entries 在存在无行条目时错位写入错误目标
        let mut pairs: Vec<(String, Recognition)> = Vec::new();
        {
            let conn = self.conn.lock();
            let mut stmt = conn.prepare_cached(
                "SELECT rel_path, status, bird_id, bird_name, class_index, confidence,
                        bbox, candidates, failure_stage, recognized_at,
                        eye_sharpness, eye_bbox
                 FROM recognition WHERE rel_path = ?1",
            )?;
            for (src_rel, dst_rel) in entries {
                let normalized = src_rel.replace('\\', "/");
                match stmt.query_row(rusqlite::params![normalized], |row| row_to_recognition(row)) {
                    Ok(rec) => pairs.push((dst_rel.clone(), rec?)),
                    Err(rusqlite::Error::QueryReturnedNoRows) => {}
                    Err(e) => return Err(e.into()),
                }
            }
        }
        // 按各自目标键 UPSERT 到目标库
        for (dst_rel, rec) in pairs {
            target_db.upsert_recognition(&dst_rel, &rec)?;
        }
        Ok(())
    }
    // ── adjustments 真相表（参数化调整，ADR 0007：随文件走，不可当缓存清除）──

    /// UPSERT 一条调整参数。全零参数仍写入（显式记录“已复位”），读取侧与无行同义。
    pub fn put_adjustments(&self, rel_path: &str, params: &AdjustParams) -> Result<(), FolderDbError> {
        let conn = self.conn.lock();
        let normalized = rel_path.replace('\\', "/");
        conn.execute(
            "INSERT OR REPLACE INTO adjustments (rel_path, exposure, contrast, saturation)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                normalized,
                params.exposure as f64,
                params.contrast as i64,
                params.saturation as i64,
            ],
        )?;
        Ok(())
    }

    /// 查询单条调整参数（无行 = None = 无调整）。
    pub fn get_adjustments(&self, rel_path: &str) -> Result<Option<AdjustParams>, FolderDbError> {
        let conn = self.conn.lock();
        let normalized = rel_path.replace('\\', "/");
        let mut stmt = conn.prepare_cached(
            "SELECT exposure, contrast, saturation FROM adjustments WHERE rel_path = ?1",
        )?;
        // i64 → i32 用 try_from：越界（数据损坏）报错而非静默截断
        match stmt.query_row(rusqlite::params![normalized], |row| {
            Ok((
                row.get::<_, f64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        }) {
            Ok((exposure, contrast, saturation)) => {
                Ok(Some(adjust_params_from_parts(exposure, contrast, saturation)?))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// 批量删除调整行（文件删除后同步）。
    pub fn delete_adjustments(&self, rel_paths: &[String]) -> Result<(), FolderDbError> {
        let conn = self.conn.lock();
        for rp in rel_paths {
            let normalized = rp.replace('\\', "/");
            conn.execute(
                "DELETE FROM adjustments WHERE rel_path = ?1",
                rusqlite::params![normalized],
            )?;
        }
        Ok(())
    }

    /// 重命名调整行的键（文件重命名后同步）。
    pub fn rename_adjustment(&self, old_rel: &str, new_rel: &str) -> Result<(), FolderDbError> {
        let conn = self.conn.lock();
        let old_norm = old_rel.replace('\\', "/");
        let new_norm = new_rel.replace('\\', "/");
        conn.execute(
            "UPDATE adjustments SET rel_path = ?1 WHERE rel_path = ?2",
            rusqlite::params![new_norm, old_norm],
        )?;
        Ok(())
    }

    /// 将一批调整行复制到目标库（跨文件夹移动/复制用）。
    /// entries: (源 rel_path, 目标 rel_path)
    pub fn copy_adjustments_to(
        &self,
        target_db: &mut FolderDb,
        entries: &[(String, String)],
    ) -> Result<(), FolderDbError> {
        // 与 copy_recognitions_to 同构：逐条按源键查询，成对收集避免索引错位
        let mut pairs: Vec<(String, AdjustParams)> = Vec::new();
        {
            let conn = self.conn.lock();
            let mut stmt = conn.prepare_cached(
                "SELECT exposure, contrast, saturation FROM adjustments WHERE rel_path = ?1",
            )?;
            for (src_rel, dst_rel) in entries {
                let normalized = src_rel.replace('\\', "/");
                match stmt.query_row(rusqlite::params![normalized], |row| {
                    Ok((
                        row.get::<_, f64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                }) {
                    Ok((exposure, contrast, saturation)) => {
                        let params = adjust_params_from_parts(exposure, contrast, saturation)?;
                        pairs.push((dst_rel.clone(), params));
                    }
                    Err(rusqlite::Error::QueryReturnedNoRows) => {}
                    Err(e) => return Err(e.into()),
                }
            }
        }
        for (dst_rel, params) in pairs {
            target_db.put_adjustments(&dst_rel, &params)?;
        }
        Ok(())
    }

    // ── xmp_meta 真相表（评分/色标/旗标持久化，不可当缓存清除）──

    /// 批量删除评分/色标/旗标行。
    pub fn delete_xmp_rows(&self, paths: &[String]) -> Result<(), FolderDbError> {
        let conn = self.conn.lock();
        for p in paths {
            conn.execute("DELETE FROM xmp_meta WHERE path = ?1", rusqlite::params![p])?;
        }
        Ok(())
    }

    /// 重命名 xmp_meta 行的键（文件移动/重命名后同步）。
    pub fn rename_xmp(&self, old_path: &str, new_path: &str) -> Result<(), FolderDbError> {
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE xmp_meta SET path = ?1 WHERE path = ?2",
            rusqlite::params![new_path, old_path],
        )?;
        Ok(())
    }

    /// 将一批 xmp_meta 行复制到目标库（跨文件夹移动/复制用）。
    /// entries: (源路径, 目标路径)
    pub fn copy_xmp_rows_to(
        &self,
        target_db: &mut FolderDb,
        entries: &[(String, String)],
    ) -> Result<(), FolderDbError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare_cached(
            "SELECT rating, color_label, flag FROM xmp_meta WHERE path = ?1",
        )?;
        for (src, dst) in entries {
            match stmt.query_row(rusqlite::params![src], |row| {
                Ok(XmpMetadata {
                    rating: row.get::<_, i32>(0)? as u8,
                    color_label: row.get(1)?,
                    flag: row.get(2)?,
                })
            }) {
                Ok(meta) => target_db.put_xmp(std::path::Path::new(dst), &meta)?,
                Err(rusqlite::Error::QueryReturnedNoRows) => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }
}
/// exif_cache 单行：指纹 + 解析后的 EXIF（供扫描批量载入后内存校验/查表）。
pub struct ExifCacheRow {
    /// 缓存时记录的指纹（SQLite INTEGER 存 i64，与 get_exif 的 file_fingerprint 同源）
    pub file_size: i64,
    pub mtime_ns: i64,
    pub exif: ExifMetadata,
}

/// 文件条目信息（由 app 层扫描产生，传给 sync_with_scan 做三表同步）。
pub struct FileEntry {
    pub full_path: PathBuf,
    /// 相对目录的正斜杠路径（用于 recognition 表键）
    pub rel_path: String,
    pub file_size: u64,
    pub mtime_ns: i64,
    pub format: ImageFormat,
}

/// 同步操作统计。
#[derive(Debug, Clone, Default)]
pub struct SyncStats {
    pub cache_deleted: usize,
    pub cache_inserted: usize,
    // cache_updated / cache_failed 为保留字段：EXIF 提取已移至 app 层
    // spawn_enrich_tasks（sync_with_scan 只做表对齐），sync 不再产生，恒为 0
    pub cache_updated: usize,
    pub cache_failed: usize,
    pub recognition_deleted: usize,
    pub adjustments_deleted: usize,
}

impl FolderDb {
    /// 以扫描结果同步三表（exif_cache / xmp_meta / recognition）：
    ///
    /// - **exif_cache**：删除文件已不存在的行。新增/指纹变化的文件**不在此提取 EXIF**，
    ///   统一由 app 层 `spawn_enrich_tasks` 并发提取回填（避免与 enrich 对同一新文件
    ///   重复 LibRaw open/unpack；RAW 100-300ms/次，串行跑会拖出数十秒的尾部）。
    /// - **xmp_meta** / **recognition**：仅删除文件已不存在的行，**不重新识别/不重新读取**。
    pub fn sync_with_scan(
        &self,
        entries: &[FileEntry],
        on_progress: &dyn Fn(usize, usize),
    ) -> Result<SyncStats, FolderDbError> {
        let entry_paths: std::collections::HashSet<String> = entries
            .iter()
            .map(|e| e.full_path.to_string_lossy().to_string())
            .collect();
        let entry_rel_paths: std::collections::HashSet<String> =
            entries.iter().map(|e| e.rel_path.clone()).collect();

        let mut stats = SyncStats::default();
        let conn = self.conn.lock();

        // ── 1. 三表删除多余行 ──
        {
            let mut stmt = conn.prepare_cached("SELECT path FROM exif_cache")?;
            let db_paths: Vec<String> = stmt.query_map([], |row| row.get(0))?
                .filter_map(|r| r.ok()).collect();
            let total = db_paths.len();
            for (i, p) in db_paths.iter().enumerate() {
                if !entry_paths.contains(p.as_str()) {
                    conn.execute("DELETE FROM exif_cache WHERE path = ?1", rusqlite::params![p])?;
                    stats.cache_deleted += 1;
                }
                on_progress(i + 1, total);
            }
        }
        {
            let mut stmt = conn.prepare_cached("SELECT path FROM xmp_meta")?;
            let db_paths: Vec<String> = stmt.query_map([], |row| row.get(0))?
                .filter_map(|r| r.ok()).collect();
            for p in &db_paths {
                if !entry_paths.contains(p.as_str()) {
                    conn.execute("DELETE FROM xmp_meta WHERE path = ?1", rusqlite::params![p])?;
                }
            }
        }
        {
            let mut stmt = conn.prepare_cached("SELECT rel_path FROM recognition")?;
            let db_rel_paths: Vec<String> = stmt.query_map([], |row| row.get(0))?
                .filter_map(|r| r.ok()).collect();
            for rp in &db_rel_paths {
                if !entry_rel_paths.contains(rp.as_str()) {
                    conn.execute("DELETE FROM recognition WHERE rel_path = ?1", rusqlite::params![rp])?;
                    stats.recognition_deleted += 1;
                }
            }
        }
        {
            let mut stmt = conn.prepare_cached("SELECT rel_path FROM adjustments")?;
            let db_rel_paths: Vec<String> = stmt.query_map([], |row| row.get(0))?
                .filter_map(|r| r.ok()).collect();
            for rp in &db_rel_paths {
                if !entry_rel_paths.contains(rp.as_str()) {
                    conn.execute("DELETE FROM adjustments WHERE rel_path = ?1", rusqlite::params![rp])?;
                    stats.adjustments_deleted += 1;
                }
            }
        }

        // ── 2. 新增/指纹变化 → 不在此提取 EXIF ──
        // 提取统一由 app 层 spawn_enrich_tasks 并发完成（get_or_extract_exif 原子地
        // 查缓存→提取→写回）。本方法只做表对齐：不能插"只有指纹的占位行"，否则
        // get_exif 会把它当命中、enrich 将不再提取；指纹不匹配/缺失的旧行保留，
        // get_exif 按指纹拒绝，由 enrich 的 put_exif 覆盖为最新数据。
        // 循环仅保留以驱动 on_progress（状态栏 done/total 计数）。
        for (i, _entry) in entries.iter().enumerate() {
            on_progress(i + 1, entries.len());
        }
        Ok(stats)
    }
}

/// u64 → i64 饱和转换（SQLite INTEGER 为有符号 64 位）。文件大小超过
/// i64::MAX（约 9.2 EB，现实中不可能）时饱和到 i64::MAX，杜绝静默回绕成负数。
fn size_to_i64(size: u64) -> i64 {
    i64::try_from(size).unwrap_or(i64::MAX)
}

/// adjustments 行 → AdjustParams。contrast/saturation 由 i64 收窄到 i32，
/// 越界（数据损坏）返回 ValueOutOfRange 而非静默截断。
fn adjust_params_from_parts(
    exposure: f64,
    contrast: i64,
    saturation: i64,
) -> Result<AdjustParams, FolderDbError> {
    Ok(AdjustParams {
        exposure: exposure as f32,
        contrast: i32::try_from(contrast)
            .map_err(|_| FolderDbError::ValueOutOfRange("contrast"))?,
        saturation: i32::try_from(saturation)
            .map_err(|_| FolderDbError::ValueOutOfRange("saturation"))?,
    })
}

fn file_fingerprint(path: &Path) -> std::io::Result<(u64, u64)> {
    let meta = std::fs::metadata(path)?;
    let size = meta.len();
    let mtime = meta
        .modified()
        .map(|t| {
            t.duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64
        })
        .unwrap_or(0);
    Ok((size, mtime))
}

fn row_to_exif(row: &rusqlite::Row) -> rusqlite::Result<ExifMetadata> {
    use photo_domain::{CameraInfo, GpsInfo, ShootingParams};

    let gps_lat = match (
        row.get::<_, Option<f64>>(15)?,
        row.get::<_, Option<f64>>(16)?,
        row.get::<_, Option<f64>>(17)?,
    ) {
        (Some(d), Some(m), Some(s)) => Some((d, m, s)),
        _ => None,
    };
    let gps_lon = match (
        row.get::<_, Option<f64>>(18)?,
        row.get::<_, Option<f64>>(19)?,
        row.get::<_, Option<f64>>(20)?,
    ) {
        (Some(d), Some(m), Some(s)) => Some((d, m, s)),
        _ => None,
    };

    Ok(ExifMetadata {
        camera: CameraInfo {
            make: row.get(0)?,
            model: row.get(1)?,
            lens: row.get(2)?,
        },
        shooting: ShootingParams {
            exposure_time: row.get(3)?,
            f_number: row.get(4)?,
            iso: row.get(5)?,
            focal_length: row.get(6)?,
            exposure_compensation: row.get(7)?,
            white_balance: row.get(8)?,
        },
        date_time_original: row.get(9)?,
        image_width: row.get(10)?,
        image_height: row.get(11)?,
        file_size: row
            .get::<_, Option<i64>>(12)?
            .map(|v| u64::try_from(v).unwrap_or(0)),
        color_space: row.get(13)?,
        orientation: row.get(14)?,
        gps: GpsInfo {
            latitude: gps_lat,
            longitude: gps_lon,
            altitude: row.get(21)?,
        },
    })
}

fn exif_to_params<'a>(
    path: &'a str,
    size: u64,
    mtime: u64,
    exif: &'a ExifMetadata,
) -> Vec<Box<dyn rusqlite::types::ToSql + 'a>> {
    let (lat_deg, lat_min, lat_sec) = exif.gps.latitude.map(|(d, m, s)| (Some(d), Some(m), Some(s))).unwrap_or((None, None, None));
    let (lon_deg, lon_min, lon_sec) = exif.gps.longitude.map(|(d, m, s)| (Some(d), Some(m), Some(s))).unwrap_or((None, None, None));

    vec![
        Box::new(path.to_string()),
        Box::new(size_to_i64(size)),
        Box::new(mtime as i64),
        Box::new(exif.camera.make.clone()),
        Box::new(exif.camera.model.clone()),
        Box::new(exif.camera.lens.clone()),
        Box::new(exif.shooting.exposure_time.clone()),
        Box::new(exif.shooting.f_number.clone()),
        Box::new(exif.shooting.iso.map(|v| v as i64)),
        Box::new(exif.shooting.focal_length.clone()),
        Box::new(exif.shooting.exposure_compensation.clone()),
        Box::new(exif.shooting.white_balance.clone()),
        Box::new(exif.date_time_original.clone()),
        Box::new(exif.image_width.map(|v| v as i64)),
        Box::new(exif.image_height.map(|v| v as i64)),
        Box::new(exif.file_size.map(|v| v as i64)),
        Box::new(exif.color_space.clone()),
        Box::new(exif.orientation.map(|v| v as i64)),
        Box::new(lat_deg),
        Box::new(lat_min),
        Box::new(lat_sec),
        Box::new(lon_deg),
        Box::new(lon_min),
        Box::new(lon_sec),
        Box::new(exif.gps.altitude),
    ]
}

fn row_to_recognition(row: &rusqlite::Row) -> rusqlite::Result<Result<Recognition, serde_json::Error>> {
    let status_str: String = row.get(1)?;
    let status = RecognitionStatus::from_str(&status_str).unwrap_or(RecognitionStatus::Unrecognized);
    let failure_stage_str: String = row.get(8)?;
    let failure_stage = RecognitionFailureStage::from_str(&failure_stage_str).unwrap_or(RecognitionFailureStage::None);
    let bird_id: Option<i64> = row.get(2)?;
    let bird_name: Option<String> = row.get(3)?;
    let class_index: Option<i64> = row.get(4)?;
    let confidence: Option<f64> = row.get(5)?;
    let bbox_str: Option<String> = row.get(6)?;
    let candidates_str: Option<String> = row.get(7)?;
    let recognized_at: String = row.get(9)?;
    let eye_sharpness: Option<f32> = row.get(10)?;
    let eye_bbox_str: Option<String> = row.get(11)?;

    let bird = match (bird_id, bird_name) {
        (Some(id), Some(cn)) => {
            Some(photo_domain::BirdMatch {
                bird_id: id,
                cn_name: cn,
                latin_name: String::new(),
            })
        }
        _ => None,
    };

    let candidates: Vec<BirdCandidate> = match candidates_str {
        Some(s) => serde_json::from_str(&s).unwrap_or_default(),
        None => Vec::new(),
    };

    Ok(Ok(Recognition {
        status,
        bird,
        class_index: class_index.map(|v| v as u32),
        confidence: confidence.map(|v| v as f32),
        bbox: bbox_str.and_then(|s| BBox::parse(&s)),
        candidates,
        failure_stage,
        recognized_at,
        eye_sharpness,
        eye_bbox: eye_bbox_str.and_then(|s| BBox::parse(&s)),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use photo_domain::{BirdMatch, BirdCandidate, CameraInfo, GpsInfo, ShootingParams};
    use tempfile::TempDir;

    fn make_exif() -> ExifMetadata {
        ExifMetadata {
            camera: CameraInfo {
                make: Some("Nikon".into()),
                model: Some("D850".into()),
                lens: Some("24-70mm f/2.8".into()),
            },
            shooting: ShootingParams {
                exposure_time: Some("1/250".into()),
                f_number: Some("f/5.6".into()),
                iso: Some(200),
                focal_length: Some("50mm".into()),
                exposure_compensation: None,
                white_balance: None,
            },
            date_time_original: Some("2024-01-15 10:30:00".into()),
            image_width: Some(8256),
            image_height: Some(5504),
            file_size: Some(42_000_000),
            color_space: Some("sRGB".into()),
            orientation: Some(1),
            gps: GpsInfo {
                latitude: Some((39.0, 54.0, 26.0)),
                longitude: Some((116.0, 23.0, 29.0)),
                altitude: Some(50.5),
            },
        }
    }

    fn make_adjustments() -> AdjustParams {
        AdjustParams {
            exposure: 1.25,
            contrast: -30,
            saturation: 45,
        }
    }

    fn make_recognition() -> Recognition {
        Recognition {
            status: RecognitionStatus::NeedsReview,
            bird: Some(BirdMatch {
                bird_id: 42,
                cn_name: "大斑啄木鸟".into(),
                latin_name: "Dendrocopos major".into(),
            }),
            class_index: Some(123),
            confidence: Some(95.5),
            bbox: Some(BBox::new(0.1, 0.2, 0.8, 0.9)),
            candidates: vec![
                BirdCandidate {
                    class_index: 123,
                    confidence: 95.5,
                    bird: Some(BirdMatch {
                        bird_id: 42,
                        cn_name: "大斑啄木鸟".into(),
                        latin_name: "Dendrocopos major".into(),
                    }),
                },
            ],
            failure_stage: RecognitionFailureStage::None,
            recognized_at: "2026-07-28T10:00:00Z".into(),
            eye_sharpness: Some(42.5),
            eye_bbox: Some(BBox::new(0.3, 0.4, 0.5, 0.6)),
        }
    }

    #[test]
    fn test_cache_put_get_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let jpg = tmp.path().join("test.jpg");
        std::fs::write(&jpg, b"fake jpeg data").unwrap();
        let db = FolderDb::open_in_dir(tmp.path()).unwrap();
        let exif = make_exif();
        assert!(db.get_exif(&jpg).unwrap().is_none());
        db.put_exif(&jpg, &exif).unwrap();
        let got = db.get_exif(&jpg).unwrap().expect("should hit");
        assert_eq!(got.camera.make.as_deref(), Some("Nikon"));
        assert_eq!(got.shooting.iso, Some(200));
        assert_eq!(got.gps.latitude, Some((39.0, 54.0, 26.0)));
    }

    #[test]
    fn test_cache_stale_on_size_change() {
        let tmp = TempDir::new().unwrap();
        let jpg = tmp.path().join("test.jpg");
        std::fs::write(&jpg, b"original").unwrap();
        let db = FolderDb::open_in_dir(tmp.path()).unwrap();
        db.put_exif(&jpg, &make_exif()).unwrap();
        std::fs::write(&jpg, b"modified data here").unwrap();
        assert!(db.get_exif(&jpg).unwrap().is_none());
    }

    #[test]
    fn test_cache_nonexistent_file() {
        let tmp = TempDir::new().unwrap();
        let db = FolderDb::open_in_dir(tmp.path()).unwrap();
        assert!(db.get_exif(Path::new("/nonexistent/file.orf")).unwrap().is_none());
    }

    #[test]
    fn test_cache_upsert() {
        let tmp = TempDir::new().unwrap();
        let jpg = tmp.path().join("test.jpg");
        std::fs::write(&jpg, b"fake jpeg").unwrap();
        let db = FolderDb::open_in_dir(tmp.path()).unwrap();
        let mut exif1 = make_exif();
        exif1.shooting.iso = Some(100);
        db.put_exif(&jpg, &exif1).unwrap();
        let mut exif2 = make_exif();
        exif2.shooting.iso = Some(400);
        db.put_exif(&jpg, &exif2).unwrap();
        let got = db.get_exif(&jpg).unwrap().unwrap();
        assert_eq!(got.shooting.iso, Some(400));
    }

    #[test]
    fn test_recognition_table_exists_and_rw() {
        let tmp = TempDir::new().unwrap();
        let db = FolderDb::open_in_dir(tmp.path()).unwrap();
        let rec = make_recognition();
        db.upsert_recognition("photos/bird.jpg", &rec).unwrap();
        let got = db.get_recognition("photos/bird.jpg").unwrap().expect("should exist");
        assert_eq!(got.status, RecognitionStatus::NeedsReview);
        assert_eq!(got.bird.as_ref().map(|b| b.cn_name.as_str()), Some("大斑啄木鸟"));
        assert_eq!(got.confidence, Some(95.5));
        assert!(got.bbox.is_some());
    }

    #[test]
    fn test_recognition_upsert_get_delete() {
        let tmp = TempDir::new().unwrap();
        let db = FolderDb::open_in_dir(tmp.path()).unwrap();
        let rec = make_recognition();
        db.upsert_recognition("a.jpg", &rec).unwrap();
        assert!(db.get_recognition("a.jpg").unwrap().is_some());
        db.delete_recognitions(&["a.jpg".into()]).unwrap();
        assert!(db.get_recognition("a.jpg").unwrap().is_none());
    }

    #[test]
    fn test_recognition_rename() {
        let tmp = TempDir::new().unwrap();
        let db = FolderDb::open_in_dir(tmp.path()).unwrap();
        let rec = make_recognition();
        db.upsert_recognition("old.jpg", &rec).unwrap();
        db.rename_recognition("old.jpg", "new.jpg").unwrap();
        assert!(db.get_recognition("old.jpg").unwrap().is_none());
        assert!(db.get_recognition("new.jpg").unwrap().is_some());
    }

    #[test]
    fn test_recognition_copy_to() {
        let tmp = TempDir::new().unwrap();
        let src = FolderDb::open_in_dir(tmp.path()).unwrap();
        let rec = make_recognition();
        src.upsert_recognition("src/bird.jpg", &rec).unwrap();

        let tmp2 = TempDir::new().unwrap();
        let mut dst = FolderDb::open_in_dir(tmp2.path()).unwrap();
        src.copy_recognitions_to(&mut dst, &[("src/bird.jpg".into(), "dst/bird.jpg".into())])
            .unwrap();
        let got = dst.get_recognition("dst/bird.jpg").unwrap().expect("should be copied");
        assert_eq!(got.status, RecognitionStatus::NeedsReview);
        assert_eq!(got.bird.as_ref().map(|b| b.bird_id), Some(42));
    }

    #[test]
    fn test_all_recognitions() {
        let tmp = TempDir::new().unwrap();
        let db = FolderDb::open_in_dir(tmp.path()).unwrap();
        let rec = make_recognition();
        db.upsert_recognition("a.jpg", &rec).unwrap();
        db.upsert_recognition("b.jpg", &rec).unwrap();
        let all = db.all_recognitions().unwrap();
        assert_eq!(all.len(), 2);
        let paths: Vec<&str> = all.iter().map(|(p, _)| p.as_str()).collect();
        assert!(paths.contains(&"a.jpg"));
        assert!(paths.contains(&"b.jpg"));
    }

    #[test]
    fn test_legacy_cache_migration() {
        let tmp = TempDir::new().unwrap();
        // 创建遗留 .pt-cache.db 文件
        let legacy_path = tmp.path().join(".pt-cache.db");
        {
            let mut conn = rusqlite::Connection::open(&legacy_path).unwrap();
            conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;").unwrap();
            conn.execute_batch(
                "CREATE TABLE exif_cache (
                    path TEXT PRIMARY KEY,
                    file_size INTEGER NOT NULL,
                    mtime_ns INTEGER NOT NULL,
                    make TEXT, model TEXT, lens TEXT,
                    exposure_time TEXT, f_number TEXT, iso INTEGER,
                    focal_length TEXT, exposure_compensation TEXT,
                    white_balance TEXT, date_time_original TEXT,
                    image_width INTEGER, image_height INTEGER,
                    file_size_cache INTEGER, color_space TEXT,
                    orientation INTEGER,
                    gps_lat_deg REAL, gps_lat_min REAL, gps_lat_sec REAL,
                    gps_lon_deg REAL, gps_lon_min REAL, gps_lon_sec REAL,
                    gps_altitude REAL
                );",
            ).unwrap();
            conn.execute(
                "INSERT INTO exif_cache (path, file_size, mtime_ns) VALUES (?1, ?2, ?3)",
                rusqlite::params!["test.jpg", 100, 1000],
            ).unwrap();
        }

        // 打开应该自动迁移到 .pt/data.db
        let db = FolderDb::open_in_dir(tmp.path()).unwrap();
        // 旧文件已迁移/删除
        assert!(!tmp.path().join(".pt-cache.db").exists());
        assert!(tmp.path().join(".pt/data.db").exists());
        // 数据可读（沿用旧数据）
        let exif = db.get_exif(&tmp.path().join("test.jpg")).unwrap();
        assert!(exif.is_none()); // 被旧的 file_fingerprint 跳过（文件不存在，返回 None）
    }

    #[test]
    fn test_recognition_upsert_get_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let db = FolderDb::open_in_dir(tmp.path()).unwrap();
        let rec = make_recognition();
        db.upsert_recognition("photos/sparrow.jpg", &rec).unwrap();
        let got = db.get_recognition("photos/sparrow.jpg").unwrap().expect("should exist");
        assert_eq!(got.status, RecognitionStatus::NeedsReview);
        assert_eq!(got.class_index, Some(123));
        assert!(got.bbox.is_some());
    }

    #[test]
    fn test_recognition_eye_fields_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let db = FolderDb::open_in_dir(tmp.path()).unwrap();
        let mut rec = make_recognition();
        rec.eye_sharpness = Some(88.3);
        rec.eye_bbox = Some(BBox::new(0.2, 0.3, 0.45, 0.55));
        db.upsert_recognition("photos/owl.jpg", &rec).unwrap();
        let got = db.get_recognition("photos/owl.jpg").unwrap().expect("should exist");
        assert_eq!(got.eye_sharpness, Some(88.3));
        assert_eq!(got.eye_bbox, Some(BBox::new(0.2, 0.3, 0.45, 0.55)));
    }

    #[test]
    fn test_recognition_eye_fields_null_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let db = FolderDb::open_in_dir(tmp.path()).unwrap();
        let mut rec = make_recognition();
        rec.eye_sharpness = None;
        rec.eye_bbox = None;
        db.upsert_recognition("photos/crow.jpg", &rec).unwrap();
        let got = db.get_recognition("photos/crow.jpg").unwrap().expect("should exist");
        assert!(got.eye_sharpness.is_none());
        assert!(got.eye_bbox.is_none());
    }

    #[test]
    fn test_recognition_migration_old_rows_null() {
        let tmp = TempDir::new().unwrap();
        let pt_dir = tmp.path().join(".pt");
        std::fs::create_dir_all(&pt_dir).unwrap();
        let db_path = pt_dir.join("data.db");
        {
            // 创建不含 eye 列的旧版数据库，模拟迁移前状态
            let mut conn = rusqlite::Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS recognition (
                    rel_path    TEXT PRIMARY KEY,
                    status      TEXT NOT NULL,
                    bird_id     INTEGER,
                    bird_name   TEXT,
                    class_index INTEGER,
                    confidence  REAL,
                    bbox        TEXT,
                    candidates  TEXT,
                    failure_stage TEXT,
                    recognized_at TEXT NOT NULL
                );",
            )
            .unwrap();
            // 插入旧行
            conn.execute(
                "INSERT INTO recognition (rel_path, status, failure_stage, recognized_at) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params!["old/bird.jpg", "Unrecognized", "None", "2026-01-01T00:00:00Z"],
            )
            .unwrap();
            // 设置 user_version = 3，模拟前三版迁移已执行
            conn.pragma_update(None, "user_version", 3i64).unwrap();
        }
        // 打开（触发迁移 4 追加 eye 列）
        let db = FolderDb::open_in_dir(tmp.path()).unwrap();
        let got = db.get_recognition("old/bird.jpg").unwrap().expect("old row should exist");
        assert!(got.eye_sharpness.is_none(), "迁移前旧行的 eye_sharpness 应为 None");
        assert!(got.eye_bbox.is_none(), "迁移前旧行的 eye_bbox 应为 None");
    }

    #[test]
    fn test_sync_with_scan_stale_delete_and_fingerprint() {
        let tmp = TempDir::new().unwrap();
        let db = FolderDb::open_in_dir(tmp.path()).unwrap();
        let exif = make_exif();
        let rec = make_recognition();
        let xmp = XmpMetadata::default();

        // keep：磁盘存在且已入库（指纹匹配）
        let keep = tmp.path().join("keep.jpg");
        std::fs::write(&keep, b"fake").unwrap();
        db.put_exif(&keep, &exif).unwrap();

        // gone：先入库再从磁盘删除 → 三表行都是垃圾
        let gone = tmp.path().join("gone.jpg");
        std::fs::write(&gone, b"fake").unwrap();
        db.put_exif(&gone, &exif).unwrap();
        db.put_xmp(&gone, &xmp).unwrap();
        db.upsert_recognition("gone.jpg", &rec).unwrap();
        std::fs::remove_file(&gone).unwrap();

        let (ksize, kmtime) = file_fingerprint(&keep).unwrap();
        let entries = vec![FileEntry {
            full_path: keep.clone(),
            rel_path: "keep.jpg".into(),
            file_size: ksize,
            mtime_ns: kmtime as i64,
            format: ImageFormat::Jpeg,
        }];

        let stats = db.sync_with_scan(&entries, &|_, _| {}).unwrap();
        // gone 的三表行被删；keep 指纹未变 → 不重提取
        assert_eq!(stats.cache_deleted, 1);
        assert_eq!(stats.recognition_deleted, 1);
        assert_eq!(stats.cache_updated, 0);
        assert_eq!(stats.cache_failed, 0);
        assert!(db.get_exif(&keep).unwrap().is_some());
        assert!(db.get_xmp(&gone).unwrap().is_none());
        assert!(db.get_recognition("gone.jpg").unwrap().is_none());

        // keep 内容变化（指纹不同）→ sync 不再串行重提取（提取统一由 app 层
        // spawn_enrich_tasks 的 get_or_extract_exif 完成），旧行保留但指纹失效
        std::fs::write(&keep, b"changed data here").unwrap();
        let (ksize2, kmtime2) = file_fingerprint(&keep).unwrap();
        let entries2 = vec![FileEntry {
            full_path: keep.clone(),
            rel_path: "keep.jpg".into(),
            file_size: ksize2,
            mtime_ns: kmtime2 as i64,
            format: ImageFormat::Jpeg,
        }];
        let stats2 = db.sync_with_scan(&entries2, &|_, _| {}).unwrap();
        assert_eq!(stats2.cache_updated, 0);
        assert_eq!(stats2.cache_failed, 0);
        // 旧行仍在但指纹不匹配：get_exif 视为未命中（由 enrich 的 put_exif 覆盖）
        assert!(db.get_exif(&keep).unwrap().is_none());
    }

    #[test]
    fn test_adjustments_table_exists_and_rw() {
        let tmp = TempDir::new().unwrap();
        let db = FolderDb::open_in_dir(tmp.path()).unwrap();
        let params = make_adjustments();
        db.put_adjustments("a.NEF", &params).unwrap();
        let got = db.get_adjustments("a.NEF").unwrap().unwrap();
        assert_eq!(got.exposure, params.exposure);
        assert_eq!(got.contrast, params.contrast);
        assert_eq!(got.saturation, params.saturation);
    }

    #[test]
    fn test_adjustments_none_when_absent() {
        let tmp = TempDir::new().unwrap();
        let db = FolderDb::open_in_dir(tmp.path()).unwrap();
        assert!(db.get_adjustments("nope.jpg").unwrap().is_none());
    }

    #[test]
    fn test_adjustments_upsert_overwrite() {
        let tmp = TempDir::new().unwrap();
        let db = FolderDb::open_in_dir(tmp.path()).unwrap();
        let a = make_adjustments();
        let b = AdjustParams {
            exposure: -0.5,
            ..a
        };
        db.put_adjustments("x.jpg", &a).unwrap();
        db.put_adjustments("x.jpg", &b).unwrap();
        let got = db.get_adjustments("x.jpg").unwrap().unwrap();
        assert_eq!(got.exposure, -0.5);
        assert_eq!(got.saturation, a.saturation);
    }

    #[test]
    fn test_adjustments_rename_and_delete() {
        let tmp = TempDir::new().unwrap();
        let db = FolderDb::open_in_dir(tmp.path()).unwrap();
        let params = make_adjustments();
        db.put_adjustments("old.jpg", &params).unwrap();
        db.rename_adjustment("old.jpg", "new.jpg").unwrap();
        assert!(db.get_adjustments("old.jpg").unwrap().is_none());
        assert!(db.get_adjustments("new.jpg").unwrap().is_some());
        db.delete_adjustments(&["new.jpg".into()]).unwrap();
        assert!(db.get_adjustments("new.jpg").unwrap().is_none());
    }

    #[test]
    fn test_adjustments_copy_to_target_db() {
        let tmp = TempDir::new().unwrap();
        let src = FolderDb::open_in_dir(tmp.path()).unwrap();
        let dst_dir = tmp.path().join("dst");
        std::fs::create_dir_all(&dst_dir).unwrap();
        let mut dst = FolderDb::open_in_dir(&dst_dir).unwrap();
        let params = make_adjustments();
        src.put_adjustments("a.jpg", &params).unwrap();
        src.copy_adjustments_to(&mut dst, &[("a.jpg".into(), "b.jpg".into())]).unwrap();
        let got = dst.get_adjustments("b.jpg").unwrap().unwrap();
        assert_eq!(got.exposure, params.exposure);
        assert_eq!(got.saturation, params.saturation);
        // 源库行保留（复制语义）
        assert!(src.get_adjustments("a.jpg").unwrap().is_some());
    }

    #[test]
    fn test_adjustments_sync_with_scan_cleanup() {
        let tmp = TempDir::new().unwrap();
        let db = FolderDb::open_in_dir(tmp.path()).unwrap();
        let params = make_adjustments();
        db.put_adjustments("gone.jpg", &params).unwrap();
        let keep = tmp.path().join("keep.jpg");
        std::fs::write(&keep, b"fake").unwrap();
        let (ksize, kmtime) = file_fingerprint(&keep).unwrap();
        let entries = vec![FileEntry {
            full_path: keep.clone(),
            rel_path: "keep.jpg".into(),
            file_size: ksize,
            mtime_ns: kmtime as i64,
            format: ImageFormat::Jpeg,
        }];
        let stats = db.sync_with_scan(&entries, &|_, _| {}).unwrap();
        assert_eq!(stats.adjustments_deleted, 1);
        assert!(db.get_adjustments("gone.jpg").unwrap().is_none());
    }
}
