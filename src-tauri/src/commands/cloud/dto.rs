//! Wire DTOs for the AIO cloud sync API (`/v1/*`) and Better Auth device flow.
//!
//! Two families: outbound payloads built with `serde_json::json!` from local rows
//! (see `outbox.rs`) and inbound DTOs deserialized here. Inbound types keep every
//! optional field explicit so a server-side addition never breaks an older client.
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One workspace entry from `GET /v1/me`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSummary {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub slug: String,
    pub role: String,
}

/// `GET /v1/me` response (account bootstrap after device sign-in).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeResponse {
    pub user: MeUser,
    #[serde(default)]
    pub workspaces: Vec<WorkspaceSummary>,
    #[serde(default)]
    pub server_version: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeUser {
    pub id: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub name: String,
}

/// `POST /api/auth/device/code` response.
#[derive(Debug, Clone, Deserialize)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    #[serde(default)]
    pub verification_uri_complete: Option<String>,
    #[serde(default)]
    pub expires_in: i64,
    #[serde(default)]
    pub interval: i64,
}

/// Successful `POST /api/auth/device/token` response.
#[derive(Debug, Clone, Deserialize)]
pub struct DeviceTokenResponse {
    pub access_token: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceStart {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    pub expires_in: i64,
    pub interval: i64,
}

/// Result of one device-code poll.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DevicePoll {
    /// `pending` | `approved` | `denied` | `expired` | `error`
    pub status: String,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushResponse {
    #[serde(default)]
    pub applied: Vec<AppliedChange>,
    #[serde(default)]
    pub conflicts: Vec<ConflictChange>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppliedChange {
    #[serde(rename = "type")]
    pub change_type: String,
    pub id: String,
    #[serde(default)]
    pub seq: Option<i64>,
    #[serde(default)]
    pub version: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictChange {
    pub id: String,
    pub server_version: i64,
    #[serde(default)]
    pub payload: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullResponse {
    #[serde(default)]
    pub changes: Vec<RemoteChange>,
    #[serde(default)]
    pub next_cursor: Option<i64>,
    #[serde(default)]
    pub has_more: bool,
}

/// One server change (payload present for upserts).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteChange {
    #[serde(default)]
    pub cursor: Option<i64>,
    #[serde(rename = "type")]
    pub change_type: String,
    pub id: String,
    #[serde(default)]
    pub op: String,
    #[serde(default)]
    pub version: i64,
    #[serde(default)]
    pub payload: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotResponse {
    #[serde(default)]
    pub entities: Vec<SnapshotEntity>,
    #[serde(default)]
    pub next_page: Option<String>,
    #[serde(default)]
    pub has_more: bool,
    #[serde(default)]
    pub snapshot_cursor: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotEntity {
    #[serde(rename = "type")]
    pub entity_type: String,
    pub id: String,
    #[serde(default)]
    pub version: i64,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default)]
    pub payload: Option<Value>,
}

// ----- Remote entity payloads (applied to SQLite) -------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteAssistant {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub mcp_server_ids: Vec<String>,
    #[serde(default)]
    pub skill_ids: Vec<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub agent_mode: Option<String>,
    #[serde(default)]
    pub assistant_type: Option<String>,
    #[serde(default)]
    pub version: i64,
    #[serde(default)]
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteThread {
    pub id: String,
    pub assistant_id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub context_trim_count: i64,
    #[serde(default)]
    pub branched_from_message_id: Option<String>,
    #[serde(default)]
    pub parent_thread_id: Option<String>,
    #[serde(default)]
    pub version: i64,
    #[serde(default)]
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteMessage {
    pub id: String,
    pub thread_id: String,
    #[serde(default)]
    pub seq: i64,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub content: Value,
    #[serde(default)]
    pub display_text: Option<String>,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub reasoning: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Value>,
    #[serde(default)]
    pub parent_message_id: Option<String>,
    #[serde(default)]
    pub branch_index: i64,
    #[serde(default)]
    pub input_tokens: Option<i64>,
    #[serde(default)]
    pub output_tokens: Option<i64>,
    #[serde(default)]
    pub cached_input_tokens: Option<i64>,
    #[serde(default)]
    pub context_tokens: Option<i64>,
    #[serde(default)]
    pub ttft_ms: Option<i64>,
    #[serde(default)]
    pub version: i64,
    #[serde(default)]
    pub created_at: Option<String>,
    /// Server-side attachment metadata referenced by this message.
    #[serde(default)]
    pub attachments: Vec<RemoteAttachmentRef>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteAttachmentRef {
    pub id: String,
    #[serde(default)]
    pub file_name: String,
    #[serde(default)]
    pub mime_type: String,
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub size_bytes: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentInitResponse {
    pub attachment_id: String,
    pub upload_url: String,
    #[serde(default)]
    pub headers: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentDownloadResponse {
    pub url: String,
}

/// A change to push (built in `outbox.rs`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutboundChange {
    pub change_id: String,
    #[serde(rename = "type")]
    pub change_type: String,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_version: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_order: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// One active announcement served by the team backend.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudAnnouncement {
    pub id: String,
    pub severity: String,
    pub scope: String,
    #[serde(default)]
    pub workspace_id: Option<String>,
    pub title: String,
    pub body: String,
    pub starts_at: String,
    #[serde(default)]
    pub ends_at: Option<String>,
}

/// `GET /v1/announcements` envelope.
#[derive(Debug, Clone, Deserialize)]
pub struct AnnouncementList {
    #[serde(default)]
    pub items: Vec<CloudAnnouncement>,
}
