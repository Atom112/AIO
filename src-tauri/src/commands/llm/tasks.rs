//! Agent task board + user-interaction tools (main agent only).
//!
//! Two mainstream-agent capabilities (Claude Code-shaped, adapted to AIO's chat UI):
//!
//! - **Task board** (`create_tasks` / `update_task` / `list_tasks`): the orchestrator
//!   decomposes multi-step work into a tracked task list and keeps statuses current; the
//!   frontend mirrors the board via `agent-tasks-changed` (top panel, per topic).
//! - **`ask_user`**: for genuine decision points (ambiguous requirements, alternative
//!   approaches, risk trade-offs) the agent presents recommended options plus free-form
//!   input, pauses the loop, and resumes with the user's answer. The pause/resume channel
//!   mirrors PendingApprovals (`respond_agent_question` resolves the oneshot).
//!
//! Both are orchestrator-internal tools: they never touch the filesystem, never mutate
//! external state, and are dispatched in the main loop before `execute_builtin_tool`
//! (like `read_artifact` / `read_skill`), so they bypass the permission system by design —
//! the user's answer IS the approval. Subagents never receive them (not injected into any
//! subagent profile tool set).

use crate::core::events as ev;
use crate::core::models::{AgentTask, AgentTaskStatus, QuestionResponse, ToolSpec};
use crate::core::state::{AgentTaskBoard, PendingAgentQuestions};
use crate::ierr;
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio_util::sync::CancellationToken;

use crate::utils::tool_result::{tool_err, tool_ok};

/// Wait budget for one ask_user request. Token cancellation (stop button) is the primary
/// escape; this timeout is only the leak-prevention safety valve.
const ASK_USER_TIMEOUT: Duration = Duration::from_secs(600);

/// Max tasks creatable in one call (create_tasks) and max questions per ask_user call.
const MAX_TASKS: usize = 10;
const MAX_QUESTIONS: usize = 4;
/// Max options per question.
const MAX_OPTIONS: usize = 4;

// ====== Tool specs ======

