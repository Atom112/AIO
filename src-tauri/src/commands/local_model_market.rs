//! Local GGUF model market discovery (local_model_market_*): searches HuggingFace and
//! ModelScope for GGUF repos so users can download local chat models without visiting
//! the sites in a browser.
//!
//! Source strategy (validated 2026-10):
//! - HuggingFace Hub API, queried through `hf-mirror.com` first (reachable from mainland
//!   China, 308-redirects to huggingface.co elsewhere) with `huggingface.co` as fallback.
//!   Anonymous access works for public repos; an optional token (secure_store) unlocks
//!   gated repos. List endpoint: `/api/models?filter=gguf&sort=downloads&search=<q>`.
//! - ModelScope dolphin API: `PUT /api/v1/dolphin/models` with a `Name` substring filter
//!   (the `SearchText` field does NOT filter). Entries self-report the gguf library tag.
//! - Files per repo: HF `/api/models/{repo}/tree/main` and MS
//!   `/api/v1/models/{repo}/repo/files?Revision=master` (exact byte sizes).
//!
//! Rankings follow the two sites' own defaults (download counts); results are merged and
//! deduplicated by model name, then classified by parameter size (name regex).

use crate::core::secure_store;
use crate::ierr;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tauri::AppHandle;

/// Hosts serving the HuggingFace Hub API (mirror first: works in mainland China and
/// redirects to the origin elsewhere; direct origin as fallback for mirror outages).
const HF_ENDPOINTS: [&str; 2] = ["https://hf-mirror.com", "https://huggingface.co"];

/// ModelScope dolphin model-search endpoint.
const MS_SEARCH_URL: &str = "https://modelscope.cn/api/v1/dolphin/models";

/// A merged market entry: one row per model repo (GGUF), with the mirror/matching repo
/// identifiers on each platform so the downloader can pick a source.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MarketModel {
    /// Canonical id (prefers the HF repo id, e.g. "unsloth/Qwen3-14B-GGUF")
    pub id: String,
    /// Repo name without the org (e.g. "Qwen3-14B-GGUF")
    pub name: String,
    /// Publisher/org (e.g. "unsloth")
    pub org: String,
    /// HuggingFace repo id when the model exists there
    pub hf_repo: Option<String>,
    /// ModelScope repo id ("org/name") when the model exists there
    pub ms_repo: Option<String>,
    /// Total parameter count in billions parsed from the name (None = unknown)
    pub params_b: Option<f64>,
    /// SPDX-ish license string when known
    pub license: Option<String>,
    /// Max download count across sources (site default ranking signal)
    pub downloads: u64,
    /// HuggingFace likes
    pub likes: u64,
    /// ModelScope stars
    pub stars: u64,
    /// ModelScope Chinese display name (e.g. "千问3-14B")
    pub chinese_name: Option<String>,
    /// Where this model can be downloaded from: "huggingface" and/or "modelscope"
    pub sources: Vec<String>,
}

/// Market query response: merged models ranked by downloads + sources that failed.
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MarketListResponse {
    pub models: Vec<MarketModel>,
    pub failed_sources: Vec<String>,
}

/// One downloadable GGUF file inside a repo (repo-relative path + exact byte size).
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MarketFile {
    pub name: String,
    pub size: u64,
}

/// Host hardware profile + the recommended parameter size for local GGUF models.
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct HardwareProfile {
    pub cpu_brand: String,
    pub cpu_cores: u32,
    pub total_ram_bytes: u64,
    pub gpu_name: Option<String>,
    pub gpu_vram_bytes: Option<u64>,
    /// Recommended total parameter count (billions) that fits RAM/VRAM at Q4_K_M
    /// (~0.62 byte/param) with headroom for KV cache and the app itself
    pub recommended_params_b: f64,
}

/// Encodes a repo id or repo-relative path for use inside a URL path segment list
/// (keeps `/`, `-`, `_`, `.`; percent-encodes spaces and reserved ASCII).
/// Public: the local-model downloader reuses it to build resolve URLs.
pub fn encode_repo_path(s: &str) -> String {
    use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};
    const FRAGMENT: &AsciiSet = &CONTROLS
        .add(b' ')
        .add(b'"')
        .add(b'#')
        .add(b'%')
        .add(b'?')
        .add(b'{')
        .add(b'}')
        .add(b'<')
        .add(b'>')
        .add(b'\\')
        .add(b'^')
        .add(b'`')
        .add(b'|');
    utf8_percent_encode(s, FRAGMENT).to_string()
}

