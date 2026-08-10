//! 名录映射：读取 pica_ref.db 只读库，将分类器类别号 (class_index)
//! 映射到具体鸟种 (BirdMatch)。
//!
//! 表结构（pica 裁剪版）：
//! - `animal_info(id PK, latin_name, cn_name, …)` —— 物种信息
//! - `sp_cls_map(species PK, cls)` —— 类别号 → 学名映射
//!
//! 映射查询：`SELECT a.id, a.cn_name, a.latin_name
//!            FROM sp_cls_map m JOIN animal_info a ON a.latin_name = m.species
//!            WHERE m.cls = ?1`
//!
//! - 0 行 = 映射失败（NeedsReview/Mapping）
//! - 1 行 = 唯一匹配
//! - >1 行 = 歧义映射失败（NeedsReview/Mapping）
//!
//! 10573 个类别号中仅约 1385 个有映射——大量 NeedsReview 是预期行为。

use std::path::Path;

use photo_domain::{BirdCandidate, BirdMatch};
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

    /// 解析单一类别号 → (BirdMatch, 失败阶段)。
    ///
    /// 返回 `(Some(match), RecognitionFailureStage::None)` 唯一,
    /// `(None, RecognitionFailureStage::Mapping)` 无映射或歧义。
    pub fn resolve_class(
        &self,
        class_index: u32,
    ) -> (Option<BirdMatch>, photo_domain::RecognitionFailureStage) {
        let matches = self.query_animal(class_index);
        match matches.len() {
            0 => (None, photo_domain::RecognitionFailureStage::Mapping),
            1 => {
                let m = matches.into_iter().next().unwrap();
                (Some(m), photo_domain::RecognitionFailureStage::None)
            }
            _ => (None, photo_domain::RecognitionFailureStage::Mapping),
        }
    }

    /// 解析 Top-N 候选列表为 BirdCandidate 列表。
    ///
    /// 与 pica `BirdLabelResolver._resolveBirdModelCandidates` 语义对等，
    /// 但差异：pica 跳过不可映射项（continue），本实现保留未映射项 (bird=None)。
    pub(crate) fn resolve_top_candidates(
        &self,
        class_indices: &[(u32, f32)],
        skip_index: u32,
    ) -> Vec<BirdCandidate> {
        class_indices
            .iter()
            .filter(|(idx, _)| *idx != skip_index)
            .take(5)
            .map(|(idx, conf)| {
                let (bird, _stage) = self.resolve_class(*idx);
                BirdCandidate {
                    class_index: *idx,
                    confidence: *conf,
                    bird,
                }
            })
            .collect()
    }

    /// 全量鸟种列表（手动修正鸟种下拉数据源）：仅鸟纲，按拼音排序。
    /// 名录库是通用动物库（昆虫/鱼等 5 万余条），修正下拉只需鸟纲（1505 种）；
    /// 拼音列（cn_name_pinyin）排序更符合中文检索习惯，缺失时回退中文名字节序。
    /// 名录库损坏/缺失 schema 时记录警告并返回空 Vec，绝不 panic。
    pub fn all_species(&self) -> Vec<BirdMatch> {
        let mut stmt = match self.conn.prepare_cached(
            "SELECT id, cn_name, latin_name FROM animal_info \
             WHERE gang_cn = '鸟纲' AND cn_name IS NOT NULL AND cn_name != '' \
             ORDER BY COALESCE(NULLIF(cn_name_pinyin, ''), cn_name)",
        ) {
            Ok(stmt) => stmt,
            Err(e) => {
                tracing::warn!("[名录] 全量查询编译失败（schema 不符？）: {e}");
                return Vec::new();
            }
        };
        let rows = match stmt.query_map([], |row| {
            Ok(BirdMatch {
                bird_id: row.get(0)?,
                cn_name: row.get(1)?,
                latin_name: row.get(2)?,
            })
        }) {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!("[名录] 全量查询执行失败: {e}");
                return Vec::new();
            }
        };
        rows.filter_map(|r| r.ok()).collect()
    }

    /// 内部：查询类别号对应的动物记录。
    ///
    /// 名录库损坏/缺失 schema 时 prepare/query 失败：记录警告并返回空 Vec，
    /// 由 [`resolve_class`] 自然映射为 `(None, Mapping)` → NeedsReview 兑底，绝不 panic。
    fn query_animal(&self, cls: u32) -> Vec<BirdMatch> {
        // sql: 与 pica DbService.getBirdRecordsByClassIndex 对等
        // Dart 源码: pica/lib/services/bird_label_resolver.dart:18
        let mut stmt = match self.conn.prepare_cached(
            "SELECT a.id, a.cn_name, a.latin_name \
             FROM sp_cls_map m \
             JOIN animal_info a ON a.latin_name = m.species \
             WHERE m.cls = ?1",
        ) {
            Ok(stmt) => stmt,
            Err(e) => {
                tracing::warn!("[名录] 映射查询编译失败（schema 不符？）: {e}");
                return Vec::new();
            }
        };

        let rows = match stmt.query_map([cls], |row| {
            Ok(BirdMatch {
                bird_id: row.get(0)?,
                cn_name: row.get(1)?,
                latin_name: row.get(2)?,
            })
        }) {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!("[名录] 映射查询执行失败: {e}");
                return Vec::new();
            }
        };

        rows.filter_map(|r| r.ok()).collect()
    }
}

// ---------------------------------------------------------------------------
// 分类后输出（catalog 需要的 Top-5 数据契约）
// ---------------------------------------------------------------------------

