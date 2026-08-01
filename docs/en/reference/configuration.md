# Configuration Reference

## Storage Location

AIO uses the app data directory and system configuration directory provided by Tauri, with the app identifier `com.loch.aio`. The exact root path depends on the operating system and installation method and should not be hardcoded in scripts.

Primary data includes:

| File or Directory               | Content                                                                                  |
| ------------------------------- | ---------------------------------------------------------------------------------------- |
| `chat_history.db`               | Assistants, topics, messages, attachment relations, project associations, and usage logs |
| `attachments/`                  | Attachments deduplicated by SHA-256                                                      |
| `provider-configs.json`         | Provider configuration, excluding newly written plaintext API keys                       |
| `mcp-servers.json`              | Global MCP Servers                                                                       |
| `skills.json`                   | Global Skills                                                                            |
| `projects.json`                 | Project paths and metadata                                                               |
| `models-catalog.json`           | Updated model catalog                                                                    |
| `mcp-registry-cache.json`       | MCP Registry cache                                                                       |
| `skill-market-cache.json`       | Skill Market cache                                                                       |
| `profile-model-overrides.json`  | Sub-agent model overrides                                                                |
| `custom-subagent-profiles.json` | Custom sub-agent profiles                                                                |
| `config.json`                   | App general configuration                                                                |

The repository retains legacy read/write paths for `activated_models.json` and `fetched_models.json` for backward compatibility. The local model list continues to use activated model data.

## App Configuration

`config.json` currently persists:

| Field              | Behavior                                                         |
| ------------------ | ---------------------------------------------------------------- |
| `api_url`          | Default API URL for backward compatibility                       |
| `default_model`    | Default model for backward compatibility                         |
| `local_model_path` | Local model path for backward compatibility                      |
| `knowledgeEnabled` | Whether cross-session project memory is enabled, default `false` |
| `autoStartEnabled` | Whether to launch at system startup, default `false`             |

The backend `AppConfig` also includes the following runtime fields:

| Field                    | Current Effective Values               |
| ------------------------ | -------------------------------------- |
| `auto_retry_enabled`     | Default `true`                         |
| `auto_retry_count`       | Default `2`                            |
| `auto_retry_delay_ms`    | Default `500` ms                       |
| `maxConcurrentSubagents` | Concurrent cap `5` when not configured |

These retry and concurrency fields currently have no settings UI and are not persisted by the disk structure of `config.json`. Do not attempt to modify runtime values by manually adding JSON fields; extending load, save, and settings UI is required before exposing them.

## SQLite Data

`chat_history.db` uses SQLite with idempotent migrations applied at startup. The main tables include:

- `assistants`;
- `topics`;
- `messages`;
- `attachments` and `message_attachments`;
- `usage_log`.

Foreign key constraints are enabled. Deleting an assistant or topic cascades to clean up messages; attachment files are removed once they lose all message associations. Agent steps, tool calls, reasoning content, token usage, and file changes are stored alongside messages, without using a single JSON history file.

## System Credential Store

Provider API keys and MCP environment variables/header secrets are preferentially written to the system credential store. JSON files only save status indicators such as `hasStoredKey`, `hasStoredSecret`, and `${KEYRING:account}` placeholders.

When the platform credential store is unavailable, the secure storage module falls back to an encrypted file `secure-store.json` in the app data directory. This is still sensitive data:

- Do not commit, sync, or paste into an Issue;
- Do not manually edit;
- When migrating devices, do not assume old system credentials are directly readable.

Old Provider configurations may still carry a legacy `apiKey` field; new code no longer writes keys to disk in plaintext. After opening an old configuration, re-save it through the settings page, then check your backup for lingering old keys.

## WebView localStorage

The following UI preferences are stored in WebView `localStorage` and are not part of project configuration:

| Key or Category                    | Content                           |
| ---------------------------------- | --------------------------------- |
| `aio-shortcut-bindings`            | Custom keyboard shortcuts         |
| `theme-color`                      | Theme color                       |
| `chat-reasoning-level`             | Reasoning strength                |
| `chat-web-search`                  | Web search toggle                 |
| `aio-last-agent-project-id`        | Last used Agent project           |
| Left/right sidebar state and width | Chat layout                       |
| `aio_ignored_update_version`       | Ignored update version            |
| `aio-local-auto-start-confirmed`   | Local engine startup confirmation |
| `user-avatar-path`                 | User avatar file path             |

Clearing WebView data will reset these UI preferences but will not delete SQLite chat history, Provider configurations, or project `.aio/` directories.

## Project Configuration

Creating a project initializes:

```text
.aio/
├── mcp-servers.json
└── skills.json
```

Additional files may appear after feature usage:

```text
.aio/
├── permissions.json
└── knowledge.json
```

- `permissions.json`: Allow, ask, and deny rules for project tools. See [Permissions System Reference](permissions.md);
- `knowledge.json`: Cross-session knowledge, with categories `decision`, `pattern`, `convention`, or `note`, retaining up to 50 entries.

## Attachment Whitelist

| Type  | Extensions                                                                                                                                | Size Limit |
| ----- | ----------------------------------------------------------------------------------------------------------------------------------------- | ---------- |
| Image | png, jpg, jpeg, webp                                                                                                                      | 10 MiB     |
| Doc   | pdf, docx, pptx                                                                                                                           | 30 MiB     |
| Text  | txt, md, json, csv, log, xml, yaml, yml, ini, tsv                                                                                        | 5 MiB      |
| Source| rs, c, h, cpp, hpp, cc, cxx, cs, go, java, rb, py, js, mjs, cjs, ts, tsx, jsx, php, swift, kt, kts, scala, lua, sql, toml, sh, bash, zsh, dart, html, css, scss, less, vue, svelte, gradle, properties, r, pl | 5 MiB      |

Extension, size, and path safety (absolute path, no `..`) are checked before parsing. The file location is not restricted: files the user explicitly picks via the system file picker may live anywhere on disk. Images are converted to base64 data URIs for model requests; other types are extracted as text. Original attachments are copied by hash into the app data directory, and the database maintains reference relationships.

## Network and Command Security

- URLs for Providers, MCP, and web tools undergo HTTP URL validation and SSRF protection.
- A Provider's `proxyUrl` affects only that Provider and does not become a global system proxy.
- stdio MCP uses command and argument arrays to launch, not concatenated shell strings.
- The Agent Shell tool classifies and checks for delete, overwrite, and other dangerous commands.
- File tools restrict paths to the project root or the app-allowed sandbox.
- MCP and built-in tools execute, prompt, or deny based on Agent mode and project permission rules.

It is recommended to exit AIO and back up the entire app data directory before making configuration changes. Do not edit SQLite directly while the app is running, and do not treat configuration backups as free of sensitive information.
