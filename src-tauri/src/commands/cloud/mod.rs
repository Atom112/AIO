//! Cloud account and conversation sync client (`cloud_*` commands).
//!
//! The desktop stays authoritative for local data: this module only mirrors local
//! changes to a self-hosted AIO cloud server and applies changes coming back. Every
//! command degrades cleanly when no account is linked, so the app keeps working offline.
//!
//! Non-negotiables encoded here:
//! - The session token lives only in the OS keychain (`secure_store`), never in SQLite.
//! - Local mutations never fail because of sync bookkeeping (`enqueue_best_effort`).
//! - Server URLs are validated before any request is sent (HTTPS unless loopback).
//! - The DB lock is never held across an `.await` (see `sync_ops`).
mod apply;
mod attachments;
pub(crate) mod client;
mod dto;
pub(crate) mod outbox;
mod payloads;
pub(crate) mod runs;
mod stream;
mod sync_ops;

use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Manager};
use tokio_util::sync::CancellationToken;

use crate::core::models::StoredAttachment;
use crate::core::secure_store;
use crate::utils::url_validation::{validate_http_url, HttpUrlOptions};
use apply::ApplyOutcome;
use dto::{
    AcceptedInvitation, CloudAnnouncement, CloudInvitation, DevicePoll, DeviceStart, MeResponse,
    RemoteAttachmentRef, RemoteChange,
};
use outbox::Account;

/// OAuth client id the desktop presents during device authorization.
pub(crate) const CLIENT_ID: &str = "aio-desktop";

/// Managed state: the shared HTTP client plus the live SSE task.
pub struct CloudState {
    pub(crate) client: reqwest::Client,
    events: Mutex<Option<(tokio::task::JoinHandle<()>, CancellationToken)>>,
    /// Active cloud-run event stream: run id plus its task and cancellation token.
    pub(crate) run_events: Mutex<Option<(String, tokio::task::JoinHandle<()>, CancellationToken)>>,
}

impl CloudState {
    /// Builds the shared client with the default AIO user agent and a request timeout
    /// long enough for a 200-change push but short enough to fail fast offline.
    pub fn new() -> Self {
        let client = crate::core::http::client_builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_default();
        Self {
            client,
            events: Mutex::new(None),
            run_events: Mutex::new(None),
        }
    }
}

impl Default for CloudState {
    fn default() -> Self {
        Self::new()
    }
}

/// Snapshot of the linked account for the settings card.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudSessionStatus {
    pub signed_in: bool,
    pub server_url: Option<String>,
    pub workspace_id: Option<String>,
    pub user_id: Option<String>,
    pub user_login: Option<String>,
    pub device_id: Option<String>,
    pub last_cursor: Option<String>,
    pub pending: i64,
}

/// Result of one push tick. Protocol hiccups are reported as data, not errors, so the
/// sync loop can back off without treating them as failures of the local database.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PushOutcome {
    pub applied: usize,
    pub conflicts: usize,
    pub uploaded: usize,
    pub pending: i64,
    pub retryable: bool,
    pub status: Option<u16>,
    pub message: Option<String>,
    pub signed_out: bool,
}

/// One pulled page plus the protocol-relevant error state.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullOutcome {
    pub changes: Vec<RemoteChange>,
    pub next_cursor: Option<i64>,
    pub has_more: bool,
    /// `true` when the cursor aged out (server answered `410`): full resync required.
    pub cursor_expired: bool,
    pub retryable: bool,
    pub status: Option<u16>,
    pub message: Option<String>,
    pub signed_out: bool,
}

/// One snapshot page, already applied locally.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotOutcome {
    pub applied: usize,
    pub changed_topic_ids: Vec<String>,
    pub next_page: Option<String>,
    pub has_more: bool,
    pub snapshot_cursor: Option<i64>,
    pub retryable: bool,
    pub status: Option<u16>,
    pub message: Option<String>,
    pub signed_out: bool,
}

fn session_token(app: &AppHandle) -> Option<String> {
    secure_store::get(app, &secure_store::accounts::cloud_session_token())
        .ok()
        .flatten()
}

fn store_session_token(app: &AppHandle, token: &str) -> Result<(), String> {
    secure_store::set(app, &secure_store::accounts::cloud_session_token(), token)
        .map_err(|e| e.to_string())
}

