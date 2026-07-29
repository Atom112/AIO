# Providers and Models

## Provider Types

AIO backend matches dedicated Providers in order, falling back to the OpenAI-compatible implementation:

| Type              | Scope                                                              |
| ----------------- | ------------------------------------------------------------------ |
| Google            | Google native generation API and its streaming response format     |
| Anthropic         | Anthropic Messages API                                             |
| Ollama            | Local or LAN Ollama service                                        |
| OpenAI-compatible | OpenAI and services compatible with `/chat/completions`, `/models` |

Provider list and model metadata come from the adjacent dependency `@aio/models-data`. Provider configuration only stores toggles, API URLs, enabled models, and other runtime information; model capabilities, context windows, and display names are provided by the catalog.

## Configuration Workflow

1. Enable a Provider in "Settings -> Provider Settings".
2. Confirm the API URL. Custom services should enter the API root path without appending `/chat/completions`.
3. If required by the network, fill in an HTTP/HTTPS proxy URL for the current Provider, e.g. `http://127.0.0.1:7890`.
4. Enter the API Key and verify with "Test Connection".
5. Enable models from the catalog or the service's returned results.

When a request fails, first check the URL, Key, and whether the service supports the selected model. AIO validates HTTP URLs and blocks addresses that do not comply with the current security policy.

Proxy settings are saved per Provider, do not modify the system proxy, and do not affect other Providers, local llama.cpp, MCP, or automatic updates. Sensitive information such as usernames and passwords should not be written directly into the proxy URL.

## Model Catalog and Online Fetching

The settings overview displays the model catalog's Provider, model count, catalog version, and last update time. Manually refreshing downloads a new `models-catalog.json` but does not automatically enable newly added models.

Provider details merge two types of models:

- Catalog models: from `@aio/models-data`, including display name, context window, release date, and capabilities;
- Online models: fetched through the Provider's model API and saved to `fetchedModels`.

For custom Providers without catalog metadata, first use "Fetch models from API"; if the service does not provide a compatible model API, models can be added manually by ID.

Model rows may display the following capability tags:

| Tag       | Meaning                                        |
| --------- | ---------------------------------------------- |
| Vision    | Supports image input                           |
| Tools     | Supports tool calls                            |
| Reasoning | Supports reasoning or native reasoning content |
| Streaming | Supports streaming response                    |
| JSON      | Supports JSON mode                             |

`preview`, `beta`, `experimental`, and `alpha` indicate upstream model status and do not imply that AIO relaxes compatibility or security checks.

## Local llama.cpp

The settings interface currently provides llama.cpp:

- Model format: GGUF;
- Default service address: `http://127.0.0.1:8080/v1`;
- First launch: automatically downloads the engine if missing;
- Process management: stops the old process before starting a new model, cleans up child processes on application exit.

Model files are selected by the user and are not copied to the application data directory. After deleting or moving the original file, the path must be re-selected.

## Engine Scanning and Auto-Discovery

The application automatically detects locally installed inference engines on the system at startup and manages their status uniformly.

| Engine    | Identifier  | Status Management                            | API URL                    |
| --------- | ----------- | -------------------------------------------- | -------------------------- |
| llama.cpp | `llama_cpp` | Install detection, start, stop, health check | `http://localhost:8080/v1` |
| vLLM      | `vllm`      | Install detection, start, stop, health check | `http://localhost:8000/v1` |
| Ollama    | `ollama`    | Install detection, start, stop, health check | `http://localhost:11434`   |

### Scanning Mechanism

The `scan_installed_engines` command iterates through registered engine plugins, returning for each:

- `supported` -- whether the current platform is supported;
- `installed` -- whether the engine binary or runtime is installed on the system;
- `version` -- the detected version number (if any);
- `default_port` / `default_api_url` -- the default connection address.

### Engine Status

Each engine has independent status management, including four states:

- **Not installed**: the engine is not detected on the system;
- **Installed**: the engine is available, waiting for the user to start it;
- **Running**: the engine process is running and health check passes;
- **Start failed**: the engine could not start normally (check the logs for the error reason).

### UI Location

The Provider details page displays an engine status indicator. Users can start/stop installed engines from the Provider page. llama.cpp, vLLM, and Ollama share a unified setup experience:

- GGUF file picker (llama.cpp / Ollama);
- Model card display;
- Start/stop buttons and process management;
- Health check (llama.cpp/vLLM use `/health`, Ollama uses `/api/version`).

The Ollama Provider UI is now unified with llama.cpp, eliminating the need for a separate Provider settings page. All three engines are managed through the engine card on the Provider details page.

## Model Selection and Override

- Each chat assistant can bind a preferred model; when unbound, the current global selection is used.
- A project can bind a model and provide working directory context for the Agent.
- Built-in and custom sub-agents can be configured with independent cloud model overrides.
- When a sub-agent has no override configured, it inherits the main Agent's model; local models also use inheritance.

See the [Configuration Reference](../reference/configuration.md) for Provider configuration file and key storage locations.
