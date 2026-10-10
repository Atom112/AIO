//! Parsing, disk persistence and markdown token rewriting for model-generated images.
//!
//! Generated-image output has two sources:
//! - `image_url`/`output_image` blocks in the `delta.content` array of chat/completions streams
//!   (vLLM Qwen-Image, OpenAI gpt-image-1 chat mode, Gemini, etc.), plus markdown images inlined in text.
//! - `b64_json`/`url` returned by the dedicated `/images/generations` endpoint (dall-e-2/3, etc.).
//!
//! Images are written to `<app_data_dir>/attachments/generated/<uuid>.<ext>`, and image URLs /
//! data URIs in message content are rewritten to local tokens `![img](aio-image://<base64(abs_path)>)`.
//! base64 avoids markdown special characters; the frontend decodes with `atob` to get the absolute
//! path, then converts it to a displayable URL via `convertFileSrc`.

use crate::core::models::GeneratedImage;
use crate::ierr;
use base64::Engine;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};
use url::Url;

/// URL scheme the frontend markdown renderer uses to recognize locally generated images.
pub const IMAGE_TOKEN_SCHEME: &str = "aio-image://";

/// Max bytes per image (20MB); larger images are skipped.
const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;

/// Only raster image types may be persisted; SVG and other unsafe types are rejected.
fn is_safe_raster_mime(mime: &str) -> bool {
    matches!(
        mime,
        "image/png" | "image/jpeg" | "image/webp" | "image/gif" | "image/avif" | "image/bmp"
    )
}

/// Decodes a data URI of the form `data:(image/...);base64,<data>`.
///
/// Returns `(mime, decoded bytes)`; `None` for non-image data URIs, a missing base64 prefix, or a failed decode.
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

/// Extracts image URLs (data URIs or http(s)) from markdown content.
///
/// Returns a deduplicated, order-preserving list. Only matches the `![alt](url)` form, not reference-style images.
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

/// Rewrites raw image URLs found in markdown content to `![img](<token>)`.
///
/// `replacements` is a list of `(raw_url, token)`; a URL is rewritten only where it actually appears as a markdown image.
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

/// Removes all `aio-image://` generated-image tokens from content.
///
/// Sanitizes token-bearing final text before it is sent back to the LLM API (prevents token leakage).
pub fn strip_generated_image_tokens(content: &str) -> String {
    let re = regex::Regex::new(r"!\[[^\]]*\]\(aio-image://[^)]*\)").expect("valid regex");
    re.replace_all(content, "").into_owned()
}

/// Generates the frontend-recognizable image token for an absolute path.
pub fn image_token_for(abs_path: &str) -> String {
    format!(
        "{}{}",
        IMAGE_TOKEN_SCHEME,
        base64::engine::general_purpose::STANDARD.encode(abs_path.as_bytes())
    )
}

/// Maps a mime type to a file extension; unknown types fall back to `bin`.
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

/// Saves generated images and rewrites message content.
///
/// - `content`: rewritten in place: raw URLs appearing in markdown become local tokens; images
///   from array deltas that do not appear in markdown are appended as `![name](token)`.
/// - `raw_urls`: image_url array items obtained from stream parsing (may be empty).
///
/// A single failed image only `warn`s and is skipped; it never fails the whole turn. Returns metadata of persisted images.
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

/// Core logic that writes images into `dir` and rewrites `content` (decoupled from AppHandle for testability).
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
                // Reject non-raster types such as SVG so unsanitized vector content never lands on disk
                if !is_safe_raster_mime(&mime) {
                    tracing::warn!("跳过非栅格生成图片 mime ({:?}): {}", mime, url);
                    continue;
                }
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
                if let Err(e) = crate::core::fs_util::atomic_write(&path, &bytes) {
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
        // Array-delta image not embedded in the text: append it to the end of content.
        appended.push_str(&format!("\n\n![{}]({})", "image", token));
    }
    if !appended.trim().is_empty() {
        content.push_str(&appended);
    }
    rewrite_markdown_image_urls(content, &replacements);
    Ok(images)
}

/// Resolves a redirect Location (relative paths supported) to an absolute URL.
fn resolve_redirect(base: &str, location: &str) -> Option<String> {
    Url::parse(base)
        .ok()?
        .join(location)
        .ok()
        .map(|u| u.to_string())
}

/// Fetches the bytes of a single image.
///
/// - data URI → decoded directly (the caller validates the mime against the safe whitelist)
/// - http(s) → downloaded after per-hop SSRF validation (30s timeout, no auth headers; every redirect hop is re-validated)
/// - other → `Ok(None)`
async fn fetch_image(url: &str) -> Result<Option<(String, Vec<u8>)>, String> {
    if url.starts_with("data:image/") {
        return Ok(decode_data_uri(url));
    }
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Ok(None);
    }

    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())?;

    let mut current = url.to_string();
    for _hop in 0..=5u32 {
        crate::utils::url_validation::validate_http_url(
            &current,
            &crate::utils::url_validation::HttpUrlOptions::local_engine(),
        )
        .map_err(|e| e.to_string())?;

        let resp = client
            .get(&current)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if resp.status().is_redirection() {
            let loc = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| {
                    ierr!(
                        "重定向缺少 Location 头",
                        "Redirect is missing the Location header"
                    )
                })?;
            current = resolve_redirect(&current, loc).ok_or_else(|| {
                ierr!(
                    "无法解析重定向地址",
                    "Failed to resolve the redirect address"
                )
            })?;
            continue;
        }

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
        return Ok(Some((mime, bytes)));
    }

    Err(ierr!(
        "重定向次数超过上限 (5)",
        "Redirect limit exceeded (5)"
    ))
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

        // Array-delta image: not embedded in markdown, should be appended to the end of content
        let mut content = "Here is your image:\n\n".to_string();
        let imgs = save_images_to_dir(&dir, &mut content, std::slice::from_ref(&uri))
            .await
            .expect("save");
        assert_eq!(imgs.len(), 1);
        assert!(content.contains("aio-image://"));
        let expected_tail = format!("\n\n![image]({})", image_token_for(&imgs[0].storage_path));
        assert!(content.ends_with(&expected_tail));
        // File is on disk with the correct size and extension
        let saved = std::fs::metadata(&imgs[0].storage_path).expect("file exists");
        assert_eq!(saved.len(), 4);
        assert_eq!(imgs[0].size, 4);
        assert!(imgs[0].storage_path.ends_with(".png"));

        // Markdown-inlined image: should be rewritten in place to a local token
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
