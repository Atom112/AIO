# Extension Development

## Adding an OpenAI-compatible Provider

Usually no Rust changes are needed. If the model service is compatible with OpenAI `/models` and `/chat/completions`:

1. Add Provider and model metadata in `aio-models-data`.
2. Update the adjacent directory content that AIO uses.
3. Add an icon mapping in `src/core/utils/modelLogo.ts` if necessary.
4. Verify using the custom URL and test connection on the settings page.

Only add a dedicated Provider plugin when the request or streaming response format differs from the existing implementation. Dedicated plugins must be registered before `openai_compat`.

## Adding a Provider Plugin

1. Add a module in `src-tauri/src/plugins/provider/`.
2. Implement the existing Provider trait, including matching conditions, model URL, request construction and stream parsing.
3. Declare the new module in the parent module.
4. Register in `ProviderManager::new()`, keeping OpenAI-compatible last.
5. Add minimal tests for request mapping and stream events.

## Adding a Local Engine

1. Add an implementation in `src-tauri/src/plugins/engine/`.
2. Implement `LocalEnginePlugin` for identification, platform, extensions, install path, startup and progress parsing.
3. Register in `EngineManager::new()`.
4. For auto-install, integrate with the existing `EngineInstaller`.
5. Explicitly expose in the frontend engine options, with format filtering and necessary safety notes.

After backend registration, the engine is automatically discovered by `EngineManager` and included in scan results. vLLM is the current example: once the plugin is registered, it is covered by the engine scanning pipeline without a separate frontend settings page. All three engines (llama.cpp, vLLM, Ollama) now share a unified engine card UI, with status indicators and start/stop operations handled in the Provider details page.

## Adding an MCP Transport

1. Add a module in `src-tauri/src/plugins/mcp/`.
2. Implement `McpServerPlugin` for start, initialize, tools, resources, prompts, call and stop behavior.
3. Register in `McpServerManager::builtin()`.
4. Extend the `McpTransport` data type and its frontend mirror type.
5. Update the settings form and Catalog delivery mapping.

Use the existing JSON-RPC connection; do not replicate request correlation, capability negotiation and error handling for a new transport.

## Adding a File Parser

1. Add the extension to the allowlist match in `src-tauri/src/utils/file_parser.rs`.
2. Implement the parsing function in the same module, reusing size, extension and sandbox checks.
3. Update the frontend selection allowlist in `ChatPage.tsx`.
4. Keep a minimal test for valid files, oversized files and error content.
5. Update the [configuration reference](../reference/configuration.md#attachment-whitelist).

Do not only modify the file picker; the Rust allowlist is the trust boundary.

## Adding a Tauri Command

1. Place it in the corresponding `commands` module and keep commands thin.
2. Add a Rust doc comment describing the parameters and behavior.
3. Export in the parent module.
4. Register in `generate_handler!` in `src-tauri/src/lib.rs`.
5. Call from the frontend with typed parameters; avoid adding `any`.

## Verification

```bash
npm run build
cd src-tauri
cargo check
```

When engine, MCP, file or system credential store changes are involved, also use `npm run tauri dev` for real desktop flow verification.

Minimal tests should target the new behavior:

- Provider: request mapping, URL matching and stream event parsing;
- Engine: platform support, extensions, install status and progress parsing;
- MCP Transport: initialize, request correlation, capabilities and disconnect cleanup;
- File parser: valid files, oversized, error content and paths outside sandbox;
- Permission or tool: at minimum cover allow, deny and critical failure paths.

Prefer adding a reproducible test case to an existing module test rather than setting up a parallel test framework for a single implementation.

## Documentation Sync

After an extension enters the UI, the same change should check:

- Whether `README.md` needs updated feature boundaries;
- Whether the user guide needs added configuration steps;
- Whether `reference/configuration.md` needs new fields, files or security notes;
- Whether `troubleshooting.md` needs actionable failure handling;
- Whether `CHANGELOG.md` needs a release change entry.

When only the backend plugin is registered without a frontend entry point, mark it as an internal capability in the development documentation rather than writing it as a workflow an ordinary user can complete.
