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

    Ok(conn)
}
