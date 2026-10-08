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
    conn.execute("PRAGMA foreign_keys = ON;", [])
        .map_err(|e| e.to_string())?;

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
    );
    CREATE TABLE IF NOT EXISTS attachments (
        id TEXT PRIMARY KEY,
        sha256 TEXT NOT NULL UNIQUE,
        file_name TEXT NOT NULL,
        mime_type TEXT NOT NULL,
        size INTEGER NOT NULL,
        storage_path TEXT NOT NULL,
        extracted_text TEXT,
        created_at DATETIME DEFAULT CURRENT_TIMESTAMP
    );
    CREATE TABLE IF NOT EXISTS message_attachments (
        message_id TEXT NOT NULL,
        attachment_id TEXT NOT NULL,
        sort_order INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (message_id, attachment_id),
        FOREIGN KEY(message_id) REFERENCES messages(id) ON DELETE CASCADE,
        FOREIGN KEY(attachment_id) REFERENCES attachments(id) ON DELETE CASCADE
    );
    CREATE INDEX IF NOT EXISTS idx_messages_topic_id ON messages(topic_id);
    CREATE INDEX IF NOT EXISTS idx_messages_timestamp ON messages(timestamp);
    CREATE INDEX IF NOT EXISTS idx_topics_assistant_id ON topics(assistant_id);
    CREATE INDEX IF NOT EXISTS idx_message_attachments_attachment_id
        ON message_attachments(attachment_id);",
    )
    .map_err(|e| e.to_string())?;
    // 上次异常退出可能留下尚未关联消息的临时上传；应用启动时安全清理。
    let mut orphan_stmt = conn
        .prepare(
            "SELECT storage_path FROM attachments
             WHERE id NOT IN (SELECT DISTINCT attachment_id FROM message_attachments)",
        )
        .map_err(|e| e.to_string())?;
    let orphan_paths = orphan_stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    drop(orphan_stmt);
    conn.execute(
        "DELETE FROM attachments
         WHERE id NOT IN (SELECT DISTINCT attachment_id FROM message_attachments)",
        [],
    )
    .map_err(|e| e.to_string())?;
    for path in orphan_paths {
        let _ = fs::remove_file(path);
    }

    // 版本化迁移（PRAGMA user_version 记录进度；每条幂等，历史库可能已部分应用）。
    crate::core::migrate::run_migrations(&conn, CHAT_DB_MIGRATIONS)?;

    Ok(conn)
}

// ====== 版本化迁移表（按 version 升序；每条幂等）======
//
// 历史：此处曾是 12+ 处手写的 "add column if missing" 序列；现统一走
// core::migrate 的 user_version 事务化运行器（应用一次，可审计、可回放）。

use crate::core::migrate::{add_column_if_missing, Migration};

