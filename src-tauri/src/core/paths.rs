//! App-level paths and identifiers — single source of truth repo-wide.
//!
//! Keeps `"com.loch.aio"` literals from scattering across modules;
//! `dirs::config_dir().unwrap()` would panic when the config directory cannot be
//! located, violating the "no panics on user-facing paths" convention.

use std::path::PathBuf;

/// App bundle identifier: shared by the config directory name, keyring service, and macOS LaunchAgent label.
pub const BUNDLE_ID: &str = "com.loch.aio";

/// App config directory: `dirs::config_dir()/com.loch.aio`.
/// Stores assistant/provider/MCP/model JSON configs.
/// (Separate from app_data_dir by design: config follows the system config dir, data/cache the app data dir.)
pub fn app_config_dir() -> Result<PathBuf, String> {
    let mut p = dirs::config_dir().ok_or_else(|| "无法获取系统配置目录".to_string())?;
    p.push(BUNDLE_ID);
    Ok(p)
}

/// Path of a file inside the config directory (ensures the directory exists).
pub fn app_config_file(file_name: &str) -> Result<PathBuf, String> {
    let p = app_config_dir()?;
    if !p.exists() {
        std::fs::create_dir_all(&p).map_err(|e| e.to_string())?;
    }
    Ok(p.join(file_name))
}
