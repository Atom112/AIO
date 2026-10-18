//! # Cross-platform secure credential storage
//!
//! Uses the [`keyring`] crate to abstract each OS's native credential manager:
//! - **Windows**: Windows Credential Manager (`wincred`)
//! - **macOS**: Keychain (`security-framework`)
//! - **Linux**: Secret Service / `libsecret` (`secret-service-rs`)
//!
//! Calls may fail in environments without a GUI (no desktop session / no dbus).
//! On failure, falls back to an AES-256-GCM encrypted file in AppData (the key is stored
//! in keyring first, otherwise a 0600 local random key file — not machine-bound, only
//! static obfuscation: any process able to read both files can decrypt).
//!
//! Service identifier: `com.loch.aio`
//! Account namespaces:
//! - `auth-token`: backend login JWT
//! - `app-api-url`: global API URL (only when the user opts into encrypted storage)
//! - `app-api-key`: global API Key
//! - `fallback-key`: AES encryption key (used to encrypt the fallback file)
//! - `provider-{provider_id}-api-key`: per-provider API Key
//! - `mcp-server-{server_id}-env-{env_key}`: per-MCP-server environment variable secret
//! - `activated-model-{hash}-api-key`: activated model API Key

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

const SERVICE: &str = crate::core::paths::BUNDLE_ID;
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

// ====== fallback file paths ======

fn fallback_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_data_dir()
        .ok()
        .map(|d| d.join(FALLBACK_FILE))
}

fn fallback_key_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_data_dir()
        .ok()
        .map(|d| d.join(".fallback-key"))
}

// ====== AES-256-GCM key management ======

/// Get or create the AES-256 encryption key for the fallback file.
///
/// Priority: keyring → local random key file (0600 permissions, weakest fallback).
/// Note: there is no "machine-bound derivation" layer — the file key only provides static obfuscation.
fn get_fallback_key(app: &AppHandle) -> Result<[u8; AES_KEY_LEN]> {
    // 1. Try reading the persisted encryption key from keyring
    if let Ok(hex_key) = keyring_get_str(FALLBACK_KEY_ACCOUNT) {
        if let Ok(bytes) = hex::decode(&hex_key) {
            if bytes.len() == AES_KEY_LEN {
                let mut key = [0u8; AES_KEY_LEN];
                key.copy_from_slice(&bytes);
                // keyring available: clean up any leftover plaintext fallback-key file
                if let Some(kp) = fallback_key_path(app) {
                    if kp.exists() {
                        let _ = fs::remove_file(&kp);
                        tracing::info!(
                            "[secure_store] keyring 已恢复，已删除残留的 fallback-key 文件"
                        );
                    }
                }
                return Ok(key);
            }
        }
    }

    // 2. keyring unavailable: try reading the key from the file
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

        // 3. Generate a new key: store in keyring first, otherwise write to a file
        let mut key = [0u8; AES_KEY_LEN];
        OsRng.fill_bytes(&mut key);
        let hex_key = hex::encode(key);

        // Try storing it in keyring
        if keyring_set_str(FALLBACK_KEY_ACCOUNT, &hex_key).is_ok() {
            // keyring write succeeded: clean up any leftover plaintext key file
            if path.exists() {
                let _ = fs::remove_file(&path);
            }
            return Ok(key);
        }

        // keyring unavailable: write to file (atomic + 0600; the temp file also gets
        // 0600 BEFORE content is written so the plaintext key is never briefly
        // world-readable during the write window)
        let _ = fs::create_dir_all(path.parent().unwrap());
        if write_key_file_atomic(&path, &hex_key).is_err() {
            return Err(SecureStoreError::FallbackIo(
                "无法写入 fallback 密钥文件".into(),
            ));
        }

        return Ok(key);
    }

    Err(SecureStoreError::FallbackIo(
        "无 AppData 路径，无法创建 fallback 密钥".into(),
    ))
}

/// 原子写 fallback 密钥文件（INFRA-4：非原子 `fs::write` 崩溃可产生半写 key，
/// 之后整个 fallback 库都解不开）。
fn write_key_file_atomic(path: &std::path::Path, content: &str) -> std::io::Result<()> {
    use std::io::Write;
    let tmp = path.with_extension("key.tmp");
    let mut f = fs::File::create(&tmp)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
    }
    f.write_all(content.as_bytes())?;
    f.sync_all()?;
    fs::rename(&tmp, path)
}

