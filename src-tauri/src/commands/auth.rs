// 在 lib.rs 或单独的 auth.rs 中
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct LoginResponse {
    // 关键修改：将 id 从 Option<u64> 改为 Option<String>
    // 因为你的数据库现在使用的是 VARCHAR(100) 存储 UUID 字符串
    pub id: Option<String>,
    pub username: String,
    pub nickname: Option<String>,
    pub avatar: Option<String>,
    pub token: String,
}

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

#[tauri::command]
pub async fn login_to_backend(username: String, password: String) -> Result<LoginResponse, String> {
    let client = reqwest::Client::new();

    // 注意：这里的 URL 要和你的 Java 后端对应
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
// 在 auth.rs 中添加此函数
#[tauri::command]
pub async fn validate_token(token: String) -> Result<LoginResponse, String> {
    let client = reqwest::Client::new();

    // 假设你的 Java 后端有一个 /api/auth/me 或者类似的验证接口
    let res = client
        .get("http://localhost:8080/api/auth/validate")
        .header("Authorization", format!("Bearer {}", token)) // 常见的 Bearer Token 格式
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
        // 如果后端返回 401 或其他错误码，说明 Token 失效
        Err("Token 已过期".to_string())
    }
}
