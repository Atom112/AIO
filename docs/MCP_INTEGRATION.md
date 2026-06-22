# MCP（Model Context Protocol）集成设计文档

> 版本：P1 (stdio + HTTP+SSE) · 状态：实现中
> 作者：AIO Team · 最后更新：2026-06-20

---

## 一、目标

将 **MCP 服务器**作为**即插即用**的扩展能力集成进 AIO：

- 用户在设置页**配置一个 MCP server → 立即可用**
- 模型在聊天中**自动调用工具 → 结果回灌 → 继续推理**
- 开发者新增传输类型**无需修改核心代码**

---

## 二、整体架构

```
┌─────────────────────────────────────────────────────────────┐
│ Frontend (SolidJS)                                          │
│   ├─ SettingsPage: McpServerList / McpServerDetail          │
│   └─ ChatInterface: tool_call 气泡 + 自动执行 + 结果回传     │
└─────────────────┬───────────────────────────────────────────┘
                  │ invoke + listen events
┌─────────────────▼───────────────────────────────────────────┐
│ Tauri Commands                                              │
│   mcp::list_servers / add_server / start / stop             │
│   mcp::list_tools / call_tool                               │
│   llm::call_llm_stream  ◄── 扩展: tools + tool_calls 解析   │
└─────────────────┬───────────────────────────────────────────┘
                  │
┌─────────────────▼───────────────────────────────────────────┐
│ plugins/mcp/  (镜像 plugins/engine/ 模式)                  │
│   ├─ McpServerPlugin trait                                  │
│   ├─ McpServerManager (HashMap<id, Arc<dyn Plugin>>)        │
│   ├─ 内置插件: stdio  / http  / streamable_http (P2)        │
│   └─ JSON-RPC 2.0 codec + McpConnection 抽象                │
│                                                              │
│ core/state.rs                                                │
│   ├─ McpServerState:  Mutex<HashMap<id, McpConnection>>    │
│   └─ McpRequestManager: DashMap<call_id, JoinHandle>        │
└─────────────────────────────────────────────────────────────┘
```

---

## 三、模块文件清单

### 3.1 Rust 后端

| 路径 | 作用 | 状态 |
|------|------|------|
| `src-tauri/src/plugins/mcp/mod.rs` | `McpServerPlugin` trait + `McpServerManager` + 持久化 | P1 |
| `src-tauri/src/plugins/mcp/connection.rs` | `McpConnection` 抽象 + JSON-RPC 2.0 codec | P1 |
| `src-tauri/src/plugins/mcp/error.rs` | `McpError` 错误类型 | P1 |
| `src-tauri/src/plugins/mcp/stdio.rs` | stdio 传输插件 | P1 |
| `src-tauri/src/plugins/mcp/http.rs` | HTTP+SSE 传输插件 | P1 |
| `src-tauri/src/plugins/mcp/streamable.rs` | streamable HTTP 传输插件 | P2 |
| `src-tauri/src/plugins/mcp/catalog.rs` | 远端 catalog 拉取 | P3 |
| `src-tauri/src/commands/mcp.rs` | 9 个 Tauri command | P1 |
| `src-tauri/src/core/models.rs` | Message +tool 字段；新增 ToolSpec / ToolCall / McpServerConfig | P1 |
| `src-tauri/src/core/state.rs` | `McpServerState` + `McpRequestManager` | P1 |
| `src-tauri/src/core/db.rs` | messages 表 +tool_call_id / name / tool_calls_json | P1 |
| `src-tauri/src/core/secure_store.rs` | accounts 扩展 mcp-server env 命名 | P1 |
| `src-tauri/src/commands/llm.rs` | call_llm_stream 接受 tools + 解析 tool_calls | P1 |
| `src-tauri/src/lib.rs` | 注册 9 个 mcp 命令 + manage Manager + window 关闭清理 | P1 |

### 3.2 前端

| 路径 | 作用 | 状态 |
|------|------|------|
| `src/core/types/mcp.ts` | TS 类型（与 Rust 镜像） | P1 |
| `src/core/store/store.ts` | mcpServers / mcpServerStatus / mcpToolsCache signals | P1 |
| `src/core/utils/mcp.ts` | parseEnvWithKeyring / mcpTransportLabel 等 | P1 |
| `src/features/settings/components/McpServerList.tsx` | 列表 + 启停 + 状态 | P1 |
| `src/features/settings/components/McpServerDetail.tsx` | 表单 + 测试连接 + 工具白名单 | P1 |
| `src/features/settings/SettingsPage.tsx` | 加 `/settings/mcp` 路由 | P1 |
| `src/components/NavBar.tsx` | 侧边栏加 "MCP 服务器" 入口 | P1 |
| `src/app/router.ts` | 路由表加 `/settings/mcp` | P1 |
| `src/features/chat/components/ToolCallBubble.tsx` | 折叠式 tool_call 气泡 | P1 |
| `src/features/chat/components/ChatInterface.tsx` | 渲染 tool_calls 消息 + role="tool" | P1 |
| `src/features/chat/ChatPage.tsx` | 工具调用循环 + 5 轮上限 | P1 |
| `src/features/settings/components/McpCatalogBrowser.tsx` | 远端 catalog 浏览 | P3 |

