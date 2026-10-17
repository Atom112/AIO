//! Official MCP Registry browsing and installation.

use crate::core::models::{
    CatalogRef, McpCatalogDelivery, McpCatalogInput, McpCatalogInstallRequest, McpCatalogPage,
    McpCatalogServer, McpServerConfig, McpTransport,
};
use crate::core::secure_store;
use crate::plugins::mcp;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};

use crate::ierr;

const REGISTRY_URL: &str = "https://registry.modelcontextprotocol.io/v0.1/servers";
const CACHE_FILE: &str = "mcp-registry-cache.json";
const CACHE_TTL_SECS: u64 = 6 * 60 * 60;

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct CacheFile {
    entries: HashMap<String, CacheEntry>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct CacheEntry {
    fetched_at: u64,
    body: String,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or(0)
}

fn cache_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_cache_dir()
        .map(|path| path.join(CACHE_FILE))
        .map_err(|error| error.to_string())
}

fn load_cache(app: &AppHandle) -> CacheFile {
    cache_path(app)
        .ok()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|body| serde_json::from_str(&body).ok())
        .unwrap_or_default()
}

fn save_cache(app: &AppHandle, cache: &CacheFile) -> Result<(), String> {
    let path = cache_path(app)?;
    let body = serde_json::to_string(cache).map_err(|error| error.to_string())?;
    crate::core::fs_util::atomic_write(&path, body.as_bytes()).map_err(|error| error.to_string())
}

fn text(value: &Value, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .unwrap_or_default()
        .to_string()
}

fn bool_value(value: &Value, keys: &[&str]) -> bool {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_bool))
        .unwrap_or(false)
}

fn parse_inputs(value: &Value, target: &str) -> Vec<McpCatalogInput> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let name = text(item, &["name", "key"]);
            if name.is_empty() {
                return None;
            }
            // MCP-14: registry-controlled names must be strictly shaped. A catalog
            // entry declaring env names like NODE_OPTIONS / npm_config_* would let its
            // own defaultValue inject loader/config settings into the npx/uvx process;
            // header names outside the HTTP token grammar are dropped likewise.
            if target == "env" && !is_safe_env_name(&name) {
                tracing::warn!("[mcp-catalog] 丢弃非法 env 名称: {name}");
                return None;
            }
            if target == "header" && !is_safe_header_name(&name) {
                tracing::warn!("[mcp-catalog] 丢弃非法 header 名称: {name}");
                return None;
            }
            Some(McpCatalogInput {
                name,
                description: text(item, &["description"]),
                required: bool_value(item, &["isRequired", "required"]),
                secret: bool_value(item, &["isSecret", "secret", "isSensitive"]),
                default_value: text(item, &["default", "defaultValue"]),
                target: target.to_string(),
            })
        })
        .collect()
}

/// Env names that turn a registry default value into code execution or package-manager
/// config injection when passed to the npx/uvx child (MCP-14).
const DANGEROUS_ENV_NAMES: &[&str] = &[
    "NODE_OPTIONS",
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    "DYLD_INSERT_LIBRARIES",
    "DYLD_LIBRARY_PATH",
    "PYTHONHOME",
    "PYTHONSTARTUP",
    "BASH_ENV",
    "ENV",
];
/// Env-name prefixes that reconfigure the package manager itself (MCP-14).
const DANGEROUS_ENV_PREFIXES: &[&str] = &["npm_config_", "pip_", "uv_"];

/// MCP-14: POSIX env-name grammar plus the loader/config denylist.
fn is_safe_env_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 64 {
        return false;
    }
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return false;
    }
    !DANGEROUS_ENV_NAMES
        .iter()
        .any(|d| d.eq_ignore_ascii_case(name))
        && !DANGEROUS_ENV_PREFIXES
            .iter()
            .any(|p| name.to_ascii_lowercase().starts_with(p))
}

/// MCP-14: HTTP header token grammar (RFC 7230 tchar minus separators of concern).
fn is_safe_header_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

/// MCP-14: npm (@scope/name) / PyPI name grammar — anything that could read as a
/// flag (leading '-') or carry shell/flag separators is rejected outright, so a
/// hostile registry entry cannot smuggle options into the npx/uvx argv.
fn is_safe_package_identifier(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('-')
        && s.len() <= 214
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'/' | b'@'))
}

