//! Skill configuration management commands.

use crate::core::models::{MarketSkill, ProjectsFile, SkillConfig, SkillsFile};
use regex::Regex;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

use crate::ierr;

const SKILLS_FILE: &str = "skills.json";
const MARKET_CACHE_FILE: &str = "skill-market-cache.json";
const SKILLS_SH: &str = "https://www.skills.sh";
const MARKET_CACHE_TTL_SECS: u64 = 6 * 60 * 60;

#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
struct MarketCacheFile {
    #[serde(default)]
    entries: HashMap<String, MarketCacheEntry>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct MarketCacheEntry {
    fetched_at: u64,
    html: String,
}

fn skills_file_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| {
        let msg = ierr!(
            "app_data_dir 解析失败: {}",
            "Failed to resolve app_data_dir: {}",
            e
        );
        tracing::error!("[skills] {msg}");
        msg
    })?;
    let path = dir.join(SKILLS_FILE);
    tracing::info!("[skills] 数据文件路径: {}", path.display());
    Ok(path)
}

fn market_cache_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_cache_dir()
        .map(|dir| dir.join(MARKET_CACHE_FILE))
        .map_err(|e| e.to_string())
}

fn load_market_cache(app: &AppHandle) -> MarketCacheFile {
    let Ok(path) = market_cache_path(app) else {
        return MarketCacheFile::default();
    };
    std::fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default()
}

fn save_market_cache(app: &AppHandle, cache: &MarketCacheFile) -> Result<(), String> {
    let path = market_cache_path(app)?;
    let content = serde_json::to_string(cache).map_err(|e| e.to_string())?;
    crate::core::fs_util::atomic_write(&path, content.as_bytes()).map_err(|e| e.to_string())
}

fn load_file(app: &AppHandle) -> SkillsFile {
    let Ok(path) = skills_file_path(app) else {
        tracing::warn!("[skills] load_file: 路径解析失败，返回空配置");
        return SkillsFile::default();
    };
    if !path.exists() {
        tracing::info!(
            "[skills] load_file: 文件不存在 ({})，返回空配置",
            path.display()
        );
        return SkillsFile::default();
    }
    match std::fs::read_to_string(&path) {
        Ok(content) => match serde_json::from_str::<SkillsFile>(&content) {
            Ok(file) => {
                tracing::info!(
                    "[skills] load_file: 加载 {} 条 skill (from {})",
                    file.skills.len(),
                    path.display()
                );
                file
            }
            Err(e) => {
                tracing::error!(
                    "[skills] load_file: JSON 解析失败 ({}): {e}",
                    path.display()
                );
                SkillsFile::default()
            }
        },
        Err(e) => {
            tracing::error!("[skills] load_file: 读取文件失败 ({}): {e}", path.display());
            SkillsFile::default()
        }
    }
}

fn save_file(app: &AppHandle, file: &SkillsFile) -> Result<(), String> {
    let path = skills_file_path(app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            let msg = ierr!("创建目录失败: {}", "Failed to create directory: {}", e);
            tracing::error!("[skills] save_file: {msg} ({})", parent.display());
            msg
        })?;
    }
    let content = serde_json::to_string_pretty(file).map_err(|e| {
        let msg = ierr!(
            "序列化 skills 失败: {}",
            "Failed to serialize skills: {}",
            e
        );
        tracing::error!("[skills] save_file: {msg}");
        msg
    })?;
    crate::core::fs_util::atomic_write(&path, content.as_bytes()).map_err(|e| {
        let msg = ierr!("写入文件失败: {}", "Failed to write file: {}", e);
        tracing::error!("[skills] save_file: {msg} ({})", path.display());
        msg
    })?;
    tracing::info!(
        "[skills] save_file: 保存 {} 条 skill → {}",
        file.skills.len(),
        path.display()
    );
    Ok(())
}

// ====== Project-level skill file operations ======

/// Loads the project-level skills.json (returns an empty file if it does not exist).
fn load_project_file(project_path: &str) -> SkillsFile {
    let path = crate::commands::project::project_skills_path(project_path);
    if !path.exists() {
        tracing::info!(
            "[skills] load_project_file: 文件不存在 ({})，返回空配置",
            path.display()
        );
        return SkillsFile::default();
    }
    match std::fs::read_to_string(&path) {
        Ok(content) => match serde_json::from_str::<SkillsFile>(&content) {
            Ok(file) => {
                tracing::info!(
                    "[skills] load_project_file: 加载 {} 条 skill (from {})",
                    file.skills.len(),
                    path.display()
                );
                file
            }
            Err(e) => {
                tracing::error!(
                    "[skills] load_project_file: JSON 解析失败 ({}): {e}",
                    path.display()
                );
                SkillsFile::default()
            }
        },
        Err(e) => {
            tracing::error!(
                "[skills] load_project_file: 读取文件失败 ({}): {e}",
                path.display()
            );
            SkillsFile::default()
        }
    }
}

/// Saves the project-level skills.json.
fn save_project_file(project_path: &str, file: &SkillsFile) -> Result<(), String> {
    let path = crate::commands::project::project_skills_path(project_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            let msg = ierr!("创建目录失败: {}", "Failed to create directory: {}", e);
            tracing::error!("[skills] save_project_file: {msg} ({})", parent.display());
            msg
        })?;
    }
    let content = serde_json::to_string_pretty(file).map_err(|e| {
        let msg = ierr!(
            "序列化 skills 失败: {}",
            "Failed to serialize skills: {}",
            e
        );
        tracing::error!("[skills] save_project_file: {msg}");
        msg
    })?;
    crate::core::fs_util::atomic_write(&path, content.as_bytes()).map_err(|e| {
        let msg = ierr!("写入文件失败: {}", "Failed to write file: {}", e);
        tracing::error!("[skills] save_project_file: {msg} ({})", path.display());
        msg
    })?;
    tracing::info!(
        "[skills] save_project_file: 保存 {} 条 skill → {}",
        file.skills.len(),
        path.display()
    );
    Ok(())
}

/// Resolves a project path from its project_id.
fn resolve_project_path(app: &AppHandle, project_id: &str) -> Result<String, String> {
    let file = load_projects_index(app);
    file.projects
        .get(project_id)
        .map(|p| p.path.clone())
        .ok_or_else(|| ierr!("项目 {} 不存在", "Project {} not found", project_id))
}

/// Loads the projects index file (implementation lives in utils::project_index).
fn load_projects_index(app: &AppHandle) -> ProjectsFile {
    crate::utils::project_index::load_projects_file(app)
}

