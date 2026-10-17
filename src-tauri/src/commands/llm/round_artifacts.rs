//! Shared machinery of the agent loop (阶段4 第1项子目标 5/5：subagents 与主循环
//! 抽公共 agent_loop——收敛两循环逐字符重复的产物构造与压缩守卫).
//!
//! 切片 1（产物构造）：assistant 消息追加 / role:tool 消息（主循环内联 ×3 + 子代理
//! ×1）/ `[Error]` 文本——收敛为单一实现。
//! 切片 2（截断续写守卫）：`finish_reason="length"` 续写块。
//! 切片 3（工具结果展示三元组）：文本/JSON 值/is_error 归一化（主循环 ×3 + 子代理私有）。
//! 切片 4（压缩守卫）：`CompressCtx`——两循环 4 处 take/call/restore + 12 参
//! 压缩调用 + 溢出恢复门（预算 + 错误分类 + 最小长度）收敛为单一实现。
//!
//! 控制流层（工具执行三阶段调度）未强行统一：主循环的 delegation/ask_user/任务板
//! 与子代理的 profile 门控/worktree 是**域语义**而非样板——强行抽取需要数十个
//! 钩子的策略对象，抽象成本高于重复成本。本模块承载全部真同构机器。

use super::streaming::{RoundResult, ToolCallAccum};
use crate::core::models::ToolResult;
use serde_json::{json, Value};

/// Extracts a tool result's first text content (serializes the whole content when
/// no text part exists). Shared by both loops — the main loop previously inlined
/// this 8-line extraction at its serial and parallel dispatch sites, the subagent
/// loop under a private helper of the identical body.
pub(crate) fn tool_result_text(tr: &ToolResult) -> String {
    tr.content
        .iter()
        .find(|c| c.kind == "text")
        .and_then(|c| c.data.get("text"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| serde_json::to_string(&tr.content).unwrap_or_default())
}

/// Normalizes a tool outcome to the display triple both loops consume:
/// (frontend text, structured JSON value, is_error). Errors map to the shared
/// `[Error] {e}` text + `{ "error": e }` value + `true` — the main loop previously
/// duplicated this triple-conversion at both dispatch sites.
pub(crate) fn tool_result_display(result: &Result<ToolResult, String>) -> (String, Value, bool) {
    match result {
        Ok(tr) => {
            let text = tool_result_text(tr);
            let result_value = serde_json::to_value(&tr.content).unwrap_or(json!([]));
            (text, result_value, tr.is_error)
        }
        Err(e) => (tool_error_text(e), json!({ "error": e }), true),
    }
}

/// Appends the round's assistant message (`role/content[/tool_calls]`) to the context.
///
/// Shared by the main loop and the subagent loop. `null_content` selects the main
/// loop's convention: an empty string content serializes as JSON null (the subagent
/// loop keeps the empty string as-is).
pub(crate) fn append_assistant_message(
    messages: &mut Vec<Value>,
    rr: &RoundResult,
    null_content: bool,
) {
    let mut asst_obj = serde_json::Map::new();
    asst_obj.insert("role".into(), json!("assistant"));
    asst_obj.insert(
        "content".into(),
        if null_content {
            match &rr.api_content {
                Value::String(s) if s.is_empty() => json!(null),
                other => other.clone(),
            }
        } else {
            rr.api_content.clone()
        },
    );
    if !rr.tool_calls.is_empty() {
        asst_obj.insert("tool_calls".into(), json!(tool_calls_json(&rr.tool_calls)));
    }
    messages.push(Value::Object(asst_obj));
}

/// tool_calls → OpenAI tool_calls JSON array (id/type/function shape).
pub(crate) fn tool_calls_json(tool_calls: &[ToolCallAccum]) -> Vec<Value> {
    tool_calls
        .iter()
        .map(|tc| {
            json!({
                "id": tc.id,
                "type": "function",
                "function": { "name": tc.name, "arguments": tc.arguments },
            })
        })
        .collect()
}

/// Pushes one `role:tool` response message (the 4-field shape both loops build).
pub(crate) fn push_tool_message(
    messages: &mut Vec<Value>,
    tool_call_id: &str,
    name: &str,
    content: &str,
) {
    let mut tool_msg = serde_json::Map::new();
    tool_msg.insert("role".into(), json!("tool"));
    tool_msg.insert("content".into(), json!(content));
    tool_msg.insert("tool_call_id".into(), json!(tool_call_id));
    tool_msg.insert("name".into(), json!(name));
    messages.push(Value::Object(tool_msg));
}

/// Maps a tool error to the shared `[Error] {e}` display text (both loops' convention).
pub(crate) fn tool_error_text(e: &str) -> String {
    format!("[Error] {}", e)
}

/// Truncation-continuation guard (shared by both loops).
///
/// When the round ended truncated (`finish_reason="length"`) with no tool calls and
/// the retry budget allows, appends the standard continuation instruction and bumps
/// the counter. Returns `true` when the caller should `continue` the loop; the
/// retry cap (`max_retries`) differs per loop (main: MAX_LENGTH_RETRIES, subagent: 1).
pub(crate) fn try_length_continuation(
    messages: &mut Vec<Value>,
    finish_reason: Option<&str>,
    tool_calls_empty: bool,
    retries: &mut u32,
    max_retries: u32,
) -> bool {
    if finish_reason == Some("length") && tool_calls_empty && *retries < max_retries {
        *retries += 1;
        messages.push(json!({
            "role": "system",
            "content": "[输出被截断] 请直接继续完成上一条回复中未完成的内容，不要重复已经输出的部分。"
        }));
        return true;
    }
    false
}

// ===== 压缩守卫（agent_loop 切片 4）：两循环的溢出恢复与预算门控 =====

/// Borrowed compression-execution context shared by both loops.
///
/// The two loops invoke the same `compress_context` with different ids/tag
/// ("main" vs "subagent"); the take/call/restore + 12-argument invocation
/// pattern was previously duplicated at **4 sites** (main: budget gate +
/// overflow gate; sub: budget gate + overflow gate via a private wrapper).
pub(crate) struct CompressCtx<'a> {
    pub window: &'a tauri::Window,
    pub client: &'a reqwest::Client,
    pub api_url: &'a str,
    pub api_key: &'a str,
    pub model: &'a str,
    /// Owning agent/topic ids (the subagent loop passes its PARENT ids — the
    /// compression events/summaries attribute to the main conversation).
    pub assistant_id: &'a str,
    pub topic_id: &'a str,
    pub token: &'a tokio_util::sync::CancellationToken,
    pub keep_recent_tokens: usize,
    /// Compression source tag: "main" | "subagent" (drives event/summary semantics).
    pub tag: &'a str,
    pub locale: &'a str,
}

