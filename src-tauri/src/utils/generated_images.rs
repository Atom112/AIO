//! 模型生成图像的解析、落盘与 markdown 标记重写。
//!
//! 图像生成的输出有两种来源：
//! - chat/completions 流式响应中 `delta.content` 数组里的 `image_url`/`output_image` 块（vLLM
//!   Qwen-Image、OpenAI gpt-image-1 chat 模式、Gemini 等），以及文本内嵌的 markdown 图片。
//! - 专用 `/images/generations` 端点返回的 `b64_json`/`url`（dall-e-2/3 等）。
//!
//! 落盘到 `<app_data_dir>/attachments/generated/<uuid>.<ext>`，并把消息内容里的图片 URL /
//! data URI 重写为本地标记 `![img](aio-image://<base64(abs_path)>)`。base64 编码避免 markdown
//! 特殊字符；前端用 `atob` 直接解码拿到绝对路径，再经 `convertFileSrc` 转成可显示 URL。

use crate::core::models::GeneratedImage;
use base64::Engine;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

/// 前端 Markdown 渲染器识别本地生成图片的 URL scheme。
pub const IMAGE_TOKEN_SCHEME: &str = "aio-image://";

/// 单张图片最大字节数（20MB），超出则跳过。
const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;

/// 解码 `data:(image/...);base64,<data>` 形式的 data URI。
///
/// 返回 `(mime, 解码后的字节)`；非图像 data URI、缺失 base64 前缀或解码失败返回 `None`。
pub fn decode_data_uri(uri: &str) -> Option<(String, Vec<u8>)> {
    let rest = uri.strip_prefix("data:")?;
    let (header, b64) = rest.split_once(';')?;
    if !header.starts_with("image/") || !b64.starts_with("base64,") {
        return None;
    }
    let mime = header.to_string();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64.trim_start_matches("base64,"))
        .ok()?;
    Some((mime, bytes))
}

/// 提取 markdown 内容里的图片 URL（data URI 或 http(s)）。
///
/// 返回去重保序的 URL 列表。仅匹配 `![alt](url)` 形态，不含引用式图片。
pub fn extract_markdown_image_urls(content: &str) -> Vec<String> {
    let re = regex::Regex::new(r"!\[[^\]]*\]\(\s*(data:image/[^)\s]+|https?://[^)\s]+)\s*\)")
        .expect("valid regex");
    let mut seen = std::collections::HashSet::new();
    let mut urls = Vec::new();
    for caps in re.captures_iter(content) {
        if let Some(m) = caps.get(1) {
            let url = m.as_str().to_string();
            if seen.insert(url.clone()) {
                urls.push(url);
            }
        }
    }
    urls
}

/// 把 markdown 内容里出现的原始图片 URL 重写为 `![img](<token>)`。
///
/// `replacements` 为 `(raw_url, token)` 列表；仅在 URL 确实以 markdown 图片形态出现时重写。
pub fn rewrite_markdown_image_urls(content: &mut String, replacements: &[(String, String)]) {
    let mut out = content.clone();
    for (raw_url, token) in replacements {
        let escaped = regex::escape(raw_url);
        let pat =
            regex::Regex::new(&format!(r"!\[[^\]]*\]\(\s*{}\s*\)", escaped)).expect("valid regex");
        out = pat
            .replace_all(&out, format!("![img]({})", token))
            .into_owned();
    }
    *content = out;
}

/// 移除内容里的全部 `aio-image://` 生成图片标记。
///
/// 用于把含标记的最终文本净化后回送给 LLM API（避免 token 泄漏）。
pub fn strip_generated_image_tokens(content: &str) -> String {
    let re = regex::Regex::new(r"!\[[^\]]*\]\(aio-image://[^)]*\)").expect("valid regex");
    re.replace_all(content, "").into_owned()
}

/// 为绝对路径生成前端可识别的图片标记 token。
pub fn image_token_for(abs_path: &str) -> String {
    format!(
        "{}{}",
        IMAGE_TOKEN_SCHEME,
        base64::engine::general_purpose::STANDARD.encode(abs_path.as_bytes())
    )
}