---

## 四、核心数据结构

### 4.1 Message 扩展

```rust
// core/models.rs
pub struct Message {
    // 既有字段 ...
    pub id: Option<String>,
    pub role: String,
    pub content: serde_json::Value,
    pub model_id: Option<String>,
    pub display_files: Option<Vec<FileMeta>>,
    pub display_text: Option<String>,
    // 新增（#[serde(default)] 兼容旧数据）
    pub tool_call_id: Option<String>,         // role="tool" 时的 call id
    pub name: Option<String>,                 // role="tool" 时的函数名
    pub tool_calls: Option<Vec<ToolCall>>,    // role="assistant" 时的 tool_calls 数组
}
```

### 4.2 工具 / 工具调用

```rust
pub struct ToolSpec {
    #[serde(rename = "type")] pub kind: String,   // "function"
    pub function: ToolFunctionSpec,
}

pub struct ToolFunctionSpec {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,    // JSON Schema
}

pub struct ToolCall {
    pub id: String,                       // "call_abc123"
    #[serde(rename = "type")] pub kind: String,    // "function"
    pub function: ToolCallFunction,
}

pub struct ToolCallFunction {
    pub name: String,
    pub arguments: String,                // JSON 字符串（OpenAI 规范）
}

pub struct ToolResult {
    pub content: Vec<ToolResultContent>,  // 文本 / 图片 / 资源
    pub is_error: bool,
}

pub struct ToolResultContent {
    #[serde(rename = "type")] pub kind: String,    // "text" | "image" | "resource"
    #[serde(flatten)] pub data: serde_json::Value,
}
```

### 4.3 MCP 传输与服务器配置

```rust
#[serde(tag = "transport", rename_all = "lowercase")]
pub enum McpTransport {
    Stdio {
        command: String,
        args: Vec<String>,
        env: BTreeMap<String, String>,    // 密钥值支持 ${KEYRING:account_id} 占位
        cwd: Option<String>,
    },
    Http {
        url: String,
        headers: BTreeMap<String, String>,
    },
    StreamableHttp {
        url: String,
        headers: BTreeMap<String, String>,
    },
}

pub struct McpServerConfig {
    pub id: String,                       // "mcp-{slug}"
    pub display_name: String,
    pub enabled: bool,
    pub transport: McpTransport,
    pub enabled_tools: Vec<String>,       // [] = 全部启用
    pub auto_start: bool,
    pub has_stored_secret: bool,
    pub from_catalog: Option<CatalogRef>,
}

pub struct CatalogRef {
    pub catalog_id: String,
    pub source_id: String,
}
```

### 4.4 持久化文件

`$APPDATA/com.loch.aio/mcp-servers.json`：

```json
{
  "version": 1,
  "updatedAt": "2026-06-20T12:00:00Z",
  "servers": {
    "mcp-filesystem": {
      "id": "mcp-filesystem",
      "displayName": "本地文件系统",
      "enabled": true,
      "transport": {
        "transport": "stdio",
        "command": "npx",
        "args": ["-y", "@modelcontextprotocol/server-filesystem", "."],
        "env": {}
      },
      "enabledTools": [],
      "autoStart": true,
      "hasStoredSecret": false
    }
  }
}
```

---

## 五、插件 trait（镜像 LocalEnginePlugin）

```rust
// plugins/mcp/mod.rs
#[async_trait]
pub trait McpServerPlugin: Send + Sync {
    fn identifier(&self) -> &'static str;        // "stdio" / "http" / "streamable_http"

    async fn start(
        &self,
        app: AppHandle,
        config: &McpServerConfig,
    ) -> Result<McpConnection, McpError>;

    async fn initialize(
        &self,
        conn: &McpConnection,
    ) -> Result<ServerInfo, McpError>;            // MCP 协议握手

    async fn list_tools(
        &self,
        conn: &McpConnection,
    ) -> Result<Vec<ToolSpec>, McpError>;

    async fn call_tool(
        &self,
        conn: &McpConnection,
        name: &str,
        arguments: serde_json::Value,
        timeout: Duration,
    ) -> Result<ToolResult, McpError>;

    async fn stop(&self, conn: McpConnection) -> Result<(), McpError>;
}

pub struct McpServerManager {
    plugins: HashMap<String, Arc<dyn McpServerPlugin>>,
}

impl McpServerManager {
    pub fn builtin() -> Self {
        // 注册 stdio / http；streamable 在 P2
    }
    pub fn register(&mut self, plugin: Arc<dyn McpServerPlugin>) { ... }
    pub fn get(&self, id: &str) -> Option<Arc<dyn McpServerPlugin>> { ... }
}
```

