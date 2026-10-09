//! Local engine domain models: installation detection results.

use serde::{Deserialize, Serialize};

/// Engine installation detection result: whether the engine is installed on the system, plus its version.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct EngineInstallInfo {
    pub installed: bool,
    pub version: Option<String>,
}