/// Merges global and project skills: a project skill with the same ID overrides the global one.
fn merge_skills(
    global: BTreeMap<String, SkillConfig>,
    project: BTreeMap<String, SkillConfig>,
) -> BTreeMap<String, SkillConfig> {
    let mut merged = global;
    for (id, skill) in project {
        merged.insert(id, skill);
    }
    merged
}

fn unix_timestamp() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn now_timestamp() -> String {
    unix_timestamp().to_string()
}

fn cache_entry_is_fresh(entry: &MarketCacheEntry, now: u64) -> bool {
    now.saturating_sub(entry.fetched_at) < MARKET_CACHE_TTL_SECS
}

fn market_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent("AIO Skill Market/0.4")
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(25))
        .build()
        .map_err(|e| e.to_string())
}

async fn fetch_market_page(path: &str) -> Result<String, String> {
    let response = market_client()?
        .get(format!("{}{}", SKILLS_SH, path))
        .send()
        .await
        .map_err(|e| {
            ierr!(
                "请求 skills.sh 失败: {}",
                "Failed to request skills.sh: {}",
                e
            )
        })?;
    if !response.status().is_success() {
        return Err(ierr!(
            "skills.sh 返回 HTTP {}",
            "skills.sh returned HTTP {}",
            response.status()
        ));
    }
    response.text().await.map_err(|e| e.to_string())
}

async fn fetch_market_page_cached(
    app: &AppHandle,
    path: &str,
    force_refresh: bool,
) -> Result<String, String> {
    let mut cache = load_market_cache(app);
    if !force_refresh {
        if let Some(entry) = cache.entries.get(path) {
            if cache_entry_is_fresh(entry, unix_timestamp()) {
                return Ok(entry.html.clone());
            }
        }
    }

    match fetch_market_page(path).await {
        Ok(html) => {
            cache.entries.insert(
                path.to_string(),
                MarketCacheEntry {
                    fetched_at: unix_timestamp(),
                    html: html.clone(),
                },
            );
            if let Err(error) = save_market_cache(app, &cache) {
                tracing::warn!("保存 Skill 市场缓存失败: {}", error);
            }
            Ok(html)
        }
        Err(error) => {
            // When the network is unavailable, allow a stale cache so the market page stays browsable.
            if force_refresh {
                Err(error)
            } else {
                cache
                    .entries
                    .get(path)
                    .map(|entry| entry.html.clone())
                    .ok_or(error)
            }
        }
    }
}

fn decode_html(value: &str) -> String {
    let mut output = value
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ");
    let numeric =
        Regex::new(r"&#(?:x([0-9A-Fa-f]+)|([0-9]+));").expect("valid numeric entity regex");
    output = numeric
        .replace_all(&output, |caps: &regex::Captures<'_>| {
            let parsed = caps
                .get(1)
                .and_then(|value| u32::from_str_radix(value.as_str(), 16).ok())
                .or_else(|| {
                    caps.get(2)
                        .and_then(|value| value.as_str().parse::<u32>().ok())
                });
            parsed
                .and_then(char::from_u32)
                .map(|c| c.to_string())
                .unwrap_or_default()
        })
        .into_owned();
    output
}

fn strip_html(value: &str) -> String {
    let block_tags =
        Regex::new(r"(?i)</?(?:p|div|h[1-6]|li|ul|ol|pre|table|tr|blockquote|br)[^>]*>")
            .expect("valid block tag regex");
    let tags = Regex::new(r"(?s)<[^>]+>").expect("valid html tag regex");
    let comments = Regex::new(r"<!--.*?-->").expect("valid comment regex");
    let with_lines = block_tags.replace_all(value, "\n");
    let without_tags = tags.replace_all(&with_lines, "");
    let without_comments = comments.replace_all(&without_tags, "");
    let decoded = decode_html(&without_comments);
    decoded
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && *line != "Show more")
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_compact_number(value: &str) -> u64 {
    let cleaned = value.trim().replace(',', "");
    let (number, multiplier) = match cleaned.chars().last() {
        Some('K') | Some('k') => (&cleaned[..cleaned.len() - 1], 1_000_f64),
        Some('M') | Some('m') => (&cleaned[..cleaned.len() - 1], 1_000_000_f64),
        _ => (cleaned.as_str(), 1_f64),
    };
    number
        .parse::<f64>()
        .map(|value| (value * multiplier) as u64)
        .unwrap_or(0)
}

fn parse_weekly_installs(value: &str) -> Vec<u64> {
    value
        .split(", ")
        .map(|part| part.trim().replace(',', ""))
        .filter_map(|part| part.parse::<u64>().ok())
        .collect()
}

