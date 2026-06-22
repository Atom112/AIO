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

/// Provider 插件 trait —— 各厂商 API 适配器
#[allow(dead_code)]
pub trait ProviderPlugin: Send + Sync {
    /// 机器标识符，如 "google" / "anthropic" / "openai_compat" / "ollama"
    fn identifier(&self) -> &'static str;

    /// 人类可读名称
    fn name(&self) -> &'static str;

    /// 通过 host 匹配判断此插件是否适用于该 api_url
    fn matches(&self, api_url: &str) -> bool;

    /// 构造带超时与代理的 reqwest client
    fn build_client(
        &self,
        proxy_url: Option<&str>,
        timeout_secs: u64,
    ) -> Result<reqwest::Client, String>;

    /// 计算 list models 端点 URL
    fn models_url(&self, api_url: &str) -> String;

    /// 给 RequestBuilder 附加认证头
    fn apply_auth(
        &self,
        req: reqwest::RequestBuilder,
        api_key: &str,
    ) -> reqwest::RequestBuilder;

    /// 从 JSON 响应解析为统一 LiveModel 列表
    fn parse_models(&self, body: &serde_json::Value) -> Vec<LiveModel>;
}

/// Provider 插件管理器：按 host 派发到第一个匹配的插件
#[allow(dead_code)]
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
}

/// 把 reqwest 错误分类为用户友好提示（中文）
pub fn classify_reqwest_error(err: &reqwest::Error, host: &str) -> String {
    if err.is_timeout() {
        format!("请求超时 ({}): 请检查网络或代理设置", host)
    } else if err.is_connect() {
        format!("网络无法访问 {}: 请检查网络或配置代理 (proxyUrl)", host)
    } else if err.is_request() {
        format!("请求错误 ({}): {}", host, err)
    } else {
        format!("HTTP 客户端错误: {}", err)
    }
}

/// 标准超时
pub const DEFAULT_TIMEOUT_SECS: u64 = 30;
/// 测试连接用较短超时
pub const TEST_TIMEOUT_SECS: u64 = 20;

/// Unix 秒时间戳 → YYYY-MM-DD（用于 OpenAI `created` 字段）
/// 不引入 chrono 依赖，手算 Howard Hinnant 日期算法
pub fn unix_to_ymd(secs: i64) -> Option<String> {
    days_since_epoch_to_ymd(secs.div_euclid(86_400))
}

/// 自 1970-01-01 起的天数 → YYYY-MM-DD（Gregorian）
fn days_since_epoch_to_ymd(days: i64) -> Option<String> {
    // Howard Hinnant 算法：http://howardhinnant.github.io/date_algorithms.html
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    Some(format!("{:04}-{:02}-{:02}", y, m, d))
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