/// MCP-14: version grammar for `pkg@ver` / `pkg==ver` — no leading '-', no spaces or
/// metacharacters (a version field of `1.0 --flag` must never reach the argv).
fn is_safe_package_version(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('-')
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'+'))
}

/// MCP-14: conservative shape for registry-provided runtime/package arguments —
/// bare values or long flags only. Short flags (`-e`, `-p`, …) are the classic
/// npx/uvx injection vector; whitespace/quotes/metachars/backslashes are all rejected.
fn is_safe_catalog_arg(s: &str) -> bool {
    if s.is_empty() || s.len() > 128 {
        return false;
    }
    if s.starts_with('-') && !s.starts_with("--") {
        return false;
    }
    if s.contains("..") {
        return false;
    }
    s.bytes().all(|b| {
        b.is_ascii_alphanumeric()
            || matches!(
                b,
                b'.' | b'_' | b'-' | b'=' | b':' | b'/' | b'+' | b'@' | b','
            )
    })
}

/// Filters registry-provided argument values down to the safe shape (MCP-14),
/// dropping everything else. The drops are EXPECTED for the official registry
/// (plenty of legitimate entries carry npx `-y`, uvx `-p`, `pkg[extra]==ver`
/// specs or URL template placeholders), and AIO self-supplies the essentials
/// (npm delivery always gets its own `-y`), so a per-arg warn floods hundreds
/// of lines per catalog fetch — debug keeps the trail without the noise.
pub(crate) fn filter_catalog_args(args: Vec<String>) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    for arg in args {
        if is_safe_catalog_arg(&arg) {
            out.push(arg);
        } else {
            tracing::debug!("[mcp-catalog] 丢弃非法 runtime 参数: {arg}");
        }
    }
    out
}

/// MCP-14: plaintext-HTTP remotes only for loopback hosts — an `http://` public
/// remote would carry auth headers and all tool traffic in cleartext.
fn is_catalog_remote_url_allowed(url: &str) -> bool {
    if url.starts_with("https://") {
        return true;
    }
    if let Some(rest) = url.strip_prefix("http://") {
        let host = rest
            .split(['/', ':'])
            .next()
            .unwrap_or("")
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_ascii_lowercase();
        return matches!(host.as_str(), "localhost" | "127.0.0.1" | "::1");
    }
    false
}

fn parse_argument_values(value: Option<&Value>) -> Vec<String> {
    filter_catalog_args(
        value
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|item| {
                item.as_str().map(str::to_string).or_else(|| {
                    let value = text(item, &["value", "default"]);
                    (!value.is_empty()).then_some(value)
                })
            })
            .collect(),
    )
}

