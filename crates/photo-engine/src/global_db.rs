//! 全局鸟种索引库（exe 同级 `data/global.db`，跨文件夹汇总）。
//!
//! 与文件夹级 `folder_db.rs`（每目录 `.pt/data.db`）不同，本库聚合所有扫描过的
//! 文件夹的鸟类识别结果，供统计视图（鸟种列表 / 单鸟种照片网格）做全库查询。
//! 便携路径由调用方传入数据目录（photo-tauri 侧为 exe 同级 `data/`），
//! 打开失败不阻塞主流程（调用方降级为 None）。
//!
//! 同步策略（photo-tauri 侧接线，见 lib.rs）：
//! - 扫描完成 → `replace_folder`（当前目录识别行全量替换，幂等；空目录即清空该文件夹行）
//! - 单张识别完成 / 人工修正 → `upsert_rows`
//! - 文件删除 / 移出 → `delete_rows`（或整文件夹 `delete_folder_rows`）

use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;
use rusqlite_migration::{M, Migrations};
use thiserror::Error;

/// 全局索引表迁移（对齐 folder_db.rs 模式：rusqlite_migration 版本化）。
/// 复合主键 (folder, rel_path)：folder = 目录完整路径，rel_path = 正斜杠相对路径
/// （键约定对齐 folder_db recognition 表）。
fn global_migrations() -> Migrations<'static> {
    Migrations::new(vec![
        M::up(
            "CREATE TABLE IF NOT EXISTS species_index (
                folder         TEXT NOT NULL,
                rel_path       TEXT NOT NULL,
                bird_name      TEXT NOT NULL,
                confidence     REAL,
                status         TEXT NOT NULL,
                eye_sharpness  REAL,
                date_taken     TEXT,
                updated_at     TEXT NOT NULL,
                PRIMARY KEY (folder, rel_path)
            );",
        ),
        // 人工修正审计日志（T 批次 Wave 2）：只追加、不删除——species_index 会被
        // 重扫覆盖（replace_folder），原模型预测只能从日志追溯。old_* = 修正前的
        // 模型预测（bird_name + confidence），new_bird = 人工指定值。
        // (folder, rel_path) 索引供 correction_stats 的「每张最新修正」查找。
        M::up(
            "CREATE TABLE IF NOT EXISTS correction_log (
                id             INTEGER PRIMARY KEY AUTOINCREMENT,
                folder         TEXT NOT NULL,
                rel_path       TEXT NOT NULL,
                old_bird       TEXT NOT NULL,
                new_bird       TEXT NOT NULL,
                old_confidence REAL,
                corrected_at   TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_correction_log_photo
                ON correction_log(folder, rel_path);",
        ),
    ])
}

#[derive(Error, Debug)]
pub enum GlobalDbError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Migration error: {0}")]
    Migration(#[from] rusqlite_migration::Error),
}

/// 全局索引单行（与 species_index 表一一对应）。
#[derive(Debug, Clone)]
pub struct SpeciesRow {
    pub folder: String,
    pub rel_path: String,
    pub bird_name: String,
    pub confidence: Option<f64>,
    pub status: String,
    pub eye_sharpness: Option<f64>,
    pub date_taken: Option<String>,
    pub updated_at: String,
}

/// 单鸟种聚合统计（species_stats 返回；avg_sharpness 为 AVG 聚合，自动忽略 NULL）。
#[derive(Debug, Clone, PartialEq)]
pub struct SpeciesStat {
    pub bird_name: String,
    pub photo_count: i64,
    pub first_date: Option<String>,
    pub last_date: Option<String>,
    pub avg_sharpness: Option<f64>,
}

/// 单鸟种识别命中率（correction_stats 返回）。
/// 按 species_index 当前鸟种聚合：predicted = 该鸟种被预测的张数（含未被修正的），
/// corrected_away = 其中被人工改成别种的张数，accuracy = 1 - corrected/predicted。
/// accuracy 恒为 [0,1] 有限值（predicted ≥ 1）；specta 导出 number|null 属防御性
/// 类型（浮点 NaN/Infinity 序列化为 null），前端按 null 兜底处理。
#[derive(Debug, Clone, PartialEq)]
pub struct CorrectionStat {
    pub bird_name: String,
    pub predicted_count: i64,
    pub corrected_away_count: i64,
    pub accuracy: f64,
}

