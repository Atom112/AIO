//! Project-level memory store (MemoryStore): four table families (facts / fact_versions / meta / FTS5) + CRUD + hybrid retrieval.
//!
//! One dedicated SQLite file per project: <project root>/.aio/memory/memory.sqlite.
//! Vectors are stored as float32 BLOBs in facts.embedding; retrieval currently uses brute-force cosine in Rust (planned switch to sqlite-vec).

use crate::core::models::{MemoryEmbeddingConfig, MemoryRerankConfig};
use crate::ierr;
use crate::utils::knowledge;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use super::vector::{brute_force_topk, bytes_to_f32, f32_to_bytes};
use crate::services::memory::CollectRowsWarn;

pub const STATUS_ACTIVE: &str = "active";
pub const STATUS_SUPERSEDED: &str = "superseded";
pub const STATUS_ARCHIVED: &str = "archived";

/// A memory fact (API-facing, camelCase serialization).
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

/// Input for creating/updating a fact.
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

/// Fact + vector tuple used by consolidation (id, content, importance, access_count, vector).
pub type FactWithVector = (String, String, f32, i64, Vec<f32>);

/// Memory store statistics.
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
    // ---- self-evolution quality probes for the last 7 days (rolling 7-day window, auto-reset) ----
    /// Number of retrieval queries.
    pub retrieval_queries_7d: i64,
    /// Number of hits returned by retrieval.
    pub retrieval_hits_7d: i64,
    /// Dedup trigger count (remember/extract/consolidate).
    pub dedup_triggers_7d: i64,
    /// Facts captured by automatic extraction.
    pub extracted_7d: i64,
    /// LLM arbitration action distribution (merge/update/supersede/keep_separate).
    pub judge_distribution_7d: std::collections::HashMap<String, i64>,
}

/// A retrieval hit (with fused score).
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

/// Version audit record.
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

/// Search options.
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

/// Per-project memory store.
pub struct MemoryStore {
    conn: Mutex<Connection>,
    project_root: String,
}

/// Meta keys for the project-level embedding config (bound to the vector store, persisted with the project memory DB).
const META_EMBED_PROVIDER: &str = "project_embedding_provider";
const META_EMBED_MODEL: &str = "project_embedding_model";
const META_EMBED_API_URL: &str = "project_embedding_api_url";
/// Project-level rerank config (single-key JSON blob, no api_key).
const META_RERANK: &str = "project_rerank_config";

// ===== quality probe counter keys for the last 7 days (rolling_*, window auto-resets) =====
pub(crate) const STAT_QUERIES: &str = "stat_retrieval_queries";
pub(crate) const STAT_HITS: &str = "stat_retrieval_hits";
pub(crate) const STAT_DEDUP: &str = "stat_dedup_triggers";
pub(crate) const STAT_EXTRACTED: &str = "stat_extracted_facts";
pub(crate) const STAT_JUDGE_MERGE: &str = "stat_judge_merge";
pub(crate) const STAT_JUDGE_UPDATE: &str = "stat_judge_update";
pub(crate) const STAT_JUDGE_SUPERSEDE: &str = "stat_judge_supersede";
pub(crate) const STAT_JUDGE_KEEP: &str = "stat_judge_keep";

