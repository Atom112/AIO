use crate::models::{SyncAssistant, SyncTopic, SyncMessage, SyncBundle};
use crate::DbState;
use rusqlite::{params, Connection};

#[tauri::command]
pub async fn perform_sync(
    state: tauri::State<'_, DbState>,
    token: String,
    push_only: bool
) -> Result<String, String> {
    // --- 第一阶段：加锁并读取本地变更 ---
    let (_last_sync, local_bundle) = {
        let conn = state.0.lock().unwrap();

        // 1. 获取上次成功同步的时间戳
        let ts: String = conn.query_row(
            "SELECT value FROM sync_metadata WHERE key = 'last_sync_time'",
            [],
            |row| row.get(0),
        ).unwrap_or_else(|_| "1970-01-01 00:00:00".to_string());

        // 2. 收集本地变更 (PUSH 部分)
        let bundle = fetch_local_changes(&conn, &ts)?;
        (ts, bundle)
    }; // 此处大括号结束，conn (MutexGuard) 被自动 drop 释放

    // --- 第二阶段：异步网络请求 (此时没有持有锁) ---
    let client = reqwest::Client::new();
    let response = client
        .post("http://localhost:8080/api/sync/exchange")
        .header("Authorization", format!("Bearer {}", token))
        .json(&local_bundle)
        .send()
        .await
        .map_err(|e| format!("Network error: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Server error: {}", response.status()));
    }

    // 如果只是推送，到这里就结束了，但仍需更新本地同步锚点
    if push_only {
        let conn = state.0.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO sync_metadata (key, value) VALUES ('last_sync_time', CURRENT_TIMESTAMP)",
            [],
        ).map_err(|e| e.to_string())?;
        return Ok("Push completed".into());
    }

    // 解析云端数据
    let remote_bundle: SyncBundle = response.json().await.map_err(|e| e.to_string())?;

    // --- 第三阶段：再次加锁并应用变更 ---
    {
        let mut conn = state.0.lock().unwrap();
        
        // 4. 应用云端接收到的变更 (PULL 部分)
        apply_cloud_changes(&mut conn, remote_bundle)?;

        // 5. 更新本地同步锚点
        conn.execute(
            "INSERT OR REPLACE INTO sync_metadata (key, value) VALUES ('last_sync_time', CURRENT_TIMESTAMP)",
            [],
        ).map_err(|e| e.to_string())?;
    }

    Ok("Sync successful".into())
}

fn fetch_local_changes(conn: &Connection, last_sync: &str) -> Result<SyncBundle, String> {
    // 获取增量助手
    let mut stmt = conn.prepare("SELECT id, name, prompt, updated_at, is_deleted FROM assistants WHERE updated_at > ?1").unwrap();
    let assistants = stmt.query_map([last_sync], |row| {
        Ok(SyncAssistant {
            id: row.get(0)?, 
            name: row.get(1)?, 
            prompt: row.get(2)?, 
            updated_at: row.get(3)?, 
            is_deleted: row.get::<_, i32>(4)? == 1,
        })
    }).unwrap().map(|r| r.unwrap()).collect();

    // 获取增量话题
    let mut stmt = conn.prepare("SELECT id, assistant_id, name, summary, updated_at, is_deleted FROM topics WHERE updated_at > ?1").unwrap();
    let topics = stmt.query_map([last_sync], |row| {
        Ok(SyncTopic {
            id: row.get(0)?, 
            assistant_id: row.get(1)?, 
            name: row.get(2)?, 
            summary: row.get(3)?, 
            updated_at: row.get(4)?, 
            is_deleted: row.get::<_, i32>(5)? == 1,
        })
    }).unwrap().map(|r| r.unwrap()).collect();

    // 获取增量消息
    let mut stmt = conn.prepare("SELECT id, topic_id, role, content, model_id, display_files, display_text, timestamp, updated_at, is_deleted FROM messages WHERE updated_at > ?1").unwrap();
    let messages = stmt.query_map([last_sync], |row| {
        Ok(SyncMessage {
            id: row.get(0)?, 
            topic_id: row.get(1)?, 
            role: row.get(2)?, 
            content: row.get(3)?, 
            model_id: row.get(4)?, 
            display_files: row.get(5)?, 
            display_text: row.get(6)?, 
            timestamp: row.get(7)?, 
            updated_at: row.get(8)?, 
            is_deleted: row.get::<_, i32>(9)? == 1,
        })
    }).unwrap().map(|r| r.unwrap()).collect();

    Ok(SyncBundle { assistants, topics, messages, last_sync_time: last_sync.to_string() })
}

fn apply_cloud_changes(conn: &mut Connection, bundle: SyncBundle) -> Result<(), String> {
    let tx = conn.transaction().map_err(|e| e.to_string())?;

    for a in bundle.assistants {
        tx.execute(
            "INSERT INTO assistants (id, name, prompt, updated_at, is_deleted) VALUES (?1, ?2, ?3, ?4, ?5) 
             ON CONFLICT(id) DO UPDATE SET name=?2, prompt=?3, updated_at=?4, is_deleted=?5",
            params![a.id, a.name, a.prompt, a.updated_at, if a.is_deleted { 1 } else { 0 }],
        ).ok();
    }
    for t in bundle.topics {
        tx.execute(
            "INSERT INTO topics (id, assistant_id, name, summary, updated_at, is_deleted) VALUES (?1, ?2, ?3, ?4, ?5, ?6) 
             ON CONFLICT(id) DO UPDATE SET name=?3, summary=?4, updated_at=?5, is_deleted=?6",
            params![t.id, t.assistant_id, t.name, t.summary, t.updated_at, if t.is_deleted { 1 } else { 0 }],
        ).ok();
    }
    for m in bundle.messages {
        tx.execute(
            "INSERT INTO messages (id, topic_id, role, content, model_id, display_files, display_text, timestamp, updated_at, is_deleted) 
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) 
             ON CONFLICT(id) DO UPDATE SET topic_id=?2, role=?3, content=?4, model_id=?5, display_files=?6, display_text=?7, timestamp=?8, updated_at=?9, is_deleted=?10",
            params![m.id, m.topic_id, m.role, m.content, m.model_id, m.display_files, m.display_text, m.timestamp, m.updated_at, if m.is_deleted { 1 } else { 0 }],
        ).ok();
    }

    tx.commit().map_err(|e| e.to_string())
}