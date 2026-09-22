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
use photo_domain::Recognition;
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
        // 多主体索引（2026-09-22 ADR 0011 落地）：索引行从「一张照片一行」扩展为
        // 「每主体一行」，主键加 subject_index；bird_name 列随泛化更名为 species_name。
        // global.db 是派生索引（删除后重扫自动重建，见 crate 文档），直接重建表换主键
        // 最稳妥——旧库数据无保留价值，避免 ALTER 主键的繁琐与风险。
        M::up(
            "DROP TABLE IF EXISTS species_index;
             CREATE TABLE species_index (
                 folder        TEXT NOT NULL,
                 rel_path      TEXT NOT NULL,
                 subject_index INTEGER NOT NULL,
                 species_name  TEXT NOT NULL,
                 confidence    REAL,
                 status        TEXT NOT NULL,
                 date_taken    TEXT,
                 updated_at    TEXT NOT NULL,
                 PRIMARY KEY (folder, rel_path, subject_index)
             );",
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

/// 全局索引单行（与 species_index 表一一对应；一张照片的多主体展开为多行）。
#[derive(Debug, Clone)]
pub struct SpeciesRow {
    pub folder: String,
    pub rel_path: String,
    /// 主体序号（0 = 主主体；旧数据/单主体恒为 0）
    pub subject_index: i32,
    /// 物种展示名（有中文名用中文名，否则学名）
    pub species_name: String,
    pub confidence: Option<f64>,
    pub status: String,
    pub date_taken: Option<String>,
    pub updated_at: String,
}

impl SpeciesRow {
    /// 从一条识别记录展开出全局索引行：每个「有物种结论」的主体一行
    /// （subject_index 展开）。旧数据（subjects 空）以顶层 taxon 退化为单主体；
    /// 无任何结论（Unrecognized / 全部主体失败）返回空 Vec。
    pub fn from_recognition(
        folder: &str,
        rel_path: &str,
        rec: &Recognition,
        date_taken: Option<&str>,
    ) -> Vec<SpeciesRow> {
        let mk = |subject_index: i32, name: &str, confidence: Option<f32>| SpeciesRow {
            folder: folder.to_string(),
            rel_path: rel_path.to_string(),
            subject_index,
            species_name: name.to_string(),
            confidence: confidence.map(|c| c as f64),
            status: rec.status.as_str().to_string(),
            date_taken: date_taken.map(|s| s.to_string()),
            updated_at: rec.recognized_at.clone(),
        };
        if rec.subjects.is_empty() {
            // 旧数据 / 单主体：顶层 taxon 兜底
            match rec.taxon.as_ref() {
                Some(t) if !t.display_name().is_empty() => vec![mk(0, t.display_name(), rec.confidence)],
                _ => Vec::new(),
            }
        } else {
            rec.subjects
                .iter()
                .enumerate()
                .filter_map(|(i, s)| {
                    let t = s.taxon.as_ref()?;
                    let name = t.display_name();
                    if name.is_empty() { return None; }
                    Some(mk(i as i32, name, s.confidence))
                })
                .collect()
        }
    }
}

/// 单鸟种聚合统计（species_stats 返回）。
#[derive(Debug, Clone, PartialEq)]
pub struct SpeciesStat {
    pub species_name: String,
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
    /// 多主体语义：同 rel_path 可有多个 subject_index 行。
    /// - 本次扫到的照片（rel_path 在 rows 里）：只保留新主体集合中的行，
    ///   旧主体行清掉（重新识别后主体集合可能变化）。
    /// - 本次没扫到的照片（单层扫描时子目录照片不在 rows 里但文件真实存在）：
    ///   磁盘上存在就整行保留，否则删除（文件被外部删除后重扫）。
    /// 事务包裹；幂等——重复调用结果一致。
    pub fn replace_folder(&self, folder: &str, rows: &[SpeciesRow]) -> Result<(), GlobalDbError> {
        let conn = self.conn.lock();
        let tx = conn.unchecked_transaction()?;
        let new_rels: HashSet<&str> = rows.iter().map(|r| r.rel_path.as_str()).collect();
        let new_keys: HashSet<(String, i32)> = rows
            .iter()
            .map(|r| (r.rel_path.clone(), r.subject_index))
            .collect();
        let existing: Vec<(String, i32)> = {
            let mut stmt =
                tx.prepare("SELECT rel_path, subject_index FROM species_index WHERE folder = ?1")?;
            let iter = stmt.query_map(rusqlite::params![folder], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i32>(1)?))
            })?;
            iter.filter_map(|r| r.ok()).collect()
        };
        for (rel, subj) in existing {
            let keep = if new_rels.contains(rel.as_str()) {
                // 本次扫到了这张照片：主体集合以新 rows 为准
                new_keys.contains(&(rel.clone(), subj))
            } else {
                // 本次没扫到（子目录照片等）：磁盘上存在就保留
                Path::new(folder).join(&rel).exists()
            };
            if !keep {
                tx.execute(
                    "DELETE FROM species_index WHERE folder = ?1 AND rel_path = ?2 AND subject_index = ?3",
                    rusqlite::params![folder, rel, subj],
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
             (folder, rel_path, subject_index, species_name, confidence, status, date_taken, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )?;
        for row in rows {
            stmt.execute(rusqlite::params![
                row.folder,
                row.rel_path,
                row.subject_index,
                row.species_name,
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
            "SELECT species_name,
                    COUNT(*)            AS record_count,
                    MIN(date_taken)     AS first_date,
                    MAX(date_taken)     AS last_date
             FROM species_index
             GROUP BY species_name
             ORDER BY record_count DESC, species_name ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(SpeciesStat {
                species_name: row.get(0)?,
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
    pub fn photos_of_species(&self, species_name: &str) -> Result<Vec<(String, String)>, GlobalDbError> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare_cached(
            "SELECT DISTINCT folder, rel_path FROM species_index
             WHERE species_name = ?1
             ORDER BY folder ASC, rel_path ASC",
        )?;
        let rows = stmt.query_map(rusqlite::params![species_name], |row| {
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
    use photo_domain::{
        BBox, CnLevel, RecognitionStatus, SubjectRecognition, TaxonMatch,
    };
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
            subject_index: 0,
            species_name: bird.to_string(),
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
        assert_eq!(stats[0].species_name, "白鹭");
        assert_eq!(stats[0].photo_count, 2);
        assert_eq!(stats[1].species_name, "翠鸟");
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
        assert_eq!(stats[0].species_name, "苍鹭");
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
        let bailu = stats.iter().find(|s| s.species_name == "白鹭").unwrap();
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
        assert_eq!(stats[0].species_name, "麻雀");
        assert_eq!(stats[0].photo_count, 3);
        assert_eq!(stats[1].species_name, "白鹭");
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

    // ── 多主体：from_recognition 展开 / replace_folder 主体集合收敛 ──

    fn taxon(cn: &str, latin: &str) -> TaxonMatch {
        TaxonMatch {
            taxon_id: None,
            cn_name: cn.to_string(),
            latin_name: latin.to_string(),
            cn_level: CnLevel::Species,
            ranks: vec![],
        }
    }

    fn subject(index: u32, cn: &str, latin: &str, conf: Option<f32>) -> SubjectRecognition {
        SubjectRecognition {
            index,
            bbox: BBox::new(0.1, 0.1, 0.5, 0.5),
            taxon: Some(taxon(cn, latin)),
            class_index: Some(0),
            confidence: conf,
            candidates: vec![],
            failure: photo_domain::RecognitionFailureStage::None,
        }
    }

    /// 多主体识别记录（顶层 = 主主体 subjects[0]）。
    fn rec(subjects: Vec<SubjectRecognition>) -> Recognition {
        let primary = subjects.first();
        Recognition {
            status: RecognitionStatus::Confirmed,
            taxon: primary.and_then(|s| s.taxon.clone()),
            class_index: primary.and_then(|s| s.class_index),
            confidence: primary.and_then(|s| s.confidence),
            bbox: primary.map(|s| s.bbox),
            candidates: vec![],
            failure_stage: photo_domain::RecognitionFailureStage::None,
            recognized_at: "2026-09-22T10:00:00Z".to_string(),
            subjects,
        }
    }

    #[test]
    fn test_global_db_from_recognition_expands_multi_subject() {
        let rows = SpeciesRow::from_recognition(
            "E:/A",
            "1.jpg",
            &rec(vec![
                subject(0, "长耳鸮", "Asio otus", Some(71.0)),
                subject(1, "粉褶蕈属", "Entoloma", Some(60.0)),
            ]),
            Some("2026-08-01"),
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].subject_index, 0);
        assert_eq!(rows[0].species_name, "长耳鸮");
        assert_eq!(rows[0].confidence, Some(71.0));
        assert_eq!(rows[0].status, "confirmed");
        assert_eq!(rows[0].date_taken.as_deref(), Some("2026-08-01"));
        assert_eq!(rows[1].subject_index, 1);
        assert_eq!(rows[1].species_name, "粉褶蕈属");
        assert_eq!(rows[1].confidence, Some(60.0));
    }

    #[test]
    fn test_global_db_from_recognition_legacy_single() {
        // 旧数据：subjects 空，以顶层 taxon 退化为单主体
        let rec = Recognition {
            status: RecognitionStatus::Confirmed,
            taxon: Some(taxon("白鹭", "Egretta garzetta")),
            class_index: Some(1),
            confidence: Some(90.0),
            bbox: Some(BBox::new(0.1, 0.1, 0.5, 0.5)),
            candidates: vec![],
            failure_stage: photo_domain::RecognitionFailureStage::None,
            recognized_at: "2026-09-22T10:00:00Z".to_string(),
            subjects: vec![],
        };
        let rows = SpeciesRow::from_recognition("E:/A", "legacy.jpg", &rec, None);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].subject_index, 0);
        assert_eq!(rows[0].species_name, "白鹭");
        assert_eq!(rows[0].confidence, Some(90.0));
    }

    #[test]
    fn test_global_db_from_recognition_unrecognized_empty() {
        let rec = Recognition {
            status: RecognitionStatus::Unrecognized,
            taxon: None,
            class_index: None,
            confidence: None,
            bbox: None,
            candidates: vec![],
            failure_stage: photo_domain::RecognitionFailureStage::Detection,
            recognized_at: "2026-09-22T10:00:00Z".to_string(),
            subjects: vec![],
        };
        assert!(SpeciesRow::from_recognition("E:/A", "none.jpg", &rec, None).is_empty());
    }

    #[test]
    fn test_global_db_replace_folder_multi_subject_converges() {
        let tmp = TempDir::new().unwrap();
        let db = GlobalDb::open(tmp.path()).unwrap();
        // 一张照片两个主体 → 两行
        let mut r1 = row("E:/A", "1.jpg", "长耳鸮", Some(71.0), "confirmed", Some("2026-08-01"));
        r1.subject_index = 0;
        let mut r2 = row("E:/A", "1.jpg", "粉褶蕈属", Some(60.0), "confirmed", Some("2026-08-01"));
        r2.subject_index = 1;
        db.replace_folder("E:/A", &[r1, r2]).unwrap();
        let stats = db.species_stats().unwrap();
        assert_eq!(stats.len(), 2);
        // 重新识别后只剩一个主体：旧主体行被清掉，不残留
        db.replace_folder(
            "E:/A",
            &[row("E:/A", "1.jpg", "长耳鸮", Some(80.0), "confirmed", Some("2026-08-01"))],
        )
        .unwrap();
        let stats = db.species_stats().unwrap();
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].species_name, "长耳鸮");
        assert_eq!(stats[0].photo_count, 1);
    }

    /// 模拟 engine_ops 扫描完成后的全局索引同步链路：
    /// folder_db 识别行 → from_recognition 展开（多主体）→ replace_folder → 统计可读。
    #[test]
    fn test_global_db_sync_from_folder_db_scan_path() {
        use crate::folder_db::FolderDb;
        let tmp = TempDir::new().unwrap();
        let photo_dir = tmp.path().join("photos");
        std::fs::create_dir_all(&photo_dir).unwrap();
        // 真实文件（replace_folder 的磁盘存在性判定需要）
        std::fs::write(photo_dir.join("multi.jpg"), b"x").unwrap();
        std::fs::write(photo_dir.join("single.jpg"), b"x").unwrap();

        let fdb = FolderDb::open_in_dir(&photo_dir).unwrap();
        // 多主体照片
        fdb.upsert_recognition(
            "multi.jpg",
            &rec(vec![
                subject(0, "长耳鸮", "Asio otus", Some(71.0)),
                subject(1, "粉褶蕈属", "Entoloma", Some(60.0)),
            ]),
        )
        .unwrap();
        // 单主体照片（subjects 有 1 个元素）
        fdb.upsert_recognition(
            "single.jpg",
            &rec(vec![subject(0, "白鹭", "Egretta garzetta", Some(90.0))]),
        )
        .unwrap();

        // engine_ops::do_background_scan 的全局同步段：
        // for (rel, rec) in &recs { rows.extend(from_recognition(...)) }; replace_folder
        let folder_str = photo_dir.to_string_lossy().to_string();
        let gdb = GlobalDb::open(tmp.path()).unwrap();
        let recs = fdb.all_recognitions().unwrap();
        let rows: Vec<SpeciesRow> = recs
            .iter()
            .flat_map(|(rel, rec)| SpeciesRow::from_recognition(&folder_str, rel, rec, None))
            .collect();
        gdb.replace_folder(&folder_str, &rows).unwrap();

        let stats = gdb.species_stats().unwrap();
        assert_eq!(stats.len(), 3);
        // 多主体展开：长耳鸮 1 条、粉褶蕈属 1 条、白鹭 1 条
        let names: Vec<&str> = stats.iter().map(|s| s.species_name.as_str()).collect();
        assert!(names.contains(&"长耳鸮"));
        assert!(names.contains(&"粉褶蕈属"));
        assert!(names.contains(&"白鹭"));
        // 多主体照片的照片列表：multi.jpg 出现 1 次（DISTINCT 按 rel_path 去重）
        let photos = gdb.photos_of_species("长耳鸮").unwrap();
        assert_eq!(photos, vec![(folder_str.clone(), "multi.jpg".to_string())]);

        // ── 场景 2：single.jpg 重识别为 Unrecognized → 旧索引行必须被清掉 ──
        // （engine_ops 对无结论的行走 delete_rows；replace_folder 的磁盘存在判定会留着旧行）
        fdb.upsert_recognition(
            "single.jpg",
            &Recognition {
                status: RecognitionStatus::Unrecognized,
                taxon: None,
                class_index: None,
                confidence: None,
                bbox: None,
                candidates: vec![],
                failure_stage: photo_domain::RecognitionFailureStage::Detection,
                recognized_at: "2026-09-22T11:00:00Z".to_string(),
                subjects: vec![],
            },
        )
        .unwrap();
        let recs = fdb.all_recognitions().unwrap();
        let mut rows = Vec::new();
        let mut stale = Vec::new();
        for (rel, rec) in &recs {
            let new_rows = SpeciesRow::from_recognition(&folder_str, rel, rec, None);
            if new_rows.is_empty() {
                stale.push(rel.clone());
            }
            rows.extend(new_rows);
        }
        gdb.replace_folder(&folder_str, &rows).unwrap();
        gdb.delete_rows(&folder_str, &stale).unwrap();
        let stats = gdb.species_stats().unwrap();
        assert_eq!(stats.len(), 2); // 白鹭 已消失
        let names: Vec<&str> = stats.iter().map(|s| s.species_name.as_str()).collect();
        assert!(!names.contains(&"白鹭"));
        assert!(gdb.photos_of_species("白鹭").unwrap().is_empty());
    }

    #[test]
    fn test_global_db_photos_of_species_dedup_multi_subject() {
        let tmp = TempDir::new().unwrap();
        let db = GlobalDb::open(tmp.path()).unwrap();
        // 同一物种两个主体（罕见但 DISTINCT 要兜住）→ 照片列表只回 1 条
        let mut r1 = row("E:/A", "1.jpg", "白鹭", Some(90.0), "confirmed", Some("2026-08-01"));
        r1.subject_index = 0;
        let mut r2 = row("E:/A", "1.jpg", "白鹭", Some(85.0), "confirmed", Some("2026-08-01"));
        r2.subject_index = 1;
        db.replace_folder("E:/A", &[r1, r2]).unwrap();
        let photos = db.photos_of_species("白鹭").unwrap();
        assert_eq!(photos, vec![("E:/A".to_string(), "1.jpg".to_string())]);
        // 统计按主体记录计：同物种两主体 = 2 条
        let stats = db.species_stats().unwrap();
        assert_eq!(stats[0].photo_count, 2);
    }

}
