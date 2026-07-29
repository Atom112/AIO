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

### Auto Mode

- Built-in file operations (read, write, replace, make directory, search) &rarr; Allow
- Git operations (all) &rarr; Allow
- Command execution (`execute_command`) &rarr; Allow
- Delete operations (`delete_file`) &rarr; Ask
- Other tools &rarr; Allow

### Plan Mode

- Read operations (`read_file`, `list_directory`, `search_*`) &rarr; Allow
- Git read operations (`git_status`, `git_diff`, `git_log`) &rarr; Allow
- Write operations, make directory, delete &rarr; Deny
- Git write operations &rarr; Deny
- Web tools &rarr; Deny
- Other tools &rarr; Ask

### Workflow Mode

- Regular file operations (read, write, replace, search, make directory) &rarr; Allow
- Git operations &rarr; Allow
- Command execution &rarr; Allow
- Delete operations &rarr; Ask
- Other tools &rarr; Allow

> The full rule list can be found in the source at `permission.rs:default_rules_for_mode`.

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

| Field         | Type                       | Required | Description                                                 |
| ------------- | -------------------------- | -------- | ----------------------------------------------------------- |
| `id`          | string                     | Yes      | Unique rule identifier                                      |
| `toolPattern` | string                     | Yes      | Tool name glob pattern (`*` matches any character sequence) |
| `serverId`    | string                     | No       | Restrict to a specific MCP Server                           |
| `modes`       | string[]                   | Yes      | Empty array matches all Agent modes                         |
| `action`      | "allow" \| "ask" \| "deny" | Yes      | Permission decision                                         |
| `pathPattern` | string                     | No       | Matches the `path` in tool arguments (glob)                 |
| `priority`    | number                     | No       | Higher values take precedence, default `10`                 |

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

## Notes

- After modifying `.aio/permissions.json`, re-enter the project to reload the configuration
- `permissions.json` is only needed when custom rules exist; do not create it manually
- Do not commit the `.aio/` directory containing sensitive paths or business information to version control
- The complete permission rule evaluation logic is in `permission.rs:check_permission`
