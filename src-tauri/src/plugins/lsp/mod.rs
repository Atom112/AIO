//! LSP (Language Server Protocol) 插件系统
//!
//! 仿照 `plugins/mcp/` 的架构，提供：
//! - `LspManager`：管理多个语言服务器的注册中心
//! - `LspClient`：单个语言服务器的连接、初始化、诊断收集
//! - 语言自动检测：根据项目特征文件匹配对应的语言服务器

pub mod client;
pub mod error;
pub mod transport;

use dashmap::DashMap;
use std::collections::BTreeMap;
use std::path::PathBuf;

pub use client::LspClient;
pub use error::LspResult;

/// 语言服务器配置
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LanguageServerConfig {
    /// 语言标识符（如 "typescript", "rust", "go", "python"）
    pub language_id: String,
    /// 显示名称
    pub name: String,
    /// 可执行文件命令
    pub command: String,
    /// 命令行参数
    pub args: Vec<String>,
    /// 特征文件列表（用于自动检测，如 ["tsconfig.json", "jsconfig.json"]）
    pub marker_files: Vec<String>,
    /// 文件扩展名列表
    pub extensions: Vec<String>,
}

/// LSP 管理器 — 集中管理所有语言服务器客户端
pub struct LspManager {
    /// 活跃的语言服务器客户端：key = "{project_path}/{language_id}"
    pub clients: DashMap<String, LspClient>,
    /// 支持的语言配置
    pub languages: Vec<LanguageServerConfig>,
}

impl LspManager {
    /// 创建 LspManager 并注册所有内置语言支持
    pub fn new() -> Self {
        let languages = vec![
            LanguageServerConfig {
                language_id: "typescript".into(),
                name: "TypeScript".into(),
                command: "typescript-language-server".into(),
                args: vec!["--stdio".into()],
                marker_files: vec!["tsconfig.json".into(), "jsconfig.json".into()],
                extensions: vec![
                    ".ts".into(),
                    ".tsx".into(),
                    ".js".into(),
                    ".jsx".into(),
                    ".mjs".into(),
                    ".cjs".into(),
                ],
            },
            LanguageServerConfig {
                language_id: "rust".into(),
                name: "Rust".into(),
                command: "rust-analyzer".into(),
                args: vec![],
                marker_files: vec!["Cargo.toml".into()],
                extensions: vec![".rs".into()],
            },
            LanguageServerConfig {
                language_id: "go".into(),
                name: "Go".into(),
                command: "gopls".into(),
                args: vec![],
                marker_files: vec!["go.mod".into(), "go.sum".into()],
                extensions: vec![".go".into()],
            },
            LanguageServerConfig {
                language_id: "python".into(),
                name: "Python (Pyright)".into(),
                command: "pyright-langserver".into(),
                args: vec!["--stdio".into()],
                marker_files: vec![
                    "pyproject.toml".into(),
                    "setup.py".into(),
                    "setup.cfg".into(),
                    "requirements.txt".into(),
                    "Pipfile".into(),
                ],
                extensions: vec![".py".into(), ".pyi".into()],
            },
        ];

        LspManager {
            clients: DashMap::new(),
            languages,
        }
    }

    /// 根据项目根路径自动检测应该启动哪些语言服务器
    pub fn auto_detect(&self, project_path: &PathBuf) -> Vec<LanguageServerConfig> {
        let mut detected = Vec::new();

        for lang in &self.languages {
            for marker in &lang.marker_files {
                let marker_path = project_path.join(marker);
                if marker_path.exists() {
                    detected.push(lang.clone());
                    break;
                }
            }
        }

        // 对于 JavaScript/TypeScript，如果存在 package.json 但无 tsconfig
        let has_ts = detected.iter().any(|l| l.language_id == "typescript");
        if !has_ts {
            let pkg_json = project_path.join("package.json");
            if pkg_json.exists() {
                if let Some(ts_lang) = self
                    .languages
                    .iter()
                    .find(|l| l.language_id == "typescript")
                {
                    detected.push(ts_lang.clone());
                }
            }
        }

        detected
    }

    /// 获取或创建客户端 key
    fn client_key(project_path: &PathBuf, language_id: &str) -> String {
        format!("{}/{}", project_path.to_string_lossy(), language_id)
    }

    /// 注册一个活跃客户端
    pub fn register_client(&self, project_path: &PathBuf, language_id: &str, client: LspClient) {
        let key = Self::client_key(project_path, language_id);
        self.clients.insert(key, client);
    }

    /// 获取活跃客户端
    pub fn get_client(&self, project_path: &PathBuf, language_id: &str) -> Option<LspClient> {
        let key = Self::client_key(project_path, language_id);
        self.clients.get(&key).map(|c| c.clone())
    }

    /// 移除并关闭客户端
    pub async fn remove_client(
        &self,
        project_path: &PathBuf,
        language_id: &str,
    ) -> LspResult<()> {
        let key = Self::client_key(project_path, language_id);
        if let Some((_, client)) = self.clients.remove(&key) {
            client.shutdown().await?;
        }
        Ok(())
    }

    /// 列出所有活跃客户端
    #[allow(dead_code)]
    pub fn list_clients(&self) -> Vec<(String, String)> {
        self.clients
            .iter()
            .map(|entry| {
                let key = entry.key().clone();
                let lang = entry.value().language_id.clone();
                (key, lang)
            })
            .collect()
    }

    /// 获取指定项目的所有诊断
    pub fn get_all_diagnostics(
        &self,
        project_path: &PathBuf,
        file_path: Option<&str>,
    ) -> BTreeMap<String, Vec<lsp_types::Diagnostic>> {
        let mut result = BTreeMap::new();
        let project_prefix = project_path.to_string_lossy().to_string();

        for entry in self.clients.iter() {
            let key = entry.key();
            let key_str: &str = key;
            if !key_str.starts_with(&project_prefix) {
                continue;
            }
            let client = entry.value();
            for (uri, diags) in client.get_diagnostics(file_path) {
                result
                    .entry(uri)
                    .or_insert_with(Vec::new)
                    .extend(diags);
            }
        }

        result
    }

    /// 列出支持的语言配置（供前端调用）
    pub fn list_supported_languages(&self) -> Vec<LanguageServerConfig> {
        self.languages.clone()
    }
}
