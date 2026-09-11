//! Skill 域模型：技能配置、来源与市场条目。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Skill 来源类型。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum SkillSource {
    /// 手动创建或旧数据（缺省值）
    #[default]
    Local,
    /// 从 skills.sh 市场下载
    Market,
    /// 从 npm/npx 生态导入
    Npx,
}

/// 可复用的助手系统指令模块。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SkillConfig {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub content: String,
    /// Skill 来源。旧数据缺省反序列化为 Local。
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
    /// npx 包名（source = Npx 时必填），如 "@anthropic-ai/skill-docx"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub npx_package: Option<String>,
    /// npx 包的已安装版本号
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub npx_version: Option<String>,
    /// npx 执行命令（默认等于 npx_package）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub npx_command: Option<String>,
}

/// Skill 持久化文件。
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

/// skills.sh 市场中的 Skill 摘要。
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
}

/// skills.sh 官方主题分类。
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SkillMarketCategory {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub skill_count: usize,
}

// ====== Agent 模式配置 ======