impl CompressCtx<'_> {
    /// Runs the shared compression over `messages` in place (take/call/restore).
    pub(crate) async fn compress(&self, messages: &mut Vec<Value>) -> Result<(), String> {
        let mut msgs = std::mem::take(messages);
        let r = super::compress::compress_context(
            self.window,
            self.client,
            self.api_url,
            self.api_key,
            self.model,
            &mut msgs,
            self.assistant_id,
            self.topic_id,
            self.token,
            self.keep_recent_tokens,
            self.tag,
            self.locale,
        )
        .await;
        *messages = msgs;
        r
    }

    /// Context-overflow auto-recovery gate (shared by both loops).
    ///
    /// When the round errored with a context-overflow classification and the retry
    /// budget allows, compresses in place. Returns `true` when the caller should
    /// `continue` the loop; `false` leaves the caller's failure path in charge.
    pub(crate) async fn try_overflow_recovery(
        &self,
        error: &str,
        message_count: usize,
        retries: &mut u32,
        max_retries: u32,
        messages: &mut Vec<Value>,
    ) -> bool {
        if overflow_recoverable(error, message_count, *retries, max_retries) {
            *retries += 1;
            if self.compress(messages).await.is_ok() {
                return true; // recovered -> caller continues the loop
            }
        }
        false
    }
}

