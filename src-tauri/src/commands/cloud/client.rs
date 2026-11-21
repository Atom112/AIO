//! HTTP client for the cloud API: Better Auth device flow, `/v1/me`, sync push/pull
//! and attachment presigning.
//!
//! Every call returns [`HttpError`] instead of a formatted string so the sync loop can
//! distinguish retryable transport/server failures (429/5xx/timeouts) from permanent
//! ones (400/401/404) without parsing messages.
use serde_json::Value;

use super::dto::{
    AnnouncementList, AttachmentDownloadResponse, AttachmentInitResponse, CloudAnnouncement,
    DeviceCodeResponse, DevicePoll, DeviceStart, DeviceTokenResponse, MeResponse, PullResponse,
    PushResponse, SnapshotResponse,
};

/// Failure from the cloud API. `status` is `None` for transport-level errors.
#[derive(Debug, Clone)]
pub(crate) struct HttpError {
    pub status: Option<u16>,
    pub message: String,
    /// Server error code from the `{ error: { code } }` envelope, when present.
    pub code: Option<String>,
}

impl HttpError {
    fn transport(error: impl std::fmt::Display) -> Self {
        Self {
            status: None,
            message: error.to_string(),
            code: None,
        }
    }

    /// Whether the sync loop should retry with backoff (429, 5xx, timeouts).
    pub(crate) fn retryable(&self) -> bool {
        match self.status {
            None => true,
            Some(status) => status == 429 || status >= 500,
        }
    }

    /// Whether the session is no longer valid (token revoked or expired).
    pub(crate) fn is_unauthorized(&self) -> bool {
        matches!(self.status, Some(401))
    }

    pub(crate) fn into_message(self) -> String {
        crate::ierr!(
            "云同步请求失败: {}",
            "Cloud request failed: {}",
            self.message
        )
    }
}

impl From<reqwest::Error> for HttpError {
    fn from(error: reqwest::Error) -> Self {
        Self::transport(error)
    }
}

fn api_base(server_url: &str) -> String {
    server_url.trim_end_matches('/').to_string()
}

/// Builds a URL with query parameters.
///
/// `reqwest`'s `query()` helper lives behind a feature flag this crate does not enable,
/// and building the URL explicitly keeps percent-encoding in one place (the `url` crate
/// is already a dependency).
pub(crate) fn with_query(base: &str, pairs: &[(&str, &str)]) -> String {
    let Ok(mut url) = url::Url::parse(base) else {
        return base.to_string();
    };
    {
        let mut query = url.query_pairs_mut();
        for (key, value) in pairs {
            query.append_pair(key, value);
        }
    }
    url.to_string()
}

