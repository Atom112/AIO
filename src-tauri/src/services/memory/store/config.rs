//! 配置与 meta 域：每项目启用开关、嵌入/重排配置、meta 表、迁移（从 store.rs 拆出）。
use super::*;
use crate::ierr;

impl MemoryStore {
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

    /// 读取 meta 键值。
    /// 读取项目级嵌入配置（未设置时返回 (本地默认, false)）。
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
                    // 兼容旧配置：历史 provider 名为 "openai_compat" / "online" / "ollama"，
                    // 自 v0.8 起全部归一化为 local（在线嵌入已移除），模型名交由解析层迁移。
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

    /// 写入项目级嵌入配置（provider/model/apiUrl/维度；apiKey 不落盘，仍在 secure_store）。
    pub fn set_embedding_cfg(&self, cfg: &MemoryEmbeddingConfig) -> Result<(), String> {
        self.meta_put(META_EMBED_PROVIDER, cfg.provider.as_str())?;
        self.meta_put(META_EMBED_MODEL, cfg.model.as_str())?;
        self.meta_put(META_EMBED_API_URL, cfg.api_url.as_str())?;
        self.meta_put(
            META_EMBEDDING_DIM,
            &cfg.dimensions.clamp(1, 8192).to_string(),
        )
    }

    /// 读取项目级重排配置（未设置时返回 (默认, false)）。旧 JSON 中的在线字段
    /// （apiUrl/apiKey）随结构体收敛被 serde 忽略；provider 统一归一化为 local。
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

    /// 写入项目级重排配置（provider/model；写入侧统一归一化 provider 为 local）。
    pub fn set_rerank_cfg(&self, cfg: &MemoryRerankConfig) -> Result<(), String> {
        let mut c = cfg.clone();
        c.provider = "local".into();
        let json = serde_json::to_string(&c).map_err(|e| e.to_string())?;
        self.meta_put(META_RERANK, &json)
    }

    /// 清空全部向量（切换嵌入模型后调用）：事实与代码块保留文本，仅弃用向量；
    /// vec0/vec_code 表删除，由后续写入按新维度重建。
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

    /// 写入 meta 键值。
    pub fn meta_put(&self, key: &str, value: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        self.meta_put_inner(&conn, key, value)
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

    /// 项目记忆库当前的向量维度（meta 记录；从未写入向量时为 None）。
    pub fn embedding_dim_meta(&self) -> Option<usize> {
        self.meta_get(META_EMBEDDING_DIM)
            .ok()
            .flatten()
            .and_then(|v| v.parse::<usize>().ok())
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
}