fn parse_server(raw: &Value) -> Option<McpCatalogServer> {
    let server = raw.get("server").unwrap_or(raw);
    let name = text(server, &["name"]);
    if name.is_empty() {
        return None;
    }
    let display_name = {
        let value = text(server, &["title", "displayName"]);
        if value.is_empty() {
            name.clone()
        } else {
            value
        }
    };
    let version = text(server, &["version"]);
    let repository_url = server
        .get("repository")
        .map(|repository| text(repository, &["url"]))
        .unwrap_or_default();
    let website_url = text(server, &["websiteUrl", "homepage"]);
    let mut deliveries = Vec::new();

    for remote in server
        .get("remotes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let url = text(remote, &["url"]);
        // MCP-14: plaintext HTTP only for loopback remotes — auth headers and tool
        // traffic to a public host over http:// would be readable by anyone in path.
        if is_catalog_remote_url_allowed(&url) {
            let kind = text(remote, &["type"]);
            let mut inputs = parse_inputs(remote.get("headers").unwrap_or(&Value::Null), "header");
            inputs.extend(parse_inputs(
                remote.get("environmentVariables").unwrap_or(&Value::Null),
                "env",
            ));
            deliveries.push(McpCatalogDelivery {
                id: format!("remote-{}", deliveries.len()),
                kind: "http".into(),
                label: if kind.is_empty() { "HTTP".into() } else { kind },
                command: String::new(),
                args: Vec::new(),
                url,
                inputs,
            });
        }
    }

    for package in server
        .get("packages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let registry_type = text(package, &["registryType", "registry"]).to_lowercase();
        if registry_type != "npm" && registry_type != "pypi" {
            continue;
        }
        let identifier = text(package, &["identifier", "name"]);
        if !is_safe_package_identifier(&identifier) {
            // MCP-14: a hostile entry could otherwise smuggle flags into npx/uvx argv
            tracing::warn!("[mcp-catalog] 丢弃非法包标识: {identifier}");
            continue;
        }
        let package_version = {
            let value = text(package, &["version"]);
            let candidate = if value.is_empty() {
                version.clone()
            } else {
                value
            };
            // MCP-14: a version field of "1.0 --flag" must never reach the argv;
            // unsafe versions fall back to "latest" semantics instead.
            if is_safe_package_version(&candidate) {
                candidate
            } else {
                String::new()
            }
        };
        let mut args = if registry_type == "npm" {
            vec![
                "-y".into(),
                if package_version.is_empty() {
                    identifier.clone()
                } else {
                    format!("{}@{}", identifier, package_version)
                },
            ]
        } else {
            vec![if package_version.is_empty() {
                identifier.clone()
            } else {
                format!("{}=={}", identifier, package_version)
            }]
        };
        args.extend(parse_argument_values(package.get("runtimeArguments")));
        args.extend(parse_argument_values(package.get("packageArguments")));
        deliveries.push(McpCatalogDelivery {
            id: format!("{}-{}", registry_type, deliveries.len()),
            kind: registry_type.clone(),
            label: registry_type.to_uppercase(),
            command: if registry_type == "npm" {
                "npx".into()
            } else {
                "uvx".into()
            },
            args,
            url: String::new(),
            inputs: parse_inputs(
                package.get("environmentVariables").unwrap_or(&Value::Null),
                "env",
            ),
        });
    }

    if deliveries.is_empty() {
        return None;
    }
    Some(McpCatalogServer {
        id: name.clone(),
        name,
        display_name,
        description: text(server, &["description"]),
        version,
        repository_url,
        website_url,
        deliveries,
    })
}

fn parse_page(body: &str) -> Result<McpCatalogPage, String> {
    let value: Value = serde_json::from_str(body).map_err(|error| error.to_string())?;
    let servers = value
        .get("servers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(parse_server)
        .collect();
    let next_cursor = value
        .pointer("/metadata/nextCursor")
        .or_else(|| value.pointer("/_meta/nextCursor"))
        .or_else(|| value.get("nextCursor"))
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok(McpCatalogPage {
        servers,
        next_cursor,
    })
}

async fn fetch_page(
    app: &AppHandle,
    search: &str,
    cursor: Option<&str>,
    limit: usize,
    force_refresh: bool,
) -> Result<String, String> {
    let mut url = url::Url::parse(REGISTRY_URL).map_err(|error| error.to_string())?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("limit", &limit.clamp(1, 100).to_string());
        query.append_pair("version", "latest");
        if !search.trim().is_empty() {
            query.append_pair("search", search.trim());
        }
        if let Some(cursor) = cursor.filter(|value| !value.is_empty()) {
            query.append_pair("cursor", cursor);
        }
    }
    let key = url.to_string();
    let mut cache = load_cache(app);
    if !force_refresh {
        if let Some(entry) = cache.entries.get(&key) {
            if now().saturating_sub(entry.fetched_at) < CACHE_TTL_SECS {
                return Ok(entry.body.clone());
            }
        }
    }
    let response = reqwest::Client::builder()
        .user_agent("AIO MCP Registry/0.4")
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(25))
        .build()
        .map_err(|error| error.to_string())?
        .get(url)
        .send()
        .await;
    let result = match response {
        Ok(result) => result,
        Err(error) => {
            if !force_refresh {
                if let Some(entry) = cache.entries.get(&key) {
                    return Ok(entry.body.clone());
                }
            }
            return Err(ierr!(
                "请求 MCP Registry 失败: {}",
                "Failed to request MCP Registry: {}",
                error
            ));
        }
    };
    if !result.status().is_success() {
        return Err(ierr!(
            "MCP Registry 返回 HTTP {}",
            "MCP Registry returned HTTP {}",
            result.status()
        ));
    }
    let body = result.text().await.map_err(|error| error.to_string())?;
    cache.entries.insert(
        key,
        CacheEntry {
            fetched_at: now(),
            body: body.clone(),
        },
    );
    let _ = save_cache(app, &cache);
    Ok(body)
}

