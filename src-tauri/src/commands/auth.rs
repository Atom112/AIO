/// 认证相关的 Tauri 命令：处理用户登录、注册、Token 验证以及头像同步等功能。

use serde::{Deserialize, Serialize};

/// 登录成功后的响应结构体
#[derive(Serialize, Deserialize)]
pub struct LoginResponse {
    /// 用户唯一标识（对应数据库中的 UUID 字符串）
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

/// 将用户头像同步至远程后端
/// 
/// Arguments
/// `token` - JWT 身份令牌
/// `avatar_data` - 处理后的头像数据（通常为 Base64 字符串）
#[tauri::command]
pub async fn sync_avatar_to_backend(token: String, avatar_data: String) -> Result<(), String> {
    let client = reqwest::Client::new();
    let res = client
        .post("http://localhost:8080/api/auth/update-avatar")
        .header("Authorization", format!("Bearer {}", token))
        .json(&serde_json::json!({ "avatar": avatar_data }))
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if res.status().is_success() {
        Ok(())
    } else {
        Err("同步头像失败".into())
    }
}

/// 调用后端登录接口
#[tauri::command]
pub async fn login_to_backend(username: String, password: String) -> Result<LoginResponse, String> {
    let client = reqwest::Client::new();

    let res = client
        .post("http://localhost:8080/api/auth/login")
        .json(&serde_json::json!({
            "username": username,
            "password": password
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if res.status().is_success() {
        let user_data = res
            .json::<LoginResponse>()
            .await
            .map_err(|e| e.to_string())?;
        Ok(user_data)
    } else {
        let err_msg = res.text().await.unwrap_or_else(|_| "登录失败".to_string());
        Err(err_msg)
    }
}

/// 用户注册功能
#[tauri::command]
pub async fn register_to_backend(
    email: String,
    password: String,
    confirm_password: String,
) -> Result<String, String> {
    let client = reqwest::Client::new();

    let res = client
        .post("http://localhost:8080/api/auth/register")
        .json(&serde_json::json!({
            "email": email,
            "password": password,
            "confirmPassword": confirm_password
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if res.status().is_success() {
        Ok("注册成功".to_string())
    } else {
        let err_msg = res.text().await.unwrap_or_else(|_| "注册失败".to_string());
        Err(err_msg)
    }
}

/// 验证现有 Token 的有效性并获取用户信息
#[tauri::command]
pub async fn validate_token(token: String) -> Result<LoginResponse, String> {
    let client = reqwest::Client::new();

    let res = client
        .get("http://localhost:8080/api/auth/validate")
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if res.status().is_success() {
        let user_data = res
            .json::<LoginResponse>()
            .await
            .map_err(|e| e.to_string())?;
        Ok(user_data)
    } else {
        Err("Token 已过期".to_string())
    }
}