//! 云端后端 - 鉴权相关端点
//!
//! 端点清单：
//! - `POST /api/auth/login`           — 用户名/密码登录
//! - `POST /api/auth/register`        — 邮箱+密码注册
//! - `GET  /api/auth/validate`        — 校验 JWT
//! - `POST /api/auth/update-avatar`   — 同步头像
//!
//! 所有命令返回 [`Result<T, String>`]（边界转换）以兼容 Tauri IPC。
//! 内部统一返回 [`crate::cloud_backend::Result<T>`]。

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::cloud_backend::client::{ensure_success, http_client, CloudBackendError, CbResult};
use crate::cloud_backend::config::api_url;
use crate::core::secure_store;

/// 云端登录成功响应
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginResponse {
    /// 用户唯一标识（UUID 字符串）
    pub id: Option<String>,
    /// 用户名
    pub username: String,
    /// 用户昵称
    pub nickname: Option<String>,
    /// 用户头像 URL 或路径
    pub avatar: Option<String>,
    /// 用于后续请求的 JWT 令牌
    pub token: String,
}

/// 将用户头像同步至云端
#[tauri::command]
pub async fn sync_avatar_to_backend(token: String, avatar_data: String) -> Result<(), String> {
    let client = http_client().map_err(|e| e.to_string())?;
    let res = client
        .post(api_url("/update-avatar"))
        .header("Authorization", format!("Bearer {}", token))
        .json(&serde_json::json!({ "avatar": avatar_data }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    ensure_success(res).await.map_err(|e| e.to_string())?;
    Ok(())
}

/// 用户名 / 密码登录；成功后 token 写入系统钥匙串
#[tauri::command]
pub async fn login_to_backend(
    app: AppHandle,
    username: String,
    password: String,
) -> std::result::Result<LoginResponse, String> {
    let client = http_client().map_err(|e| e.to_string())?;
    let res = client
        .post(api_url("/login"))
        .json(&serde_json::json!({
            "username": username,
            "password": password
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let resp = ensure_success(res).await.map_err(|e| e.to_string())?;
    let user_data: LoginResponse = resp.json().await.map_err(|e| e.to_string())?;

    // 持久化 token 到 keyring（不写 localStorage）
    secure_store::set(&app, secure_store::accounts::AUTH_TOKEN, &user_data.token)
        .map_err(|e| format!("保存 token 失败: {}", e))?;

    Ok(user_data)
}

/// 邮箱+密码注册
#[tauri::command]
pub async fn register_to_backend(
    email: String,
    password: String,
    confirm_password: String,
) -> std::result::Result<String, String> {
    let client = http_client().map_err(|e| e.to_string())?;
    let res = client
        .post(api_url("/register"))
        .json(&serde_json::json!({
            "email": email,
            "password": password,
            "confirmPassword": confirm_password
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    ensure_success(res).await.map_err(|e| e.to_string())?;
    Ok("注册成功".to_string())
}

/// 校验 token 有效性，返回当前用户信息
#[tauri::command]
pub async fn validate_token(token: String) -> std::result::Result<LoginResponse, String> {
    let client = http_client().map_err(|e| e.to_string())?;
    let res = client
        .get(api_url("/validate"))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let resp = ensure_success(res).await.map_err(|e| e.to_string())?;
    resp.json::<LoginResponse>().await.map_err(|e| e.to_string())
}

/// 显式登出：清空系统钥匙串中的 token
#[tauri::command]
pub fn logout_clear(app: AppHandle) -> std::result::Result<(), String> {
    secure_store::delete(&app, secure_store::accounts::AUTH_TOKEN)
        .map_err(|e| e.to_string())
}

/// 从钥匙串读取 token（前端不再读 localStorage）
#[tauri::command]
pub fn read_auth_token(app: AppHandle) -> std::result::Result<Option<String>, String> {
    secure_store::get(&app, secure_store::accounts::AUTH_TOKEN)
        .map_err(|e| e.to_string())
}

/// 内部：把 [`CbResult<T>`] 转换为 `Result<T, String>` 的边界适配器
#[allow(dead_code)]
pub fn to_user_err<T>(r: CbResult<T>) -> std::result::Result<T, String> {
    r.map_err(|e| match e {
        CloudBackendError::Server { status, message } => {
            format!("云端返回 HTTP {}: {}", status, message)
        }
        CloudBackendError::Request(_) => "网络异常，请检查网络或代理设置".to_string(),
        CloudBackendError::ClientBuild(_) => "本地 HTTP 客户端初始化失败".to_string(),
    })
}
