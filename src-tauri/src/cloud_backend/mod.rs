//! # 云端后端集成（Cloud Backend）
//!
//! 集中管理所有指向**预留云端后端**（即处理用户登录、账户同步的远端 Java 服务，
//! 非本地 Rust 后端也非 LLM Provider）的 HTTP 调用。
//!
//! ## 设计目标
//! - **唯一入口**：base URL 仅在 [`config`] 模块维护，支持 `AIO_CLOUD_BACKEND_URL` 环境变量覆盖
//! - **统一超时**：所有请求走 [`client::http_client`]，禁止命令层自建 `reqwest::Client`
//! - **统一错误**：所有错误归并为 [`client::CloudBackendError`]，命令层在边界做 `to_string()`
//! - **可扩展**：新增端点时，在 [`auth`]（或新增 `profile.rs`/`sync.rs`）中加函数，并到 [`mod.rs`] 暴露
//!
//! ## 端点清单
//! | 命令 | 方法 | 路径 | 用途 |
//! |------|------|------|------|
//! | `login_to_backend` | POST | `/api/auth/login` | 用户名/密码登录，返回 JWT |
//! | `register_to_backend` | POST | `/api/auth/register` | 邮箱+密码注册 |
//! | `validate_token` | GET | `/api/auth/validate` | 校验 JWT 有效性 |
//! | `sync_avatar_to_backend` | POST | `/api/auth/update-avatar` | 同步头像到云端 |
//! | `logout_clear` | - | - | 清本地 keyring 中 token |
//! | `read_auth_token` | - | - | 读 keyring 中 token |
//!
//! ## 安全约束
//! - base URL 强制 HTTPS（`config::base_url` 启动时校验）
//! - Token 仅在系统钥匙串中持久化（[`crate::core::secure_store`]），不写 localStorage
//! - 错误信息做脱敏后回传前端

pub mod auth;
pub mod client;
pub mod config;
