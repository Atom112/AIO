/// 引擎安装路径管理
/// 仅保留路径解析工具（安装/下载功能已移除）
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

/// 引擎安装状态（供插件查询）
pub struct EngineInstaller;

impl EngineInstaller {
    /// 获取引擎安装目录（app data 下）
    pub fn get_engine_dir(app: &AppHandle) -> PathBuf {
        let data_dir = app
            .path()
            .app_data_dir()
            .unwrap_or_else(|_| PathBuf::from("."));
        data_dir.join("engines").join("llama-cpp")
    }

    /// 获取引擎可执行文件路径
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

    /// 检查引擎是否已安装（可执行文件存在）
    pub fn is_installed(app: &AppHandle) -> bool {
        Self::get_exe_path(app).exists()
    }
}
