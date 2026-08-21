# Troubleshooting

## Provider Cannot Connect

1. Re-run the connection test in "Settings &rarr; Provider Settings".
2. Check if the API URL includes an extraneous `/chat/completions`.
3. Confirm the API Key has permission for the target model.
4. Confirm the service response format is Google, Anthropic, Ollama, or OpenAI-compatible.
5. For self-hosted services on an internal network, check if the URL is blocked by SSRF policy.

If the model catalog shows models but requests fail, catalog metadata does not mean your account has access to that model.

## Provider Proxy Not Working

- The proxy URL only applies to the current Provider; it does not proxy MCP, update checks, or the local engine.
- Use a complete HTTP/HTTPS URL, e.g. `http://127.0.0.1:7890`.
- Confirm the proxy process is listening on the expected address and allows AIO to connect.
- Temporarily clear the proxy URL and re-test to isolate proxy vs. API configuration issues.
- Do not write passwords directly into the URL; this field is not a credential store entry.

Catalog refresh and chat requests use different services. Success of the former does not mean the Provider proxy is correctly configured.

## Local llama.cpp Fails to Start

- Confirm you have selected a readable `.gguf` file.
- The first launch requires downloading the engine over the network; check your firewall and proxy.
- Confirm port `8080` is not occupied by another program.
- If the model is too large, reduce model size or close other memory/GPU-intensive programs.
- After moving the model file, re-select the path and add the model again.