fn status_from_account(
    account: Option<Account>,
    token_present: bool,
    pending: i64,
) -> CloudSessionStatus {
    match account {
        Some(account) => CloudSessionStatus {
            signed_in: token_present,
            server_url: Some(account.server_url),
            workspace_id: Some(account.workspace_id),
            user_id: Some(account.user_id),
            user_login: Some(account.user_login),
            device_id: Some(account.device_id),
            last_cursor: account.last_cursor,
            pending,
        },
        None => CloudSessionStatus {
            // A token without a bound workspace is the normal state right after device
            // sign-in: the UI must offer workspace selection, not another sign-in.
            signed_in: token_present,
            server_url: None,
            workspace_id: None,
            user_id: None,
            user_login: None,
            device_id: None,
            last_cursor: None,
            pending,
        },
    }
}

/// Validates a server URL: HTTPS everywhere except loopback, where plain HTTP is
/// required by design (a self-hosted dev server has no certificate).
fn validate_server_url(raw: &str) -> Result<String, String> {
    let parsed = url::Url::parse(raw.trim())
        .map_err(|e| crate::ierr!("服务器地址无效: {}", "Invalid server URL: {}", e))?;
    let is_loopback = matches!(
        parsed.host_str(),
        Some("localhost") | Some("127.0.0.1") | Some("[::1]")
    );
    let options = if is_loopback {
        HttpUrlOptions {
            allow_http: true,
            allow_localhost: true,
        }
    } else {
        HttpUrlOptions::https_only()
    };
    let normalized = validate_http_url(raw, &options)?;
    Ok(normalized.trim_end_matches('/').to_string())
}

/// Starts GitHub device authorization: returns the user code and verification URL.
#[tauri::command]
pub async fn cloud_device_start(app: AppHandle, server_url: String) -> Result<DeviceStart, String> {
    let base = validate_server_url(&server_url)?;
    let state = app.state::<CloudState>();
    client::device_start(&state.client, &base)
        .await
        .map_err(client::HttpError::into_message)
}

/// Polls the device grant once. On approval the session token is stored in the keychain
/// and the caller proceeds to workspace selection.
#[tauri::command]
pub async fn cloud_device_poll(
    app: AppHandle,
    server_url: String,
    device_code: String,
) -> Result<DevicePoll, String> {
    let base = validate_server_url(&server_url)?;
    let poll = {
        let state = app.state::<CloudState>();
        client::device_poll(&state.client, &base, &device_code)
            .await
            .map_err(client::HttpError::into_message)?
    };
    if poll.status == "approved" {
        if let Some(token) = poll.message.as_deref() {
            store_session_token(&app, token)?;
        }
        return Ok(DevicePoll {
            status: "approved".to_string(),
            message: None,
        });
    }
    Ok(poll)
}

/// Lists the workspaces this session can bind to (`GET /v1/me`).
#[tauri::command]
pub async fn cloud_list_workspaces(
    app: AppHandle,
    server_url: String,
) -> Result<MeResponse, String> {
    let base = validate_server_url(&server_url)?;
    let token = session_token(&app)
        .ok_or_else(|| crate::ierr!("尚未登录云同步", "Not signed in to cloud sync"))?;
    let me = {
        let state = app.state::<CloudState>();
        let result = client::fetch_me(&state.client, &base, &token).await;
        result.map_err(client::HttpError::into_message)
    }?;
    Ok(me)
}

/// Binds this installation to one workspace (Phase 1: one workspace per profile).
///
/// The device id is generated once and kept, so re-binding preserves the same device
/// registration and its model key.
#[tauri::command]
pub async fn cloud_bind_workspace(
    app: AppHandle,
    server_url: String,
    workspace_id: String,
) -> Result<CloudSessionStatus, String> {
    let base = validate_server_url(&server_url)?;
    let token = session_token(&app)
        .ok_or_else(|| crate::ierr!("尚未登录云同步", "Not signed in to cloud sync"))?;
    let me = {
        let state = app.state::<CloudState>();
        client::fetch_me(&state.client, &base, &token)
            .await
            .map_err(client::HttpError::into_message)?
    };
    let workspace = me
        .workspaces
        .iter()
        .find(|workspace| workspace.id == workspace_id)
        .ok_or_else(|| {
            crate::ierr!(
                "工作区不存在或无访问权限",
                "Workspace not found or not accessible"
            )
        })?;

    let existing = sync_ops::account(&app)?;
    let device_id = existing
        .as_ref()
        .map(|account| account.device_id.clone())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let account = Account {
        server_url: base,
        workspace_id: workspace.id.clone(),
        user_id: me.user.id.clone(),
        user_login: if me.user.name.is_empty() {
            me.user.email.clone()
        } else {
            me.user.name.clone()
        },
        device_id,
        last_cursor: None,
    };
    sync_ops::bind(&app, &account)?;
    Ok(status_from_account(
        Some(account),
        true,
        sync_ops::pending_count(&app),
    ))
}

