//! # 应用更新模块 (Application Updater Module)
//!
//! ## 功能描述
//! 封装 tauri-plugin-updater 的核心能力：
//! 1. `check_app_update` —— 检查 GitHub Releases 是否有新版本（仅稳定版）。
//! 2. `install_app_update` —— 后台流式下载更新包并通过 Tauri 事件报告进度。
//! 3. `restart_app` —— 立即重启应用以应用已下载的更新。
//! 4. `get_updater_endpoint` —— 返回当前配置的更新清单地址，用于在 UI 中展示调试信息。
//!
//! ## 数据流向
//! 1. **前端 -> 本地后端**: 前端通过 Tauri `invoke` 调用此模块的异步函数。
//! 2. **本地后端 -> GitHub**: 由 `tauri-plugin-updater` 内部完成，使用 `tauri.conf.json`
//!    `plugins.updater.endpoints` 配置的清单地址。
//! 3. **进度回传**: 下载阶段通过 `app.emit("app-update-progress", pct)` 广播，前端可监听。

use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime};
use tauri_plugin_updater::UpdaterExt;
use tauri_plugin_updater::Error as UpdaterError;

/// 标准化后的更新信息（返回给前端）
#[derive(Serialize, Clone, Debug)]
pub struct AppUpdateInfo {
    /// 最新版本号（已去掉 `v` 前缀）
    pub version: String,
    /// 当前应用版本号
    pub current_version: String,
    /// Release notes（Markdown 原文）
    pub notes: Option<String>,
    /// 发布时间（ISO8601 字符串）
    pub pub_date: Option<String>,
}

/// 手动/自动检查更新的统一结果。
///
/// 使用 `kind` 字段做 tag，前端可按类别显示不同提示：
/// - `up_to_date`        已是最新
/// - `update_available`  发现新版本
/// - `service_not_ready` 远端 release 尚未发布 latest.json（正常情况：v0.3.1 之前用旧 CI 发布过）
/// - `network`           网络错误
/// - `failed`            其他错误（含配置错误、平台不匹配等）
#[derive(Serialize, Clone, Debug)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CheckUpdateResult {
    UpToDate {
        current_version: String,
    },
    UpdateAvailable {
        info: AppUpdateInfo,
    },
    ServiceNotReady {
        current_version: String,
        endpoint: String,
        reason: String,
    },
    Network {
        current_version: String,
        endpoint: String,
        reason: String,
    },
    Failed {
        current_version: String,
        endpoint: String,
        reason: String,
    },
}

/// 获取当前配置的 updater endpoint 列表（用于 UI 调试展示）
#[tauri::command]
pub fn get_updater_endpoint<R: Runtime>(app: AppHandle<R>) -> Vec<String> {
    get_updater_endpoint_generic(&app)
}

/// 检查应用是否有可用更新（结构化结果）
#[tauri::command]
pub async fn check_app_update<R: Runtime>(app: AppHandle<R>) -> CheckUpdateResult {
    let current_version = app.package_info().version.to_string();
    let endpoint = get_updater_endpoint_generic(&app)
        .into_iter()
        .next()
        .unwrap_or_default();

    let updater = match app.updater() {
        Ok(u) => u,
        Err(e) => {
            return CheckUpdateResult::Failed {
                current_version: current_version.clone(),
                endpoint,
                reason: format!("初始化更新服务失败: {e}"),
            };
        }
    };

    match updater.check().await {
        Ok(Some(update)) => {
            let version = update
                .version
                .strip_prefix('v')
                .unwrap_or(&update.version)
                .to_string();
            CheckUpdateResult::UpdateAvailable {
                info: AppUpdateInfo {
                    version,
                    current_version,
                    notes: update.body.clone(),
                    pub_date: update.date.map(|d| d.to_string()),
                },
            }
        }
        Ok(None) => CheckUpdateResult::UpToDate { current_version },
        Err(e) => translate_error(e, &current_version, &endpoint),
    }
}