/// Extracts the total parameter size in billions from a repo/model name
/// (e.g. "Qwen3-14B-GGUF" -> 14.0, "Qwen3-30B-A3B" -> 30.0 total, "gemma-3-4b" -> 4.0).
/// Version-looking numbers ("3.1-8B" -> 8) are not matched because the B suffix anchors the match.
fn params_from_name(name: &str) -> Option<f64> {
    use once_cell::sync::Lazy;
    use regex::Regex;
    static RE: Lazy<Regex> = Lazy::new(|| {
        // The regex crate has no look-ahead: the trailing (non-alphanumeric | end-of-string)
        // boundary is matched and consumed; only group 1 is used.
        Regex::new(r"(\d+(?:\.\d+)?)\s*[bB](?:[^a-zA-Z0-9]|$)").unwrap()
    });
    RE.captures(name).and_then(|c| c[1].parse::<f64>().ok())
}

/// Builds a short-lived HTTP client for metadata queries.
fn market_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .connect_timeout(Duration::from_secs(8))
        .build()
        .map_err(|e| e.to_string())
}

/// Optional HuggingFace bearer token from the secure store.
fn hf_token(app: &AppHandle) -> Option<String> {
    secure_store::get(app, &secure_store::accounts::hf_token())
        .ok()
        .flatten()
}

/// One HuggingFace list row (subset of the Hub API fields we consume).
#[derive(Deserialize, Debug)]
struct HfListRow {
    id: String,
    #[serde(default)]
    downloads: u64,
    #[serde(default)]
    likes: u64,
    #[serde(default)]
    tags: Vec<String>,
}

/// Queries the HuggingFace GGUF model list (optionally filtered by search text) through
/// the mirror-first endpoint chain. Returns None when all endpoints fail.
async fn hf_market_list(query: &str, limit: u32, token: Option<&str>) -> Option<Vec<HfListRow>> {
    let search = query.trim();
    let mut url = format!(
        "{}/api/models?filter=gguf&sort=downloads&direction=-1&limit={}",
        HF_ENDPOINTS[0],
        limit.min(100)
    );
    if !search.is_empty() {
        url.push_str(&format!("&search={}", encode_repo_path(search)));
    }
    let client = market_client().ok()?;
    let mut last_err = None;
    for base in HF_ENDPOINTS {
        let url = url.replacen(HF_ENDPOINTS[0], base, 1);
        let mut req = client.get(&url);
        if let Some(t) = token {
            req = req.bearer_auth(t);
        }
        match req.send().await {
            Ok(resp) if resp.status().is_success() => match resp.json::<Vec<HfListRow>>().await {
                Ok(rows) => return Some(rows),
                Err(e) => last_err = Some(e.to_string()),
            },
            Ok(resp) => last_err = Some(format!("HTTP {}", resp.status())),
            Err(e) => last_err = Some(e.to_string()),
        }
    }
    tracing::warn!("[local-model-market] HF list query failed: {:?}", last_err);
    None
}

/// One ModelScope dolphin row (subset of the very wide response we consume).
#[derive(Deserialize, Debug)]
struct MsListRow {
    #[serde(default)]
    path: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    downloads: u64,
    #[serde(default)]
    stars: u64,
    #[serde(default)]
    license: Option<String>,
    #[serde(default)]
    chinese_name: Option<String>,
    #[serde(default)]
    libraries: Vec<String>,
    #[serde(default)]
    tags: Vec<String>,
}

