# AIO (All-In-One AI) — Agent Coding Guide

## Project Overview

AIO is a lightweight, cross-platform AI assistant client built with **Tauri 2.x** (Rust backend) and **SolidJS** (frontend). It supports both remote API providers (OpenAI-compatible) and local inference engines (llama.cpp, with vLLM planned).

## Tech Stack & Versions

| Layer | Technology | Version |
|-------|-----------|---------|
| Frontend Framework | SolidJS | ^1.9.3 |
| Routing | @solidjs/router | ^0.15.3 |
| Styling | Tailwind CSS | ^3.4.19 |
| Build Tool | Vite | ^6.0.3 |
| Desktop Framework | Tauri | 2.10.1 |
| Backend Language | Rust | 1.75+ |
| Database | SQLite (rusqlite) | bundled |
| State Management | SolidJS Store + Signals | built-in |

## Directory Structure

### Frontend (`src/`)

```
src/
├── core/
│   ├── types/           # Unified TypeScript interfaces (Message, Topic, Assistant, etc.)
│   ├── utils/           # Pure utility functions (modelLogo, apiUrl helpers)
│   └── store/           # Global reactive state (SolidJS Store + Signals)
├── features/
│   ├── chat/
│   │   ├── components/  # ChatInterface, AssistantSidebar, TopicSidebar
│   │   └── ChatPage.tsx # Main chat page
│   └── settings/
│       ├── components/  # ProviderSettings, AccountSettings, AppSettings
│       └── SettingsPage.tsx
├── shared/
│   └── components/      # Reusable UI components (Icon, Markdown, Modal, NavBar, etc.)
├── app/
│   ├── index.tsx        # Application entry point
│   ├── Layout.tsx       # Root layout with NavBar
│   └── router.ts        # Route configuration
└── index.css            # Global styles + Tailwind directives
```

### Backend (`src-tauri/src/`)

```
src-tauri/src/
├── main.rs
├── lib.rs               # Application bootstrap, state management, invoke handler registration
├── core/
│   ├── mod.rs
│   ├── models.rs        # Shared data structures (ActivatedModel, Message, etc.)
│   ├── db.rs            # SQLite initialization and schema
│   └── state.rs         # Global Tauri state definitions (StreamManager, DbState, LocalEngineState)
├── plugins/
│   ├── mod.rs           # Plugin system registry
│   └── engine/
│       ├── mod.rs       # LocalEnginePlugin trait + EngineManager
│       ├── llama_cpp.rs # Llama.cpp plugin implementation
│       └── vllm.rs      # vLLM plugin placeholder
├── commands/
│   ├── mod.rs
│   ├── llm.rs           # LLM streaming, model fetching, summarization
│   ├── config.rs        # App config, assistant/model persistence
│   ├── auth.rs          # Backend authentication
│   └── engine.rs        # Local engine management commands (replaces llama_server.rs)
└── utils/
    ├── mod.rs
    └── file_parser.rs   # File content extraction (PDF, Office, images, text)
```

## Coding Conventions

### Rust

- Use `snake_case` for functions, variables, modules, and file names.
- Use `PascalCase` for structs, enums, and traits.
- Error handling: prefer `map_err(|e| e.to_string())?` in command functions. Avoid panicking in user-facing paths.
- All `#[tauri::command]` functions MUST have a doc comment describing params and behavior.
- Keep commands thin; delegate business logic to modules in `plugins/` or `utils/`.
- When adding a new Tauri command:
  1. Implement in the appropriate `commands/*.rs` file.
  2. Export it in `commands/mod.rs`.
  3. Register it in `lib.rs` via `tauri::generate_handler![...]`.

### SolidJS / TypeScript

- Components: `PascalCase` files and export names.
- Signals: `camelCase`, use `createSignal` for local component state.
- Store updates: use functional path syntax (`setDatas('assistants', a => a.id === id, 'name', value)`).
- Avoid `any` in new code. Define interfaces in `core/types/`.
- Utility functions go to `core/utils/`, never duplicate logic across components.

### Naming & Strings