/// Current link state (used on startup and after sign-in/out).
#[tauri::command]
pub async fn cloud_session_status(app: AppHandle) -> Result<CloudSessionStatus, String> {
    let account = sync_ops::account(&app)?;
    let pending = sync_ops::pending_count(&app);
    let token_present = session_token(&app).is_some();
    Ok(status_from_account(account, token_present, pending))
}

/// Active announcements for the linked account. Returns an empty list when the
/// account is not linked (or has no session token), never an error: announcements
/// are advisory, so a signed-out desktop simply shows none.
#[tauri::command]
pub async fn cloud_list_announcements(app: AppHandle) -> Result<Vec<CloudAnnouncement>, String> {
    let Some(account) = sync_ops::account(&app)? else {
        return Ok(Vec::new());
    };
    let Some(token) = session_token(&app) else {
        return Ok(Vec::new());
    };
    let state = app.state::<CloudState>();
    client::fetch_announcements(&state.client, &account.server_url, &token)
        .await
        .map_err(client::HttpError::into_message)
}

/// Pending workspace invitations for the linked account. Empty (not an error)
/// when no account is linked, so the settings card renders nothing extra.
#[tauri::command]
pub async fn cloud_list_invitations(app: AppHandle) -> Result<Vec<CloudInvitation>, String> {
    let Some(account) = sync_ops::account(&app)? else {
        return Ok(Vec::new());
    };
    let Some(token) = session_token(&app) else {
        return Ok(Vec::new());
    };
    let state = app.state::<CloudState>();
    client::fetch_invitations(&state.client, &account.server_url, &token)
        .await
        .map_err(client::HttpError::into_message)
}

/// Joins the workspace behind an invitation. The caller refreshes the workspace
/// list afterwards; this command only performs the accept.
#[tauri::command]
pub async fn cloud_accept_invitation(
    app: AppHandle,
    invitation_id: String,
) -> Result<AcceptedInvitation, String> {
    let account = sync_ops::account(&app)?
        .ok_or_else(|| crate::ierr!("尚未登录云同步", "Not signed in to cloud sync"))?;
    let token = session_token(&app)
        .ok_or_else(|| crate::ierr!("尚未登录云同步", "Not signed in to cloud sync"))?;
    let state = app.state::<CloudState>();
    client::accept_invitation(&state.client, &account.server_url, &token, &invitation_id)
        .await
        .map_err(client::HttpError::into_message)
}

/// Declines an invitation to the caller's account.
#[tauri::command]
pub async fn cloud_reject_invitation(app: AppHandle, invitation_id: String) -> Result<(), String> {
    let account = sync_ops::account(&app)?
        .ok_or_else(|| crate::ierr!("尚未登录云同步", "Not signed in to cloud sync"))?;
    let token = session_token(&app)
        .ok_or_else(|| crate::ierr!("尚未登录云同步", "Not signed in to cloud sync"))?;
    let state = app.state::<CloudState>();
    client::reject_invitation(&state.client, &account.server_url, &token, &invitation_id)
        .await
        .map_err(client::HttpError::into_message)
}

/// Signs out: revokes the server session (best effort), deletes the keychain token and
/// clears sync bookkeeping. Local conversations are never touched.
#[tauri::command]
pub async fn cloud_logout(app: AppHandle) -> Result<(), String> {
    if let Ok(Some(account)) = sync_ops::account(&app) {
        if let Some(token) = session_token(&app) {
            let state = app.state::<CloudState>();
            client::sign_out(&state.client, &account.server_url, &token).await;
        }
    }
    stop_events(&app);
    let _ = secure_store::delete(&app, &secure_store::accounts::cloud_session_token());
    sync_ops::reset_local(&app)?;
    Ok(())
}