/// DDL statements (idempotent; writes are parameterized, DDL contains no string literals).
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
    // Code-level RAG: index file chunks
    "CREATE TABLE IF NOT EXISTS code_chunks (",
    "id TEXT PRIMARY KEY,",
    "file_path TEXT NOT NULL,",
    "start_line INTEGER NOT NULL,",
    "end_line INTEGER NOT NULL,",
    "content TEXT NOT NULL,",
    "symbol TEXT,",
    "embedding BLOB,",
    "embedding_model TEXT,",
    "embedding_dim INTEGER,",
    "created_at TEXT NOT NULL",
    ");",
    "CREATE INDEX IF NOT EXISTS idx_code_path ON code_chunks(file_path);",
    // Note: idx_code_symbol is deliberately NOT created in SCHEMA — on legacy DBs (no symbol
    // column) that CREATE INDEX would fail before the ALTER; the versioned migration run in
    // open_connection creates the index for both new and old DBs.
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
    // Lightweight knowledge graph: entity-relation links between facts (Graphiti-lite; single-hop relations only, no multi-hop queries)
    "CREATE TABLE IF NOT EXISTS facts_relations (",
    "id INTEGER PRIMARY KEY AUTOINCREMENT,",
    "fact_id TEXT NOT NULL REFERENCES facts(id) ON DELETE CASCADE,",
    "entity1 TEXT NOT NULL,",
    "relation TEXT NOT NULL,",
    "entity2 TEXT NOT NULL,",
    "valid_from TEXT,",
    "valid_until TEXT,",
    "created_at TEXT NOT NULL",
    ");",
    "CREATE INDEX IF NOT EXISTS idx_relations_fact ON facts_relations(fact_id);",
    "CREATE INDEX IF NOT EXISTS idx_relations_entity ON facts_relations(entity1, entity2);",
);

mod config;
mod graph;
mod search;
mod stats;
impl MemoryStore {
    /// Opens (or creates) the project memory store: <project root>/.aio/memory/memory.sqlite.
    /// Enables the vec0 virtual table when sqlite-vec is available; otherwise falls back to brute-force cosine.
    pub fn open(project_root: &str) -> Result<Self, String> {
        let conn = Self::open_connection(project_root)?;
        Ok(Self {
            conn: Mutex::new(conn),
            project_root: project_root.to_string(),
        })
    }

