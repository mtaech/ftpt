/// 基于 `PRAGMA user_version` 的轻量 SQLite 迁移。
///
/// 用法：为每个 DB 维护一个按版本升序的迁移列表，打开连接后调用
/// `run_migrations` 即可。新 DB 从 0 开始全部执行；老 DB 只执行
/// 高于其当前版本的迁移。
pub struct Migration {
    /// 迁移版本号（单调递增，从 1 开始）
    pub version: i32,
    /// 该版本要执行的 SQL（必须幂等，建议 `IF NOT EXISTS`）
    pub sql: &'static str,
}

/// 在连接上按序执行未应用过的迁移，并把 `user_version` 推进到最新。
pub fn run_migrations(
    conn: &rusqlite::Connection,
    migrations: &[Migration],
) -> Result<(), rusqlite::Error> {
    let current: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    for m in migrations {
        if m.version > current {
            conn.execute_batch(m.sql)?;
            conn.execute_batch(&format!("PRAGMA user_version = {}", m.version))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_MIGRATIONS: &[Migration] = &[
        Migration {
            version: 1,
            sql: "CREATE TABLE IF NOT EXISTS t1 (id INTEGER PRIMARY KEY);",
        },
        Migration {
            version: 2,
            sql: "ALTER TABLE t1 ADD COLUMN name TEXT;",
        },
    ];

    #[test]
    fn test_fresh_db_runs_all_migrations() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        run_migrations(&conn, TEST_MIGRATIONS).unwrap();
        let v: i32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, 2);
        // 表和列都存在
        conn.execute("INSERT INTO t1 (id, name) VALUES (1, 'a')", [])
            .unwrap();
    }

    #[test]
    fn test_only_newer_migrations_run() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        // 手动推进到版本 1 并建表
        conn.execute_batch(
            "CREATE TABLE t1 (id INTEGER PRIMARY KEY);
             PRAGMA user_version = 1;",
        )
        .unwrap();
        // 只有 version=2 的 ALTER 会执行
        run_migrations(&conn, TEST_MIGRATIONS).unwrap();
        let v: i32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, 2);
        conn.execute("INSERT INTO t1 (id, name) VALUES (1, 'b')", [])
            .unwrap();
    }
}