/// Returns the specs of the four orchestrator-interaction tools.
pub fn get_agent_task_tool_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            kind: "function".into(),
            function: crate::core::models::ToolFunctionSpec {
                name: "create_tasks".into(),
                description: concat!(
                    "创建任务清单：把复杂/多步任务拆解为可跟踪的子任务列表，用户会实时看到进度面板。",
                    "开始多步工作时先建清单，随进展用 update_task 更新状态。",
                    "每个任务应是一个可独立验证的阶段性成果（约对应一个委托子任务或一组相关工具调用），",
                    "不要把单个工具调用拆成任务，也不要为琐碎任务建清单。"
                )
                .into(),
                parameters: json!({
                    "type":"object",
                    "properties":{
                        "tasks":{
                            "type":"array",
                            "description":"任务列表（1-10 项，按执行顺序）",
                            "items":{
                                "type":"object",
                                "properties":{
                                    "title":{"type":"string","description":"任务标题：一句话、动词开头（如「实现下载重试逻辑」）"}
                                },
                                "required":["title"]
                            },
                            "minItems":1,
                            "maxItems":10
                        },
                        "clear_existing":{
                            "type":"boolean",
                            "description":"是否清空本话题已有任务清单（重新规划时传 true）；默认 false 追加"
                        }
                    },
                    "required":["tasks"]
                }),
            },
        },
        ToolSpec {
            kind: "function".into(),
            function: crate::core::models::ToolFunctionSpec {
                name: "update_task".into(),
                description: concat!(
                    "更新任务状态或改写任务标题。每开始/完成一个任务应及时更新，让用户看到真实进度。",
                    "status：pending（未开始）/ in_progress（进行中）/ completed（已完成）。"
                )
                .into(),
                parameters: json!({
                    "type":"object",
                    "properties":{
                        "task_id":{"type":"string","description":"要更新的任务 ID（create_tasks 返回）"},
                        "status":{"type":"string","enum":["pending","in_progress","completed"],"description":"新状态"},
                        "title":{"type":"string","description":"新标题（可选，改写任务时使用）"}
                    },
                    "required":["task_id"]
                }),
            },
        },
        ToolSpec {
            kind: "function".into(),
            function: crate::core::models::ToolFunctionSpec {
                name: "list_tasks".into(),
                description: "查看当前话题的任务清单（含各任务 ID 与状态）。新回合继续工作时先查看，避免重复建任务。".into(),
                parameters: json!({"type":"object","properties":{}}),
            },
        },
        ToolSpec {
            kind: "function".into(),
            function: crate::core::models::ToolFunctionSpec {
                name: "ask_user".into(),
                description: concat!(
                    "就真正需要用户拍板的争议点向用户提问：需求歧义、多种可行方案、风险取舍、",
                    "缺少关键约束等无法从上下文自行决定的问题。一次提出全部问题（最多 4 个）；",
                    "每个问题给出 2-4 个具体可执行的选项，把最推荐的选项放首位并标 recommended=true；",
                    "用户也可以输入自定义回答。调用后本轮暂停等待用户作答，回答将作为结果返回，",
                    "据此继续执行。注意：能从上下文合理推断或属于实现细节的不要问；",
                    "每个争议点只问一次，不要反复确认。"
                )
                .into(),
                parameters: json!({
                    "type":"object",
                    "properties":{
                        "questions":{
                            "type":"array",
                            "description":"问题列表（1-4 个）",
                            "items":{
                                "type":"object",
                                "properties":{
                                    "question":{"type":"string","description":"具体、可直接回答的问题"},
                                    "options":{
                                        "type":"array",
                                        "description":"候选项（0-4 个），最推荐的放首位",
                                        "items":{
                                            "type":"object",
                                            "properties":{
                                                "label":{"type":"string","description":"选项（简短动词短语）"},
                                                "description":{"type":"string","description":"选项说明：一句话讲清影响或代价"},
                                                "recommended":{"type":"boolean","description":"是否为推荐选项"}
                                            },
                                            "required":["label"]
                                        },
                                        "maxItems":4
                                    },
                                    "allow_custom":{"type":"boolean","description":"是否允许自定义回答（默认 true）"}
                                },
                                "required":["question"]
                            },
                            "minItems":1,
                            "maxItems":4
                        }
                    },
                    "required":["questions"]
                }),
            },
        },
    ]
}

// ====== Task board ======

/// Event payload (snake_case, consistent with the other LLM streaming events).
#[derive(Serialize, Clone)]
pub struct AgentTasksPayload {
    pub assistant_id: String,
    pub topic_id: String,
    pub tasks: Vec<AgentTask>,
}