    /// Opens a standalone connection to the project memory store (for code.rs and similar; WAL allows concurrent connections).
    pub fn open_connection(project_root: &str) -> Result<Connection, String> {
        // The auto-extension must be registered before opening the connection.
        let _ = crate::services::memory::vec0::vec0_available();
        let dir = Path::new(project_root).join(".aio").join("memory");
        std::fs::create_dir_all(&dir).map_err(|e| {
            ierr!(
                "创建记忆目录失败: {}",
                "Failed to create memory directory: {}",
                e
            )
        })?;
        let path = dir.join("memory.sqlite");
        let conn = Connection::open(&path).map_err(|e| e.to_string())?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| e.to_string())?;
        conn.pragma_update(None, "busy_timeout", 5000)
            .map_err(|e| e.to_string())?;
        conn.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
        // Versioned migrations (progress tracked via user_version).
        // Must run AFTER SCHEMA (CREATE TABLE IF NOT EXISTS) — legacy code_chunks tables
        // have no symbol column, so an inline CREATE INDEX in SCHEMA would fail first.
        crate::core::migrate::run_migrations(&conn, MEMORY_DB_MIGRATIONS)?;
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
                .unwrap_or(384);
            crate::services::memory::vec0::ensure_vec_table(&conn, dim)
                .map_err(|e| e.to_string())?;
        }
        Ok(conn)
    }

    /// Upsert a fact: if the key exists, the fact is updated in place (with vector near-neighbor
    /// arbitration layered on top); embedding Some((model, dim, vector)) also refreshes the
    /// vector, while None keeps the existing vector / skips writing one.
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
                // Version audit: record the old content (reason=update).
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
        // When a vector is explicitly provided (applies to both insert and update; also syncs vec0).
        if let Some(emb) = &embedding {
            self.write_embedding(&tx, &fid, emb)?;
        }
        // FTS5 sync (implicit rowid of the facts table).
        tx.execute(
            "INSERT OR REPLACE INTO fts_facts (rowid, fact_id, content, category, source_type) SELECT rowid, id, content, category, COALESCE(source_type, ?1) FROM facts WHERE id = ?2",
            params!["", fid],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        drop(conn);
        self.get_fact(&fid)
    }

    /// Reads a fact by id.
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
        .ok_or_else(|| ierr!("事实不存在: {}", "Fact not found: {}", id))
    }

    /// Lists facts (optional status/category filters, ordered by updated_at desc, paginated).
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
            .collect_rows("store");
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

    /// Deletes a fact (cascades to version audit and FTS entries).
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

    /// Changes a fact's status (active / superseded / archived).
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
        if status != STATUS_ACTIVE {
            // Also remove the vector and FTS entry so non-active facts no longer participate in retrieval/injection.
            if crate::services::memory::vec0::vec0_available() {
                crate::services::memory::vec0::vec0_delete(&conn, id).map_err(|e| e.to_string())?;
            }
            conn.execute("DELETE FROM fts_facts WHERE fact_id = ?1", [id])
                .map_err(|e| e.to_string())?;
        }
        drop(conn);
        self.get_fact(id)
    }

    /// Pin / unpin a fact.
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

    /// Writes a vector (facts BLOB + vec0 sync; rebuilds the vec0 table when the dimension changes).
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

    /// Updates a fact's vector alone (called after arbitration revises the content; None clears the vector).
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

    /// Exports all facts as JSON (vectors excluded; re-embed after import).
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
        let facts: Vec<serde_json::Value> = rows.collect_rows("store");
        serde_json::to_string_pretty(&serde_json::json!({
            "version": 1,
            "exportedAt": now_iso(),
            "facts": facts,
        }))
        .map_err(|e| e.to_string())
    }

    /// Imports facts from exported JSON (deduplicated by key; returns the number imported).
    pub fn import_facts(&self, json: &str) -> Result<usize, String> {
        let parsed: serde_json::Value = serde_json::from_str(json)
            .map_err(|e| ierr!("解析导入数据失败: {}", "Failed to parse import data: {}", e))?;
        let facts = parsed
            .get("facts")
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                ierr!(
                    "导入数据缺少 facts 数组",
                    "Import data is missing the facts array"
                )
            })?;
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

    /// Directly revises fact content (keeps the same key) and writes a version audit entry (reason=update/merge).
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

    /// Revises a fact's category (keeps the same key) and syncs the FTS row (same strategy as update_content).
    /// Used by the keyless branch of memory_update, which must not silently drop the category parameter.
    pub fn set_category(&self, id: &str, category: &str) -> Result<MemoryFact, String> {
        let now = now_iso();
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE facts SET category = ?1, updated_at = ?2 WHERE id = ?3",
            params![category, now, id],
        )
        .map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE fts_facts SET category = ?1 WHERE fact_id = ?2",
            params![category, id],
        )
        .map_err(|e| e.to_string())?;
        drop(conn);
        self.get_fact(id)
    }

    /// Merges two facts: target is kept (content updated), source is marked superseded with a version audit entry.
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
        tx.execute("DELETE FROM fts_facts WHERE fact_id = ?1", [source_id])
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

    /// Marks a fact as superseded (conflicted/outdated) and records a version audit entry.
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
        // MEM-02 加重项：supersede 与 set_status 一致地清理 vec0（此前只删 FTS，
        // 被取代事实的向量继续参与召回、挤占 top-k 名额）
        if crate::services::memory::vec0::vec0_available() {
            crate::services::memory::vec0::vec0_delete(&tx, id).map_err(|e| e.to_string())?;
        }
        tx.execute("DELETE FROM fts_facts WHERE fact_id = ?1", [id])
            .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        drop(conn);
        self.get_fact(id)
    }

    /// Reads a fact's version audit chain.
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
        Ok(rows.collect_rows("store"))
    }

    /// Capacity governance: when active facts exceed the limit, archive the lowest-scoring non-pinned facts, score = importance times access decay.
    pub fn prune_to_capacity(&self, max_facts: u32) -> Result<usize, String> {
        // MEM-03: collect ids under the lock, then archive via set_status —
        // the historic inline UPDATE only flipped the status column, leaving the
        // vec0 and FTS entries in place; the vec0 search leg has no status filter,
        // so archived facts kept being recalled and eating top-k slots.
        let ids: Vec<String> = {
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
            let mut stmt = conn
                .prepare("SELECT id FROM facts WHERE status = ?1 AND pinned = 0 ORDER BY (importance * (1.0 + 0.1 * MIN(access_count, 20))) ASC, updated_at ASC LIMIT ?2")
                .map_err(|e| e.to_string())?;
            let ids: Vec<String> = stmt
                .query_map(params![STATUS_ACTIVE, excess as i64], |r| {
                    r.get::<_, String>(0)
                })
                .map_err(|e| e.to_string())?
                .collect_rows("store");
            ids
        };
        let now = now_iso();
        let mut archived = 0usize;
        for id in ids {
            if self.set_status(&id, STATUS_ARCHIVED, Some(&now)).is_ok() {
                archived += 1;
            }
        }
        Ok(archived)
    }

    /// Clears all facts (deletes the project memory).
    ///
    /// MEM-05: also clears the relation table, the vec0 facts table and the code-index
    /// tables — the historic clear left ghost vectors (fetch/extract then treated them
    /// as "already deduped" hits and silently dropped new facts) and orphan code chunks.
    /// CASCADE was dead code (foreign_keys is never enabled on the memory DB).
    pub fn clear(&self) -> Result<(), String> {
        let mut conn = self.conn.lock().map_err(|e| e.to_string())?;
        let tx = conn.transaction().map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM facts", [])
            .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM fact_versions", [])
            .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM fts_facts", [])
            .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM facts_relations", [])
            .map_err(|e| e.to_string())?;
        for table in [
            "vec_facts",
            "code_chunks",
            "fts_code",
            "vec_code",
            "code_file_state",
        ] {
            if table_exists(&tx, table) {
                let _ = tx.execute(&format!("DELETE FROM {table}"), []);
            }
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Rebuilds/backfills vectors: generates embeddings for all active facts without one.
    /// The closure returns (model name, vector); async to support network embeddings.
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
        // When the embedding model/dimension changes, all old vectors are invalidated, forcing a full rebuild.
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
            rows.collect_rows("store")
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
}

const META_LAST_REINDEX: &str = "last_reindex_at";
const META_EMBEDDING_DIM: &str = "embedding_dim";
const META_MIGRATED_KNOWLEDGE: &str = "knowledge_json_migrated";
const META_PROJECT_ENABLED: &str = "project_memory_enabled";

/// Rough token estimate (for injection budgets; scenarios needing exact counts go through token_counter).
fn count_approx(text: &str) -> usize {
    text.chars().count() / 2 + 1
}

/// Sanitizes an FTS query: keep only alphanumerics (including CJK), replace the rest with spaces to avoid FTS syntax errors.
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

/// YYYY-MM-DDTHH:MM:SS → Unix seconds (for extract pipeline debouncing).
/// Implementation is provided centrally by utils::time.
pub fn iso_to_secs_pub(iso: &str) -> Option<u64> {
    crate::utils::time::iso_to_secs(iso)
}

/// YYYY-MM-DDTHH:MM:SS → Unix seconds (internal use in this module).
fn iso_to_secs(iso: &str) -> Option<u64> {
    crate::utils::time::iso_to_secs(iso)
}

/// Writes a version audit entry inside a transaction (auto-computes the next version number).
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

/// Current time as ISO (provided centrally by utils::time).
fn now_iso() -> String {
    crate::utils::time::now_iso()
}

/// MEM-05 辅助：表是否存在（vec0/FTS 虚表按需建，DELETE 前必须确认存在）。
fn table_exists(conn: &Connection, name: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name = ?1",
        [name],
        |r| r.get::<_, i64>(0),
    )
    .map(|n| n > 0)
    .unwrap_or(false)
}

