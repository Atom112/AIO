//! # Application Updater Module
//!
//! ## Overview
//! Wraps the core capabilities of tauri-plugin-updater:
//! 1. `check_app_update` - checks GitHub Releases for a new version (stable channel only).
//! 2. `install_app_update` - downloads the update package in the background, reporting progress via Tauri events.
//! 3. `restart_app` - restarts the app immediately to apply the downloaded update.
//! 4. `get_updater_endpoint` - returns the configured update manifest endpoint, for debug display in the UI.
//!
//! ## Data Flow
//! 1. **Frontend -> local backend**: the frontend calls this module's async functions via Tauri `invoke`.
//! 2. **Local backend -> GitHub**: handled inside `tauri-plugin-updater`, using the manifest
//!    endpoint configured in `tauri.conf.json` `plugins.updater.endpoints`.
//! 3. **Progress reporting**: during download, broadcasts `app.emit(ev::APP_UPDATE_PROGRESS, pct)`; the frontend listens.

use crate::core::events as ev;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime};
use tauri_plugin_updater::Error as UpdaterError;
use tauri_plugin_updater::UpdaterExt;

use crate::ierr;

/// Normalized update info (returned to the frontend)
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdateInfo {
    /// Latest version number (with the `v` prefix stripped)
    pub version: String,
    /// Current app version number
    pub current_version: String,
    /// Release notes (raw Markdown)
    pub notes: Option<String>,
    /// Publish time (ISO8601 string)
    pub pub_date: Option<String>,
}

/// Unified result of manual/automatic update checks.
///
/// The `kind` field acts as a tag so the frontend can show a per-category hint:
/// - `up_to_date`        already the latest version
/// - `update_available`  a new version was found
/// - `service_not_ready` the remote release has no latest.json yet (expected: releases
///   before v0.3.1 were published with the old CI)
/// - `network`           network error
/// - `failed`            other errors (config errors, platform mismatch, etc.)
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

/// Gets the currently configured updater endpoint list (for debug display in the UI)
#[tauri::command]
pub fn get_updater_endpoint<R: Runtime>(app: AppHandle<R>) -> Vec<String> {
    get_updater_endpoint_generic(&app)
}

/// Checks whether the app has an available update (structured result)
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
                reason: ierr!(
                    "初始化更新服务失败: {}",
                    "Failed to initialize the update service: {}",
                    e
                ),
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

