//! Versioned SQLite migration runner.
//!
//! Tracks the applied version via `PRAGMA user_version`; each migration runs in its own
//! transaction and bumps the version on success. Migration actions must be idempotent:
//! legacy databases may have partially applied the same change via the old ad-hoc
//! "temporary ALTER" path (the first versioned run starts at user_version=0).
//!
//! Shared by chat_history.db (core::db) and per-project memory stores
//! (services::memory::store) as the single source of truth for schema migrations.

use crate::ierr;
use rusqlite::Connection;

/// A single migration: `version` must be strictly increasing and > 0.
pub struct Migration {
    /// Target version written to user_version after applying
    pub version: u32,
    /// Migration name (for log lookup)
    pub name: &'static str,
    /// Migration action (must be idempotent)
    pub run: fn(&Connection) -> Result<(), String>,
}

/// Runs unapplied migrations in version order; returns how many were applied this run.
pub fn run_migrations(conn: &Connection, migrations: &[Migration]) -> Result<usize, String> {
    let current: u32 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(|e| {
            ierr!(
                "读取 user_version 失败: {}",
                "Failed to read user_version: {}",
                e
            )
        })?;

    let mut applied = 0usize;
    for m in migrations.iter().filter(|m| m.version > current) {
        let tx = conn.unchecked_transaction().map_err(|e| {
            ierr!(
                "开启迁移事务失败: {}",
                "Failed to begin migration transaction: {}",
                e
            )
        })?;
        (m.run)(&tx).map_err(|e| {
            ierr!(
                "迁移 {} (v{}) 失败: {}",
                "Migration {} (v{}) failed: {}",
                m.name,
                m.version,
                e
            )
        })?;
        tx.pragma_update(None, "user_version", m.version)
            .map_err(|e| {
                ierr!(
                    "写入 user_version 失败: {}",
                    "Failed to write user_version: {}",
                    e
                )
            })?;
        tx.commit().map_err(|e| {
            ierr!(
                "提交迁移事务失败: {}",
                "Failed to commit migration transaction: {}",
                e
            )
        })?;
        tracing::info!("[migrate] v{} {} 已应用", m.version, m.name);
        applied += 1;
    }
    Ok(applied)
}

/// Adds a column via ALTER TABLE ADD COLUMN when the table lacks it (idempotent helper).
///
/// # Safety
/// table / column allow only letters, digits, and underscores (SQL identifier whitelist);
/// col_type is a **literal** constant supplied by the caller (never external input).
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
        return Err(ierr!("非法的表名: '{}'", "Invalid table name: '{}'", table));
    }
    if !is_valid_ident(column) {
        return Err(ierr!(
            "非法的列名: '{}'",
            "Invalid column name: '{}'",
            column
        ));
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
        // A second run is a no-op (idempotent)
        assert_eq!(run_migrations(&conn, MIGS).unwrap(), 0);
    }

    #[test]
    fn add_column_if_missing_is_idempotent_and_guards_identifiers() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE t (id TEXT);").unwrap();
        add_column_if_missing(&conn, "t", "a", "TEXT").unwrap();
        add_column_if_missing(&conn, "t", "a", "TEXT").unwrap(); // Repeated calls do not error
        assert!(add_column_if_missing(&conn, "t", "bad name", "TEXT").is_err());
        assert!(add_column_if_missing(&conn, "t; DROP", "a", "TEXT").is_err());
    }
}