**新增传输类型的步骤**（开发者）：
1. 在 `plugins/mcp/` 下新建 `xxx.rs`
2. 实现 `McpServerPlugin` trait
3. 在 `McpServerManager::builtin()` 中 `register(Arc::new(XxxPlugin))`
4. 在 `McpTransport` 枚举中加新变体
5. 完。无需修改核心代码。

---

## 六、状态管理

```rust
// core/state.rs
/// 所有已连接 MCP server 的连接池
pub struct McpServerState(pub Mutex<HashMap<String, Arc<McpConnection>>>);

/// 在途工具调用（用于 abort / 取消）
pub struct McpRequestManager(pub Arc<DashMap<String, JoinHandle<()>>>);
```

注册在 `lib.rs`：
```rust
.manage(McpServerState::default())
.manage(McpRequestManager::default())
.manage(McpServerManager::builtin())
```

---

## 七、Tauri Commands

`src-tauri/src/commands/mcp.rs` 导出 9 个：

| 命令 | 入参 | 返回 | 说明 |
|------|------|------|------|
| `list_mcp_servers` | — | `Vec<McpServerConfig>` | 列出所有配置 |
| `add_mcp_server` | `config: McpServerConfig` | `()` | 新增/更新（按 id upsert） |
| `remove_mcp_server` | `id: String` | `()` | 删除（含 keyring 清理） |
| `start_mcp_server` | `id: String` | `Vec<ToolSpec>` | 启动并返回工具列表 |
| `stop_mcp_server` | `id: String` | `()` | 停止连接 |
| `list_mcp_server_status` | — | `HashMap<String, McpStatus>` | 全部状态 |
| `list_mcp_tools` | — | `Vec<ToolSpec>` | 全部已连接 server 的合并工具列表 |
| `call_mcp_tool` | `server_id, name, args, call_id` | `ToolResult` | 单次调用 |
| `test_mcp_server_connection` | `config: McpServerConfig` | `Vec<ToolSpec>` | 测试但不保存 |

**事件**：

| 事件名 | 负载 | 时机 |
|--------|------|------|
| `llm-tool-call` | `{assistant_id, topic_id, tool_call_id, name, arguments}` | LLM 流式累积完一个 tool_call |
| `mcp-server-status` | `{id, status, message?}` | server 启停 / 错误 |
| `mcp-tool-progress` | `{call_id, percent}` | 长任务进度（P1 不发） |

---

## 八、关键数据流

### 8.1 启动聊天（含工具）

```
ChatPage.handleSendMessage
  │
  ├─ invoke list_mcp_tools() → Vec<ToolSpec>
  │    (过滤 disabled server + enabled_tools 白名单)
  │
  └─ invoke call_llm_stream({tools, messages, ...})
       │
       ▼
  LLM 流式返回
  ├─ delta.content       → llm-chunk 事件（现有）
  └─ delta.tool_calls    → 累积（按 index 拼接 arguments）
                            完成时发 llm-tool-call 事件
```

### 8.2 工具调用循环

```
ChatPage 监听 llm-tool-call
  │
  ├─ 1. 渲染 tool_call 气泡（折叠、loading 态）
  │
  ├─ 2. invoke call_mcp_tool(serverId, name, args, callId)
  │      ↓ (默认 30s 超时)
  │      返回 ToolResult
  │
  ├─ 3. 追加消息：{role:"tool", tool_call_id, name, content}
  │
  ├─ 4. 递归 invoke call_llm_stream(带新消息)
  │
  └─ 5. 轮数 +1；若 ≥ 5 → 终止并提示 "已达工具调用上限"
```

### 8.3 边界处理

| 场景 | 行为 |
|------|------|
| 多个 tool_calls | **顺序执行**（先全部收完再依次调用，避免上下文错乱） |
| 工具错误 | 错误信息作为 `role="tool" content` 回灌，LLM 决定下一步 |
| 超时（>30s） | 错误信息回灌，带 `[Tool timeout after 30s]` |
| 用户停止 | abort McpRequestManager 中所有在途调用 |
| 5 轮上限 | 中断循环，提示 "工具调用超过 5 轮，已停止" |
| 工具结果 > 64KB | 截断 + `[Truncated]` 标记 |

