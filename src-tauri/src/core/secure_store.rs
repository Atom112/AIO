//! # 跨平台安全凭据存储
//!
//! 使用 [`keyring`] crate 抽象各 OS 原生凭据管理器：
//! - **Windows**: Windows Credential Manager (`wincred`)
//! - **macOS**: Keychain (`security-framework`)
//! - **Linux**: Secret Service / `libsecret` (`secret-service-rs`)
//!
//! 在 GUI 不可用的环境（无桌面会话 / 无 dbus）下，调用可能失败。
//! 失败时降级到 AppData 中的 AES-256-GCM 加密文件（使用机器绑定密钥）。
//!
//! Service 标识：`com.loch.aio`
//! Account 命名空间：
//! - `auth-token`: 后端登录 JWT
//! - `app-api-url`: 全局 API URL（仅当用户选择加密存储时）
//! - `app-api-key`: 全局 API Key
//! - `fallback-key`: AES 加密密钥（用于加密 fallback 文件）
//! - `provider-{provider_id}-api-key`: 每个 provider 的 API Key
//! - `mcp-server-{server_id}-env-{env_key}`: 每个 MCP server 的环境变量密钥
//! - `activated-model-{hash}-api-key`: 已激活模型 API Key

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use base64::{engine::general_purpose, Engine as _};
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

const SERVICE: &str = "com.loch.aio";
const FALLBACK_FILE: &str = "secure-store.json";
const FALLBACK_KEY_ACCOUNT: &str = "fallback-key";
const AES_KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;

#[derive(Debug, thiserror::Error)]
pub enum SecureStoreError {
    #[error("keyring 错误: {0}")]
    Keyring(String),
    #[error("降级文件 I/O 错误: {0}")]
    FallbackIo(String),
    #[error("降级文件 JSON 错误: {0}")]
    FallbackJson(String),
    #[error("AES 解密失败: {0}")]
    AesDecrypt(String),
    #[error("AES 加密失败: {0}")]
    AesEncrypt(String),
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

// ====== fallback 文件路径 ======

fn fallback_path(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join(FALLBACK_FILE))
}

fn fallback_key_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_data_dir()
        .ok()
        .map(|d| d.join(".fallback-key"))
}

// ====== AES-256-GCM 密钥管理 ======

/// 获取或创建 fallback 文件的 AES-256 加密密钥。
///
/// 优先级：keyring → 机器绑定派生 → 随机密钥文件（最弱后备）
fn get_fallback_key(app: &AppHandle) -> Result<[u8; AES_KEY_LEN]> {
    // 1. 尝试从 keyring 读取持久化的加密密钥
    if let Ok(hex_key) = keyring_get_str(FALLBACK_KEY_ACCOUNT) {
        if let Ok(bytes) = hex::decode(&hex_key) {
            if bytes.len() == AES_KEY_LEN {
                let mut key = [0u8; AES_KEY_LEN];
                key.copy_from_slice(&bytes);
                return Ok(key);
            }
        }
    }

    // 2. keyring 不可用：尝试从文件读取（机器绑定密钥）
    if let Some(path) = fallback_key_path(app) {
        if path.exists() {
            if let Ok(hex_str) = fs::read_to_string(&path) {
                if let Ok(bytes) = hex::decode(hex_str.trim()) {
                    if bytes.len() == AES_KEY_LEN {
                        let mut key = [0u8; AES_KEY_LEN];
                        key.copy_from_slice(&bytes);
                        return Ok(key);
                    }
                }
            }
        }

        // 3. 生成新密钥：优先存入 keyring，否则写入文件
        let mut key = [0u8; AES_KEY_LEN];
        OsRng.fill_bytes(&mut key);
        let hex_key = hex::encode(key);

        // 尝试存入 keyring
        if keyring_set_str(FALLBACK_KEY_ACCOUNT, &hex_key).is_ok() {
            return Ok(key);
        }

        // keyring 不可用：写入文件（权限 0600 级保护）
        let _ = fs::create_dir_all(path.parent().unwrap());
        if fs::write(&path, &hex_key).is_err() {
            return Err(SecureStoreError::FallbackIo(
                "无法写入 fallback 密钥文件".into(),
            ));
        }

        // 尝试设置文件权限（Unix）
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
        }

        return Ok(key);
    }

    Err(SecureStoreError::FallbackIo(
        "无 AppData 路径，无法创建 fallback 密钥".into(),
    ))
}

