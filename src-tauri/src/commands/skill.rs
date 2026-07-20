//! Skill 配置管理命令。

use crate::core::models::{MarketSkill, ProjectsFile, SkillConfig, SkillMarketCategory, SkillsFile};
use regex::Regex;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

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
    let dir = app.path()
        .app_data_dir()
        .map_err(|e| {
            let msg = format!("app_data_dir 解析失败: {e}");
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
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let content = serde_json::to_string(cache).map_err(|e| e.to_string())?;
    std::fs::write(path, content).map_err(|e| e.to_string())
}

fn load_file(app: &AppHandle) -> SkillsFile {
    let Ok(path) = skills_file_path(app) else {
        tracing::warn!("[skills] load_file: 路径解析失败，返回空配置");
        return SkillsFile::default();
    };
    if !path.exists() {
        tracing::info!("[skills] load_file: 文件不存在 ({})，返回空配置", path.display());
        return SkillsFile::default();
    }
    match std::fs::read_to_string(&path) {
        Ok(content) => match serde_json::from_str::<SkillsFile>(&content) {
            Ok(file) => {
                tracing::info!("[skills] load_file: 加载 {} 条 skill (from {})", file.skills.len(), path.display());
                file
            }
            Err(e) => {
                tracing::error!("[skills] load_file: JSON 解析失败 ({}): {e}", path.display());
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
            let msg = format!("创建目录失败: {e}");
            tracing::error!("[skills] save_file: {msg} ({})", parent.display());
            msg
        })?;
    }
    let content = serde_json::to_string_pretty(file).map_err(|e| {
        let msg = format!("序列化 skills 失败: {e}");
        tracing::error!("[skills] save_file: {msg}");
        msg
    })?;
    std::fs::write(&path, &content).map_err(|e| {
        let msg = format!("写入文件失败: {e}");
        tracing::error!("[skills] save_file: {msg} ({})", path.display());
        msg
    })?;
    tracing::info!("[skills] save_file: 保存 {} 条 skill → {}", file.skills.len(), path.display());
    Ok(())
}

// ====== 项目级 Skill 文件操作 ======

/// 加载项目级 skills.json（不存在则返回空）。
fn load_project_file(project_path: &str) -> SkillsFile {
    let path = crate::commands::project::project_skills_path(project_path);
    if !path.exists() {
        tracing::info!("[skills] load_project_file: 文件不存在 ({})，返回空配置", path.display());
        return SkillsFile::default();
    }
    match std::fs::read_to_string(&path) {
        Ok(content) => match serde_json::from_str::<SkillsFile>(&content) {
            Ok(file) => {
                tracing::info!("[skills] load_project_file: 加载 {} 条 skill (from {})", file.skills.len(), path.display());
                file
            }
            Err(e) => {
                tracing::error!("[skills] load_project_file: JSON 解析失败 ({}): {e}", path.display());
                SkillsFile::default()
            }
        },
        Err(e) => {
            tracing::error!("[skills] load_project_file: 读取文件失败 ({}): {e}", path.display());
            SkillsFile::default()
        }
    }
}

/// 保存项目级 skills.json。
fn save_project_file(project_path: &str, file: &SkillsFile) -> Result<(), String> {
    let path = crate::commands::project::project_skills_path(project_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            let msg = format!("创建目录失败: {e}");
            tracing::error!("[skills] save_project_file: {msg} ({})", parent.display());
            msg
        })?;
    }
    let content = serde_json::to_string_pretty(file).map_err(|e| {
        let msg = format!("序列化 skills 失败: {e}");
        tracing::error!("[skills] save_project_file: {msg}");
        msg
    })?;
    std::fs::write(&path, &content).map_err(|e| {
        let msg = format!("写入文件失败: {e}");
        tracing::error!("[skills] save_project_file: {msg} ({})", path.display());
        msg
    })?;
    tracing::info!("[skills] save_project_file: 保存 {} 条 skill → {}", file.skills.len(), path.display());
    Ok(())
}

/// 通过 project_id 解析项目路径。
fn resolve_project_path(app: &AppHandle, project_id: &str) -> Result<String, String> {
    let file = load_projects_index(app);
    file.projects
        .get(project_id)
        .map(|p| p.path.clone())
        .ok_or_else(|| format!("项目 {} 不存在", project_id))
}

