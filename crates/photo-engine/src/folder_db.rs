//! 照片文件夹级中心数据库（`.pt/data.db`），管理三类表：
//!
//! - **exif_cache**：缓存表，EXIF 元数据的 LRU 风格缓存。
//!   缓存表**可被清除**（清空释放空间）——丢失后只会触发重新提取。
//!
//! - **adjustments**：调整参数（用户手动编辑，属真相表）。
//!
//! - **duplicates**：近重复检测结果（**派生数据**，重跑检测即覆盖）。
//!   与 exif_cache 同类，可整表清空；`load_duplicates` 顺手清磁盘上已不存在的行。
//!
//! - **xmp_meta** / **recognition** / **keywords**：真相表，存储 XMP 元数据、物种识别结果
//!   与用户关键词标签。
//!   **任何清理缓存的操作都不得触碰 xmp_meta、recognition 与 keywords 表**——
//!   识别结果不可重新计算（需要 YOLO + 模型推理），XMP 元数据与关键词为用户手动编辑。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use parking_lot::Mutex;
use thiserror::Error;
use rusqlite_migration::{Migrations, M};

use photo_domain::{
    AdjustParams, BBox, CnLevel, ImageFormat, Recognition, RecognitionFailureStage,
    RecognitionStatus, SubjectRecognition, TaxonCandidate, TaxonMatch,
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
                gps_altitude REAL,
                focus_point  TEXT
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
        // keywords 真相表：用户手动编辑的关键词标签，键为完整路径（与 xmp_meta 同键约定）
        M::up(
            "CREATE TABLE IF NOT EXISTS keywords (
                path    TEXT NOT NULL,
                keyword TEXT NOT NULL,
                PRIMARY KEY (path, keyword)
            );",
        ),
        // 识别结论泛化到物种：追加学名与中文名级别两列。
        // BioCLIP 后端大多数预测不在本地名录里（bird_id 为 NULL），只存中文名的话这些
        // 结果重载后只剩空字符串（展示名退化），必须持久化学名；cn_level 记中文名来自
        // 种 / 属 / 科哪一级。migration 3 的建表语句是 v3 历史 schema，保持原样不动
        // （新库同样按 3 → 本条顺序执行），新列在链末尾 ALTER 补上，新库与迁移库 schema 一致。
        // ALTER TABLE 一次只能加一列，两条分开写。
        M::up(
            "ALTER TABLE recognition ADD COLUMN latin_name TEXT;
             ALTER TABLE recognition ADD COLUMN cn_level TEXT;",
        ),
        // 移除鸟眼锐度（该阶段 2026-09-22 整体删除）。链中间的 ADD COLUMN 是历史，
        // 不能删改（会打乱既有库的 user_version 记账），所以在末尾 DROP 收敛到同一 schema。
        M::up(
            "ALTER TABLE recognition DROP COLUMN eye_sharpness;
             ALTER TABLE recognition DROP COLUMN eye_bbox;",
        ),
        // 多主体：识别结论从「一张图一个」扩展为「每主体一个」，
        // 完整主体列表以 JSON 存 subjects 列（顶层列仍是主主体，兼容旧展示）。
        M::up("ALTER TABLE recognition ADD COLUMN subjects TEXT;"),
        // 近重复检测结果（**派生数据，可重算**）：每行 = 一张入组照片，组内首张 keeper=1。
        // 键为相对路径（与 recognition / adjustments 同约定）；阈值与计算时间随行存储，
        // 供 UI 展示「这组是什么时候用什么阈值算出来的」。清理缓存时可整表清空。
        // 调整参数扩展（3.13）：阴影 / 高光 / 色温 / 色调四列，追加到链末尾。
        // 老库已有 adjustments 表（含历史 crop 列）→ ALTER 补列；新库按同一条链执行后同样补列，
        // 两条路径 schema 一致（crop 列是历史遗留，保持不动）。
        M::up(
            "ALTER TABLE adjustments ADD COLUMN shadows INTEGER NOT NULL DEFAULT 0;
             ALTER TABLE adjustments ADD COLUMN highlights INTEGER NOT NULL DEFAULT 0;
             ALTER TABLE adjustments ADD COLUMN temperature INTEGER NOT NULL DEFAULT 0;
             ALTER TABLE adjustments ADD COLUMN tint INTEGER NOT NULL DEFAULT 0;",
        ),
        M::up(
            "CREATE TABLE IF NOT EXISTS duplicates (
                rel_path    TEXT PRIMARY KEY,
                group_index INTEGER NOT NULL,
                keeper      INTEGER NOT NULL DEFAULT 0,
                threshold   INTEGER NOT NULL,
                computed_at TEXT NOT NULL
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
    /// 照片目录根（recognition/adjustments 的相对路径键 → 磁盘存在性判定用）
    root: PathBuf,
}

