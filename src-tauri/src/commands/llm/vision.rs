//! Auto vision analysis: the vision subagent's "look at the image" call.
//!
//! When the user sends images to a model without vision capability, the
//! frontend invokes this command BEFORE the main-model request: a
//! vision-capable model describes the images, and the description replaces
//! the raw image parts in the conversation (a non-vision model would
//! otherwise ignore the base64 payloads or reject the request outright).

// 阶段4 第1项（消 glob）：显式 use 取代原父模块 glob。
use crate::commands::llm::client::{
    chat_error_message, chat_text_content, non_streaming_http_client_pinned, post_chat_completion,
};

use crate::core::state::DbState;
use crate::ierr;
use base64::engine::general_purpose;
use base64::Engine as _;
use serde_json::json;
use tauri::{AppHandle, Manager};

const VISION_SYSTEM_ZH: &str = "你是 AIO 的视觉分析智能体。主对话模型不支持图像输入，由你代为查看用户发送的图片。请输出结构化的描述，不要臆造图中不存在的信息。";

const VISION_SYSTEM_EN: &str = "You are the AIO vision analysis agent. The main conversation model cannot process images, so you look at the user's images on its behalf. Output a structured description; do not invent anything not present in the images.";

/// Analyze user image attachments with a vision-capable model.
///
/// # Parameters
/// - `api_url` / `api_key` / `model` — endpoint of the vision-capable model
/// - `attachment_ids` — attachment ids; image/* entries are loaded from the
///   DB and base64-encoded into the request (non-image ids are skipped)
/// - `user_text` — the user's message text, so the analysis stays relevant
///   to what was actually asked
/// - `locale` — "en-US" analyzes in English, otherwise Chinese
///
/// # Returns
/// The structured image description text (replaces the images for the
/// non-vision main model). Non-streaming, 45s timeout.
#[tauri::command]
pub async fn vision_analyze_images(
    app: AppHandle,
    api_url: String,
    api_key: String,
    model: String,
    attachment_ids: Vec<String>,
    user_text: String,
    locale: Option<String>,
) -> Result<String, String> {
    // SSRF protection (same policy as the other LLM calls)
    crate::utils::url_validation::validate_http_url(
        &api_url,
        &crate::utils::url_validation::HttpUrlOptions::local_engine(),
    )?;

    // Load image attachments (mirrors message_for_api's attachment path).
    // Scoped so the connection guard is dropped before the first await —
    // a guard alive across an await would make the command future !Send.
    let (image_parts, names): (Vec<serde_json::Value>, Vec<String>) = {
        let state = app.state::<DbState>();
        let conn = state.0.lock();
        let mut parts: Vec<serde_json::Value> = Vec::new();
        let mut file_names: Vec<String> = Vec::new();
        for id in &attachment_ids {
            let row = conn
                .query_row(
                    "SELECT file_name, mime_type, storage_path FROM attachments WHERE id = ?1",
                    [id.as_str()],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    },
                )
                .map_err(|e| {
                    ierr!(
                        "读取附件 {} 失败: {}",
                        "Failed to read attachment {}: {}",
                        id,
                        e
                    )
                })?;
            if !row.1.starts_with("image/") {
                continue;
            }
            let bytes = std::fs::read(&row.2).map_err(|e| {
                ierr!(
                    "读取图片附件 {} 失败: {}",
                    "Failed to read image attachment {}: {}",
                    row.0,
                    e
                )
            })?;
            parts.push(json!({
                "type": "image_url",
                "image_url": { "url": format!("data:{};base64,{}", row.1, general_purpose::STANDARD.encode(bytes)) }
            }));
            file_names.push(row.0);
        }
        (parts, file_names)
    };
    if image_parts.is_empty() {
        return Err(ierr!(
            "附件中没有图片",
            "No images found in the attachments"
        ));
    }

    let is_en = locale.as_deref() == Some("en-US");
    let user_label = if is_en {
        "User message"
    } else {
        "用户消息"
    };
    let instruction = if is_en {
        format!(
            "The images below were sent by the user ({}). The main model cannot see them, so describe everything relevant: the scene and main subjects, any visible text (transcribe verbatim), charts/tables with their data, UI elements, and details related to the user's question. Output a structured description in English.",
            names.join(", ")
        )
    } else {
        format!(
            "以下是用户发送的图片（{}）。主模型无法查看这些图片，请描述其中所有关键内容：场景与主体、可见文字（逐字转录）、图表与其中的数据、界面元素、以及与用户问题直接相关的细节。输出结构化的中文描述。",
            names.join("、")
        )
    };
    let text_part = if user_text.trim().is_empty() {
        instruction
    } else {
        format!("{}\n\n{}：{}", instruction, user_label, user_text)
    };
    let mut parts = vec![json!({ "type": "text", "text": text_part })];
    parts.extend(image_parts);

    let body = json!({
        "model": model,
        "messages": [
            { "role": "system", "content": if is_en { VISION_SYSTEM_EN } else { VISION_SYSTEM_ZH } },
            { "role": "user", "content": serde_json::Value::Array(parts) }
        ],
        "stream": false,
        "max_tokens": 1024
    });

    // SEC-7: pinned resolution (key-bearing request; local/LAN allowed, metadata refused)
    let client = non_streaming_http_client_pinned(&api_url).await?;
    let (res, is_anthropic) = tokio::time::timeout(
        std::time::Duration::from_secs(45),
        post_chat_completion(&client, &api_url, &api_key, &body, None),
    )
    .await
    .map_err(|_| {
        ierr!(
            "视觉分析请求超时（45s）",
            "Vision analysis request timed out (45s)"
        )
    })?
    .map_err(|e| e.to_string())?;

    let val: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;
    if let Some(err) = chat_error_message(&val) {
        return Err(err);
    }
    let text = chat_text_content(&val, is_anthropic);
    if text.is_empty() {
        return Err(ierr!(
            "视觉分析结果为空",
            "Vision analysis returned an empty result"
        ));
    }
    Ok(text)
}

