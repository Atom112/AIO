//! Code-level RAG: chunks and vectorizes key project files to back the search_code tool and semantic code retrieval.
//!
//! Coexists with the facts store in the same memory.sqlite: code_chunks + fts_code + vec_code.
//! Indexing is on-demand (memory_code_index / memory panel button) and incremental via file mtime/size.

use crate::core::models::MemoryEmbeddingConfig;
use crate::ierr;
use crate::services::memory::CollectRowsWarn;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

/// Index result statistics.
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

/// A code chunk hit from retrieval.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct CodeChunkHit {
    pub chunk_id: String,
    pub file_path: String,
    pub start_line: i64,
    pub end_line: i64,
    /// Containing symbol name (symbol-aware chunking; None for line-window chunks)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    pub content: String,
    pub score: f32,
}

/// Index status.
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

/// Resolve the project-level embedding config (same as memory commands; first use migrates from the legacy global config, idempotent).
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

/// Index project code (incremental: skipped when mtime/size are unchanged and force is false).
pub async fn index_code(
    app: &AppHandle,
    project_root: &str,
    force: bool,
    mut on_progress: impl FnMut(usize, usize) + Send,
) -> Result<CodeIndexResult, String> {
    let started = std::time::Instant::now();
    let conn = open_conn(project_root)?;
    ensure_code_vec_table(app, project_root, &conn)?;
    // MEM-08: the directory walk (full recursive stat sweep) runs on the blocking
    // pool — it must not tie up an async worker thread for the whole project.
    let root_walk = project_root.to_string();
    let files = tokio::task::spawn_blocking(move || collect_files(&root_walk))
        .await
        .map_err(|e| e.to_string())?;
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
        // MEM-08: metadata (and the content read below) are disk I/O — keep them off
        // the async worker threads; the SQLite statements are tiny WAL writes and stay
        // inline (they are far cheaper than one spawn_blocking handoff each).
        let f_meta = f.clone();
        let (mtime, size) = tokio::task::spawn_blocking(move || {
            let meta = std::fs::metadata(&f_meta).map_err(|e| e.to_string())?;
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos() as i64)
                .unwrap_or(0);
            Ok::<(i64, u64), String>((mtime, meta.len()))
        })
        .await
        .map_err(|e| {
            ierr!(
                "索引元数据读取线程失败: {}",
                "Index metadata thread failed: {}",
                e
            )
        })?
        .map_err(|e| {
            ierr!(
                "读取文件元数据失败: {}",
                "Failed to read file metadata: {}",
                e
            )
        })?;
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
        let f_read = f.clone();
        let content = match tokio::task::spawn_blocking(move || std::fs::read_to_string(&f_read))
            .await
            .map_err(|e| e.to_string())
        {
            Ok(Ok(c)) => c,
            _ => {
                result.failed += 1;
                done += 1;
                continue;
            }
        };
        // MEM-04: 先按旧 chunk_id 清理 vec_code——重建会生成全新 uuid，旧 id 的
        // 幽灵向量若不清理将永远留在 vec_code 里，持续消耗向量召回的 top-k 候选
        // 名额（检索腿退化为纯 FTS）。必须先于 code_chunks 的删除执行（子查询
        // 依赖旧 chunks 行）；表按需建，删除前确认存在。
        delete_chunks_for_file(&conn, f.to_string_lossy().as_ref())?;
        // Contextual retrieval: the embedded text carries the relative path to improve semantic
        // chunk localization; display and FTS still use the raw code content
        let rel_path = std::path::Path::new(f)
            .strip_prefix(project_root)
            .unwrap_or_else(|_| std::path::Path::new(f))
            .display()
            .to_string();
        // Symbol-aware chunking: when LSP is available, split at function/class boundaries (chunks carry the symbol name); otherwise fall back to line windows
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
    // MEM-08: prune chunks/state of files that no longer exist on disk — previously
    // they stayed forever, permanently polluting retrieval with ghost chunks.
    let removed = prune_deleted_files(&conn, &files)?;
    if removed > 0 {
        tracing::debug!("[code-index] 清理已删除文件的残留索引: {removed} 个文件");
    }
    conn.execute(
        "INSERT OR REPLACE INTO meta (k, v) VALUES (?1, ?2)",
        params![META_CODE_LAST_INDEX, crate::utils::time::now_iso()],
    )
    .map_err(|e| e.to_string())?;
    result.elapsed_ms = started.elapsed().as_millis() as u64;
    // Terminal lifecycle event moved to the command layer (memory_code_index):
    // it owns the project identity and also reports the error path, which this
    // core fn would miss via its `?` returns.
    Ok(result)
}

/// Semantic code retrieval: vector + FTS5 RRF fusion.
///
/// This function is async: embedding the query vector requires await (never use
/// `Handle::block_on` in an async context; tokio panics outright); the DB retrieval
/// section moves to the blocking pool via `spawn_blocking` to avoid tying up async workers.
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
    // Query vector: None when the embedder is unavailable, degrading to pure FTS keyword retrieval
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