/// 把 mime 映射为文件扩展名；未知类型回退 `bin`。
fn ext_for_mime(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "image/avif" => "avif",
        "image/bmp" => "bmp",
        "image/svg+xml" => "svg",
        _ => "bin",
    }
}

/// 保存生成图片并重写消息内容。
///
/// - `content`：将被就地重写：markdown 里出现的原始 URL 替换为本地标记；来自数组 delta、
///   未出现在 markdown 里的图追加为 `![name](token)`。
/// - `raw_urls`：流式解析得到的 image_url 数组项（可能为空）。
///
/// 单张图片失败只 `warn` 并跳过，绝不使整轮失败。返回落盘图像的元数据。
pub async fn save_generated_images(
    app: &AppHandle,
    content: &mut String,
    raw_urls: &[String],
) -> Result<Vec<GeneratedImage>, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("attachments")
        .join("generated");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    save_images_to_dir(&dir, content, raw_urls).await
}

/// 把图片写入 `dir` 并重写 `content` 的核心逻辑（与 AppHandle 解耦，便于测试）。
async fn save_images_to_dir(
    dir: &std::path::Path,
    content: &mut String,
    raw_urls: &[String],
) -> Result<Vec<GeneratedImage>, String> {
    let mut md_urls = extract_markdown_image_urls(content);
    let mut seen = std::collections::HashSet::new();
    let mut urls: Vec<String> = Vec::new();
    for u in md_urls.drain(..).chain(raw_urls.iter().cloned()) {
        if seen.insert(u.clone()) {
            urls.push(u);
        }
    }
    if urls.is_empty() {
        return Ok(vec![]);
    }

    let dir = dir.to_path_buf();

    let mut images = Vec::new();
    let mut replacements: Vec<(String, String)> = Vec::new();

    for url in urls {
        match fetch_image(&url).await {
            Ok(Some((mime, bytes))) => {
                if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
                    tracing::warn!("跳过生成图片(尺寸超限或为空): {}", url);
                    continue;
                }
                let ext = ext_for_mime(&mime);
                let file_name = format!(
                    "generated_{}.{}",
                    &uuid::Uuid::new_v4().simple().to_string()[..8],
                    ext
                );
                let path = dir.join(&file_name);
                if let Err(e) = std::fs::write(&path, &bytes) {
                    tracing::warn!("写入生成图片失败 {}: {}", url, e);
                    continue;
                }
                let abs = path.canonicalize().unwrap_or_else(|_| PathBuf::from(&path));
                let abs_str = abs.to_string_lossy().to_string();
                images.push(GeneratedImage {
                    name: file_name,
                    mime_type: mime,
                    size: bytes.len() as u64,
                    storage_path: abs_str.clone(),
                });
                replacements.push((url, image_token_for(&abs_str)));
            }
            Ok(None) => {
                tracing::warn!("跳过无法识别的生成图片 URL: {}", url);
            }
            Err(e) => {
                tracing::warn!("获取生成图片失败 {}: {}", url, e);
            }
        }
    }

    if replacements.is_empty() {
        return Ok(images);
    }

    let in_markdown: std::collections::HashSet<String> =
        extract_markdown_image_urls(content).into_iter().collect();
    let mut appended = String::new();
    for (raw, token) in &replacements {
        if in_markdown.contains(raw) {
            continue;
        }
        // 数组 delta 的图未嵌入文本：追加到内容末尾。
        appended.push_str(&format!("\n\n![{}]({})", "image", token));
    }
    if !appended.trim().is_empty() {
        content.push_str(&appended);
    }
    rewrite_markdown_image_urls(content, &replacements);
    Ok(images)
}

