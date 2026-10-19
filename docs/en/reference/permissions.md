# Permissions System Reference

## Three-Tier Model

AIO's permission system is based on a three-state decision model:

| Decision  | Behavior                          |
| --------- | --------------------------------- |
| **Allow** | Execute directly, no confirmation |
| **Ask**   | Requires user approval via dialog |
| **Deny**  | Refuse execution                  |

**Default policy:** When no rule matches, Ask is used (fail-closed).

## Evaluation Order

Permission evaluation follows these rules:

1. **Deny rules always take precedence over Allow** (safety first)
2. Matching rules are evaluated in descending `priority` order
3. When both Deny and Allow match simultaneously, **Deny wins**
4. No matching rule &rarr; Ask

## Built-in Default Rules

Overview of the default rules for each Agent mode:

### Chat Mode (Off)

- Deny all tool calls (`*` &rarr; Deny)

### Normal Mode

- Read operations (`read_file`, `list_directory`, `search_*`) &rarr; Allow
- Git read operations (`git_status`, `git_diff`, `git_log`) &rarr; Allow
- Write operations (`write_file`, `replace_in_file`, `make_directory`) &rarr; Ask
- Git write operations (`git_add`, `git_commit`) &rarr; Ask
- Delete operations (`delete_file`) &rarr; Ask
- Other tools &rarr; Ask

### Full Permission / Machine Full Trust Mode (Auto / Danger)

Both tiers share the same rule set (matching the `Auto | Danger` arm in `permission.rs`):

- Built-in file operations (read, write, replace, make directory, search) &rarr; Allow
- Git operations (all) &rarr; Allow
- Command execution (`execute_command`) &rarr; Allow
- Delete operations (`delete_file`) &rarr; Allow (aligned with `execute_command`, which can already `del`/`rm`; deletion stays confined to the project sandbox. Normal mode keeps per-call confirmation)
- Other tools &rarr; Allow

The difference between the tiers is not in permission rules but in **whether commands run inside the OS sandbox**: Full permission (Auto) keeps the AppContainer/bwrap/SBPL sandbox; Machine full trust (Danger) runs commands natively with full user privileges, entirely outside the sandbox (entry shows a dedicated red risk warning).

### Plan Mode

- Read operations (`read_file`, `list_directory`, `search_*`) &rarr; Allow
- Git read operations (`git_status`, `git_diff`, `git_log`) &rarr; Allow
- Write operations, make directory, delete &rarr; Deny
- Git write operations &rarr; Deny
- Command execution (`execute_command`) &rarr; Deny
- Web tools &rarr; Deny
- Other tools &rarr; Ask

> The full rule list can be found in the source at `permission.rs:default_rules_for_mode`.

## Command Sandbox and Network Egress

`execute_command` runs inside an OS-level sandbox with **no network by default**:

| Platform | Sandbox                                                              | File-write scope                                                  | Network (default)             |
| -------- | -------------------------------------------------------------------- | ----------------------------------------------------------------- | ----------------------------- |
| Linux    | bubblewrap (fail-closed per `sandboxRequired` when missing)          | project root + private /tmp only                                  | blocked via `--unshare-net`   |
| macOS    | sandbox-exec (SBPL)                                                  | project root only                                                 | blocked via `(deny network*)` |
| Windows  | AppContainer (per-project profile; disable via `sandboxWindows=off`) | project root + private temp dir + configured toolchain dirs (R+X) | blocked (no capability)       |

