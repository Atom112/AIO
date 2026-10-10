/// Engine install path management
/// Path resolution utilities only (no install/download logic)
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

/// Engine installation status (queried by plugins)
pub struct EngineInstaller;

impl EngineInstaller {
    /// Get the engine install directory (under app data)
    pub fn get_engine_dir(app: &AppHandle) -> PathBuf {
        let data_dir = app
            .path()
            .app_data_dir()
            .unwrap_or_else(|_| PathBuf::from("."));
        data_dir.join("engines").join("llama-cpp")
    }

    /// Get the engine executable path
    pub fn get_exe_path(app: &AppHandle) -> PathBuf {
        let dir = Self::get_engine_dir(app);
        #[cfg(target_os = "windows")]
        {
            dir.join("llama-server.exe")
        }
        #[cfg(not(target_os = "windows"))]
        {
            dir.join("llama-server")
        }
    }

    /// Check whether the engine is installed (executable exists)
    pub fn is_installed(app: &AppHandle) -> bool {
        Self::get_exe_path(app).exists()
    }
}