- **Do NOT hardcode engine names** (e.g., `"Local-Llama.cpp"`) anywhere. Use `model.engine_type` or URL-based heuristics (`isLocalUrl`).
- Frontend event names should be descriptive and kebab-case (e.g., `engine-progress`, `llm-chunk`).

## Extensibility Guide

### Adding a New API Provider

1. **Backend**: No structural changes needed. The `llm.rs` commands already support any OpenAI-compatible API.
2. **Frontend**: Add the provider's logo mapping in `core/utils/modelLogo.ts`.
3. **UI**: If the provider requires special fields, extend `ProviderSettings.tsx` accordingly.

### Adding a New Local Inference Engine

Follow this 4-step process to achieve "plug-and-play" integration:

1. **Implement the Trait**: Create a new file in `plugins/engine/` (e.g., `plugins/engine/olama.rs`). Implement the `LocalEnginePlugin` trait:
   ```rust
   #[async_trait]
   impl LocalEnginePlugin for OllamaPlugin {
       fn name(&self) -> &'static str { "Ollama" }
       fn identifier(&self) -> &'static str { "ollama" }
       fn supported_extensions(&self) -> &[&'static str] { &["gguf"] }
       async fn start(&self, app: AppHandle, model_path: &str, port: u16, gpu_layers: i32) -> Result<String, String> { ... }
       fn build_command(&self, ...) -> std::process::Command { ... }
       fn parse_progress_from_log(&self, line: &str) -> Option<f64> { ... }
   }
   ```

2. **Register the Plugin**: In `plugins/engine/mod.rs`, add the plugin to `EngineManager::new()`:
   ```rust
   mgr.register(Box::new(OllamaPlugin));
   ```

3. **Place Binaries**: Put the engine's runtime files in `src-tauri/resources/engines/<identifier>/` (e.g., `resources/engines/ollama/`).

4. **Update Resources**: Ensure `tauri.conf.json`'s `bundle.resources` includes the new directory:
   ```json
   "resources": ["resources/engines/**/*"]
   ```

The frontend will automatically show the new engine in the local model list via the `engine_type` field on `ActivatedModel`.

### Adding a New File Parser

1. Open `utils/file_parser.rs`.
2. Add a new match arm in `process_file_content` for the file extension.
3. If complex parsing is needed, create a private helper function in the same file.

## State Management Rules

### Global State (`core/store/`)

- `datas` (Store): Holds `assistants`, `activatedModels`, `user`, `isLoggedIn`.
- `config` (Signal): Holds `apiUrl`, `apiKey`.
- `selectedModel` (Signal): Currently selected `ActivatedModel`.
- `currentAssistantId` / `currentTopicId` (Signals): Navigation state.

### Backend State (`core/state.rs`)

- `StreamManager`: Manages active LLM stream tasks keyed by `"{assistant_id}-{topic_id}"`.
- `DbState`: Wraps the SQLite connection.
- `LocalEngineState`: Wraps the currently running local engine process and its type identifier.

## Important Compatibility Notes

- **Command Stability**: Public Tauri command names (`start_local_server`, `stop_local_server`, `is_local_server_running`, `call_llm_stream`, etc.) must remain stable to preserve frontend compatibility.
- **Config Migration**: The `ActivatedModel` struct supports an optional `engine_type` field. Old configs without this field deserialize safely (defaults to `None`), and logic treats `None` as legacy llama.cpp behavior.
- **Event Names**: The `llama-progress` event is kept for backward compatibility. New generic code should prefer `engine-progress` where applicable.

## Build & Dev Commands

```bash
# Install dependencies
npm install

# Dev mode (frontend + Tauri)
npm run tauri dev

# Production build
npm run tauri build

# Check Rust code
cd src-tauri && cargo check
```

## Review Checklist for Agents

Before submitting changes, verify:
- [ ] New Rust modules are declared in their parent `mod.rs`.
- [ ] New Tauri commands are registered in `lib.rs`.
- [ ] Frontend imports use the new `core/` and `features/` path conventions.
- [ ] No hardcoded engine/provider names in logic; use constants or `engine_type` fields.
- [ ] `cargo check` passes without warnings.
- [ ] `npm run build` completes successfully.
