//! 后端 → 前端事件名常量 —— 全仓唯一来源。
//!
//! 历史：23 个事件名以裸字符串散落在 50 处 emit 调用点，改名/查引用只能靠全局搜索。
//! 约定：新增事件必须在此登记（kebab-case），前端 `core/api/events.ts` 使用同名常量。

/// 引擎启动进度（通用；所有本地引擎默认使用）
pub const ENGINE_PROGRESS: &str = "engine-progress";
/// llama.cpp 历史事件名（向后兼容保留，新代码用 ENGINE_PROGRESS）
pub const LLAMA_PROGRESS: &str = "llama-progress";
/// 模型下载进度
pub const MODEL_DOWNLOAD_PROGRESS: &str = "model-download-progress";
/// 嵌入模型索引重建进度
pub const REINDEX_PROGRESS: &str = "reindex-progress";
/// 代码库索引进度
pub const CODE_INDEX_PROGRESS: &str = "code-index-progress";
/// 应用更新下载进度
pub const APP_UPDATE_PROGRESS: &str = "app-update-progress";

/// 一轮 LLM 调用开始（前端 push 空 assistant 占位）
pub const LLM_ROUND_START: &str = "llm-round-start";
/// 流式增量（文本）
pub const LLM_CHUNK: &str = "llm-chunk";
/// 流式增量（推理/思考内容）
pub const LLM_REASONING: &str = "llm-reasoning";
/// 工具调用开始（展示"调用中"气泡）
pub const LLM_TOOL_CALL: &str = "llm-tool-call";
/// 工具调用结果
pub const LLM_TOOL_RESULT: &str = "llm-tool-result";
/// 上下文压缩完成
pub const LLM_COMPRESSION: &str = "llm-compression";
/// 上下文压缩失败
pub const LLM_COMPRESSION_FAILED: &str = "llm-compression-failed";
/// 未完成任务已生成交接文档（payload: assistant_id/topic_id/path/reason）
pub const LLM_HANDOFF: &str = "llm-handoff";

/// 子智能体启动
pub const SUBAGENT_START: &str = "subagent-start";
/// 子智能体单步进展
pub const SUBAGENT_STEP: &str = "subagent-step";
/// 子智能体完成
pub const SUBAGENT_DONE: &str = "subagent-done";
/// 子智能体错误
pub const SUBAGENT_ERROR: &str = "subagent-error";

/// 工作流开始 / 步骤开始 / 步骤完成 / 完成
pub const WORKFLOW_START: &str = "workflow-start";
pub const WORKFLOW_STEP_START: &str = "workflow-step-start";
pub const WORKFLOW_STEP_COMPLETE: &str = "workflow-step-complete";
pub const WORKFLOW_COMPLETE: &str = "workflow-complete";

/// MCP server 状态变化
pub const MCP_SERVER_STATUS: &str = "mcp-server-status";
/// MCP server stderr 输出
pub const MCP_SERVER_STDERR: &str = "mcp-server-stderr";
/// MCP server 工具列表变化（notifications/tools/list_changed，P4.1）
pub const MCP_TOOLS_CHANGED: &str = "mcp-tools-changed";

/// 工具审批请求（等待用户确认）
pub const TOOL_APPROVAL_REQUESTED: &str = "tool-approval-requested";

/// BTW 旁路问答的流式增量（前端覆盖层）
pub const BTW_CHUNK: &str = "btw-chunk";

#[cfg(test)]
mod tests {
    use super::*;

    /// 事件名必须保持 kebab-case（前端监听依赖此约定）。
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
