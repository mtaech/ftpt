//! 全局鸟种索引库（exe 同级 `data/global.db`，跨文件夹汇总）。
//!
//! 与文件夹级 `folder_db.rs`（每目录 `.pt/data.db`）不同，本库聚合所有扫描过的
//! 文件夹的鸟类识别结果，供统计视图（鸟种列表 / 单鸟种照片网格）做全库查询。
//! 便携路径由调用方传入数据目录（`photo-ui` 侧为 exe 同级 `data/`），
//! 打开失败不阻塞主流程（调用方降级为 None）。
//!
//! 同步策略（`photo-ui` 侧接线，见 `crates/photo-ui/src/state/engine_ops.rs`）：
//! - 扫描完成 → `replace_folder`（当前目录识别行全量替换，幂等；空目录即清空该文件夹行）
//! - 单张识别完成 / 人工修正 → `upsert_rows`
//! - 文件删除 / 移出 → `delete_rows`（或整文件夹 `delete_folder_rows`）

use std::collections::HashSet;
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
        // 人工修正审计日志（T 批次 Wave 2 历史建表）：随修正对话框一起下线（2026-09-22），
        // 表在链末尾的迁移里 DROP。保留历史建表语句不动，避免打乱既有库的 user_version。
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
        // 鸟眼锐度与人工修正审计一起下线（2026-09-22：锐度阶段删除、修正对话框删除）。
        // 历史建表留在链中间不动（删改序号会破坏既有库的 user_version 记账），末尾收敛。
        M::up(
            "DROP TABLE IF EXISTS correction_log;
             ALTER TABLE species_index DROP COLUMN eye_sharpness;",
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
    pub date_taken: Option<String>,
    pub updated_at: String,
}

/// 单鸟种聚合统计（species_stats 返回）。
#[derive(Debug, Clone, PartialEq)]
pub struct SpeciesStat {
    pub bird_name: String,
    pub photo_count: i64,
    pub first_date: Option<String>,
    pub last_date: Option<String>,
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