/// Flushes the outbox: uploads due attachments, then pushes one bounded batch.
#[tauri::command]
pub async fn cloud_sync_push(app: AppHandle) -> Result<PushOutcome, String> {
    let Some(account) = sync_ops::account(&app)? else {
        return Ok(PushOutcome {
            applied: 0,
            conflicts: 0,
            uploaded: 0,
            pending: 0,
            retryable: false,
            status: None,
            message: None,
            signed_out: true,
        });
    };
    let Some(token) = session_token(&app) else {
        return Ok(PushOutcome {
            applied: 0,
            conflicts: 0,
            uploaded: 0,
            pending: sync_ops::pending_count(&app),
            retryable: false,
            status: Some(401),
            message: None,
            signed_out: true,
        });
    };

    let (uploaded, upload_error) = {
        let state = app.state::<CloudState>();
        let client_http = state.client.clone();
        attachments::upload_pending(
            &app,
            &client_http,
            &account.server_url,
            &token,
            &account.workspace_id,
        )
        .await
    };

    let batch = sync_ops::queued_batch(&app)?;
    if batch.is_empty() {
        let pending = sync_ops::pending_count(&app);
        return Ok(PushOutcome {
            applied: 0,
            conflicts: 0,
            uploaded,
            pending,
            retryable: upload_error
                .as_ref()
                .map(client::HttpError::retryable)
                .unwrap_or(false),
            status: upload_error.as_ref().and_then(|error| error.status),
            message: upload_error.map(|error| error.message),
            signed_out: false,
        });
    }

    let changes: Vec<serde_json::Value> = batch
        .iter()
        .map(|change| serde_json::to_value(change).unwrap_or(serde_json::Value::Null))
        .collect();

    let response = {
        let state = app.state::<CloudState>();
        client::push(
            &state.client,
            &account.server_url,
            &token,
            &account.workspace_id,
            &account.device_id,
            &changes,
        )
        .await
    };

    match response {
        Ok(response) => {
            let (applied, conflicts) = sync_ops::record_push_result(&app, &response)?;
            // Payload artifacts upload only after their parent message was accepted.
            let _ = upload_pending_payloads(&app, &account, &token).await;
            Ok(PushOutcome {
                applied,
                conflicts,
                uploaded,
                pending: sync_ops::pending_count(&app),
                retryable: false,
                status: None,
                message: None,
                signed_out: false,
            })
        }
        Err(error) => {
            let retryable = error.retryable();
            let signed_out = error.is_unauthorized();
            Ok(PushOutcome {
                applied: 0,
                conflicts: 0,
                uploaded,
                pending: sync_ops::pending_count(&app),
                retryable,
                status: error.status,
                message: Some(error.message),
                signed_out,
            })
        }
    }
}

/// Uploads queued payload artifacts whose parent message already reached the server.
async fn upload_pending_payloads(
    app: &AppHandle,
    account: &Account,
    token: &str,
) -> Result<(), String> {
    const MAX_PAYLOADS_PER_TICK: usize = 3;
    let pending = {
        let state = app.state::<crate::core::state::DbState>();
        let conn = state.0.lock();
        payloads::pending_uploads(&conn, MAX_PAYLOADS_PER_TICK)?
    };
    for (message_id, kind) in pending {
        let json = {
            let state = app.state::<crate::core::state::DbState>();
            let conn = state.0.lock();
            payloads::local_payload(&conn, &message_id, &kind)?
        };
        let Some(json) = json else {
            continue;
        };
        let state = app.state::<CloudState>();
        if payloads::upload_one(
            &state.client,
            &account.server_url,
            token,
            &message_id,
            &kind,
            &json,
        )
        .await
        .is_ok()
        {
            let state = app.state::<crate::core::state::DbState>();
            let conn = state.0.lock();
            payloads::mark_uploaded(&conn, &message_id, &kind)?;
        }
    }
    Ok(())
}