/// 单次分类的原始输出。
#[derive(Debug, Clone)]
pub struct ClassificationOutput {
    /// Top-1 类别索引
    pub class_index: u32,
    /// Top-1 置信度 (0-100)
    pub confidence: f32,
    /// Top-5 降序候选，每项 (class_index, confidence 0-100)
    pub top_candidates: Vec<(u32, f32)>,
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// 创建一个内联的 pica_ref 小库供测试用
    fn create_test_db() -> (TempDir, CatalogDb) {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("pica_ref_test.db");
        let conn = Connection::open(&db_path).unwrap();

        conn.execute_batch(
            "CREATE TABLE animal_info (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                latin_name TEXT, cn_name TEXT,
                gang_cn TEXT, cn_name_pinyin TEXT
            );
            CREATE TABLE sp_cls_map (species TEXT PRIMARY KEY, cls INTEGER);
            -- 唯一匹配（鸟种 A，拼音 wu）
            INSERT INTO animal_info (id, latin_name, cn_name, gang_cn, cn_name_pinyin) VALUES (1, 'Turdus merula', '乌鸫', '鸟纲', 'wudong');
            INSERT INTO sp_cls_map (species, cls) VALUES ('Turdus merula', 100);
            -- 唯一匹配（鸟种 B，拼音 da）
            INSERT INTO animal_info (id, latin_name, cn_name, gang_cn, cn_name_pinyin) VALUES (2, 'Parus major', '大山雀', '鸟纲', 'dashanque');
            INSERT INTO sp_cls_map (species, cls) VALUES ('Parus major', 200);
            -- 歧义匹配：类别 300 对应两个学名
            INSERT INTO animal_info (id, latin_name, cn_name, gang_cn, cn_name_pinyin) VALUES (3, 'Species A', '物种A', '鸟纲', 'wuzhonga');
            INSERT INTO animal_info (id, latin_name, cn_name, gang_cn, cn_name_pinyin) VALUES (4, 'Species B', '物种B', '鸟纲', 'wuzhongb');
            INSERT INTO sp_cls_map (species, cls) VALUES ('Species A', 300);
            INSERT INTO sp_cls_map (species, cls) VALUES ('Species B', 300);
            -- 无映射：类别 400 无记录
            -- 非鸟纲（昆虫纲）：all_species 应排除
            INSERT INTO animal_info (id, latin_name, cn_name, gang_cn, cn_name_pinyin) VALUES (5, 'Insect X', '某虫', '昆虫纲', 'mouchong');
            -- 无中文名：all_species 应排除
            INSERT INTO animal_info (id, latin_name, cn_name, gang_cn, cn_name_pinyin) VALUES (6, 'Fish Y', NULL, '辐鳍鱼纲', NULL);",
        )
        .unwrap();

        let db = CatalogDb {
            conn: Connection::open(&db_path).unwrap(),
        };
        (dir, db)
    }

    #[test]
    fn test_all_species_sorted_and_complete() {
        let (_dir, db) = create_test_db();
        let all = db.all_species();
        // 仅鸟纲 4 条（排除昆虫纲某虫、无中文名的鱼），按拼音排序（da < wu）
        assert_eq!(all.len(), 4);
        let names: Vec<&str> = all.iter().map(|b| b.cn_name.as_str()).collect();
        assert_eq!(names, vec!["大山雀", "乌鸫", "物种A", "物种B"]);
        // 字段完整
        let ba = all.iter().find(|b| b.cn_name == "大山雀").unwrap();
        assert_eq!(ba.bird_id, 2);
        assert_eq!(ba.latin_name, "Parus major");
    }

    #[test]
    fn test_resolve_class_unique() {
        let (_dir, db) = create_test_db();
        let (bird, stage) = db.resolve_class(100);
        assert_eq!(stage, photo_domain::RecognitionFailureStage::None);
        let b = bird.unwrap();
        assert_eq!(b.bird_id, 1);
        assert_eq!(b.cn_name, "乌鸫");
        assert_eq!(b.latin_name, "Turdus merula");
    }

    #[test]
    fn test_resolve_class_zero_mapping() {
        let (_dir, db) = create_test_db();
        let (bird, stage) = db.resolve_class(400);
        assert_eq!(stage, photo_domain::RecognitionFailureStage::Mapping);
        assert!(bird.is_none());
    }

    #[test]
    fn test_resolve_class_ambiguous() {
        let (_dir, db) = create_test_db();
        let (bird, stage) = db.resolve_class(300);
        assert_eq!(stage, photo_domain::RecognitionFailureStage::Mapping);
        assert!(bird.is_none());
    }

    #[test]
    fn test_top_candidates_skip_self() {
        let (_dir, db) = create_test_db();
        // Top-5: [(100, 95.0), (200, 80.0), (300, 60.0), (400, 40.0), (100, 30.0)]
        let indices = [
            (100u32, 95.0f32),
            (200, 80.0),
            (300, 60.0),
            (400, 40.0),
            (100, 30.0),
        ];
        let candidates = db.resolve_top_candidates(&indices, 100);
        // 跳过 class_index=100 的自身，保留其他
        // 200 -> 唯一匹配；300 -> 歧义（保留但 bird=None）；400 -> 无映射（保留但 bird=None）
        assert_eq!(candidates.len(), 3); // 5 个候选跳过 2 个 idx=100 的
        assert_eq!(candidates[0].class_index, 200);
        assert!(candidates[0].bird.is_some());
        assert_eq!(candidates[1].class_index, 300);
        assert!(candidates[1].bird.is_none()); // 歧义保留无 bird
        assert_eq!(candidates[2].class_index, 400);
        assert!(candidates[2].bird.is_none()); // 无映射保留无 bird
    }
}