/// Queries the ModelScope model list with a `Name` substring filter (empty query falls
/// back to "gguf" so the default page shows GGUF repos). Only entries tagged gguf are
/// kept, sorted by download count descending. Returns None when the request fails.
async fn ms_market_list(query: &str, limit: u32) -> Option<Vec<MsListRow>> {
    let search = {
        let q = query.trim();
        if q.is_empty() {
            "gguf"
        } else {
            q
        }
    };
    let body = serde_json::json!({
        "PageSize": limit.min(100),
        "PageNumber": 1,
        "SortBy": "Default",
        "Name": search,
    });
    let client = market_client().ok()?;
    let resp = client.put(MS_SEARCH_URL).json(&body).send().await.ok()?;
    if !resp.status().is_success() {
        tracing::warn!("[local-model-market] MS search HTTP {}", resp.status());
        return None;
    }
    let value: serde_json::Value = resp.json().await.ok()?;
    // Data.Model.Models[] (the Data.Files field on the model-info API is stale/null)
    let rows: Vec<MsListRow> = value
        .pointer("/Data/Model/Models")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let mut gguf: Vec<MsListRow> = rows
        .into_iter()
        .filter(|r| {
            (r.libraries.iter().any(|l| l.eq_ignore_ascii_case("gguf"))
                || r.tags.iter().any(|t| t.eq_ignore_ascii_case("gguf")))
                && !r.name.is_empty()
        })
        .collect();
    gguf.sort_by_key(|r| std::cmp::Reverse(r.downloads));
    Some(gguf)
}

/// Merges HF + MS rows by lowercased model name (org-agnostic), ranked by downloads.
fn merge_market(hf: Vec<HfListRow>, ms: Vec<MsListRow>) -> Vec<MarketModel> {
    let mut merged: Vec<MarketModel> = Vec::new();
    let mut index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for row in hf {
        let (org, name) = match row.id.split_once('/') {
            Some((o, n)) => (o.to_string(), n.to_string()),
            None => (String::new(), row.id.clone()),
        };
        let license = row
            .tags
            .iter()
            .find_map(|t| t.strip_prefix("license:").map(|s| s.to_string()));
        let entry = MarketModel {
            id: row.id.clone(),
            name: name.clone(),
            org,
            hf_repo: Some(row.id),
            ms_repo: None,
            params_b: params_from_name(&name),
            license,
            downloads: row.downloads,
            likes: row.likes,
            stars: 0,
            chinese_name: None,
            sources: vec!["huggingface".into()],
        };
        index.insert(name.to_lowercase(), merged.len());
        merged.push(entry);
    }
    for row in ms {
        let repo = format!("{}/{}", row.path, row.name);
        let key = row.name.to_lowercase();
        if let Some(&i) = index.get(&key) {
            let e = &mut merged[i];
            e.ms_repo = Some(repo);
            e.stars = row.stars;
            e.chinese_name = row.chinese_name.clone();
            e.downloads = e.downloads.max(row.downloads);
            if !e.sources.iter().any(|s| s == "modelscope") {
                e.sources.push("modelscope".into());
            }
            if e.license.is_none() {
                e.license = row.license.clone();
            }
        } else {
            let entry = MarketModel {
                id: repo.clone(),
                name: row.name.clone(),
                org: row.path.clone(),
                hf_repo: None,
                ms_repo: Some(repo),
                params_b: params_from_name(&row.name),
                license: row.license.clone(),
                downloads: row.downloads,
                likes: 0,
                stars: row.stars,
                chinese_name: row.chinese_name,
                sources: vec!["modelscope".into()],
            };
            index.insert(key, merged.len());
            merged.push(entry);
        }
    }
    merged.sort_by_key(|m| std::cmp::Reverse(m.downloads));
    merged
}

/// Searches the local model market across HuggingFace and ModelScope.
///
/// - `query`: search text; empty lists the sites' default GGUF rankings (by downloads)
/// - `limit`: max rows per source (default 50, capped at 100)
///
/// Sources failing entirely are reported in `failedSources` (partial results are still
/// returned; an empty query result with both sources failed returns an error).
#[tauri::command]
pub async fn local_model_market_list(
    app: AppHandle,
    query: String,
    limit: Option<u32>,
) -> Result<MarketListResponse, String> {
    let limit = limit.unwrap_or(50).clamp(1, 100);
    let token = hf_token(&app);
    let (hf, ms) = tokio::join!(
        hf_market_list(&query, limit, token.as_deref()),
        ms_market_list(&query, limit)
    );
    let mut failed_sources = Vec::new();
    if hf.is_none() {
        failed_sources.push("huggingface".into());
    }
    if ms.is_none() {
        failed_sources.push("modelscope".into());
    }
    if failed_sources.len() == 2 {
        return Err(ierr!(
            "模型市场暂不可用：HuggingFace 与 ModelScope 均无法访问，请检查网络",
            "Model market unavailable: both HuggingFace and ModelScope are unreachable, check your network"
        ));
    }
    let models = merge_market(hf.unwrap_or_default(), ms.unwrap_or_default());
    Ok(MarketListResponse {
        models,
        failed_sources,
    })
}