/// Browse the official MCP Registry with search and cursor pagination.
#[tauri::command]
pub async fn list_mcp_catalog(
    app: AppHandle,
    search: String,
    cursor: Option<String>,
    limit: usize,
    force_refresh: bool,
) -> Result<McpCatalogPage, String> {
    let body = fetch_page(&app, &search, cursor.as_deref(), limit, force_refresh).await?;
    parse_page(&body)
}

// ====== Full-registry local table (real-time client-side search) ======

/// Cache key for the full parsed catalog snapshot (the body is the serialized
/// Vec<McpCatalogServer> — parsed once at fetch time, not raw page JSON).
const FULL_CACHE_KEY: &str = "__full_catalog_v1__";
/// Pages are fetched at the API's max page size.
const FULL_PAGE_SIZE: usize = 100;
/// Safety cap on the pagination loop (100 pages x 100 = 10k servers).
const FULL_MAX_PAGES: usize = 100;

/// Fetch EVERY registry page and return the parsed catalog as one table. The
/// snapshot is disk-cached with the same 6h TTL as paged fetches; the refresh
/// button is the only path that force-refetches. A mid-loop page failure falls
/// back to the stale snapshot when one exists (an older complete table beats a
/// fresh partial one for search completeness).
async fn fetch_full_catalog(
    app: &AppHandle,
    force_refresh: bool,
) -> Result<Vec<McpCatalogServer>, String> {
    let stale = |cache: &CacheFile| {
        cache
            .entries
            .get(FULL_CACHE_KEY)
            .and_then(|entry| serde_json::from_str::<Vec<McpCatalogServer>>(&entry.body).ok())
    };
    let mut cache = load_cache(app);
    if !force_refresh {
        if let Some(entry) = cache.entries.get(FULL_CACHE_KEY) {
            if now().saturating_sub(entry.fetched_at) < CACHE_TTL_SECS {
                if let Ok(list) = serde_json::from_str::<Vec<McpCatalogServer>>(&entry.body) {
                    return Ok(list);
                }
            }
        }
    }

    let client = reqwest::Client::builder()
        .user_agent("AIO MCP Registry/0.4")
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(25))
        .build()
        .map_err(|error| error.to_string())?;

    let mut all: Vec<McpCatalogServer> = Vec::new();
    let mut cursor: Option<String> = None;
    for page_idx in 0..FULL_MAX_PAGES {
        let mut url = url::Url::parse(REGISTRY_URL).map_err(|error| error.to_string())?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("limit", &FULL_PAGE_SIZE.to_string());
            query.append_pair("version", "latest");
            if let Some(c) = cursor.as_deref().filter(|c| !c.is_empty()) {
                query.append_pair("cursor", c);
            }
        }
        let page = match client.get(url).send().await {
            Ok(resp) if resp.status().is_success() => {
                let body = resp.text().await.map_err(|error| error.to_string())?;
                parse_page(&body)?
            }
            outcome => {
                tracing::warn!(
                    "[mcp-catalog] 全量拉取第 {} 页失败: {:?}",
                    page_idx + 1,
                    outcome
                );
                if let Some(list) = stale(&cache) {
                    return Ok(list);
                }
                if all.is_empty() {
                    return Err(ierr!(
                        "请求 MCP Registry 失败（第 {} 页）",
                        "Failed to request the MCP Registry (page {})",
                        page_idx + 1
                    ));
                }
                break;
            }
        };
        all.extend(page.servers);
        match page.next_cursor.filter(|c| !c.is_empty()) {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }

    tracing::info!("[mcp-catalog] 全量目录拉取完成: {} 个 server", all.len());
    cache.entries.insert(
        FULL_CACHE_KEY.to_string(),
        CacheEntry {
            fetched_at: now(),
            body: serde_json::to_string(&all).map_err(|error| error.to_string())?,
        },
    );
    let _ = save_cache(app, &cache);
    Ok(all)
}

