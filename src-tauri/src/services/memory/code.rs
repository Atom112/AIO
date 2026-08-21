//! 代码级 RAG（P4）：把项目关键文件分块向量化，支撑 search_code 工具与代码语义检索。
//!
//! 与事实库（facts）并存于同一 memory.sqlite：code_chunks + fts_code + vec_code。
//! 索引按需触发（memory_code_index / 记忆面板按钮），增量基于文件 mtime/size。

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter};

/// 索引结果统计。
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CodeIndexResult {
    pub files: usize,
    pub chunks: usize,
    pub embedded: usize,
    pub failed: usize,
    pub skipped: usize,
    pub elapsed_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// 检索命中的代码块。
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CodeChunkHit {
    pub chunk_id: String,
    pub file_path: String,
    pub start_line: i64,
    pub end_line: i64,
    pub content: String,
    pub score: f32,
}

/// 索引状态。
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CodeIndexStatus {
    pub chunks: i64,
    pub files: i64,
    pub indexed_files: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_index_at: Option<String>,
}

const SKIP_DIRS: [&str; 12] = [
    ".git",
    ".aio",
    "node_modules",
    "target",
    "dist",
    "build",
    ".venv",
    "venv",
    "__pycache__",
    ".idea",
    ".vscode",
    ".temp",
];

const ALLOWED_EXT: [&str; 34] = [
    "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "py", "go", "java", "c", "cpp", "h", "hpp", "cs",
    "rb", "php", "swift", "kt", "vue", "css", "scss", "md", "json", "toml", "yaml", "yml", "sql",
    "sh", "html", "htm", "xml", "proto", "prisma",
];
const MAX_FILE_BYTES: u64 = 200 * 1024;
const CHUNK_LINES: usize = 60;
const CHUNK_OVERLAP: usize = 10;
const META_CODE_LAST_INDEX: &str = "code_last_index_at";

fn open_conn(project_root: &str) -> Result<Connection, String> {
    crate::services::memory::store::MemoryStore::open_connection(project_root)
}

/// 索引项目代码（增量：mtime/size 未变化且非 force 时跳过）。
pub async fn index_code(
    app: &AppHandle,
    project_root: &str,
    force: bool,
    mut on_progress: impl FnMut(usize, usize) + Send,
) -> Result<CodeIndexResult, String> {
    let started = std::time::Instant::now();
    let conn = open_conn(project_root)?;
    ensure_code_vec_table(app, &conn)?;
    let files = collect_files(project_root);
    let mut result = CodeIndexResult {
        files: files.len(),
        chunks: 0,
        embedded: 0,
        failed: 0,
        skipped: 0,
        elapsed_ms: 0,
        error: None,
    };
    let mut done = 0usize;
    for f in &files {
        let meta = std::fs::metadata(f).map_err(|e| format!("读取文件元数据失败: {e}"))?;
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0);
        let size = meta.len();
        if !force {
            let unchanged = conn
                .query_row(
                    "SELECT 1 FROM code_file_state WHERE file_path = ?1 AND mtime_nanos = ?2 AND size_bytes = ?3",
                    params![f.to_string_lossy(), mtime, size as i64],
                    |_| Ok(()),
                )
                .optional()
                .map_err(|e| e.to_string())?
                .is_some();
            if unchanged {
                result.skipped += 1;
                done += 1;
                on_progress(done, files.len().max(1));
                continue;
            }
        }
        let content = match std::fs::read_to_string(f) {
            Ok(c) => c,
            Err(_) => {
                result.failed += 1;
                done += 1;
                continue;
            }
        };
        let chunks = chunk_content(&content);
        conn.execute(
            "DELETE FROM fts_code WHERE file_path = ?1",
            [f.to_string_lossy()],
        )
        .map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM code_chunks WHERE file_path = ?1",
            [f.to_string_lossy()],
        )
        .map_err(|e| e.to_string())?;
        let texts: Vec<String> = chunks.iter().map(|c| c.content.clone()).collect();
        let vecs = embed_batch(app, &texts).await;
        for (i, chunk) in chunks.iter().enumerate() {
            let id = uuid::Uuid::new_v4().to_string();
            let vec = vecs.get(i).cloned().flatten();
            conn.execute(
                "INSERT INTO code_chunks (id, file_path, start_line, end_line, content, embedding, embedding_model, embedding_dim, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    id,
                    f.to_string_lossy(),
                    chunk.start_line as i64,
                    chunk.end_line as i64,
                    chunk.content,
                    vec.as_ref().map(|v| crate::services::memory::vector::f32_to_bytes(v)),
                    vec.as_ref().map(|_| ""),
                    vec.as_ref().map(|v| v.len() as i64),
                    crate::utils::knowledge::now_iso(),
                ],
            )
            .map_err(|e| e.to_string())?;
            if let Some(v) = vec {
                let _ = crate::services::memory::vec0::vec0_upsert_code(&conn, &id, &v);
                result.embedded += 1;
            }
            conn.execute(
                "INSERT OR REPLACE INTO fts_code (rowid, chunk_id, content, file_path) SELECT rowid, id, content, file_path FROM code_chunks WHERE id = ?1",
                [&id],
            )
            .map_err(|e| e.to_string())?;
            result.chunks += 1;
        }
        conn.execute(
            "INSERT OR REPLACE INTO code_file_state (file_path, mtime_nanos, size_bytes) VALUES (?1, ?2, ?3)",
            params![f.to_string_lossy(), mtime, size as i64],
        )
        .map_err(|e| e.to_string())?;
        done += 1;
        on_progress(done, files.len().max(1));
    }
    conn.execute(
        "INSERT OR REPLACE INTO meta (k, v) VALUES (?1, ?2)",
        params![META_CODE_LAST_INDEX, crate::utils::knowledge::now_iso()],
    )
    .map_err(|e| e.to_string())?;
    result.elapsed_ms = started.elapsed().as_millis() as u64;
    let _ = app.emit(
        "code-index-progress",
        serde_json::json!({ "status": "done", "chunks": result.chunks, "files": result.files, "elapsedMs": result.elapsed_ms }),
    );
    Ok(result)
}

