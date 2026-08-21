//! 项目级记忆库（MemoryStore）：facts / fact_versions / meta / FTS5 四类表 + CRUD + 混合检索。
//!
//! 每个项目一个独立 SQLite 文件：<项目根>/.aio/memory/memory.sqlite。
//! 向量以 float32 BLOB 存在 facts.embedding，检索先用 Rust 暴力余弦（P3 换 sqlite-vec）。

use crate::utils::knowledge;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use super::vector::{brute_force_topk, bytes_to_f32, f32_to_bytes};

pub const STATUS_ACTIVE: &str = "active";
pub const STATUS_SUPERSEDED: &str = "superseded";
pub const STATUS_ARCHIVED: &str = "archived";

/// 记忆事实（API 视角，camelCase 序列化）。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MemoryFact {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub content: String,
    pub category: String,
    pub importance: f32,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_refs: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub access_count: i64,
    pub pinned: bool,
}

/// 新增/更新事实的输入。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct NewFact {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub content: String,
    #[serde(default = "default_category")]
    pub category: String,
    #[serde(default = "default_importance")]
    pub importance: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_refs: Option<String>,
    #[serde(default)]
    pub pinned: bool,
}

fn default_category() -> String {
    "note".to_string()
}

fn default_importance() -> f32 {
    0.5
}

/// 记忆库统计。
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MemoryStats {
    pub total_facts: i64,
    pub active_facts: i64,
    pub archived_facts: i64,
    pub superseded_facts: i64,
    pub embedded_facts: i64,
    pub db_size_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_reindex_at: Option<String>,
}

/// 检索命中的事实（带融合分数）。
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ScoredFact {
    pub id: String,
    pub content: String,
    pub category: String,
    pub score: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

/// 版本审计记录。
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FactVersion {
    pub id: i64,
    pub fact_id: String,
    pub version: i64,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_before: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_after: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub related_fact_ids: Option<String>,
    pub created_at: String,
}

/// 检索选项。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SearchOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default = "default_k")]
    pub k: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_score: Option<f32>,
}

fn default_k() -> usize {
    10
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            category: None,
            k: default_k(),
            min_score: None,
        }
    }
}

/// 每项目记忆库。
pub struct MemoryStore {
    conn: Mutex<Connection>,
    project_root: String,
}

/// 建表语句（幂等；参数化写入，DDL 不含字符串字面量）。
const SCHEMA: &str = concat!(
    "CREATE TABLE IF NOT EXISTS facts (",
    "id TEXT PRIMARY KEY,",
    "key TEXT,",
    "content TEXT NOT NULL,",
    "category TEXT NOT NULL,",
    "importance REAL NOT NULL,",
    "status TEXT NOT NULL,",
    "embedding BLOB,",
    "embedding_model TEXT,",
    "embedding_dim INTEGER,",
    "confidence REAL,",
    "source_type TEXT,",
    "source_refs TEXT,",
    "valid_from TEXT,",
    "valid_until TEXT,",
    "created_at TEXT NOT NULL,",
    "updated_at TEXT NOT NULL,",
    "access_count INTEGER NOT NULL,",
    "last_access_at TEXT,",
    "pinned INTEGER NOT NULL",
    ");",
    "CREATE INDEX IF NOT EXISTS idx_facts_status ON facts(status);",
    "CREATE INDEX IF NOT EXISTS idx_facts_category ON facts(category);",
    "CREATE INDEX IF NOT EXISTS idx_facts_key ON facts(key);",
    "CREATE TABLE IF NOT EXISTS fact_versions (",
    "id INTEGER PRIMARY KEY AUTOINCREMENT,",
    "fact_id TEXT NOT NULL REFERENCES facts(id) ON DELETE CASCADE,",
    "version INTEGER NOT NULL,",
    "reason TEXT NOT NULL,",
    "content_before TEXT,",
    "content_after TEXT,",
    "related_fact_ids TEXT,",
    "judge_model TEXT,",
    "created_at TEXT NOT NULL",
    ");",
    "CREATE INDEX IF NOT EXISTS idx_versions_fact ON fact_versions(fact_id);",
    "CREATE TABLE IF NOT EXISTS meta (",
    "k TEXT PRIMARY KEY,",
    "v TEXT NOT NULL",
    ");",
    "CREATE VIRTUAL TABLE IF NOT EXISTS fts_facts USING fts5(",
    "fact_id UNINDEXED,",
    "content,",
    "category,",
    "source_type,",
    "tokenize=trigram",
    ");",
    // P4 代码级 RAG：索引文件分块
    "CREATE TABLE IF NOT EXISTS code_chunks (",
    "id TEXT PRIMARY KEY,",
    "file_path TEXT NOT NULL,",
    "start_line INTEGER NOT NULL,",
    "end_line INTEGER NOT NULL,",
    "content TEXT NOT NULL,",
    "embedding BLOB,",
    "embedding_model TEXT,",
    "embedding_dim INTEGER,",
    "created_at TEXT NOT NULL",
    ");",
    "CREATE INDEX IF NOT EXISTS idx_code_path ON code_chunks(file_path);",
    "CREATE TABLE IF NOT EXISTS code_file_state (",
    "file_path TEXT PRIMARY KEY,",
    "mtime_nanos INTEGER NOT NULL,",
    "size_bytes INTEGER NOT NULL",
    ");",
    "CREATE VIRTUAL TABLE IF NOT EXISTS fts_code USING fts5(",
    "chunk_id UNINDEXED,",
    "content,",
    "file_path,",
    "tokenize=trigram",
    ");",
);

impl MemoryStore {
    /// 打开（或创建）项目记忆库：<项目根>/.aio/memory/memory.sqlite。
    /// 若 sqlite-vec 可用则启用 vec0 虚拟表（P3），不可用时回退暴力余弦。
    pub fn open(project_root: &str) -> Result<Self, String> {
        let conn = Self::open_connection(project_root)?;
        Ok(Self {
            conn: Mutex::new(conn),
            project_root: project_root.to_string(),
        })
    }

