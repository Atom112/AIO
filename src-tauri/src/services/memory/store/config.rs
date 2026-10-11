//! Config and meta domain: per-project enable flag, embedding/rerank config, meta table, migrations (split from store.rs).
use super::*;
use crate::ierr;

impl MemoryStore {
    /// Read this project's memory toggle (meta override; None = follow the global default).
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

    /// Set this project's memory toggle (None = clear the override and follow the global default).
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

    /// Read a meta key's value.
    /// Read the project-level embedding config (returns (local default, false) when unset).
    pub fn embedding_cfg(&self) -> (MemoryEmbeddingConfig, bool) {
        let provider = self.meta_get(META_EMBED_PROVIDER).ok().flatten();
        let model = self.meta_get(META_EMBED_MODEL).ok().flatten();
        let api_url = self.meta_get(META_EMBED_API_URL).ok().flatten();
        let dims = self
            .meta_get(META_EMBEDDING_DIM)
            .ok()
            .flatten()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(384);
        match provider {
            Some(_) => (
                MemoryEmbeddingConfig {
                    // Backward compat: legacy provider names were "openai_compat" / "online" / "ollama";
                    // since v0.8 all normalize to local (online embedding was removed); model names are migrated by the resolution layer.
                    provider: "local".into(),
                    model: model.unwrap_or_default(),
                    api_url: api_url.unwrap_or_default(),
                    dimensions: dims,
                    enabled: true,
                    api_key: String::new(),
                },
                true,
            ),
            None => (MemoryEmbeddingConfig::default(), false),
        }
    }

    /// Write the project-level embedding config (provider/model/apiUrl/dimensions; apiKey is not persisted here, it stays in secure_store).
    pub fn set_embedding_cfg(&self, cfg: &MemoryEmbeddingConfig) -> Result<(), String> {
        self.meta_put(META_EMBED_PROVIDER, cfg.provider.as_str())?;
        self.meta_put(META_EMBED_MODEL, cfg.model.as_str())?;
        self.meta_put(META_EMBED_API_URL, cfg.api_url.as_str())?;
        self.meta_put(
            META_EMBEDDING_DIM,
            &cfg.dimensions.clamp(1, 8192).to_string(),
        )
    }

    /// Read the project-level rerank config (returns (default, false) when unset). Online fields in old JSON
    /// (apiUrl/apiKey) are ignored by serde now that the struct was narrowed; provider is always normalized to local.
    pub fn rerank_cfg(&self) -> (MemoryRerankConfig, bool) {
        match self.meta_get(META_RERANK).ok().flatten() {
            Some(json) => match serde_json::from_str::<MemoryRerankConfig>(&json) {
                Ok(mut cfg) => {
                    cfg.provider = "local".into();
                    (cfg, true)
                }
                Err(_) => (MemoryRerankConfig::default(), false),
            },
            None => (MemoryRerankConfig::default(), false),
        }
    }

    /// Write the project-level rerank config (provider/model; provider is always normalized to local on write).
    pub fn set_rerank_cfg(&self, cfg: &MemoryRerankConfig) -> Result<(), String> {
        let mut c = cfg.clone();
        c.provider = "local".into();
        let json = serde_json::to_string(&c).map_err(|e| e.to_string())?;
        self.meta_put(META_RERANK, &json)
    }

    /// Clear all vectors (called after switching embedding models): facts and code chunks keep their text, only vectors are dropped;
    /// the vec0/vec_code tables are deleted and rebuilt with the new dimensions by later writes.
    pub fn reset_vectors(&self) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute_batch(
            "UPDATE facts SET embedding = NULL, embedding_model = NULL, embedding_dim = NULL WHERE embedding IS NOT NULL;              UPDATE code_chunks SET embedding = NULL, embedding_model = NULL, embedding_dim = NULL WHERE embedding IS NOT NULL;              DROP TABLE IF EXISTS vec_facts; DROP TABLE IF EXISTS vec_code;",
        )
        .map_err(|e| ierr!("清空向量失败: {}", "Failed to clear vectors: {}", e))
    }

    pub fn meta_get(&self, key: &str) -> Result<Option<String>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.query_row("SELECT v FROM meta WHERE k = ?1", [key], |r| r.get(0))
            .optional()
            .map_err(|e| e.to_string())
    }

    /// Write a meta key's value.
    pub fn meta_put(&self, key: &str, value: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        self.meta_put_inner(&conn, key, value)
    }

    /// MEM-9 atomic expiring-lock claim: one statement does the whole check-and-set
    /// (INSERT … ON CONFLICT DO UPDATE … WHERE old value <= cutoff), so two
    /// concurrent claimants cannot both pass a separate read-then-write sequence.
    /// ISO 8601 timestamps in one format sort lexicographically, making the
    /// comparison valid directly in SQL. Returns true when this caller holds the lock.
    pub fn meta_try_lock(
        &self,
        key: &str,
        cutoff_iso: &str,
        now_value: &str,
    ) -> Result<bool, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let claimed = conn
            .execute(
                "INSERT INTO meta (k, v) VALUES (?1, ?2)
                 ON CONFLICT(k) DO UPDATE SET v = ?2 WHERE meta.v <= ?3",
                params![key, now_value, cutoff_iso],
            )
            .map_err(|e| e.to_string())?;
        Ok(claimed > 0)
    }

    pub(crate) fn meta_put_inner(
        &self,
        conn: &Connection,
        key: &str,
        value: &str,
    ) -> Result<(), String> {
        conn.execute(
            "INSERT OR REPLACE INTO meta (k, v) VALUES (?1, ?2)",
            params![key, value],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Current embedding dimension of the project memory store (tracked in meta; None if no vector has ever been written).
    pub fn embedding_dim_meta(&self) -> Option<usize> {
        self.meta_get(META_EMBEDDING_DIM)
            .ok()
            .flatten()
            .and_then(|v| v.parse::<usize>().ok())
    }

    /// Migrate facts from the legacy .aio/knowledge.json (idempotent; skipped when already migrated).
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
}
