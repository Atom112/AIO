//! # Provider 插件系统
//!
//! 仿 LobeHub 架构：每个 provider（OpenAI / Anthropic / Google Gemini / Ollama 等）
//! 实现 [`ProviderPlugin`] trait，描述其特定的：
//! - 端点 URL 拼接规则
//! - HTTP 认证头（Bearer / x-goog-api-key / x-api-key / 无 auth）
//! - 响应 JSON 字段映射（`{data:[{id}]}` / `{models:[{name}]}` / `{data:[{id,display_name}]}`）
//! - 拉取失败时的硬编码默认模型列表
//!
//! [`ProviderManager`] 按 host 派发到第一个匹配的插件；无匹配时回退到 [`openai_compat`]。
//!
//! 与 [`crate::plugins::engine`] 的本地推理引擎插件形态对齐（同步 trait，
//! 内部 I/O 由调用方在 async 命令中完成），避免引入 `async_trait` 依赖。

pub mod anthropic;
pub mod google;
pub mod ollama;
pub mod openai_compat;

use crate::core::models::LiveModel;
use crate::ierr;

/// 构造带超时与代理的 reqwest client（4 个插件的共享实现——历史上逐字复制了 4 份）。
/// user-agent 携带 crate 版本号，避免硬编码版本随发布漂移。
pub fn build_http_client(
    proxy_url: Option<&str>,
    timeout_secs: u64,
    provider_tag: &str,
) -> Result<reqwest::Client, String> {
    let mut b = reqwest::Client::builder()
        .user_agent(format!(
            "AIO-Desktop/{} ({}-provider)",
            env!("CARGO_PKG_VERSION"),
            provider_tag
        ))
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(timeout_secs));
    if let Some(p) = proxy_url {
        if !p.trim().is_empty() {
            let proxy = reqwest::Proxy::all(p)
                .map_err(|e| ierr!("代理 URL 非法: {}", "Invalid proxy URL: {}", e))?;
            b = b.proxy(proxy);
        }
    }
    b.build().map_err(|e| {
        ierr!(
            "构造 HTTP 客户端失败: {}",
            "Failed to build HTTP client: {}",
            e
        )
    })
}

/// Provider 插件 trait —— 各厂商 API 适配器
pub trait ProviderPlugin: Send + Sync {
    /// 机器标识符，如 "google" / "anthropic" / "openai_compat" / "ollama"
    fn identifier(&self) -> &'static str;

    /// 通过 host 匹配判断此插件是否适用于该 api_url
    fn matches(&self, api_url: &str) -> bool;

    /// 构造带超时与代理的 reqwest client（默认实现 = build_http_client(identifier 作 tag)）
    fn build_client(
        &self,
        proxy_url: Option<&str>,
        timeout_secs: u64,
    ) -> Result<reqwest::Client, String> {
        build_http_client(proxy_url, timeout_secs, self.identifier())
    }

    /// 计算 list models 端点 URL
    fn models_url(&self, api_url: &str) -> String;

    /// 给 RequestBuilder 附加认证头
    fn apply_auth(&self, req: reqwest::RequestBuilder, api_key: &str) -> reqwest::RequestBuilder;

    /// 从 JSON 响应解析为统一 LiveModel 列表
    fn parse_models(&self, body: &serde_json::Value) -> Vec<LiveModel>;

    /// 计算 chat completions 端点 URL（含 provider 特定路径前缀）。
    /// 默认行为：去除尾部斜杠和已有 /chat/completions 后缀，再拼接 /chat/completions。
    /// Ollama 等 provider 可 override 以插入 /v1 等前缀。
    fn chat_completions_url(&self, api_url: &str) -> String {
        let base = api_url
            .trim_end_matches('/')
            .replace("/chat/completions", "");
        format!("{}/chat/completions", base)
    }
}

/// Provider 插件管理器：按 host 派发到第一个匹配的插件
pub struct ProviderManager {
    plugins: Vec<Box<dyn ProviderPlugin>>,
}

