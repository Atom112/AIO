//! Legacy cross-session project knowledge (`.aio/knowledge.json`).
//!
//! Superseded by project-level semantic memory (RAG): new memories are written to `.aio/memory/memory.sqlite`.
//! This module keeps only read capability: on first open of project memory, the old knowledge.json is migrated into facts (idempotent).

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

/// A single project-level knowledge entry.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeEntry {
    /// Short identifier of the knowledge (used for dedup and retrieval).
    pub key: String,
    /// Knowledge content.
    pub content: String,
    /// Category: "decision" | "pattern" | "convention" | "note"
    pub category: String,
    /// Creation time (ISO 8601).
    pub created_at: String,
    /// Last update time (ISO 8601).
    pub updated_at: String,
}

/// On-disk format of `.aio/knowledge.json`.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct KnowledgeFile {
    pub entries: Vec<KnowledgeEntry>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

// Time helpers are centralized in utils::time (now_iso / secs_to_iso / civil-date conversion).
// This module is read-only legacy code and must not become a dependency point for new code.

// ---------------------------------------------------------------------------
// Core functions
// ---------------------------------------------------------------------------

/// Load the knowledge file from `{project_root}/.aio/knowledge.json`.
/// Returns an empty `KnowledgeFile` if the file does not exist.
pub fn load_knowledge(project_root: &str) -> KnowledgeFile {
    let path = Path::new(project_root).join(".aio").join("knowledge.json");
    match fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => KnowledgeFile::default(),
    }
}
