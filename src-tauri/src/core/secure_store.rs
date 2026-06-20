//! # 跨平台安全凭据存储
//!
//! 使用 [`keyring`] crate 抽象各 OS 原生凭据管理器：
//! - **Windows**: Windows Credential Manager (`wincred`)
//! - **macOS**: Keychain (`security-framework`)
//! - **Linux**: Secret Service / `libsecret` (`secret-service-rs`)
//!
//! 在 GUI 不可用的环境（无桌面会话 / 无 dbus）下，调用可能失败。
//! 失败时降级到 AppData 中的纯 JSON 文件（仅用于开发/无桌面场景）。
//!
//! Service 标识：`com.loch.aio`
//! Account 命名空间：
//! - `auth-token`: 后端登录 JWT
//! - `app-api-url`: 全局 API URL（仅当用户选择加密存储时）
//! - `app-api-key`: 全局 API Key
//! - `provider-{provider_id}-api-key`: 每个 provider 的 API Key
//! - `mcp-server-{server_id}-env-{env_key}`: 每个 MCP server 的环境变量密钥

use base64::{engine::general_purpose, Engine as _};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

const SERVICE: &str = "com.loch.aio";
const FALLBACK_FILE: &str = "secure-store.json";

#[derive(Debug, thiserror::Error)]
pub enum SecureStoreError {
    #[error("keyring 错误: {0}")]
    Keyring(String),
    #[error("降级文件 I/O 错误: {0}")]
    FallbackIo(String),
    #[error("降级文件 JSON 错误: {0}")]
    FallbackJson(String),
}

impl From<String> for SecureStoreError {
    fn from(s: String) -> Self {
        SecureStoreError::Keyring(s)
    }
}

pub type Result<T> = std::result::Result<T, SecureStoreError>;

#[derive(Default, Serialize, Deserialize)]
struct FallbackStore {
    entries: HashMap<String, String>,
}

fn fallback_path(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join(FALLBACK_FILE))
}

fn load_fallback(app: &AppHandle) -> FallbackStore {
    let Some(p) = fallback_path(app) else { return FallbackStore::default() };
    if !p.exists() {
        return FallbackStore::default();
    }
    fs::read_to_string(&p)
        .ok()
        .and_then(|s| serde_json::from_str::<FallbackStore>(&s).ok())
        .unwrap_or_default()
}

fn save_fallback(app: &AppHandle, store: &FallbackStore) -> Result<()> {
    let p = fallback_path(app).ok_or_else(|| SecureStoreError::FallbackIo("无 AppData 路径".into()))?;
    if let Some(parent) = p.parent() {
        let _ = fs::create_dir_all(parent);
    }
    // 落地前做 base64 混淆以避免明文 keys 出现在文件搜索结果中
    let mut encoded = FallbackStore::default();
    for (k, v) in &store.entries {
        encoded.entries.insert(k.clone(), general_purpose::STANDARD.encode(v.as_bytes()));
    }
    let json = serde_json::to_string_pretty(&encoded).map_err(|e| SecureStoreError::FallbackJson(e.to_string()))?;
    fs::write(&p, json).map_err(|e| SecureStoreError::FallbackIo(e.to_string()))?;
    Ok(())
}

/// 在 keyring 中存/取/删 字符串
fn keyring_get(account: &str) -> Result<String> {
    let entry = keyring::Entry::new(SERVICE, account).map_err(|e| SecureStoreError::Keyring(e.to_string()))?;
    entry.get_password().map_err(|e| SecureStoreError::Keyring(e.to_string()))
}

fn keyring_set(account: &str, value: &str) -> Result<()> {
    let entry = keyring::Entry::new(SERVICE, account).map_err(|e| SecureStoreError::Keyring(e.to_string()))?;
    entry.set_password(value).map_err(|e| SecureStoreError::Keyring(e.to_string()))
}

fn keyring_delete(account: &str) -> Result<()> {
    let entry = keyring::Entry::new(SERVICE, account).map_err(|e| SecureStoreError::Keyring(e.to_string()))?;
    entry.delete_credential().map_err(|e| SecureStoreError::Keyring(e.to_string()))
}

/// 高层 API：存取值；keyring 不可用时降级到 AppData 文件
pub fn get(app: &AppHandle, account: &str) -> Result<Option<String>> {
    match keyring_get(account) {
        Ok(v) => Ok(Some(v)),
        Err(SecureStoreError::Keyring(_)) => {
            let store = load_fallback(app);
            if let Some(encoded) = store.entries.get(account) {
                if let Ok(bytes) = general_purpose::STANDARD.decode(encoded) {
                    if let Ok(s) = String::from_utf8(bytes) {
                        return Ok(Some(s));
                    }
                }
            }
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

pub fn set(app: &AppHandle, account: &str, value: &str) -> Result<()> {
    match keyring_set(account, value) {
        Ok(()) => {
            // 同步清理降级文件中的同 key，避免双写
            let mut store = load_fallback(app);
            if store.entries.remove(account).is_some() {
                let _ = save_fallback(app, &store);
            }
            Ok(())
        }
        Err(_) => {
            let mut store = load_fallback(app);
            store.entries.insert(account.to_string(), value.to_string());
            save_fallback(app, &store)
        }
    }
}

pub fn delete(app: &AppHandle, account: &str) -> Result<()> {
    let keyring_ok = match keyring_delete(account) {
        Ok(()) => true,
        Err(SecureStoreError::Keyring(_)) => false,
        Err(e) => return Err(e),
    };
    let mut store = load_fallback(app);
    if store.entries.remove(account).is_some() {
        save_fallback(app, &store)?;
    }
    let _ = keyring_ok;
    Ok(())
}

/// 已知 account 命名空间
pub mod accounts {
    pub const AUTH_TOKEN: &str = "auth-token";
    pub const APP_API_KEY: &str = "app-api-key";
    pub fn provider_key(id: &str) -> String {
        format!("provider-{}-api-key", id)
    }
    /// MCP server 环境变量密钥：${KEYRING:mcp-server-{server_id}-env-{env_key}}
    pub fn mcp_server_env(server_id: &str, env_key: &str) -> String {
        format!("mcp-server-{}-env-{}", server_id, env_key)
    }
    /// 删除某 MCP server 的所有密钥（删除 server 时调用）
    #[allow(dead_code)]
    pub fn mcp_server_prefix(server_id: &str) -> String {
        format!("mcp-server-{}-", server_id)
    }
}

/// 列出 keyring 中某前缀下的所有 account 名。
/// 用于删除 MCP server 时清理其所有环境变量。
/// 注：keyring crate 暂未提供"按前缀列出"API，这里返回空 vec；
/// 推荐做法是在删除 server 时由调用方显式传入要删除的 env 键列表。
#[allow(dead_code)]
pub fn list_with_prefix(_app: &AppHandle, _prefix: &str) -> Vec<String> {
    Vec::new()
}
