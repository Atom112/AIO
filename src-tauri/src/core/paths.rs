//! 应用级路径与标识符 —— 全仓唯一来源。
//!
//! 历史背景：`"com.loch.aio"` 字面量曾散落在 commands/config.rs（12 次）、
//! secure_store.rs、provider_config.rs（各自重复定义）；`dirs::config_dir().unwrap()`
//! 会在无法定位配置目录时直接 panic（违反"避免在用户路径上 panic"约定）。

use std::path::PathBuf;

/// 应用 Bundle 标识：配置目录名、keyring service、macOS LaunchAgent label 共用。
pub const BUNDLE_ID: &str = "com.loch.aio";

/// 应用配置目录：`dirs::config_dir()/com.loch.aio`。
/// 助手/provider/MCP/模型等 JSON 配置存放于此。
/// （与 app_data_dir 分离是历史决定：配置跟随系统配置目录，数据/缓存跟随应用数据目录。）
pub fn app_config_dir() -> Result<PathBuf, String> {
    let mut p = dirs::config_dir().ok_or_else(|| "无法获取系统配置目录".to_string())?;
    p.push(BUNDLE_ID);
    Ok(p)
}

/// 配置目录下的指定文件路径（自动确保目录存在）。
pub fn app_config_file(file_name: &str) -> Result<PathBuf, String> {
    let p = app_config_dir()?;
    if !p.exists() {
        std::fs::create_dir_all(&p).map_err(|e| e.to_string())?;
    }
    Ok(p.join(file_name))
}
