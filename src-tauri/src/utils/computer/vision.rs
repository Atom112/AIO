//! 截图 → 文字描述（视觉桥）。
//!
//! 边界：只做一次非流式多模态请求，把截图变成结构化文字；**不**解析目录、不读密钥库
//! （路由解析属于命令层）。主模型因此不需要视觉能力：像素路径变成文本路径。

use base64::engine::general_purpose;
use base64::Engine as _;
use serde_json::json;

use super::{UiWindow, VisionRoute};
use crate::ierr;
use crate::plugins::llm::{
    chat_error_message, chat_text_content, non_streaming_http_client_pinned, post_chat_completion,
};

/// 屏幕分析系统提示（中文）。
const SCREEN_SYSTEM_ZH: &str = "你是 AIO 的屏幕分析智能体。主模型看不到屏幕，由你代为查看这张截图。请输出结构化描述：\n1) 前台窗口标题与应用；\n2) 可交互元素：控件类型、可见文字、中心点的归一化坐标 (x,y)，取值 0-1；\n3) 画面中的可见文字（逐字转录；密码框内容不要转写）；\n4) 当前状态、弹窗或异常提示。\n不要臆造截图中不存在的内容。";

/// 屏幕分析系统提示（英文）。
const SCREEN_SYSTEM_EN: &str = "You are AIO's screen analyst. The main model cannot see the screen, so you inspect this screenshot on its behalf. Output a structured description:\n1) foreground window title and app;\n2) interactive elements: control type, visible label, center point as normalized (x,y) in 0..1;\n3) visible text (transcribe verbatim; never transcribe password fields);\n4) current state, dialogs, or error prompts.\nDo not invent anything not present in the screenshot.";

/// 描述一张截图（PNG 字节）。45 秒超时，返回结构化文字。
pub(crate) async fn describe(
    route: &VisionRoute,
    png: &[u8],
    window: &UiWindow,
) -> Result<String, String> {
    // SSRF：与其它 LLM 调用同一策略（本地/局域网放行，元数据地址拒绝）
    crate::utils::url_validation::validate_http_url(
        &route.api_url,
        &crate::utils::url_validation::HttpUrlOptions::local_engine(),
    )?;

    let is_en = route.locale.as_deref() == Some("en-US");
    let user_text = if is_en {
        format!(
            "Screenshot window: \"{}\" ({}).",
            window.title, window.app_name
        )
    } else {
        format!("截图窗口: \"{}\" ({}).", window.title, window.app_name)
    };
    let body = json!({
        "model": route.model,
        "messages": [
            {
                "role": "system",
                "content": if is_en { SCREEN_SYSTEM_EN } else { SCREEN_SYSTEM_ZH }
            },
            {
                "role": "user",
                "content": [
                    { "type": "text", "text": user_text },
                    {
                        "type": "image_url",
                        "image_url": {
                            "url": format!(
                                "data:image/png;base64,{}",
                                general_purpose::STANDARD.encode(png)
                            )
                        }
                    }
                ]
            }
        ],
        "stream": false,
        "max_tokens": 1500
    });

    let client = non_streaming_http_client_pinned(&route.api_url).await?;
    let (res, is_anthropic) = tokio::time::timeout(
        std::time::Duration::from_secs(45),
        post_chat_completion(&client, &route.api_url, &route.api_key, &body, None),
    )
    .await
    .map_err(|_| {
        ierr!(
            "屏幕描述请求超时（45s）",
            "Screen description request timed out (45s)"
        )
    })?
    .map_err(|e| e.to_string())?;

    let val: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;
    if let Some(err) = chat_error_message(&val) {
        return Err(err);
    }
    let text = chat_text_content(&val, is_anthropic);
    if text.trim().is_empty() {
        return Err(ierr!(
            "视觉模型返回了空描述",
            "The vision model returned an empty description"
        ));
    }
    Ok(text)
}