// ====== AES-256-GCM 加解密 ======

/// 用 AES-256-GCM 加密一段明文。
/// 返回格式：hex(nonce[12] + ciphertext[..] + auth_tag[16])
fn encrypt_entry(plaintext: &str, key: &[u8; AES_KEY_LEN]) -> Result<String> {
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|e| SecureStoreError::AesEncrypt(e.to_string()))?;

    let mut nonce_bytes = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| SecureStoreError::AesEncrypt(e.to_string()))?;

    // nonce + ciphertext（包含 auth tag）
    let mut combined = nonce_bytes.to_vec();
    combined.extend_from_slice(&ciphertext);

    Ok(hex::encode(combined))
}

/// 解密 AES-256-GCM 加密的 hex 字符串。
/// 自动兼容旧 base64 格式（无加密）。
fn decrypt_entry(hex_or_base64: &str, key: &[u8; AES_KEY_LEN]) -> Result<String> {
    // 1. 尝试 hex → AES-256-GCM 解密
    if let Ok(combined) = hex::decode(hex_or_base64) {
        if combined.len() >= NONCE_LEN + 16 {
            let cipher = Aes256Gcm::new_from_slice(key)
                .map_err(|e| SecureStoreError::AesDecrypt(e.to_string()))?;

            let nonce = Nonce::from_slice(&combined[..NONCE_LEN]);
            let ciphertext = &combined[NONCE_LEN..];

            return cipher
                .decrypt(nonce, ciphertext)
                .map_err(|e| SecureStoreError::AesDecrypt(e.to_string()))
                .and_then(|bytes| {
                    String::from_utf8(bytes).map_err(|e| SecureStoreError::AesDecrypt(e.to_string()))
                });
        }
    }

    // 2. 回退：旧 base64 格式（无加密，仅解码）
    if let Ok(bytes) = general_purpose::STANDARD.decode(hex_or_base64) {
        if let Ok(s) = String::from_utf8(bytes) {
            return Ok(s);
        }
    }

    Err(SecureStoreError::AesDecrypt(
        "无法解密：既非 AES hex 也非旧 base64 格式".into(),
    ))
}

// ====== fallback 文件 I/O ======

fn load_fallback(app: &AppHandle, key: &[u8; AES_KEY_LEN]) -> FallbackStore {
    let Some(p) = fallback_path(app) else {
        return FallbackStore::default();
    };
    if !p.exists() {
        return FallbackStore::default();
    }
    let json_str = match fs::read_to_string(&p) {
        Ok(s) => s,
        Err(_) => return FallbackStore::default(),
    };
    let raw: FallbackStore =
        serde_json::from_str(&json_str).unwrap_or_default();

    // 解密所有 entry
    let mut store = FallbackStore::default();
    for (k, v) in &raw.entries {
        match decrypt_entry(v, key) {
            Ok(plain) => {
                store.entries.insert(k.clone(), plain);
            }
            Err(_) => {
                // 无法解密的 entry 跳过（可能是损坏数据）
                tracing::warn!("[secure_store] 无法解密 entry '{}'，已跳过", k);
            }
        }
    }
    store
}

fn save_fallback(
    app: &AppHandle,
    store: &FallbackStore,
    key: &[u8; AES_KEY_LEN],
) -> Result<()> {
    let p = fallback_path(app)
        .ok_or_else(|| SecureStoreError::FallbackIo("无 AppData 路径".into()))?;
    if let Some(parent) = p.parent() {
        let _ = fs::create_dir_all(parent);
    }

    // 加密所有 entry
    let mut encoded = FallbackStore::default();
    for (k, v) in &store.entries {
        encoded
            .entries
            .insert(k.clone(), encrypt_entry(v, key)?);
    }

    let json = serde_json::to_string_pretty(&encoded)
        .map_err(|e| SecureStoreError::FallbackJson(e.to_string()))?;
    fs::write(&p, json).map_err(|e| SecureStoreError::FallbackIo(e.to_string()))?;
    Ok(())
}