/// Pure predicate of the overflow gate: retry budget + error classification +
/// minimum-length floor (context of <=4 messages cannot shrink usefully).
pub(crate) fn overflow_recoverable(
    error: &str,
    message_count: usize,
    retries: u32,
    max_retries: u32,
) -> bool {
    retries < max_retries && super::budget::is_context_overflow_error(error) && message_count > 4
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::llm::streaming::RoundResult;

    fn rr(content: &str, api_content: Value, tcs: Vec<ToolCallAccum>) -> RoundResult {
        RoundResult {
            content: content.to_string(),
            reasoning: String::new(),
            tool_calls: tcs,
            input_tokens: 0,
            output_tokens: 0,
            cached_input_tokens: 0,
            ttft_ms: None,
            images: Vec::new(),
            api_content,
            finish_reason: None,
        }
    }

    fn tc(id: &str, name: &str, args: &str) -> ToolCallAccum {
        ToolCallAccum {
            id: id.to_string(),
            name: name.to_string(),
            arguments: args.to_string(),
        }
    }

    #[test]
    fn assistant_message_shape_with_tool_calls() {
        let mut msgs = vec![json!({"role": "user", "content": "go"})];
        append_assistant_message(
            &mut msgs,
            &rr("", json!("working"), vec![tc("a1", "read_file", "{}")]),
            false,
        );
        let asst = msgs.last().unwrap();
        assert_eq!(asst["role"], "assistant");
        assert_eq!(asst["content"], "working");
        let tcs = asst["tool_calls"].as_array().unwrap();
        assert_eq!(tcs[0]["id"], "a1");
        assert_eq!(tcs[0]["type"], "function");
        assert_eq!(tcs[0]["function"]["name"], "read_file");
        assert_eq!(tcs[0]["function"]["arguments"], "{}");
    }

    #[test]
    fn assistant_message_no_tool_calls_omits_the_field() {
        let mut msgs = Vec::new();
        append_assistant_message(&mut msgs, &rr("hi", json!("hi"), Vec::new()), false);
        assert!(msgs[0].get("tool_calls").is_none());
    }

    #[test]
    fn main_loop_null_content_convention() {
        let mut msgs = Vec::new();
        // Empty string api_content -> JSON null (main loop convention)
        append_assistant_message(&mut msgs, &rr("", json!(""), Vec::new()), true);
        assert_eq!(msgs[0]["content"], Value::Null);
        // Non-empty stays a string
        let mut msgs2 = Vec::new();
        append_assistant_message(&mut msgs2, &rr("x", json!("x"), Vec::new()), true);
        assert_eq!(msgs2[0]["content"], "x");
    }

    #[test]
    fn tool_message_four_field_shape() {
        let mut msgs = Vec::new();
        push_tool_message(&mut msgs, "t9", "write_file", "done");
        let tm = &msgs[0];
        assert_eq!(tm["role"], "tool");
        assert_eq!(tm["content"], "done");
        assert_eq!(tm["tool_call_id"], "t9");
        assert_eq!(tm["name"], "write_file");
        assert_eq!(tm.as_object().unwrap().len(), 4, "恰好 4 字段，无多余键");
    }

    #[test]
    fn error_text_convention() {
        assert_eq!(tool_error_text("boom"), "[Error] boom");
    }

    /// 溢出恢复谓词（agent_loop 切片 4）：预算 + 错误分类 + 最小长度三条件。
    #[test]
    fn overflow_recoverable_gates_on_all_three_conditions() {
        // 典型溢出错误 + 预算可用 + 足够消息 -> 可恢复
        assert!(overflow_recoverable("context length exceeded", 5, 0, 1));
        assert!(overflow_recoverable(
            "Maximum context window exceeded",
            8,
            0,
            2
        ));
        // 预算耗尽 -> 不可恢复
        assert!(!overflow_recoverable("context length exceeded", 5, 1, 1));
        // 非溢出错误 -> 不可恢复
        assert!(!overflow_recoverable("connection reset by peer", 5, 0, 1));
        // 消息不足（<=4 无法有效收缩）-> 不可恢复
        assert!(!overflow_recoverable("context length exceeded", 4, 0, 1));
        // 组合：非溢出错误即使预算充足也不可恢复
        assert!(!overflow_recoverable("rate limited", 20, 0, 1));
    }

    #[test]
    fn length_continuation_appends_and_consumes_budget() {
        let mut msgs: Vec<Value> = Vec::new();
        let mut retries = 0;
        // Truncated with no tool calls, budget available -> continue
        assert!(try_length_continuation(
            &mut msgs,
            Some("length"),
            true,
            &mut retries,
            1
        ));
        assert_eq!(retries, 1);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["role"], "system");
        assert!(msgs[0]["content"].as_str().unwrap().contains("继续完成"));
        // Budget exhausted -> no second continuation
        assert!(!try_length_continuation(
            &mut msgs,
            Some("length"),
            true,
            &mut retries,
            1
        ));
        assert_eq!(msgs.len(), 1);
    }

    #[test]
    fn length_continuation_requires_length_and_no_tools() {
        let mut msgs: Vec<Value> = Vec::new();
        let mut retries = 0;
        // finish_reason != "length" -> no continuation
        assert!(!try_length_continuation(
            &mut msgs,
            Some("stop"),
            true,
            &mut retries,
            1
        ));
        // tool calls present -> no continuation (model is still working)
        assert!(!try_length_continuation(
            &mut msgs,
            Some("length"),
            false,
            &mut retries,
            1
        ));
        assert!(msgs.is_empty());
        assert_eq!(retries, 0);
    }

    /// Shared display-triple: text part wins over serialization; error maps to [Error].
    #[test]
    fn tool_result_display_normalizes_both_loops() {
        use crate::core::models::{ToolResult, ToolResultContent};
        let tr = ToolResult {
            content: vec![ToolResultContent {
                kind: "text".into(),
                data: json!({ "text": "hello" }),
            }],
            is_error: false,
        };
        let (text, value, is_err) = tool_result_display(&Ok(tr));
        assert_eq!(text, "hello");
        // ToolResultContent serializes with its serde field names ({type, text}), not the Rust struct fields
        assert_eq!(value, json!([{ "type": "text", "text": "hello" }]));
        assert!(!is_err);

        // No text part -> whole-content serialization
        let tr2 = ToolResult {
            content: vec![ToolResultContent {
                kind: "image".into(),
                data: json!({"url": "u"}),
            }],
            is_error: true,
        };
        let (text2, _, is_err2) = tool_result_display(&Ok(tr2));
        assert!(text2.contains("image"));
        assert!(is_err2);

        // Error -> shared [Error] text
        let (text3, value3, is_err3) = tool_result_display(&Err("boom".into()));
        assert_eq!(text3, "[Error] boom");
        assert_eq!(value3, json!({ "error": "boom" }));
        assert!(is_err3);
    }
}
