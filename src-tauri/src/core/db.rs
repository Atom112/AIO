/// 数据库模块：负责初始化 SQLite 数据库连接，创建必要的表结构，并提供数据库访问接口。

use rusqlite::{Connection, Result};
use std::fs;
use tauri::AppHandle;
use tauri::Manager;

pub fn init_db(app: &AppHandle) -> Result<Connection, String> {

    let app_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    
    if !app_dir.exists() {
        fs::create_dir_all(&app_dir).map_err(|e| e.to_string())?;
    }
    
    let db_path = app_dir.join("chat_history.db");
    let conn = Connection::open(db_path).map_err(|e| e.to_string())?;

    // 启用外键支持
    conn.execute("PRAGMA foreign_keys = ON;", []).map_err(|e| e.to_string())?;

    // 创建表结构
    conn.execute_batch(
    "CREATE TABLE IF NOT EXISTS assistants (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        prompt TEXT
    );
    CREATE TABLE IF NOT EXISTS topics (
        id TEXT PRIMARY KEY,
        assistant_id TEXT NOT NULL,
        name TEXT NOT NULL,
        summary TEXT,
        renamed INTEGER NOT NULL DEFAULT 0,
        FOREIGN KEY(assistant_id) REFERENCES assistants(id) ON DELETE CASCADE
    );
    CREATE TABLE IF NOT EXISTS messages (
        id TEXT PRIMARY KEY,
        topic_id TEXT NOT NULL,
        role TEXT NOT NULL,
        content TEXT NOT NULL,
        model_id TEXT,
        display_files TEXT,
        display_text TEXT,
        timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
        FOREIGN KEY(topic_id) REFERENCES topics(id) ON DELETE CASCADE
    );"
).map_err(|e| e.to_string())?;

    // 迁移：给已存在的 topics 表添加 renamed 列（仅当列不存在时执行）
    // 行为：旧话题行标记为 1（已重命名），避免升级后历史话题被自动改名
    let has_renamed: i32 = conn
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('topics') WHERE name='renamed'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if has_renamed == 0 {
        conn.execute(
            "ALTER TABLE topics ADD COLUMN renamed INTEGER NOT NULL DEFAULT 0",
            [],
        )
        .map_err(|e| e.to_string())?;
        conn.execute("UPDATE topics SET renamed = 1", [])
            .map_err(|e| e.to_string())?;
    }

    // 迁移：MCP 工具调用支持（向后兼容）
    add_column_if_missing(&conn, "messages", "tool_call_id", "TEXT")?;
    add_column_if_missing(&conn, "messages", "name", "TEXT")?;
    add_column_if_missing(&conn, "messages", "tool_calls_json", "TEXT")?;

    // 迁移：助手绑定首选模型（向后兼容）
    // 旧助手行缺少 model_id 列，反序列化时按 None 处理，视为使用全局默认模型
    add_column_if_missing(&conn, "assistants", "model_id", "TEXT")?;

    Ok(conn)
}

/// 若指定表缺少指定列，则执行 ALTER TABLE ADD COLUMN。
/// MCP 集成使用此为 messages 表增加 tool_call_id / name / tool_calls_json 三列。
fn add_column_if_missing(
    conn: &Connection,
    table: &str,
    column: &str,
    col_type: &str,
) -> Result<(), String> {
    let exists: i32 = conn
        .query_row(
            &format!(
                "SELECT COUNT(*) FROM pragma_table_info('{}') WHERE name=?1",
                table
            ),
            [column],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if exists == 0 {
        let sql = format!("ALTER TABLE {} ADD COLUMN {} {}", table, column, col_type);
        conn.execute(&sql, []).map_err(|e| e.to_string())?;
    }
    Ok(())
}