    /// 以本次扫描结果替换某文件夹的索引行（扫描完成时调用）。
    /// 只删除「本次未出现 **且磁盘上确实不存在**」的行：单层扫描时子目录照片不在
    /// rows 里但文件真实存在，整批清空会丢掉子目录索引（切换扫描深度时常见）。
    /// 事务包裹；幂等——重复调用结果一致。
    pub fn replace_folder(&self, folder: &str, rows: &[SpeciesRow]) -> Result<(), GlobalDbError> {
        let conn = self.conn.lock();
        let tx = conn.unchecked_transaction()?;
        let new_rels: HashSet<&str> = rows.iter().map(|r| r.rel_path.as_str()).collect();
        let existing: Vec<String> = {
            let mut stmt = tx.prepare("SELECT rel_path FROM species_index WHERE folder = ?1")?;
            let iter = stmt.query_map(rusqlite::params![folder], |row| row.get::<_, String>(0))?;
            iter.filter_map(|r| r.ok()).collect()
        };
        for rel in existing {
            if new_rels.contains(rel.as_str()) {
                continue;
            }
            if !Path::new(folder).join(&rel).exists() {
                tx.execute(
                    "DELETE FROM species_index WHERE folder = ?1 AND rel_path = ?2",
                    rusqlite::params![folder, rel],
                )?;
            }
        }
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
             (folder, rel_path, bird_name, confidence, status, date_taken, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;
        for row in rows {
            stmt.execute(rusqlite::params![
                row.folder,
                row.rel_path,
                row.bird_name,
                row.confidence,
                row.status,
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
    pub fn species_stats(&self) -> Result<Vec<SpeciesStat>, GlobalDbError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare_cached(
            "SELECT bird_name,
                    COUNT(*)            AS photo_count,
                    MIN(date_taken)     AS first_date,
                    MAX(date_taken)     AS last_date
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
        date: Option<&str>,
    ) -> SpeciesRow {
        SpeciesRow {
            folder: folder.to_string(),
            rel_path: rel.to_string(),
            bird_name: bird.to_string(),
            confidence: conf,
            status: status.to_string(),
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
            &[row("E:/A", "1.jpg", "白鹭", Some(90.0), "confirmed", Some("2026-08-01"))],
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
            row("E:/A", "1.jpg", "白鹭", Some(90.0), "confirmed", Some("2026-08-01T10:00:00")),
            row("E:/A", "2.jpg", "白鹭", Some(80.0), "confirmed", Some("2026-08-02T10:00:00")),
            row("E:/A", "3.jpg", "翠鸟", Some(95.0), "confirmed", Some("2026-08-03T10:00:00")),
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
    fn test_global_db_replace_folder_keeps_existing_unscanned_rows() {
        let tmp = TempDir::new().unwrap();
        let db = GlobalDb::open(tmp.path()).unwrap();
        let folder = tmp.path().to_string_lossy().to_string();
        // 子目录文件真实存在：模拟「包含子目录」扫描入库后切回单层扫描
        let sub = tmp.path().join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("bird.jpg"), b"x").unwrap();

        db.replace_folder(
            &folder,
            &[row(&folder, "sub/bird.jpg", "白鹭", None, "confirmed", None)],
        )
        .unwrap();
        // 单层扫描：rows 为空但文件仍在磁盘 → 索引行保留
        db.replace_folder(&folder, &[]).unwrap();
        assert_eq!(db.photos_of_species("白鹭").unwrap().len(), 1);

        // 文件被外部删除后重扫：行才被清掉
        std::fs::remove_file(sub.join("bird.jpg")).unwrap();
        db.replace_folder(&folder, &[]).unwrap();
        assert!(db.photos_of_species("白鹭").unwrap().is_empty());
    }

    #[test]
    fn test_global_db_upsert_rows() {
        let tmp = TempDir::new().unwrap();
        let db = GlobalDb::open(tmp.path()).unwrap();
        db.upsert_rows(&[row("E:/A", "1.jpg", "白鹭", Some(90.0), "confirmed", Some("2026-08-01"))])
            .unwrap();
        // 同键再 upsert：覆盖更新（改鸟名/置信度），不新增行
        db.upsert_rows(&[row("E:/A", "1.jpg", "苍鹭", Some(99.0), "confirmed", Some("2026-08-01"))])
            .unwrap();
        let stats = db.species_stats().unwrap();
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].bird_name, "苍鹭");
        assert_eq!(stats[0].photo_count, 1);
        // 新增键：行数增长
        db.upsert_rows(&[row("E:/A", "2.jpg", "苍鹭", Some(80.0), "needs_review", Some("2026-08-02"))])
            .unwrap();
        assert_eq!(db.photos_of_species("苍鹭").unwrap().len(), 2);
    }

    #[test]
    fn test_global_db_delete_rows_and_folder() {
        let tmp = TempDir::new().unwrap();
        let db = GlobalDb::open(tmp.path()).unwrap();
        let rows = vec![
            row("E:/A", "1.jpg", "白鹭", None, "confirmed", None),
            row("E:/A", "2.jpg", "白鹭", None, "confirmed", None),
            row("E:/B", "3.jpg", "白鹭", None, "confirmed", None),
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
    fn test_global_db_stats_first_last_date() {
        let tmp = TempDir::new().unwrap();
        let db = GlobalDb::open(tmp.path()).unwrap();
        db.replace_folder(
            "E:/A",
            &[
                row("E:/A", "1.jpg", "白鹭", None, "confirmed", Some("2026-08-01")),
                row("E:/A", "2.jpg", "白鹭", None, "confirmed", Some("2026-08-03")),
                row("E:/A", "3.jpg", "翠鸟", None, "confirmed", Some("2026-08-02")),
            ],
        )
        .unwrap();
        let stats = db.species_stats().unwrap();
        assert_eq!(stats.len(), 2);
        let bailu = stats.iter().find(|s| s.bird_name == "白鹭").unwrap();
        assert_eq!(bailu.photo_count, 2);
        // 首末见日期（TEXT MIN/MAX）
        assert_eq!(bailu.first_date.as_deref(), Some("2026-08-01"));
        assert_eq!(bailu.last_date.as_deref(), Some("2026-08-03"));
    }

    #[test]
    fn test_global_db_stats_sort_by_count_desc() {
        let tmp = TempDir::new().unwrap();
        let db = GlobalDb::open(tmp.path()).unwrap();
        db.replace_folder(
            "E:/A",
            &[
                row("E:/A", "1.jpg", "麻雀", None, "confirmed", None),
                row("E:/A", "2.jpg", "麻雀", None, "confirmed", None),
                row("E:/A", "3.jpg", "麻雀", None, "confirmed", None),
                row("E:/A", "4.jpg", "白鹭", None, "confirmed", None),
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
            row("E:/A", "1.jpg", "白鹭", None, "confirmed", None),
            row("E:/A", "2.jpg", "翠鸟", None, "confirmed", None),
            row("E:/B", "3.jpg", "白鹭", None, "confirmed", None),
            row("E:/B", "4.jpg", "白鹭", None, "confirmed", None),
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
                row("E:/A", "1.jpg", "白鹭", None, "confirmed", None),
                row("E:/A", "2.jpg", "翠鸟", None, "confirmed", None),
            ],
        )
        .unwrap();
        db.replace_folder(
            "E:/B",
            &[row("E:/B", "3.jpg", "白鹭", None, "confirmed", None)],
        )
        .unwrap();
        assert_eq!(db.distinct_folder_count().unwrap(), 2);
        db.delete_folder_rows("E:/A").unwrap();
        assert_eq!(db.distinct_folder_count().unwrap(), 1);
    }

}
