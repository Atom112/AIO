//! 出站 HTTP 的公共约定（默认身份 / 客户端起手式）。
//!
//! **reqwest 默认不发送 `User-Agent`**（与 curl、浏览器不同）——这在本项目里已经
//! 造成过真实故障，不是风格问题：
//!
//! - 2026-10：本地模型市场下载 ModelScope 模型全部失败，报
//!   `403 Forbidden`（`download DeepCoder-14B-Preview-Q4_K_M.gguf failed
//!   (all sources tried): HTTP 403 Forbidden`），而同一台机器用浏览器从
//!   modelscope.cn 网页下载完全正常。原因：`/models/<repo>/resolve/<rev>/<file>`
//!   302 到对象存储 CDN `cdn-lfs-cn-1.modelscope.cn`，该 CDN 前置的 WAF
//!   （响应带 `acw_tc` cookie）对**缺失 UA 的取流请求直接回 403**；ModelScope
//!   自身的 JSON API（`/api/v1/...`）不校验 UA，所以市场搜索、文件列表都正常，
//!   只有文件下载全灭。
//! - 同理，GitHub REST API 与 HuggingFace Hub 的使用条款都要求可识别的 UA
//!   （GitHub 对无 UA 请求直接 403）。
//!
//! 因此：**任何面向第三方站点/对象存储/API 的出站客户端都必须带 UA**。新增
//! 出站客户端请用 [`client_builder`] 起手（已带默认 UA），不要裸
//! `reqwest::Client::new()`；确需自定义（例如 MCP Registry、Skill Market 想用
//! 更细的产品标识）时，`.user_agent(...)` 也必须带上版本号。
//!
//! 注意 UA 只在**跨主机重定向**中保留——reqwest 只剥离 `Authorization` /
//! `Cookie` / `Proxy-Authorization`，所以 resolve → CDN 的两跳链路里 UA 会带到
//! 真正取流的那一跳（这正是修复生效的原因）。

/// 出站请求的默认身份：`AIO-Desktop/<包版本>`。
///
/// 版本号随包版本自动更新，便于上游按版本排查异常流量；LLM 网关（如 OpenCode Go）
/// 也会据此识别非通用 SDK 客户端。
pub fn user_agent() -> String {
    format!("AIO-Desktop/{}", env!("CARGO_PKG_VERSION"))
}

/// 出站客户端的起手 builder（已带默认 [`user_agent`]）。
///
/// 调用方按需继续追加超时/代理/连接池参数后 `.build()`。
pub fn client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder().user_agent(user_agent())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// UA 必须自报产品名与版本（上游 WAF 只要求"有 UA"，但排查问题靠版本号）。
    #[test]
    fn user_agent_identifies_product_and_version() {
        let ua = user_agent();
        assert!(ua.starts_with("AIO-Desktop/"), "unexpected UA: {ua}");
        assert!(
            ua.ends_with(env!("CARGO_PKG_VERSION")),
            "UA must carry the crate version: {ua}"
        );
        assert!(!ua.contains(' '), "UA must not contain spaces: {ua}");
    }

    /// 回归护栏：`client_builder()` 起手的客户端必须真的把 UA 发出去。
    ///
    /// 这是 2026-10 ModelScope 403 的直接回归测试——当时下载器用
    /// `Client::builder()` 裸起，请求里没有 UA 头，被对象存储 CDN 挡下。
    /// 用一个最小本地 HTTP 服务器抓取请求头，不依赖外网。
    #[test]
    fn client_builder_sends_user_agent_header() {
        use std::io::{Read, Write};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("绑定本地端口失败");
        let addr = listener.local_addr().expect("读取本地端口失败");
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("接受连接失败");
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).expect("读取请求失败");
            let head = String::from_utf8_lossy(&buf[..n]).to_string();
            let _ = stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
            head
        });

        let client = client_builder().build().expect("构建客户端失败");
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("构建运行时失败");
        rt.block_on(async {
            let resp = client
                .get(format!("http://{addr}/probe"))
                .send()
                .await
                .expect("请求失败");
            assert_eq!(resp.status(), 200);
            let _ = resp.bytes().await;
        });

        let head = server.join().expect("服务器线程失败").to_lowercase();
        let expected = format!("user-agent: {}", user_agent().to_lowercase());
        assert!(
            head.contains(&expected),
            "请求缺少 {expected}；实际请求头：{head}"
        );
    }
}