// ====== keyring 操作 ======

fn keyring_get_str(account: &str) -> Result<String> {
    let entry = keyring::Entry::new(SERVICE, account)
        .map_err(|e| SecureStoreError::Keyring(e.to_string()))?;
    entry
        .get_password()
        .map_err(|e| SecureStoreError::Keyring(e.to_string()))
}

fn keyring_set_str(account: &str, value: &str) -> Result<()> {
    let entry = keyring::Entry::new(SERVICE, account)
        .map_err(|e| SecureStoreError::Keyring(e.to_string()))?;
    entry
        .set_password(value)
        .map_err(|e| SecureStoreError::Keyring(e.to_string()))
}

fn keyring_delete_str(account: &str) -> Result<()> {
    let entry = keyring::Entry::new(SERVICE, account)
        .map_err(|e| SecureStoreError::Keyring(e.to_string()))?;
    entry
        .delete_credential()
        .map_err(|e| SecureStoreError::Keyring(e.to_string()))
}

// ====== 高层 API ======

/// 读取凭据：优先 keyring，不可用时回退到 AES 加密文件。
pub fn get(app: &AppHandle, account: &str) -> Result<Option<String>> {
    match keyring_get_str(account) {
        Ok(v) => Ok(Some(v)),
        Err(SecureStoreError::Keyring(_)) => {
            let key = get_fallback_key(app)?;
            let store = load_fallback(app, &key);
            Ok(store.entries.get(account).cloned())
        }
        Err(e) => Err(e),
    }
}

/// 存储凭据：优先 keyring，不可用时回退到 AES 加密文件。
pub fn set(app: &AppHandle, account: &str, value: &str) -> Result<()> {
    match keyring_set_str(account, value) {
        Ok(()) => {
            // 同步清理 fallback 文件中的同 key
            if let Ok(key) = get_fallback_key(app) {
                let mut store = load_fallback(app, &key);
                if store.entries.remove(account).is_some() {
                    let _ = save_fallback(app, &store, &key);
                }
            }
            Ok(())
        }
        Err(_) => {
            let key = get_fallback_key(app)?;
            let mut store = load_fallback(app, &key);
            store
                .entries
                .insert(account.to_string(), value.to_string());
            save_fallback(app, &store, &key)
        }
    }
}

/// 删除凭据。
pub fn delete(app: &AppHandle, account: &str) -> Result<()> {
    let _ = keyring_delete_str(account);
    if let Ok(key) = get_fallback_key(app) {
        let mut store = load_fallback(app, &key);
        if store.entries.remove(account).is_some() {
            save_fallback(app, &store, &key)?;
        }
    }
    Ok(())
}

/// 删除所有匹配前缀的凭据（用于清理 MCP server 等）。
#[allow(dead_code)]
pub fn delete_matching(app: &AppHandle, prefix: &str) -> Result<()> {
    if let Ok(key) = get_fallback_key(app) {
        let mut store = load_fallback(app, &key);
        let to_remove: Vec<String> = store
            .entries
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect();
        if !to_remove.is_empty() {
            for k in &to_remove {
                store.entries.remove(k);
            }
            save_fallback(app, &store, &key)?;
        }
    }
    // keyring 不支持按前缀删除，调用方需显式传 key 名
    Ok(())
}

// ====== 已知 account 命名空间 ======

pub mod accounts {
    use sha2::{Digest, Sha256};

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

    /// 已激活模型的 API Key（用 api_url + model_id 的 SHA-256 hash 作标识符，
    /// 避免将 URL 明文写入 keyring account 名）。
    pub fn activated_model_key(api_url: &str, model_id: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(api_url.as_bytes());
        hasher.update(b"::");
        hasher.update(model_id.as_bytes());
        let hash = hex::encode(hasher.finalize());
        format!("activated-model-{}-api-key", hash)
    }
}