/// 把 tauri-plugin-updater 的内部错误翻译为用户友好的分类结果
fn translate_error(err: UpdaterError, current_version: &str, endpoint: &str) -> CheckUpdateResult {
    match err {
        // 1) 清单不存在 / 解析失败 / 没有当前平台的安装包 —— 都归类为「更新服务尚未配置」
        //    这种情况下 AIO 仓库的 release 没有附 latest.json（v0.3.1 之前都用旧 CI 发的）。
        UpdaterError::ReleaseNotFound
        | UpdaterError::TargetNotFound(_)
        | UpdaterError::TargetsNotFound(_) => CheckUpdateResult::ServiceNotReady {
            current_version: current_version.to_string(),
            endpoint: endpoint.to_string(),
            reason: "GitHub Release 尚未附 latest.json 更新清单，需要用新 CI 发布一次后才会启用自动更新".to_string(),
        },

        // 2) 网络层错误（HTTP、DNS、连接超时等）
        UpdaterError::Reqwest(req_err) => {
            if req_err.is_status() {
                let status = req_err.status().map(|s| s.as_u16()).unwrap_or(0);
                if status == 404 {
                    CheckUpdateResult::ServiceNotReady {
                        current_version: current_version.to_string(),
                        endpoint: endpoint.to_string(),
                        reason: format!("更新清单不存在 (HTTP 404) — GitHub Release 尚未附 latest.json"),
                    }
                } else {
                    CheckUpdateResult::Network {
                        current_version: current_version.to_string(),
                        endpoint: endpoint.to_string(),
                        reason: format!("更新服务器返回 HTTP {status}"),
                    }
                }
            } else if req_err.is_connect() || req_err.is_timeout() || req_err.is_request() {
                CheckUpdateResult::Network {
                    current_version: current_version.to_string(),
                    endpoint: endpoint.to_string(),
                    reason: format!("无法连接更新服务器: {req_err}"),
                }
            } else {
                CheckUpdateResult::Failed {
                    current_version: current_version.to_string(),
                    endpoint: endpoint.to_string(),
                    reason: format!("网络错误: {req_err}"),
                }
            }
        }

        // 3) 显式声明的网络错误
        UpdaterError::Network(msg) => CheckUpdateResult::Network {
            current_version: current_version.to_string(),
            endpoint: endpoint.to_string(),
            reason: msg,
        },

        // 4) 配置错误
        UpdaterError::InsecureTransportProtocol => CheckUpdateResult::Failed {
            current_version: current_version.to_string(),
            endpoint: endpoint.to_string(),
            reason: "更新服务地址必须使用 HTTPS 协议".to_string(),
        },
        UpdaterError::EmptyEndpoints => CheckUpdateResult::Failed {
            current_version: current_version.to_string(),
            endpoint: endpoint.to_string(),
            reason: "尚未配置任何更新服务地址".to_string(),
        },

        // 5) 平台/架构不支持
        UpdaterError::UnsupportedOs => CheckUpdateResult::Failed {
            current_version: current_version.to_string(),
            endpoint: endpoint.to_string(),
            reason: "当前操作系统不支持自动更新".to_string(),
        },
        UpdaterError::UnsupportedArch => CheckUpdateResult::Failed {
            current_version: current_version.to_string(),
            endpoint: endpoint.to_string(),
            reason: "当前 CPU 架构不支持自动更新".to_string(),
        },

        // 6) 其他未明确分类的错误 —— 把原始信息透传
        other => CheckUpdateResult::Failed {
            current_version: current_version.to_string(),
            endpoint: endpoint.to_string(),
            reason: other.to_string(),
        },
    }
}

/// 通用 endpoint 解析（不依赖具体 Runtime）
fn get_updater_endpoint_generic<R: Runtime>(app: &AppHandle<R>) -> Vec<String> {
    let cfg = app.config();
    if let Some(value) = cfg.plugins.0.get("updater") {
        if let Some(arr) = value.get("endpoints").and_then(|v| v.as_array()) {
            return arr
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect();
        }
    }
    Vec::new()
}

/// 下载并安装更新（重启后生效）
/// 通过 `app-update-progress` 事件向前端实时汇报下载进度 (0.0 ~ 1.0)
#[tauri::command]
pub async fn install_app_update<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    let updater = app.updater().map_err(|e| e.to_string())?;

    let update = updater.check().await.map_err(|e| e.to_string())?;
    let Some(update) = update else {
        return Err("未发现可用更新".into());
    };

    let app_clone = app.clone();
    update
        .download_and_install(
            |chunk_len, total| {
                if let Some(total) = total {
                    if total > 0 {
                        let pct = chunk_len as f64 / total as f64;
                        let _ = app_clone.emit("app-update-progress", pct);
                    }
                }
            },
            || {
                let _ = app_clone.emit("app-update-progress", 0.0_f64);
            },
        )
        .await
        .map_err(|e| e.to_string())?;

    let _ = app.emit("app-update-progress", 1.0_f64);
    Ok(())
}

/// 重启应用以应用已下载的更新
#[tauri::command]
pub fn restart_app<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    app.request_restart();
    Ok(())
}