/// Reads the error envelope `{ error: { code, message } }` and builds an [`HttpError`].
async fn to_http_error(response: reqwest::Response) -> HttpError {
    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    let parsed: Option<Value> = serde_json::from_str(&body).ok();
    let code = parsed
        .as_ref()
        .and_then(|v| v.pointer("/error/code"))
        .and_then(Value::as_str)
        .map(str::to_string);
    // OAuth device-flow errors use the flat `{ error: "…", error_description: "…" }` shape.
    let oauth_error = parsed
        .as_ref()
        .and_then(|v| v.get("error"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let message = parsed
        .as_ref()
        .and_then(|v| v.pointer("/error/message"))
        .and_then(Value::as_str)
        .or_else(|| {
            parsed
                .as_ref()
                .and_then(|v| v.get("error_description"))
                .and_then(Value::as_str)
        })
        .map(str::to_string)
        .unwrap_or_else(|| {
            if body.is_empty() {
                format!("HTTP {status}")
            } else {
                format!(
                    "HTTP {status}: {}",
                    body.chars().take(300).collect::<String>()
                )
            }
        });
    HttpError {
        status: Some(status),
        message,
        code: code.or(oauth_error),
    }
}

/// `POST /api/auth/device/code` — starts desktop sign-in and returns the user code.
pub(crate) async fn device_start(
    client: &reqwest::Client,
    server_url: &str,
) -> Result<DeviceStart, HttpError> {
    let url = format!("{}/api/auth/device/code", api_base(server_url));
    let response = client
        .post(url)
        .json(&serde_json::json!({ "client_id": super::CLIENT_ID }))
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(to_http_error(response).await);
    }
    let parsed: DeviceCodeResponse = response.json().await?;
    Ok(DeviceStart {
        device_code: parsed.device_code,
        user_code: parsed.user_code,
        verification_uri: parsed.verification_uri,
        verification_uri_complete: parsed.verification_uri_complete,
        expires_in: parsed.expires_in,
        interval: parsed.interval.max(1),
    })
}

/// `POST /api/auth/device/token` — polls the device-code grant once.
///
/// `Ok(DevicePoll)` describes the grant state (`pending`, `approved`, `denied`,
/// `expired`); transport/server faults are `Err` so the caller can back off.
pub(crate) async fn device_poll(
    client: &reqwest::Client,
    server_url: &str,
    device_code: &str,
) -> Result<DevicePoll, HttpError> {
    let url = format!("{}/api/auth/device/token", api_base(server_url));
    let response = client
        .post(url)
        .json(&serde_json::json!({
            "grant_type": "urn:ietf:params:oauth:grant-type:device_code",
            "device_code": device_code,
            "client_id": super::CLIENT_ID,
        }))
        .send()
        .await?;

    if response.status().is_success() {
        let token: DeviceTokenResponse = response.json().await?;
        return Ok(DevicePoll {
            status: "approved".to_string(),
            message: Some(token.access_token),
        });
    }

    let error = to_http_error(response).await;
    // RFC 8628 grant errors come back as a 400 with an OAuth `error` code.
    if error.status == Some(400) {
        let code = error.code.clone().unwrap_or_default();
        let status = match code.as_str() {
            "authorization_pending" | "slow_down" => "pending",
            "access_denied" => "denied",
            "expired_token" => "expired",
            _ => "error",
        };
        return Ok(DevicePoll {
            status: status.to_string(),
            message: if status == "pending" {
                None
            } else {
                Some(error.message)
            },
        });
    }
    Err(error)
}

/// `GET /v1/me` — user identity plus workspace memberships for the session token.
pub(crate) async fn fetch_me(
    client: &reqwest::Client,
    server_url: &str,
    token: &str,
) -> Result<MeResponse, HttpError> {
    let url = format!("{}/v1/me", api_base(server_url));
    let response = client.get(url).bearer_auth(token).send().await?;
    if !response.status().is_success() {
        return Err(to_http_error(response).await);
    }
    Ok(response.json().await?)
}

/// `GET /v1/announcements` — active notices for this account.
pub(crate) async fn fetch_announcements(
    client: &reqwest::Client,
    server_url: &str,
    token: &str,
) -> Result<Vec<CloudAnnouncement>, HttpError> {
    let url = format!("{}/v1/announcements", api_base(server_url));
    let response = client.get(url).bearer_auth(token).send().await?;
    if !response.status().is_success() {
        return Err(to_http_error(response).await);
    }
    let list: AnnouncementList = response.json().await?;
    Ok(list.items)
}

/// `POST /api/auth/sign-out` — best effort; a failure never blocks local sign-out.
pub(crate) async fn sign_out(client: &reqwest::Client, server_url: &str, token: &str) {
    let url = format!("{}/api/auth/sign-out", api_base(server_url));
    let _ = client
        .post(url)
        .bearer_auth(token)
        .json(&serde_json::json!({}))
        .send()
        .await;
}

/// `POST /v1/sync/push` with a bounded batch.
pub(crate) async fn push(
    client: &reqwest::Client,
    server_url: &str,
    token: &str,
    workspace_id: &str,
    device_id: &str,
    changes: &[Value],
) -> Result<PushResponse, HttpError> {
    let url = format!("{}/v1/sync/push", api_base(server_url));
    let body = serde_json::json!({
        "workspaceId": workspace_id,
        "deviceId": device_id,
        "changes": changes,
    });
    let response = client
        .post(url)
        .bearer_auth(token)
        .json(&body)
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(to_http_error(response).await);
    }
    Ok(response.json().await?)
}

/// `GET /v1/sync/pull` for one page of changes.
pub(crate) async fn pull(
    client: &reqwest::Client,
    server_url: &str,
    token: &str,
    workspace_id: &str,
    cursor: Option<&str>,
    limit: u32,
) -> Result<PullResponse, HttpError> {
    let limit_text = limit.to_string();
    let mut pairs = vec![
        ("workspaceId", workspace_id),
        ("limit", limit_text.as_str()),
    ];
    if let Some(cursor) = cursor {
        pairs.push(("cursor", cursor));
    }
    let url = with_query(&format!("{}/v1/sync/pull", api_base(server_url)), &pairs);
    let response = client.get(url).bearer_auth(token).send().await?;
    if !response.status().is_success() {
        return Err(to_http_error(response).await);
    }
    Ok(response.json().await?)
}

