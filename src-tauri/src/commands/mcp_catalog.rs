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
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    std::fs::write(
        path,
        serde_json::to_string(cache).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())
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

fn parse_argument_values(value: Option<&Value>) -> Vec<String> {
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
        .collect()
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
        if url.starts_with("http://") || url.starts_with("https://") {
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
        if identifier.is_empty() {
            continue;
        }
        let package_version = {
            let value = text(package, &["version"]);
            if value.is_empty() {
                version.clone()
            } else {
                value
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
            return Err(format!("请求 MCP Registry 失败: {}", error));
        }
    };
    if !result.status().is_success() {
        return Err(format!("MCP Registry 返回 HTTP {}", result.status()));
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

/// Check whether the runtime required by a catalog package is available on PATH.
#[tauri::command]
pub async fn check_mcp_catalog_runtime(package_type: String) -> Result<bool, String> {
    let command = match package_type.as_str() {
        "npm" => "npx",
        "pypi" => "uvx",
        "http" => return Ok(true),
        _ => return Ok(false),
    };
    #[cfg(windows)]
    let status = tokio::process::Command::new("cmd")
        .args(["/C", command, "--version"])
        .kill_on_drop(true)
        .status()
        .await;
    #[cfg(not(windows))]
    let status = tokio::process::Command::new(command)
        .arg("--version")
        .kill_on_drop(true)
        .status()
        .await;
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
        .ok_or_else(|| "未找到选择的安装方式".to_string())?;
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
            return Err(format!("缺少必填配置: {}", input.name));
        }
    }

    let mut env = BTreeMap::new();
    let mut headers = BTreeMap::new();
    for input in &delivery.inputs {
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
            if !(delivery.url.starts_with("https://") || delivery.url.starts_with("http://")) {
                return Err("Registry 返回了非法的 HTTP 地址".into());
            }
            McpTransport::Http {
                url: delivery.url.clone(),
                headers,
            }
        }
        "npm" | "pypi" => McpTransport::Stdio {
            command: if delivery.kind == "npm" {
                "npx".into()
            } else {
                "uvx".into()
            },
            args: delivery.args.clone(),
            env,
            cwd: None,
        },
        _ => return Err("AIO 暂不支持此安装方式".into()),
    };
    let config = McpServerConfig {
        id: server_id,
        display_name: request.server.display_name.clone(),
        transport,
        enabled_tools: Vec::new(),
        auto_start: true,
        has_stored_secret: !request.secrets.is_empty(),
        from_catalog: Some(CatalogRef {
            catalog_id: "official-mcp-registry".into(),
            source_id: request.server.name,
            version: (!request.server.version.is_empty()).then_some(request.server.version),
            delivery: Some(delivery.kind.clone()),
        }),
    };
    mcp::upsert_config(&app, config.clone()).map_err(|error| error.to_string())?;
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
}