/// Pulls one page of server changes (the caller applies them with `cloud_sync_apply`).
#[tauri::command]
pub async fn cloud_sync_pull(
    app: AppHandle,
    cursor: Option<String>,
    limit: Option<u32>,
) -> Result<PullOutcome, String> {
    let empty =
        |retryable: bool, status: Option<u16>, message: Option<String>, signed_out: bool| {
            PullOutcome {
                changes: Vec::new(),
                next_cursor: None,
                has_more: false,
                cursor_expired: false,
                retryable,
                status,
                message,
                signed_out,
            }
        };

    let Some(account) = sync_ops::account(&app)? else {
        return Ok(empty(false, None, None, true));
    };
    let Some(token) = session_token(&app) else {
        return Ok(empty(false, Some(401), None, true));
    };
    let effective_cursor = cursor.or(account.last_cursor.clone());

    let response = {
        let state = app.state::<CloudState>();
        client::pull(
            &state.client,
            &account.server_url,
            &token,
            &account.workspace_id,
            effective_cursor.as_deref(),
            limit.unwrap_or(200),
        )
        .await
    };

    match response {
        Ok(page) => Ok(PullOutcome {
            changes: page.changes,
            next_cursor: page.next_cursor,
            has_more: page.has_more,
            cursor_expired: false,
            retryable: false,
            status: None,
            message: None,
            signed_out: false,
        }),
        Err(error) => {
            let cursor_expired = error.status == Some(410);
            let retryable = error.retryable();
            let signed_out = error.is_unauthorized();
            let mut outcome = empty(retryable, error.status, Some(error.message), signed_out);
            outcome.cursor_expired = cursor_expired;
            Ok(outcome)
        }
    }
}

/// Applies pulled changes locally, advances the cursor in the same transaction, and
/// reports which topics changed (the frontend refreshes only those).
#[tauri::command]
pub async fn cloud_sync_apply(
    app: AppHandle,
    changes: Vec<RemoteChange>,
    cursor: Option<i64>,
) -> Result<ApplyOutcome, String> {
    let cursor_text = cursor.map(|value| value.to_string());
    sync_ops::apply_pulled(&app, &changes, cursor_text.as_deref())
}

/// Fetches and applies one snapshot page (recovery path after `410 cursor_expired`).
#[tauri::command]
pub async fn cloud_sync_snapshot(
    app: AppHandle,
    page: Option<String>,
    limit: Option<u32>,
) -> Result<SnapshotOutcome, String> {
    let empty =
        |retryable: bool, status: Option<u16>, message: Option<String>, signed_out: bool| {
            SnapshotOutcome {
                applied: 0,
                changed_topic_ids: Vec::new(),
                next_page: None,
                has_more: false,
                snapshot_cursor: None,
                retryable,
                status,
                message,
                signed_out,
            }
        };

    let Some(account) = sync_ops::account(&app)? else {
        return Ok(empty(false, None, None, true));
    };
    let Some(token) = session_token(&app) else {
        return Ok(empty(false, Some(401), None, true));
    };

    let response = {
        let state = app.state::<CloudState>();
        client::snapshot(
            &state.client,
            &account.server_url,
            &token,
            &account.workspace_id,
            page.as_deref(),
            limit.unwrap_or(100),
        )
        .await
    };

    match response {
        Ok(page) => {
            let outcome = sync_ops::apply_snapshot_page(&app, &page.entities)?;
            Ok(SnapshotOutcome {
                applied: outcome.applied,
                changed_topic_ids: outcome.changed_topic_ids,
                next_page: page.next_page,
                has_more: page.has_more,
                snapshot_cursor: page.snapshot_cursor,
                retryable: false,
                status: None,
                message: None,
                signed_out: false,
            })
        }
        Err(error) => {
            let retryable = error.retryable();
            let signed_out = error.is_unauthorized();
            Ok(empty(
                retryable,
                error.status,
                Some(error.message),
                signed_out,
            ))
        }
    }
}

/// Starts the SSE change feed (idempotent: an existing feed is left running).
#[tauri::command]
pub async fn cloud_start_events(app: AppHandle) -> Result<(), String> {
    if sync_ops::account(&app)?.is_none() {
        return Ok(());
    }
    let state = app.state::<CloudState>();
    let mut guard = state.events.lock();
    if guard.is_some() {
        return Ok(());
    }
    let token = CancellationToken::new();
    let handle = stream::spawn(app.clone(), state.client.clone(), token.clone());
    *guard = Some((handle, token));
    Ok(())
}

/// Stops the SSE change feed (no-op when it is not running).
#[tauri::command]
pub async fn cloud_stop_events(app: AppHandle) -> Result<(), String> {
    stop_events(&app);
    Ok(())
}