---

## 九、JSON-RPC 2.0 协议

MCP 协议基于 JSON-RPC 2.0 over stdio / HTTP / SSE。

**Request**：
```json
{ "jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": null }
```

**Response**：
```json
{ "jsonrpc": "2.0", "id": 1, "result": { "tools": [...] } }
```

**Error**：
```json
{ "jsonrpc": "2.0", "id": 1, "error": { "code": -32601, "message": "Method not found" } }
```

**Notification**（无 id）：
```json
{ "jsonrpc": "2.0", "method": "notifications/message", "params": {...} }
```

**id 关联**：用 `DashMap<u64, oneshot::Sender<JsonRpcResponse>>` 关联 in-flight 请求和响应。

---

## 十、数据库迁移

`messages` 表新增 3 列（`core/db.rs::init_db` 启动时检查并 ALTER）：

```sql
ALTER TABLE messages ADD COLUMN tool_call_id TEXT;
ALTER TABLE messages ADD COLUMN name TEXT;
ALTER TABLE messages ADD COLUMN tool_calls_json TEXT;
```

按 `core/db.rs:51-67` 模式：先 `PRAGMA table_info` 检查再条件 ALTER。

---

## 十一、安全模型

| 风险 | 措施 |
|------|------|
| env 字段泄露密钥 | 落盘前替换为 `${KEYRING:account_id}` 占位符；运行时反向解析 |
| stdio 子进程权限过大 | **不继承**系统 env，仅传白名单 env 字段 |
| args 命令注入 | `tokio::process::Command` 直接传 args，**不走 shell** |
| HTTP 远程 server 不可信 | P1 不做 host 白名单（仅依赖 HTTPS + 凭据） |
| 工具死循环 | 单次响应最多 **5 轮** 工具调用 |
| 工具返回过大 | 截断到 **64KB** |
| tools/list 返回过大 | 限制单个 server 最多 **100 工具** |

---

## 十二、阶段切分

| Phase | 范围 | 估时 | 状态 |
|-------|------|------|------|
| **P1** | 全部基础 + stdio + HTTP+SSE + 折叠气泡 UI + 自动执行 | 4-5 天 | **实现中** |
| **P2** | streamable HTTP 传输 | 0.5-1 天 | 未开始 |
| **P3** | 远端 catalog + 一键安装 | 1-2 天 | 未开始 |
| **P4** | OAuth / sampling / roots / per-tool 权限 | 视需求 | 未规划 |

---

## 十三、P1 验证计划

```bash
cd src-tauri && cargo check          # 0 errors
cd src-tauri && cargo build          # 0 errors / 0 warnings
cd src-tauri && cargo test --lib     # 单元测试（含 JSON-RPC codec）
npx tsc --noEmit                     # 0 errors
npm run build                        # vite build 成功
```

**端到端测试用例**（P1 必须全部通过）：
1. 添加 `npx -y @modelcontextprotocol/server-filesystem` → 启动 → 列出 read_file/write_file
2. 聊天问"列出当前目录的 markdown 文件" → 模型调用 read_file → 返回结果 → 气泡显示
3. 多轮：问"读取 foo.md 然后写 bar.md" → 模型顺序调用两个工具
4. 工具错误：删除文件后调用 → 错误回灌 → 模型友好回复
5. 启停：禁用 server → 工具从 tools 列表消失
6. 白名单：仅勾选 read_file → 写操作不被允许
7. 5 轮上限：构造死循环 → 第 6 轮自动中断
8. 用户停止：点停止 → 在途工具调用 abort

---

## 十四、决策记录

| 决策 | 选择 | 原因 |
|------|------|------|
| 库选型 | 手写 JSON-RPC | 零依赖、~300 行可控、未来可迁移 rmcp |
| 工具权限 | 全自动执行 | 简化 UX，后续可加 per-tool 审批 |
| 配置持久化 | 独立 mcp-servers.json | 与 provider-configs 平级，版本化迁移清晰 |
| UI 形态 | 折叠式小气泡 | 不打断主对话流 |
| 调用超时 | 30 秒 | LLM 上下文窗口压力与用户等待感平衡 |
| 轮数上限 | 5 轮 | 防止 LLM 死循环，参考 Claude 工具规范 |
| enabled_tools 语义 | 空 = 全部启用 | 降低使用门槛 |
| 结果显示 | 完整结果 | 成功详情对调试与信任有重要价值 |
| Catalog URL | github.com/Atom112/aio-mcp-catalog | 与 models-catalog 仓库结构对齐 |
| P1 范围 | stdio + HTTP+SSE | streamable 推后，最常用传输优先 |