// ====== AES-256-GCM encrypt/decrypt ======

/// Encrypt a plaintext string with AES-256-GCM.
/// Return format: hex(nonce[12] + ciphertext[..] + auth_tag[16])
fn encrypt_entry(plaintext: &str, key: &[u8; AES_KEY_LEN]) -> Result<String> {
    let cipher =
        Aes256Gcm::new_from_slice(key).map_err(|e| SecureStoreError::AesEncrypt(e.to_string()))?;

    let mut nonce_bytes = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| SecureStoreError::AesEncrypt(e.to_string()))?;

    // nonce + ciphertext (includes the auth tag)
    let mut combined = nonce_bytes.to_vec();
    combined.extend_from_slice(&ciphertext);

    Ok(hex::encode(combined))
}

/// Decrypt a hex string encrypted with AES-256-GCM.
/// Also accepts the legacy base64 format (no encryption).
fn decrypt_entry(hex_or_base64: &str, key: &[u8; AES_KEY_LEN]) -> Result<String> {
    // 1. Try hex → AES-256-GCM decryption
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
                    String::from_utf8(bytes)
                        .map_err(|e| SecureStoreError::AesDecrypt(e.to_string()))
                });
        }
    }

    // 2. Fallback: legacy base64 format (no encryption, decode only)
    if let Ok(bytes) = general_purpose::STANDARD.decode(hex_or_base64) {
        if let Ok(s) = String::from_utf8(bytes) {
            return Ok(s);
        }
    }

    Err(SecureStoreError::AesDecrypt(
        "无法解密：既非 AES hex 也非旧 base64 格式".into(),
    ))
}

// ====== fallback file I/O ======

/// INFRA-4：fallback 文件的读-改-写必须跨线程串行（主线程同步命令与 async
/// 命令可能并发 set/delete，无锁 RMW 会丢更新——后写覆盖先写）。
static FALLBACK_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn load_fallback(app: &AppHandle, key: &[u8; AES_KEY_LEN]) -> FallbackStore {
    let Some(p) = fallback_path(app) else {
        return FallbackStore::default();
    };
    if !p.exists() {
        return FallbackStore::default();
    }
    let json_str = match fs::read_to_string(&p) {
        Ok(s) => s,
        Err(_) => {
            quarantine_corrupt(&p, "读取失败");
            return FallbackStore::default();
        }
    };
    let raw: FallbackStore = match serde_json::from_str(&json_str) {
        Ok(v) => v,
        Err(_) => {
            quarantine_corrupt(&p, "解析失败");
            return FallbackStore::default();
        }
    };

    // Decrypt all entries
    let mut store = FallbackStore::default();
    for (k, v) in &raw.entries {
        match decrypt_entry(v, key) {
            Ok(plain) => {
                store.entries.insert(k.clone(), plain);
            }
            Err(_) => {
                // Skip entries that fail to decrypt (possibly corrupted data)
                tracing::warn!("[secure_store] 无法解密 entry '{}'，已跳过", k);
            }
        }
    }
    store
}

/// INFRA-4：损坏的 fallback 文件先隔离为 `<name>.corrupt.bak`（保留现场），
/// 而不是当作空库继续运行——否则一次崩溃半写就会在下一次保存时把全部
/// fallback 密钥静默抹掉。
fn quarantine_corrupt(path: &std::path::Path, reason: &str) {
    let bak = path.with_extension("corrupt.bak");
    match fs::rename(path, &bak) {
        Ok(()) => tracing::warn!(
            "[secure_store] fallback 文件损坏（{reason}），已隔离为 {}",
            bak.display()
        ),
        Err(e) => tracing::warn!("[secure_store] fallback 文件损坏（{reason}），隔离失败: {e}"),
    }
}