/// Lists the GGUF files of one repo with exact byte sizes.
///
/// - `source`: "huggingface" or "modelscope"
/// - `repo`: the platform's repo id ("org/name")
#[tauri::command]
pub async fn local_model_market_files(
    app: AppHandle,
    source: String,
    repo: String,
) -> Result<Vec<MarketFile>, String> {
    let repo = repo.trim().trim_matches('/').to_string();
    if repo.is_empty() {
        return Err(ierr!("仓库 ID 不能为空", "Repo id must not be empty"));
    }
    let token = hf_token(&app);
    let client = market_client()?;
    match source.as_str() {
        "modelscope" => {
            let url = format!(
                "https://modelscope.cn/api/v1/models/{}/repo/files?Revision=master&Root=%2F",
                encode_repo_path(&repo)
            );
            let resp = client.get(&url).send().await.map_err(|e| {
                ierr!(
                    "访问 ModelScope 失败: {}",
                    "Failed to reach ModelScope: {}",
                    e
                )
            })?;
            if !resp.status().is_success() {
                return Err(ierr!(
                    "获取文件列表失败（ModelScope {}）",
                    "Failed to list files (ModelScope {})",
                    resp.status()
                ));
            }
            let value: serde_json::Value = resp
                .json()
                .await
                .map_err(|e| ierr!("解析响应失败: {}", "Failed to parse response: {}", e))?;
            let files: Vec<MarketFile> = value
                .pointer("/Data/Files")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|f| {
                            let path = f.get("Path")?.as_str()?.to_string();
                            if !path.to_lowercase().ends_with(".gguf") {
                                return None;
                            }
                            let size = f.get("Size").and_then(|s| s.as_u64()).unwrap_or(0);
                            Some(MarketFile { name: path, size })
                        })
                        .collect()
                })
                .unwrap_or_default();
            Ok(files)
        }
        "huggingface" => {
            let path = format!("/api/models/{}/tree/main", encode_repo_path(&repo));
            let mut last_err = String::new();
            for base in HF_ENDPOINTS {
                let mut req = client.get(format!("{base}{path}"));
                if let Some(t) = token.as_deref() {
                    req = req.bearer_auth(t);
                }
                match req.send().await {
                    Ok(resp) if resp.status().is_success() => {
                        match resp.json::<Vec<serde_json::Value>>().await {
                            Ok(rows) => {
                                let files: Vec<MarketFile> = rows
                                    .iter()
                                    .filter_map(|r| {
                                        let path = r.get("path")?.as_str()?.to_string();
                                        if !path.to_lowercase().ends_with(".gguf") {
                                            return None;
                                        }
                                        let size =
                                            r.get("size").and_then(|s| s.as_u64()).unwrap_or(0);
                                        Some(MarketFile { name: path, size })
                                    })
                                    .collect();
                                return Ok(files);
                            }
                            Err(e) => last_err = e.to_string(),
                        }
                    }
                    Ok(resp) => last_err = format!("HTTP {}", resp.status()),
                    Err(e) => last_err = e.to_string(),
                }
            }
            Err(ierr!(
                "获取文件列表失败（HuggingFace）: {}",
                "Failed to list files (HuggingFace): {}",
                last_err
            ))
        }
        other => Err(ierr!("未知来源: {}", "Unknown source: {}", other)),
    }
}