/// Synchronous retrieval body of `search_code` (vector/FTS/RRF fusion), runs on a blocking thread.
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
        // Prefer vec0; fall back to brute-force cosine when unavailable/dimension-mismatched/empty (same strategy as the facts-store vector_search)
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
            // Filtering: path substring + symbol name match (the model can narrow to a file/symbol on demand)
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

/// Code index status.
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
    /// Containing symbol name (set for symbol-aware chunks; None for line-window chunks).
    symbol: Option<String>,
}

/// Line-window chunking (60 lines per chunk, 10-line overlap).
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

/// Symbol-aware chunking: split at LSP symbol ranges (function/class bodies) first; symbol gaps fall back to line windows.
/// symbols is (name, start_line, end_line); LSP lines are 0-based and converted to the file's 1-based line numbers.
fn chunk_content_symbol_aware(content: &str, symbols: &[(String, i64, i64)]) -> Vec<RawChunk> {
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }
    let mut covered = vec![false; lines.len()];
    let mut out: Vec<RawChunk> = Vec::new();
    // 1) Chunk at symbol boundaries (nested symbols are all kept: fine-grained function blocks + coarse class blocks; coverage flags only block gap-filling)
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
    // 2) Line ranges covered by no symbol are filled in with line-window chunks (symbol gaps / file head and tail)
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
        let run_end = i; // exclusive
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

/// Delete every stored row belonging to one file (vec_code ghosts first — the
/// subquery needs the old code_chunks rows — then FTS, chunks and state).
fn delete_chunks_for_file(conn: &Connection, file_path: &str) -> Result<(), String> {
    let vec_code_exists: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name = 'vec_code'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map(|n| n > 0)
        .unwrap_or(false);
    if vec_code_exists {
        conn.execute(
            "DELETE FROM vec_code WHERE chunk_id IN (SELECT id FROM code_chunks WHERE file_path = ?1)",
            [file_path],
        )
        .map_err(|e| e.to_string())?;
    }
    conn.execute("DELETE FROM fts_code WHERE file_path = ?1", [file_path])
        .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM code_chunks WHERE file_path = ?1", [file_path])
        .map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM code_file_state WHERE file_path = ?1",
        [file_path],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// MEM-08: remove stored chunks of files that vanished from disk. Returns how many
/// files were pruned.
fn prune_deleted_files(conn: &Connection, current: &[PathBuf]) -> Result<usize, String> {
    let known: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT file_path FROM code_file_state")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        rows.collect_rows("code_file_state")
    };
    let current_set: std::collections::HashSet<String> = current
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    let mut removed = 0usize;
    for path in known {
        if !current_set.contains(&path) {
            delete_chunks_for_file(conn, &path)?;
            removed += 1;
        }
    }
    Ok(removed)
}