// ====== 电脑控制的视觉路由（截图描述用） ======

use crate::commands::config::load_activated_models_inner;
use crate::core::models::ProfileModelOverride;
use crate::utils::computer::VisionRoute;
use std::collections::HashSet;

/// 截图描述所需的路由素材（主模型 + 语言 + 「视觉」档位覆盖，均来自当前回合）。
#[derive(Clone)]
pub(crate) struct ComputerVisionCtx {
    /// 主模型端点
    pub api_url: String,
    /// 主模型密钥
    pub api_key: String,
    /// 主模型 ID
    pub model: String,
    /// UI 语言（`en-US` 用英文描述）
    pub locale: Option<String>,
    /// 子智能体模型覆盖（`vision` 档位是用户显式指定视觉模型的入口）
    pub overrides: Vec<ProfileModelOverride>,
}

impl ComputerVisionCtx {
    /// 从回合参数构造（`overrides` 为回退到文件读取前的内存副本）。
    pub(crate) fn new(
        api_url: &str,
        api_key: &str,
        model: &str,
        locale: Option<&str>,
        overrides: &[ProfileModelOverride],
    ) -> Self {
        Self {
            api_url: api_url.to_string(),
            api_key: api_key.to_string(),
            model: model.to_string(),
            locale: locale.map(str::to_string),
            overrides: overrides.to_vec(),
        }
    }
}

/// 选择截图描述用的视觉模型路由。
///
/// 顺序（前者恒优先）：
/// 1. 用户显式配置的 `vision` 档位覆盖（三个字段齐全才算数——用户的选择胜过目录判断）；
/// 2. 主模型本身具备视觉能力；
/// 3. 已激活模型中第一个「目录标记为视觉可用」的；
/// 4. 都没有 → `None`（快照仍可用，只是不带截图描述）。
pub(crate) fn pick_vision_route(
    overrides: &[ProfileModelOverride],
    main: VisionRoute,
    main_has_vision: bool,
    activated: &[(VisionRoute, bool)],
) -> Option<VisionRoute> {
    if let Some(route) = overrides
        .iter()
        .find(|o| {
            o.profile_id == "vision"
                && !o.api_key.trim().is_empty()
                && !o.api_url.trim().is_empty()
                && !o.model_id.trim().is_empty()
        })
        .map(|o| VisionRoute {
            api_url: o.api_url.clone(),
            api_key: o.api_key.clone(),
            model: o.model_id.clone(),
            locale: main.locale.clone(),
        })
    {
        return Some(route);
    }
    if main_has_vision {
        return Some(main.clone());
    }
    activated
        .iter()
        .find(|(route, has_vision)| {
            *has_vision && !route.api_key.trim().is_empty() && !route.api_url.trim().is_empty()
        })
        .map(|(route, _)| route.clone())
}

/// 汇总目录 + 已激活模型，解析出一次截图描述用的路由。
///
/// 目录/密钥库都是阻塞 I/O，调用方负责放进阻塞池（见 `execute_builtin_tool`）。
pub(crate) fn resolve_computer_vision_route(
    app: &AppHandle,
    ctx: &ComputerVisionCtx,
) -> Option<VisionRoute> {
    let vision_models = crate::commands::catalog::load_models_catalog_full(app.clone())
        .ok()
        .map(|response| vision_model_ids(&response.json))
        .unwrap_or_default();

    let main = VisionRoute {
        api_url: ctx.api_url.clone(),
        api_key: ctx.api_key.clone(),
        model: ctx.model.clone(),
        locale: ctx.locale.clone(),
    };
    let main_has_vision = supports_vision(&vision_models, &ctx.model);

    let mut activated: Vec<(VisionRoute, bool)> = Vec::new();
    for model in load_activated_models_inner(app.clone()).unwrap_or_default() {
        if model.api_key.trim().is_empty() || model.api_url.trim().is_empty() {
            continue;
        }
        let has_vision = supports_vision(&vision_models, &model.model_id);
        activated.push((
            VisionRoute {
                api_url: model.api_url,
                api_key: model.api_key,
                model: model.model_id,
                locale: ctx.locale.clone(),
            },
            has_vision,
        ));
    }

    pick_vision_route(&ctx.overrides, main, main_has_vision, &activated)
}