- Commands needing network (dependency downloads, API calls, localhost services) must declare `network: true` explicitly (Windows injects the internetClient capability; the approval dialog is annotated).
- **Fail-closed**: when the sandbox is unavailable on Linux/macOS, full-permission (Auto) mode refuses execution (`sandboxRequired` defaults on); other modes annotate the approval dialog with a sandbox-unavailable warning.
- Windows toolchain dirs (cargo/scoop user paths) are granted R+X via `sandboxWindowsToolchainDirs`; system dirs rely on built-in ALL APPLICATION PACKAGES ACEs.
- **`.git` history protection (`sandboxProtectGit`, default on)**: borrowed from Codex's sensitive-dir read-only net — inside the sandbox, `<project root>/.git` is **recursively read-only** for commands (Windows: the package SID's write grants are revoked with inheritance frozen; Linux: a read-only re-bind over the RW root bind; macOS: an SBPL deny rule), preventing prompt injection from rewriting Git history via the shell. Read-only git ops (`status/diff/log`) keep working; shell-level `git add/commit` is denied inside the sandbox — **use the builtin `git_*` tools for Git writes** (host process, with file locks and permission rules). Subagent worktree-isolated workspaces are exempt (their `.git` is a pointer file back into the main repo). Disable to restore sandbox-writable history. Known bounds: nested repos/submodules (`vendor/x/.git`) are not covered; moving the directory itself (`ren .git .git-old`) may succeed on some machines — the guarantee is **history cannot be rewritten** (the moved-away old tree stays read-only), on par with Codex's ro-bind.
- **Full-permission self-heal fallback switch (`sandboxUnsandboxedFallback`, default **off**)**: the last link of the Auto self-heal chain — "one unsandboxed retry for a NOT-FOUND denial when the session grant cannot land" — requires explicit opt-in. When off (default), the runtime never breaks the sandbox boundary: denied commands stay denied and the model must route around them (matching Codex's boundary philosophy: static config defines the edge, the runtime never overrides it).
- **Script-interpreter forwarding**: bare names of `python`/`python3`/`py`/`pip`/`node`/`npm`/`npx`/`git`/`cargo` (`.exe` suffix included) — as the first command token or inside a `cmd /c` wrapper — are rewritten to the host-PATH-resolved absolute path (PATH-only lookup; the project cwd is never consulted); inside the sandboxed shell these names otherwise always exit 9009. The Windows sandbox additionally grants the executable's install dir R+X on a best-effort basis. Command output is decoded UTF-8-first with an ANSI-code-page (GBK) fallback, so localized error text no longer arrives as mojibake.
- **Sandbox expansion request**: when a command is denied inside the Windows sandbox (WinError 5, "Access is denied"), or a forwarded interpreter reports "not recognized"/9009 inside it, a choice dialog pops — "Grant the interpreter directory and retry (this session)", "Grant and remember permanently (written to `sandboxWindowsToolchainDirs`)", "Run without sandbox (this once)", or "Ignore"; a free-form directory path can also be typed. At most one request per **operation** (`cmd /c X` and bare `X` share one key) per 30 minutes; no response within 600 seconds degrades to "Ignore" (unattended runs fall back to the original error and never stall the turn). **Full-permission (Auto) mode skips the dialog and is NOT throttled by the 30-minute window**, with the remediation surface narrowed per security review: only the "interpreter/toolchain invisible" class (not-found + a derivable directory) grants the session and retries — on success the directory is persisted permanently into `sandboxWindowsToolchainDirs` (annotated in the result text). **Access-denied denials (write targets, `.git` protection, out-of-project paths) NEVER escalate to unsandboxed execution** — the denial text comes from command output the model can influence (echoing "Access is denied" hits the signature), so it must never gate a boundary break. The only runtime exception: a NOT-FOUND denial whose grant failed to land (e.g. EDR-blocked ACL write) retries once outside the sandbox, requires `sandboxUnsandboxedFallback` (default off) and respects a per-turn budget; the unsandboxed retry is explicitly annotated in the tool result ("ran OUTSIDE the sandbox"; the network declaration is NOT enforced on that path).
- **Tool failure breaker**: a tool failing with consecutive **environment-class** errors (command not found / access denied / missing dependency — detected from the result text) reaches the threshold (default 6, configurable via `toolBreakerThreshold`) and is disabled for the rest of the turn; further calls short-circuit with guidance text. Business failures (the command ran and returned nonzero — e.g. failing tests) never count. From the 3rd consecutive env failure a remaining-budget warning rides along with the result. A success resets the counter.
- **Shell vs built-in tool boundary**: shell commands run under the sandbox token, so some OS-level operations (e.g. `cmd dir` directory enumeration) may be denied by the OS/endpoint security; built-in file tools (`list_directory`, …) execute in the host process and are unaffected — the behavioral difference is intended layering; prefer the built-in tools when probing.
- The lexical dangerous-command check is a defense-in-depth advisory layer (Critical always hard-blocked); OS sandboxing is the enforcement boundary.

## `.aio/permissions.json` File Format

### Top-Level Structure

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

### Field Reference

| Field       | Type   | Required | Description                           |
| ----------- | ------ | -------- | ------------------------------------- |
| `version`   | number | Yes      | File format version, currently `1`    |
| `updatedAt` | string | Yes      | ISO 8601 last update timestamp string |
| `rules`     | array  | Yes      | List of custom rules                  |

### Rule Field Reference

| Field           | Type                       | Required | Description                                                                                                                            |
| --------------- | -------------------------- | -------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| `id`            | string                     | Yes      | Unique rule identifier                                                                                                                 |
| `toolPattern`   | string                     | Yes      | Tool name glob pattern (`*` matches any character sequence)                                                                            |
| `serverId`      | string                     | No       | Restrict to a specific MCP Server                                                                                                      |
| `modes`         | string[]                   | Yes      | Empty array matches all Agent modes                                                                                                    |
| `action`        | "allow" \| "ask" \| "deny" | Yes      | Permission decision                                                                                                                    |
| `pathPattern`   | string                     | No       | Matches the `path` in tool arguments (glob)                                                                                            |
| `commandPrefix` | string                     | No       | `execute_command` only: exact match on the command's first token                                                                       |
| `auto`          | boolean                    | No       | Marks rules derived from the approval dialog's "Always allow"; command rules with `auto: true` only match chain-operator-free commands |
| `priority`      | number                     | No       | Higher values take precedence, default `10`                                                                                            |

### Glob Matching Rules

- Only `*` is supported (matches any character sequence)
- `?` and `[...]` are not supported
- Supports full matches and single leading or trailing `*`
- For example, `"delete_*"` matches `delete_file`, `"write_*"` matches `write_file`, `"*"` matches everything

## Examples

### Allow Normal mode to modify files in docs/, deny all delete operations

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

### Allow Auto mode to execute npm/pnpm commands

```json
{
  "id": "allow-package-managers",
  "toolPattern": "execute_command",
  "modes": ["auto"],
  "action": "allow",
  "priority": 30
}
```

### Globally forbid destructive operations like rm/del

```json
{
  "id": "deny-destructive-shell",
  "toolPattern": "execute_command",
  "modes": [],
  "action": "deny",
  "priority": 200
}
```

## Relationship with Modes

Custom rules override/stack on top of mode default rules. Deny rules always take precedence over Allow, regardless of whether they are custom. `priority` is only compared among rules of the same type -- Deny always beats Allow, no matter the priority values.

## "Always Allow" in the Approval Dialog

The approval bubble offers three actions: Deny / Always allow / Approve (once). For project-bound calls, "Always allow" **persists** a derived rule into the project's `.aio/permissions.json` (still effective after restarts; without a project context it degrades to a session-scoped rule):

- `execute_command` → a first-token prefix rule (approving `npm test` derives `commandPrefix: "npm"`); derived rules carry `auto: true` and only match commands **without chain operators** (`&&`/`||`/`;`/`|`/backticks/`$()`) — approving `cargo build` does not allow `cargo build && <anything>`;
- File write/delete/mkdir tools → an exact-path rule for **the same file only**;
- MCP tools → a rule scoped to the same server + same tool. This also covers tools of untrusted servers: once the rule lands, calls to that tool no longer ask per call (equivalent to pre-configuring it);
- Other tools (e.g. `delegate_task`) → a same-tool-name rule.

Derived rules use `priority: 50` and apply to all modes; Deny rules still win globally. Repeated clicks are deduplicated — no duplicate entries. Both the approval bubble and the sandbox-expansion dialog wait up to 600 seconds (a timeout is treated as "not approved / Ignore" and the bubble disappears).

## Notes

- `.aio/permissions.json` is re-read before every tool call, so manual edits take effect immediately; rules written by "Always allow" are effective right away too
- `permissions.json` is only needed when custom rules exist; do not create it manually
- Do not commit the `.aio/` directory containing sensitive paths or business information to version control
- The complete permission rule evaluation logic is in `permission.rs:check_permission`
