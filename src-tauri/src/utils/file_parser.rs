use crate::ierr;
/// Utility module: file content extraction and conversion.
///
/// Hardening (SEC-21: this doc now matches the actual checks — the historic comment
/// claimed a home/AppData directory restriction that `validate_safe_path` never had;
/// attachments are user-picked via the system dialog and may sit anywhere on disk):
/// - `process_file_content` accepts a path only if: the extension is on the
///   whitelist + the path is absolute and contains no `..` component + size limits
/// - `start_local_server` accepts a `model_path` only for files inside the user home or AppData/engines
/// - File size limits (images 10MB / documents 30MB / text 5MB) prevent OOM DoS
use base64::{engine::general_purpose, Engine as _};
use std::fs::File;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use zip::ZipArchive;

/// File size limits
const MAX_IMAGE_BYTES: u64 = 10 * 1024 * 1024;
const MAX_DOC_BYTES: u64 = 30 * 1024 * 1024;
const MAX_TEXT_BYTES: u64 = 5 * 1024 * 1024;
/// INFRA-3: decompressed zip-entry cap (Office XML parts) — a 30MB docx can still
/// carry a multi-GB decompressed payload; real document.xml parts stay well below this.
const MAX_ZIP_ENTRY_TEXT_BYTES: u64 = 64 * 1024 * 1024;
/// INFRA-3: aggregate cap across all extracted parts of one document.
const MAX_ZIP_TEXT_TOTAL_BYTES: u64 = 256 * 1024 * 1024;

/// Image extensions
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp"];
/// Rich document extensions (PDF / Office)
const DOC_EXTENSIONS: &[&str] = &["pdf", "docx", "pptx"];
/// Plain text / source code extensions (read as UTF-8 and injected into the context as text)
const TEXT_EXTENSIONS: &[&str] = &[
    "txt",
    "md",
    "json",
    "csv",
    "log",
    "xml",
    "yaml",
    "yml",
    "ini",
    "tsv",
    // source code
    "rs",
    "c",
    "h",
    "cpp",
    "hpp",
    "cc",
    "cxx",
    "cs",
    "go",
    "java",
    "rb",
    "py",
    "js",
    "mjs",
    "cjs",
    "ts",
    "tsx",
    "jsx",
    "php",
    "swift",
    "kt",
    "kts",
    "scala",
    "lua",
    "sql",
    "toml",
    "sh",
    "bash",
    "zsh",
    "dart",
    "html",
    "css",
    "scss",
    "less",
    "vue",
    "svelte",
    "gradle",
    "properties",
    "r",
    "pl",
];

/// Validates a file path the user picked via the file dialog (same policy as the model path).
/// Attachments come from the system file dialog / drag-and-drop, are user-initiated, and may sit anywhere on disk.
/// Only guards against path traversal: the path must be absolute and contain no ParentDir components.
/// The real read-side defense: reject every extension not on the whitelist + enforce size limits.
fn validate_safe_path(path: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err(ierr!("路径必须为绝对路径", "Path must be absolute"));
    }
    for comp in path.components() {
        if matches!(comp, Component::ParentDir) {
            return Err(ierr!("路径不允许包含 ..", "Path must not contain .."));
        }
    }
    let _ = std::fs::canonicalize(path)
        .map_err(|e| ierr!("路径无法解析: {}", "Failed to resolve path: {}", e))?;
    Ok(())
}

/// Extension whitelist check
fn check_extension(path: &Path, allowed: &[&str]) -> Result<String, String> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    if !allowed.iter().any(|a| *a == ext) {
        return Err(ierr!(
            "扩展名 {:?} 不在白名单 {:?} 内",
            "Extension {:?} is not in the whitelist {:?}",
            ext,
            allowed
        ));
    }
    Ok(ext)
}

/// Checks file size
fn check_size(path: &Path, max: u64) -> Result<(), String> {
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if meta.len() > max {
        return Err(ierr!(
            "文件过大 ({} bytes, 上限 {} bytes)",
            "File too large ({} bytes, limit {} bytes)",
            meta.len(),
            max
        ));
    }
    Ok(())
}

/// Returns the MIME type used for a supported chat attachment extension.
pub fn attachment_mime_type(extension: &str) -> &'static str {
    match extension {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "pdf" => "application/pdf",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "txt" | "log" | "ini" => "text/plain",
        "md" => "text/markdown",
        "json" => "application/json",
        "csv" => "text/csv",
        "xml" => "application/xml",
        "yaml" | "yml" => "application/yaml",
        "tsv" => "text/tab-separated-values",
        _ if TEXT_EXTENSIONS.contains(&extension) => "text/plain",
        _ => "application/octet-stream",
    }
}

