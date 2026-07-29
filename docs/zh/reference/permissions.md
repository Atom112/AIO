# 权限系统参考

## 三层模型

AIO 的权限系统基于三态决策模型：

| 决策      | 行为                 |
| --------- | -------------------- |
| **Allow** | 直接执行，无需确认   |
| **Ask**   | 需要用户审批弹窗确认 |
| **Deny**  | 拒绝执行             |

**默认策略：** 无匹配规则时使用 Ask（fail-closed）。

## 评估顺序

权限评估按照以下规则进行：

1. **Deny 规则总是优先于 Allow**（安全优先）
2. 匹配规则按 `priority` 降序评估
3. 同时匹配到 Deny 和 Allow 时，**Deny 获胜**
4. 无匹配规则 → Ask

## 内置默认规则

每个 Agent 模式的默认规则概览：

### Chat 模式（Off）

- 拒绝所有工具调用（`*` → Deny）

### Normal 模式（普通）

- 读操作（`read_file`、`list_directory`、`search_*`）→ Allow
- Git 读操作（`git_status`、`git_diff`、`git_log`）→ Allow
- 写操作（`write_file`、`replace_in_file`、`make_directory`）→ Ask
- Git 写操作（`git_add`、`git_commit`）→ Ask
- 删除操作（`delete_file`）→ Ask
- 其他工具 → Ask

### Auto 模式

- 内置文件操作（读、写、替换、建目录、搜索）→ Allow
- Git 操作（全部）→ Allow
- 命令执行（`execute_command`）→ Allow
- 删除操作（`delete_file`）→ Ask
- 其他工具 → Allow

### Plan 模式

- 读操作（`read_file`、`list_directory`、`search_*`）→ Allow
- Git 读操作（`git_status`、`git_diff`、`git_log`）→ Allow
- 写操作、建目录、删除 → Deny
- Git 写操作 → Deny
- Web 工具 → Deny
- 其他工具 → Ask

### Workflow 模式

- 常规文件操作（读、写、替换、搜索、建目录）→ Allow
- Git 操作 → Allow
- 命令执行 → Allow
- 删除操作 → Ask
- 其他工具 → Allow

> 具体规则列表可在源码 `permission.rs:default_rules_for_mode` 查看。

## `.aio/permissions.json` 文件格式

### 顶层结构

```json
{
  "version": 1,
  "updatedAt": "2026-07-26T00:00:00",
  "rules": [
    {
      "id": "allow-doc-writes",
      "toolPattern": "write_file",
      "modes": ["normal"],
      "action": "allow",
      "pathPattern": "docs/**",
      "priority": 20
    }
  ]
}
```

### 字段参考

| 字段        | 类型   | 必填 | 描述                          |
| ----------- | ------ | ---- | ----------------------------- |
| `version`   | number | 是   | 文件格式版本，当前为 `1`      |
| `updatedAt` | string | 是   | ISO 8601 最后更新时间的字符串 |
| `rules`     | array  | 是   | 自定义规则列表                |

### 规则字段参考

| 字段          | 类型                       | 必填 | 描述                                     |
| ------------- | -------------------------- | ---- | ---------------------------------------- |
| `id`          | string                     | 是   | 规则唯一标识                             |
| `toolPattern` | string                     | 是   | 工具名 glob 模式（`*` 匹配任意字符序列） |
| `serverId`    | string                     | 否   | 限定到指定 MCP Server                    |
| `modes`       | string[]                   | 是   | 空数组匹配所有 Agent 模式                |
| `action`      | "allow" \| "ask" \| "deny" | 是   | 权限决策                                 |
| `pathPattern` | string                     | 否   | 匹配工具参数中的 `path`（glob）          |
| `priority`    | number                     | 否   | 越大越优先，默认 `10`                    |

### Glob 匹配规则

- 仅支持 `*`（匹配任意字符序列）
- 不支持 `?` 或 `[...]`
- 支持完整匹配以及开头或结尾的单个 `*`
- 如 `"delete_*"` 匹配 `delete_file`，`"write_*"` 匹配 `write_file`，`"*"` 匹配所有

## 示例

### 允许 Normal 模式修改 docs/ 内文件，拒绝所有删除操作

```json
{
  "version": 1,
  "updatedAt": "2026-07-26T00:00:00",
  "rules": [
    {
      "id": "allow-doc-writes",
      "toolPattern": "write_file",
      "modes": ["normal"],
      "action": "allow",
      "pathPattern": "docs/*",
      "priority": 20
    },
    {
      "id": "deny-delete",
      "toolPattern": "delete_*",
      "modes": [],
      "action": "deny",
      "priority": 100
    }
  ]
}
```

### 允许 Auto 模式执行 npm/pnpm 命令

```json
{
  "id": "allow-package-managers",
  "toolPattern": "execute_command",
  "modes": ["auto"],
  "action": "allow",
  "priority": 30
}
```

### 全局禁止 rm/del 等破坏性操作

```json
{
  "id": "deny-destructive-shell",
  "toolPattern": "execute_command",
  "modes": [],
  "action": "deny",
  "priority": 200
}
```

## 与模式的关系

自定义规则覆盖/叠加在模式默认规则之上。Deny 规则无论是否自定义都优先于 Allow。`priority` 仅在同类型规则间比较——Deny 始终比 Allow 优先，无论两者的 priority 值。

## 注意事项

- 修改 `.aio/permissions.json` 后需要重新进入项目，让配置重新加载
- `permissions.json` 只在存在自定义规则时需要，不必手动创建
- 不要提交包含敏感路径或业务信息的 `.aio/` 目录到版本控制
- 完整权限规则评估逻辑在 `permission.rs:check_permission`
