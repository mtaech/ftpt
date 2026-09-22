//! 名录查询：读取 bird_catalog.db 只读库，给识别结论补上名录主键与中文名。
//!
//! 表结构（2026-09-22 已瘦身，见 AGENTS「识别资产」）：
//! - `animal_info(id PK, latin_name, cn_name)` —— 物种信息
//!
//! BioCLIP 的标签本身就是学名，所以主路径是**按学名**查（[`CatalogDb::resolve_latin`]）：
//! 全物种下查不到是常态（本地名录只有 5.6 万条动物，植物/真菌都不在里面），
//! 查不到不是错误——识别结果照样成立，只是没有 taxon_id 与名录侧中文名。

use std::path::Path;

use photo_domain::{CnLevel, TaxonMatch};
use rusqlite::Connection;

use crate::RecognizeError;

/// 名录库只读连接
pub struct CatalogDb {
    conn: Connection,
}

impl CatalogDb {
    /// 以只读方式打开名录库。
    pub fn open(path: &Path) -> Result<Self, RecognizeError> {
        let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        Ok(Self { conn })
    }

    /// 按**学名**直接解析（标签本身就是学名，不经过类别号映射表）。
    ///
    /// 全物种下查不到是常态（名录只有 5.6 万条动物，植物/真菌都不在里面），
    /// 返回 None 不是错误 —— 识别结果照样成立，只是没有名录主键与名录侧的中文名。
    /// 同名多行时优先取有中文名的那条。
    pub fn resolve_latin(&self, latin: &str) -> Option<TaxonMatch> {
        let mut stmt = match self.conn.prepare_cached(
            "SELECT id, cn_name, latin_name FROM animal_info \
             WHERE latin_name = ?1 \
             ORDER BY CASE WHEN cn_name IS NULL OR cn_name = '' THEN 1 ELSE 0 END \
             LIMIT 1",
        ) {
            Ok(stmt) => stmt,
            Err(e) => {
                tracing::warn!("[名录] 学名查询编译失败（schema 不符？）: {e}");
                return None;
            }
        };
        stmt.query_row(rusqlite::params![latin], |row| {
            let cn: Option<String> = row.get(1)?;
            Ok(TaxonMatch {
                taxon_id: Some(row.get(0)?),
                cn_name: cn.unwrap_or_default(),
                latin_name: row.get(2)?,
                cn_level: CnLevel::Species,
                ranks: vec![],
            })
        })
        .ok()
    }
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// 创建与瘦身后的 bird_catalog 同构的测试小库
    fn create_test_db() -> (TempDir, CatalogDb) {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("bird_catalog_test.db");
        let conn = Connection::open(&db_path).unwrap();

        conn.execute_batch(
            "CREATE TABLE animal_info (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                latin_name TEXT, cn_name TEXT
            );
            INSERT INTO animal_info (id, latin_name, cn_name) VALUES (1, 'Turdus merula', '乌鸫');
            INSERT INTO animal_info (id, latin_name, cn_name) VALUES (2, 'Parus major', '大山雀');
            INSERT INTO animal_info (id, latin_name, cn_name) VALUES (3, 'Fish Y', NULL);",
        )
        .unwrap();

        let db = CatalogDb {
            conn: Connection::open(&db_path).unwrap(),
        };
        (dir, db)
    }

    #[test]
    fn test_resolve_latin_species_and_miss() {
        let (_dir, db) = create_test_db();
        // 命中：名录里的种 → taxon_id + 名录侧中文名（BioCLIP 主路径）
        let m = db.resolve_latin("Turdus merula").expect("名录里应有乌鸫");
        assert_eq!(m.taxon_id, Some(1));
        assert_eq!(m.cn_name, "乌鸫");
        assert_eq!(m.cn_level, photo_domain::CnLevel::Species);
        assert!(m.ranks.is_empty(), "七级分类由标签路径补，名录不提供");
        // 未命中：全物种下植物/真菌不在本地名录 → None（不是错误）
        assert!(db.resolve_latin("Prunus mume").is_none());
        // 无中文名的行也能命中（中文名留空，交给标签路径的 zh_names 兜底）
        let fish = db.resolve_latin("Fish Y").expect("无中文名也可命中");
        assert_eq!(fish.taxon_id, Some(3));
        assert!(fish.cn_name.is_empty());
    }
}
