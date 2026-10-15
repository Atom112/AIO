/// Database module: initializes the SQLite connection, creates the required tables, and provides the database access interface.
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

    // Enable foreign key support
    conn.execute("PRAGMA foreign_keys = ON;", [])
        .map_err(|e| e.to_string())?;

    // Create the table schema
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
    // A previous abnormal exit may leave temporary uploads not yet linked to messages; clean them up safely at startup.
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

    // Versioned migrations (PRAGMA user_version tracks progress; each is idempotent, legacy DBs may be partially applied).
    crate::core::migrate::run_migrations(&conn, CHAT_DB_MIGRATIONS)?;

    Ok(conn)
}

// ====== Versioned migration table (ascending by version; each entry idempotent) ======
//
// All migrations run through the core::migrate user_version transactional runner
// (applied once, auditable, replayable).

use crate::core::migrate::{add_column_if_missing, Migration};

/// v1: topics.renamed column + marking legacy topics (runs only when the column is missing,
/// keeping the historical semantics of marking existing topics as renamed when the column is added - prevents legacy topics being auto-renamed after upgrade).
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

/// v2: MCP tool-call and chain-of-thought columns on messages.
fn m_messages_tool_calls(c: &Connection) -> Result<(), String> {
    add_column_if_missing(c, "messages", "tool_call_id", "TEXT")?;
    add_column_if_missing(c, "messages", "name", "TEXT")?;
    add_column_if_missing(c, "messages", "tool_calls_json", "TEXT")?;
    add_column_if_missing(c, "messages", "reasoning", "TEXT")
}

/// v3: assistants extension columns (preferred model / MCP binding / skill binding / project ownership).
fn m_assistants_extensions(c: &Connection) -> Result<(), String> {
    add_column_if_missing(c, "assistants", "model_id", "TEXT")?;
    add_column_if_missing(c, "assistants", "mcp_server_ids", "TEXT")?;
    add_column_if_missing(c, "assistants", "skill_ids", "TEXT")?;
    add_column_if_missing(c, "assistants", "project_id", "TEXT")
}

/// v4: projects table (first creation).
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

/// v5: assistants.agent_mode (default off = chat mode).
fn m_assistants_agent_mode(c: &Connection) -> Result<(), String> {
    add_column_if_missing(c, "assistants", "agent_mode", "TEXT NOT NULL DEFAULT 'off'")
}

/// v6: token usage columns on messages (retained across restarts).
fn m_messages_tokens(c: &Connection) -> Result<(), String> {
    add_column_if_missing(c, "messages", "input_tokens", "INTEGER")?;
    add_column_if_missing(c, "messages", "output_tokens", "INTEGER")
}

/// v7: agent process columns on messages (agent_steps / interim / start_time survive restarts).
fn m_messages_agent_process(c: &Connection) -> Result<(), String> {
    add_column_if_missing(c, "messages", "agent_steps_json", "TEXT")?;
    add_column_if_missing(c, "messages", "interim_content", "TEXT")?;
    add_column_if_missing(c, "messages", "agent_start_time", "INTEGER")
}

/// v8: messages.images_json (generated image metadata, used for download and replay).
fn m_messages_images(c: &Connection) -> Result<(), String> {
    add_column_if_missing(c, "messages", "images_json", "TEXT")
}

/// v9: usage_log table and indexes (immutable append-only usage records).
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

/// v10: usage_log.cached_input_tokens (OpenAI cached_tokens / DeepSeek cache hits).
fn m_usage_cached_tokens(c: &Connection) -> Result<(), String> {
    add_column_if_missing(c, "usage_log", "cached_input_tokens", "INTEGER")
}

/// v11: assistants.assistant_type + mark the built-in default assistant as chat.
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

/// v12: messages branch columns + parent index (conversation branch tree).
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

/// v13: topics branch columns (topic-level branch origin).
fn m_topics_branch(c: &Connection) -> Result<(), String> {
    add_column_if_missing(c, "topics", "branched_from_message_id", "TEXT")?;
    add_column_if_missing(c, "topics", "parent_topic_id", "TEXT")
}

/// v14: agent turn checkpoint table (lossless resume after a crash).
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

/// v15: messages.cached_input_tokens (cache-hit stats survive restarts).
fn m_messages_cached_tokens(c: &Connection) -> Result<(), String> {
    add_column_if_missing(c, "messages", "cached_input_tokens", "INTEGER")
}

/// v16: messages.context_tokens (peak context = last API call's input_tokens; the
/// frontend context-window bar must survive restarts — without this column the bar
/// fell back to input_tokens, which for agent turns is the SUM over all rounds and
/// wildly overstates the real context size).
fn m_messages_context_tokens(c: &Connection) -> Result<(), String> {
    add_column_if_missing(c, "messages", "context_tokens", "INTEGER")
}

/// v17: topics.context_trim_count (compaction context boundary). Compaction keeps the
/// FULL transcript visible in the UI (mainstream: DSH shadows surface nodes, pi keeps
/// raw entries) while the LLM request starts at this boundary: context = summary +
/// history[trim..]. Without it, compaction had to delete old rows from topic.history
/// to bound the context, wiping them from the message record.
fn m_topics_context_trim(c: &Connection) -> Result<(), String> {
    add_column_if_missing(c, "topics", "context_trim_count", "INTEGER")
}

/// Migration table for chat_history.db (strictly increasing; new migrations may only be appended, released entries must never be modified).
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
    Migration {
        version: 16,
        name: "messages.context_tokens",
        run: m_messages_context_tokens,
    },
    Migration {
        version: 17,
        name: "topics.context_trim_count",
        run: m_topics_context_trim,
    },
];
