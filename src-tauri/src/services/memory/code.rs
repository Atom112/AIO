//! 代码级 RAG（P4）：把项目关键文件分块向量化，支撑 search_code 工具与代码语义检索。
//!
//! 与事实库（facts）并存于同一 memory.sqlite：code_chunks + fts_code + vec_code。
//! 索引按需触发（memory_code_index / 记忆面板按钮），增量基于文件 mtime/size。

use crate::core::events as ev;
use crate::core::models::MemoryEmbeddingConfig;
use crate::ierr;
use crate::services::memory::CollectRowsWarn;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager};

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
    /// 所在符号名（符号感知分块；行窗口分块为 None）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
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

/// 解析项目级嵌入配置（与记忆命令一致；首次使用从旧全局配置迁移，幂等）。
fn project_embedding_cfg(app: &AppHandle, project_root: &str) -> MemoryEmbeddingConfig {
    match app
        .state::<crate::services::memory::MemoryStoreManager>()
        .open(project_root)
    {
        Ok(store) => crate::services::memory::resolve_project_embedding(app, &store),
        Err(_) => {
            crate::core::config::load_app_config(app)
                .unwrap_or_default()
                .memory_embedding
        }
    }
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
    ensure_code_vec_table(app, project_root, &conn)?;
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
        let meta = std::fs::metadata(f).map_err(|e| {
            ierr!(
                "读取文件元数据失败: {}",
                "Failed to read file metadata: {}",
                e
            )
        })?;
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
        // 上下文补全：嵌入文本附带相对路径（Contextual Retrieval），提升代码块语义定位；
        // 展示与 FTS 仍用原始代码内容
        let rel_path = std::path::Path::new(f)
            .strip_prefix(project_root)
            .unwrap_or_else(|_| std::path::Path::new(f))
            .display()
            .to_string();
        // P3-4 符号感知分块：LSP 可用时按函数/类边界切块（chunk 附带符号名），否则回退行窗口
        let chunks = match crate::utils::lsp_agent_tools::fetch_document_symbol_ranges(
            app,
            project_root,
            &rel_path,
        )
        .await
        {
            Ok(symbols) if !symbols.is_empty() => chunk_content_symbol_aware(&content, &symbols),
            _ => chunk_content(&content),
        };
        let texts: Vec<String> = chunks
            .iter()
            .map(|c| format!("文件：{rel_path}\n{}", c.content))
            .collect();
        let (emb_model, vecs) = embed_batch(app, project_root, &texts).await;
        for (i, chunk) in chunks.iter().enumerate() {
            let id = uuid::Uuid::new_v4().to_string();
            let vec = vecs.get(i).cloned().flatten();
            conn.execute(
                "INSERT INTO code_chunks (id, file_path, start_line, end_line, content, symbol, embedding, embedding_model, embedding_dim, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    id,
                    f.to_string_lossy(),
                    chunk.start_line as i64,
                    chunk.end_line as i64,
                    chunk.content,
                    chunk.symbol.as_deref(),
                    vec.as_ref().map(|v| crate::services::memory::vector::f32_to_bytes(v)),
                    emb_model.as_deref(),
                    vec.as_ref().map(|v| v.len() as i64),
                    crate::utils::time::now_iso(),
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
        params![META_CODE_LAST_INDEX, crate::utils::time::now_iso()],
    )
    .map_err(|e| e.to_string())?;
    result.elapsed_ms = started.elapsed().as_millis() as u64;
    let _ = app.emit(
        ev::CODE_INDEX_PROGRESS,
        serde_json::json!({ "status": "done", "chunks": result.chunks, "files": result.files, "elapsedMs": result.elapsed_ms }),
    );
    Ok(result)
}

/// 代码语义检索：向量 + FTS5 RRF 融合。
///
/// 本函数为 async：查询向量的嵌入计算需要 await（严禁在 async 上下文里用
/// `Handle::block_on`，tokio 会直接 panic）；DB 检索段经 `spawn_blocking`
/// 移入阻塞线程池，避免占用 async worker。
pub async fn search_code(
    app: &AppHandle,
    project_root: &str,
    query: &str,
    k: usize,
    file_filter: Option<&str>,
    symbol_filter: Option<&str>,
) -> Result<Vec<CodeChunkHit>, String> {
    let k = k.clamp(1, 30);
    let ecfg = project_embedding_cfg(app, project_root);
    // 查询向量：嵌入器不可用时为 None，退化为纯 FTS 关键词检索
    let qv: Option<Vec<f32>> = if let Ok(embedder) = crate::plugins::embed::resolve(app, &ecfg) {
        let q = query.to_string();
        embedder
            .embed(&[q])
            .await
            .ok()
            .and_then(|v| v.into_iter().next())
    } else {
        None
    };
    let root = project_root.to_string();
    let q = query.to_string();
    let ff = file_filter.map(str::to_string);
    let sf = symbol_filter.map(str::to_string);
    tokio::task::spawn_blocking(move || {
        search_code_db(&root, &q, k, qv.as_deref(), ff.as_deref(), sf.as_deref())
    })
    .await
    .map_err(|e| {
        ierr!(
            "search_code 检索线程失败: {}",
            "search_code retrieval thread failed: {}",
            e
        )
    })?
}

/// `search_code` 的同步检索主体（向量/FTS/RRF 融合），运行在阻塞线程上。
fn search_code_db(
    project_root: &str,
    query: &str,
    k: usize,
    qv: Option<&[f32]>,
    file_filter: Option<&str>,
    symbol_filter: Option<&str>,
) -> Result<Vec<CodeChunkHit>, String> {
    let conn = open_conn(project_root)?;
    let mut vec_ranks: HashMap<String, usize> = HashMap::new();
    if let Some(qv) = qv {
        // vec0 优先；不可用/维度不匹配/空结果时回退暴力余弦（与事实库 vector_search 同策略）
        let mut used_vec0 = false;
        if crate::services::memory::vec0::vec0_available() {
            if let Ok(rows) = crate::services::memory::vec0::vec0_search_code(&conn, qv, k * 2) {
                if !rows.is_empty() {
                    used_vec0 = true;
                    for (i, (id, _)) in rows.iter().enumerate() {
                        vec_ranks.insert(id.clone(), i);
                    }
                }
            }
        }
        if !used_vec0 {
            if let Ok(mut stmt) =
                conn.prepare("SELECT id, embedding FROM code_chunks WHERE embedding IS NOT NULL")
            {
                if let Ok(rows) = stmt.query_map([], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))
                }) {
                    let decoded: Vec<(String, Vec<f32>)> = rows
                        .collect_rows("code")
                        .into_iter()
                        .map(|(id, b)| (id, crate::services::memory::vector::bytes_to_f32(&b)))
                        .collect();
                    let top =
                        crate::services::memory::vector::brute_force_topk(&decoded, qv, k * 2);
                    for (i, (id, _)) in top.iter().enumerate() {
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
                for (i, id) in rows.collect_rows("code").into_iter().enumerate() {
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
        let row: Option<(String, i64, i64, String, Option<String>)> = conn
            .query_row(
                "SELECT file_path, start_line, end_line, content, symbol FROM code_chunks WHERE id = ?1",
                [&id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        if let Some((file_path, start_line, end_line, content, symbol)) = row {
            // P3-4 过滤：路径子串 + 符号名匹配（模型可按需定位到文件/符号）
            if let Some(fp) = file_filter {
                if !fp.is_empty() && !file_path.contains(fp) {
                    continue;
                }
            }
            if let Some(sym) = symbol_filter {
                if !sym.is_empty()
                    && !symbol
                        .as_deref()
                        .map(|s| s == sym || s.contains(sym) || sym.contains(s))
                        .unwrap_or(false)
                {
                    continue;
                }
            }
            out.push(CodeChunkHit {
                chunk_id: id,
                file_path,
                start_line,
                end_line,
                symbol,
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
    /// 所在符号名（符号感知分块时填充；行窗口分块为 None）。
    symbol: Option<String>,
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
            symbol: None,
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
            symbol: None,
        });
        if end >= lines.len() {
            break;
        }
        start += step;
    }
    out
}

/// 符号感知分块（P3-4）：优先按 LSP 符号范围（函数/类体）切分，符号间隙回退行窗口。
/// symbols 为 (符号名, start_line, end_line)，LSP 行号从 0 起，按文件 1 基行号换算。
fn chunk_content_symbol_aware(content: &str, symbols: &[(String, i64, i64)]) -> Vec<RawChunk> {
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }
    let mut covered = vec![false; lines.len()];
    let mut out: Vec<RawChunk> = Vec::new();
    // 1) 符号边界切块（嵌套符号都保留：细粒度函数块 + 粗粒度类块；覆盖标记只阻止间隙补块）
    for (name, s0, e0) in symbols {
        let s = ((*s0).max(0) as usize).min(lines.len());
        let e = (((*e0).max(0) as usize) + 1).min(lines.len());
        if e <= s {
            continue;
        }
        for line in &mut covered[s..e] {
            *line = true;
        }
        out.push(RawChunk {
            start_line: s + 1,
            end_line: e,
            content: lines[s..e].join("\n"),
            symbol: Some(name.clone()),
        });
    }
    // 2) 未被任何符号覆盖的行区间按行窗口补充分块（符号间隙/文件头尾）
    let mut i = 0usize;
    while i < lines.len() {
        if covered[i] {
            i += 1;
            continue;
        }
        let run_start = i;
        while i < lines.len() && !covered[i] {
            i += 1;
        }
        let run_end = i; // 独占
        let run = &lines[run_start..run_end];
        let step = CHUNK_LINES - CHUNK_OVERLAP;
        let mut start = 0usize;
        while start < run.len() {
            let end = (start + CHUNK_LINES).min(run.len());
            out.push(RawChunk {
                start_line: run_start + start + 1,
                end_line: run_start + end,
                content: run[start..end].join("\n"),
                symbol: None,
            });
            if end >= run.len() {
                break;
            }
            start += step;
        }
    }
    out.sort_by_key(|c| c.start_line);
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

/// 批量嵌入（失败项为 None）；返回 (嵌入模型名, 每 chunk 向量)。
async fn embed_batch(
    app: &AppHandle,
    project_root: &str,
    texts: &[String],
) -> (Option<String>, Vec<Option<Vec<f32>>>) {
    let ecfg = project_embedding_cfg(app, project_root);
    let Ok(embedder) = crate::plugins::embed::resolve(app, &ecfg) else {
        return (None, texts.iter().map(|_| None).collect());
    };
    let model = embedder.model_key();
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
    (Some(model), out)
}

/// 确保代码 vec0 表存在（按配置维度）。
fn ensure_code_vec_table(
    app: &AppHandle,
    project_root: &str,
    conn: &Connection,
) -> Result<(), String> {
    if !crate::services::memory::vec0::vec0_available() {
        return Ok(());
    }
    let dim = project_embedding_cfg(app, project_root)
        .dimensions
        .clamp(1, 8192);
    let sql = format!(
        "CREATE VIRTUAL TABLE IF NOT EXISTS vec_code USING vec0(embedding float[{dim}] distance_metric=cosine, chunk_id text +)"
    );
    conn.execute_batch(&sql).map_err(|e| {
        ierr!(
            "创建 vec_code 表失败: {}",
            "Failed to create vec_code table: {}",
            e
        )
    })
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

    #[test]
    fn test_chunk_content_symbol_aware() {
        // 200 行：fn_a 覆盖 1..80，fn_b 覆盖 100..140；间隙与尾部回退行窗口
        let c = (1..=200)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let symbols = vec![
            ("fn_a".to_string(), 0i64, 79i64),
            ("fn_b".to_string(), 99i64, 139i64),
        ];
        let chunks = chunk_content_symbol_aware(&c, &symbols);
        let a = chunks
            .iter()
            .find(|c| c.symbol.as_deref() == Some("fn_a"))
            .expect("应有 fn_a 符号块");
        assert_eq!(a.start_line, 1);
        assert_eq!(a.end_line, 80);
        assert!(a.content.contains("line 1") && a.content.contains("line 80"));
        let b = chunks
            .iter()
            .find(|c| c.symbol.as_deref() == Some("fn_b"))
            .expect("应有 fn_b 符号块");
        assert_eq!(b.start_line, 100);
        assert_eq!(b.end_line, 140);
        // 符号间隙（80..99）与文件尾（140..200）以行窗口补充分块
        let windows = chunks.iter().filter(|c| c.symbol.is_none()).count();
        assert!(windows >= 2, "间隙与尾部分块应存在: {}", windows);
        // 总块数有序且不重叠跳序
        assert!(chunks
            .windows(2)
            .all(|w| w[0].start_line <= w[1].start_line));
    }
}