/// 加载项目索引文件。
fn load_projects_index(app: &AppHandle) -> ProjectsFile {
    let Ok(path) = app
        .path()
        .app_data_dir()
        .map(|dir| dir.join("projects.json"))
    else {
        return ProjectsFile::default();
    };
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default()
}

/// 合并全局和项目 Skill：项目同 ID 覆盖全局。
fn merge_skills(global: BTreeMap<String, SkillConfig>, project: BTreeMap<String, SkillConfig>) -> BTreeMap<String, SkillConfig> {
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
        .map_err(|e| format!("请求 skills.sh 失败: {}", e))?;
    if !response.status().is_success() {
        return Err(format!("skills.sh 返回 HTTP {}", response.status()));
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
            // 网络不可用时允许使用过期缓存，保证市场页仍可浏览。
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
    let numeric = Regex::new(r"&#(?:x([0-9A-Fa-f]+)|([0-9]+));").expect("valid numeric entity regex");
    output = numeric
        .replace_all(&output, |caps: &regex::Captures<'_>| {
            let parsed = caps
                .get(1)
                .and_then(|value| u32::from_str_radix(value.as_str(), 16).ok())
                .or_else(|| caps.get(2).and_then(|value| value.as_str().parse::<u32>().ok()));
            parsed.and_then(char::from_u32).map(|c| c.to_string()).unwrap_or_default()
        })
        .into_owned();
    output
}

fn strip_html(value: &str) -> String {
    let block_tags = Regex::new(r"(?i)</?(?:p|div|h[1-6]|li|ul|ol|pre|table|tr|blockquote|br)[^>]*>")
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
    number.parse::<f64>().map(|value| (value * multiplier) as u64).unwrap_or(0)
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
    let installs_re = Regex::new(
        r#"<span class="font-mono text-sm text-foreground">([^<]+)</span>"#,
    )
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
                category: None,
            })
        })
        .collect()
}

