//! 内部保留标识符常量 —— 避免魔法字符串散落全仓（历史：`__builtin__` 曾出现于
//! 25 处、`__aio-filesystem__` 曾出现于 22 处，均为裸字面量）。

/// 内置文件系统 MCP server 的保留 id。
///
/// 历史别名：早期文件工具经 `mcp_fs_server.rs`（stdio）提供，权限规则与该 id 绑定；
/// 现进程内文件工具仍沿用同一 id 以兼容存量 `.aio/permissions.json` 与项目配置。
/// 勿改字符串值（会破坏存量权限规则匹配）。
pub const BUILTIN_FS_SERVER_ID: &str = "__aio-filesystem__";

/// Agent 循环内部工具的调用方标识（非 MCP 通道的内置工具）。
pub const BUILTIN_SERVER_ID: &str = "__builtin__";