impl FolderDb {
    /// 在指定目录中打开或创建 `.pt/data.db`，自动迁移表结构。
    /// 自动处理从旧版 `.pt-cache.db` 的迁移。
    pub fn open_in_dir(dir: &Path) -> Result<Self, FolderDbError> {
        let pt_dir = dir.join(".pt");
        std::fs::create_dir_all(&pt_dir)?;
        let db_path = pt_dir.join("data.db");

        // 检测遗留 .pt-cache.db，迁移到新位置。
        // 只在「新库不存在」时迁移；若 data.db 已存在而遗留文件仍在（部分迁移/
        // 旧版残留），一律保留——无条件删除可能丢掉尚未合并的 -wal 已提交事务。
        let legacy_path = dir.join(".pt-cache.db");
        if legacy_path.exists() && !db_path.exists() {
            for legacy_ext in &["", "-wal", "-shm"] {
                let legacy_file = dir.join(format!(".pt-cache.db{}", legacy_ext));
                if legacy_file.exists() {
                    let new_file = pt_dir.join(format!("data.db{}", legacy_ext));
                    std::fs::rename(&legacy_file, &new_file)?;
                }
            }
        }

        let mut conn = rusqlite::Connection::open(&db_path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
        folder_migrations().to_latest(&mut conn)?;
        // 兼容旧库：exif_cache 若缺 focus_point 列则补列。
        // 不能写进 migration——bundled SQLite 不支持 ADD COLUMN IF NOT EXISTS
        // （3.35+ 才引入），且畸形旧库可能整个表缺失；这里用列元数据检查，
        // 新库（migration 建表已含该列）直接跳过。
        ensure_exif_focus_column(&mut conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            root: dir.to_path_buf(),
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
                    gps_lon_deg, gps_lon_min, gps_lon_sec, gps_altitude, focus_point
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
              gps_lon_deg, gps_lon_min, gps_lon_sec, gps_altitude, focus_point)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26)",
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
        // 提取失败不写缓存：失败若被写成全 NULL 行，会以正确指纹命中缓存，
        // 之后 enrich 永远拿到空 EXIF 且不再重试（负缓存毒化）。
        match crate::exif::extract_exif(path, format) {
            Ok(exif) => {
                let _ = self.put_exif(path, &exif);
                Ok(exif)
            }
            Err(e) => {
                tracing::warn!(
                    "EXIF 提取失败（不写缓存，下次重试）: {} — {e}",
                    path.display()
                );
                Ok(ExifMetadata::default())
            }
        }
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
                    gps_lon_deg, gps_lon_min, gps_lon_sec, gps_altitude, focus_point,
                    path, file_size, mtime_ns
             FROM exif_cache",
        )?;
        let rows = stmt.query_map([], |row| {
            // 列 0-22 与 row_to_exif 对齐；23-25 追加 path/指纹
            let exif = row_to_exif(row)?;
            Ok((
                row.get::<_, String>(23)?,
                ExifCacheRow {
                    file_size: row.get(24)?,
                    mtime_ns: row.get(25)?,
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
        let subjects_str = serde_json::to_string(&rec.subjects)?;
        // bird_id 列即名录主键（taxon_id），列名保持历史不动；BioCLIP 预测不在名录中时为 NULL。
        // ranks 是七级分类明细，不持久化（识别当次可用，落库无消费方）。
        conn.execute(
            "INSERT OR REPLACE INTO recognition
             (rel_path, status, bird_id, bird_name, class_index, confidence,
              bbox, candidates, failure_stage, recognized_at,
              latin_name, cn_level, subjects)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            rusqlite::params![
                normalized,
                rec.status.as_str(),
                rec.taxon.as_ref().and_then(|t| t.taxon_id),
                rec.taxon.as_ref().map(|t| t.cn_name.as_str()),
                rec.class_index.map(|v| v as i64),
                rec.confidence,
                bbox_str,
                candidates_str,
                rec.failure_stage.as_str(),
                rec.recognized_at,
                rec.taxon.as_ref().map(|t| t.latin_name.as_str()),
                rec.taxon.as_ref().map(|t| t.cn_level.as_str()),
                subjects_str,
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
                    latin_name, cn_level, subjects
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
                    latin_name, cn_level, subjects
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
                        latin_name, cn_level, subjects
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
            "INSERT OR REPLACE INTO adjustments
                (rel_path, exposure, contrast, saturation, shadows, highlights, temperature, tint)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                normalized,
                params.exposure as f64,
                params.contrast as i64,
                params.saturation as i64,
                params.shadows as i64,
                params.highlights as i64,
                params.temperature as i64,
                params.tint as i64,
            ],
        )?;
        Ok(())
    }

    /// 查询单条调整参数（无行 = None = 无调整）。
    pub fn get_adjustments(&self, rel_path: &str) -> Result<Option<AdjustParams>, FolderDbError> {
        let conn = self.conn.lock();
        let normalized = rel_path.replace('\\', "/");
        let mut stmt = conn.prepare_cached(
            "SELECT exposure, contrast, saturation, shadows, highlights, temperature, tint
             FROM adjustments WHERE rel_path = ?1",
        )?;
        // i64 → i32 用 try_from：越界（数据损坏）报错而非静默截断
        match stmt.query_row(rusqlite::params![normalized], |row| {
            Ok((
                row.get::<_, f64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
            ))
        }) {
            Ok((exposure, contrast, saturation, shadows, highlights, temperature, tint)) => {
                Ok(Some(adjust_params_from_parts(
                    exposure,
                    contrast,
                    saturation,
                    shadows,
                    highlights,
                    temperature,
                    tint,
                )?))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// 全量读取调整行（rel_path → 参数）。重扫回填「已调整」标识用：
    /// 一次 SQL 拿全集，避免逐张 get（N 次加锁 + N 次查询）。
    pub fn all_adjustments(&self) -> Result<HashMap<String, AdjustParams>, FolderDbError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare_cached(
            "SELECT rel_path, exposure, contrast, saturation, shadows, highlights, temperature, tint
             FROM adjustments",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, f64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
            ))
        })?;
        let mut out: HashMap<String, AdjustParams> = HashMap::new();
        for row in rows {
            let (rel, exposure, contrast, saturation, shadows, highlights, temperature, tint) = row?;
            out.insert(
                rel,
                adjust_params_from_parts(
                    exposure,
                    contrast,
                    saturation,
                    shadows,
                    highlights,
                    temperature,
                    tint,
                )?,
            );
        }
        Ok(out)
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
                "SELECT exposure, contrast, saturation, shadows, highlights, temperature, tint
                 FROM adjustments WHERE rel_path = ?1",
            )?;
            for (src_rel, dst_rel) in entries {
                let normalized = src_rel.replace('\\', "/");
                match stmt.query_row(rusqlite::params![normalized], |row| {
                    Ok((
                        row.get::<_, f64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                    ))
                }) {
                    Ok((exposure, contrast, saturation, shadows, highlights, temperature, tint)) => {
                        let params = adjust_params_from_parts(
                            exposure,
                            contrast,
                            saturation,
                            shadows,
                            highlights,
                            temperature,
                            tint,
                        )?;
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

    // ── keywords 真相表（用户关键词标签，键为完整路径，不可当缓存清除）──

    /// 全量替换某路径的关键词集合：先删后插（原子语义由 conn 互斥保证，
    /// 无其他线程可穿插）。关键词归一化：去首尾空白、去空串、去重（保序）。
    pub fn set_keywords(&self, path: &str, keywords: &[String]) -> Result<(), FolderDbError> {
        let conn = self.conn.lock();
        conn.execute("DELETE FROM keywords WHERE path = ?1", rusqlite::params![path])?;
        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for kw in keywords {
            let trimmed = kw.trim();
            if trimmed.is_empty() || !seen.insert(trimmed) {
                continue;
            }
            conn.execute(
                "INSERT INTO keywords (path, keyword) VALUES (?1, ?2)",
                rusqlite::params![path, trimmed],
            )?;
        }
        Ok(())
    }

    /// 全量读取 keywords 真相表（扫描闭包一次性载入内存，键为完整路径字符串，
    /// 与 set_keywords 一致）。每个路径的关键词按插入序返回。
    pub fn all_keywords(&self) -> Result<HashMap<String, Vec<String>>, FolderDbError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare_cached(
            "SELECT path, keyword FROM keywords ORDER BY path, rowid",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        for row in rows {
            let (path, keyword) = row?;
            map.entry(path).or_default().push(keyword);
        }
        Ok(map)
    }

    /// 批量删除关键词行（文件删除后同步，对齐 delete_xmp_rows）。
    pub fn delete_keyword_rows(&self, paths: &[String]) -> Result<(), FolderDbError> {
        let conn = self.conn.lock();
        for p in paths {
            conn.execute("DELETE FROM keywords WHERE path = ?1", rusqlite::params![p])?;
        }
        Ok(())
    }

    /// 重命名关键词行的键（文件移动/重命名后同步，对齐 rename_xmp）。
    pub fn rename_keywords(&self, old_path: &str, new_path: &str) -> Result<(), FolderDbError> {
        let conn = self.conn.lock();
        conn.execute(
            "UPDATE keywords SET path = ?1 WHERE path = ?2",
            rusqlite::params![new_path, old_path],
        )?;
        Ok(())
    }

    /// 将一批 keywords 行复制到目标库（跨文件夹移动/复制用，对齐 copy_xmp_rows_to）。
    /// entries: (源路径, 目标路径)。目标键已有关键词时整体替换（复制覆盖语义）。
    pub fn copy_keywords_to(
        &self,
        target_db: &mut FolderDb,
        entries: &[(String, String)],
    ) -> Result<(), FolderDbError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare_cached(
            "SELECT keyword FROM keywords WHERE path = ?1 ORDER BY rowid",
        )?;
        for (src, dst) in entries {
            let keywords: Vec<String> = stmt
                .query_map(rusqlite::params![src], |row| row.get::<_, String>(0))?
                .filter_map(|r| r.ok())
                .collect();
            if !keywords.is_empty() {
                target_db.set_keywords(dst, &keywords)?;
            }
        }
        Ok(())
    }

    // ── 近重复检测结果（派生缓存：可重算、可整表清空）──

    /// 整体替换重复检测结果（重跑即覆盖；`groups` 为空 = 清空）。
    ///
    /// `groups` 是**相对路径（正斜杠）**分组，组内第 0 个即保留锚点（keeper=1）。
    /// `threshold` / `computed_at` 随行存储，供 UI 展示与后续阈值对比。
    pub fn replace_duplicates(
        &self,
        groups: &[Vec<String>],
        threshold: u32,
        computed_at: &str,
    ) -> Result<(), FolderDbError> {
        let conn = self.conn.lock();
        let tx = conn.unchecked_transaction()?;
        tx.execute("DELETE FROM duplicates", [])?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT OR REPLACE INTO duplicates
                    (rel_path, group_index, keeper, threshold, computed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for (gi, group) in groups.iter().enumerate() {
                for (mi, rel) in group.iter().enumerate() {
                    stmt.execute(rusqlite::params![
                        rel.replace('\\', "/"),
                        gi as i64,
                        i64::from(mi == 0),
                        threshold as i64,
                        computed_at
                    ])?;
                }
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// 读取全部重复检测结果（按组序 + keeper 优先 + 路径序，结果确定）。
    ///
    /// 磁盘上已不存在的行顺手清掉（删照片后没重跑检测也不至于把幽灵条目喂给 UI），
    /// 与 sync_with_scan 的孤儿清理同一口径：**只在文件确实不存在时删**。
    pub fn load_duplicates(&self) -> Result<Vec<DuplicateRow>, FolderDbError> {
        let conn = self.conn.lock();
        let rows: Vec<DuplicateRow> = {
            let mut stmt = conn.prepare_cached(
                "SELECT rel_path, group_index, keeper, threshold, computed_at
                 FROM duplicates ORDER BY group_index, keeper DESC, rel_path",
            )?;
            let mapped = stmt.query_map([], |row| {
                let threshold: i64 = row.get(3)?;
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)? != 0,
                    threshold,
                    row.get::<_, String>(4)?,
                ))
            })?;
            let mut out = Vec::new();
            for r in mapped {
                let (rel_path, group_index, keeper, threshold, computed_at) = r?;
                let threshold = u32::try_from(threshold)
                    .map_err(|_| FolderDbError::ValueOutOfRange("duplicates.threshold"))?;
                out.push(DuplicateRow { rel_path, group_index, keeper, threshold, computed_at });
            }
            out
        };
        drop(conn);
        let mut live = Vec::with_capacity(rows.len());
        let mut stale: Vec<String> = Vec::new();
        for r in rows {
            if self.full_path_of_rel(&r.rel_path).exists() {
                live.push(r);
            } else {
                stale.push(r.rel_path);
            }
        }
        if !stale.is_empty() {
            let conn = self.conn.lock();
            let mut stmt = conn.prepare_cached("DELETE FROM duplicates WHERE rel_path = ?1")?;
            for rel in &stale {
                stmt.execute(rusqlite::params![rel])?;
            }
        }
        Ok(live)
    }

    /// 清空重复检测结果（重跑检测前 / 换目录时调用）。
    pub fn clear_duplicates(&self) -> Result<(), FolderDbError> {
        let conn = self.conn.lock();
        conn.execute("DELETE FROM duplicates", [])?;
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

/// 近重复检测结果单行（派生缓存，可重算）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateRow {
    /// 相对目录的正斜杠路径（duplicates 表主键）
    pub rel_path: String,
    /// 组序号（同一组内相同；越小越靠前）
    pub group_index: i64,
    /// 是否为该组保留锚点（组内首张）
    pub keeper: bool,
    /// 计算该结果时用的汉明距离阈值
    pub threshold: u32,
    /// 检测时间（RFC3339 字符串，UI 直接展示）
    pub computed_at: String,
}

/// 文件条目信息（由 app 层扫描产生，传给 sync_with_scan 做三表同步）。
pub struct FileEntry {
    pub full_path: PathBuf,
    /// 相对目录的正斜杠路径（用于 recognition 表键）。
    /// 递归扫描时含子目录分隔符（`sub/deep/bird.jpg`）：recognition/adjustments
    /// 表的 rel_path 主键存储即正斜杠相对路径，upsert/get/sync 均做 `\`→`/`
    /// 归一化比较（见 upsert_recognition），子目录键天然兼容，无需按目录拆分表。
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
    /// 相对路径（正斜杠）→ 磁盘完整路径（孤儿清理的存在性判定用）。
    fn full_path_of_rel(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

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
                // 只清「不在本次扫描清单 **且磁盘上确实不存在**」的行：单层扫描时
                // 子目录文件不在清单里但真实存在，不能当孤儿删掉（会丢用户数据）
                if !entry_paths.contains(p.as_str()) && !Path::new(p).exists() {
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
                if !entry_paths.contains(p.as_str()) && !Path::new(p).exists() {
                    conn.execute("DELETE FROM xmp_meta WHERE path = ?1", rusqlite::params![p])?;
                }
            }
        }
        {
            // keywords 与 xmp_meta 同键约定（完整路径）：文件已不存在的行一并清理，
            // 防孤儿行；仍存在的文件其关键词行**保留**（真相表，随文件走）
            let mut stmt = conn.prepare_cached("SELECT DISTINCT path FROM keywords")?;
            let db_paths: Vec<String> = stmt.query_map([], |row| row.get(0))?
                .filter_map(|r| r.ok()).collect();
            for p in &db_paths {
                if !entry_paths.contains(p.as_str()) && !Path::new(p).exists() {
                    conn.execute("DELETE FROM keywords WHERE path = ?1", rusqlite::params![p])?;
                }
            }
        }
        {
            let mut stmt = conn.prepare_cached("SELECT rel_path FROM recognition")?;
            let db_rel_paths: Vec<String> = stmt.query_map([], |row| row.get(0))?
                .filter_map(|r| r.ok()).collect();
            for rp in &db_rel_paths {
                if !entry_rel_paths.contains(rp.as_str()) && !self.full_path_of_rel(rp).exists() {
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
                if !entry_rel_paths.contains(rp.as_str()) && !self.full_path_of_rel(rp).exists() {
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
    shadows: i64,
    highlights: i64,
    temperature: i64,
    tint: i64,
) -> Result<AdjustParams, FolderDbError> {
    // 非有限曝光归零（库被手改成 nan 时，f32::NAN 会一路传染进 tone 表）
    let exposure = if exposure.is_finite() {
        exposure as f32
    } else {
        0.0
    };
    let narrow = |v: i64, name: &'static str| -> Result<i32, FolderDbError> {
        i32::try_from(v).map_err(|_| FolderDbError::ValueOutOfRange(name))
    };
    Ok(AdjustParams {

        exposure,
        contrast: narrow(contrast, "contrast")?,
        saturation: narrow(saturation, "saturation")?,
        shadows: narrow(shadows, "shadows")?,
        highlights: narrow(highlights, "highlights")?,
        temperature: narrow(temperature, "temperature")?,
        tint: narrow(tint, "tint")?,
            ..AdjustParams::default()
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

/// 检查 exif_cache 表是否含 focus_point 列，缺则补列（旧库升级；新库建表已含）。
/// 表整体缺失的畸形旧库不补（migration 的 CREATE IF NOT EXISTS 建表时已含列）。
fn ensure_exif_focus_column(conn: &mut rusqlite::Connection) -> Result<(), FolderDbError> {
    let table_exists: i64 = conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'exif_cache'",
        [],
        |row| row.get(0),
    )?;
    if table_exists == 0 {
        return Ok(());
    }
    let has = conn
        .prepare("PRAGMA table_info(exif_cache)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .filter_map(|r| r.ok())
        .any(|name| name == "focus_point");
    if !has {
        conn.execute_batch("ALTER TABLE exif_cache ADD COLUMN focus_point TEXT;")?;
    }
    Ok(())
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
        // focus_point 列：JSON 序列化（exif_to_params 同源），坏数据回退 None
        focus_point: row
            .get::<_, Option<String>>(22)?
            .and_then(|s| serde_json::from_str(&s).ok()),
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
        // focus_point：JSON 序列化（row_to_exif 同源解析）
        Box::new(
            exif.focus_point
                .map(|fp| serde_json::to_string(&fp).unwrap_or_default()),
        ),
    ]
}

/// 行 → Recognition（列序与各 SELECT 的列清单一致）。
///
/// 物种字段映射：bird_id 列即 taxon_id（历史列名保留不动），bird_name 列即 cn_name。
/// 两者都 NULL → taxon 为 None；只要一侧非空就重建——BioCLIP 预测大多不在本地名录里
/// （taxon_id 为 NULL），学名与中文名必须保住。
/// ranks（七级分类）不持久化，读回恒为空 vec。
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
    let latin_name: Option<String> = row.get(10)?;
    let cn_level_str: Option<String> = row.get(11)?;
    let subjects_str: Option<String> = row.get(12)?;

    let taxon = if bird_id.is_none() && bird_name.is_none() {
        None
    } else {
        Some(TaxonMatch {
            taxon_id: bird_id,
            cn_name: bird_name.unwrap_or_default(),
            latin_name: latin_name.unwrap_or_default(),
            // 空列 / 非法文本都按「无中文名」处理（历史行 cn_level 为 NULL）
            cn_level: cn_level_str
                .as_deref()
                .and_then(CnLevel::from_str)
                .unwrap_or(CnLevel::Missing),
            ranks: vec![],
        })
    };

    let candidates: Vec<TaxonCandidate> = match candidates_str {
        Some(s) => serde_json::from_str(&s).unwrap_or_default(),
        None => Vec::new(),
    };

    // 多主体：subjects 列 JSON；旧行（NULL）为空 Vec，读侧以顶层字段兜底单主体
    let subjects: Vec<SubjectRecognition> = match subjects_str {
        Some(s) => serde_json::from_str(&s).unwrap_or_default(),
        None => Vec::new(),
    };

    Ok(Ok(Recognition {
        status,
        taxon,
        class_index: class_index.map(|v| v as u32),
        confidence: confidence.map(|v| v as f32),
        bbox: bbox_str.and_then(|s| BBox::parse(&s)),
        candidates,
        failure_stage,
        recognized_at,
        subjects,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use photo_domain::{CameraInfo, GpsInfo, ShootingParams};
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
            focus_point: Some(photo_domain::FocusPoint::point(0.5, 0.5)),
        }
    }

    fn make_adjustments() -> AdjustParams {
        AdjustParams {

            exposure: 1.25,
            contrast: -30,
            saturation: 45,
            ..AdjustParams::default()
        }
    }

    /// 名录命中的物种（taxon_id 有值 + 种级中文名）
    fn taxon(id: i64, cn: &str, latin: &str) -> TaxonMatch {
        TaxonMatch {
            taxon_id: Some(id),
            cn_name: cn.into(),
            latin_name: latin.into(),
            cn_level: CnLevel::Species,
            ranks: vec![],
        }
    }

    fn make_recognition() -> Recognition {
        Recognition {
            status: RecognitionStatus::NeedsReview,
            taxon: Some(taxon(42, "大斑啄木鸟", "Dendrocopos major")),
            class_index: Some(123),
            confidence: Some(95.5),
            bbox: Some(BBox::new(0.1, 0.2, 0.8, 0.9)),
            candidates: vec![
                TaxonCandidate {
                    class_index: 123,
                    confidence: 95.5,
                    taxon: Some(taxon(42, "大斑啄木鸟", "Dendrocopos major")),
                },
            ],
            failure_stage: RecognitionFailureStage::None,
            recognized_at: "2026-07-28T10:00:00Z".into(),
            subjects: vec![],
        }
    }

    #[test]
    fn test_recognition_multi_subject_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let db = FolderDb::open_in_dir(tmp.path()).unwrap();
        let mut rec = make_recognition();
        rec.subjects = vec![
            SubjectRecognition {
                index: 0,
                bbox: BBox::new(0.1, 0.2, 0.4, 0.5),
                taxon: Some(taxon(42, "大斑啄木鸟", "Dendrocopos major")),
                class_index: Some(123),
                confidence: Some(95.5),
                candidates: vec![],
                failure: RecognitionFailureStage::None,
            },
            SubjectRecognition {
                index: 1,
                bbox: BBox::new(0.6, 0.2, 0.9, 0.6),
                taxon: Some(taxon(7, "乌鸫", "Turdus merula")),
                class_index: Some(200),
                confidence: Some(80.0),
                candidates: vec![],
                failure: RecognitionFailureStage::None,
            },
        ];
        db.upsert_recognition("photos/multi.jpg", &rec).unwrap();
        let got = db.get_recognition("photos/multi.jpg").unwrap().expect("should exist");
        assert_eq!(got.subjects.len(), 2);
        // 主主体 = subjects[0]（顶层字段一致）
        assert_eq!(got.taxon.as_ref().and_then(|t| t.taxon_id), Some(42));
        assert_eq!(got.subjects[1].taxon.as_ref().and_then(|t| t.taxon_id), Some(7));
        assert_eq!(got.subjects[1].confidence, Some(80.0));
    }

    #[test]
    fn test_recognition_legacy_row_subjects_empty() {
        // 旧行（subjects 列 NULL）：读回 subjects 为空，UI 侧以顶层字段退化为单主体
        let tmp = TempDir::new().unwrap();
        let db = FolderDb::open_in_dir(tmp.path()).unwrap();
        let rec = make_recognition();
        db.upsert_recognition("photos/legacy.jpg", &rec).unwrap();
        let got = db.get_recognition("photos/legacy.jpg").unwrap().expect("should exist");
        assert!(got.subjects.is_empty(), "无 subjects 数据的行应为空 Vec");
        assert!(got.taxon.is_some());
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
        assert_eq!(got.taxon.as_ref().map(|t| t.cn_name.as_str()), Some("大斑啄木鸟"));
        assert_eq!(got.taxon.as_ref().map(|t| t.latin_name.as_str()), Some("Dendrocopos major"));
        assert_eq!(got.taxon.as_ref().map(|t| t.cn_level), Some(CnLevel::Species));
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
        assert_eq!(got.taxon.as_ref().and_then(|t| t.taxon_id), Some(42));
        assert_eq!(got.taxon.as_ref().map(|t| t.latin_name.as_str()), Some("Dendrocopos major"));
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
            let conn = rusqlite::Connection::open(&legacy_path).unwrap();
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
    fn test_legacy_cache_kept_when_new_db_exists() {
        let tmp = TempDir::new().unwrap();
        let pt = tmp.path().join(".pt");
        std::fs::create_dir_all(&pt).unwrap();
        // 有效的新库（上次迁移已成功）
        drop(rusqlite::Connection::open(pt.join("data.db")).unwrap());
        // 遗留文件（部分迁移残留）：不能被无条件删除（可能含未合并的已提交事务）
        std::fs::write(tmp.path().join(".pt-cache.db"), b"legacy").unwrap();
        std::fs::write(tmp.path().join(".pt-cache.db-wal"), b"wal").unwrap();

        let _db = FolderDb::open_in_dir(tmp.path()).unwrap();
        assert!(
            tmp.path().join(".pt-cache.db").exists(),
            "新库已存在时不得删遗留库"
        );
        assert!(
            tmp.path().join(".pt-cache.db-wal").exists(),
            "新库已存在时不得删遗留 WAL"
        );
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
    fn test_recognition_taxon_without_catalog_id_roundtrip() {
        // BioCLIP 场景：预测不在本地名录（taxon_id 为 None）、中文名无来源，学名必须保住
        let tmp = TempDir::new().unwrap();
        let db = FolderDb::open_in_dir(tmp.path()).unwrap();
        let mut rec = make_recognition();
        rec.taxon = Some(TaxonMatch {
            taxon_id: None,
            cn_name: String::new(),
            latin_name: "Corvus corax".into(),
            cn_level: CnLevel::Missing,
            ranks: vec!["Animalia".into()],
        });
        db.upsert_recognition("photos/raven.jpg", &rec).unwrap();
        let got = db.get_recognition("photos/raven.jpg").unwrap().expect("should exist");
        let t = got.taxon.expect("taxon_id 为 None 也应重建：学名/中文名是唯一展示来源");
        assert_eq!(t.taxon_id, None);
        assert_eq!(t.cn_name, "");
        assert_eq!(t.latin_name, "Corvus corax");
        assert_eq!(t.cn_level, CnLevel::Missing);
        assert_eq!(t.display_name(), "Corvus corax");
        // ranks 不持久化：读回恒为空
        assert!(t.ranks.is_empty());
    }

    #[test]
    fn test_recognition_migration_old_rows_null() {
        let tmp = TempDir::new().unwrap();
        let pt_dir = tmp.path().join(".pt");
        std::fs::create_dir_all(&pt_dir).unwrap();
        let db_path = pt_dir.join("data.db");
        {
            // 创建 v3 的旧版数据库（不加锐度列），模拟迁移前状态
            let conn = rusqlite::Connection::open(&db_path).unwrap();
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
        // 打开（触发迁移链：追加锐度列 → … → 末尾再 DROP 掉，收敛到当前 schema）
        let db = FolderDb::open_in_dir(tmp.path()).unwrap();
        // 读 SELECT 已含 latin_name/cn_level：迁移漏列会在此报 no such column
        let got = db.get_recognition("old/bird.jpg").unwrap().expect("old row should exist");
        // 旧两列均为 NULL → 无物种结论（不凭空字符串造出假物种）
        assert_eq!(got.taxon, None, "旧行物种两列皆 NULL 时应为 None");
    }

    #[test]
    fn test_recognition_schema_same_for_new_and_migrated_db() {
        // 「新建 + 迁移」必须收敛到同一 schema：新库顺序执行迁移 1→7，旧库（v3 建表）
        // 靠链末尾的 ALTER 补齐，两者 recognition 列集（名 + 类型 + 顺序）应完全一致。
        fn columns(db: &FolderDb) -> Vec<(String, String)> {
            let conn = db.conn.lock();
            let mut stmt = conn.prepare("PRAGMA table_info(recognition)").unwrap();
            let rows = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(1)?, row.get::<_, String>(2)?))
                })
                .unwrap();
            rows.filter_map(|r| r.ok()).collect()
        }

        let fresh = TempDir::new().unwrap();
        let fresh_db = FolderDb::open_in_dir(fresh.path()).unwrap();

        let old = TempDir::new().unwrap();
        let pt = old.path().join(".pt");
        std::fs::create_dir_all(&pt).unwrap();
        {
            let conn = rusqlite::Connection::open(pt.join("data.db")).unwrap();
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
            conn.pragma_update(None, "user_version", 3i64).unwrap();
        }
        let migrated_db = FolderDb::open_in_dir(old.path()).unwrap();

        let cols = columns(&fresh_db);
        assert_eq!(cols, columns(&migrated_db), "新库与迁移库 recognition schema 应一致");
        assert!(cols.iter().any(|(n, _)| n == "latin_name"));
        assert!(cols.iter().any(|(n, _)| n == "cn_level"));
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

    /// 全量读取：一次拿到所有调整行（重扫回填「已调整」徽标用）
    #[test]
    fn test_all_adjustments_lists_every_row() {
        let tmp = TempDir::new().unwrap();
        let db = FolderDb::open_in_dir(tmp.path()).unwrap();
        let params = make_adjustments();
        db.put_adjustments("a.NEF", &params).unwrap();
        db.put_adjustments("sub/b.jpg", &AdjustParams::default())
            .unwrap();
        let all = db.all_adjustments().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all.get("a.NEF"), Some(&params));
        // 全零行也在：是否算「已调整」由调用方的 is_neutral 判定，查询层不丢数据
        assert!(all.get("sub/b.jpg").is_some_and(|p| p.is_neutral()));
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
    fn test_sync_with_scan_empty_entries_cleans_all() {
        // 目录被外部清空后重扫：entries 为空 → 四表全部行都是孤儿，一次清干净
        let tmp = TempDir::new().unwrap();
        let db = FolderDb::open_in_dir(tmp.path()).unwrap();
        let exif = make_exif();
        let xmp = XmpMetadata::default();
        let rec = make_recognition();
        let params = make_adjustments();
        let f = tmp.path().join("f.jpg");
        std::fs::write(&f, b"fake").unwrap();
        db.put_exif(&f, &exif).unwrap();
        db.put_xmp(&f, &xmp).unwrap();
        db.upsert_recognition("f.jpg", &rec).unwrap();
        db.put_adjustments("f.jpg", &params).unwrap();
        std::fs::remove_file(&f).unwrap();

        let stats = db.sync_with_scan(&[], &|_, _| {}).unwrap();
        assert_eq!(stats.cache_deleted, 1);
        assert_eq!(stats.recognition_deleted, 1);
        assert_eq!(stats.adjustments_deleted, 1);
        assert!(db.all_exif().unwrap().is_empty());
        assert!(db.all_xmp_meta().unwrap().is_empty());
        assert!(db.all_recognitions().unwrap().is_empty());
        assert!(db.get_adjustments("f.jpg").unwrap().is_none());
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

    #[test]
    fn test_keywords_table_exists_and_set() {
        let tmp = TempDir::new().unwrap();
        let db = FolderDb::open_in_dir(tmp.path()).unwrap();
        db.set_keywords("a.jpg", &["鸟".into(), "天空".into()]).unwrap();
        let all = db.all_keywords().unwrap();
        let kws = all.get("a.jpg").expect("应有 a.jpg 的关键词");
        assert_eq!(kws, &["鸟".to_string(), "天空".to_string()]);
    }

    #[test]
    fn test_keywords_set_normalizes_and_overwrites() {
        let tmp = TempDir::new().unwrap();
        let db = FolderDb::open_in_dir(tmp.path()).unwrap();
        // 去空白/去空串/去重（保序），且全量替换语义：第二次 set 覆盖第一次
        db.set_keywords("a.jpg", &[" 鸟 ".into(), "".into(), "鸟".into(), "天空".into()])
            .unwrap();
        let kws = db.all_keywords().unwrap().get("a.jpg").cloned().unwrap_or_default();
        assert_eq!(kws, vec!["鸟".to_string(), "天空".to_string()]);
        db.set_keywords("a.jpg", &["新标签".into()]).unwrap();
        let kws = db.all_keywords().unwrap().get("a.jpg").cloned().unwrap_or_default();
        assert_eq!(kws, vec!["新标签".to_string()]);
    }

    #[test]
    fn test_keywords_all_keywords_multi_path() {
        let tmp = TempDir::new().unwrap();
        let db = FolderDb::open_in_dir(tmp.path()).unwrap();
        db.set_keywords("a.jpg", &["鸟".into()]).unwrap();
        db.set_keywords("b.jpg", &["鸟".into(), "天空".into()]).unwrap();
        let all = db.all_keywords().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all.get("a.jpg").unwrap(), &["鸟".to_string()]);
        assert_eq!(all.get("b.jpg").unwrap(), &["鸟".to_string(), "天空".to_string()]);
    }

    #[test]
    fn test_keywords_delete_and_rename() {
        let tmp = TempDir::new().unwrap();
        let db = FolderDb::open_in_dir(tmp.path()).unwrap();
        db.set_keywords("old.jpg", &["鸟".into()]).unwrap();
        db.rename_keywords("old.jpg", "new.jpg").unwrap();
        let all = db.all_keywords().unwrap();
        assert!(all.get("old.jpg").is_none());
        assert_eq!(all.get("new.jpg").unwrap(), &["鸟".to_string()]);
        db.delete_keyword_rows(&["new.jpg".into()]).unwrap();
        assert!(db.all_keywords().unwrap().is_empty());
    }

    #[test]
    fn test_keywords_copy_to_target_db() {
        let tmp = TempDir::new().unwrap();
        let src = FolderDb::open_in_dir(tmp.path()).unwrap();
        let dst_dir = tmp.path().join("dst");
        std::fs::create_dir_all(&dst_dir).unwrap();
        let mut dst = FolderDb::open_in_dir(&dst_dir).unwrap();
        src.set_keywords("a.jpg", &["鸟".into(), "天空".into()]).unwrap();
        src.copy_keywords_to(&mut dst, &[("a.jpg".into(), "b.jpg".into())]).unwrap();
        let got = dst.all_keywords().unwrap().get("b.jpg").cloned().unwrap_or_default();
        assert_eq!(got, vec!["鸟".to_string(), "天空".to_string()]);
        // 源库行保留（复制语义）
        assert!(src.all_keywords().unwrap().contains_key("a.jpg"));
        // 源无关键词的条目不产生空行
        src.copy_keywords_to(&mut dst, &[("nope.jpg".into(), "c.jpg".into())]).unwrap();
        assert!(dst.all_keywords().unwrap().get("c.jpg").is_none());
    }

    #[test]
    fn test_keywords_sync_with_scan_cleanup() {
        // 文件删除后重扫：gone 的关键词行清理，keep 的关键词行保留（真相表随文件走）
        let tmp = TempDir::new().unwrap();
        let db = FolderDb::open_in_dir(tmp.path()).unwrap();
        let keep = tmp.path().join("keep.jpg");
        std::fs::write(&keep, b"fake").unwrap();
        db.set_keywords(&keep.to_string_lossy(), &["鸟".into()]).unwrap();
        let gone = tmp.path().join("gone.jpg");
        std::fs::write(&gone, b"fake").unwrap();
        db.set_keywords(&gone.to_string_lossy(), &["旧标签".into()]).unwrap();
        std::fs::remove_file(&gone).unwrap();

        let (ksize, kmtime) = file_fingerprint(&keep).unwrap();
        let entries = vec![FileEntry {
            full_path: keep.clone(),
            rel_path: "keep.jpg".into(),
            file_size: ksize,
            mtime_ns: kmtime as i64,
            format: ImageFormat::Jpeg,
        }];
        db.sync_with_scan(&entries, &|_, _| {}).unwrap();
        let all = db.all_keywords().unwrap();
        assert!(all.get(&gone.to_string_lossy().to_string()).is_none(), "gone 的关键词行应被清理");
        assert!(all.get(&keep.to_string_lossy().to_string()).is_some(), "keep 的关键词行应保留");
    }

    #[test]
    fn test_keywords_survive_cache_cleanup() {
        // 清理缓存语义：exif_cache 行可清（gone 文件的缓存行被删除），
        // keywords 真相表不得被当作缓存触碰——仍存在文件的关键词行必须保留
        let tmp = TempDir::new().unwrap();
        let db = FolderDb::open_in_dir(tmp.path()).unwrap();
        let f = tmp.path().join("f.jpg");
        std::fs::write(&f, b"fake").unwrap();
        db.put_exif(&f, &make_exif()).unwrap();
        db.set_keywords(&f.to_string_lossy(), &["鸟".into()]).unwrap();
        let gone = tmp.path().join("gone.jpg");
        std::fs::write(&gone, b"fake").unwrap();
        db.put_exif(&gone, &make_exif()).unwrap();
        std::fs::remove_file(&gone).unwrap();

        let (fsize, fmtime) = file_fingerprint(&f).unwrap();
        let entries = vec![FileEntry {
            full_path: f.clone(),
            rel_path: "f.jpg".into(),
            file_size: fsize,
            mtime_ns: fmtime as i64,
            format: ImageFormat::Jpeg,
        }];
        let stats = db.sync_with_scan(&entries, &|_, _| {}).unwrap();
        // gone 的 exif 缓存行被清理；f 的关键词行原样保留
        assert_eq!(stats.cache_deleted, 1);
        let all = db.all_keywords().unwrap();
        assert_eq!(all.get(&f.to_string_lossy().to_string()).unwrap(), &["鸟".to_string()]);
    }

    #[test]
    fn test_keywords_migration_old_db() {
        // 模拟旧版库（user_version = 5，仅到 adjustments 迁移）：打开后应追加 keywords 表。
        // v5 的真实 schema 含 recognition（v3 建表 + 迁移 4 的 eye 两列），链末尾新增的
        // latin_name/cn_level 迁移会 ALTER 它，故这里一并建出（畸形缺表的旧库不在迁移修复范围）。
        let tmp = TempDir::new().unwrap();
        let pt_dir = tmp.path().join(".pt");
        std::fs::create_dir_all(&pt_dir).unwrap();
        let db_path = pt_dir.join("data.db");
        {
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS adjustments (
                    rel_path TEXT PRIMARY KEY,
                    exposure REAL NOT NULL DEFAULT 0,
                    contrast INTEGER NOT NULL DEFAULT 0,
                    saturation INTEGER NOT NULL DEFAULT 0,
                    crop TEXT
                );
                CREATE TABLE IF NOT EXISTS recognition (
                    rel_path    TEXT PRIMARY KEY,
                    status      TEXT NOT NULL,
                    bird_id     INTEGER,
                    bird_name   TEXT,
                    class_index INTEGER,
                    confidence  REAL,
                    bbox        TEXT,
                    candidates  TEXT,
                    failure_stage TEXT,
                    recognized_at TEXT NOT NULL,
                    eye_sharpness REAL,
                    eye_bbox    TEXT
                );",
            )
            .unwrap();
            conn.pragma_update(None, "user_version", 5i64).unwrap();
        }
        let db = FolderDb::open_in_dir(tmp.path()).unwrap();
        db.set_keywords("a.jpg", &["鸟".into()]).unwrap();
        let all = db.all_keywords().unwrap();
        assert_eq!(all.get("a.jpg").unwrap(), &["鸟".to_string()]);
    }

    #[test]
    fn test_sync_with_scan_nested_rel_path_keys() {
        // 递归扫描模式下 FileEntry.rel_path 含子目录分隔符（sub/deep/bird.jpg），
        // 三表键按正斜杠相对路径存储，天然兼容；本测试验证 sync 按嵌套键对齐
        // （保留匹配行、删除孤儿行），证明单层/递归切换不破坏 DB 键约定
        let tmp = TempDir::new().unwrap();
        let db = FolderDb::open_in_dir(tmp.path()).unwrap();
        let rec = make_recognition();
        let params = make_adjustments();
        db.upsert_recognition("sub/deep/bird.jpg", &rec).unwrap();
        db.put_adjustments("sub/deep/bird.jpg", &params).unwrap();
        // 孤儿行：不在本次扫描条目内
        db.upsert_recognition("gone/nested.jpg", &rec).unwrap();

        let f = tmp.path().join("sub").join("deep").join("bird.jpg");
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(&f, b"fake").unwrap();
        let (ksize, kmtime) = file_fingerprint(&f).unwrap();
        let entries = vec![FileEntry {
            full_path: f.clone(),
            rel_path: "sub/deep/bird.jpg".into(),
            file_size: ksize,
            mtime_ns: kmtime as i64,
            format: ImageFormat::Jpeg,
        }];
        let stats = db.sync_with_scan(&entries, &|_, _| {}).unwrap();
        // 匹配嵌套键：无删除；孤儿行被清
        assert_eq!(stats.recognition_deleted, 1);
        assert_eq!(stats.adjustments_deleted, 0);
        assert!(db.get_recognition("sub/deep/bird.jpg").unwrap().is_some());
        assert!(db.get_adjustments("sub/deep/bird.jpg").unwrap().is_some());
        assert!(db.get_recognition("gone/nested.jpg").unwrap().is_none());
    }

    #[test]
    fn test_duplicates_roundtrip_keeper_and_threshold() {
        let tmp = TempDir::new().unwrap();
        let db = FolderDb::open_in_dir(tmp.path()).unwrap();
        for name in ["a.jpg", "b.jpg", "c.jpg"] {
            std::fs::write(tmp.path().join(name), b"x").unwrap();
        }
        let groups = vec![
            vec!["a.jpg".to_string(), "b.jpg".to_string()],
            vec!["c.jpg".to_string()],
        ];
        // 单张不成组的组也别落库（调用方在 UI 侧已过滤；这里按传入原样存，明确定义）
        db.replace_duplicates(&groups, 10, "2026-09-23T10:00:00Z").unwrap();

        let rows = db.load_duplicates().unwrap();
        assert_eq!(rows.len(), 3);
        // 组序 + 组内 keeper 优先
        assert_eq!(rows[0].rel_path, "a.jpg");
        assert!(rows[0].keeper);
        assert_eq!(rows[0].group_index, 0);
        assert_eq!(rows[1].rel_path, "b.jpg");
        assert!(!rows[1].keeper);
        assert_eq!(rows[2].rel_path, "c.jpg");
        assert!(rows[2].keeper);
        assert_eq!(rows[2].group_index, 1);
        assert!(rows.iter().all(|r| r.threshold == 10));
        assert_eq!(rows[0].computed_at, "2026-09-23T10:00:00Z");
    }

    #[test]
    fn test_duplicates_replace_overwrites_and_empty_clears() {
        let tmp = TempDir::new().unwrap();
        let db = FolderDb::open_in_dir(tmp.path()).unwrap();
        for name in ["a.jpg", "b.jpg", "old.jpg"] {
            std::fs::write(tmp.path().join(name), b"x").unwrap();
        }
        db.replace_duplicates(&[vec!["old.jpg".into()]], 8, "t1").unwrap();
        // 重跑检测（新阈值、新分组）→ 旧结果整体消失
        db.replace_duplicates(&[vec!["a.jpg".into(), "b.jpg".into()]], 12, "t2").unwrap();
        let rows = db.load_duplicates().unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.threshold == 12 && r.computed_at == "t2"));
        // 空组 = 清空
        db.replace_duplicates(&[], 12, "t3").unwrap();
        assert!(db.load_duplicates().unwrap().is_empty());
        db.replace_duplicates(&[vec!["a.jpg".into(), "b.jpg".into()]], 12, "t4").unwrap();
        db.clear_duplicates().unwrap();
        assert!(db.load_duplicates().unwrap().is_empty());
    }

    #[test]
    fn test_duplicates_load_drops_missing_files_only() {
        let tmp = TempDir::new().unwrap();
        let db = FolderDb::open_in_dir(tmp.path()).unwrap();
        std::fs::write(tmp.path().join("alive.jpg"), b"x").unwrap();
        // ghost.jpg 从未落盘 → 读回时应被剔除并清行
        db.replace_duplicates(
            &[vec!["alive.jpg".into(), "ghost.jpg".into()]],
            10,
            "t",
        )
        .unwrap();
        let rows = db.load_duplicates().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].rel_path, "alive.jpg");
        // 幽灵行已被清掉（第二次读回同样只有 1 行）
        assert_eq!(db.load_duplicates().unwrap().len(), 1);
    }
}
