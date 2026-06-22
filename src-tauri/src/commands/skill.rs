//! Skill 配置管理命令。

use crate::core::models::{MarketSkill, SkillConfig, SkillMarketCategory, SkillsFile};
use regex::Regex;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
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
    app.path()
        .app_data_dir()
        .map(|dir| dir.join(SKILLS_FILE))
        .map_err(|e| e.to_string())
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
        return SkillsFile::default();
    };
    std::fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default()
}

fn save_file(app: &AppHandle, file: &SkillsFile) -> Result<(), String> {
    let path = skills_file_path(app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let content = serde_json::to_string_pretty(file).map_err(|e| e.to_string())?;
    std::fs::write(path, content).map_err(|e| e.to_string())
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

/// 返回全部 Skill 配置。
#[tauri::command]
pub fn list_skills(app: AppHandle) -> Result<Vec<SkillConfig>, String> {
    Ok(load_file(&app).skills.into_values().collect())
}

/// 新增或更新一个 Skill。id、名称和指令内容不能为空。
#[tauri::command]
pub fn save_skill(app: AppHandle, skill: SkillConfig) -> Result<(), String> {
    if skill.id.trim().is_empty() || skill.name.trim().is_empty() || skill.content.trim().is_empty() {
        return Err("Skill id、名称和指令内容不能为空".to_string());
    }
    let mut file = load_file(&app);
    file.skills.insert(skill.id.clone(), skill);
    file.updated_at = now_timestamp();
    save_file(&app, &file)
}

/// 删除指定 Skill 配置。
#[tauri::command]
pub fn delete_skill(app: AppHandle, id: String) -> Result<(), String> {
    let mut file = load_file(&app);
    file.skills.remove(&id);
    file.updated_at = now_timestamp();
    save_file(&app, &file)
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

/// 从 skills.sh 详情页下载 Skill 内容并保存到本地 Skill 库。
#[tauri::command]
pub async fn download_market_skill(
    app: AppHandle,
    owner: String,
    repo: String,
    slug: String,
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
        source_url: Some(source_url),
        source_owner: Some(owner),
        source_repo: Some(repo),
        source_slug: Some(slug),
        installs,
    };
    let mut file = load_file(&app);
    file.skills.insert(skill.id.clone(), skill.clone());
    file.updated_at = now_timestamp();
    save_file(&app, &file)?;
    Ok(skill)
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
