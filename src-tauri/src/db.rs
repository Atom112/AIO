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

    conn.execute("PRAGMA foreign_keys = ON;", []).map_err(|e| e.to_string())?;

    // 创建表结构，增加同步所需字段
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS sync_metadata (
            key TEXT PRIMARY KEY,
            value TEXT
        );
        CREATE TABLE IF NOT EXISTS assistants (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            prompt TEXT,
            updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            is_deleted INTEGER DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS topics (
            id TEXT PRIMARY KEY,
            assistant_id TEXT NOT NULL,
            name TEXT NOT NULL,
            summary TEXT,
            updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            is_deleted INTEGER DEFAULT 0,
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
            updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            is_deleted INTEGER DEFAULT 0,
            FOREIGN KEY(topic_id) REFERENCES topics(id) ON DELETE CASCADE
        );"
    ).map_err(|e| e.to_string())?;

    // 为每个表创建更新触发器，确保每次修改记录时自动刷新 updated_at
    let triggers = [
        ("tg_asst_upd", "assistants"),
        ("tg_topic_upd", "topics"),
        ("tg_msg_upd", "messages"),
    ];
    for (name, table) in triggers {
        let sql = format!(
            "CREATE TRIGGER IF NOT EXISTS {} AFTER UPDATE ON {} 
             BEGIN UPDATE {} SET updated_at = CURRENT_TIMESTAMP WHERE id = old.id; END;",
            name, table, table
        );
        conn.execute(&sql, []).map_err(|e| e.to_string())?;
    }

    Ok(conn)
}