fn parse_market_skills(html: &str) -> Vec<MarketSkill> {
    let anchor = Regex::new(
        r#"(?s)<a class="group grid[^"]*" href="/([^/"]+)/([^/"]+)/([^/"]+)">(.*?)</a>"#,
    )
    .expect("valid market anchor regex");
    let name_re = Regex::new(r#"(?s)<h3[^>]*>(.*?)</h3>"#).expect("valid name regex");
    let description_re = Regex::new(r#"(?s)<p class="lg:col-span-9[^"]*">(.*?)</p>"#)
        .expect("valid description regex");
    let weekly_re = Regex::new(r#"aria-label="Weekly installs: ([^"]+)""#)
        .expect("valid weekly installs regex");
    let installs_re =
        Regex::new(r#"<span class="font-mono text-sm text-foreground">([^<]+)</span>"#)
            .expect("valid installs regex");

    let mut seen = HashSet::new();
    anchor
        .captures_iter(html)
        .filter_map(|caps| {
            let owner = caps.get(1)?.as_str().to_string();
            let repo = caps.get(2)?.as_str().to_string();
            let slug = caps.get(3)?.as_str().to_string();
            let body = caps.get(4)?.as_str();
            let path = format!("{}/{}/{}", owner, repo, slug);
            if !seen.insert(path.clone()) {
                return None;
            }
            let name = name_re
                .captures(body)
                .and_then(|capture| capture.get(1))
                .map(|value| strip_html(value.as_str()))
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| slug.clone());
            let description = description_re
                .captures(body)
                .and_then(|capture| capture.get(1))
                .map(|value| strip_html(value.as_str()))
                .unwrap_or_default();
            let installs_label = installs_re
                .captures(body)
                .and_then(|capture| capture.get(1))
                .map(|value| decode_html(value.as_str()))
                .unwrap_or_default();
            let weekly_installs = weekly_re
                .captures(body)
                .and_then(|capture| capture.get(1))
                .map(|value| parse_weekly_installs(value.as_str()))
                .unwrap_or_default();
            Some(MarketSkill {
                id: format!("skills-sh-{}-{}-{}", owner, repo, slug),
                name,
                owner,
                repo,
                slug,
                description,
                source_url: format!("{}/{}", SKILLS_SH, path),
                installs: parse_compact_number(&installs_label),
                installs_label,
                weekly_installs,
            })
        })
        .collect()
}

/// Returns all skill configs. When project_id is present, global and project skills are merged (project wins).
/// HOR-4: disk-backed skill files (global + project) — async + blocking pool, never the main thread.
#[tauri::command]
pub async fn list_skills(
    app: AppHandle,
    project_id: Option<String>,
) -> Result<Vec<SkillConfig>, String> {
    tokio::task::spawn_blocking(move || {
        let global = load_file(&app).skills;
        let project = match project_id {
            Some(ref pid) => {
                let project_path = resolve_project_path(&app, pid)?;
                load_project_file(&project_path).skills
            }
            None => BTreeMap::new(),
        };
        Ok(merge_skills(global, project).into_values().collect())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Creates or updates a skill. When project_id is present, writes to the project-level file.
/// HOR-4: fs read-modify-write — async + blocking pool.
#[tauri::command]
pub async fn save_skill(
    app: AppHandle,
    skill: SkillConfig,
    project_id: Option<String>,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        if skill.id.trim().is_empty()
            || skill.name.trim().is_empty()
            || skill.content.trim().is_empty()
        {
            return Err(ierr!(
                "Skill id、名称和指令内容不能为空",
                "Skill id, name, and instruction content must not be empty"
            ));
        }
        match project_id {
            Some(ref pid) => {
                let project_path = resolve_project_path(&app, pid)?;
                let mut file = load_project_file(&project_path);
                file.skills.insert(skill.id.clone(), skill);
                file.updated_at = now_timestamp();
                save_project_file(&project_path, &file)
            }
            None => {
                let mut file = load_file(&app);
                file.skills.insert(skill.id.clone(), skill);
                file.updated_at = now_timestamp();
                save_file(&app, &file)
            }
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Deletes the given skill config. When project_id is present, deletes from the project-level file.
/// HOR-4: fs read-modify-write — async + blocking pool.
#[tauri::command]
pub async fn delete_skill(
    app: AppHandle,
    id: String,
    project_id: Option<String>,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || match project_id {
        Some(ref pid) => {
            let project_path = resolve_project_path(&app, pid)?;
            let mut file = load_project_file(&project_path);
            file.skills.remove(&id);
            file.updated_at = now_timestamp();
            save_project_file(&project_path, &file)
        }
        None => {
            let mut file = load_file(&app);
            file.skills.remove(&id);
            file.updated_at = now_timestamp();
            save_file(&app, &file)
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Browses the skills.sh market. sort supports all, trending, hot.
#[tauri::command]
pub async fn list_skill_market(
    app: AppHandle,
    sort: String,
    force_refresh: bool,
) -> Result<Vec<MarketSkill>, String> {
    let path = match sort.as_str() {
        "trending" => "/trending",
        "hot" => "/hot",
        _ => "/",
    };
    let skills = parse_market_skills(&fetch_market_page_cached(&app, path, force_refresh).await?);
    Ok(skills)
}

/// skills.sh search API item (`/api/search?q=`, Algolia-backed fuzzy search).
#[derive(serde::Deserialize)]
struct SkillsShSearchItem {
    /// `owner/repo/slug`
    id: String,
    name: String,
    #[serde(default)]
    installs: u64,
}

/// Compact install-count label matching the scraped pages' style (1.2k / 3.4M).
fn format_installs_label(installs: u64) -> String {
    if installs >= 1_000_000 {
        format!("{:.1}M", installs as f64 / 1_000_000.0)
    } else if installs >= 1_000 {
        format!("{:.1}k", installs as f64 / 1_000.0)
    } else {
        installs.to_string()
    }
}

/// Map the skills.sh `/api/search` JSON body into MarketSkill rows
/// (owner/repo/slug split from the 3-segment id; malformed ids dropped).
fn map_search_response(value: &Value) -> Vec<MarketSkill> {
    let items: Vec<SkillsShSearchItem> = value
        .get("skills")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    items
        .into_iter()
        .filter_map(|item| {
            let mut parts = item.id.splitn(3, '/');
            let (owner, repo, slug) = (
                parts.next()?.to_string(),
                parts.next()?.to_string(),
                parts.next()?.to_string(),
            );
            if slug.is_empty() {
                return None;
            }
            Some(MarketSkill {
                id: format!("skills-sh-{}-{}-{}", owner, repo, slug),
                name: item.name,
                source_url: format!("{}/{}/{}/{}", SKILLS_SH, owner, repo, slug),
                installs: item.installs,
                installs_label: format_installs_label(item.installs),
                owner,
                repo,
                slug,
                description: String::new(),
                weekly_installs: Vec::new(),
            })
        })
        .collect()
}

/// Searches the full skills.sh catalog via its JSON search API — the market pages
/// only ship the top slice per sort view, so skills outside it were unfindable
/// before this existed. Results are cached per query with the market cache TTL.
#[tauri::command]
pub async fn search_skill_market(
    app: AppHandle,
    query: String,
) -> Result<Vec<MarketSkill>, String> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let encoded: String = url::form_urlencoded::byte_serialize(q.as_bytes()).collect();
    let path = format!("/api/search?q={encoded}");
    let body = fetch_market_page_cached(&app, &path, false).await?;
    let value: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    Ok(map_search_response(&value))
}

/// Downloads skill content from a skills.sh detail page and saves it to the skill library. When project_id is present, writes to the project-level file.
#[tauri::command]
pub async fn download_market_skill(
    app: AppHandle,
    owner: String,
    repo: String,
    slug: String,
    project_id: Option<String>,
) -> Result<SkillConfig, String> {
    if [&owner, &repo, &slug]
        .iter()
        .any(|value| value.is_empty() || value.contains('/') || value.contains(".."))
    {
        return Err(ierr!("非法的 Skill 路径", "Invalid skill path"));
    }
    let source_url = format!("{}/{}/{}/{}", SKILLS_SH, owner, repo, slug);
    let html = fetch_market_page(&format!("/{}/{}/{}", owner, repo, slug)).await?;

    let json_ld = Regex::new(r#"(?s)<script type="application/ld\+json">(\{.*?"@type":"SoftwareApplication".*?\})</script>"#)
        .expect("valid json-ld regex");
    let metadata: Value = json_ld
        .captures(&html)
        .and_then(|capture| capture.get(1))
        .and_then(|value| serde_json::from_str(value.as_str()).ok())
        .unwrap_or(Value::Null);
    let name = metadata
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or(&slug)
        .to_string();
    let description = metadata
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let installs = metadata
        .pointer("/interactionStatistic/userInteractionCount")
        .and_then(Value::as_u64)
        .unwrap_or(0);

    let marker = "<span>SKILL.md</span>";
    let marker_index = html
        .find(marker)
        .ok_or_else(|| ierr!("详情页缺少 SKILL.md", "Detail page is missing SKILL.md"))?;
    let prose_start = html[marker_index..]
        .find("<div class=\"prose ")
        .map(|index| marker_index + index)
        .ok_or_else(|| ierr!("无法定位 Skill 内容", "Unable to locate skill content"))?;
    let content_start = html[prose_start..]
        .find('>')
        .map(|index| prose_start + index + 1)
        .ok_or_else(|| ierr!("无法解析 Skill 内容", "Unable to parse skill content"))?;
    let content_end = html[content_start..]
        .find("<div class=\"relative\">")
        .map(|index| content_start + index)
        .or_else(|| {
            html[content_start..]
                .find("<section class=\"mt-16\">")
                .map(|index| content_start + index)
        })
        .ok_or_else(|| {
            ierr!(
                "无法确定 Skill 内容边界",
                "Unable to determine skill content boundary"
            )
        })?;
    let content = strip_html(&html[content_start..content_end]);
    if content.trim().is_empty() {
        return Err(ierr!(
            "下载到的 Skill 内容为空",
            "Downloaded skill content is empty"
        ));
    }

    let skill = SkillConfig {
        id: format!("skills-sh-{}-{}-{}", owner, repo, slug),
        name,
        description,
        content,
        source: crate::core::models::SkillSource::Market,
        source_url: Some(source_url),
        source_owner: Some(owner),
        source_repo: Some(repo),
        source_slug: Some(slug),
        installs,
        npx_package: None,
        npx_version: None,
        npx_command: None,
        source_disk_path: None,
    };
    match project_id {
        Some(ref pid) => {
            let project_path = resolve_project_path(&app, pid)?;
            let mut file = load_project_file(&project_path);
            file.skills.insert(skill.id.clone(), skill.clone());
            file.updated_at = now_timestamp();
            save_project_file(&project_path, &file)?;
        }
        None => {
            let mut file = load_file(&app);
            file.skills.insert(skill.id.clone(), skill.clone());
            file.updated_at = now_timestamp();
            save_file(&app, &file)?;
        }
    }
    Ok(skill)
}

// ====== npx skill discovery and import ======

/// Validates an npx package name.
///
/// Rules:
/// - Follows npm naming conventions: `@scope/name` or `name`, lowercase letters, digits, `-`, `_`, `.` only
/// - Total length <= 214 characters
/// - No shell metacharacters
/// - Not on the high-risk blocklist
fn validate_npx_package_name(package_name: &str) -> Result<(), String> {
    // Reject empty names
    if package_name.trim().is_empty() {
        return Err(ierr!("包名不能为空", "Package name must not be empty"));
    }

    // Reject path traversal
    if package_name.contains("..") {
        return Err(ierr!(
            "包名包含非法字符 '..'",
            "Package name contains illegal characters '..'"
        ));
    }

    // Reject shell metacharacters
    for ch in package_name.chars() {
        if matches!(
            ch,
            ';' | '|' | '&' | '$' | '`' | '\\' | '\'' | '"' | '<' | '>' | '!' | '\n' | '\r' | '\t'
        ) {
            return Err(ierr!(
                "包名包含禁止字符: '{}'",
                "Package name contains a forbidden character: '{}'",
                ch
            ));
        }
    }

    // npm naming convention: @scope/name or name
    let name = if let Some(rest) = package_name.strip_prefix('@') {
        // scoped package: @scope/name
        let parts: Vec<&str> = rest.splitn(2, '/').collect();
        if parts.len() != 2 {
            return Err(ierr!(
                "scoped 包名格式错误: 应为 @scope/name",
                "Invalid scoped package name: expected @scope/name"
            ));
        }
        let scope = parts[0];
        let name_part = parts[1];
        if scope.is_empty() || name_part.is_empty() {
            return Err(ierr!(
                "scoped 包名格式错误: scope 或 name 为空",
                "Invalid scoped package name: scope or name is empty"
            ));
        }
        // Validate the scope
        if !scope.chars().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_' || c == '.'
        }) {
            return Err(ierr!(
                "scoped 包名 scope 部分包含非法字符",
                "Scoped package name scope contains illegal characters"
            ));
        }
        name_part
    } else {
        package_name
    };

    // Validate the name part
    if name.is_empty() || name.len() > 214 {
        return Err(ierr!(
            "包名长度不合法: {} 字符（最大 214）",
            "Invalid package name length: {} characters (max 214)",
            name.len()
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_' || c == '.')
    {
        return Err(ierr!(
            "包名包含非法字符（仅允许小写字母、数字、-、_、.）",
            "Package name contains illegal characters (only lowercase letters, digits, -, _, . are allowed)"
        ));
    }

    // Reject names impersonating Node.js built-in modules
    let node_builtins = [
        "child_process",
        "fs",
        "os",
        "path",
        "process",
        "buffer",
        "stream",
        "net",
        "tls",
        "http",
        "https",
        "dns",
        "dgram",
        "crypto",
        "util",
        "events",
        "assert",
        "vm",
        "v8",
        "worker_threads",
        "cluster",
        "module",
    ];
    let lower_name = name.to_lowercase();
    let base_name = if let Some(pos) = lower_name.rfind('/') {
        &lower_name[pos + 1..]
    } else {
        &lower_name
    };
    if node_builtins.contains(&base_name) {
        return Err(ierr!(
            "包名 '{}' 伪装为 Node.js 内置模块，拒绝导入",
            "Package name '{}' impersonates a Node.js built-in module; import rejected",
            package_name
        ));
    }

    // Reject git protocol URL spoofing
    if package_name.starts_with("git://")
        || package_name.starts_with("git+")
        || package_name.starts_with("ssh://")
    {
        return Err(ierr!(
            "包名不是合法的 npm 包名",
            "Package name is not a valid npm package name"
        ));
    }

    Ok(())
}

/// Information about an npx skill package detected on the system.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct NpxSkillInfo {
    pub package_name: String,
    pub version: String,
    pub description: String,
    pub source_path: String,
    pub source_type: String,
    pub already_imported: bool,
}

/// Scans the system for installed npx skill packages.
#[tauri::command]
pub async fn discover_npx_skills(app: AppHandle) -> Result<Vec<NpxSkillInfo>, String> {
    let mut discovered: Vec<NpxSkillInfo> = Vec::new();
    let existing = load_file(&app).skills;
    tracing::info!(
        "[skills] discover_npx: 当前已导入 {} 条 skill（用于 already_imported 标记）",
        existing.len()
    );

    // 1) Primary method: get the installed skill list via `npx skills list --json`
    tracing::info!("[skills] discover_npx: 正在执行 npx --yes skills list -g --json ...");

    // Use tokio spawn_blocking to avoid blocking an async worker thread
    let skills_cli_result = tokio::task::spawn_blocking(move || {
        use crate::utils::exec_resolve::HiddenConsole;
        let npx_cmd = resolve_command("npx");
        let child = match std::process::Command::new(&npx_cmd)
            .args(["--yes", "skills", "list", "-g", "--json"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .hidden_console()
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                return Err(ierr!(
                    "启动 {} 失败: {}",
                    "Failed to start {}: {}",
                    npx_cmd,
                    e
                ))
            }
        };

        // Windows: assign each run to its own Job Object (the guard cleans up the whole process tree on Drop)
        let _job_guard = match crate::utils::sandbox::assign_to_job(&child) {
            Ok(g) => Some(g),
            Err(e) => {
                tracing::warn!("[skills] discover npx sandbox 分配失败: {e}");
                None
            }
        };

        let output = child.wait_with_output().map_err(|e| {
            ierr!(
                "等待 npx 进程失败: {}",
                "Failed to wait for npx process: {}",
                e
            )
        })?;

        // Truncate output to 1MB
        let stdout = if output.stdout.len() > 1_048_576 {
            tracing::warn!(
                "[skills] discover npx stdout 过大 ({}B)，已截断",
                output.stdout.len()
            );
            output.stdout[..1_048_576].to_vec()
        } else {
            output.stdout
        };
        Ok((
            output.status.success(),
            String::from_utf8_lossy(&stdout).to_string(),
            String::from_utf8_lossy(&output.stderr).to_string(),
        ))
    })
    .await
    .unwrap_or_else(|_| Err(ierr!("npx 线程 panic", "npx thread panicked")));

    // Handle the npx skills list result
    match skills_cli_result {
        Ok((success, stdout, stderr)) => {
            if success {
                match serde_json::from_str::<Vec<serde_json::Value>>(&stdout) {
                    Ok(json_list) => {
                        tracing::info!(
                            "[skills] discover_npx: skills CLI 返回 {} 条 skill",
                            json_list.len()
                        );
                        for entry in &json_list {
                            let name = entry["name"].as_str().unwrap_or("").to_string();
                            let path_str = entry["path"].as_str().unwrap_or("").to_string();
                            if name.is_empty() {
                                continue;
                            }
                            let skill_dir = std::path::Path::new(&path_str);
                            let (version, description) = try_read_package_meta_from_dir(skill_dir);
                            let already = existing.contains_key(&format!("npx-{}", name));
                            if discovered.iter().any(|d| d.package_name == name) {
                                continue;
                            }
                            tracing::info!(
                                "[skills] discover_npx:   - {} (v{}, already_imported={})",
                                name,
                                version,
                                already
                            );
                            discovered.push(NpxSkillInfo {
                                package_name: name,
                                version,
                                description,
                                source_path: path_str,
                                source_type: "skills-cli".to_string(),
                                already_imported: already,
                            });
                        }
                    }
                    Err(e) => {
                        tracing::warn!("[skills] discover_npx: skills CLI JSON 解析失败: {e}");
                        tracing::warn!(
                            "[skills] discover_npx: stdout 前 200 字符: {}",
                            &stdout.chars().take(200).collect::<String>()
                        );
                    }
                }
            } else {
                tracing::warn!(
                    "[skills] discover_npx: skills CLI 退出码非 0 (stderr: {})",
                    stderr.trim()
                );
            }
        }
        Err(e) => {
            tracing::warn!("[skills] discover_npx: npx 启动失败: {e}");
        }
    }

    // If the skills CLI returned nothing, fall back to directory scanning
    if discovered.is_empty() {
        tracing::info!("[skills] discover_npx: skills CLI 无结果，回退到目录扫描");
        // 2) Scan known skill registry directories: ~/.claude/skills/ and ~/.agents/skills/
        let skill_dirs: Vec<std::path::PathBuf> = dirs::home_dir()
            .into_iter()
            .flat_map(|h| {
                vec![
                    h.join(".claude").join("skills"),
                    h.join(".agents").join("skills"),
                ]
            })
            .filter(|p| p.exists())
            .collect();

        for dir in &skill_dirs {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        if let Some(info) = read_skill_dir_info(&path, &existing, &discovered) {
                            discovered.push(info);
                        }
                    }
                }
            }
        }

        // 3) Scan global npm packages with skill-like traits (fallback)
        let npm_output = {
            use crate::utils::exec_resolve::HiddenConsole;
            std::process::Command::new(resolve_command("npm"))
                .args(["list", "-g", "--depth=0", "--json"])
                .hidden_console()
                .output()
        };
        if let Ok(output) = npm_output {
            if output.status.success() {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(
                    &String::from_utf8_lossy(&output.stdout),
                ) {
                    if let Some(deps) = json.get("dependencies").and_then(|v| v.as_object()) {
                        for (name, info) in deps {
                            let is_skill_pkg = name.contains("skill")
                                || name.starts_with("@anthropic-ai/skill-")
                                || name.starts_with("@claude/");
                            if !is_skill_pkg {
                                continue;
                            }
                            let version = info["version"].as_str().unwrap_or("0.0.0").to_string();
                            let description =
                                info["description"].as_str().unwrap_or("").to_string();
                            let already = existing.contains_key(&format!("npx-{}", name));
                            if discovered.iter().any(|d| d.package_name == *name) {
                                continue;
                            }
                            discovered.push(NpxSkillInfo {
                                package_name: name.clone(),
                                version,
                                description,
                                source_path: format!("global-npm:{}", name),
                                source_type: "global-npm".to_string(),
                                already_imported: already,
                            });
                        }
                    }
                }
            }
        }
    }

    // Sort by name
    discovered.sort_by(|a, b| a.package_name.cmp(&b.package_name));
    tracing::info!(
        "[skills] discover_npx: 最终发现 {} 条 npx skill",
        discovered.len()
    );
    Ok(discovered)
}

/// Reads version and description from a skill directory's package.json.
fn read_skill_dir_info(
    path: &std::path::Path,
    existing: &std::collections::BTreeMap<String, SkillConfig>,
    discovered: &[NpxSkillInfo],
) -> Option<NpxSkillInfo> {
    let pkg_json = path.join("package.json");
    let content = std::fs::read_to_string(&pkg_json).ok()?;
    let pkg: serde_json::Value = serde_json::from_str(&content).ok()?;
    let name = pkg["name"].as_str().unwrap_or("").to_string();
    if name.is_empty() {
        return None;
    }
    let version = pkg["version"].as_str().unwrap_or("0.0.0").to_string();
    let description = pkg["description"].as_str().unwrap_or("").to_string();
    let already = existing.contains_key(&format!("npx-{}", name));
    if discovered.iter().any(|d| d.package_name == name) {
        return None;
    }
    Some(NpxSkillInfo {
        package_name: name,
        version,
        description,
        source_path: path.to_string_lossy().to_string(),
        source_type: "skill-dir".to_string(),
        already_imported: already,
    })
}

/// Reads version and description from a skill directory's package.json (for skills CLI JSON results).
fn try_read_package_meta_from_dir(dir: &std::path::Path) -> (String, String) {
    let pkg_json = dir.join("package.json");
    match std::fs::read_to_string(&pkg_json) {
        Ok(content) => match serde_json::from_str::<serde_json::Value>(&content) {
            Ok(pkg) => (
                pkg["version"].as_str().unwrap_or("0.0.0").to_string(),
                pkg["description"].as_str().unwrap_or("").to_string(),
            ),
            Err(_) => ("0.0.0".to_string(), String::new()),
        },
        Err(_) => ("0.0.0".to_string(), String::new()),
    }
}

// ====== npx skill execution security layer ======

/// Runs an npx subprocess with timeout and output limits.
///
/// # Parameters
/// - `args` — arguments passed to npx (excluding "npx" itself)
/// - `timeout_secs` — timeout in seconds; the process is force-killed on expiry
/// - `max_output_bytes` — maximum allowed stdout bytes
///
/// # Returns
/// Ok(Some(stdout)) — succeeded with output
/// Ok(None) — succeeded but output is empty
/// Err(msg) — spawn failure, non-zero exit code, or timeout
fn run_npx_sandboxed(
    args: &[&str],
    timeout_secs: u64,
    max_output_bytes: usize,
) -> Result<Option<String>, String> {
    let npx_cmd = resolve_command("npx");
    let child = {
        use crate::utils::exec_resolve::HiddenConsole;
        std::process::Command::new(&npx_cmd)
            .args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .hidden_console()
            .spawn()
            .map_err(|e| ierr!("启动 {} 失败: {}", "Failed to start {}: {}", npx_cmd, e))?
    };

    // Windows: assign each run to its own Job Object. Dropping the guard (including the
    // early `?` return on timeout) triggers KILL_ON_JOB_CLOSE and tears down the whole
    // npx process tree, so timed-out children never outlive the run.
    let _job_guard = match crate::utils::sandbox::assign_to_job(&child) {
        Ok(g) => Some(g),
        Err(e) => {
            tracing::warn!("[skills] npx sandbox 分配失败: {e}");
            None
        }
    };

    // Wait with timeout
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let output = child.wait_with_output();
        let _ = tx.send(output);
    });

    let output = rx
        .recv_timeout(std::time::Duration::from_secs(timeout_secs))
        .map_err(|_| {
            ierr!(
                "npx 命令超时（>{}s），已终止",
                "npx command timed out (>{}s), terminated",
                timeout_secs
            )
        })?
        .map_err(|e| {
            ierr!(
                "等待 npx 进程失败: {}",
                "Failed to wait for npx process: {}",
                e
            )
        })?;

    // Log stderr for auditing
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stderr.is_empty() {
        let stderr_short: String = stderr.chars().take(300).collect();
        tracing::info!("[skills] npx stderr: {}", stderr_short);
    }

    if !output.status.success() {
        let stderr_short: String = stderr.chars().take(300).collect();
        return Err(ierr!(
            "npx 退出码 {}: {}",
            "npx exited with code {}: {}",
            output.status,
            stderr_short
        ));
    }

    // Truncate output
    let stdout_bytes = if output.stdout.len() > max_output_bytes {
        tracing::warn!(
            "[skills] npx stdout 过大 ({}B > {}B limit)，已截断",
            output.stdout.len(),
            max_output_bytes
        );
        &output.stdout[..max_output_bytes]
    } else {
        &output.stdout
    };

    let content = String::from_utf8_lossy(stdout_bytes).to_string();
    if content.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(content))
}

/// Reads skill content from an on-disk skill directory (skills CLI / skill-dir
/// sources): SKILL.md / skill.md / README.md / index.md，取第一个非空者。
fn read_content_from_skill_dir(dir: &std::path::Path) -> Option<String> {
    for file in ["SKILL.md", "skill.md", "README.md", "index.md"] {
        let p = dir.join(file);
        if let Ok(s) = std::fs::read_to_string(&p) {
            if !s.trim().is_empty() {
                return Some(s);
            }
        }
    }
    None
}

/// Sanitizes npx stdout content before storing it in a SkillConfig.
///
/// Strips ANSI escape sequences and null bytes, validates UTF-8,
/// and checks whether the content is pure binary.
fn sanitize_skill_content(raw: &str) -> Result<String, String> {
    // 1. Strip ANSI escape sequences (CSI sequences: ESC [ ... m)
    let re_ansi = regex::Regex::new("\x1b\\[[0-9;]*[a-zA-Z]").unwrap();
    let mut cleaned = re_ansi.replace_all(raw, "").to_string();

    // 2. Strip null bytes
    cleaned = cleaned.replace('\0', "");

    // 3. Strip other control characters (keeping newline and tab)
    cleaned = cleaned
        .chars()
        .filter(|c| *c == '\n' || *c == '\r' || *c == '\t' || !c.is_control())
        .collect();

    // 4. Trim leading and trailing whitespace
    cleaned = cleaned.trim().to_string();

    if cleaned.is_empty() {
        return Err(ierr!(
            "清洗后内容为空（可能为纯控制字符）",
            "Content is empty after sanitization (may be pure control characters)"
        ));
    }

    // 5. Check the printable-character ratio of the first 512 bytes (reject binary blobs)
    let sample = if cleaned.len() > 512 {
        &cleaned[..512]
    } else {
        &cleaned
    };
    let printable_count = sample
        .chars()
        .filter(|c| c.is_ascii_graphic() || c.is_whitespace())
        .count();
    if !sample.is_empty() && (printable_count as f64 / sample.len() as f64) < 0.5 {
        return Err(ierr!(
            "内容看似为二进制数据，拒绝导入",
            "Content appears to be binary data; import rejected"
        ));
    }

    Ok(cleaned)
}

/// Imports an npx skill package: runs npx to fetch content and saves it to the skill pool.
/// 内容获取分源：skills CLI / skill-dir 来源（discovery 的 source_type）的 name **不是
/// npm 包**——`npx <name>` 必然报 "could not determine executable to run"（round20 实测），
/// 正确路径是直接从安装目录读取 SKILL.md；仅真正的 npm 包（bin 打印 Markdown）走 npx 执行。
#[tauri::command]
pub async fn import_npx_skill(
    app: AppHandle,
    package_name: String,
    project_id: Option<String>,
    source_path: Option<String>,
) -> Result<SkillConfig, String> {
    // Validate the package name
    validate_npx_package_name(&package_name)?;

    // Content fetch, source-aware: an on-disk skill directory (from discovery) is
    // read directly; otherwise fall back to running the package via npx.
    let raw_content = match source_path
        .as_deref()
        .map(std::path::Path::new)
        .filter(|p| p.is_dir())
        .and_then(read_content_from_skill_dir)
    {
        Some(c) => c,
        None => {
            let pkg = package_name.clone();
            let npx_result =
                tokio::task::spawn_blocking(move || run_npx_sandboxed(&[&pkg], 30, 64 * 1024))
                    .await
                    .map_err(|_| ierr!("npx 线程 panic", "npx thread panicked"))?;
            match npx_result {
                Ok(Some(raw)) => raw,
                Ok(None) => {
                    return Err(ierr!(
                        "npx {} 未返回任何内容",
                        "npx {} returned no content",
                        package_name
                    ))
                }
                Err(e) if e.contains("could not determine executable") => {
                    return Err(ierr!(
                        "npx {} 不是可执行 npm 包（无 bin 入口）。skills CLI 安装的 skill 会自动从安装目录导入；手工导入需要「执行后打印 Markdown 的 npm 包」",
                        "npx {} is not an executable npm package (no bin entry). Skills installed via the skills CLI are imported from their install directory automatically; manual import requires an npm package whose bin prints Markdown",
                        package_name
                    ));
                }
                Err(e) => return Err(e),
            }
        }
    };

    // Sanitize content (strip ANSI sequences, null bytes, control characters)
    let content = sanitize_skill_content(&raw_content)?;

    // Try to read metadata from package.json (spawn_blocking avoids blocking an async worker)
    let pkg2 = package_name.clone();
    let (name, description, version) =
        tokio::task::spawn_blocking(move || try_read_package_meta(&pkg2))
            .await
            .unwrap_or_else(|_| (String::new(), String::new(), "0.0.0".to_string()));

    // skills CLI / skill-dir 来源的元数据以安装目录为准（npm registry 上查不到它们）
    let (dir_version, dir_description) = match source_path
        .as_deref()
        .map(std::path::Path::new)
        .map(try_read_package_meta_from_dir)
    {
        Some((v, d)) => (v, d),
        None => (String::new(), String::new()),
    };
    let name = if name.is_empty() && !dir_description.is_empty() {
        package_name.clone()
    } else {
        name
    };
    let description = if description.is_empty() {
        dir_description
    } else {
        description
    };
    let version = if version.is_empty() {
        dir_version
    } else {
        version
    };

    let skill = SkillConfig {
        id: format!("npx-{}", package_name),
        name: if name.is_empty() {
            package_name.clone()
        } else {
            name
        },
        description,
        content,
        source: crate::core::models::SkillSource::Npx,
        source_url: Some(format!("https://www.npmjs.com/package/{}", package_name)),
        source_owner: None,
        source_repo: None,
        source_slug: None,
        installs: 0,
        npx_package: Some(package_name.clone()),
        npx_version: Some(version),
        npx_command: Some(package_name.clone()),
        source_disk_path: source_path.clone(),
    };

    match project_id {
        Some(ref pid) => {
            let project_path = resolve_project_path(&app, pid)?;
            let mut file = load_project_file(&project_path);
            file.skills.insert(skill.id.clone(), skill.clone());
            file.updated_at = now_timestamp();
            save_project_file(&project_path, &file)?;
        }
        None => {
            let mut file = load_file(&app);
            file.skills.insert(skill.id.clone(), skill.clone());
            file.updated_at = now_timestamp();
            save_file(&app, &file)?;
        }
    }
    Ok(skill)
}

/// Refreshes an imported npx skill (re-runs npx to fetch the latest content).
#[tauri::command]
pub async fn refresh_npx_skill(
    app: AppHandle,
    id: String,
    project_id: Option<String>,
) -> Result<SkillConfig, String> {
    // Look up the existing skill
    let existing = match project_id {
        Some(ref pid) => {
            let project_path = resolve_project_path(&app, pid)?;
            load_project_file(&project_path).skills.get(&id).cloned()
        }
        None => load_file(&app).skills.get(&id).cloned(),
    }
    .ok_or_else(|| ierr!("Skill {} 不存在", "Skill {} not found", id))?;

    let pkg_name = existing.npx_package.as_ref().ok_or_else(|| {
        ierr!(
            "该 Skill 不是 npx 来源",
            "This skill is not from an npx source"
        )
    })?;

    // Re-validate the package name in case the stored value was tampered with
    validate_npx_package_name(pkg_name)?;

    // 刷新同样分源：skills CLI / skill-dir 来源（有 source_disk_path）直接从安装
    // 目录重读（其 name 不是 npm 包，npx 执行必然失败）；npm 包来源走 npx 重跑。
    let (raw_content, disk_version) = match existing
        .source_disk_path
        .as_deref()
        .map(std::path::Path::new)
    {
        Some(dir) if dir.is_dir() => {
            let content = read_content_from_skill_dir(dir).ok_or_else(|| {
                ierr!(
                    "安装目录 {} 中未找到 SKILL.md / README.md",
                    "No SKILL.md / README.md found in install dir {}",
                    dir.display()
                )
            })?;
            let (version, _) = try_read_package_meta_from_dir(dir);
            (content, version)
        }
        _ => {
            let pkg = pkg_name.to_string();
            let raw =
                tokio::task::spawn_blocking(move || run_npx_sandboxed(&[&pkg], 30, 64 * 1024))
                    .await
                    .map_err(|_| ierr!("npx 线程 panic", "npx thread panicked"))??
                    .ok_or_else(|| {
                        ierr!(
                            "npx {} 未返回任何内容",
                            "npx {} returned no content",
                            pkg_name
                        )
                    })?;
            (raw, String::new())
        }
    };

    let content = sanitize_skill_content(&raw_content)?;

    let pkg2 = pkg_name.to_string();
    let (_, _, version) = tokio::task::spawn_blocking(move || try_read_package_meta(&pkg2))
        .await
        .unwrap_or_else(|_| (String::new(), String::new(), "0.0.0".to_string()));
    // 磁盘来源的版本以安装目录为准（npm registry 查不到它们）
    let version = if disk_version.is_empty() {
        version
    } else {
        disk_version
    };

    let updated = SkillConfig {
        content,
        npx_version: Some(version),
        ..existing.clone()
    };

    match project_id {
        Some(ref pid) => {
            let project_path = resolve_project_path(&app, pid)?;
            let mut file = load_project_file(&project_path);
            file.skills.insert(id, updated.clone());
            file.updated_at = now_timestamp();
            save_project_file(&project_path, &file)?;
        }
        None => {
            let mut file = load_file(&app);
            file.skills.insert(id, updated.clone());
            file.updated_at = now_timestamp();
            save_file(&app, &file)?;
        }
    }
    Ok(updated)
}

/// Tries to read metadata from the npm global package.json (with a 10s timeout).
fn try_read_package_meta(package_name: &str) -> (String, String, String) {
    use std::io::Read;

    use crate::utils::exec_resolve::HiddenConsole;
    let mut child = match std::process::Command::new(resolve_command("npm"))
        .args(["list", "-g", "--depth=0", "--json"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .hidden_console()
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return (String::new(), String::new(), "0.0.0".to_string()),
    };

    // Wait with a 10s timeout
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut stdout = Vec::new();
        let _ = child.stdout.as_mut().map(|s| s.read_to_end(&mut stdout));
        let status = child.wait();
        let _ = tx.send((status, stdout));
    });

    let (status, stdout): (std::io::Result<std::process::ExitStatus>, Vec<u8>) =
        match rx.recv_timeout(std::time::Duration::from_secs(10)) {
            Ok((s, o)) => (s, o),
            Err(_) => return (String::new(), String::new(), "0.0.0".to_string()),
        };

    if !status.is_ok_and(|s: std::process::ExitStatus| s.success()) {
        return (String::new(), String::new(), "0.0.0".to_string());
    }

    // Cap output at 256KB
    let capped = if stdout.len() > 256 * 1024 {
        &stdout[..256 * 1024]
    } else {
        &stdout
    };
    let json_str = String::from_utf8_lossy(capped);

    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&json_str) {
        if let Some(dep) = json
            .get("dependencies")
            .and_then(|deps| deps.get(package_name))
        {
            let name = dep
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or(package_name)
                .to_string();
            let desc = dep
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let ver = dep
                .get("version")
                .and_then(|v| v.as_str())
                .unwrap_or("0.0.0")
                .to_string();
            return (name, desc, ver);
        }
    }
    (String::new(), String::new(), "0.0.0".to_string())
}

/// Resolves a bare command name (e.g. "npx") to a full path when possible.
/// Delegates to the canonical PATH-only resolver (SEC-06; also handles the Windows
/// PATHEXT walk): bare-name spawns must not consult the current directory.
/// Unresolvable names are returned unchanged so the OS produces its native error.
fn resolve_command(command: &str) -> String {
    crate::utils::exec_resolve::resolve_or_keep(command)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_market_row_and_install_metrics() {
        let html = r#"
        <a class="group grid grid-cols-16" href="/vercel-labs/skills/find-skills">
          <h3 class="font-semibold">find-skills</h3>
          <p class="text-xs">vercel-labs/skills</p>
          <svg aria-label="Weekly installs: 102,724, 112,234"></svg>
          <span class="font-mono text-sm text-foreground">2.1M</span>
        </a>
        "#;
        let skills = parse_market_skills(html);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].owner, "vercel-labs");
        assert_eq!(skills[0].installs, 2_100_000);
        assert_eq!(skills[0].weekly_installs, vec![102_724, 112_234]);
    }

    #[test]
    fn market_cache_expires_after_six_hours() {
        let entry = MarketCacheEntry {
            fetched_at: 1_000,
            html: "cached".to_string(),
        };
        assert!(cache_entry_is_fresh(
            &entry,
            1_000 + MARKET_CACHE_TTL_SECS - 1
        ));
        assert!(!cache_entry_is_fresh(&entry, 1_000 + MARKET_CACHE_TTL_SECS));
    }

    #[test]
    fn installs_label_formats_compact() {
        assert_eq!(format_installs_label(0), "0");
        assert_eq!(format_installs_label(999), "999");
        assert_eq!(format_installs_label(2_236), "2.2k");
        assert_eq!(format_installs_label(210_769), "210.8k");
        assert_eq!(format_installs_label(1_234_567), "1.2M");
    }

    /// skills.sh `/api/search` items map into MarketSkill rows (owner/repo/slug
    /// split, installs compacted, source URL rebuilt).
    #[test]
    fn search_response_maps_to_market_skills() {
        let body = r#"{"query":"pdf","skills":[{"id":"anthropics/skills/pdf","name":"pdf","installs":210769},{"id":"claude-office-skills/skills/pdf-ocr-extraction","name":"PDF OCR Extraction","installs":5436},{"id":"malformed-no-slash","name":"x","installs":1}]}"#;
        let value: Value = serde_json::from_str(body).unwrap();
        let skills = map_search_response(&value);
        assert_eq!(skills.len(), 2, "畸形 id 必须被丢弃");
        assert_eq!(skills[0].owner, "anthropics");
        assert_eq!(skills[0].repo, "skills");
        assert_eq!(skills[0].slug, "pdf");
        assert_eq!(skills[0].installs_label, "210.8k");
        assert_eq!(
            skills[0].source_url,
            "https://www.skills.sh/anthropics/skills/pdf"
        );
        assert_eq!(skills[1].slug, "pdf-ocr-extraction");
    }
}
