//! Backend-to-frontend event-name constants - the single source of truth repo-wide.
//!
//! Convention: every new event must be registered here (kebab-case); the frontend `core/api/events.ts` uses matching constants.

/// Engine start progress (generic; default for all local engines)
pub const ENGINE_PROGRESS: &str = "engine-progress";
/// Legacy llama.cpp event name (kept for backward compatibility; new code uses ENGINE_PROGRESS)
pub const LLAMA_PROGRESS: &str = "llama-progress";
/// Model download progress
pub const MODEL_DOWNLOAD_PROGRESS: &str = "model-download-progress";
/// Embedding model reindex progress
pub const REINDEX_PROGRESS: &str = "reindex-progress";
/// Codebase indexing progress
pub const CODE_INDEX_PROGRESS: &str = "code-index-progress";
/// App update download progress
pub const APP_UPDATE_PROGRESS: &str = "app-update-progress";

/// One LLM round starts (the frontend pushes an empty assistant placeholder)
pub const LLM_ROUND_START: &str = "llm-round-start";
/// Streaming delta (text)
pub const LLM_CHUNK: &str = "llm-chunk";
/// Streaming delta (reasoning/thinking content)
pub const LLM_REASONING: &str = "llm-reasoning";
/// Tool call starts (frontend shows a "running" bubble)
pub const LLM_TOOL_CALL: &str = "llm-tool-call";
/// Tool call result
pub const LLM_TOOL_RESULT: &str = "llm-tool-result";
/// Context compression completed
pub const LLM_COMPRESSION: &str = "llm-compression";
/// Context compression failed
pub const LLM_COMPRESSION_FAILED: &str = "llm-compression-failed";
/// Handoff document generated for an unfinished task (payload: assistant_id/topic_id/path/reason)
pub const LLM_HANDOFF: &str = "llm-handoff";

/// Subagent started
pub const SUBAGENT_START: &str = "subagent-start";
/// Subagent step progress
pub const SUBAGENT_STEP: &str = "subagent-step";
/// Subagent finished
pub const SUBAGENT_DONE: &str = "subagent-done";
/// Subagent error
pub const SUBAGENT_ERROR: &str = "subagent-error";

/// Workflow started / step started / step completed / finished
pub const WORKFLOW_START: &str = "workflow-start";
pub const WORKFLOW_STEP_START: &str = "workflow-step-start";
pub const WORKFLOW_STEP_COMPLETE: &str = "workflow-step-complete";
pub const WORKFLOW_COMPLETE: &str = "workflow-complete";

/// MCP server status change
pub const MCP_SERVER_STATUS: &str = "mcp-server-status";
/// MCP server stderr output
pub const MCP_SERVER_STDERR: &str = "mcp-server-stderr";
/// MCP server tool list changed (notifications/tools/list_changed)
pub const MCP_TOOLS_CHANGED: &str = "mcp-tools-changed";

/// Tool approval request (waiting for user confirmation)
pub const TOOL_APPROVAL_REQUESTED: &str = "tool-approval-requested";

/// Streaming delta for BTW side-question answers (frontend overlay)
pub const BTW_CHUNK: &str = "btw-chunk";

#[cfg(test)]
mod tests {
    use super::*;

    /// Event names must stay kebab-case (frontend listeners rely on this convention).
    #[test]
    fn event_names_are_kebab_case() {
        for name in [
            ENGINE_PROGRESS,
            LLAMA_PROGRESS,
            MODEL_DOWNLOAD_PROGRESS,
            REINDEX_PROGRESS,
            CODE_INDEX_PROGRESS,
            APP_UPDATE_PROGRESS,
            LLM_ROUND_START,
            LLM_CHUNK,
            LLM_REASONING,
            LLM_TOOL_CALL,
            LLM_TOOL_RESULT,
            LLM_COMPRESSION,
            LLM_COMPRESSION_FAILED,
            LLM_HANDOFF,
            SUBAGENT_START,
            SUBAGENT_STEP,
            SUBAGENT_DONE,
            SUBAGENT_ERROR,
            WORKFLOW_START,
            WORKFLOW_STEP_START,
            WORKFLOW_STEP_COMPLETE,
            WORKFLOW_COMPLETE,
            MCP_SERVER_STATUS,
            MCP_SERVER_STDERR,
            MCP_TOOLS_CHANGED,
            TOOL_APPROVAL_REQUESTED,
            BTW_CHUNK,
        ] {
            assert!(
                name.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "事件名必须为 kebab-case: {name}"
            );
        }
    }
}
