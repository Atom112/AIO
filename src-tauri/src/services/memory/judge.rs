//! LLM 仲裁：新事实与既有事实高度相似/冲突时，判定 merge / update / supersede / keep_separate。
//!
//! 仲裁只在向量近邻高分（如 >= 0.92）时触发，且失败一律 fail-safe 为 keep_separate（宁可冗余，不误并误删）。

use crate::services::memory::llm::{chat_json, LlmConfig};
use serde::{Deserialize, Serialize};

/// 仲裁动作。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum JudgeAction {
    /// 两件事不同，保持两条（默认/兜底）
    KeepSeparate,
    /// 新事实更准确，以新事实订正既有事实
    Update,
    /// 合并为一条（内容由模型生成）
    Merge,
    /// 新事实取代既有事实（冲突/过时）
    Supersede,
}

/// 模型返回的仲裁 JSON。
#[derive(Deserialize)]
struct JudgeResponse {
    #[serde(default = "default_action")]
    action: String,
    #[serde(default)]
    merged_content: String,
    #[serde(default, rename = "reason")]
    _reason: String,
}

fn default_action() -> String {
    "keep_separate".into()
}

/// 仲裁：给定既有事实与新事实，返回动作（与合并内容）。
pub async fn judge(cfg: &LlmConfig, existing: &str, incoming: &str) -> (JudgeAction, String) {
    let system = concat!(
        "你是项目记忆库的仲裁器。两个事实来自同一项目，语义高度相似。判定二者关系：\n",
        "- merge：可无损合并为一条更完整的表述（输出 merged_content）；\n",
        "- update：新事实更准确/更新，以新事实为准（旧内容作废）；\n",
        "- supersede：新事实明确取代旧事实（如依赖迁移、方案变更，旧内容过时）；\n",
        "- keep_separate：二者是不同的事实，即便有重叠。\n",
        "只输出 JSON：{action: merge|update|supersede|keep_separate, merged_content: 字符串, reason: 一句话}。",
    );
    let user = format!("既有事实：\n{existing}\n\n新事实：\n{incoming}");
    match chat_json(cfg, system, &user, 300).await {
        Ok(v) => parse_response(&v),
        Err(_) => (JudgeAction::KeepSeparate, String::new()),
    }
}

/// 解析仲裁响应（容错：任何异常都回退 keep_separate）。
pub fn parse_response(v: &serde_json::Value) -> (JudgeAction, String) {
    let fallback = JudgeResponse {
        action: "keep_separate".into(),
        merged_content: String::new(),
        reason: String::new(),
    };
    let resp = serde_json::from_value::<JudgeResponse>(v.clone()).unwrap_or(fallback);
    let action = match resp.action.as_str() {
        "update" => JudgeAction::Update,
        "merge" => JudgeAction::Merge,
        "supersede" => JudgeAction::Supersede,
        _ => JudgeAction::KeepSeparate,
    };
    (action, resp.merged_content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parse_actions() {
        assert_eq!(
            parse_response(&json!({ "action": "merge", "merged_content": "A+B", "reason": "r" })).0,
            JudgeAction::Merge,
        );
        assert_eq!(
            parse_response(&json!({ "action": "update" })).0,
            JudgeAction::Update
        );
        assert_eq!(
            parse_response(&json!({ "action": "supersede" })).0,
            JudgeAction::Supersede,
        );
        assert_eq!(
            parse_response(&json!({ "action": "garbage" })).0,
            JudgeAction::KeepSeparate,
        );
        assert_eq!(parse_response(&json!([])).0, JudgeAction::KeepSeparate);
    }

    #[test]
    fn test_merged_content() {
        let (action, content) =
            parse_response(&json!({ "action": "merge", "merged_content": "整合后的内容" }));
        assert_eq!(action, JudgeAction::Merge);
        assert_eq!(content, "整合后的内容");
    }
}