/// Small pipeline helper (for closure chains).
trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}
impl<T> Pipe for T {}

// ====== Versioned migrations (memory.sqlite, progress tracked via user_version) ======

/// v1: add the symbol column to legacy code_chunks and create the symbol index (idempotent).
fn m_code_chunks_symbol(conn: &rusqlite::Connection) -> Result<(), String> {
    let has_symbol: bool = {
        let mut stmt = conn
            .prepare("PRAGMA table_info(code_chunks)")
            .map_err(|e| e.to_string())?;
        let cols: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(1))
            .map_err(|e| e.to_string())?
            .collect_rows("store");
        cols.iter().any(|c| c == "symbol")
    };
    if !has_symbol {
        // SQLite ALTER ADD COLUMN has no IF NOT EXISTS support, so it must be run separately.
        conn.execute_batch("ALTER TABLE code_chunks ADD COLUMN symbol TEXT")
            .map_err(|e| e.to_string())?;
    }
    conn.execute_batch("CREATE INDEX IF NOT EXISTS idx_code_symbol ON code_chunks(symbol)")
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Migration table for the memory DB (strictly increasing; new migrations may only be appended, published entries must not be modified).
const MEMORY_DB_MIGRATIONS: &[crate::core::migrate::Migration] =
    &[crate::core::migrate::Migration {
        version: 1,
        name: "code_chunks.symbol 列与符号索引",
        run: m_code_chunks_symbol,
    }];

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

    /// MEM-9: the expiring meta lock is claimed atomically — a fresh lock blocks the
    /// second claimant; a lock older than the cutoff is re-claimable.
    #[test]
    fn test_meta_try_lock_is_atomic_and_expiring() {
        let root = temp_project("metalock");
        let s = MemoryStore::open(&root).unwrap();
        let t0 = crate::utils::time::now_unix_secs();
        let cutoff = |now: u64| crate::utils::time::secs_to_iso(now.saturating_sub(60));
        // 首次认领成功
        assert!(s
            .meta_try_lock("lock_t", &cutoff(t0), &crate::utils::time::secs_to_iso(t0))
            .unwrap());
        // 持锁期内（TTL 内）再次认领必须失败
        assert!(!s
            .meta_try_lock("lock_t", &cutoff(t0), &crate::utils::time::secs_to_iso(t0))
            .unwrap());
        // 70 秒后的视角：cutoff = t0+10 > 已存的 t0 → 锁已过期，可重新认领
        let later = t0 + 70;
        assert!(s
            .meta_try_lock(
                "lock_t",
                &cutoff(later),
                &crate::utils::time::secs_to_iso(later)
            )
            .unwrap());
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
        // Build 8 facts (8-dim orthogonal basis vectors) + 2 queries to verify vector retrieval ranking quality (NDCG@5=1).
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

    // ===== Regression: dedup threshold semantics (cosine, not the RRF fused score) =====
    #[test]
    fn test_search_similar_returns_cosine_not_rrf() {
        let root = temp_project("similar");
        let store = MemoryStore::open(&root).unwrap();
        let v1 = vec![1.0f32, 0.0, 0.0, 0.0];
        let v2 = vec![0.0f32, 1.0, 0.0, 0.0];
        let saved = store
            .upsert_fact(
                &fact("数据库使用 rusqlite", None),
                Some(("test".into(), 4, v1.clone())),
            )
            .unwrap();
        store
            .upsert_fact(
                &fact("前端使用 SolidJS", None),
                Some(("test".into(), 4, v2)),
            )
            .unwrap();
        let dups = store.search_similar(&v1, 3).unwrap();
        assert_eq!(dups.len(), 2);
        assert_eq!(dups[0].0, saved.id);
        let (id, cosine) = &dups[0];
        assert!(
            (*cosine - 1.0).abs() < 1e-5,
            "相同向量余弦应为 ~1.0，实际 {cosine}（若为 RRF 融合分则 <0.05，仲裁永不触发）"
        );
        assert!(!id.is_empty());
        fs::remove_dir_all(&root).ok();
    }

    // ===== Regression: category filter takes effect =====
    #[test]
    fn test_category_filter_applies() {
        let root = temp_project("category");
        let store = MemoryStore::open(&root).unwrap();
        let mut d = fact("使用 rusqlite 存储", None);
        d.category = "decision".into();
        store.upsert_fact(&d, None).unwrap();
        let mut n = fact("今天天气不错", None);
        n.category = "note".into();
        store.upsert_fact(&n, None).unwrap();
        let res = store
            .search_hybrid(
                "存储",
                None,
                &SearchOptions {
                    category: Some("decision".into()),
                    k: 10,
                    min_score: None,
                },
            )
            .unwrap();
        assert!(!res.is_empty());
        assert!(
            res.iter().all(|f| f.category == "decision"),
            "category 过滤应生效: {:?}",
            res.iter().map(|f| f.category.clone()).collect::<Vec<_>>()
        );
        fs::remove_dir_all(&root).ok();
    }

    // ===== Regression: short queries (<= 2 chars, no trigram match) go through LIKE =====
    #[test]
    fn test_short_query_uses_like() {
        let root = temp_project("shortq");
        let store = MemoryStore::open(&root).unwrap();
        store
            .upsert_fact(&fact("重构了缓存层，加入 LRU", None), None)
            .unwrap();
        let res = store
            .search_hybrid(
                "重构",
                None,
                &SearchOptions {
                    category: None,
                    k: 10,
                    min_score: None,
                },
            )
            .unwrap();
        assert!(
            res.iter().any(|f| f.content.contains("重构")),
            "2 字中文查询应经 LIKE 命中: {:?}",
            res.iter().map(|f| f.content.clone()).collect::<Vec<_>>()
        );
        fs::remove_dir_all(&root).ok();
    }

    // ===== Regression: superseded/archived facts no longer participate in retrieval (both FTS and vector paths) =====
    #[test]
    fn test_superseded_excluded_from_search() {
        let root = temp_project("status");
        let store = MemoryStore::open(&root).unwrap();
        let v = vec![1.0f32, 0.0, 0.0, 0.0];
        let saved = store
            .upsert_fact(
                &fact("数据库使用 rusqlite 存储聊天记录", None),
                Some(("test".into(), 4, v.clone())),
            )
            .unwrap();
        // First verify it is retrievable, then verify it is not after archiving.
        assert!(!store
            .search_hybrid(
                "rusqlite",
                None,
                &SearchOptions {
                    category: None,
                    k: 10,
                    min_score: None
                }
            )
            .unwrap()
            .is_empty());
        store.set_status(&saved.id, STATUS_ARCHIVED, None).unwrap();
        let after_archive = store
            .search_hybrid(
                "rusqlite",
                None,
                &SearchOptions {
                    category: None,
                    k: 10,
                    min_score: None,
                },
            )
            .unwrap();
        assert!(
            after_archive.iter().all(|f| f.id != saved.id),
            "archived 事实不应再被检索: {:?}",
            after_archive
                .iter()
                .map(|f| f.content.clone())
                .collect::<Vec<_>>()
        );
        // The vector path excludes it as well.
        let vec_res = store.search_similar(&v, 10).unwrap();
        assert!(vec_res.iter().all(|(id, _)| *id != saved.id));
        fs::remove_dir_all(&root).ok();
    }

    // ===== Regression: MEM-03 —— prune 走 set_status（归档清理 vec0/FTS）=====
    #[test]
    fn test_prune_archives_via_set_status_cleansing_retrieval() {
        let root = temp_project("prune");
        let store = MemoryStore::open(&root).unwrap();
        let v = vec![1.0f32, 0.0, 0.0, 0.0];
        for i in 0..3 {
            store
                .upsert_fact(
                    &fact(&format!("低分事实 {i}：无足轻重的描述"), None),
                    Some(("test".into(), 4, v.clone())),
                )
                .unwrap();
        }
        // 容量 1：必须归档 2 条最低分事实
        let archived = store.prune_to_capacity(1).unwrap();
        assert_eq!(archived, 2);
        // 关键回归：归档后向量/FTS 检索不再召回（历史实现只改 status 列，
        // vec0 腿无 status 过滤，归档事实继续被召回挤占 top-k）
        let vec_res = store.search_similar(&v, 10).unwrap();
        assert!(
            vec_res.len() <= 1,
            "归档事实的向量不应继续参与召回: {vec_res:?}"
        );
        let fts_res = store
            .search_hybrid(
                "低分事实",
                None,
                &SearchOptions {
                    category: None,
                    k: 10,
                    min_score: None,
                },
            )
            .unwrap();
        assert!(
            fts_res.len() <= 1,
            "归档事实不应继续被检索（容量 1 只应剩 1 条）: {:?}",
            fts_res
                .iter()
                .map(|f| f.content.clone())
                .collect::<Vec<_>>()
        );
        fs::remove_dir_all(&root).ok();
    }

    // ===== Regression: MEM-05 —— clear 清掉 vec_facts/facts_relations 幽灵 =====
    #[test]
    fn test_clear_removes_vector_and_relation_ghosts() {
        let root = temp_project("clear");
        let store = MemoryStore::open(&root).unwrap();
        let v = vec![1.0f32, 0.0, 0.0, 0.0];
        store
            .upsert_fact(
                &fact("clear 测试：会被清空的事实", None),
                Some(("test".into(), 4, v.clone())),
            )
            .unwrap();
        assert!(!store.search_similar(&v, 3).unwrap().is_empty());
        store.clear().unwrap();
        // 关键回归：向量幽灵被清——旧实现 clear 不删 vec_facts，与幽灵相似的新事实
        // 在 extract 判重时被"读取失败→已判重"静默丢弃
        assert!(
            store.search_similar(&v, 3).unwrap().is_empty(),
            "clear 后不应残留向量幽灵"
        );
        assert!(store.list_facts(None, None, 100, 0).unwrap().0.is_empty());
        fs::remove_dir_all(&root).ok();
    }

    // ===== Regression: 7-day rolling quality probes =====
    #[test]
    fn test_rolling_stats_and_stats_exposure() {
        let root = temp_project("rolling");
        let store = MemoryStore::open(&root).unwrap();
        assert_eq!(store.rolling_get(STAT_QUERIES), 0);
        store.rolling_bump(STAT_QUERIES, 1);
        store.rolling_bump(STAT_QUERIES, 2);
        assert_eq!(store.rolling_get(STAT_QUERIES), 3);
        store.rolling_bump(STAT_JUDGE_MERGE, 1);
        let stats = store.stats().unwrap();
        assert_eq!(stats.retrieval_queries_7d, 3);
        assert_eq!(stats.judge_distribution_7d.get("merge"), Some(&1));
        fs::remove_dir_all(&root).ok();
    }

    // ===== Regression: reading active facts with vectors (consolidation input) =====
    #[test]
    fn test_list_active_facts_with_vectors() {
        let root = temp_project("vecs");
        let store = MemoryStore::open(&root).unwrap();
        let v = vec![1.0f32, 0.0, 0.0, 0.0];
        let s1 = store
            .upsert_fact(&fact("A", None), Some(("test".into(), 4, v.clone())))
            .unwrap();
        let s2 = store.upsert_fact(&fact("无向量", None), None).unwrap();
        let rows = store.list_active_facts_with_vectors().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, s1.id);
        // Facts without vectors are excluded from the list.
        assert_ne!(rows[0].0, s2.id);
        // Archived facts no longer appear in the list.
        store.set_status(&s1.id, STATUS_ARCHIVED, None).unwrap();
        assert!(store.list_active_facts_with_vectors().unwrap().is_empty());
        fs::remove_dir_all(&root).ok();
    }

    // ===== Related: injection block budget trimming stays stable =====
    #[test]
    fn test_build_block_from_budget() {
        let root = temp_project("block");
        let store = MemoryStore::open(&root).unwrap();
        store
            .upsert_fact(&fact("决策：使用 rusqlite 作为存储", None), None)
            .unwrap();
        store
            .upsert_fact(&fact("决策：P2 引入 sqlite-vec", None), None)
            .unwrap();
        let res = store
            .search_hybrid(
                "决策",
                None,
                &SearchOptions {
                    category: None,
                    k: 10,
                    min_score: None,
                },
            )
            .unwrap();
        let block = store.build_block_from(res.clone(), 500).unwrap().unwrap();
        assert!(block.starts_with("项目记忆"));
        assert!(block.contains("rusqlite"));
        // An extremely low budget should still fit at least the first fact.
        let tiny = store.build_block_from(res.clone(), 4).unwrap();
        assert!(tiny.is_some());
        fs::remove_dir_all(&root).ok();
    }

    // ===== Regression: legacy DB (code_chunks without the symbol column) opens cleanly on upgrade (migration ordering) =====
    #[test]
    fn test_open_migrates_legacy_db_without_symbol_column() {
        let root = temp_project("legacy");
        let dir = Path::new(&root).join(".aio").join("memory");
        std::fs::create_dir_all(&dir).unwrap();
        // Build a legacy DB: code_chunks without the symbol column (a real schema subset from an earlier version).
        let conn = rusqlite::Connection::open(dir.join("memory.sqlite")).unwrap();
        conn.execute_batch(
            "CREATE TABLE code_chunks (id TEXT PRIMARY KEY, file_path TEXT NOT NULL, start_line INTEGER NOT NULL, end_line INTEGER NOT NULL, content TEXT NOT NULL, embedding BLOB, embedding_model TEXT, embedding_dim INTEGER, created_at TEXT NOT NULL);",
        )
        .unwrap();
        drop(conn);

        // Upgrade path: open must succeed (it previously failed when a CREATE INDEX in SCHEMA referenced a missing column).
        let store = MemoryStore::open(&root).unwrap();
        let conn2 = MemoryStore::open_connection(&root).unwrap();
        let cols: Vec<String> = {
            let mut stmt = conn2.prepare("PRAGMA table_info(code_chunks)").unwrap();
            stmt.query_map([], |r| r.get::<_, String>(1))
                .unwrap()
                .collect_rows("store")
        };
        assert!(
            cols.iter().any(|c| c == "symbol"),
            "旧库应补齐 symbol 列: {cols:?}"
        );
        let index_count: i64 = conn2
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_code_symbol'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(index_count, 1, "idx_code_symbol 索引应存在");
        // New tables (facts_relations) are created as well on the upgrade path.
        let rel_count: i64 = conn2
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='facts_relations'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rel_count, 1);
        drop(conn2);
        let _ = store;
        fs::remove_dir_all(&root).ok();
    }

    // ===== Fresh DB path: the symbol column exists from the start, index present (idempotent) =====
    #[test]
    fn test_fresh_db_has_symbol_column_and_index() {
        let root = temp_project("fresh");
        let store = MemoryStore::open(&root).unwrap();
        let conn = MemoryStore::open_connection(&root).unwrap();
        let cols: Vec<String> = {
            let mut stmt = conn.prepare("PRAGMA table_info(code_chunks)").unwrap();
            stmt.query_map([], |r| r.get::<_, String>(1))
                .unwrap()
                .collect_rows("store")
        };
        assert!(cols.iter().any(|c| c == "symbol"));
        let index_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_code_symbol'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(index_count, 1);
        let _ = store;
        fs::remove_dir_all(&root).ok();
    }

    /// Regression: the keyless "update content + rewrite vector" path must not create duplicate rows
    /// (upsert_fact(key: None) always inserts, so rewriting a vector through it would add a new fact per edit).
    #[test]
    fn test_update_then_set_embedding_does_not_duplicate() {
        let root = temp_project("no_dup");
        let store = MemoryStore::open(&root).unwrap();
        let saved = store.upsert_fact(&fact("原始内容", None), None).unwrap();
        let (_, n0) = store.list_facts(Some(STATUS_ACTIVE), None, 100, 0).unwrap();

        store
            .update_content(&saved.id, "更新后的内容", "update")
            .unwrap();
        store
            .set_embedding(
                &saved.id,
                Some(("test-model".to_string(), 3, vec![0.1, 0.2, 0.3])),
            )
            .unwrap();

        let (_, n1) = store.list_facts(Some(STATUS_ACTIVE), None, 100, 0).unwrap();
        assert_eq!(n0, n1, "更新不得产生重复行");
        let got = store.get_fact(&saved.id).unwrap();
        assert_eq!(got.content, "更新后的内容");
        assert_eq!(got.embedding_model.as_deref(), Some("test-model"));
        fs::remove_dir_all(&root).ok();
    }

    /// set_category directly revises the category and syncs the FTS row.
    #[test]
    fn test_set_category() {
        let root = temp_project("set_cat");
        let store = MemoryStore::open(&root).unwrap();
        let saved = store
            .upsert_fact(&fact("带类别的事实", None), None)
            .unwrap();
        assert_eq!(saved.category, "note");
        let updated = store.set_category(&saved.id, "decision").unwrap();
        assert_eq!(updated.category, "decision");
        let got = store.get_fact(&saved.id).unwrap();
        assert_eq!(got.category, "decision");
        fs::remove_dir_all(&root).ok();
    }
}
