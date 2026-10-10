//! Reserved internal identifier constants — avoids magic strings scattered repo-wide.

/// Reserved id of the built-in filesystem MCP server.
///
/// Permission rules bind to this id (originally served by the stdio `mcp_fs_server.rs`);
/// in-process file tools keep it for compatibility with existing `.aio/permissions.json`
/// and project configs. Do not change the string value (breaks permission rule matching).
pub const BUILTIN_FS_SERVER_ID: &str = "__aio-filesystem__";

/// Caller identifier for agent-loop built-in tools (not routed through MCP).
pub const BUILTIN_SERVER_ID: &str = "__builtin__";
