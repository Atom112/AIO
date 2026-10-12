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
- 删除操作（`delete_file`）→ Allow（与 `execute_command` 对齐：命令本就能 `del`/`rm`，且删除被沙箱限定在项目内；Normal 模式保持逐次确认）
- 其他工具 → Allow

### Plan 模式

- 读操作（`read_file`、`list_directory`、`search_*`）→ Allow
- Git 读操作（`git_status`、`git_diff`、`git_log`）→ Allow
- 写操作、建目录、删除 → Deny
- Git 写操作 → Deny
- 命令执行（`execute_command`）→ Deny
- Web 工具 → Deny
- 其他工具 → Ask

### Workflow 模式

- 常规文件操作（读、写、替换、搜索、建目录）→ Allow
- Git 操作 → Allow
- 命令执行 → Allow
- 删除操作 → Allow
- 其他工具 → Allow

> 具体规则列表可在源码 `permission.rs:default_rules_for_mode` 查看。

### 工作流步骤的权限模式

工作流（`create_workflow`）的步骤默认**继承启动时的 Agent 模式**：从 Normal 模式启动的工作流，其步骤内的写操作仍会逐次弹出审批。可在应用配置中设置 `workflowStepMode` 为 `"auto"` 恢复旧行为（步骤自动放行，适合完全信任的自动化场景）。

## 命令沙箱与网络出口

`execute_command` 在 OS 级沙箱内执行，**默认无网络**：

| 平台    | 沙箱机制                                                    | 文件写范围                                      | 网络（默认）           |
| ------- | ----------------------------------------------------------- | ----------------------------------------------- | ---------------------- |
| Linux   | bubblewrap（缺失时按 `sandboxRequired` fail-closed）        | 仅项目根 + 专用 /tmp                            | `--unshare-net` 阻断   |
| macOS   | sandbox-exec（SBPL）                                        | 仅项目根                                        | `(deny network*)` 阻断 |
| Windows | AppContainer（每项目 profile；`sandboxWindows=off` 可关闭） | 仅项目根 + 专用临时目录 + 配置的工具链 R+X 目录 | 无 capability = 阻断   |

- 命令需要联网（下载依赖、API 调用、访问 localhost 服务）时，模型须显式声明 `network: true`（Windows 注入 internetClient capability；审批弹窗标注"需要网络访问"）。
- **fail-closed**：Linux/macOS 沙箱不可用时，Auto/Workflow 模式直接拒绝执行（`sandboxRequired` 默认开）；Normal 模式审批弹窗附带沙箱不可用警示。
- Windows 工具链目录（cargo/scoop 等用户安装路径）经 `sandboxWindowsToolchainDirs` 配置追加 R+X；系统目录依赖内置 ALL APPLICATION PACKAGES ACE。
- **脚本解释器转发**：`python`/`python3`/`py`/`pip`/`node`/`npm`/`npx`/`git`/`cargo`（含 `.exe` 后缀）作为首命令或 `cmd /c` 包裹内的裸名出现时，自动改写为宿主 PATH 解析的绝对路径（仅 PATH 查找，不查项目目录）——沙箱 shell 内这些名字原本一律 9009；Windows 沙箱同时对其安装目录做尽力 R+X 授权。命令输出按「UTF-8 优先、系统 ANSI 代码页（GBK）回退」解码，中文错误信息不再乱码。
- **沙箱扩权请求**：命令在 Windows 沙箱内被拒绝（WinError 5「拒绝访问 / Access is denied」），或转发目标解释器在沙箱内「不是内部或外部命令 / 9009」时，弹出选择对话框——「授权解释器目录并重试（本次会话）」「授权并永久记住（写入 `sandboxWindowsToolchainDirs`）」「跳出沙箱执行（仅本次）」「忽略」，也支持直接输入任意目录路径授权。**同一操作**（`cmd /c X` 与裸 `X` 视为同一操作）30 分钟内最多询问一次；60 秒无响应按「忽略」处理（无人值守自动降级为原始错误，不会卡住回合）。
- **工具失败熔断器**：同一工具连续**环境类**失败（命令不存在 / 权限被拒 / 缺依赖——按结果文本签名判定）达到阈值（默认 6，`toolBreakerThreshold` 可配）后，本回合内停用该工具，后续调用短路返回引导文案；业务类失败（命令正常运行后返回非零，如测试未通过）**不计入**熔断。从第 3 次连续环境类失败起，结果文本附带剩余预算预警。成功调用重置计数。
- **shell 与内置工具的权限边界**：shell 命令运行在沙箱 token 下，部分系统级操作（如 `cmd dir` 的目录枚举）可能被 OS/安全软件拒绝；内置文件工具（`list_directory` 等）经宿主进程执行，不受此影响——二者行为差异属预期分层，排查问题时优先用内置工具。
- 危险命令词法检测是 defense-in-depth 预警层（Critical 级无条件硬阻断），安全强制以 OS 沙箱为准。

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