fn stop_events(app: &AppHandle) {
    let state = app.state::<CloudState>();
    let mut guard = state.events.lock();
    if let Some((handle, token)) = guard.take() {
        token.cancel();
        handle.abort();
    }
}

/// Uploads the agent-steps or full-tool-result payload for one message.
///
/// Payloads are gzip JSON objects: they are too large for the sync feed and are fetched
/// lazily by other devices.
#[tauri::command]
pub async fn cloud_upload_message_payload(
    app: AppHandle,
    message_id: String,
    kind: String,
) -> Result<(), String> {
    let Some(account) = sync_ops::account(&app)? else {
        return Ok(());
    };
    let Some(token) = session_token(&app) else {
        return Ok(());
    };
    let json = {
        let state = app.state::<crate::core::state::DbState>();
        let conn = state.0.lock();
        payloads::local_payload(&conn, &message_id, &kind)?
    };
    let Some(json) = json else {
        return Ok(());
    };
    let state = app.state::<CloudState>();
    let result = payloads::upload_one(
        &state.client,
        &account.server_url,
        &token,
        &message_id,
        &kind,
        &json,
    )
    .await;
    match result {
        Ok(()) => {
            let state = app.state::<crate::core::state::DbState>();
            let conn = state.0.lock();
            payloads::mark_uploaded(&conn, &message_id, &kind)
        }
        Err(error) => Err(error.into_message()),
    }
}

/// Fetches a payload from the server and merges it into the local message.
///
/// Returns the JSON so the UI can update the store without another read.
#[tauri::command]
pub async fn cloud_get_message_payload(
    app: AppHandle,
    message_id: String,
    kind: String,
) -> Result<String, String> {
    let Some(account) = sync_ops::account(&app)? else {
        return Err(crate::ierr!(
            "尚未登录云同步",
            "Not signed in to cloud sync"
        ));
    };
    let Some(token) = session_token(&app) else {
        return Err(crate::ierr!(
            "尚未登录云同步",
            "Not signed in to cloud sync"
        ));
    };
    let state = app.state::<CloudState>();
    let json = payloads::fetch_one(
        &state.client,
        &account.server_url,
        &token,
        &message_id,
        &kind,
    )
    .await
    .map_err(client::HttpError::into_message)?;

    {
        let state = app.state::<crate::core::state::DbState>();
        let conn = state.0.lock();
        match kind.as_str() {
            payloads::KIND_AGENT_STEPS => payloads::store_agent_steps(&conn, &message_id, &json)?,
            payloads::KIND_TOOL_FULL_RESULT => {
                payloads::merge_tool_full_results(&conn, &message_id, &json)?
            }
            _ => {}
        }
    }
    Ok(json)
}

/// Uploads queued attachments now and returns how many were transferred.
#[tauri::command]
pub async fn cloud_attachment_upload(app: AppHandle) -> Result<usize, String> {
    let Some(account) = sync_ops::account(&app)? else {
        return Ok(0);
    };
    let Some(token) = session_token(&app) else {
        return Err(crate::ierr!(
            "尚未登录云同步",
            "Not signed in to cloud sync"
        ));
    };
    let (uploaded, error) = {
        let state = app.state::<CloudState>();
        let client_http = state.client.clone();
        attachments::upload_pending(
            &app,
            &client_http,
            &account.server_url,
            &token,
            &account.workspace_id,
        )
        .await
    };
    if let Some(error) = error {
        return Err(error.into_message());
    }
    Ok(uploaded)
}

/// Downloads a remote attachment into the local store and returns the local reference.
#[tauri::command]
pub async fn cloud_attachment_download(
    app: AppHandle,
    attachment: RemoteAttachmentRef,
) -> Result<StoredAttachment, String> {
    let Some(account) = sync_ops::account(&app)? else {
        return Err(crate::ierr!(
            "尚未登录云同步",
            "Not signed in to cloud sync"
        ));
    };
    let Some(token) = session_token(&app) else {
        return Err(crate::ierr!(
            "尚未登录云同步",
            "Not signed in to cloud sync"
        ));
    };
    let state = app.state::<CloudState>();
    let client_http = state.client.clone();
    attachments::download(
        &app,
        &client_http,
        &account.server_url,
        &token,
        &account.workspace_id,
        &attachment,
    )
    .await
}
