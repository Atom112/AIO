//! Skill domain models: skill config, source kinds, and market entries.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Skill source kind.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum SkillSource {
    /// Manually created or legacy data (default)
    #[default]
    Local,
    /// Downloaded from the skills.sh market
    Market,
    /// Imported from the npm/npx ecosystem
    Npx,
}

/// Reusable assistant system-instruction module.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SkillConfig {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub content: String,
    /// Skill source. Legacy data without this field deserializes to Local.
    #[serde(default)]
    pub source: SkillSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_repo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_slug: Option<String>,
    #[serde(default)]
    pub installs: u64,
    /// npx package name (required when source = Npx), e.g. "@anthropic-ai/skill-docx"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub npx_package: Option<String>,
    /// Installed version of the npx package
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub npx_version: Option<String>,
    /// npx execution command (defaults to npx_package)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub npx_command: Option<String>,
    /// On-disk skill directory (skills CLI / skill-dir sources): refresh reads the
    /// content directly from here instead of re-running npx (their names are not
    /// npm packages — `npx <name>` fails with "could not determine executable").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_disk_path: Option<String>,
}

/// Skill persistence file.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SkillsFile {
    pub version: u32,
    pub updated_at: String,
    pub skills: BTreeMap<String, SkillConfig>,
}

impl Default for SkillsFile {
    fn default() -> Self {
        Self {
            version: 1,
            updated_at: String::new(),
            skills: BTreeMap::new(),
        }
    }
}

/// Skill summary from the skills.sh market.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MarketSkill {
    pub id: String,
    pub name: String,
    pub owner: String,
    pub repo: String,
    pub slug: String,
    #[serde(default)]
    pub description: String,
    pub source_url: String,
    #[serde(default)]
    pub installs: u64,
    #[serde(default)]
    pub installs_label: String,
    #[serde(default)]
    pub weekly_installs: Vec<u64>,
}

// ====== Agent mode configuration ======