/// Renders the board as numbered lines for the tool result.
fn format_board(tasks: &[AgentTask]) -> String {
    if tasks.is_empty() {
        return "（当前无任务）".to_string();
    }
    tasks
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let status = match t.status {
                AgentTaskStatus::Pending => "待办",
                AgentTaskStatus::InProgress => "进行中",
                AgentTaskStatus::Completed => "已完成",
            };
            format!("{}. [{}] {} ({})", i + 1, status, t.title, t.id)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Parses a status string into AgentTaskStatus.
fn parse_status(s: &str) -> Result<AgentTaskStatus, String> {
    match s {
        "pending" => Ok(AgentTaskStatus::Pending),
        "in_progress" => Ok(AgentTaskStatus::InProgress),
        "completed" => Ok(AgentTaskStatus::Completed),
        _ => Err(ierr!(
            "未知任务状态 '{}'（可选：pending / in_progress / completed）",
            "Unknown task status '{}' (options: pending / in_progress / completed)",
            s
        )),
    }
}

/// Creates tasks on the board (append, or replace when clear_existing). Returns the new list.
fn board_create(
    board: &AgentTaskBoard,
    key: &(String, String),
    titles: Vec<String>,
    clear_existing: bool,
) -> Vec<AgentTask> {
    let mut tasks = if clear_existing {
        Vec::new()
    } else {
        board.tasks(key)
    };
    for title in titles {
        tasks.push(AgentTask {
            id: format!("task-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]),
            title,
            status: AgentTaskStatus::Pending,
        });
    }
    board.set_tasks(key.clone(), tasks.clone());
    tasks
}

/// Updates one task by id. Returns the updated list or an error naming valid ids.
fn board_update(
    board: &AgentTaskBoard,
    key: &(String, String),
    task_id: &str,
    status: Option<AgentTaskStatus>,
    title: Option<String>,
) -> Result<Vec<AgentTask>, String> {
    let mut tasks = board.tasks(key);
    let Some(task) = tasks.iter_mut().find(|t| t.id == task_id) else {
        let ids = tasks
            .iter()
            .map(|t| t.id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(ierr!(
            "未找到任务 '{}'。当前任务 ID：{}",
            "Task '{}' not found. Current task IDs: {}",
            task_id,
            if ids.is_empty() {
                "（无任务）".to_string()
            } else {
                ids
            }
        ));
    };
    if let Some(s) = status {
        task.status = s;
    }
    if let Some(t) = title {
        if !t.trim().is_empty() {
            task.title = t.trim().to_string();
        }
    }
    board.set_tasks(key.clone(), tasks.clone());
    Ok(tasks)
}

/// Executes one task-board tool (create_tasks / update_task / list_tasks) and emits the
/// board snapshot to the frontend.
pub(crate) fn execute_task_board_tool(
    app: &AppHandle,
    assistant_id: &str,
    topic_id: &str,
    tool_name: &str,
    arguments: &Value,
) -> crate::core::models::ToolResult {
    let board = app.state::<AgentTaskBoard>();
    let key = (assistant_id.to_string(), topic_id.to_string());

    let tasks = match tool_name {
        "create_tasks" => {
            let Some(arr) = arguments["tasks"].as_array() else {
                return tool_err(&ierr!(
                    "create_tasks 缺少必填参数 'tasks'（应为数组）",
                    "create_tasks is missing the required 'tasks' parameter (expected an array)"
                ));
            };
            if arr.is_empty() || arr.len() > MAX_TASKS {
                return tool_err(&ierr!(
                    "tasks 数组需 1-{} 项",
                    "tasks array allows 1-{} items",
                    MAX_TASKS
                ));
            }
            let mut titles = Vec::with_capacity(arr.len());
            for (idx, item) in arr.iter().enumerate() {
                let title = item["title"].as_str().unwrap_or("").trim().to_string();
                if title.is_empty() {
                    return tool_err(&ierr!(
                        "第 {} 个任务缺少非空 'title'",
                        "task {} is missing a non-empty 'title'",
                        idx + 1
                    ));
                }
                titles.push(title);
            }
            let clear = arguments["clear_existing"].as_bool().unwrap_or(false);
            board_create(&board, &key, titles, clear)
        }
        "update_task" => {
            let task_id = arguments["task_id"].as_str().unwrap_or("").trim();
            if task_id.is_empty() {
                return tool_err(&ierr!(
                    "update_task 缺少必填参数 'task_id'",
                    "update_task is missing the required 'task_id' parameter"
                ));
            }
            let status = match arguments["status"].as_str() {
                Some("") | None => None,
                Some(s) => match parse_status(s) {
                    Ok(st) => Some(st),
                    Err(e) => return tool_err(&e),
                },
            };
            let title = arguments["title"].as_str().map(|s| s.to_string());
            if status.is_none() && title.is_none() {
                return tool_err(&ierr!(
                    "update_task 至少提供 status 或 title 之一",
                    "update_task requires at least one of status / title"
                ));
            }
            match board_update(&board, &key, task_id, status, title) {
                Ok(t) => t,
                Err(e) => return tool_err(&e),
            }
        }
        "list_tasks" => board.tasks(&key),
        _ => {
            return tool_err(&ierr!(
                "未知任务工具: {}",
                "Unknown task tool: {}",
                tool_name
            ))
        }
    };

    // Mirror the board to the frontend (task panel)
    let _ = app.emit(
        ev::AGENT_TASKS_CHANGED,
        AgentTasksPayload {
            assistant_id: assistant_id.to_string(),
            topic_id: topic_id.to_string(),
            tasks: tasks.clone(),
        },
    );
    tool_ok(format_board(&tasks))
}

// ====== ask_user ======

/// One question sent to the frontend (snake_case payload fields).
#[derive(Serialize, Clone, Debug)]
pub struct QuestionView {
    pub question_id: String,
    pub question: String,
    pub options: Vec<QuestionOptionView>,
    pub allow_custom: bool,
}

/// One option of a question.
#[derive(Serialize, Clone, Debug)]
pub struct QuestionOptionView {
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub recommended: bool,
}

/// Event payload for agent-question-requested.
#[derive(Serialize, Clone)]
pub struct QuestionRequestedPayload {
    pub request_id: String,
    pub assistant_id: String,
    pub topic_id: String,
    pub questions: Vec<QuestionView>,
}

/// Parses and validates the ask_user arguments into the question view list.
fn parse_questions(arguments: &Value) -> Result<Vec<QuestionView>, String> {
    let Some(arr) = arguments["questions"].as_array() else {
        return Err(ierr!(
            "ask_user 缺少必填参数 'questions'（应为数组）",
            "ask_user is missing the required 'questions' parameter (expected an array)"
        ));
    };
    if arr.is_empty() || arr.len() > MAX_QUESTIONS {
        return Err(ierr!(
            "questions 数组需 1-{} 项",
            "questions array allows 1-{} items",
            MAX_QUESTIONS
        ));
    }
    let mut views = Vec::with_capacity(arr.len());
    for (idx, item) in arr.iter().enumerate() {
        let question = item["question"].as_str().unwrap_or("").trim().to_string();
        if question.is_empty() {
            return Err(ierr!(
                "第 {} 个问题缺少非空 'question'",
                "question {} is missing non-empty 'question'",
                idx + 1
            ));
        }
        let mut options = Vec::new();
        if let Some(opts) = item["options"].as_array() {
            if opts.len() > MAX_OPTIONS {
                return Err(ierr!(
                    "第 {} 个问题的选项最多 {} 个",
                    "question {} allows at most {} options",
                    idx + 1,
                    MAX_OPTIONS
                ));
            }
            for (oi, opt) in opts.iter().enumerate() {
                let label = opt["label"].as_str().unwrap_or("").trim().to_string();
                if label.is_empty() {
                    return Err(ierr!(
                        "第 {} 个问题的第 {} 个选项缺少非空 'label'",
                        "question {} option {} is missing a non-empty 'label'",
                        idx + 1,
                        oi + 1
                    ));
                }
                options.push(QuestionOptionView {
                    label,
                    description: opt["description"].as_str().map(|s| s.trim().to_string()),
                    recommended: opt["recommended"].as_bool().unwrap_or(false),
                });
            }
        }
        views.push(QuestionView {
            question_id: format!("q{}", idx + 1),
            question,
            options,
            allow_custom: item["allow_custom"].as_bool().unwrap_or(true),
        });
    }
    Ok(views)
}

/// Formats the user's response as the tool result text for the model.
fn format_response(questions: &[QuestionView], response: &QuestionResponse) -> String {
    if response.skipped {
        return "[用户跳过了提问] 请基于现有信息自行决定并继续，不要再重复提问。".to_string();
    }
    let mut out = String::from("[用户回答]");
    for (i, q) in questions.iter().enumerate() {
        let ans = response
            .answers
            .iter()
            .find(|a| a.question_id == q.question_id);
        out.push_str(&format!("\n{}. 问题: {}", i + 1, q.question));
        match ans {
            Some(a) if !a.answer.trim().is_empty() => {
                out.push_str(&format!("\n   回答: {}", a.answer.trim()));
            }
            _ => out.push_str("\n   回答: （未作答，请自行决定）"),
        }
    }
    out
}

/// Executes the ask_user tool: emit the questions, pause until the user answers
/// (respond_agent_question), the token cancels, or the safety-valve timeout fires.
pub(crate) async fn execute_ask_user(
    app: &AppHandle,
    assistant_id: &str,
    topic_id: &str,
    arguments: &Value,
    token: &CancellationToken,
) -> Result<crate::core::models::ToolResult, String> {
    let questions = parse_questions(arguments)?;

    let pending = app.state::<PendingAgentQuestions>();
    let (tx, rx) = tokio::sync::oneshot::channel::<QuestionResponse>();
    let request_id = pending.insert(tx);

    let _ = app.emit(
        ev::AGENT_QUESTION_REQUESTED,
        QuestionRequestedPayload {
            request_id: request_id.clone(),
            assistant_id: assistant_id.to_string(),
            topic_id: topic_id.to_string(),
            questions: questions.clone(),
        },
    );

    let response = tokio::select! {
        _ = token.cancelled() => {
            pending.remove(&request_id);
            return Err("cancelled".into());
        }
        res = tokio::time::timeout(ASK_USER_TIMEOUT, rx) => match res {
            Ok(Ok(r)) => r,
            Ok(Err(_)) => {
                pending.remove(&request_id);
                return Err(ierr!(
                    "提问通道意外关闭",
                    "Question channel closed unexpectedly"
                ));
            }
            Err(_) => {
                pending.remove(&request_id);
                return Err(ierr!(
                    "等待用户回答超时，请基于现有信息自行决定并继续",
                    "Timed out waiting for the user's answer; decide on your own and continue"
                ));
            }
        },
    };

    Ok(tool_ok(format_response(&questions, &response)))
}

/// Command the frontend calls to answer an ask_user request.
///
/// # Parameters
/// - `request_id`: the ask_user request id from the `agent-question-requested` event
/// - `skipped`: the user chose to let the agent decide
/// - `answers`: one answer per question (questionId / answer / isCustom)
#[tauri::command]
pub async fn respond_agent_question(
    pending: State<'_, PendingAgentQuestions>,
    request_id: String,
    skipped: bool,
    answers: Vec<crate::core::models::QuestionAnswer>,
) -> Result<(), String> {
    let tx = pending.remove(&request_id).ok_or_else(|| {
        ierr!(
            "提问请求 {} 不存在或已过期",
            "Question request {} does not exist or has expired",
            request_id
        )
    })?;
    let response = QuestionResponse { skipped, answers };
    tx.send(response)
        .map_err(|_| ierr!("发送回答失败", "Failed to send the answer"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board() -> AgentTaskBoard {
        AgentTaskBoard::new()
    }

    fn key() -> (String, String) {
        ("a1".into(), "t1".into())
    }

    /// create_tasks appends by default, replaces with clear_existing, ids stay unique.
    #[test]
    fn board_create_append_and_clear() {
        let b = board();
        let k = key();
        let t1 = board_create(&b, &k, vec!["任务一".into()], false);
        assert_eq!(t1.len(), 1);
        assert_eq!(t1[0].status, AgentTaskStatus::Pending);
        let t2 = board_create(&b, &k, vec!["任务二".into()], false);
        assert_eq!(t2.len(), 2, "默认应追加");
        let t3 = board_create(&b, &k, vec!["任务三".into()], true);
        assert_eq!(t3.len(), 1, "clear_existing 应清空重建");
        assert_eq!(t3[0].title, "任务三");
        assert_ne!(t1[0].id, t3[0].id, "任务 ID 必须唯一");
        assert_eq!(b.tasks(&k).len(), 1);
    }

    /// update_task changes status/title and errors with valid ids when not found.
    #[test]
    fn board_update_status_title_notfound() {
        let b = board();
        let k = key();
        let t = board_create(&b, &k, vec!["任务一".into()], false);
        let id = t[0].id.clone();

        let updated = board_update(&b, &k, &id, Some(AgentTaskStatus::InProgress), None).unwrap();
        assert_eq!(updated[0].status, AgentTaskStatus::InProgress);

        let updated = board_update(&b, &k, &id, Some(AgentTaskStatus::Completed), None).unwrap();
        assert_eq!(updated[0].status, AgentTaskStatus::Completed);

        let updated = board_update(&b, &k, &id, None, Some("改名".into())).unwrap();
        assert_eq!(updated[0].title, "改名");

        let err = board_update(&b, &k, "task-nope", None, Some("x".into())).unwrap_err();
        assert!(err.contains(&id), "未找到时应列出有效 ID: {err}");

        assert!(b.tasks(&("other".into(), "other".into())).is_empty());
    }

    /// format_board renders numbered lines with status and ids.
    #[test]
    fn format_board_lines() {
        let b = board();
        let k = key();
        let t = board_create(&b, &k, vec!["甲".into(), "乙".into()], false);
        board_update(&b, &k, &t[0].id, Some(AgentTaskStatus::Completed), None).unwrap();
        let text = format_board(&b.tasks(&k));
        assert!(text.contains("1. [已完成] 甲"), "{text}");
        assert!(text.contains("2. [待办] 乙"), "{text}");
        assert_eq!(format_board(&[]), "（当前无任务）");
    }

    /// parse_questions: full validation matrix.
    #[test]
    fn parse_questions_validates() {
        // Missing questions array
        assert!(parse_questions(&json!({})).is_err());
        // Empty / too many
        assert!(parse_questions(&json!({"questions": []})).is_err());
        let too_many: Vec<Value> = (0..5)
            .map(|i| json!({"question": format!("q{i}")}))
            .collect();
        assert!(parse_questions(&json!({"questions": too_many})).is_err());
        // Empty question text
        assert!(parse_questions(&json!({"questions": [{"question": "  "}]})).is_err());
        // Too many options
        let five_opts: Vec<Value> = (0..5).map(|i| json!({"label": format!("o{i}")})).collect();
        assert!(
            parse_questions(&json!({"questions": [{"question": "q", "options": five_opts}]}))
                .is_err()
        );
        // Option without label
        assert!(parse_questions(
            &json!({"questions": [{"question": "q", "options": [{"label": ""}]}]})
        )
        .is_err());
        // Valid: ids assigned in order, allow_custom defaults true, recommended parsed
        let qs = parse_questions(&json!({
            "questions": [{
                "question": "选哪个方案？",
                "options": [
                    {"label": "方案A", "description": "更稳", "recommended": true},
                    {"label": "方案B"}
                ]
            }]
        }))
        .unwrap();
        assert_eq!(qs.len(), 1);
        assert_eq!(qs[0].question_id, "q1");
        assert!(qs[0].allow_custom);
        assert_eq!(qs[0].options.len(), 2);
        assert!(qs[0].options[0].recommended);
        assert!(!qs[0].options[1].recommended);
        assert_eq!(qs[0].options[0].description.as_deref(), Some("更稳"));
        // allow_custom=false respected
        let qs = parse_questions(&json!({"questions": [{"question": "q", "allow_custom": false}]}))
            .unwrap();
        assert!(!qs[0].allow_custom);
    }

    /// format_response: answers map by question id; skip and empty both produce
    /// self-decide instructions.
    #[test]
    fn format_response_answers_skip_missing() {
        let qs = parse_questions(&json!({"questions": [{"question": "Q1"}, {"question": "Q2"}]}))
            .unwrap();
        let resp = QuestionResponse {
            skipped: false,
            answers: vec![
                crate::core::models::QuestionAnswer {
                    question_id: "q1".into(),
                    answer: "方案A".into(),
                    is_custom: false,
                },
                crate::core::models::QuestionAnswer {
                    question_id: "q2".into(),
                    answer: "自定义内容".into(),
                    is_custom: true,
                },
            ],
        };
        let text = format_response(&qs, &resp);
        assert!(text.contains("[用户回答]"), "{text}");
        assert!(
            text.contains("问题: Q1") && text.contains("回答: 方案A"),
            "{text}"
        );
        assert!(text.contains("回答: 自定义内容"), "{text}");

        let skip = QuestionResponse {
            skipped: true,
            answers: vec![],
        };
        let text = format_response(&qs, &skip);
        assert!(text.contains("[用户跳过了提问]"), "{text}");

        let partial = QuestionResponse {
            skipped: false,
            answers: vec![],
        };
        let text = format_response(&qs, &partial);
        assert!(text.contains("（未作答，请自行决定）"), "{text}");
    }
}