/// Translates tauri-plugin-updater internal errors into user-friendly categorized results
fn translate_error(err: UpdaterError, current_version: &str, endpoint: &str) -> CheckUpdateResult {
    match err {
        // 1) Manifest missing / parse failure / no package for the current platform: all
        //    classified as "update service not configured yet". In that case the AIO repo's
        //    release has no latest.json attached (releases before v0.3.1 used the old CI).
        UpdaterError::ReleaseNotFound
        | UpdaterError::TargetNotFound(_)
        | UpdaterError::TargetsNotFound(_) => CheckUpdateResult::ServiceNotReady {
            current_version: current_version.to_string(),
            endpoint: endpoint.to_string(),
            reason: ierr!(
                "GitHub Release 尚未附 latest.json 更新清单，需要用新 CI 发布一次后才会启用自动更新",
                "The GitHub Release has no latest.json update manifest yet; automatic updates will be enabled once a release is published with the new CI"
            ),
        },

        // 2) Network-layer errors (HTTP, DNS, connect timeout, etc.)
        UpdaterError::Reqwest(req_err) => {
            if req_err.is_status() {
                let status = req_err.status().map(|s| s.as_u16()).unwrap_or(0);
                if status == 404 {
                    CheckUpdateResult::ServiceNotReady {
                        current_version: current_version.to_string(),
                        endpoint: endpoint.to_string(),
                        reason: ierr!(
                            "更新清单不存在 (HTTP 404) — GitHub Release 尚未附 latest.json",
                            "Update manifest not found (HTTP 404) — the GitHub Release has no latest.json attached"
                        ),
                    }
                } else {
                    CheckUpdateResult::Network {
                        current_version: current_version.to_string(),
                        endpoint: endpoint.to_string(),
                        reason: ierr!("更新服务器返回 HTTP {}", "Update server returned HTTP {}", status),
                    }
                }
            } else if req_err.is_connect() || req_err.is_timeout() || req_err.is_request() {
                CheckUpdateResult::Network {
                    current_version: current_version.to_string(),
                    endpoint: endpoint.to_string(),
                    reason: ierr!("无法连接更新服务器: {}", "Unable to connect to the update server: {}", req_err),
                }
            } else {
                CheckUpdateResult::Failed {
                    current_version: current_version.to_string(),
                    endpoint: endpoint.to_string(),
                    reason: ierr!("网络错误: {}", "Network error: {}", req_err),
                }
            }
        }

        // 3) Explicitly declared network error
        UpdaterError::Network(msg) => CheckUpdateResult::Network {
            current_version: current_version.to_string(),
            endpoint: endpoint.to_string(),
            reason: msg,
        },

        // 4) Configuration errors
        UpdaterError::InsecureTransportProtocol => CheckUpdateResult::Failed {
            current_version: current_version.to_string(),
            endpoint: endpoint.to_string(),
            reason: ierr!("更新服务地址必须使用 HTTPS 协议", "The update service endpoint must use HTTPS"),
        },
        UpdaterError::EmptyEndpoints => CheckUpdateResult::Failed {
            current_version: current_version.to_string(),
            endpoint: endpoint.to_string(),
            reason: ierr!("尚未配置任何更新服务地址", "No update service endpoint is configured"),
        },

        // 5) Unsupported platform/architecture
        UpdaterError::UnsupportedOs => CheckUpdateResult::Failed {
            current_version: current_version.to_string(),
            endpoint: endpoint.to_string(),
            reason: ierr!(
                "当前操作系统不支持自动更新",
                "Automatic updates are not supported on this operating system"
            ),
        },
        UpdaterError::UnsupportedArch => CheckUpdateResult::Failed {
            current_version: current_version.to_string(),
            endpoint: endpoint.to_string(),
            reason: ierr!(
                "当前 CPU 架构不支持自动更新",
                "Automatic updates are not supported on this CPU architecture"
            ),
        },

        // 6) Other unclassified errors: pass the original message through
        other => CheckUpdateResult::Failed {
            current_version: current_version.to_string(),
            endpoint: endpoint.to_string(),
            reason: other.to_string(),
        },
    }
}

/// Generic endpoint resolution (independent of the concrete Runtime)
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

/// Downloads and installs the update (takes effect after restart)
/// Reports download progress (0.0 ~ 1.0) to the frontend live via the `app-update-progress` event
#[tauri::command]
pub async fn install_app_update<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    let updater = app.updater().map_err(|e| e.to_string())?;

    let update = updater.check().await.map_err(|e| e.to_string())?;
    let Some(update) = update else {
        return Err(ierr!("未发现可用更新", "No available update found"));
    };

    let app_clone = app.clone();
    update
        .download_and_install(
            |chunk_len, total| {
                if let Some(total) = total {
                    if total > 0 {
                        let pct = chunk_len as f64 / total as f64;
                        let _ = app_clone.emit(ev::APP_UPDATE_PROGRESS, pct);
                    }
                }
            },
            || {
                let _ = app_clone.emit(ev::APP_UPDATE_PROGRESS, 0.0_f64);
            },
        )
        .await
        .map_err(|e| e.to_string())?;

    let _ = app.emit(ev::APP_UPDATE_PROGRESS, 1.0_f64);
    Ok(())
}

/// Restarts the app to apply the downloaded update
#[tauri::command]
pub fn restart_app<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    app.request_restart();
    Ok(())
}
