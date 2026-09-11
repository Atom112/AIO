//! 版本化 SQLite 迁移运行器。
//!
//! 以 `PRAGMA user_version` 记录已应用到第几版；每条迁移在独立事务中执行，
//! 成功后推进版本号。迁移动作必须是**幂等**的：历史库可能已通过旧的
//! "临时 ALTER" 路径部分应用过同一变更（首个版本化运行从 user_version=0 起跑）。
//!
//! 供 chat_history.db（core::db）与项目记忆库（services::memory::store）共用，
//! 替代此前两处各自手写的 "add column if missing" 序列。

use rusqlite::Connection;

/// 一条迁移：`version` 必须严格递增且 > 0。
pub struct Migration {
    /// 应用后写入 user_version 的目标版本
    pub version: u32,
    /// 迁移名（日志定位用）
    pub name: &'static str,
    /// 迁移动作（必须是幂等的）
    pub run: fn(&Connection) -> Result<(), String>,
}

/// 按版本顺序执行未应用的迁移；返回本次实际应用的条数。
pub fn run_migrations(conn: &Connection, migrations: &[Migration]) -> Result<usize, String> {
    let current: u32 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(|e| format!("读取 user_version 失败: {e}"))?;

    let mut applied = 0usize;
    for m in migrations.iter().filter(|m| m.version > current) {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("开启迁移事务失败: {e}"))?;
        (m.run)(&tx).map_err(|e| format!("迁移 {} (v{}) 失败: {e}", m.name, m.version))?;
        tx.pragma_update(None, "user_version", m.version)
            .map_err(|e| format!("写入 user_version 失败: {e}"))?;
        tx.commit().map_err(|e| format!("提交迁移事务失败: {e}"))?;
        tracing::info!("[migrate] v{} {} 已应用", m.version, m.name);
        applied += 1;
    }
    Ok(applied)
}

/// 若指定表缺少指定列，则执行 ALTER TABLE ADD COLUMN（幂等辅助）。
///
/// # 安全
/// table / column 仅允许字母、数字、下划线（SQL 标识符白名单），col_type 由调用方
/// 传入**字面量**常量（不接受外部输入）。
pub fn add_column_if_missing(
    conn: &Connection,
    table: &str,
    column: &str,
    col_type: &str,
) -> Result<(), String> {
    fn is_valid_ident(s: &str) -> bool {
        !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || c == '_')
    }
    if !is_valid_ident(table) {
        return Err(format!("非法的表名: '{table}'"));
    }
    if !is_valid_ident(column) {
        return Err(format!("非法的列名: '{column}'"));
    }
    let exists: i32 = conn
        .query_row(
            &format!("SELECT COUNT(*) FROM pragma_table_info('{table}') WHERE name=?1"),
            [column],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if exists == 0 {
        conn.execute(
            &format!("ALTER TABLE {table} ADD COLUMN {column} {col_type}"),
            [],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m1(c: &Connection) -> Result<(), String> {
        c.execute_batch("CREATE TABLE IF NOT EXISTS t (id TEXT PRIMARY KEY);")
            .map_err(|e| e.to_string())
    }
    fn m2(c: &Connection) -> Result<(), String> {
        add_column_if_missing(c, "t", "extra", "TEXT")
    }

    const MIGS: &[Migration] = &[
        Migration {
            version: 1,
            name: "create t",
            run: m1,
        },
        Migration {
            version: 2,
            name: "add extra",
            run: m2,
        },
    ];

    #[test]
    fn applies_once_and_tracks_version() {
        let conn = Connection::open_in_memory().unwrap();
        assert_eq!(run_migrations(&conn, MIGS).unwrap(), 2);
        let v: u32 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(v, 2);
        // 二次运行应为空操作（幂等）
        assert_eq!(run_migrations(&conn, MIGS).unwrap(), 0);
    }

    #[test]
    fn add_column_if_missing_is_idempotent_and_guards_identifiers() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE t (id TEXT);").unwrap();
        add_column_if_missing(&conn, "t", "a", "TEXT").unwrap();
        add_column_if_missing(&conn, "t", "a", "TEXT").unwrap(); // 重复调用不报错
        assert!(add_column_if_missing(&conn, "t", "bad name", "TEXT").is_err());
        assert!(add_column_if_missing(&conn, "t; DROP", "a", "TEXT").is_err());
    }
}