impl ProviderManager {
    pub fn new() -> Self {
        let mut mgr = Self {
            plugins: Vec::new(),
        };
        // 注册顺序敏感：先注册专属插件，openai_compat 必须放最后作为兜底
        mgr.register(Box::new(google::GoogleProvider));
        mgr.register(Box::new(anthropic::AnthropicProvider));
        mgr.register(Box::new(ollama::OllamaProvider));
        mgr.register(Box::new(openai_compat::OpenAICompatProvider));
        mgr
    }

    pub fn register(&mut self, plugin: Box<dyn ProviderPlugin>) {
        self.plugins.push(plugin);
    }

    /// 按 api_url 找第一个匹配的插件；找不到时返回最后一个（openai_compat 兜底）
    pub fn for_url(&self, api_url: &str) -> &dyn ProviderPlugin {
        self.plugins
            .iter()
            .find(|p| p.matches(api_url))
            .map(|p| p.as_ref())
            .unwrap_or_else(|| {
                self.plugins
                    .last()
                    .expect("at least one provider plugin must be registered")
                    .as_ref()
            })
    }

    /// [`for_url`] 的拥有型版本：按 identifier 重建一个 `Box<dyn ProviderPlugin>`。
    /// 需要把插件移出管理器作用域时使用（如命令层的"测试连接/拉取模型"）；
    /// 新增插件时必须在此注册一个分支（order 测试会保证兜底在末位）。
    pub fn boxed_for_url(&self, api_url: &str) -> Box<dyn ProviderPlugin> {
        let matched = self.for_url(api_url).identifier();
        match matched {
            "google" => Box::new(google::GoogleProvider),
            "anthropic" => Box::new(anthropic::AnthropicProvider),
            "ollama" => Box::new(ollama::OllamaProvider),
            _ => Box::new(openai_compat::OpenAICompatProvider),
        }
    }
}

#[cfg(test)]
mod manager_tests {
    use super::*;

    /// 注册顺序不变式：openai_compat（兜底）必须始终在末位。
    /// for_url 的兜底逻辑假设"最后一个插件是 openai_compat"；顺序被改动会静默错派。
    #[test]
    fn openai_compat_stays_last() {
        let mgr = ProviderManager::new();
        let last = mgr.plugins.last().unwrap();
        assert_eq!(last.identifier(), "openai_compat");
        // 通用 URL 必须派发到兜底
        assert_eq!(
            mgr.for_url("https://api.example.com/v1").identifier(),
            "openai_compat"
        );
    }
}

/// 把 reqwest 错误分类为用户友好提示（跟随 locale）
pub fn classify_reqwest_error(err: &reqwest::Error, host: &str) -> String {
    if err.is_timeout() {
        ierr!(
            "请求超时 ({}): 请检查网络或代理设置",
            "Request timed out ({}): check your network or proxy settings",
            host
        )
    } else if err.is_connect() {
        ierr!(
            "网络无法访问 {}: 请检查网络或配置代理 (proxyUrl)",
            "Cannot reach {}: check your network or configure a proxy (proxyUrl)",
            host
        )
    } else if err.is_request() {
        ierr!("请求错误 ({}): {}", "Request error ({}): {}", host, err)
    } else {
        ierr!("HTTP 客户端错误: {}", "HTTP client error: {}", err)
    }
}

/// 标准超时
pub const DEFAULT_TIMEOUT_SECS: u64 = 30;
/// 测试连接用较短超时
pub const TEST_TIMEOUT_SECS: u64 = 20;

/// Unix 秒时间戳 → YYYY-MM-DD（用于 OpenAI `created` 字段）。
/// 实现统一由 utils::time 提供。
pub fn unix_to_ymd(secs: i64) -> Option<String> {
    crate::utils::time::unix_to_ymd(secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unix_to_ymd() {
        // 2025-01-01 00:00:00 UTC = 1735689600
        assert_eq!(unix_to_ymd(1_735_689_600), Some("2025-01-01".to_string()));
        // 2000-01-01 00:00:00 UTC = 946684800
        assert_eq!(unix_to_ymd(946_684_800), Some("2000-01-01".to_string()));
        // 1970-01-01 00:00:00 UTC = 0
        assert_eq!(unix_to_ymd(0), Some("1970-01-01".to_string()));
    }
}