For engine scanning and auto-discovery, see [providers-and-models.md &rarr; Engine Scanning and Auto-Discovery](usage/providers-and-models.md#engine-scanning-and-auto-discovery). vLLM and other local engines are automatically registered through engine scanning; start/stop operations are managed uniformly in the engine card on the Provider details page.

## MCP Server Fails to Start

1. Check the server status and error information.
2. stdio Server: Confirm the command is available in the system PATH, and that arguments and working directory are valid.
3. NPX Server: Confirm Node.js/npm is installed and the network can reach the package source.
4. HTTP / Streamable HTTP Server: Confirm the transport type, URL, request headers, and server capabilities.
5. When installing from the Catalog, first run the runtime check and fill in required keys.
6. Project Server: Confirm the correct project is currently selected.

If the server connects but tools are not visible, check whether the current assistant/project has that Server bound and the `enabledTools` allowlist.

## MCP Tool Keeps Waiting for Confirmation

- Normal Agent mode requests confirmation for sensitive operations.
- Check for any unhandled approval dialogs.
- Check if `.aio/permissions.json` has the tool configured as `ask` or `deny`.
- Auto mode does not override explicit deny rules and dangerous command protection.

## Skill Not Taking Effect

1. Confirm the Skill has been downloaded or imported.
2. Confirm whether the scope is global or project.
3. Enable the Skill in the assistant or project settings.
4. After switching projects, wait for the Skill list to reload.
5. If an NPX Skill fails to update, check the package manager and network.

## Attachment Fails to Upload

Check the extension and size:

- Images: up to 10 MiB;
- PDF, DOCX, PPTX: up to 30 MiB;
- Text files: up to 5 MiB.

If a PDF or Office file uploads but yields no usable text, the file may be a scanned document, encrypted, or contain a structure not supported by the current parser.

## Agent Cannot Access Project Files

- Confirm that when creating the project, you selected a directory, not a file.
- Confirm the project path still exists.
- Check whether the current assistant belongs to the project.
- Plan mode does not modify files.
- Both file tools and the built-in filesystem MCP are restricted to the project root.
- Check the Problems panel for LSP not installed warnings; LSP failure does not affect normal file reads.

## Usage Statistics Empty

Usage data is automatically written to the `usage_log` table after each LLM call completes. The following situations may result in empty or incomplete statistics:

- First use: Fresh installation or just started using AIO; records will only appear after a few conversation rounds.
- Old messages: Messages from earlier versions may lack token fields and will not be counted.
- Cancelled requests: Cancelled requests or requests where the Provider returned no usage data are recorded as zero.
- Automatic recording: The `usage_log` table is written after each successful call; no manual enabling is required.

Setting path: Settings Center &rarr; Usage. See [Chat and Agent &rarr; Usage Statistics](usage/chat-and-agent.md#usage-statistics) for details.

## Shortcuts Not Working or Conflicting

- When the input field is focused, most global shortcuts do not trigger; `Esc` and send-related keys are exceptions.
- Confirm current bindings in "Settings &rarr; App Settings &rarr; Shortcut Settings".
- If a key combination is already occupied, AIO will ask for confirmation before overriding; canceling preserves the old binding.
- Click the individual reset button, or use "Restore Defaults" to clear all custom bindings.
- macOS displays symbols differently from Windows/Linux text, but the binding logic is the same.

If the Command Palette shows an action but it does not execute, first navigate to the corresponding page; some handlers are only available when the chat page is mounted.

## Auto Update Fails

- "The current Release does not have an auto-update service configured" means that version lacks a `latest.json` or signed artifacts, not that the local installation is corrupted.
- On network errors, confirm you can access GitHub Releases; Provider proxy does not apply to update requests.
- Only officially signed Release artifacts can be installed via the built-in updater.
- Local source builds run normally but cannot replace official update package testing for signed installation.

If the download completes but installation fails, keep the current version and manually download the install package for your platform from the Releases page.

## System Auto-Start Settings Fail

- If setting fails, the toggle will automatically revert; this does not mean registration succeeded.
- Confirm the current user has permission to create user-level startup items.
- Linux desktop environments may use different auto-start directories or policies.
- macOS or Windows system management policies may prevent the app from self-registering.

System auto-start only launches AIO and will not bypass the local engine's first-start confirmation.

## Project Memory Not Taking Effect

1. In the project settings dialog, confirm that "Project Memory" (semantic RAG) is enabled.
2. Confirm the current assistant is bound to a project, not a normal chat assistant.
3. Check that `.aio/memory/memory.sqlite` exists and that the memory panel shows entries.
4. Have the Agent explicitly remember a fact, then retrieve it in a new topic.

A legacy `.aio/knowledge.json` is auto-migrated the first time memory is opened; the old file is kept afterwards and can be deleted manually.

## Sub-Agent Waiting or Partial Failure

- Batch delegation allows 1-10 subtasks; by default a maximum of 5 run concurrently, with the rest queued.
- In non-auto mode, check if there is an unhandled batch approval dialog.
- Sub-agent model overrides only list cloud models; local models are used by inheriting from the main Agent.
- Failure of one subtask does not cancel other completed results; check the error in the corresponding step card.
- If a custom role lacks the required tools, adjust its allow/deny list or switch to a suitable built-in role.

## Diff or Undo Unavailable

- Diff only records changes captured by AIO file tools; modifications made in external editors are not automatically attributed to a particular response round.
- Full undo requires the project to be a Git repository with `HEAD` present.
- Undo restores files using `git checkout HEAD -- <file>`, which also discards any uncommitted changes in that file.
- After deleting, moving, or switching branches, old file paths in historical messages may no longer be recoverable.

Before performing an undo, use `git status` and Diff to confirm the target; important changes should be committed or backed up first.

## Credential Storage Failed

- When the system credential store is unavailable, AIO falls back to the encrypted `secure-store.json`.
- Confirm the app data directory is writable and that security software is not blocking credential store access.
- Do not upload `secure-store.json`, full Provider configurations, or MCP configurations to an Issue.
- After copying configuration from another device, re-enter API keys and MCP secrets through the settings page.

If an old configuration contains a plaintext `apiKey`, rotate the key and re-save through the current settings page.

## Source Dependency Installation Failed

If `npm ci` reports that `@aio/models-data` does not exist, confirm the directory structure:

```text
parent/
├── AIO/
└── aio-models-data/
    └── dist/data/models.json
```

Both repositories must be in the same parent directory, and the model data repository must contain the built `dist/data/models.json`.

## Rust Build or Tauri Build Failed

- Use Rust **stable** toolchain and Node.js 20.
- Install Tauri v2 prerequisites for your platform.
- For Linux, install WebKit2GTK, AppIndicator3, and related dependencies.
- Run `npm run build` first, then run `cargo check` in `src-tauri`.
- Signature or updater artifact errors only affect official release builds and do not affect normal development builds.

If the issue persists, provide your system version, AIO commit hash, reproduction steps, and sanitized error logs on [GitHub Issues](https://github.com/Atom112/AIO/issues).