/// 获取单张图片字节。
///
/// - data URI → 直接解码
/// - http(s) → SSRF 校验后下载（30s 超时，无鉴权头）
/// - 其它 → `Ok(None)`
async fn fetch_image(url: &str) -> Result<Option<(String, Vec<u8>)>, String> {
    if url.starts_with("data:image/") {
        Ok(decode_data_uri(url))
    } else if url.starts_with("http://") || url.starts_with("https://") {
        crate::utils::url_validation::validate_http_url(
            url,
            &crate::utils::url_validation::HttpUrlOptions::local_engine(),
        )
        .map_err(|e| e.to_string())?;
        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| e.to_string())?;
        let resp = client.get(url).send().await.map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status()));
        }
        let mime = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("image/png")
            .to_string();
        let bytes = resp.bytes().await.map_err(|e| e.to_string())?.to_vec();
        Ok(Some((mime, bytes)))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    #[test]
    fn decode_data_uri_valid() {
        let b64 = base64::engine::general_purpose::STANDARD.encode(b"\x89PNG");
        let uri = format!("data:image/png;base64,{}", b64);
        let (mime, bytes) = decode_data_uri(&uri).expect("decode");
        assert_eq!(mime, "image/png");
        assert_eq!(bytes, b"\x89PNG");
    }

    #[test]
    fn decode_data_uri_invalid() {
        assert!(decode_data_uri("not a uri").is_none());
        assert!(decode_data_uri("data:text/plain;base64,AA==").is_none());
        assert!(decode_data_uri("data:image/png;base64,!!notbase64!!").is_none());
    }

    #[test]
    fn extract_markdown_image_urls_forms() {
        let content = "![a](data:image/png;base64,AAAA) text ![b](https://x.com/i.png) tail";
        let urls = extract_markdown_image_urls(content);
        assert_eq!(
            urls,
            vec!["data:image/png;base64,AAAA", "https://x.com/i.png"]
        );
    }

    #[test]
    fn extract_markdown_image_urls_none_and_dedup() {
        assert!(extract_markdown_image_urls("no image here").is_empty());
        let urls =
            extract_markdown_image_urls("![a](https://x.com/i.png) ![b](https://x.com/i.png)");
        assert_eq!(urls, vec!["https://x.com/i.png"]);
    }

    #[test]
    fn rewrite_markdown_image_urls_in_place() {
        let mut content = "pre ![alt](data:image/png;base64,AAAA) post".to_string();
        rewrite_markdown_image_urls(
            &mut content,
            &[(
                "data:image/png;base64,AAAA".to_string(),
                "aio-image://abc".to_string(),
            )],
        );
        assert_eq!(content, "pre ![img](aio-image://abc) post");
    }

    #[test]
    fn strip_generated_image_tokens_removes() {
        let content = "text ![img](aio-image://AAAA) tail".to_string();
        let stripped = strip_generated_image_tokens(&content);
        assert_eq!(stripped, "text  tail");
    }

    #[tokio::test]
    async fn save_images_to_dir_writes_and_rewrites() {
        let b64 = base64::engine::general_purpose::STANDARD.encode(b"\x89PNG");
        let uri = format!("data:image/png;base64,{}", b64);
        let dir = std::env::temp_dir().join(format!("aio-img-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        // 数组 delta 图：未嵌入 markdown，应追加到内容末尾
        let mut content = "Here is your image:\n\n".to_string();
        let imgs = save_images_to_dir(&dir, &mut content, std::slice::from_ref(&uri))
            .await
            .expect("save");
        assert_eq!(imgs.len(), 1);
        assert!(content.contains("aio-image://"));
        let expected_tail = format!("\n\n![image]({})", image_token_for(&imgs[0].storage_path));
        assert!(content.ends_with(&expected_tail));
        // 文件已写盘且大小/扩展名正确
        let saved = std::fs::metadata(&imgs[0].storage_path).expect("file exists");
        assert_eq!(saved.len(), 4);
        assert_eq!(imgs[0].size, 4);
        assert!(imgs[0].storage_path.ends_with(".png"));

        // markdown 内嵌图：应被就地重写为本地标记
        let mut content2 = format!("pre ![alt]({}) post", uri);
        let imgs2 = save_images_to_dir(&dir, &mut content2, &[])
            .await
            .expect("save2");
        assert_eq!(imgs2.len(), 1);
        assert!(content2.contains("![img](aio-image://"));
        assert!(!content2.contains("data:image"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn image_token_for_roundtrip() {
        let abs = r"C:\Path With Space\生成图 片.png";
        let token = image_token_for(abs);
        assert!(token.starts_with("aio-image://"));
        let b64 = token.trim_start_matches("aio-image://");
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .expect("decode");
        assert_eq!(String::from_utf8(decoded).unwrap(), abs);
    }
}
