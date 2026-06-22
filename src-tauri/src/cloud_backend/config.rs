//! 云端后端配置层
//!
//! 唯一负责 base URL 与端点路径定义。修改 host / 切换 staging 环境仅需改此处。

use std::env;
use std::sync::OnceLock;

/// 默认生产环境 base URL（HTTPS）
const DEFAULT_BASE_URL: &str = "https://localhost:8443";

/// 环境变量覆盖 key（dev / staging / e2e 测试用）
const ENV_BASE_URL: &str = "AIO_CLOUD_BACKEND_URL";

/// API 路径前缀（与后端 Java 服务约定）
pub const API_PREFIX: &str = "/api/auth";

/// 缓存首次校验后的 base URL
static BASE_URL: OnceLock<String> = OnceLock::new();

/// 启动时校验 + 缓存的 base URL
/// - 优先读 `AIO_CLOUD_BACKEND_URL` 环境变量
/// - 兜底用 `DEFAULT_BASE_URL`
/// - 强制必须是 `https://` 开头
pub fn base_url() -> &'static str {
    BASE_URL.get_or_init(|| {
        let raw = env::var(ENV_BASE_URL).unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());
        let normalized = raw.trim_end_matches('/').to_string();
        if !normalized.to_lowercase().starts_with("https://") {
            // 启动期日志：使用 warn 让运维可见
            eprintln!(
                "[cloud_backend] WARNING: base URL {:?} is not HTTPS, falling back to default",
                normalized
            );
            return DEFAULT_BASE_URL.to_string();
        }
        normalized
    })
}

/// 拼接完整 API URL
///
/// # Example
/// ```
/// let url = cloud_backend::config::api_url("/login");
/// // => "https://localhost:8443/api/auth/login"
/// ```
pub fn api_url(path: &str) -> String {
    let normalized = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{}", path)
    };
    format!("{}{}{}", base_url(), API_PREFIX, normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_base_url_is_https() {
        assert!(base_url().starts_with("https://"));
    }

    #[test]
    fn api_url_joins_correctly() {
        assert_eq!(api_url("/login"), format!("{}{}/login", base_url(), API_PREFIX));
        assert_eq!(api_url("login"), format!("{}{}/login", base_url(), API_PREFIX));
    }
}