fn parse_topic_skill_descriptions(html: &str) -> HashMap<String, String> {
    let anchor = Regex::new(
        r#"(?s)<a class="group grid[^"]*" href="/([^/"]+/[^/"]+/[^/"]+)">(.*?)</a>"#,
    )
    .expect("valid topic skill regex");
    let description = Regex::new(r#"(?s)<p class="lg:col-span-9[^"]*">(.*?)</p>"#)
        .expect("valid topic description regex");
    anchor
        .captures_iter(html)
        .filter_map(|capture| {
            let path = capture.get(1)?.as_str().to_string();
            let body = capture.get(2)?.as_str();
            let text = description
                .captures(body)
                .and_then(|inner| inner.get(1))
                .map(|value| strip_html(value.as_str()))
                .unwrap_or_default();
            Some((path, text))
        })
        .collect()
}

fn parse_categories(html: &str) -> Vec<SkillMarketCategory> {
    let category = Regex::new(r#"(?s)<a[^>]*href="/topic/([^"]+)"[^>]*>(.*?)</a>"#)
        .expect("valid category regex");
    let name_re = Regex::new(r#"(?s)<h2[^>]*>(.*?)</h2>"#).expect("valid category name regex");
    let paragraphs = Regex::new(r#"(?s)<p[^>]*>(.*?)</p>"#).expect("valid category paragraph regex");
    let count_re = Regex::new(r"([0-9]+)").expect("valid category count regex");
    category
        .captures_iter(html)
        .filter_map(|capture| {
            let body = capture.get(2)?.as_str();
            let name = name_re
                .captures(body)
                .and_then(|inner| inner.get(1))
                .map(|value| strip_html(value.as_str()))?;
            let paragraph_values: Vec<String> = paragraphs
                .captures_iter(body)
                .filter_map(|inner| inner.get(1).map(|value| strip_html(value.as_str())))
                .collect();
            let description = paragraph_values.first().cloned().unwrap_or_default();
            let skill_count = paragraph_values
                .get(1)
                .and_then(|value| count_re.captures(value))
                .and_then(|inner| inner.get(1))
                .and_then(|value| value.as_str().parse().ok())
                .unwrap_or(0);
            Some(SkillMarketCategory {
                id: capture.get(1)?.as_str().to_string(),
                name: name.replace(" skills", ""),
                description,
                skill_count,
            })
        })
        .collect()
}

/// 返回全部 Skill 配置。project_id 存在时合并全局 + 项目（项目优先）。
#[tauri::command]
pub fn list_skills(app: AppHandle, project_id: Option<String>) -> Result<Vec<SkillConfig>, String> {
    let global = load_file(&app).skills;
    let project = match project_id {
        Some(ref pid) => {
            let project_path = resolve_project_path(&app, pid)?;
            load_project_file(&project_path).skills
        }
        None => BTreeMap::new(),
    };
    Ok(merge_skills(global, project).into_values().collect())
}

/// 新增或更新一个 Skill。project_id 存在时写入项目级文件。
#[tauri::command]
pub fn save_skill(app: AppHandle, skill: SkillConfig, project_id: Option<String>) -> Result<(), String> {
    if skill.id.trim().is_empty() || skill.name.trim().is_empty() || skill.content.trim().is_empty() {
        return Err("Skill id、名称和指令内容不能为空".to_string());
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
}

/// 删除指定 Skill 配置。project_id 存在时从项目级文件删除。
#[tauri::command]
pub fn delete_skill(app: AppHandle, id: String, project_id: Option<String>) -> Result<(), String> {
    match project_id {
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
    }
}

/// 返回 skills.sh 官方分类。
#[tauri::command]
pub async fn list_skill_market_categories(
    app: AppHandle,
    force_refresh: bool,
) -> Result<Vec<SkillMarketCategory>, String> {
    let html = fetch_market_page_cached(&app, "/topic", force_refresh).await?;
    Ok(parse_categories(&html))
}

/// 浏览 skills.sh 市场。sort 支持 all、trending、hot；category 可选。
#[tauri::command]
pub async fn list_skill_market(
    app: AppHandle,
    sort: String,
    category: Option<String>,
    force_refresh: bool,
) -> Result<Vec<MarketSkill>, String> {
    let path = match sort.as_str() {
        "trending" => "/trending",
        "hot" => "/hot",
        _ => "/",
    };
    let mut skills = parse_market_skills(
        &fetch_market_page_cached(&app, path, force_refresh).await?,
    );
    if let Some(category_id) = category.filter(|value| !value.is_empty() && value != "all") {
        let topic_html = fetch_market_page_cached(
            &app,
            &format!("/topic/{}", category_id),
            force_refresh,
        )
        .await?;
        let topic_skills = parse_topic_skill_descriptions(&topic_html);
        skills.retain_mut(|skill| {
            let path = format!("{}/{}/{}", skill.owner, skill.repo, skill.slug);
            if let Some(description) = topic_skills.get(&path) {
                skill.category = Some(category_id.clone());
                if skill.description.is_empty() {
                    skill.description = description.clone();
                }
                true
            } else {
                false
            }
        });
    }
    Ok(skills)
}

/// 从 skills.sh 详情页下载 Skill 内容并保存到 Skill 库。project_id 存在时写入项目级。
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
        return Err("非法的 Skill 路径".to_string());
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
    let marker_index = html.find(marker).ok_or_else(|| "详情页缺少 SKILL.md".to_string())?;
    let prose_start = html[marker_index..]
        .find("<div class=\"prose ")
        .map(|index| marker_index + index)
        .ok_or_else(|| "无法定位 Skill 内容".to_string())?;
    let content_start = html[prose_start..]
        .find('>')
        .map(|index| prose_start + index + 1)
        .ok_or_else(|| "无法解析 Skill 内容".to_string())?;
    let content_end = html[content_start..]
        .find("<div class=\"relative\">")
        .map(|index| content_start + index)
        .or_else(|| html[content_start..].find("<section class=\"mt-16\">").map(|index| content_start + index))
        .ok_or_else(|| "无法确定 Skill 内容边界".to_string())?;
    let content = strip_html(&html[content_start..content_end]);
    if content.trim().is_empty() {
        return Err("下载到的 Skill 内容为空".to_string());
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

// ====== npx Skill 发现与导入 ======

/// 验证 npx 包名是否合法。
///
/// 规则：
/// - 符合 npm 命名规范：`@scope/name` 或 `name`，仅含小写字母、数字、`-`、`_`、`.`
/// - 总长度 ≤ 214 字符
/// - 不含 shell 元字符
/// - 不在高危黑名单中
fn validate_npx_package_name(package_name: &str) -> Result<(), String> {
    // 拒绝空
    if package_name.trim().is_empty() {
        return Err("包名不能为空".into());
    }

    // 拒绝路径穿越
    if package_name.contains("..") {
        return Err("包名包含非法字符 '..'".into());
    }

    // 拒绝 shell 元字符
    for ch in package_name.chars() {
        if matches!(ch, ';' | '|' | '&' | '$' | '`' | '\\' | '\'' | '"' | '<' | '>' | '!' | '\n' | '\r' | '\t') {
            return Err(format!("包名包含禁止字符: '{}'", ch));
        }
    }

    // npm 命名规范: @scope/name 或 name
    let name = if let Some(rest) = package_name.strip_prefix('@') {
        // scoped package: @scope/name
        let parts: Vec<&str> = rest.splitn(2, '/').collect();
        if parts.len() != 2 {
            return Err("scoped 包名格式错误: 应为 @scope/name".into());
        }
        let scope = parts[0];
        let name_part = parts[1];
        if scope.is_empty() || name_part.is_empty() {
            return Err("scoped 包名格式错误: scope 或 name 为空".into());
        }
        // 验证 scope
        if !scope.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_' || c == '.') {
            return Err("scoped 包名 scope 部分包含非法字符".into());
        }
        name_part
    } else {
        package_name
    };

    // 验证 name 部分
    if name.is_empty() || name.len() > 214 {
        return Err(format!("包名长度不合法: {} 字符（最大 214）", name.len()));
    }
    if !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_' || c == '.') {
        return Err("包名包含非法字符（仅允许小写字母、数字、-、_、.）".into());
    }

    // 拒绝伪装成 Node.js 内置模块
    let node_builtins = [
        "child_process", "fs", "os", "path", "process", "buffer",
        "stream", "net", "tls", "http", "https", "dns", "dgram",
        "crypto", "util", "events", "assert", "vm", "v8",
        "worker_threads", "cluster", "module",
    ];
    let lower_name = name.to_lowercase();
    let base_name = if let Some(pos) = lower_name.rfind('/') { &lower_name[pos + 1..] } else { &lower_name };
    if node_builtins.contains(&base_name) {
        return Err(format!("包名 '{}' 伪装为 Node.js 内置模块，拒绝导入", package_name));
    }

    // 拒绝 git 协议 URL 伪装
    if package_name.starts_with("git://") || package_name.starts_with("git+") || package_name.starts_with("ssh://") {
        return Err("包名不是合法的 npm 包名".into());
    }

    Ok(())
}

/// 系统上检测到的 npx skill 包信息。
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

/// 扫描系统上已安装的 npx skill 包。
#[tauri::command]
pub async fn discover_npx_skills(app: AppHandle) -> Result<Vec<NpxSkillInfo>, String> {
    let mut discovered: Vec<NpxSkillInfo> = Vec::new();
    let existing = load_file(&app).skills;
    tracing::info!("[skills] discover_npx: 当前已导入 {} 条 skill（用于 already_imported 标记）", existing.len());

    // 1) 主要方式：通过 `npx skills list --json` 获取已安装 skill 列表
    tracing::info!("[skills] discover_npx: 正在执行 npx --yes skills list -g --json ...");

    // 使用 tokio spawn_blocking 避免阻塞 async worker 线程
    let skills_cli_result = tokio::task::spawn_blocking(move || {
        let npx_cmd = resolve_command("npx");
        let child = match std::process::Command::new(&npx_cmd)
            .args(["--yes", "skills", "list", "-g", "--json"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => return Err(format!("启动 {} 失败: {}", npx_cmd, e)),
        };

        // Windows: 分配到限制性 Job Object
        if let Err(e) = crate::utils::sandbox::assign_to_job(&child) {
            tracing::warn!("[skills] discover npx sandbox 分配失败: {e}");
        }

        let output = child
            .wait_with_output()
            .map_err(|e| format!("等待 npx 进程失败: {e}"))?;

        // 截断输出到 1MB
        let stdout = if output.stdout.len() > 1_048_576 {
            tracing::warn!("[skills] discover npx stdout 过大 ({}B)，已截断", output.stdout.len());
            output.stdout[..1_048_576].to_vec()
        } else {
            output.stdout
        };
        Ok((output.status.success(), String::from_utf8_lossy(&stdout).to_string(), String::from_utf8_lossy(&output.stderr).to_string()))
    })
    .await
    .unwrap_or_else(|_| Err("npx 线程 panic".to_string()));

    // 处理 npx skills list 结果
    match skills_cli_result {
        Ok((success, stdout, stderr)) => {
            if success {
                match serde_json::from_str::<Vec<serde_json::Value>>(&stdout) {
                    Ok(json_list) => {
                        tracing::info!("[skills] discover_npx: skills CLI 返回 {} 条 skill", json_list.len());
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
                            tracing::info!("[skills] discover_npx:   - {} (v{}, already_imported={})", name, version, already);
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
                        tracing::warn!("[skills] discover_npx: stdout 前 200 字符: {}", &stdout.chars().take(200).collect::<String>());
                    }
                }
            } else {
                tracing::warn!("[skills] discover_npx: skills CLI 退出码非 0 (stderr: {})", stderr.trim());
            }
        }
        Err(e) => {
            tracing::warn!("[skills] discover_npx: npx 启动失败: {e}");
        }
    }

    // 如果 skills CLI 没有返回结果，回退到目录扫描
    if discovered.is_empty() {
        tracing::info!("[skills] discover_npx: skills CLI 无结果，回退到目录扫描");
        // 2) 扫描已知的 skill 注册目录：~/.claude/skills/ 和 ~/.agents/skills/
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

        // 3) 扫描全局 npm 包中带有 skill 特征的包（回退方案）
        let npm_output = std::process::Command::new("npm")
            .args(["list", "-g", "--depth=0", "--json"])
            .output();
        if let Ok(output) = npm_output {
            if output.status.success() {
                if let Ok(json) =
                    serde_json::from_str::<serde_json::Value>(&String::from_utf8_lossy(&output.stdout))
                {
                    if let Some(deps) = json.get("dependencies").and_then(|v| v.as_object()) {
                        for (name, info) in deps {
                            let is_skill_pkg = name.contains("skill")
                                || name.starts_with("@anthropic-ai/skill-")
                                || name.starts_with("@claude/");
                            if !is_skill_pkg {
                                continue;
                            }
                            let version = info["version"].as_str().unwrap_or("0.0.0").to_string();
                            let description = info["description"].as_str().unwrap_or("").to_string();
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

    // 按名称排序
    discovered.sort_by(|a, b| a.package_name.cmp(&b.package_name));
    tracing::info!("[skills] discover_npx: 最终发现 {} 条 npx skill", discovered.len());
    Ok(discovered)
}

/// 从 skill 目录的 package.json 读取版本和描述。
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

/// 从 skill 目录的 package.json 读取版本和描述（用于 skills CLI JSON 结果）。
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

// ====== npx Skill 执行安全层 ======

/// 带超时和输出限制的 npx 子进程执行。
///
/// # 参数
/// - `args` — 传递给 npx 的参数（不含 "npx" 本身）
/// - `timeout_secs` — 超时（秒），超时后强制 kill 进程
/// - `max_output_bytes` — stdout 最大允许字节数
///
/// # 返回
/// Ok(Some(stdout)) — 执行成功且有输出
/// Ok(None) — 执行成功但输出为空
/// Err(msg) — 启动失败、非零退出码、或超时
fn run_npx_sandboxed(
    args: &[&str],
    timeout_secs: u64,
    max_output_bytes: usize,
) -> Result<Option<String>, String> {
    let npx_cmd = resolve_command("npx");
    let child = std::process::Command::new(&npx_cmd)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("启动 {} 失败: {}", npx_cmd, e))?;

    // Windows: 分配到限制性 Job Object
    if let Err(e) = crate::utils::sandbox::assign_to_job(&child) {
        tracing::warn!("[skills] npx sandbox 分配失败: {e}");
    }

    // 带超时的等待
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let output = child.wait_with_output();
        let _ = tx.send(output);
    });

    let output = rx
        .recv_timeout(std::time::Duration::from_secs(timeout_secs))
        .map_err(|_| format!("npx 命令超时（>{timeout_secs}s），已终止"))?
        .map_err(|e| format!("等待 npx 进程失败: {e}"))?;

    // 记录 stderr 用于审计
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !stderr.is_empty() {
        let stderr_short: String = stderr.chars().take(300).collect();
        tracing::info!("[skills] npx stderr: {}", stderr_short);
    }

    if !output.status.success() {
        let stderr_short: String = stderr.chars().take(300).collect();
        return Err(format!("npx 退出码 {}: {}", output.status, stderr_short));
    }

    // 截断输出
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

/// 清洗 npx stdout 内容后再存入 SkillConfig。
///
/// 去除 ANSI 转义序列、null 字节，验证 UTF-8 有效性，
/// 并检查是否为纯二进制内容。
fn sanitize_skill_content(raw: &str) -> Result<String, String> {
    // 1. 去除 ANSI 转义序列（CSI 序列: ESC [ ... m）
    let re_ansi = regex::Regex::new("\x1b\\[[0-9;]*[a-zA-Z]").unwrap();
    let mut cleaned = re_ansi.replace_all(raw, "").to_string();

    // 2. 去除 null 字节
    cleaned = cleaned.replace('\0', "");

    // 3. 去除其他控制字符（保留换行、制表符）
    cleaned = cleaned
        .chars()
        .filter(|c| *c == '\n' || *c == '\r' || *c == '\t' || !c.is_control())
        .collect();

    // 4. 修剪首尾空白
    cleaned = cleaned.trim().to_string();

    if cleaned.is_empty() {
        return Err("清洗后内容为空（可能为纯控制字符）".into());
    }

    // 5. 检查前 512 字节中可打印字符比例（拒绝二进制 blob）
    let sample = if cleaned.len() > 512 { &cleaned[..512] } else { &cleaned };
    let printable_count = sample.chars().filter(|c| c.is_ascii_graphic() || c.is_whitespace()).count();
    if sample.len() > 0 && (printable_count as f64 / sample.len() as f64) < 0.5 {
        return Err("内容看似为二进制数据，拒绝导入".into());
    }

    Ok(cleaned)
}

/// 导入一个 npx skill 包：执行 npx 获取内容，保存到 Skill 池。
#[tauri::command]
pub async fn import_npx_skill(
    app: AppHandle,
    package_name: String,
    project_id: Option<String>,
) -> Result<SkillConfig, String> {
    // 包名校验
    validate_npx_package_name(&package_name)?;

    // 沙箱执行 npx <package> 获取 skill 内容（spawn_blocking 避免阻塞 async worker）
    let pkg = package_name.clone();
    let raw_content = tokio::task::spawn_blocking(move || {
        run_npx_sandboxed(&[&pkg], 30, 64 * 1024)
    })
    .await
    .map_err(|_| "npx 线程 panic".to_string())??
    .ok_or_else(|| format!("npx {} 未返回任何内容", package_name))?;

    // 清洗内容（去除 ANSI 序列、null 字节、控制字符）
    let content = sanitize_skill_content(&raw_content)?;

    // 尝试从 package.json 获取元数据（spawn_blocking 避免阻塞 async worker）
    let pkg2 = package_name.clone();
    let (name, description, version) = tokio::task::spawn_blocking(move || {
        try_read_package_meta(&pkg2)
    })
    .await
    .unwrap_or_else(|_| (String::new(), String::new(), "0.0.0".to_string()));

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

/// 刷新一个已导入的 npx skill（重新执行 npx 拉取最新内容）。
#[tauri::command]
pub async fn refresh_npx_skill(
    app: AppHandle,
    id: String,
    project_id: Option<String>,
) -> Result<SkillConfig, String> {
    // 查找已有 skill
    let existing = match project_id {
        Some(ref pid) => {
            let project_path = resolve_project_path(&app, pid)?;
            load_project_file(&project_path).skills.get(&id).cloned()
        }
        None => load_file(&app).skills.get(&id).cloned(),
    }
    .ok_or_else(|| format!("Skill {} 不存在", id))?;

    let pkg_name = existing
        .npx_package
        .as_ref()
        .ok_or_else(|| "该 Skill 不是 npx 来源".to_string())?;

    // 包名校验（重新验证，防止存储的包名被篡改）
    validate_npx_package_name(pkg_name)?;

    // 沙箱刷新执行（spawn_blocking 避免阻塞 async worker）
    let pkg = pkg_name.to_string();
    let raw_content = tokio::task::spawn_blocking(move || {
        run_npx_sandboxed(&[&pkg], 30, 64 * 1024)
    })
    .await
    .map_err(|_| "npx 线程 panic".to_string())??
    .ok_or_else(|| format!("npx {} 未返回任何内容", pkg_name))?;

    let content = sanitize_skill_content(&raw_content)?;

    let pkg2 = pkg_name.to_string();
    let (_, _, version) = tokio::task::spawn_blocking(move || {
        try_read_package_meta(&pkg2)
    })
    .await
    .unwrap_or_else(|_| (String::new(), String::new(), "0.0.0".to_string()));

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

/// 尝试读取 npm 全局包的 package.json 获取元数据（带 10s 超时）。
fn try_read_package_meta(package_name: &str) -> (String, String, String) {
    use std::io::Read;

    let mut child = match std::process::Command::new("npm")
        .args(["list", "-g", "--depth=0", "--json"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return (String::new(), String::new(), "0.0.0".to_string()),
    };

    // 带超时等待（10s）
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut stdout = Vec::new();
        let _ = child.stdout.as_mut().map(|s| s.read_to_end(&mut stdout));
        let status = child.wait();
        let _ = tx.send((status, stdout));
    });

    let (status, stdout): (std::io::Result<std::process::ExitStatus>, Vec<u8>) = match rx.recv_timeout(std::time::Duration::from_secs(10)) {
        Ok((s, o)) => (s, o),
        Err(_) => return (String::new(), String::new(), "0.0.0".to_string()),
    };

    if !status.map_or(false, |s: std::process::ExitStatus| s.success()) {
        return (String::new(), String::new(), "0.0.0".to_string());
    }

    // 输出上限 256KB
    let capped = if stdout.len() > 256 * 1024 { &stdout[..256 * 1024] } else { &stdout };
    let json_str = String::from_utf8_lossy(capped);

    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&json_str) {
        if let Some(dep) = json
            .get("dependencies")
            .and_then(|deps| deps.get(package_name))
        {
            let name = dep.get("name").and_then(|v| v.as_str()).unwrap_or(package_name).to_string();
            let desc = dep.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let ver = dep.get("version").and_then(|v| v.as_str()).unwrap_or("0.0.0").to_string();
            return (name, desc, ver);
        }
    }
    (String::new(), String::new(), "0.0.0".to_string())
}

/// Windows: 将裸命令名（如 "npx"）解析为完整路径（如 "C:\...\npx.cmd"）。
/// Process::Command 不会按 PATHEXT 扩展名搜索，需要手动处理。
/// 非 Windows 直接返回原命令名。
fn resolve_command(command: &str) -> String {
    #[cfg(windows)]
    {
        use std::path::{Path, PathBuf};
        let p = Path::new(command);
        if p.extension().is_some() {
            return command.to_string();
        }
        let file_name = match p.file_name() {
            Some(f) => f.to_string_lossy(),
            None => return command.to_string(),
        };
        let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT;.COM".to_string());
        let exts: Vec<&str> = pathext.split(';').filter(|s| !s.is_empty()).collect();
        let dirs: Vec<PathBuf> = match p.parent() {
            Some(d) if !d.as_os_str().is_empty() => vec![d.to_path_buf()],
            _ => std::env::var("PATH")
                .unwrap_or_default()
                .split(';')
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .collect(),
        };
        for dir in dirs {
            for ext in &exts {
                let candidate = dir.join(format!("{}{}", file_name, ext));
                if candidate.is_file() {
                    return candidate.to_string_lossy().into_owned();
                }
            }
        }
    }
    command.to_string()
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
    fn parses_topic_categories() {
        let html = r#"
        <a href="/topic/react"><h2>Frontend &amp; React skills</h2>
          <p>React production guidance.</p><p>8<!-- --> skills</p>
        </a>
        "#;
        let categories = parse_categories(html);
        assert_eq!(categories.len(), 1);
        assert_eq!(categories[0].id, "react");
        assert_eq!(categories[0].name, "Frontend & React");
        assert_eq!(categories[0].skill_count, 8);
    }

    #[test]
    fn market_cache_expires_after_six_hours() {
        let entry = MarketCacheEntry {
            fetched_at: 1_000,
            html: "cached".to_string(),
        };
        assert!(cache_entry_is_fresh(&entry, 1_000 + MARKET_CACHE_TTL_SECS - 1));
        assert!(!cache_entry_is_fresh(&entry, 1_000 + MARKET_CACHE_TTL_SECS));
    }
}
