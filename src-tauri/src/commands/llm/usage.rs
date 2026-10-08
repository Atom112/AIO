//! Token 用量统计与计数命令（usage_log 表写入/查询、count_tokens）。

use crate::commands::llm::*;
use crate::core::state::DbState;

/// 将单轮 LLM 调用的 token 用量写入 usage_log 表（不可变 append-only 记录）。
///
/// 跳过 input + output 均为 0 的空记录（本地引擎可能不返回 usage）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn insert_usage_log(
    app: &AppHandle,
    assistant_id: &str,
    topic_id: &str,
    model_id: &str,
    round: u32,
    input_tokens: u32,
    output_tokens: u32,
    cached_input_tokens: u32,
) {
    if input_tokens == 0 && output_tokens == 0 {
        return;
    }
    let db = app.state::<DbState>();
    let conn = db.0.lock();
    let id = uuid::Uuid::new_v4().to_string();
    let _ = conn.execute(
        "INSERT INTO usage_log (id, assistant_id, topic_id, model_id, round, input_tokens, output_tokens, cached_input_tokens) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            id,
            assistant_id,
            topic_id,
            model_id,
            round,
            input_tokens,
            output_tokens,
            cached_input_tokens
        ],
    );
}

/// 返回按日期降序排列的每日 input/output tokens 与请求次数。
/// 用于前端用量面板展示（如"过去 7 天 / 30 天用量"）。
#[tauri::command]
pub(crate) fn get_usage_summary(
    db_state: tauri::State<'_, DbState>,
    days: u32,
) -> Result<Vec<UsageSummary>, String> {
    let conn = db_state.0.lock();
    let mut stmt = conn
        .prepare(
            "SELECT date(timestamp) as day,
                    SUM(input_tokens) as total_input,
                    SUM(output_tokens) as total_output,
                    COALESCE(SUM(cached_input_tokens), 0) as total_cached,
                    COUNT(*) as request_count
             FROM usage_log
             WHERE timestamp >= datetime('now', ?1)
             GROUP BY day
             ORDER BY day DESC",
        )
        .map_err(|e| e.to_string())?;
    let days_param = format!("-{} days", days);
    let rows = stmt
        .query_map([&days_param], |row| {
            Ok(UsageSummary {
                date: row.get(0)?,
                input_tokens: row.get(1)?,
                output_tokens: row.get(2)?,
                cached_input_tokens: row.get(3)?,
                request_count: row.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut summaries = Vec::new();
    for row in rows {
        summaries.push(row.map_err(|e| e.to_string())?);
    }
    Ok(summaries)
}

/// 查询最近 N 天的 token 用量按模型聚合。
///
/// 返回按总 token 数降序排列的各模型用量，用于前端模型分布面板。
#[tauri::command]
pub(crate) fn get_usage_summary_by_model(
    db_state: tauri::State<'_, DbState>,
    days: u32,
    date: Option<String>,
) -> Result<Vec<UsageSummaryByModel>, String> {
    let conn = db_state.0.lock();
    if let Some(ref date_str) = date {
        let mut stmt = conn
            .prepare(
                "SELECT model_id,
                        SUM(input_tokens) as total_input,
                        SUM(output_tokens) as total_output,
                        COALESCE(SUM(cached_input_tokens), 0) as total_cached,
                        COUNT(*) as request_count
                 FROM usage_log
                 WHERE date(timestamp) = ?1
                 GROUP BY model_id
                 ORDER BY (SUM(input_tokens) + SUM(output_tokens)) DESC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([date_str.as_str()], |row| {
                Ok(UsageSummaryByModel {
                    model_id: row.get(0)?,
                    input_tokens: row.get(1)?,
                    output_tokens: row.get(2)?,
                    cached_input_tokens: row.get(3)?,
                    request_count: row.get(4)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut summaries = Vec::new();
        for row in rows {
            summaries.push(row.map_err(|e| e.to_string())?);
        }
        Ok(summaries)
    } else {
        let mut stmt = conn
            .prepare(
                "SELECT model_id,
                        SUM(input_tokens) as total_input,
                        SUM(output_tokens) as total_output,
                        COALESCE(SUM(cached_input_tokens), 0) as total_cached,
                        COUNT(*) as request_count
                 FROM usage_log
                 WHERE timestamp >= datetime('now', ?1)
                 GROUP BY model_id
                 ORDER BY (SUM(input_tokens) + SUM(output_tokens)) DESC",
            )
            .map_err(|e| e.to_string())?;
        let days_param = format!("-{} days", days);
        let rows = stmt
            .query_map([&days_param], |row| {
                Ok(UsageSummaryByModel {
                    model_id: row.get(0)?,
                    input_tokens: row.get(1)?,
                    output_tokens: row.get(2)?,
                    cached_input_tokens: row.get(3)?,
                    request_count: row.get(4)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut summaries = Vec::new();
        for row in rows {
            summaries.push(row.map_err(|e| e.to_string())?);
        }
        Ok(summaries)
    }
}

/// 估算文本的 token 数（供前端输入框实时预览）。
/// 包装调用 `utils::token_counter::count_tokens_cmd` 纯函数。
#[tauri::command]
pub(crate) fn count_tokens_cmd(model: String, text: String) -> Result<usize, String> {
    crate::utils::token_counter::count_tokens_cmd(model, text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::GeneratedImage;

    fn conn() -> rusqlite::Connection {
        rusqlite::Connection::open_in_memory().unwrap()
    }

    #[test]
    fn message_for_api_expands_images_and_strips_token() {
        let dir = std::env::temp_dir().join(format!("aio-msg-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let img_path = dir.join("g.png");
        std::fs::write(&img_path, b"\x89PNG").unwrap();
        let abs = img_path
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .to_string();

        let token = crate::utils::generated_images::image_token_for(&abs);
        let content_html = format!("text before ![img]({})", token);
        let msg = Message {
            id: None,
            role: "assistant".into(),
            content: serde_json::json!(content_html),
            model_id: None,
            display_files: None,
            display_text: None,
            tool_call_id: None,
            name: None,
            tool_calls: None,
            reasoning: None,
            input_tokens: None,
            output_tokens: None,
            cached_input_tokens: None,
            agent_steps: None,
            interim_content: None,
            agent_start_time: None,
            parent_message_id: None,
            branch_index: 0,
            images: Some(vec![GeneratedImage {
                name: "g.png".into(),
                mime_type: "image/png".into(),
                size: 4,
                storage_path: abs.clone(),
            }]),
            full_tool_result: None,
        };
        let out = message_for_api(&conn(), &msg).unwrap();
        let obj = out.as_object().unwrap();
        let parts = obj.get("content").unwrap().as_array().unwrap();
        // 文本部分已剥离 aio-image 标记（防 token 泄漏）
        let text = parts[0]["text"].as_str().unwrap();
        assert!(!text.contains("aio-image://"));
        assert!(text.contains("text before"));
        // 图像部分重新展开为 image_url data URI（历史回放）
        assert_eq!(parts[1]["type"], "image_url");
        let url = parts[1]["image_url"]["url"].as_str().unwrap();
        assert!(url.starts_with("data:image/png;base64,"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