    /// 打开到项目记忆库的独立连接（供 code.rs 等场景使用；WAL 支持多连接并发）。
    pub fn open_connection(project_root: &str) -> Result<Connection, String> {
        // 必须先注册自动扩展，再打开连接
        let _ = crate::services::memory::vec0::vec0_available();
        let dir = Path::new(project_root).join(".aio").join("memory");
        std::fs::create_dir_all(&dir).map_err(|e| format!("创建记忆目录失败: {e}"))?;
        let path = dir.join("memory.sqlite");
        let conn = Connection::open(&path).map_err(|e| e.to_string())?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| e.to_string())?;
        conn.pragma_update(None, "busy_timeout", 5000)
            .map_err(|e| e.to_string())?;
        conn.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
        if crate::services::memory::vec0::vec0_available() {
            let dim = conn
                .query_row(
                    "SELECT v FROM meta WHERE k = ?1",
                    [META_EMBEDDING_DIM],
                    |r| r.get::<_, String>(0),
                )
                .optional()
                .map_err(|e| e.to_string())?
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(1024);
            crate::services::memory::vec0::ensure_vec_table(&conn, dim)
                .map_err(|e| e.to_string())?;
        }
        Ok(conn)
    }

    /// 写入/更新一条事实：key 存在则原地更新（P2 起叠加向量近邻仲裁）；
    /// embedding 为 Some((模型, 维度, 向量)) 时同步更新向量，None 则保留/不写向量。
    pub fn upsert_fact(
        &self,
        fact: &NewFact,
        embedding: Option<(String, usize, Vec<f32>)>,
    ) -> Result<MemoryFact, String> {
        let now = now_iso();
        let new_id = uuid::Uuid::new_v4().to_string();
        let mut conn = self.conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let existing: Option<String> = if let Some(k) = &fact.key {
            let sql = "SELECT id FROM facts WHERE key = ?1 AND status != ?2";
            tx.query_row(sql, params![k, STATUS_ARCHIVED], |r| r.get(0))
                .optional()
                .map_err(|e| e.to_string())?
        } else {
            None
        };
        let fid = match existing {
            Some(fid) => {
                // 版本审计：记录旧内容（reason=update）
                let old_content: Option<String> = tx
                    .query_row("SELECT content FROM facts WHERE id = ?1", [&fid], |r| {
                        r.get(0)
                    })
                    .optional()
                    .map_err(|e| e.to_string())?;
                tx.execute(
                    "UPDATE facts SET content = ?1, category = ?2, importance = ?3, updated_at = ?4, source_type = COALESCE(?5, source_type), source_refs = COALESCE(?6, source_refs) WHERE id = ?7",
                    params![fact.content, fact.category, fact.importance, now, fact.source_type, fact.source_refs, fid],
                )
                .map_err(|e| e.to_string())?;
                log_version_tx(
                    &tx,
                    &fid,
                    "update",
                    old_content.as_deref(),
                    Some(&fact.content),
                    None,
                    "",
                )?;
                fid
            }
            None => {
                tx.execute(
                    "INSERT INTO facts (id, key, content, category, importance, status, embedding, embedding_model, embedding_dim, confidence, source_type, source_refs, valid_from, created_at, updated_at, access_count, pinned) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
                    params![
                        new_id,
                        fact.key,
                        fact.content,
                        fact.category,
                        fact.importance,
                        STATUS_ACTIVE,
                        embedding.as_ref().map(|(_, _, v)| f32_to_bytes(v)),
                        embedding.as_ref().map(|(m, _, _)| m.clone()),
                        embedding.as_ref().map(|(_, d, _)| *d as i64),
                        fact.importance,
                        fact.source_type,
                        fact.source_refs,
                        now,
                        now,
                        now,
                        0i64,
                        fact.pinned as i64,
                    ],
                )
                .map_err(|e| e.to_string())?;
                new_id
            }
        };
        // 显式提供向量时（新增或更新都生效；同步 vec0）
        if let Some(emb) = &embedding {
            self.write_embedding(&tx, &fid, emb)?;
        }
        // FTS5 同步（facts 表隐式 rowid）
        tx.execute(
            "INSERT OR REPLACE INTO fts_facts (rowid, fact_id, content, category, source_type) SELECT rowid, id, content, category, COALESCE(source_type, ?1) FROM facts WHERE id = ?2",
            params!["", fid],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        drop(conn);
        self.get_fact(&fid)
    }

    /// 按 id 读取事实。
    pub fn get_fact(&self, id: &str) -> Result<MemoryFact, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.query_row(
            "SELECT id, key, content, category, importance, status, embedding_model, source_type, source_refs, created_at, updated_at, access_count, pinned FROM facts WHERE id = ?1",
            [id],
            |r| {
                Ok(MemoryFact {
                    id: r.get(0)?,
                    key: r.get(1)?,
                    content: r.get(2)?,
                    category: r.get(3)?,
                    importance: r.get(4)?,
                    status: r.get(5)?,
                    embedding_model: r.get(6)?,
                    source_type: r.get(7)?,
                    source_refs: r.get(8)?,
                    created_at: r.get(9)?,
                    updated_at: r.get(10)?,
                    access_count: r.get(11)?,
                    pinned: r.get::<_, i64>(12)? != 0,
                })
            },
        )
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("事实不存在: {id}"))
    }

    /// 列出事实（status/category 可选过滤，按更新时间倒序，分页）。
    pub fn list_facts(
        &self,
        status: Option<&str>,
        category: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<(Vec<MemoryFact>, i64), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let base = "SELECT id, key, content, category, importance, status, embedding_model, source_type, source_refs, created_at, updated_at, access_count, pinned FROM facts";
        let where_clause =
            "WHERE status = COALESCE(?1, status) AND category = COALESCE(?2, category)";
        let mut stmt = conn
            .prepare(&format!(
                "{base} {where_clause} ORDER BY updated_at DESC LIMIT ?3 OFFSET ?4"
            ))
            .map_err(|e| e.to_string())?;
        let facts: Vec<MemoryFact> = stmt
            .query_map(params![status, category, limit, offset], |r| {
                Ok(MemoryFact {
                    id: r.get(0)?,
                    key: r.get(1)?,
                    content: r.get(2)?,
                    category: r.get(3)?,
                    importance: r.get(4)?,
                    status: r.get(5)?,
                    embedding_model: r.get(6)?,
                    source_type: r.get(7)?,
                    source_refs: r.get(8)?,
                    created_at: r.get(9)?,
                    updated_at: r.get(10)?,
                    access_count: r.get(11)?,
                    pinned: r.get::<_, i64>(12)? != 0,
                })
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        drop(stmt);
        let total: i64 = conn
            .query_row(
                &format!("SELECT COUNT(*) FROM facts {where_clause}"),
                params![status, category],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        Ok((facts, total))
    }

    /// 删除事实（级联删除版本审计与 FTS 条目）。
    pub fn delete_fact(&self, id: &str) -> Result<(), String> {
        let mut conn = self.conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM fts_facts WHERE fact_id = ?1", [id])
            .map_err(|e| e.to_string())?;
        if crate::services::memory::vec0::vec0_available() {
            crate::services::memory::vec0::vec0_delete(&tx, id).map_err(|e| e.to_string())?;
        }
        tx.execute("DELETE FROM facts WHERE id = ?1", [id])
            .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    }

    /// 修改事实状态（active / superseded / archived）。
    pub fn set_status(
        &self,
        id: &str,
        status: &str,
        valid_until: Option<&str>,
    ) -> Result<MemoryFact, String> {
        let now = now_iso();
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE facts SET status = ?1, valid_until = COALESCE(?2, valid_until), updated_at = ?3 WHERE id = ?4",
            params![status, valid_until, now, id],
        )
        .map_err(|e| e.to_string())?;
        if status != STATUS_ACTIVE && crate::services::memory::vec0::vec0_available() {
            crate::services::memory::vec0::vec0_delete(&conn, id).map_err(|e| e.to_string())?;
        }
        drop(conn);
        self.get_fact(id)
    }

    /// 读取本项目记忆开关（meta 覆盖；None = 跟随全局默认）。
    pub fn project_enabled(&self) -> Result<Option<bool>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let v: Option<String> = conn
            .query_row(
                "SELECT v FROM meta WHERE k = ?1",
                [META_PROJECT_ENABLED],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        Ok(v.and_then(|s| match s.as_str() {
            "1" => Some(true),
            "0" => Some(false),
            _ => None,
        }))
    }

    /// 设置本项目记忆开关（None = 清除覆盖，跟随全局默认）。
    pub fn set_project_enabled(&self, enabled: Option<bool>) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        match enabled {
            Some(true) => {
                conn.execute(
                    "INSERT OR REPLACE INTO meta (k, v) VALUES (?1, ?2)",
                    params![META_PROJECT_ENABLED, "1"],
                )
                .map_err(|e| e.to_string())?;
            }
            Some(false) => {
                conn.execute(
                    "INSERT OR REPLACE INTO meta (k, v) VALUES (?1, ?2)",
                    params![META_PROJECT_ENABLED, "0"],
                )
                .map_err(|e| e.to_string())?;
            }
            None => {
                conn.execute("DELETE FROM meta WHERE k = ?1", [META_PROJECT_ENABLED])
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }

    /// 钉住 / 取消钉住。
    pub fn set_pinned(&self, id: &str, pinned: bool) -> Result<MemoryFact, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE facts SET pinned = ?1 WHERE id = ?2",
            params![pinned as i64, id],
        )
        .map_err(|e| e.to_string())?;
        drop(conn);
        self.get_fact(id)
    }

    /// 写入向量（facts BLOB + vec0 同步；维度变化时重建 vec0 表）。
    fn write_embedding(
        &self,
        conn: &Connection,
        id: &str,
        embedding: &(String, usize, Vec<f32>),
    ) -> Result<(), String> {
        let (model, dim, vec) = embedding;
        conn.execute(
            "UPDATE facts SET embedding = ?1, embedding_model = ?2, embedding_dim = ?3 WHERE id = ?4",
            params![f32_to_bytes(vec), model, *dim as i64, id],
        )
        .map_err(|e| e.to_string())?;
        let _ = self.meta_put_inner(conn, META_EMBEDDING_DIM, &dim.to_string());
        if crate::services::memory::vec0::vec0_available() {
            let current = crate::services::memory::vec0::vec0_dim(conn);
            if current != Some(*dim) {
                conn.execute_batch("DROP TABLE IF EXISTS vec_facts")
                    .map_err(|e| e.to_string())?;
                crate::services::memory::vec0::ensure_vec_table(conn, *dim)
                    .map_err(|e| e.to_string())?;
            }
            crate::services::memory::vec0::vec0_upsert(conn, id, vec).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// 向量检索：vec0 优先，失败回退暴力余弦（表缺失/维度变化等）。
    fn vector_search(
        &self,
        conn: &Connection,
        query: &[f32],
        k: usize,
    ) -> Result<Vec<(String, f32)>, String> {
        if crate::services::memory::vec0::vec0_available() {
            if let Ok(rows) = crate::services::memory::vec0::vec0_search(conn, query, k) {
                if !rows.is_empty() || k == 0 {
                    return Ok(rows);
                }
            }
        }
        let mut stmt = conn
            .prepare("SELECT id, embedding FROM facts WHERE status = ?1 AND embedding IS NOT NULL")
            .map_err(|e| e.to_string())?;
        let rows: Vec<(String, Vec<u8>)> = stmt
            .query_map(params![STATUS_ACTIVE], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        drop(stmt);
        let decoded: Vec<(String, Vec<f32>)> = rows
            .iter()
            .map(|(id, b)| (id.clone(), bytes_to_f32(b)))
            .collect();
        Ok(brute_force_topk(&decoded, query, k))
    }

    /// 单独更新事实向量（仲裁订正内容后调用；None 表示清除向量）。
    pub fn set_embedding(
        &self,
        id: &str,
        embedding: Option<(String, usize, Vec<f32>)>,
    ) -> Result<MemoryFact, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        if let Some(emb) = &embedding {
            self.write_embedding(&conn, id, emb)?;
        } else {
            conn.execute("UPDATE facts SET embedding = NULL, embedding_model = NULL, embedding_dim = NULL WHERE id = ?1", [id])
                .map_err(|e| e.to_string())?;
            if crate::services::memory::vec0::vec0_available() {
                crate::services::memory::vec0::vec0_delete(&conn, id).map_err(|e| e.to_string())?;
            }
        }
        drop(conn);
        self.get_fact(id)
    }

    /// 导出全部事实为 JSON（不含向量；导入后重新向量化）。
    pub fn export_facts(&self) -> Result<String, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT key, content, category, importance, status, source_type, source_refs, pinned, created_at, updated_at FROM facts")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok(serde_json::json!({
                    "key": r.get::<_, Option<String>>(0)?,
                    "content": r.get::<_, String>(1)?,
                    "category": r.get::<_, String>(2)?,
                    "importance": r.get::<_, f32>(3)?,
                    "status": r.get::<_, String>(4)?,
                    "sourceType": r.get::<_, Option<String>>(5)?,
                    "sourceRefs": r.get::<_, Option<String>>(6)?,
                    "pinned": r.get::<_, i64>(7)? != 0,
                    "createdAt": r.get::<_, String>(8)?,
                    "updatedAt": r.get::<_, String>(9)?,
                }))
            })
            .map_err(|e| e.to_string())?;
        let facts: Vec<serde_json::Value> = rows.filter_map(|r| r.ok()).collect();
        serde_json::to_string_pretty(&serde_json::json!({
            "version": 1,
            "exportedAt": now_iso(),
            "facts": facts,
        }))
        .map_err(|e| e.to_string())
    }

    /// 从导出的 JSON 导入事实（按 key 去重；返回导入条数）。
    pub fn import_facts(&self, json: &str) -> Result<usize, String> {
        let parsed: serde_json::Value =
            serde_json::from_str(json).map_err(|e| format!("解析导入数据失败: {e}"))?;
        let facts = parsed
            .get("facts")
            .and_then(|v| v.as_array())
            .ok_or_else(|| "导入数据缺少 facts 数组".to_string())?;
        let mut count = 0usize;
        for item in facts {
            let fact = NewFact {
                key: item
                    .get("key")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                content: item
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                category: item
                    .get("category")
                    .and_then(|v| v.as_str())
                    .unwrap_or("note")
                    .to_string(),
                importance: item
                    .get("importance")
                    .and_then(|v| v.as_f64())
                    .map(|v| v as f32)
                    .unwrap_or(0.5),
                source_type: item
                    .get("sourceType")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                source_refs: item
                    .get("sourceRefs")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                pinned: item
                    .get("pinned")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            };
            if fact.content.trim().is_empty() {
                continue;
            }
            self.upsert_fact(&fact, None)?;
            count += 1;
        }
        Ok(count)
    }

    /// 读取 meta 键值。
    pub fn meta_get(&self, key: &str) -> Result<Option<String>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.query_row("SELECT v FROM meta WHERE k = ?1", [key], |r| r.get(0))
            .optional()
            .map_err(|e| e.to_string())
    }

    /// 写入 meta 键值。
    pub fn meta_put(&self, key: &str, value: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        self.meta_put_inner(&conn, key, value)
    }

    fn meta_put_inner(&self, conn: &Connection, key: &str, value: &str) -> Result<(), String> {
        conn.execute(
            "INSERT OR REPLACE INTO meta (k, v) VALUES (?1, ?2)",
            params![key, value],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// 直接订正事实内容（不换 key），并写入版本审计（reason=update/merge）。
    pub fn update_content(
        &self,
        id: &str,
        content: &str,
        reason: &str,
    ) -> Result<MemoryFact, String> {
        let now = now_iso();
        let mut conn = self.conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let old: Option<String> = tx
            .query_row("SELECT content FROM facts WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .optional()
            .map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE facts SET content = ?1, updated_at = ?2 WHERE id = ?3",
            params![content, now, id],
        )
        .map_err(|e| e.to_string())?;
        log_version_tx(&tx, id, reason, old.as_deref(), Some(content), None, "")?;
        tx.execute(
            "INSERT OR REPLACE INTO fts_facts (rowid, fact_id, content, category, source_type) SELECT rowid, id, content, category, COALESCE(source_type, ?1) FROM facts WHERE id = ?2",
            params!["", id],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        drop(conn);
        self.get_fact(id)
    }

    /// 合并两条事实：target 保留（内容更新），source 标记 superseded 并记录版本审计。
    pub fn merge_facts(
        &self,
        target_id: &str,
        source_id: &str,
        merged_content: &str,
    ) -> Result<MemoryFact, String> {
        let now = now_iso();
        let mut conn = self.conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let old_target: Option<String> = tx
            .query_row(
                "SELECT content FROM facts WHERE id = ?1",
                [target_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE facts SET content = ?1, updated_at = ?2 WHERE id = ?3",
            params![merged_content, now, target_id],
        )
        .map_err(|e| e.to_string())?;
        log_version_tx(
            &tx,
            target_id,
            "merge",
            old_target.as_deref(),
            Some(merged_content),
            Some(&format!("[{source_id}]")),
            "",
        )?;
        log_version_tx(&tx, source_id, "supersede", None, None, None, "")?;
        tx.execute(
            "UPDATE facts SET status = ?1, valid_until = ?2, updated_at = ?3 WHERE id = ?4",
            params![STATUS_SUPERSEDED, now, now, source_id],
        )
        .map_err(|e| e.to_string())?;
        if crate::services::memory::vec0::vec0_available() {
            crate::services::memory::vec0::vec0_delete(&tx, source_id)
                .map_err(|e| e.to_string())?;
        }
        tx.execute(
            "INSERT OR REPLACE INTO fts_facts (rowid, fact_id, content, category, source_type) SELECT rowid, id, content, category, COALESCE(source_type, ?1) FROM facts WHERE id = ?2",
            params!["", target_id],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        drop(conn);
        self.get_fact(target_id)
    }

    /// 标记事实为 superseded（冲突/过时），记录版本审计。
    pub fn supersede_fact(&self, id: &str, reason: &str) -> Result<MemoryFact, String> {
        let now = now_iso();
        let mut conn = self.conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        let old: Option<String> = tx
            .query_row("SELECT content FROM facts WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .optional()
            .map_err(|e| e.to_string())?;
        log_version_tx(&tx, id, "supersede", old.as_deref(), None, None, reason)?;
        tx.execute(
            "UPDATE facts SET status = ?1, valid_until = ?2, updated_at = ?3 WHERE id = ?4",
            params![STATUS_SUPERSEDED, now, now, id],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        drop(conn);
        self.get_fact(id)
    }

    /// 读取事实的版本审计链。
    pub fn get_versions(&self, fact_id: &str) -> Result<Vec<FactVersion>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT id, fact_id, version, reason, content_before, content_after, related_fact_ids, created_at FROM fact_versions WHERE fact_id = ?1 ORDER BY version DESC")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([fact_id], |r| {
                Ok(FactVersion {
                    id: r.get(0)?,
                    fact_id: r.get(1)?,
                    version: r.get(2)?,
                    reason: r.get(3)?,
                    content_before: r.get(4)?,
                    content_after: r.get(5)?,
                    related_fact_ids: r.get(6)?,
                    created_at: r.get(7)?,
                })
            })
            .map_err(|e| e.to_string())?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// 容量治理：活跃事实超过上限时，按 score = importance 乘访问衰减归档最低分的非 pinned 事实。
    pub fn prune_to_capacity(&self, max_facts: u32) -> Result<usize, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let active: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM facts WHERE status = ?1",
                [STATUS_ACTIVE],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if active as u32 <= max_facts {
            return Ok(0);
        }
        let excess = (active as u32 - max_facts) as usize;
        // 最低分优先归档：importance * (1 + 0.1 * min(access_count, 20))
        let mut stmt = conn
            .prepare("SELECT id FROM facts WHERE status = ?1 AND pinned = 0 ORDER BY (importance * (1.0 + 0.1 * MIN(access_count, 20))) ASC, updated_at ASC LIMIT ?2")
            .map_err(|e| e.to_string())?;
        let ids: Vec<String> = stmt
            .query_map(params![STATUS_ACTIVE, excess as i64], |r| {
                r.get::<_, String>(0)
            })
            .map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
        drop(stmt);
        let now = now_iso();
        let mut archived = 0usize;
        for id in ids {
            let n = conn
                .execute(
                    "UPDATE facts SET status = ?1, valid_until = ?2, updated_at = ?3 WHERE id = ?4",
                    params![STATUS_ARCHIVED, now, now, id],
                )
                .map_err(|e| e.to_string())?;
            archived += n;
        }
        Ok(archived)
    }

    /// 清空全部事实（删除项目记忆）。
    pub fn clear(&self) -> Result<(), String> {
        let mut conn = self.conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM facts", [])
            .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM fact_versions", [])
            .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM fts_facts", [])
            .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    }

    /// 检索计数累加（供使用统计）。
    pub fn bump_access(&self, ids: &[String]) {
        if ids.is_empty() {
            return;
        }
        let now = now_iso();
        if let Ok(conn) = self.conn.lock() {
            for id in ids {
                let _ = conn.execute(
                    "UPDATE facts SET access_count = access_count + 1, last_access_at = ?1 WHERE id = ?2",
                    params![now, id],
                );
            }
        }
    }

    /// 混合检索：向量 top-k（query_vec 提供时）+ FTS5 BM25 → RRF 融合 + 时新度加成。
    pub fn search_hybrid(
        &self,
        query_text: &str,
        query_vec: Option<&[f32]>,
        opts: &SearchOptions,
    ) -> Result<Vec<ScoredFact>, String> {
        let k = opts.k.clamp(1, 50);
        let conn = self.conn.lock().map_err(|e| e.to_string())?;

        // 1) 向量召回（vec0 加速，不可用时暴力余弦）
        let mut vector_ranks: Vec<(String, usize)> = Vec::new();
        if let Some(qv) = query_vec {
            let top = self.vector_search(&conn, qv, k * 2)?;
            for (i, (id, _)) in top.iter().enumerate() {
                vector_ranks.push((id.clone(), i));
            }
        }

        // 2) FTS5 召回（trigram，支持中文子串；非法表达式降级为 LIKE）
        let mut fts_ranks: Vec<(String, usize)> = Vec::new();
        let sanitized = sanitize_fts_query(query_text);
        if !sanitized.trim().is_empty() {
            let sql = "SELECT fact_id FROM fts_facts WHERE fts_facts MATCH ?1 ORDER BY bm25(fts_facts) LIMIT ?2";
            let matched: Result<Vec<String>, String> = (|| {
                let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
                let rows = stmt
                    .query_map(params![sanitized, (k * 2) as i64], |r| {
                        r.get::<_, String>(0)
                    })
                    .map_err(|e| e.to_string())?;
                rows.filter_map(|r| r.ok())
                    .collect::<Vec<String>>()
                    .pipe(Ok)
            })();
            match matched {
                Ok(ids) => {
                    for (i, id) in ids.iter().enumerate() {
                        fts_ranks.push((id.clone(), i));
                    }
                }
                Err(_) => {
                    // FTS 表达式解析失败时回退 LIKE 模糊匹配
                    let pattern = format!("%{}%", query_text.trim());
                    let mut stmt = conn
                        .prepare("SELECT id FROM facts WHERE status = ?1 AND (content LIKE ?2 OR category LIKE ?2)")
                        .map_err(|e| e.to_string())?;
                    let rows = stmt
                        .query_map(params![STATUS_ACTIVE, pattern], |r| r.get::<_, String>(0))
                        .map_err(|e| e.to_string())?;
                    for (i, id) in rows.filter_map(|r| r.ok()).take(k * 2).enumerate() {
                        fts_ranks.push((id, i));
                    }
                }
            }
        }

        // 3) RRF 融合
        let mut rrf: HashMap<String, f32> = HashMap::new();
        for (id, rank) in vector_ranks.iter().chain(fts_ranks.iter()) {
            let entry = rrf.entry(id.clone()).or_insert(0.0);
            *entry += 1.0 / (60.0 + *rank as f32 + 1.0);
        }
        if rrf.is_empty() {
            return Ok(Vec::new());
        }

        // 4) 取详情 + 时新度加成 + 排序
        let now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut scored: Vec<ScoredFact> = Vec::new();
        for (id, score) in rrf.iter() {
            if let Ok(fact) = self.fetch_scored(id, *score, now_secs, &conn) {
                if let Some(min) = opts.min_score {
                    if fact.score < min {
                        continue;
                    }
                }
                scored.push(fact);
            }
        }
        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(k);
        Ok(scored)
    }

    /// 生成注入系统提示词的记忆块（Markdown），按 token 预算裁剪；空结果返回 None。
    pub fn build_inject_block(
        &self,
        query_text: &str,
        query_vec: Option<&[f32]>,
        budget_tokens: usize,
    ) -> Result<Option<String>, String> {
        if budget_tokens == 0 {
            return Ok(None);
        }
        let results = self.search_hybrid(
            query_text,
            query_vec,
            &SearchOptions {
                category: None,
                k: 20,
                min_score: None,
            },
        )?;
        if results.is_empty() {
            return Ok(None);
        }
        let mut block = String::from("项目记忆（跨会话沉淀，仅作快速参考，以实际代码为准）：\n");
        let mut used = count_approx(&block);
        let mut count = 0usize;
        for f in results {
            let line = format!(
                "- [{}] {}（更新时间 {}）\n",
                f.category,
                f.content,
                f.updated_at.clone().unwrap_or_default(),
            );
            let t = count_approx(&line);
            if used + t > budget_tokens && count > 0 {
                break;
            }
            block.push_str(&line);
            used += t;
            count += 1;
        }
        if count == 0 {
            return Ok(None);
        }
        Ok(Some(block))
    }

    /// 记忆库统计。
    pub fn stats(&self) -> Result<MemoryStats, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let total: i64 = conn
            .query_row("SELECT COUNT(*) FROM facts", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        let active: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM facts WHERE status = ?1",
                [STATUS_ACTIVE],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let archived: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM facts WHERE status = ?1",
                [STATUS_ARCHIVED],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let superseded: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM facts WHERE status = ?1",
                [STATUS_SUPERSEDED],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let embedded: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM facts WHERE embedding IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let last_reindex: Option<String> = conn
            .query_row(
                "SELECT v FROM meta WHERE k = ?1",
                [META_LAST_REINDEX],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        drop(conn);
        let db_size = std::fs::metadata(
            Path::new(&self.project_root)
                .join(".aio")
                .join("memory")
                .join("memory.sqlite"),
        )
        .map(|m| m.len())
        .unwrap_or(0);
        Ok(MemoryStats {
            total_facts: total,
            active_facts: active,
            archived_facts: archived,
            superseded_facts: superseded,
            embedded_facts: embedded,
            db_size_bytes: db_size,
            last_reindex_at: last_reindex,
        })
    }

    /// 重建/补建向量：为所有无 embedding 的 active 事实生成向量。
    /// 闭包返回 (模型名, 向量)；async 以支持网络嵌入。
    pub async fn reindex<F, Fut, P>(
        &self,
        mut embed_one: F,
        mut on_progress: P,
    ) -> Result<(usize, usize), String>
    where
        F: FnMut(String) -> Fut,
        Fut: std::future::Future<Output = Result<Option<(String, usize, Vec<f32>)>, String>>,
        P: FnMut(usize, usize) + Send,
    {
        // 嵌入模型/维度变化时，旧向量全部作废，强制全量重建
        {
            let conn = self.conn.lock().map_err(|e| e.to_string())?;
            let meta_dim = conn
                .query_row(
                    "SELECT v FROM meta WHERE k = ?1",
                    [META_EMBEDDING_DIM],
                    |r| r.get::<_, String>(0),
                )
                .optional()
                .map_err(|e| e.to_string())?
                .and_then(|v| v.parse::<usize>().ok());
            let vec_dim = crate::services::memory::vec0::vec0_dim(&conn);
            if meta_dim.is_some() && vec_dim.is_some() && meta_dim != vec_dim {
                conn.execute(
                    "UPDATE facts SET embedding = NULL, embedding_model = NULL, embedding_dim = NULL WHERE embedding IS NOT NULL",
                    [],
                )
                .map_err(|e| e.to_string())?;
                let _ = crate::services::memory::vec0::vec0_clear(&conn);
            }
        }
        let ids: Vec<String> = {
            let conn = self.conn.lock().map_err(|e| e.to_string())?;
            let mut stmt = conn
                .prepare("SELECT id, content FROM facts WHERE status = ?1 AND embedding IS NULL")
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map(params![STATUS_ACTIVE], |r| r.get::<_, String>(0))
                .map_err(|e| e.to_string())?;
            rows.filter_map(|r| r.ok()).collect()
        };
        let mut embedded = 0usize;
        let mut failed = 0usize;
        let total = ids.len();
        for id in ids {
            let content = self.get_fact(&id).map(|f| f.content).unwrap_or_default();
            on_progress(embedded, total);
            match embed_one(content).await {
                Ok(Some((model, dim, v))) => {
                    let conn = self.conn.lock().map_err(|e| e.to_string())?;
                    self.write_embedding(&conn, &id, &(model, dim, v))
                        .map_err(|e| e.to_string())?;
                    drop(conn);
                    embedded += 1;
                }
                Ok(None) => {}
                Err(_) => failed += 1,
            }
        }
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT OR REPLACE INTO meta (k, v) VALUES (?1, ?2)",
            params![META_LAST_REINDEX, now_iso()],
        )
        .map_err(|e| e.to_string())?;
        Ok((embedded, failed))
    }

    /// 从旧版 .aio/knowledge.json 迁移事实（幂等，已迁移则跳过）。
    pub fn migrate_from_knowledge_json(&self) -> Result<usize, String> {
        let migrated: Option<String> = {
            let conn = self.conn.lock().map_err(|e| e.to_string())?;
            conn.query_row(
                "SELECT v FROM meta WHERE k = ?1",
                [META_MIGRATED_KNOWLEDGE],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
        };
        if migrated.as_deref() == Some("1") {
            return Ok(0);
        }
        let knowledge = knowledge::load_knowledge(&self.project_root);
        let mut count = 0usize;
        for entry in knowledge.entries {
            let fact = NewFact {
                key: Some(entry.key.clone()),
                content: entry.content.clone(),
                category: entry.category.clone(),
                importance: 0.5,
                source_type: Some("user".into()),
                source_refs: Some(format!("knowledge.json:{}", entry.key)),
                pinned: false,
            };
            if self.upsert_fact(&fact, None).is_ok() {
                count += 1;
            }
        }
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT OR REPLACE INTO meta (k, v) VALUES (?1, ?2)",
            params![META_MIGRATED_KNOWLEDGE, "1"],
        )
        .map_err(|e| e.to_string())?;
        Ok(count)
    }

    fn fetch_scored(
        &self,
        id: &str,
        score: f32,
        now_secs: u64,
        conn: &Connection,
    ) -> Result<ScoredFact, String> {
        let row: (String, String, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT content, category, source_type, updated_at FROM facts WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .map_err(|e| e.to_string())?;
        let mut final_score = score;
        if let Some(upd) = &row.3 {
            if let Some(secs) = iso_to_secs(upd) {
                let age_days = now_secs.saturating_sub(secs) / 86400;
                if age_days <= 90 {
                    final_score *= 1.1;
                }
            }
        }
        Ok(ScoredFact {
            id: id.to_string(),
            content: row.0,
            category: row.1,
            score: final_score,
            source_type: row.2,
            updated_at: row.3,
        })
    }
}

const META_LAST_REINDEX: &str = "last_reindex_at";
const META_EMBEDDING_DIM: &str = "embedding_dim";
const META_MIGRATED_KNOWLEDGE: &str = "knowledge_json_migrated";
const META_PROJECT_ENABLED: &str = "project_memory_enabled";

/// 简易 token 估算（用于注入预算；精确计数交给 token_counter 的场景除外）。
fn count_approx(text: &str) -> usize {
    text.chars().count() / 2 + 1
}

/// 清洗 FTS 查询：仅保留字母数字（含 CJK），其余替换为空格，避免 FTS 语法错误。
fn sanitize_fts_query(q: &str) -> String {
    let mut out = String::new();
    for c in q.chars() {
        if c.is_alphanumeric() {
            out.push(c);
        } else {
            out.push(' ');
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// YYYY-MM-DDTHH:MM:SS → Unix 秒。
/// 公开包装：ISO 时间转 Unix 秒（供提取管线去抖）。
pub fn iso_to_secs_pub(iso: &str) -> Option<u64> {
    iso_to_secs(iso)
}

fn iso_to_secs(iso: &str) -> Option<u64> {
    let s = iso.trim();
    if s.len() < 19 {
        return None;
    }
    let y: u64 = s[0..4].parse().ok()?;
    let mo: u64 = s[5..7].parse().ok()?;
    let d: u64 = s[8..10].parse().ok()?;
    let h: u64 = s[11..13].parse().ok()?;
    let mi: u64 = s[14..16].parse().ok()?;
    let se: u64 = s[17..19].parse().ok()?;
    Some(days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + se)
}

/// 儒略日换算（Howard Hinnant civil_from_days 逆运算的简化版）。
fn days_from_civil(y: u64, m: u64, d: u64) -> u64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// 在事务内写入版本审计（自动计算下一版本号）。
fn log_version_tx(
    tx: &rusqlite::Transaction<'_>,
    fact_id: &str,
    reason: &str,
    before: Option<&str>,
    after: Option<&str>,
    related: Option<&str>,
    judge_model: &str,
) -> Result<(), String> {
    let version: i64 = tx
        .query_row(
            "SELECT COALESCE(MAX(version), 0) + 1 FROM fact_versions WHERE fact_id = ?1",
            [fact_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    tx.execute(
        "INSERT INTO fact_versions (fact_id, version, reason, content_before, content_after, related_fact_ids, judge_model, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![fact_id, version, reason, before, after, related, judge_model, now_iso()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 当前时间 ISO（复用 knowledge 模块实现）。
fn now_iso() -> String {
    knowledge::now_iso()
}

/// 小型管道辅助（配合闭包链）。
trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}
impl<T> Pipe for T {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_project(tag: &str) -> String {
        let dir =
            std::env::temp_dir().join(format!("aio-memory-test-{tag}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir.to_string_lossy().to_string()
    }

    fn fact(content: &str, key: Option<&str>) -> NewFact {
        NewFact {
            key: key.map(|s| s.to_string()),
            content: content.to_string(),
            category: "note".into(),
            importance: 0.5,
            source_type: None,
            source_refs: None,
            pinned: false,
        }
    }

    #[test]
    fn test_open_and_schema_idempotent() {
        let root = temp_project("schema");
        let s1 = MemoryStore::open(&root).unwrap();
        let s2 = MemoryStore::open(&root).unwrap();
        drop(s1);
        drop(s2);
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn test_upsert_and_get() {
        let root = temp_project("upsert");
        let store = MemoryStore::open(&root).unwrap();
        let saved = store
            .upsert_fact(&fact("项目使用 rusqlite", Some("k1")), None)
            .unwrap();
        assert_eq!(saved.key.as_deref(), Some("k1"));
        let saved2 = store
            .upsert_fact(&fact("项目迁移到 sqlite-vec", Some("k1")), None)
            .unwrap();
        assert_eq!(saved2.id, saved.id);
        assert!(saved2.content.contains("sqlite-vec"));
        let (facts, total) = store.list_facts(Some(STATUS_ACTIVE), None, 10, 0).unwrap();
        assert_eq!(total, 1);
        assert_eq!(facts.len(), 1);
        store.delete_fact(&saved.id).unwrap();
        assert!(store.get_fact(&saved.id).is_err());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn test_hybrid_search_fts() {
        let root = temp_project("fts");
        let store = MemoryStore::open(&root).unwrap();
        store
            .upsert_fact(&fact("数据库使用 rusqlite 存储聊天记录", None), None)
            .unwrap();
        store
            .upsert_fact(&fact("前端使用 SolidJS 与 Tailwind CSS", None), None)
            .unwrap();
        let res = store
            .search_hybrid("rusqlite", None, &SearchOptions::default())
            .unwrap();
        assert!(!res.is_empty());
        assert!(res.iter().any(|f| f.content.contains("rusqlite")));
        let res2 = store
            .search_hybrid("聊天记录", None, &SearchOptions::default())
            .unwrap();
        assert!(
            res2.iter().any(|f| f.content.contains("聊天记录")),
            "trigram 应命中中文子串: {:?}",
            res2.iter().map(|f| f.content.clone()).collect::<Vec<_>>(),
        );
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn test_vector_scoring() {
        let root = temp_project("vector");
        let store = MemoryStore::open(&root).unwrap();
        let v = vec![1.0f32, 0.0, 0.0, 0.0];
        store
            .upsert_fact(&fact("甲", None), Some(("test".into(), 4, v.clone())))
            .unwrap();
        store
            .upsert_fact(
                &fact("乙", None),
                Some(("test".into(), 4, vec![0.0, 1.0, 0.0, 0.0])),
            )
            .unwrap();
        let res = store
            .search_hybrid(
                "查询",
                Some(&v),
                &SearchOptions {
                    category: None,
                    k: 5,
                    min_score: None,
                },
            )
            .unwrap();
        assert_eq!(res.len(), 2);
        assert!(
            res[0].content == "甲",
            "与查询向量相同的应排第一: {:?}",
            res.iter().map(|f| f.content.clone()).collect::<Vec<_>>(),
        );
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn test_hybrid_ndcg_sanity() {
        // 构造 8 条事实（8 维正交基向量）+ 2 个查询，验证向量检索排序质量（NDCG@5=1）
        let root = temp_project("ndcg");
        let store = MemoryStore::open(&root).unwrap();
        let tags = [
            "alpha", "beta", "gamma", "delta", "eps", "zeta", "eta", "theta",
        ];
        for (i, tag) in tags.iter().enumerate() {
            let mut v = vec![0.0f32; 8];
            v[i] = 1.0;
            store
                .upsert_fact(
                    &fact(&format!("feature {tag}"), None),
                    Some(("test".into(), 8, v)),
                )
                .unwrap();
        }
        let mut q1 = vec![0.0f32; 8];
        q1[0] = 1.0;
        let res = store
            .search_hybrid(
                "feature alpha",
                Some(&q1),
                &SearchOptions {
                    category: None,
                    k: 5,
                    min_score: None,
                },
            )
            .unwrap();
        assert_eq!(res[0].content, "feature alpha");
        let ndcg = ndcg_at_k(&res, &["feature alpha"], 5);
        assert!((ndcg - 1.0).abs() < 1e-5, "NDCG@5 应为 1.0，实际 {ndcg}");
        let mut q2 = vec![0.0f32; 8];
        q2[1] = 1.0;
        let res2 = store
            .search_hybrid(
                "feature beta",
                Some(&q2),
                &SearchOptions {
                    category: None,
                    k: 5,
                    min_score: None,
                },
            )
            .unwrap();
        assert_eq!(res2[0].content, "feature beta");
        fs::remove_dir_all(&root).ok();
    }

    fn ndcg_at_k(results: &[ScoredFact], relevant: &[&str], k: usize) -> f64 {
        let k = k.min(results.len());
        let mut dcg = 0.0f64;
        for (i, f) in results.iter().take(k).enumerate() {
            let rel = if relevant.contains(&f.content.as_str()) {
                1.0
            } else {
                0.0
            };
            dcg += rel / (i as f64 + 2.0).log2();
        }
        let mut idcg = 0.0f64;
        for i in 0..relevant.len().min(k) {
            idcg += 1.0 / (i as f64 + 2.0).log2();
        }
        if idcg == 0.0 {
            0.0
        } else {
            dcg / idcg
        }
    }

    #[test]
    fn test_project_enabled_override() {
        let root = temp_project("override");
        let store = MemoryStore::open(&root).unwrap();
        assert_eq!(store.project_enabled().unwrap(), None);
        store.set_project_enabled(Some(true)).unwrap();
        assert_eq!(store.project_enabled().unwrap(), Some(true));
        store.set_project_enabled(Some(false)).unwrap();
        assert_eq!(store.project_enabled().unwrap(), Some(false));
        store.set_project_enabled(None).unwrap();
        assert_eq!(store.project_enabled().unwrap(), None);
        fs::remove_dir_all(&root).ok();
    }
}