/// Return the full registry table (disk-cached 6h; `force_refresh` re-downloads).
/// The market page filters this locally for real-time search.
#[tauri::command]
pub async fn list_mcp_catalog_all(
    app: AppHandle,
    force_refresh: bool,
) -> Result<Vec<McpCatalogServer>, String> {
    fetch_full_catalog(&app, force_refresh).await
}

/// Check whether the runtime required by a catalog package is available on PATH.
#[tauri::command]
pub async fn check_mcp_catalog_runtime(package_type: String) -> Result<bool, String> {
    let command = match package_type.as_str() {
        "npm" => "npx",
        "pypi" => "uvx",
        "http" => return Ok(true),
        _ => return Ok(false),
    };
    // SEC-06: PATH-only resolution — an unresolvable runtime simply counts as absent.
    let Some(runtime) = crate::utils::exec_resolve::resolve_executable(command) else {
        return Ok(false);
    };
    let Some(shell) =
        crate::utils::exec_resolve::resolve_executable(if cfg!(windows) { "cmd" } else { "sh" })
    else {
        return Ok(false);
    };
    // Windows batch wrappers (npx.cmd) must run through cmd /C; POSIX shebangs run via sh -c.
    // hidden_console: the GUI-subsystem release build owns no console, so a bare
    // cmd.exe child would flash a terminal window on every runtime probe.
    use crate::utils::exec_resolve::HiddenConsole;
    let status = if cfg!(windows) {
        tokio::process::Command::new(shell)
            .args(["/C", &runtime.to_string_lossy(), "--version"])
            .kill_on_drop(true)
            .hidden_console()
            .status()
            .await
    } else {
        tokio::process::Command::new(shell)
            .arg("-c")
            .arg(format!("{} --version", runtime.to_string_lossy()))
            .kill_on_drop(true)
            .hidden_console()
            .status()
            .await
    };
    Ok(status.map(|value| value.success()).unwrap_or(false))
}

fn placeholder(server_id: &str, target: &str, name: &str) -> String {
    format!("${{KEYRING:mcp-server-{}-{}-{}}}", server_id, target, name)
}