/// 代码语义检索：向量 + FTS5 RRF 融合。
pub fn search_code(
    app: &AppHandle,
    project_root: &str,
    query: &str,
    k: usize,
) -> Result<Vec<CodeChunkHit>, String> {
    let conn = open_conn(project_root)?;
    let k = k.clamp(1, 30);
    let mut vec_ranks: HashMap<String, usize> = HashMap::new();
    let cfg = crate::commands::config::load_app_config(app.clone()).unwrap_or_default();
    let mut ecfg = cfg.memory_embedding;
    if ecfg.provider == "openai_compat" && ecfg.api_key.is_empty() {
        if let Ok(Some(key)) = crate::core::secure_store::get(app, "embedding_api_key") {
            ecfg.api_key = key;
        }
    }
    if let Ok(embedder) = crate::plugins::embed::resolve(&ecfg) {
        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            let q = query.to_string();
            let qv = rt.block_on(async {
                embedder
                    .embed(&[q])
                    .await
                    .ok()
                    .and_then(|v| v.into_iter().next())
            });
            if let Some(qv) = qv {
                if let Ok(rows) = crate::services::memory::vec0::vec0_search_code(&conn, &qv, k * 2)
                {
                    for (i, (id, _)) in rows.iter().enumerate() {
                        vec_ranks.insert(id.clone(), i);
                    }
                }
            }
        }
    }
    let mut fts_ranks: HashMap<String, usize> = HashMap::new();
    let sanitized: String = query
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    let sanitized = sanitized.split_whitespace().collect::<Vec<_>>().join(" ");
    if !sanitized.is_empty() {
        if let Ok(mut stmt) = conn.prepare("SELECT chunk_id FROM fts_code WHERE fts_code MATCH ?1 ORDER BY bm25(fts_code) LIMIT ?2") {
            if let Ok(rows) = stmt.query_map(params![sanitized, (k * 2) as i64], |r| r.get::<_, String>(0)) {
                for (i, id) in rows.filter_map(|r| r.ok()).enumerate() {
                    fts_ranks.insert(id, i);
                }
            }
        }
    }
    let mut rrf: HashMap<String, f32> = HashMap::new();
    for (id, rank) in vec_ranks.iter().chain(fts_ranks.iter()) {
        *rrf.entry(id.clone()).or_insert(0.0) += 1.0 / (60.0 + *rank as f32 + 1.0);
    }
    let mut out: Vec<CodeChunkHit> = Vec::new();
    for (id, score) in rrf {
        let row: Option<(String, i64, i64, String)> = conn
            .query_row(
                "SELECT file_path, start_line, end_line, content FROM code_chunks WHERE id = ?1",
                [&id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if let Some((file_path, start_line, end_line, content)) = row {
            out.push(CodeChunkHit {
                chunk_id: id,
                file_path,
                start_line,
                end_line,
                content: content.chars().take(600).collect(),
                score,
            });
        }
    }
    out.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out.truncate(k);
    Ok(out)
}

/// 代码索引状态。
pub fn code_status(_app: &AppHandle, project_root: &str) -> Result<CodeIndexStatus, String> {
    let conn = open_conn(project_root)?;
    let chunks: i64 = conn
        .query_row("SELECT COUNT(*) FROM code_chunks", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    let files: i64 = conn
        .query_row(
            "SELECT COUNT(DISTINCT file_path) FROM code_chunks",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let indexed: i64 = conn
        .query_row("SELECT COUNT(*) FROM code_file_state", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    let last: Option<String> = conn
        .query_row(
            "SELECT v FROM meta WHERE k = ?1",
            [META_CODE_LAST_INDEX],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    Ok(CodeIndexStatus {
        chunks,
        files,
        indexed_files: indexed,
        last_index_at: last,
    })
}

struct RawChunk {
    start_line: usize,
    end_line: usize,
    content: String,
}

/// 按行分块（60 行一块，10 行重叠）。
fn chunk_content(content: &str) -> Vec<RawChunk> {
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }
    if lines.len() <= CHUNK_LINES {
        return vec![RawChunk {
            start_line: 1,
            end_line: lines.len(),
            content: lines.join("\n"),
        }];
    }
    let mut out = Vec::new();
    let step = CHUNK_LINES - CHUNK_OVERLAP;
    let mut start = 0usize;
    while start < lines.len() {
        let end = (start + CHUNK_LINES).min(lines.len());
        out.push(RawChunk {
            start_line: start + 1,
            end_line: end,
            content: lines[start..end].join("\n"),
        });
        if end >= lines.len() {
            break;
        }
        start += step;
    }
    out
}

/// 收集可索引文件（递归，跳过黑名单目录与超大文件）。
fn collect_files(root: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![PathBuf::from(root)];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                if SKIP_DIRS.contains(&name.as_str()) {
                    continue;
                }
                stack.push(path);
                continue;
            }
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            if !ALLOWED_EXT.contains(&ext.as_str()) {
                continue;
            }
            if let Ok(meta) = std::fs::metadata(&path) {
                if meta.len() > MAX_FILE_BYTES {
                    continue;
                }
            }
            out.push(path);
        }
    }
    out.sort();
    out
}

/// 批量嵌入（失败项为 None）。
async fn embed_batch(app: &AppHandle, texts: &[String]) -> Vec<Option<Vec<f32>>> {
    let cfg = crate::commands::config::load_app_config(app.clone()).unwrap_or_default();
    let mut ecfg = cfg.memory_embedding;
    if ecfg.provider == "openai_compat" && ecfg.api_key.is_empty() {
        if let Ok(Some(key)) = crate::core::secure_store::get(app, "embedding_api_key") {
            ecfg.api_key = key;
        }
    }
    let Ok(embedder) = crate::plugins::embed::resolve(&ecfg) else {
        return texts.iter().map(|_| None).collect();
    };
    let mut out: Vec<Option<Vec<f32>>> = Vec::new();
    for chunk in texts.chunks(8) {
        match embedder.embed(chunk).await {
            Ok(vecs) => {
                for v in vecs {
                    out.push(Some(v));
                }
            }
            Err(_) => {
                for _ in chunk {
                    out.push(None);
                }
            }
        }
    }
    out
}

/// 确保代码 vec0 表存在（按配置维度）。
fn ensure_code_vec_table(app: &AppHandle, conn: &Connection) -> Result<(), String> {
    if !crate::services::memory::vec0::vec0_available() {
        return Ok(());
    }
    let cfg = crate::commands::config::load_app_config(app.clone()).unwrap_or_default();
    let dim = cfg.memory_embedding.dimensions.clamp(1, 8192);
    let sql = format!(
        "CREATE VIRTUAL TABLE IF NOT EXISTS vec_code USING vec0(embedding float[{dim}] distance_metric=cosine, chunk_id text +)"
    );
    conn.execute_batch(&sql)
        .map_err(|e| format!("创建 vec_code 表失败: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_content_small() {
        let c = "a\nb\nc";
        let chunks = chunk_content(c);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].start_line, 1);
        assert_eq!(chunks[0].end_line, 3);
    }

    #[test]
    fn test_chunk_content_large() {
        let c = (1..=200)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let chunks = chunk_content(&c);
        assert!(chunks.len() >= 3);
        assert_eq!(chunks[0].end_line, 60);
        assert_eq!(chunks.last().unwrap().end_line, 200);
    }
}