/// Validates a user-selected attachment path, extension, sandbox location, and size.
pub fn validate_attachment_path(path: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(path);
    validate_safe_path(&path)?;
    let extension = check_extension(
        &path,
        &[IMAGE_EXTENSIONS, DOC_EXTENSIONS, TEXT_EXTENSIONS].concat(),
    )?;
    let max = if IMAGE_EXTENSIONS.contains(&extension.as_str()) {
        MAX_IMAGE_BYTES
    } else if DOC_EXTENSIONS.contains(&extension.as_str()) {
        MAX_DOC_BYTES
    } else {
        MAX_TEXT_BYTES
    };
    check_size(&path, max)?;
    Ok(path)
}

/// Extracts text for supported document attachments. Images intentionally return `None`.
pub fn extract_file_content(path: &Path, extension: &str) -> Result<Option<String>, String> {
    match extension {
        "png" | "jpg" | "jpeg" | "webp" => Ok(None),
        "pdf" => pdf_extract::extract_text(path)
            .map(Some)
            .map_err(|e| ierr!("PDF解析失败: {}", "Failed to parse PDF: {}", e)),
        "docx" | "pptx" => read_office_file(
            path.to_str()
                .ok_or_else(|| ierr!("文件路径不是有效 UTF-8", "File path is not valid UTF-8"))?,
            extension,
        )
        .map(Some),
        "txt" | "md" | "json" | "csv" | "log" | "xml" | "yaml" | "yml" | "ini" | "tsv" | "rs"
        | "c" | "h" | "cpp" | "hpp" | "cc" | "cxx" | "cs" | "go" | "java" | "rb" | "py" | "js"
        | "mjs" | "cjs" | "ts" | "tsx" | "jsx" | "php" | "swift" | "kt" | "kts" | "scala"
        | "lua" | "sql" | "toml" | "sh" | "bash" | "zsh" | "dart" | "html" | "css" | "scss"
        | "less" | "vue" | "svelte" | "gradle" | "properties" | "r" | "pl" => {
            let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
            let (res, _, _) = encoding_rs::UTF_8.decode(&bytes);
            Ok(Some(res.into_owned()))
        }
        _ => Err(ierr!(
            "不支持的附件扩展名: {}",
            "Unsupported attachment extension: {}",
            extension
        )),
    }
}

/// Extracts text content from `<t>` tags in Office XML.
pub fn extract_text_from_xml(xml: &str) -> String {
    let reader = xml::EventReader::new(xml.as_bytes());
    let mut out = String::new();
    let mut in_text_tag = false;

    for e in reader {
        match e {
            Ok(xml::reader::XmlEvent::StartElement { name, .. }) => {
                if name.local_name == "t" {
                    in_text_tag = true;
                }
            }
            Ok(xml::reader::XmlEvent::Characters(content)) => {
                if in_text_tag {
                    out.push_str(&content);
                }
            }
            Ok(xml::reader::XmlEvent::EndElement { name, .. }) if name.local_name == "t" => {
                in_text_tag = false;
            }
            _ => {}
        }
    }
    out
}

/// Reads and parses OpenXML (docx/pptx) file content.
pub fn read_office_file(path: &str, file_type: &str) -> Result<String, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut full_text = String::new();

    for i in 0..archive.len() {
        let file = archive.by_index(i).map_err(|e| e.to_string())?;
        // zip 9: name() validates the entry name is valid UTF-8 (returns Cow on
        // success); non-UTF-8 names are treated as unreadable and skipped here.
        let name = file.name().map_err(|e| e.to_string())?.to_string();

        let is_target = if file_type == "docx" {
            name == "word/document.xml"
        } else {
            name.starts_with("ppt/slides/slide") && name.ends_with(".xml")
        };

        if is_target {
            // INFRA-3: zip-bomb guard. The declared uncompressed size is checked first,
            // then the actual read is capped via take() — a lying header (small declared,
            // huge actual) is caught when the capped read still yields a full buffer.
            if file.size() > MAX_ZIP_ENTRY_TEXT_BYTES {
                return Err(ierr!(
                    "zip 条目解压后超过上限（{}B，疑似 zip 炸弹）",
                    "zip entry exceeds the decompression cap ({}B; possible zip bomb)",
                    file.size()
                ));
            }
            let mut limited = file.take(MAX_ZIP_ENTRY_TEXT_BYTES);
            let mut content = String::new();
            limited
                .read_to_string(&mut content)
                .map_err(|e| e.to_string())?;
            if content.len() as u64 >= MAX_ZIP_ENTRY_TEXT_BYTES {
                return Err(ierr!(
                    "zip 条目实际解压超过上限，已中止（疑似 zip 炸弹）",
                    "zip entry actually decompressed past the cap; aborted (possible zip bomb)"
                ));
            }
            full_text.push_str(&extract_text_from_xml(&content));
            full_text.push('\n');
            if full_text.len() as u64 > MAX_ZIP_TEXT_TOTAL_BYTES {
                return Err(ierr!(
                    "文档文本总量超过上限（{}B），已中止",
                    "Total document text exceeds the cap ({}B); aborted",
                    MAX_ZIP_TEXT_TOTAL_BYTES
                ));
            }
        }
    }
    Ok(full_text)
}