/// Convert a Registry delivery into a persisted AIO MCP configuration.
#[tauri::command]
pub async fn install_mcp_catalog_server(
    app: AppHandle,
    request: McpCatalogInstallRequest,
) -> Result<McpServerConfig, String> {
    let delivery = request
        .server
        .deliveries
        .iter()
        .find(|item| item.id == request.delivery_id)
        .ok_or_else(|| {
            ierr!(
                "未找到选择的安装方式",
                "Selected installation method not found"
            )
        })?;
    let server_id = format!(
        "registry-{}",
        request
            .server
            .name
            .chars()
            .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
            .collect::<String>()
            .trim_matches('-')
            .to_lowercase()
    );
    for input in &delivery.inputs {
        let supplied = request
            .secrets
            .get(&input.name)
            .or_else(|| request.values.get(&input.name))
            .filter(|value| !value.is_empty())
            .or_else(|| (!input.default_value.is_empty()).then_some(&input.default_value));
        if input.required && supplied.is_none() {
            return Err(ierr!(
                "缺少必填配置: {}",
                "Missing required configuration: {}",
                input.name
            ));
        }
    }

    let mut env = BTreeMap::new();
    let mut headers = BTreeMap::new();
    for input in &delivery.inputs {
        // MCP-14 defense-in-depth: the frontend normally forwards catalog-parsed
        // inputs, but a crafted install request must not be able to smuggle
        // loader/config env names (NODE_OPTIONS / npm_config_* …) into the child.
        if input.target == "header" {
            if !is_safe_header_name(&input.name) {
                tracing::warn!(
                    "[mcp-catalog] 安装请求中的非法 header 名称被拒绝: {}",
                    input.name
                );
                continue;
            }
        } else if !is_safe_env_name(&input.name) {
            tracing::warn!(
                "[mcp-catalog] 安装请求中的非法 env 名称被拒绝: {}",
                input.name
            );
            continue;
        }
        let secret_value = request
            .secrets
            .get(&input.name)
            .filter(|value| !value.is_empty());
        let plain_value = request
            .values
            .get(&input.name)
            .filter(|value| !value.is_empty())
            .cloned()
            .unwrap_or_else(|| input.default_value.clone());
        let value = if let Some(secret) = secret_value {
            let account = format!("mcp-server-{}-{}-{}", server_id, input.target, input.name);
            secure_store::set(&app, &account, secret).map_err(|error| error.to_string())?;
            placeholder(&server_id, &input.target, &input.name)
        } else {
            plain_value
        };
        if !value.is_empty() {
            if input.target == "header" {
                headers.insert(input.name.clone(), value);
            } else {
                env.insert(input.name.clone(), value);
            }
        }
    }

    let transport = match delivery.kind.as_str() {
        "http" => {
            let url = crate::utils::url_validation::validate_http_url(
                &delivery.url,
                &crate::utils::url_validation::HttpUrlOptions::local_engine(),
            )
            .map_err(|e| {
                ierr!(
                    "Registry 返回了非法的 HTTP 地址: {}",
                    "Registry returned an invalid HTTP URL: {}",
                    e
                )
            })?;
            McpTransport::Http { url, headers }
        }
        "npm" | "pypi" => McpTransport::Stdio {
            command: if delivery.kind == "npm" {
                "npx".into()
            } else {
                "uvx".into()
            },
            // MCP-14 defense-in-depth: re-filter the argv even though the parse path
            // already validated it (crafted install requests bypass the parse path).
            args: filter_catalog_args(delivery.args.clone()),
            env,
            cwd: None,
        },
        _ => {
            return Err(ierr!(
                "AIO 暂不支持此安装方式",
                "AIO does not support this installation method yet"
            ))
        }
    };
    let config = McpServerConfig {
        id: server_id,
        display_name: request.server.display_name.clone(),
        transport,
        enabled_tools: Vec::new(),
        auto_start: true,
        has_stored_secret: !request.secrets.is_empty(),
        // Catalog-installed servers are untrusted by default: tools require per-call approval in Auto (完全权限) mode, marked trusted only after user confirmation
        trusted: false,
        from_catalog: Some(CatalogRef {
            catalog_id: "official-mcp-registry".into(),
            source_id: request.server.name,
            version: (!request.server.version.is_empty()).then_some(request.server.version),
            delivery: Some(delivery.kind.clone()),
        }),
    };
    // Install scope: project-level writes ship with the repo (`.aio/mcp-servers.json`)
    // and — like add_mcp_server's project path — record trust immediately (the install
    // click through the UI is the user's authorization; the P0-1 gate targets
    // repo-shipped configs, not UI installs). Global installs stay in the app-data file.
    match &request.project_id {
        Some(pid) => {
            let project_path = mcp::resolve_project_path(&app, pid).map_err(|e| e.to_string())?;
            mcp::upsert_project_config(&project_path, config.clone()).map_err(|e| e.to_string())?;
            mcp::mark_project_server_trusted(&app, &project_path, &config);
        }
        None => {
            mcp::upsert_config(&app, config.clone()).map_err(|error| error.to_string())?;
        }
    }
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_supported_registry_deliveries() {
        let page = parse_page(
            r#"{"servers":[{"server":{"name":"io.example/demo","title":"Demo","version":"1.2.3","packages":[{"registryType":"npm","identifier":"@example/demo","environmentVariables":[{"name":"TOKEN","isRequired":true,"isSecret":true}]}],"remotes":[{"type":"streamable-http","url":"https://example.com/mcp"}]}}],"metadata":{"nextCursor":"next"}}"#,
        )
        .unwrap();
        assert_eq!(page.servers.len(), 1);
        assert_eq!(page.servers[0].deliveries.len(), 2);
        assert_eq!(page.next_cursor.as_deref(), Some("next"));
        assert_eq!(page.servers[0].deliveries[1].args[1], "@example/demo@1.2.3");
    }

    /// MCP-14: registry-controlled strings feeding the npx/uvx argv are strictly
    /// shaped — hostile identifiers/versions/args/remote URLs are dropped or degraded.
    #[test]
    fn registry_injection_vectors_are_neutralized() {
        // Package identifiers: scoped/plain ok; flag-look-alikes and metachar strings rejected.
        assert!(is_safe_package_identifier("@example/demo"));
        assert!(is_safe_package_identifier("demo-server"));
        assert!(!is_safe_package_identifier("--cache=/tmp/x"));
        assert!(!is_safe_package_identifier("demo;rm"));
        assert!(!is_safe_package_identifier(""));
        // Versions: semver-ish ok; option smuggling rejected (falls back to empty).
        assert!(is_safe_package_version("1.2.3"));
        assert!(is_safe_package_version("2.0.0-beta.1+build"));
        assert!(!is_safe_package_version("1.0 --flag"));
        assert!(!is_safe_package_version("-e"));
        // Runtime args: long flags and bare values pass; short flags, shell
        // metachars, whitespace, backslashes, traversal are all dropped.
        assert!(is_safe_catalog_arg("--env"));
        assert!(is_safe_catalog_arg("--endpoint=https://api.example.com/v1"));
        assert!(is_safe_catalog_arg("stdio"));
        assert!(!is_safe_catalog_arg("-e"));
        assert!(!is_safe_catalog_arg("a b"));
        assert!(!is_safe_catalog_arg("--x;y"));
        assert!(!is_safe_catalog_arg("--x|y"));
        assert!(!is_safe_catalog_arg("$(x)"));
        assert!(!is_safe_catalog_arg("C:\\evil"));
        assert!(!is_safe_catalog_arg("a`b`"));
        assert!(!is_safe_catalog_arg("../etc/passwd"));
        // Env names: POSIX shape, loader/config vectors denied.
        assert!(is_safe_env_name("API_TOKEN"));
        assert!(!is_safe_env_name("NODE_OPTIONS"));
        assert!(!is_safe_env_name("npm_config_cache"));
        assert!(!is_safe_env_name("PIP_CONFIG_FILE"));
        assert!(!is_safe_env_name("2FA_KEY"));
        assert!(is_safe_env_name("node_env_file"));
        // Header names: token grammar only.
        assert!(is_safe_header_name("X-Api-Key"));
        assert!(!is_safe_header_name("X Api: Key"));
        // Remote URLs: https always; http only on loopback.
        assert!(is_catalog_remote_url_allowed("https://example.com/mcp"));
        assert!(is_catalog_remote_url_allowed("http://localhost:3333/mcp"));
        assert!(is_catalog_remote_url_allowed("http://127.0.0.1/mcp"));
        assert!(!is_catalog_remote_url_allowed("http://api.example.com/mcp"));
        assert!(!is_catalog_remote_url_allowed("ftp://example.com"));
    }

    /// End-to-end parse: a hostile registry entry loses its flag-smuggling
    /// identifier/args/version entirely instead of reaching the delivery.
    #[test]
    fn hostile_registry_entry_is_sanitized() {
        let page = parse_page(
            r#"{"servers":[{"server":{"name":"io.evil/x","title":"Evil","version":"1.0 --cache=/tmp/x","packages":[
                {"registryType":"npm","identifier":"--inspect-brk","runtimeArguments":["--userconfig=C:/evil/.npmrc"],"packageArguments":["-e","ok-flag"]},
                {"registryType":"npm","identifier":"good-pkg","runtimeArguments":["-p","evil"],"packageArguments":["--stdio"]},
                {"registryType":"pypi","identifier":"another-pkg"}
            ]}}]}"#,
        )
        .unwrap();
        let deliveries = &page.servers[0].deliveries;
        let all_args: Vec<&str> = deliveries
            .iter()
            .flat_map(|d| d.args.iter().map(String::as_str))
            .collect();
        // The option-smuggling identifier never reaches the argv
        assert!(all_args.iter().all(|a| !a.contains("inspect")));
        // npm config injection is dropped
        assert!(all_args.iter().all(|a| !a.contains("userconfig")));
        // Short-flag injection is dropped (the "-y" AIO adds itself is fine)
        assert!(all_args.iter().all(|a| *a != "-p" && *a != "-e"));
        // The smuggling version falls back to bare identifiers, not "pkg@1.0 --cache"
        assert!(all_args.iter().all(|a| !a.contains("--cache")));
        // The legitimate package keeps its legitimate flag
        let good = deliveries
            .iter()
            .find(|d| d.args.contains(&"good-pkg".to_string()))
            .expect("合法包应保留");
        assert!(good.args.contains(&"--stdio".to_string()));
        // pypi delivery intact (bare identifier, no version)
        assert!(deliveries
            .iter()
            .any(|d| d.command == "uvx" && d.args.contains(&"another-pkg".to_string())));
    }
}
