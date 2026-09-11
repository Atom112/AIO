//! 本地引擎域模型：安装检测结果。

use serde::{Deserialize, Serialize};

/// 引擎安装检测结果：是否已安装在系统上及版本号。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct EngineInstallInfo {
    pub installed: bool,
    pub version: Option<String>,
}