fn save_fallback(app: &AppHandle, store: &FallbackStore, key: &[u8; AES_KEY_LEN]) -> Result<()> {
    let p =
        fallback_path(app).ok_or_else(|| SecureStoreError::FallbackIo("无 AppData 路径".into()))?;
    if let Some(parent) = p.parent() {
        let _ = fs::create_dir_all(parent);
    }

    // Encrypt all entries
    let mut encoded = FallbackStore::default();
    for (k, v) in &store.entries {
        encoded.entries.insert(k.clone(), encrypt_entry(v, key)?);
    }

    let json = serde_json::to_string_pretty(&encoded)
        .map_err(|e| SecureStoreError::FallbackJson(e.to_string()))?;
    // INFRA-4：原子写（temp + fsync + rename）——崩溃/掉电不再产生半写文件
    crate::core::fs_util::atomic_write(&p, json.as_bytes())
        .map_err(|e| SecureStoreError::FallbackIo(e.to_string()))?;
    Ok(())
}

// ====== keyring operations ======

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

// ====== high-level API ======

/// Read a credential: keyring first, falling back to the AES-encrypted file when unavailable.
pub fn get(app: &AppHandle, account: &str) -> Result<Option<String>> {
    match keyring_get_str(account) {
        Ok(v) => Ok(Some(v)),
        Err(SecureStoreError::Keyring(_)) => {
            // INFRA-4 同款互斥：无锁时两个并发首访 get() 可同时进入密钥"生成
            // 新钥"分支，两把密钥先后落盘——先落盘密钥加密的条目从此永久失解
            // （可用性损失）。与 set/delete 共用一把锁，读写全程串行。
            let _guard = FALLBACK_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
            let key = get_fallback_key(app)?;
            let store = load_fallback(app, &key);
            Ok(store.entries.get(account).cloned())
        }
        Err(e) => Err(e),
    }
}

/// Store a credential: keyring first, falling back to the AES-encrypted file when unavailable.
pub fn set(app: &AppHandle, account: &str, value: &str) -> Result<()> {
    // INFRA-4: serialize the fallback read-modify-write across threads
    let _guard = FALLBACK_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    match keyring_set_str(account, value) {
        Ok(()) => {
            // Also remove the same key from the fallback file
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
            store.entries.insert(account.to_string(), value.to_string());
            save_fallback(app, &store, &key)
        }
    }
}

/// Store a credential with failure visibility: logs an error and returns false on failure.
///
/// Callers use the return value to decide whether to keep the plaintext field in data:
/// swallowing a `set(...)` failure and then stripping plaintext silently loses the secret
/// when keyring and the fallback file both fail.
pub fn set_reported(app: &AppHandle, account: &str, value: &str) -> bool {
    match set(app, account, value) {
        Ok(()) => true,
        Err(e) => {
            tracing::error!("安全存储写入失败 (account={account}): {e}");
            false
        }
    }
}

/// Delete a credential.
pub fn delete(app: &AppHandle, account: &str) -> Result<()> {
    // INFRA-4: serialize the fallback read-modify-write across threads
    let _guard = FALLBACK_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _ = keyring_delete_str(account);
    if let Ok(key) = get_fallback_key(app) {
        let mut store = load_fallback(app, &key);
        if store.entries.remove(account).is_some() {
            save_fallback(app, &store, &key)?;
        }
    }
    Ok(())
}

// ====== known account namespaces ======

pub mod accounts {
    use sha2::{Digest, Sha256};

    pub const APP_API_KEY: &str = "app-api-key";

    /// Optional HuggingFace access token (read scope) for gated-repo downloads
    pub fn hf_token() -> String {
        "hf-access-token".to_string()
    }

    pub fn provider_key(id: &str) -> String {
        format!("provider-{}-api-key", id)
    }

    /// MCP server environment variable secret: ${KEYRING:mcp-server-{server_id}-env-{env_key}}
    pub fn mcp_server_env(server_id: &str, env_key: &str) -> String {
        format!("mcp-server-{}-env-{}", server_id, env_key)
    }

    /// Harness tool API key (injected as the tool's provider env var at spawn):
    /// ${KEYRING:harness-tool-{tool_id}-api-key}
    pub fn harness_tool_key(tool_id: &str) -> String {
        format!("harness-tool-{}-api-key", tool_id)
    }

    /// API Key for an activated model (identified by the SHA-256 hash of api_url + model_id,
    /// avoiding writing the URL in plaintext into the keyring account name).
    pub fn activated_model_key(api_url: &str, model_id: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(api_url.as_bytes());
        hasher.update(b"::");
        hasher.update(model_id.as_bytes());
        let hash = hex::encode(hasher.finalize());
        format!("activated-model-{}-api-key", hash)
    }
}