/// `GET /v1/sync/snapshot` for one snapshot page (recovery after a cursor expiry).
pub(crate) async fn snapshot(
    client: &reqwest::Client,
    server_url: &str,
    token: &str,
    workspace_id: &str,
    page: Option<&str>,
    limit: u32,
) -> Result<SnapshotResponse, HttpError> {
    let limit_text = limit.to_string();
    let mut pairs = vec![
        ("workspaceId", workspace_id),
        ("limit", limit_text.as_str()),
    ];
    if let Some(page) = page {
        pairs.push(("page", page));
    }
    let url = with_query(
        &format!("{}/v1/sync/snapshot", api_base(server_url)),
        &pairs,
    );
    let response = client.get(url).bearer_auth(token).send().await?;
    if !response.status().is_success() {
        return Err(to_http_error(response).await);
    }
    Ok(response.json().await?)
}

/// `POST /v1/attachments/init` — presigned upload target for one file.
pub(crate) async fn attachment_init(
    client: &reqwest::Client,
    server_url: &str,
    token: &str,
    workspace_id: &str,
    file: &UploadCandidate,
) -> Result<AttachmentInitResponse, HttpError> {
    let url = format!("{}/v1/attachments/init", api_base(server_url));
    let response = client
        .post(url)
        .bearer_auth(token)
        .header("x-workspace-id", workspace_id)
        .json(&serde_json::json!({
            "fileName": file.file_name,
            "mimeType": file.mime_type,
            "sizeBytes": file.size_bytes,
            "sha256": file.sha256,
        }))
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(to_http_error(response).await);
    }
    Ok(response.json().await?)
}

/// `POST /v1/attachments/:id/complete` after the bytes are uploaded.
pub(crate) async fn attachment_complete(
    client: &reqwest::Client,
    server_url: &str,
    token: &str,
    workspace_id: &str,
    attachment_id: &str,
) -> Result<(), HttpError> {
    let url = format!(
        "{}/v1/attachments/{}/complete",
        api_base(server_url),
        attachment_id
    );
    let response = client
        .post(url)
        .bearer_auth(token)
        .header("x-workspace-id", workspace_id)
        .json(&serde_json::json!({}))
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(to_http_error(response).await);
    }
    Ok(())
}

/// `GET /v1/attachments/:id/download` — presigned download target.
pub(crate) async fn attachment_download_url(
    client: &reqwest::Client,
    server_url: &str,
    token: &str,
    workspace_id: &str,
    attachment_id: &str,
) -> Result<AttachmentDownloadResponse, HttpError> {
    let url = format!(
        "{}/v1/attachments/{}/download",
        api_base(server_url),
        attachment_id
    );
    let response = client
        .get(url)
        .bearer_auth(token)
        .header("x-workspace-id", workspace_id)
        .send()
        .await?;
    if !response.status().is_success() {
        return Err(to_http_error(response).await);
    }
    Ok(response.json().await?)
}

/// Uploads bytes to a presigned URL, applying the headers the server signed.
pub(crate) async fn put_presigned(
    client: &reqwest::Client,
    upload_url: &str,
    headers: &std::collections::BTreeMap<String, String>,
    bytes: Vec<u8>,
) -> Result<(), HttpError> {
    let mut request = client.put(upload_url).body(bytes);
    for (key, value) in headers {
        request = request.header(key, value);
    }
    let response = request.send().await?;
    if !response.status().is_success() {
        return Err(to_http_error(response).await);
    }
    Ok(())
}

/// Downloads an object from a presigned URL into memory (attachments are size-capped).
pub(crate) async fn get_presigned(
    client: &reqwest::Client,
    url: &str,
) -> Result<Vec<u8>, HttpError> {
    let response = client.get(url).send().await?;
    if !response.status().is_success() {
        return Err(to_http_error(response).await);
    }
    Ok(response.bytes().await?.to_vec())
}

/// One local file queued for upload.
pub(crate) struct UploadCandidate {
    pub attachment_id: String,
    pub file_name: String,
    pub mime_type: String,
    pub size_bytes: i64,
    pub sha256: String,
    pub storage_path: String,
}
