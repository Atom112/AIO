//! LSP (Language Server Protocol) plugin system
//!
//! Modeled after the `plugins/mcp/` architecture, providing:
//! - `LspManager`: registry that manages multiple language servers
//! - `LspClient`: connection, initialization, and diagnostics collection for a single language server
//! - Language auto-detection: match language servers by project marker files

pub mod client;
pub mod error;
pub mod transport;

use dashmap::DashMap;
use std::collections::BTreeMap;
use std::path::Path;

pub use client::LspClient;
pub use error::LspResult;

/// Language server configuration
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LanguageServerConfig {
    /// Language identifier (e.g. "typescript", "rust", "go", "python")
    pub language_id: String,
    /// Display name
    pub name: String,
    /// Executable command
    pub command: String,
    /// Command-line arguments
    pub args: Vec<String>,
    /// Marker files for auto-detection (e.g. ["tsconfig.json", "jsconfig.json"])
    pub marker_files: Vec<String>,
    /// File extension list
    pub extensions: Vec<String>,
}

/// LSP manager - centrally manages all language server clients
pub struct LspManager {
    /// Active language server clients: key = "{project_path}/{language_id}"
    pub clients: DashMap<String, LspClient>,
    /// Supported language configurations
    pub languages: Vec<LanguageServerConfig>,
}

impl LspManager {
    /// Create an LspManager and register all built-in language support
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

    /// Auto-detect which language servers should be started for a project root path
    pub fn auto_detect(&self, project_path: &Path) -> Vec<LanguageServerConfig> {
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

        // For JavaScript/TypeScript: also start when package.json exists without a tsconfig
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

    /// Get or create the client key
    fn client_key(project_path: &Path, language_id: &str) -> String {
        format!("{}/{}", project_path.to_string_lossy(), language_id)
    }

    /// Register an active client
    pub fn register_client(&self, project_path: &Path, language_id: &str, client: LspClient) {
        let key = Self::client_key(project_path, language_id);
        self.clients.insert(key, client);
    }

    /// Get an active client
    pub fn get_client(&self, project_path: &Path, language_id: &str) -> Option<LspClient> {
        let key = Self::client_key(project_path, language_id);
        self.clients.get(&key).map(|c| c.clone())
    }

    /// Remove and shut down a client
    pub async fn remove_client(&self, project_path: &Path, language_id: &str) -> LspResult<()> {
        let key = Self::client_key(project_path, language_id);
        if let Some((_, client)) = self.clients.remove(&key) {
            client.shutdown().await?;
        }
        Ok(())
    }

    /// List all active clients
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

    /// Get all diagnostics for a given project
    pub fn get_all_diagnostics(
        &self,
        project_path: &Path,
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
                result.entry(uri).or_insert_with(Vec::new).extend(diags);
            }
        }

        result
    }

    /// List supported language configurations (for frontend use)
    pub fn list_supported_languages(&self) -> Vec<LanguageServerConfig> {
        self.languages.clone()
    }
}