/// Root .gitignore lines (MEM-08): non-empty, non-comment, non-negated. Full gitignore
/// semantics (nested files, `**`, negation) are out of scope; the common shapes are
/// honored by [`gitignore_matches`].
fn load_gitignore(root: &str) -> Vec<String> {
    std::fs::read_to_string(std::path::Path::new(root).join(".gitignore"))
        .map(|content| {
            content
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with('!'))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// MEM-08: minimal .gitignore matching for the root file's lines:
/// - name-level patterns (no `/`, e.g. `target`, `*.log`) match any path component;
/// - trailing-slash paths (e.g. `docs/generated/`) prefix-match whole directories;
/// - plain paths (e.g. `docs/README.md`) glob-match the relative path.
fn gitignore_matches(patterns: &[String], rel: &str) -> bool {
    if patterns.is_empty() {
        return false;
    }
    let rel = rel.replace('\\', "/");
    let components: Vec<&str> = rel.split('/').collect();
    for pat in patterns {
        let is_dir_pat = pat.ends_with('/');
        let body = pat.trim_end_matches('/');
        if !body.contains('/') {
            let Ok(gp) = glob::Pattern::new(body) else {
                // Non-glob characters: fall back to exact-name comparison
                if components.contains(&body) {
                    return true;
                }
                continue;
            };
            if components.iter().any(|c| gp.matches(c)) {
                return true;
            }
        } else if is_dir_pat {
            if rel.starts_with(&format!("{body}/")) {
                return true;
            }
        } else {
            let Ok(gp) = glob::Pattern::new(body) else {
                if rel == *body {
                    return true;
                }
                continue;
            };
            if gp.matches(&rel) {
                return true;
            }
        }
    }
    false
}

/// Collect indexable files (recursive; skips blacklisted directories, .gitignore
/// entries, symlinks and oversized files).
fn collect_files(root: &str) -> Vec<PathBuf> {
    let gitignore = load_gitignore(root);
    let root_path = PathBuf::from(root);
    let mut out = Vec::new();
    let mut stack = vec![PathBuf::from(root)];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            // MEM-08: file_type() does NOT follow symlinks (Path::is_dir does). A
            // symlinked directory cycle would otherwise loop the walk forever, and
            // symlinked files double-index aliased content — both are skipped.
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_symlink() {
                continue;
            }
            let path = entry.path();
            let rel = path
                .strip_prefix(&root_path)
                .unwrap_or(&path)
                .to_string_lossy()
                .into_owned();
            if file_type.is_dir() {
                let name = entry.file_name().to_string_lossy().to_string();
                if SKIP_DIRS.contains(&name.as_str()) {
                    continue;
                }
                if gitignore_matches(&gitignore, &rel) {
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
            if gitignore_matches(&gitignore, &rel) {
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

/// Batch embed (failed items are None); returns (embedding model name, per-chunk vector).
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

/// Ensure the code vec0 table exists (at the configured dimension).
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
        // 200 lines: fn_a covers 1..80, fn_b covers 100..140; gaps and tail fall back to line windows
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
        // Symbol gap (80..99) and file tail (140..200) are chunked as line windows
        let windows = chunks.iter().filter(|c| c.symbol.is_none()).count();
        assert!(windows >= 2, "间隙与尾部分块应存在: {}", windows);
        // Overall chunks are ordered (non-decreasing start lines)
        assert!(chunks
            .windows(2)
            .all(|w| w[0].start_line <= w[1].start_line));
    }

    /// MEM-08: root .gitignore lines filter the walk — name-level, dir-path, and
    /// whole-path patterns; negated lines and comments are ignored.
    #[test]
    fn gitignore_rules_filter_walked_files() {
        let dir = std::env::temp_dir().join(format!("aio-codeignore-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("target").join("nested")).unwrap();
        std::fs::create_dir_all(dir.join("docs").join("generated")).unwrap();
        std::fs::write(dir.join("target").join("lib.rs"), "x").unwrap();
        std::fs::write(dir.join("target").join("nested").join("lib.rs"), "x").unwrap();
        std::fs::write(dir.join("docs").join("generated").join("a.md"), "x").unwrap();
        std::fs::write(dir.join("debug.log"), "x").unwrap();
        std::fs::write(dir.join("docs").join("README.md"), "x").unwrap();
        std::fs::write(
            dir.join(".gitignore"),
            "target\n*.log\ndocs/generated/\n# comment\n!important\n",
        )
        .unwrap();

        let files = collect_files(&dir.to_string_lossy());
        let rels: Vec<String> = files
            .iter()
            .map(|f| {
                f.strip_prefix(&dir)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        assert!(
            !rels.iter().any(|r| r.starts_with("target")),
            "target 目录整体应被忽略: {rels:?}"
        );
        assert!(
            !rels.iter().any(|r| r.starts_with("docs/generated")),
            "带尾斜杠的目录前缀应被忽略: {rels:?}"
        );
        assert!(
            !rels.iter().any(|r| r.ends_with("debug.log")),
            "*.log 应被忽略"
        );
        assert!(
            rels.iter().any(|r| r == "docs/README.md"),
            "未被忽略的文件应保留: {rels:?}"
        );
        // 纯函数：name-level 模式匹配任意层级组件
        let pats = vec!["vendor".to_string(), "*.tmp".to_string()];
        assert!(gitignore_matches(&pats, "src/vendor"));
        assert!(gitignore_matches(&pats, "src/x/vendor/lib.rs"));
        assert!(gitignore_matches(&pats, "a/b.tmp"));
        assert!(!gitignore_matches(&pats, "src/main.rs"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// MEM-08: chunks of files deleted from disk are pruned on the next index run.
    #[test]
    fn prune_removes_ghost_file_chunks() {
        let dir = std::env::temp_dir().join(format!("aio-codeprune-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("live.rs"), "fn main() {}").unwrap();
        let conn = open_conn(&dir.to_string_lossy()).expect("连接成功");
        // 残留：磁盘上已删除的 ghost.rs
        for path in ["ghost.rs", "live.rs"] {
            conn.execute(
                "INSERT INTO code_chunks (id, file_path, start_line, end_line, content, symbol, embedding, embedding_model, embedding_dim, created_at) \
                 VALUES (?1, ?2, 1, 1, 'x', NULL, NULL, NULL, NULL, ?3)",
                params![
                    format!("chunk-{path}"),
                    dir.join(path).to_string_lossy(),
                    crate::utils::time::now_iso()
                ],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO fts_code (rowid, chunk_id, content, file_path) VALUES (\
                 (SELECT rowid FROM code_chunks WHERE id = ?1), ?1, 'x', ?2)",
                params![format!("chunk-{path}"), dir.join(path).to_string_lossy()],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO code_file_state (file_path, mtime_nanos, size_bytes) VALUES (?1, 1, 1)",
                [dir.join(path).to_string_lossy().as_ref()],
            )
            .unwrap();
        }
        let current = collect_files(&dir.to_string_lossy());
        let removed = prune_deleted_files(&conn, &current).expect("清理执行成功");
        assert_eq!(removed, 1, "只应清理磁盘上已消失的 ghost.rs");
        let ghost_left: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM code_chunks WHERE file_path LIKE ?1",
                ["%ghost.rs"],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(ghost_left, 0, "ghost.rs 的 chunks 应被清空");
        let live_left: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM code_chunks WHERE file_path LIKE ?1",
                ["%live.rs"],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(live_left, 1, "仍存在的文件不应被误清");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