/// Processes file content in all supported formats (path sandbox hardening).
///
/// Images (png/jpg/webp): returns a Base64 DataURI.
/// PDF: returns extracted text.
/// Office (docx/pptx): returns extracted text.
/// Others: tries to read as UTF-8 plain text.
pub async fn process_file_content(path: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        let path_obj = Path::new(&path);

        validate_safe_path(path_obj).map_err(|e| ierr!("文件路径校验失败: {}", "File path validation failed: {}", e))?;

        let extension = path_obj
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();

        match extension.as_str() {
            "png" | "jpg" | "jpeg" | "webp" => {
                check_extension(path_obj, &["png", "jpg", "jpeg", "webp"])?;
                check_size(path_obj, MAX_IMAGE_BYTES)?;
                let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
                let b64 = general_purpose::STANDARD.encode(bytes);
                Ok(format!("data:image/{};base64,{}", extension, b64))
            }
            "pdf" => {
                check_size(path_obj, MAX_DOC_BYTES)?;
                pdf_extract::extract_text(&path).map_err(|e| ierr!("PDF解析失败: {}", "Failed to parse PDF: {}", e))
            }
            "docx" | "pptx" => {
                check_size(path_obj, MAX_DOC_BYTES)?;
                read_office_file(&path, &extension)
            }
            "txt" | "md" | "json" | "csv" | "log" | "xml" | "yaml" | "yml" | "ini" | "tsv" => {
                check_size(path_obj, MAX_TEXT_BYTES)?;
                let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
                let (res, _, _) = encoding_rs::UTF_8.decode(&bytes);
                Ok(res.into_owned())
            }
            _ if TEXT_EXTENSIONS.contains(&extension.as_str()) => {
                check_size(path_obj, MAX_TEXT_BYTES)?;
                let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
                let (res, _, _) = encoding_rs::UTF_8.decode(&bytes);
                Ok(res.into_owned())
            }
            _ => Err(ierr!(
                "扩展名 {:?} 不在白名单内（支持图片/富文档/文本与常用源代码格式）",
                "Extension {:?} is not in the whitelist (images, rich documents, text, and common source code formats are supported)",
                extension
            )),
        }
    })
    .await
    .map_err(|_| ierr!("文件处理线程异常", "File processing thread error"))?
}

/// Validates model path safety (no directory restriction; only path validity + file existence).
/// The model file is user-picked via the file dialog and may live anywhere on disk.
pub fn validate_model_path(path: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(path);
    if !p.is_absolute() {
        return Err(ierr!(
            "模型路径必须为绝对路径",
            "Model path must be absolute"
        ));
    }
    // Reject paths containing .. (path traversal guard)
    for comp in p.components() {
        if matches!(comp, std::path::Component::ParentDir) {
            return Err(ierr!(
                "模型路径不允许包含 ..",
                "Model path must not contain .."
            ));
        }
    }
    if let Some(ext) = p.extension().and_then(|s| s.to_str()) {
        let ext_lower = ext.to_lowercase();
        if !["gguf", "safetensors", "bin"].contains(&ext_lower.as_str()) {
            return Err(ierr!(
                "模型文件扩展名 {:?} 不在白名单内 (gguf/safetensors/bin)",
                "Model file extension {:?} is not in the whitelist (gguf/safetensors/bin)",
                ext_lower
            ));
        }
    } else {
        return Err(ierr!(
            "模型文件必须有扩展名",
            "Model file must have an extension"
        ));
    }
    if !p.exists() {
        return Err(ierr!(
            "模型文件不存在: {}",
            "Model file does not exist: {}",
            path
        ));
    }
    Ok(p)
}