/// 全局鸟种索引库：单连接 + 互斥（对齐 FolderDb 线程模型，全同步）。
#[derive(Clone)]
pub struct GlobalDb {
    conn: Arc<Mutex<rusqlite::Connection>>,
}

impl GlobalDb {
    /// 在指定数据目录打开/创建 `global.db`（便携路径由调用方传入，如 exe 同级 `data/`）。
    /// 目录不存在则自动创建；表结构自动迁移到最新版本。
    pub fn open(data_dir: &Path) -> Result<Self, GlobalDbError> {
        std::fs::create_dir_all(data_dir)?;
        let db_path = data_dir.join("global.db");
        let mut conn = rusqlite::Connection::open(&db_path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
        global_migrations().to_latest(&mut conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// 全量替换某文件夹的索引行（扫描完成时调用；rows 为空 = 清空该文件夹全部行）。
    /// 事务包裹；幂等——重复调用结果一致。
    pub fn replace_folder(&self, folder: &str, rows: &[SpeciesRow]) -> Result<(), GlobalDbError> {
        let conn = self.conn.lock();
        let tx = conn.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM species_index WHERE folder = ?1",
            rusqlite::params![folder],
        )?;
        Self::insert_rows(&tx, rows)?;
        tx.commit()?;
        Ok(())
    }

    /// UPSERT 若干行（单张识别完成 / 人工修正时调用；同键已存在则覆盖）。
    pub fn upsert_rows(&self, rows: &[SpeciesRow]) -> Result<(), GlobalDbError> {
        let conn = self.conn.lock();
        let tx = conn.unchecked_transaction()?;
        Self::insert_rows(&tx, rows)?;
        tx.commit()?;
        Ok(())
    }

    fn insert_rows(
        tx: &rusqlite::Transaction<'_>,
        rows: &[SpeciesRow],
    ) -> Result<(), GlobalDbError> {
        let mut stmt = tx.prepare_cached(
            "INSERT OR REPLACE INTO species_index
             (folder, rel_path, bird_name, confidence, status, eye_sharpness, date_taken, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )?;
        for row in rows {
            stmt.execute(rusqlite::params![
                row.folder,
                row.rel_path,
                row.bird_name,
                row.confidence,
                row.status,
                row.eye_sharpness,
                row.date_taken,
                row.updated_at,
            ])?;
        }
        Ok(())
    }

    /// 删除某文件夹的指定行（文件删除/移出时调用；键 = 复合主键 (folder, rel_path)）。
    /// rel_paths 为空时 no-op。
    pub fn delete_rows(&self, folder: &str, rel_paths: &[String]) -> Result<(), GlobalDbError> {
        if rel_paths.is_empty() {
            return Ok(());
        }
        let conn = self.conn.lock();
        let tx = conn.unchecked_transaction()?;
        {
            let mut stmt =
                tx.prepare_cached("DELETE FROM species_index WHERE folder = ?1 AND rel_path = ?2")?;
            for rel in rel_paths {
                stmt.execute(rusqlite::params![folder, rel])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// 删除某文件夹全部行（整目录移除/重扫时调用）。
    pub fn delete_folder_rows(&self, folder: &str) -> Result<(), GlobalDbError> {
        let conn = self.conn.lock();
        conn.execute(
            "DELETE FROM species_index WHERE folder = ?1",
            rusqlite::params![folder],
        )?;
        Ok(())
    }

    /// 全库聚合统计：按鸟种分组。排序 = 张数降序，同张数按鸟名升序（稳定确定性）。
    /// avg_sharpness 用 AVG 聚合，NULL 自动忽略（无锐度分的照片不进分母）。
    pub fn species_stats(&self) -> Result<Vec<SpeciesStat>, GlobalDbError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare_cached(
            "SELECT bird_name,
                    COUNT(*)            AS photo_count,
                    MIN(date_taken)     AS first_date,
                    MAX(date_taken)     AS last_date,
                    AVG(eye_sharpness)  AS avg_sharpness
             FROM species_index
             GROUP BY bird_name
             ORDER BY photo_count DESC, bird_name ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(SpeciesStat {
                bird_name: row.get(0)?,
                photo_count: row.get(1)?,
                first_date: row.get(2)?,
                last_date: row.get(3)?,
                avg_sharpness: row.get(4)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// 某鸟种全部照片定位（folder, rel_path），按文件夹+路径排序保证确定性。
    pub fn photos_of_species(&self, bird_name: &str) -> Result<Vec<(String, String)>, GlobalDbError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare_cached(
            "SELECT folder, rel_path FROM species_index
             WHERE bird_name = ?1
             ORDER BY folder ASC, rel_path ASC",
        )?;
        let rows = stmt.query_map(rusqlite::params![bird_name], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// 全库覆盖的文件夹数（统计视图汇总条「覆盖文件夹数」）。
    pub fn distinct_folder_count(&self) -> Result<i64, GlobalDbError> {
        let conn = self.conn.lock();
        Ok(conn.query_row(
            "SELECT COUNT(DISTINCT folder) FROM species_index",
            [],
            |row| row.get(0),
        )?)
    }

    /// 追加一条人工修正审计日志（correct_bird 落库时调用；只追加不修改，
    /// 重扫覆盖 species_index 后原模型预测仍可追溯）。corrected_at 取当前时间。
    pub fn log_correction(
        &self,
        folder: &str,
        rel_path: &str,
        old_bird: &str,
        new_bird: &str,
        old_confidence: Option<f64>,
    ) -> Result<(), GlobalDbError> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO correction_log
                 (folder, rel_path, old_bird, new_bird, old_confidence, corrected_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                folder,
                rel_path,
                old_bird,
                new_bird,
                old_confidence,
                chrono::Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    /// 全库识别命中率：按 species_index 当前鸟种聚合。
    ///
    /// 判定「被修正走」：某张 species_index 行的最新一条修正日志（按 id 取每张
    /// 最新）的 new_bird 与该行当前 bird_name 不一致 → 该行被人工改成别种。
    /// 该判定能覆盖「修正后被重扫覆盖」的场景：重扫把模型预测写回 bird_name，
    /// 与最新 new_bird 不一致即命中；未被修正的行无日志，恒为一致。
    ///
    /// 取舍说明（简化方案对比）：更精确的做法是聚合 correction_log.old_bird
    /// （原模型预测）+ 未修正行数，但「未修正行」仍必须靠 species_index 反查，
    /// 无法省掉 LEFT JOIN；且 old_bird 链式修正（A→B→C）会丢失「中间态」。
    /// 本方案以 species_index.bird_name 为预测口径，代价是「被人工改入」的行会
    /// 计入目标鸟种的 predicted（其最新 new_bird 恰等于当前名，判为未修正），
    /// 属可接受的近似——命中率视图定位「模型哪些预测值得复核」，偏保守方向。
    pub fn correction_stats(&self) -> Result<Vec<CorrectionStat>, GlobalDbError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare_cached(
            "SELECT s.bird_name,
                    COUNT(*) AS predicted_count,
                    SUM(CASE WHEN c.latest_new IS NOT NULL
                              AND c.latest_new <> s.bird_name
                             THEN 1 ELSE 0 END) AS corrected_away_count
             FROM species_index s
             LEFT JOIN (
                 SELECT cl.folder, cl.rel_path, cl.new_bird AS latest_new
                 FROM correction_log cl
                 WHERE cl.id = (SELECT MAX(id) FROM correction_log c2
                                WHERE c2.folder = cl.folder AND c2.rel_path = cl.rel_path)
             ) c ON c.folder = s.folder AND c.rel_path = s.rel_path
             GROUP BY s.bird_name
             ORDER BY s.bird_name ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (bird_name, predicted_count, corrected_away_count) = row?;
            let accuracy = if predicted_count > 0 {
                1.0 - corrected_away_count as f64 / predicted_count as f64
            } else {
                1.0
            };
            out.push(CorrectionStat {
                bird_name,
                predicted_count,
                corrected_away_count,
                accuracy,
            });
        }
        Ok(out)
    }

    /// 高频鸟种：species_index 按张数降序的鸟种名（去 NULL/空白），同张数按鸟名
    /// 升序保证确定性。供修正鸟种下拉「常用」分组——本机使用频次即区域相关性代理。
    pub fn frequent_species(&self, limit: usize) -> Result<Vec<String>, GlobalDbError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare_cached(
            "SELECT bird_name
             FROM species_index
             WHERE bird_name IS NOT NULL AND trim(bird_name) <> ''
             GROUP BY bird_name
             ORDER BY COUNT(*) DESC, bird_name ASC
             LIMIT ?1",
        )?;
        let rows = stmt.query_map(rusqlite::params![limit as i64], |row| row.get(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn row(
        folder: &str,
        rel: &str,
        bird: &str,
        conf: Option<f64>,
        status: &str,
        sharp: Option<f64>,
        date: Option<&str>,
    ) -> SpeciesRow {
        SpeciesRow {
            folder: folder.to_string(),
            rel_path: rel.to_string(),
            bird_name: bird.to_string(),
            confidence: conf,
            status: status.to_string(),
            eye_sharpness: sharp,
            date_taken: date.map(|s| s.to_string()),
            updated_at: "2026-08-11T10:00:00Z".to_string(),
        }
    }

    #[test]
    fn test_global_db_open_migrate_reopen() {
        let tmp = TempDir::new().unwrap();
        // 目录不存在时自动创建
        let data_dir = tmp.path().join("data");
        let db = GlobalDb::open(&data_dir).unwrap();
        assert!(data_dir.join("global.db").exists());
        // 迁移幂等：重开不报错、数据保留
        db.replace_folder(
            "E:/A",
            &[row("E:/A", "1.jpg", "白鹭", Some(90.0), "confirmed", Some(42.5), Some("2026-08-01"))],
        )
        .unwrap();
        drop(db);
        let db2 = GlobalDb::open(&data_dir).unwrap();
        assert_eq!(db2.photos_of_species("白鹭").unwrap().len(), 1);
    }

    #[test]
    fn test_global_db_replace_folder_idempotent() {
        let tmp = TempDir::new().unwrap();
        let db = GlobalDb::open(tmp.path()).unwrap();
        let rows = vec![
            row("E:/A", "1.jpg", "白鹭", Some(90.0), "confirmed", Some(42.5), Some("2026-08-01T10:00:00")),
            row("E:/A", "2.jpg", "白鹭", Some(80.0), "confirmed", None, Some("2026-08-02T10:00:00")),
            row("E:/A", "3.jpg", "翠鸟", Some(95.0), "confirmed", Some(30.0), Some("2026-08-03T10:00:00")),
        ];
        db.replace_folder("E:/A", &rows).unwrap();
        // 幂等：重复替换结果一致（先删后插，无累积）
        db.replace_folder("E:/A", &rows).unwrap();
        let stats = db.species_stats().unwrap();
        assert_eq!(stats.len(), 2);
        // 张数降序：白鹭 2 张在前
        assert_eq!(stats[0].bird_name, "白鹭");
        assert_eq!(stats[0].photo_count, 2);
        assert_eq!(stats[1].bird_name, "翠鸟");
        assert_eq!(stats[1].photo_count, 1);
        // 空 rows 替换 = 清空该文件夹（文件被外部删除后重扫场景）
        db.replace_folder("E:/A", &[]).unwrap();
        assert!(db.species_stats().unwrap().is_empty());
    }

    #[test]
    fn test_global_db_upsert_rows() {
        let tmp = TempDir::new().unwrap();
        let db = GlobalDb::open(tmp.path()).unwrap();
        db.upsert_rows(&[row("E:/A", "1.jpg", "白鹭", Some(90.0), "confirmed", Some(42.5), Some("2026-08-01"))])
            .unwrap();
        // 同键再 upsert：覆盖更新（改鸟名/置信度），不新增行
        db.upsert_rows(&[row("E:/A", "1.jpg", "苍鹭", Some(99.0), "confirmed", Some(50.0), Some("2026-08-01"))])
            .unwrap();
        let stats = db.species_stats().unwrap();
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].bird_name, "苍鹭");
        assert_eq!(stats[0].photo_count, 1);
        assert_eq!(stats[0].avg_sharpness, Some(50.0));
        // 新增键：行数增长
        db.upsert_rows(&[row("E:/A", "2.jpg", "苍鹭", Some(80.0), "needs_review", None, Some("2026-08-02"))])
            .unwrap();
        assert_eq!(db.photos_of_species("苍鹭").unwrap().len(), 2);
    }

    #[test]
    fn test_global_db_delete_rows_and_folder() {
        let tmp = TempDir::new().unwrap();
        let db = GlobalDb::open(tmp.path()).unwrap();
        let rows = vec![
            row("E:/A", "1.jpg", "白鹭", None, "confirmed", None, None),
            row("E:/A", "2.jpg", "白鹭", None, "confirmed", None, None),
            row("E:/B", "3.jpg", "白鹭", None, "confirmed", None, None),
        ];
        db.replace_folder("E:/A", &rows[..2]).unwrap();
        db.replace_folder("E:/B", &rows[2..]).unwrap();
        // 删除单行：仅 A/2.jpg 消失，A/1.jpg 与 B/3.jpg 保留
        db.delete_rows("E:/A", &["2.jpg".to_string()]).unwrap();
        let photos = db.photos_of_species("白鹭").unwrap();
        assert_eq!(photos.len(), 2);
        assert!(photos.contains(&("E:/A".to_string(), "1.jpg".to_string())));
        assert!(photos.contains(&("E:/B".to_string(), "3.jpg".to_string())));
        // 空 rel 列表 no-op
        db.delete_rows("E:/A", &[]).unwrap();
        assert_eq!(db.photos_of_species("白鹭").unwrap().len(), 2);
        // 整文件夹删除
        db.delete_folder_rows("E:/A").unwrap();
        let photos = db.photos_of_species("白鹭").unwrap();
        assert_eq!(photos, vec![("E:/B".to_string(), "3.jpg".to_string())]);
    }

    #[test]
    fn test_global_db_stats_avg_ignores_null() {
        let tmp = TempDir::new().unwrap();
        let db = GlobalDb::open(tmp.path()).unwrap();
        // 白鹭：一张有锐度 42.5、一张 NULL → AVG = 42.5（NULL 不占分母）
        db.replace_folder(
            "E:/A",
            &[
                row("E:/A", "1.jpg", "白鹭", None, "confirmed", Some(42.5), Some("2026-08-01")),
                row("E:/A", "2.jpg", "白鹭", None, "confirmed", None, Some("2026-08-03")),
                row("E:/A", "3.jpg", "翠鸟", None, "confirmed", Some(60.0), Some("2026-08-02")),
            ],
        )
        .unwrap();
        let stats = db.species_stats().unwrap();
        assert_eq!(stats.len(), 2);
        let bailu = stats.iter().find(|s| s.bird_name == "白鹭").unwrap();
        assert_eq!(bailu.photo_count, 2);
        assert_eq!(bailu.avg_sharpness, Some(42.5));
        // 首末见日期（TEXT MIN/MAX）
        assert_eq!(bailu.first_date.as_deref(), Some("2026-08-01"));
        assert_eq!(bailu.last_date.as_deref(), Some("2026-08-03"));
        // 无锐度数据的鸟种：avg 为 NULL（不是 0）
        let cui = stats.iter().find(|s| s.bird_name == "翠鸟").unwrap();
        assert_eq!(cui.avg_sharpness, Some(60.0));
    }

    #[test]
    fn test_global_db_stats_sort_by_count_desc() {
        let tmp = TempDir::new().unwrap();
        let db = GlobalDb::open(tmp.path()).unwrap();
        db.replace_folder(
            "E:/A",
            &[
                row("E:/A", "1.jpg", "麻雀", None, "confirmed", None, None),
                row("E:/A", "2.jpg", "麻雀", None, "confirmed", None, None),
                row("E:/A", "3.jpg", "麻雀", None, "confirmed", None, None),
                row("E:/A", "4.jpg", "白鹭", None, "confirmed", None, None),
            ],
        )
        .unwrap();
        let stats = db.species_stats().unwrap();
        // 张数降序：麻雀 3 张在前
        assert_eq!(stats[0].bird_name, "麻雀");
        assert_eq!(stats[0].photo_count, 3);
        assert_eq!(stats[1].bird_name, "白鹭");
        assert_eq!(stats[1].photo_count, 1);
    }

    #[test]
    fn test_global_db_photos_of_species() {
        let tmp = TempDir::new().unwrap();
        let db = GlobalDb::open(tmp.path()).unwrap();
        let rows = vec![
            row("E:/A", "1.jpg", "白鹭", None, "confirmed", None, None),
            row("E:/A", "2.jpg", "翠鸟", None, "confirmed", None, None),
            row("E:/B", "3.jpg", "白鹭", None, "confirmed", None, None),
            row("E:/B", "4.jpg", "白鹭", None, "confirmed", None, None),
        ];
        db.replace_folder("E:/A", &rows[..2]).unwrap();
        db.replace_folder("E:/B", &rows[2..]).unwrap();
        let photos = db.photos_of_species("白鹭").unwrap();
        assert_eq!(
            photos,
            vec![
                ("E:/A".to_string(), "1.jpg".to_string()),
                ("E:/B".to_string(), "3.jpg".to_string()),
                ("E:/B".to_string(), "4.jpg".to_string()),
            ]
        );
        // 不存在的鸟种返回空
        assert!(db.photos_of_species("不存在").unwrap().is_empty());
    }

    #[test]
    fn test_global_db_distinct_folder_count() {
        let tmp = TempDir::new().unwrap();
        let db = GlobalDb::open(tmp.path()).unwrap();
        assert_eq!(db.distinct_folder_count().unwrap(), 0);
        db.replace_folder(
            "E:/A",
            &[
                row("E:/A", "1.jpg", "白鹭", None, "confirmed", None, None),
                row("E:/A", "2.jpg", "翠鸟", None, "confirmed", None, None),
            ],
        )
        .unwrap();
        db.replace_folder(
            "E:/B",
            &[row("E:/B", "3.jpg", "白鹭", None, "confirmed", None, None)],
        )
        .unwrap();
        assert_eq!(db.distinct_folder_count().unwrap(), 2);
        db.delete_folder_rows("E:/A").unwrap();
        assert_eq!(db.distinct_folder_count().unwrap(), 1);
    }

    #[test]
    fn test_global_db_log_correction_and_stats() {
        let tmp = TempDir::new().unwrap();
        let db = GlobalDb::open(tmp.path()).unwrap();
        // 1.jpg 白鹭：从未修正（未被修正计数）
        // 2.jpg 苍鹭：模型预测白鹭 → 人工改为苍鹭（species_index 已是苍鹭）
        // 3.jpg 白鹭：模型预测白鹭 → 人工改苍鹭 → 重扫覆盖回白鹭（判定被修正走）
        // 4.jpg 翠鸟：从未修正
        db.upsert_rows(&[
            row("E:/A", "1.jpg", "白鹭", Some(90.0), "confirmed", None, None),
            row("E:/A", "2.jpg", "苍鹭", Some(100.0), "confirmed", None, None),
            row("E:/A", "3.jpg", "白鹭", Some(85.0), "confirmed", None, None),
            row("E:/A", "4.jpg", "翠鸟", Some(95.0), "confirmed", None, None),
        ])
        .unwrap();
        db.log_correction("E:/A", "2.jpg", "白鹭", "苍鹭", Some(90.0)).unwrap();
        db.log_correction("E:/A", "3.jpg", "白鹭", "苍鹭", Some(85.0)).unwrap();

        let stats = db.correction_stats().unwrap();
        let bailu = stats.iter().find(|s| s.bird_name == "白鹭").unwrap();
        // predicted = 当前名为白鹭的行数（1.jpg + 3.jpg）= 2；3.jpg 被修正走 → 1
        assert_eq!(bailu.predicted_count, 2);
        assert_eq!(bailu.corrected_away_count, 1);
        assert!((bailu.accuracy - 0.5).abs() < 1e-9);
        // 苍鹭：被人工改入，最新 new_bird == 当前名 → 判未修正
        let cang = stats.iter().find(|s| s.bird_name == "苍鹭").unwrap();
        assert_eq!(cang.predicted_count, 1);
        assert_eq!(cang.corrected_away_count, 0);
        assert_eq!(cang.accuracy, 1.0);
        // 翠鸟：从未修正，全计数保留
        let cui = stats.iter().find(|s| s.bird_name == "翠鸟").unwrap();
        assert_eq!(cui.predicted_count, 1);
        assert_eq!(cui.corrected_away_count, 0);
        assert_eq!(cui.accuracy, 1.0);
    }

    #[test]
    fn test_global_db_correction_stats_chain_latest_wins() {
        let tmp = TempDir::new().unwrap();
        let db = GlobalDb::open(tmp.path()).unwrap();
        // 链式修正 麻雀 → 白鹭 → 山斑鸠：只取最新一条（new_bird = 山斑鸠 == 当前名）
        db.upsert_rows(&[row("E:/A", "5.jpg", "山斑鸠", Some(100.0), "confirmed", None, None)])
            .unwrap();
        db.log_correction("E:/A", "5.jpg", "麻雀", "白鹭", Some(70.0)).unwrap();
        db.log_correction("E:/A", "5.jpg", "白鹭", "山斑鸠", Some(100.0)).unwrap();
        let stats = db.correction_stats().unwrap();
        // 取舍说明：被改入的行计入目标鸟种 predicted（最新 new_bird == 当前名 → 未修正）
        let shan = stats.iter().find(|s| s.bird_name == "山斑鸠").unwrap();
        assert_eq!(shan.predicted_count, 1);
        assert_eq!(shan.corrected_away_count, 0);
        // 麻雀在 species_index 中已无行 → 不参与聚合（原预测只能从日志反查，属已知近似）
        assert!(stats.iter().all(|s| s.bird_name != "麻雀"));
    }

    #[test]
    fn test_global_db_correction_stats_accuracy_boundaries() {
        let tmp = TempDir::new().unwrap();
        let db = GlobalDb::open(tmp.path()).unwrap();
        // 全对：白鹭 3 张从未修正 → accuracy 1.0
        // 全被改：麻雀 2 张均被改成白鹭 → accuracy 0.0
        db.upsert_rows(&[
            row("E:/A", "1.jpg", "白鹭", Some(90.0), "confirmed", None, None),
            row("E:/A", "2.jpg", "白鹭", Some(80.0), "confirmed", None, None),
            row("E:/A", "3.jpg", "白鹭", Some(70.0), "confirmed", None, None),
            row("E:/A", "4.jpg", "麻雀", Some(60.0), "confirmed", None, None),
            row("E:/A", "5.jpg", "麻雀", Some(55.0), "confirmed", None, None),
        ])
        .unwrap();
        db.log_correction("E:/A", "4.jpg", "麻雀", "白鹭", Some(60.0)).unwrap();
        db.log_correction("E:/A", "5.jpg", "麻雀", "白鹭", Some(55.0)).unwrap();

        let stats = db.correction_stats().unwrap();
        let bailu = stats.iter().find(|s| s.bird_name == "白鹭").unwrap();
        assert_eq!(bailu.predicted_count, 3);
        assert_eq!(bailu.corrected_away_count, 0);
        assert_eq!(bailu.accuracy, 1.0);
        let maque = stats.iter().find(|s| s.bird_name == "麻雀").unwrap();
        assert_eq!(maque.predicted_count, 2);
        assert_eq!(maque.corrected_away_count, 2);
        assert_eq!(maque.accuracy, 0.0);
    }

    #[test]
    fn test_global_db_frequent_species() {
        let tmp = TempDir::new().unwrap();
        let db = GlobalDb::open(tmp.path()).unwrap();
        // 白鹭 3 张、翠鸟 2 张、麻雀 1 张、空白名 1 张（trim 过滤）
        db.replace_folder(
            "E:/A",
            &[
                row("E:/A", "1.jpg", "白鹭", None, "confirmed", None, None),
                row("E:/A", "2.jpg", "白鹭", None, "confirmed", None, None),
                row("E:/A", "3.jpg", "白鹭", None, "confirmed", None, None),
                row("E:/A", "4.jpg", "翠鸟", None, "confirmed", None, None),
                row("E:/A", "5.jpg", "翠鸟", None, "confirmed", None, None),
                row("E:/A", "6.jpg", "麻雀", None, "confirmed", None, None),
                row("E:/A", "7.jpg", "", None, "confirmed", None, None),
            ],
        )
        .unwrap();
        // 张数降序：白鹭在前；空名被排除
        let all = db.frequent_species(10).unwrap();
        assert_eq!(all, vec!["白鹭", "翠鸟", "麻雀"]);
        // limit 截断
        let top = db.frequent_species(2).unwrap();
        assert_eq!(top, vec!["白鹭", "翠鸟"]);
        // 同张数按鸟名升序（确定性）
        db.upsert_rows(&[row("E:/B", "8.jpg", "麻雀", None, "confirmed", None, None)])
            .unwrap();
        let tie = db.frequent_species(10).unwrap();
        assert_eq!(tie, vec!["白鹭", "翠鸟", "麻雀"]);
    }
}