/// 目录中被标记为视觉可用的模型 ID 集合（含 `provider/id` 形式的后缀与别名）。
fn vision_model_ids(json: &str) -> HashSet<String> {
    let mut out: HashSet<String> = HashSet::new();
    let Ok(catalog) = serde_json::from_str::<serde_json::Value>(json) else {
        return out;
    };
    let Some(models) = catalog.get("models").and_then(|v| v.as_array()) else {
        return out;
    };
    for model in models {
        let has_vision = model
            .get("capabilities")
            .and_then(|c| c.get("vision"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !has_vision {
            continue;
        }
        if let Some(id) = model.get("id").and_then(|v| v.as_str()) {
            let lower = id.to_lowercase();
            out.insert(lower.clone());
            if let Some((_, bare)) = lower.rsplit_once('/') {
                out.insert(bare.to_string());
            }
        }
        if let Some(aliases) = model.get("aliases").and_then(|v| v.as_array()) {
            for alias in aliases.iter().filter_map(|v| v.as_str()) {
                out.insert(alias.to_lowercase());
            }
        }
    }
    out
}

/// 模型 ID 是否被目录标记为视觉可用（大小写不敏感，兼容 `provider/id` 写法）。
fn supports_vision(vision_models: &HashSet<String>, model_id: &str) -> bool {
    let lower = model_id.trim().to_lowercase();
    if lower.is_empty() {
        return false;
    }
    if vision_models.contains(&lower) {
        return true;
    }
    match lower.rsplit_once('/') {
        Some((_, bare)) => vision_models.contains(bare),
        None => false,
    }
}

#[cfg(test)]
mod computer_vision_tests {
    use super::*;

    fn route(model: &str, key: &str) -> VisionRoute {
        VisionRoute {
            api_url: "https://example.test/v1".into(),
            api_key: key.into(),
            model: model.into(),
            locale: Some("zh-CN".into()),
        }
    }

    fn override_for(profile: &str, key: &str) -> ProfileModelOverride {
        ProfileModelOverride {
            profile_id: profile.into(),
            model_id: "vision-model".into(),
            api_url: "https://vision.test/v1".into(),
            api_key: key.into(),
        }
    }

    #[test]
    fn pick_route_prefers_explicit_override() {
        let overrides = vec![override_for("vision", "k")];
        let picked = pick_vision_route(
            &overrides,
            route("main", "k"),
            true,
            &[(route("alt", "k"), true)],
        )
        .unwrap();
        assert_eq!(picked.model, "vision-model");
        assert_eq!(picked.api_url, "https://vision.test/v1");
    }

    #[test]
    fn pick_route_uses_main_when_vision() {
        let picked = pick_vision_route(&[], route("main", "k"), true, &[]).unwrap();
        assert_eq!(picked.model, "main");
    }

    #[test]
    fn pick_route_falls_back_to_activated() {
        let activated = vec![(route("blind", "k"), false), (route("seer", "k"), true)];
        let picked = pick_vision_route(&[], route("main", "k"), false, &activated).unwrap();
        assert_eq!(picked.model, "seer");
    }

    #[test]
    fn pick_route_none_without_vision() {
        let activated = vec![(route("blind", "k"), false)];
        assert!(pick_vision_route(&[], route("main", "k"), false, &activated).is_none());
    }

    #[test]
    fn pick_route_ignores_override_without_key() {
        let overrides = vec![override_for("vision", "  ")];
        assert!(pick_vision_route(&overrides, route("main", "k"), false, &[]).is_none());
        // 非 vision 档位的覆盖不参与
        let overrides = vec![override_for("coder", "k")];
        assert!(pick_vision_route(&overrides, route("main", "k"), false, &[]).is_none());
    }

    #[test]
    fn catalog_vision_ids_match_prefixed_and_bare_forms() {
        let json = r#"{"models":[
            {"id":"xai/grok-4","capabilities":{"vision":true},"aliases":["grok-4-latest"]},
            {"id":"plain-model","capabilities":{"vision":false},"aliases":[]}
        ]}"#;
        let set = vision_model_ids(json);
        assert!(supports_vision(&set, "xai/grok-4"));
        assert!(supports_vision(&set, "grok-4"));
        assert!(supports_vision(&set, "grok-4-latest"));
        assert!(!supports_vision(&set, "plain-model"));
        assert!(!supports_vision(&set, ""));
    }
}