/// Probes an NVIDIA GPU via nvidia-smi (name + VRAM); None when absent or non-NVIDIA.
fn probe_nvidia_gpu() -> Option<(String, u64)> {
    let out = std::process::Command::new("nvidia-smi")
        .args([
            "--query-gpu=name,memory.total",
            "--format=csv,noheader,nounits",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let first = text.lines().next()?;
    let mut parts = first.split(',').map(str::trim);
    let name = parts.next()?.to_string();
    let mib: u64 = parts.next()?.parse().ok()?;
    if name.is_empty() || mib == 0 {
        return None;
    }
    Some((name, mib * 1024 * 1024))
}

/// Reports host hardware and the recommended local-model parameter size.
///
/// Recommendation model: GGUF Q4_K_M needs ~0.62 byte per parameter. The memory budget is
/// 85% of VRAM when an NVIDIA GPU with >= 4GB is present (GPU offload), otherwise 45% of
/// system RAM (CPU inference also caps the recommendation at 9B for usable speed).
/// Apple Silicon (no discrete VRAM) follows the RAM path with unified memory.
#[tauri::command]
pub fn local_model_hardware_profile() -> Result<HardwareProfile, String> {
    let sys = sysinfo::System::new_all();
    let cpu_brand = sys
        .cpus()
        .first()
        .map(|c| c.brand().trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Unknown CPU".into());
    let cpu_cores = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1);
    let total_ram_bytes = sys.total_memory();
    let (gpu_name, gpu_vram_bytes) = match probe_nvidia_gpu() {
        Some((name, vram)) => (Some(name), Some(vram)),
        None => (None, None),
    };
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    let (budget, gpu_backed) = match gpu_vram_bytes {
        Some(v) if (v as f64) >= 4.0 * GB => ((v as f64) * 0.85, true),
        _ => (total_ram_bytes as f64 * 0.45, false),
    };
    let mut params = budget / 0.62 / 1e9;
    if !gpu_backed {
        params = params.min(9.0);
    }
    Ok(HardwareProfile {
        cpu_brand,
        cpu_cores,
        total_ram_bytes,
        gpu_name,
        gpu_vram_bytes,
        recommended_params_b: (params * 10.0).round() / 10.0,
    })
}

/// Reads the optional HuggingFace access token (None when never configured).
#[tauri::command]
pub fn local_model_hf_token_get(app: AppHandle) -> Result<Option<String>, String> {
    secure_store::get(&app, &secure_store::accounts::hf_token())
        .map_err(|e| ierr!("读取 Token 失败: {}", "Failed to read token: {}", e))
}

/// Saves (non-empty) or clears (empty) the optional HuggingFace access token.
/// Only needed for gated repos (e.g. Llama): the user must also accept the model license
/// on the HuggingFace website once; public repos work without any token.
#[tauri::command]
pub fn local_model_hf_token_set(app: AppHandle, token: String) -> Result<(), String> {
    let account = secure_store::accounts::hf_token();
    let token = token.trim();
    if token.is_empty() {
        secure_store::delete(&app, &account)
            .map_err(|e| ierr!("清除 Token 失败: {}", "Failed to clear token: {}", e))
    } else {
        secure_store::set_reported(&app, &account, token);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_from_name_common_patterns() {
        assert_eq!(params_from_name("Qwen3-14B-GGUF"), Some(14.0));
        assert_eq!(params_from_name("Qwen3-30B-A3B-Instruct"), Some(30.0));
        assert_eq!(params_from_name("gemma-3-4b-it-GGUF"), Some(4.0));
        assert_eq!(params_from_name("Llama-3.1-8B-Instruct"), Some(8.0));
        assert_eq!(params_from_name("Qwen2.5-0.5B-Instruct-GGUF"), Some(0.5));
        assert_eq!(params_from_name("all-MiniLM-L6-v2"), None);
        assert_eq!(params_from_name("phi-4"), None);
        assert_eq!(params_from_name("BF16-raw"), None);
    }

    #[test]
    fn merge_market_prefers_hf_metadata() {
        let hf = vec![HfListRow {
            id: "unsloth/Qwen3-14B-GGUF".into(),
            downloads: 100,
            likes: 5,
            tags: vec!["gguf".into(), "license:apache-2.0".into()],
        }];
        let ms = vec![MsListRow {
            path: "unsloth".into(),
            name: "Qwen3-14B-GGUF".into(),
            downloads: 200,
            stars: 7,
            license: Some("other".into()),
            chinese_name: Some("千问3-14B".into()),
            libraries: vec!["gguf".into()],
            tags: vec![],
        }];
        let merged = merge_market(hf, ms);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].id, "unsloth/Qwen3-14B-GGUF");
        assert_eq!(merged[0].downloads, 200);
        assert_eq!(merged[0].license.as_deref(), Some("apache-2.0"));
        assert_eq!(merged[0].chinese_name.as_deref(), Some("千问3-14B"));
        assert_eq!(merged[0].sources.len(), 2);
    }

    #[test]
    fn encode_repo_path_keeps_slash_and_encodes_space() {
        assert_eq!(encode_repo_path("org/name.gguf"), "org/name.gguf");
        assert_eq!(encode_repo_path("my model.gguf"), "my%20model.gguf");
    }
}
