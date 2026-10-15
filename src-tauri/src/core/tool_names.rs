//! Built-in agent tool name constants (P3 魔法串：单一来源).
//!
//! 工具名此前以字符串字面量散布 14 个模块（subagent 配置表 / permission 内建
//! 规则 / mod 与 turn_runner 的分发与比较 / file_locks 与 diff 的写入分类 /
//! mcp 审批分类 …）——改名任一工具要全仓追字符串。此表是唯一来源：改一处
//! 即全仓生效。**MCP 服务器自定义工具名不在此列**（运行时来自服务器）；
//! glob 模式（`"delete_*"`/`"*_file"`）是模式字面量，非工具名，不收编。

/// Read a file's content from the project root.
pub const READ_FILE: &str = "read_file";
/// Create or overwrite a file.
pub const WRITE_FILE: &str = "write_file";
/// Partial edit of an existing file (expects old/new hash anchors).
pub const REPLACE_IN_FILE: &str = "replace_in_file";
/// Delete a file.
pub const DELETE_FILE: &str = "delete_file";
/// Grep-like content search in the project.
pub const SEARCH_CONTENT: &str = "search_content";
/// List a directory's entries.
pub const LIST_DIRECTORY: &str = "list_directory";
/// Create a directory (mkdir -p).
pub const MAKE_DIRECTORY: &str = "make_directory";
/// Execute a shell command inside the sandbox.
pub const EXECUTE_COMMAND: &str = "execute_command";
/// Fetch a web page (SSRF-guarded, HTTPS-only).
pub const WEB_FETCH: &str = "web_fetch";
/// Search the web.
pub const WEB_SEARCH: &str = "web_search";
/// The reasoning/thinking tool.
pub const THINK: &str = "think";
/// Project structure map tool.
pub const PROJECT_MAP: &str = "project_map";
/// Remember a fact into project memory.
pub const REMEMBER: &str = "remember";
/// Recall facts from project memory.
pub const RECALL: &str = "recall";
/// Semantic memory search.
pub const SEARCH_MEMORY: &str = "search_memory";
/// Delegate a task to a subagent.
pub const DELEGATE_TASK: &str = "delegate_task";
/// Batch-parallel delegation variant.
pub const DELEGATE_TASKS: &str = "delegate_tasks";
/// Wait/poll a background subagent's result.
pub const GET_SUBAGENT_RESULT: &str = "get_subagent_result";
/// Create the agent task board.
pub const CREATE_TASKS: &str = "create_tasks";
/// Update one task-board entry.
pub const UPDATE_TASK: &str = "update_task";
/// Ask the user a clarifying question.
pub const ASK_USER: &str = "ask_user";