/// v1：topics.renamed 列 + 旧话题标记（仅列不存在时执行，
/// 保留"列新加时把存量话题标记为已重命名"的历史语义——避免升级后历史话题被自动改名）。
fn m_topics_renamed(c: &Connection) -> Result<(), String> {
    let has: i32 = c
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('topics') WHERE name='renamed'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    if has == 0 {
        c.execute(
            "ALTER TABLE topics ADD COLUMN renamed INTEGER NOT NULL DEFAULT 0",
            [],
        )
        .map_err(|e| e.to_string())?;
        c.execute("UPDATE topics SET renamed = 1", [])
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// v2：messages 的 MCP 工具调用与思维链列。
fn m_messages_tool_calls(c: &Connection) -> Result<(), String> {
    add_column_if_missing(c, "messages", "tool_call_id", "TEXT")?;
    add_column_if_missing(c, "messages", "name", "TEXT")?;
    add_column_if_missing(c, "messages", "tool_calls_json", "TEXT")?;
    add_column_if_missing(c, "messages", "reasoning", "TEXT")
}

/// v3：assistants 扩展列（首选模型 / MCP 绑定 / Skill 绑定 / 项目归属）。
fn m_assistants_extensions(c: &Connection) -> Result<(), String> {
    add_column_if_missing(c, "assistants", "model_id", "TEXT")?;
    add_column_if_missing(c, "assistants", "mcp_server_ids", "TEXT")?;
    add_column_if_missing(c, "assistants", "skill_ids", "TEXT")?;
    add_column_if_missing(c, "assistants", "project_id", "TEXT")
}

/// v4：projects 表（首次创建）。
fn m_projects_table(c: &Connection) -> Result<(), String> {
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS projects (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            path TEXT NOT NULL UNIQUE,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );",
    )
    .map_err(|e| e.to_string())
}

/// v5：assistants.agent_mode（默认 off = 对话模式）。
fn m_assistants_agent_mode(c: &Connection) -> Result<(), String> {
    add_column_if_missing(c, "assistants", "agent_mode", "TEXT NOT NULL DEFAULT 'off'")
}

/// v6：messages 的 token 用量列（修复重启清零）。
fn m_messages_tokens(c: &Connection) -> Result<(), String> {
    add_column_if_missing(c, "messages", "input_tokens", "INTEGER")?;
    add_column_if_missing(c, "messages", "output_tokens", "INTEGER")
}

/// v7：messages 的 Agent 过程列（跨重启保留 agent_steps / interim / start_time）。
fn m_messages_agent_process(c: &Connection) -> Result<(), String> {
    add_column_if_missing(c, "messages", "agent_steps_json", "TEXT")?;
    add_column_if_missing(c, "messages", "interim_content", "TEXT")?;
    add_column_if_missing(c, "messages", "agent_start_time", "INTEGER")
}

/// v8：messages.images_json（生成图像元数据，供下载与回放）。
fn m_messages_images(c: &Connection) -> Result<(), String> {
    add_column_if_missing(c, "messages", "images_json", "TEXT")
}

/// v9：usage_log 表与索引（不可变 append-only 用量记录）。
fn m_usage_log(c: &Connection) -> Result<(), String> {
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS usage_log (
            id TEXT PRIMARY KEY,
            timestamp TEXT NOT NULL DEFAULT (datetime('now')),
            assistant_id TEXT NOT NULL,
            topic_id TEXT NOT NULL,
            model_id TEXT NOT NULL,
            round INTEGER NOT NULL DEFAULT 1,
            input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            cached_input_tokens INTEGER NOT NULL DEFAULT 0,
            FOREIGN KEY(assistant_id) REFERENCES assistants(id) ON DELETE CASCADE,
            FOREIGN KEY(topic_id) REFERENCES topics(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_usage_log_timestamp ON usage_log(timestamp);
        CREATE INDEX IF NOT EXISTS idx_usage_log_topic ON usage_log(topic_id);",
    )
    .map_err(|e| e.to_string())
}

/// v10：usage_log.cached_input_tokens（OpenAI cached_tokens / DeepSeek cache hit）。
fn m_usage_cached_tokens(c: &Connection) -> Result<(), String> {
    add_column_if_missing(c, "usage_log", "cached_input_tokens", "INTEGER")
}

/// v11：assistants.assistant_type + 内置默认助手标记为 chat。
fn m_assistants_type(c: &Connection) -> Result<(), String> {
    add_column_if_missing(
        c,
        "assistants",
        "assistant_type",
        "TEXT NOT NULL DEFAULT 'project'",
    )?;
    c.execute(
        "UPDATE assistants SET assistant_type = 'chat' WHERE id = 'default-assistant-id' AND (assistant_type IS NULL OR assistant_type != 'chat')",
        [],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// v12：messages 分支列 + parent 索引（会话分支树）。
fn m_messages_branch(c: &Connection) -> Result<(), String> {
    add_column_if_missing(c, "messages", "parent_message_id", "TEXT")?;
    add_column_if_missing(c, "messages", "branch_index", "INTEGER NOT NULL DEFAULT 0")?;
    c.execute(
        "CREATE INDEX IF NOT EXISTS idx_messages_parent ON messages(parent_message_id)",
        [],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// v13：topics 分支列（话题级分支来源）。
fn m_topics_branch(c: &Connection) -> Result<(), String> {
    add_column_if_missing(c, "topics", "branched_from_message_id", "TEXT")?;
    add_column_if_missing(c, "topics", "parent_topic_id", "TEXT")
}

/// v14：Agent 回合检查点表（P4.4：崩溃后无损续跑）。
fn m_agent_checkpoints(c: &Connection) -> Result<(), String> {
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS agent_turn_checkpoints (
            topic_id TEXT NOT NULL,
            turn_id TEXT NOT NULL,
            round INTEGER NOT NULL,
            messages_json TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'active',
            created_at INTEGER NOT NULL,
            PRIMARY KEY (topic_id, turn_id, round)
        );
        CREATE INDEX IF NOT EXISTS idx_checkpoints_topic_status
            ON agent_turn_checkpoints(topic_id, status);",
    )
    .map_err(|e| e.to_string())
}

/// v15：messages.cached_input_tokens（跨重启保留缓存命中统计）。
fn m_messages_cached_tokens(c: &Connection) -> Result<(), String> {
    add_column_if_missing(c, "messages", "cached_input_tokens", "INTEGER")
}

/// chat_history.db 的迁移表（严格递增；新增迁移只允许追加，不得修改已发布条目）。
const CHAT_DB_MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "topics.renamed 列",
        run: m_topics_renamed,
    },
    Migration {
        version: 2,
        name: "messages: MCP 工具调用与思维链列",
        run: m_messages_tool_calls,
    },
    Migration {
        version: 3,
        name: "assistants: 扩展列",
        run: m_assistants_extensions,
    },
    Migration {
        version: 4,
        name: "projects 表",
        run: m_projects_table,
    },
    Migration {
        version: 5,
        name: "assistants.agent_mode",
        run: m_assistants_agent_mode,
    },
    Migration {
        version: 6,
        name: "messages: token 用量列",
        run: m_messages_tokens,
    },
    Migration {
        version: 7,
        name: "messages: agent 过程列",
        run: m_messages_agent_process,
    },
    Migration {
        version: 8,
        name: "messages.images_json",
        run: m_messages_images,
    },
    Migration {
        version: 9,
        name: "usage_log 表与索引",
        run: m_usage_log,
    },
    Migration {
        version: 10,
        name: "usage_log.cached_input_tokens",
        run: m_usage_cached_tokens,
    },
    Migration {
        version: 11,
        name: "assistants.assistant_type",
        run: m_assistants_type,
    },
    Migration {
        version: 12,
        name: "messages: 分支列与索引",
        run: m_messages_branch,
    },
    Migration {
        version: 13,
        name: "topics: 分支列",
        run: m_topics_branch,
    },
    Migration {
        version: 14,
        name: "agent_turn_checkpoints 表（P4.4 检查点）",
        run: m_agent_checkpoints,
    },
    Migration {
        version: 15,
        name: "messages.cached_input_tokens",
        run: m_messages_cached_tokens,
    },
];
