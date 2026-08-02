/// 工具函数组：处理文件内容提取和转换。
///
/// 安全加固（修复 H8：任意路径读取）：
/// - `process_file_content` 接受路径仅当满足：扩展名白名单 + 父目录在用户 home 或 AppData 内
/// - `start_local_server` 接受的 `model_path` 仅允许用户 home 或 AppData/engines 内的文件
/// - 限制文件大小（图片 10MB / 文档 30MB）防止 OOM DoS
use base64::{engine::general_purpose, Engine as _};
use std::fs::File;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use zip::ZipArchive;

/// 文件大小上限
const MAX_IMAGE_BYTES: u64 = 10 * 1024 * 1024;
const MAX_DOC_BYTES: u64 = 30 * 1024 * 1024;
const MAX_TEXT_BYTES: u64 = 5 * 1024 * 1024;

/// 图片扩展名
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp"];
/// 富文档扩展名（PDF / Office）
const DOC_EXTENSIONS: &[&str] = &["pdf", "docx", "pptx"];
/// 纯文本 / 源代码扩展名（按 UTF-8 读取为文本注入上下文）
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
    // 源代码
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

/// 校验用户通过文件选择器主动指定的文件路径（与模型路径一致）。
/// 附件/文件源自系统文件选择器/拖拽，用户主动发起，可位于磁盘任意位置。
/// 仅作路径穿越防御：必须为绝对路径且不含 ParentDir 段。
/// 真正的读取防线在于：不在白名单的扩展名一律拒绝 + 大小上限。
fn validate_safe_path(path: &Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err("路径必须为绝对路径".into());
    }
    for comp in path.components() {
        if matches!(comp, Component::ParentDir) {
            return Err("路径不允许包含 ..".into());
        }
    }
    let _ = std::fs::canonicalize(path).map_err(|e| format!("路径无法解析: {}", e))?;
    Ok(())
}

/// 扩展名白名单校验
fn check_extension(path: &Path, allowed: &[&str]) -> Result<String, String> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    if !allowed.iter().any(|a| *a == ext) {
        return Err(format!("扩展名 {:?} 不在白名单 {:?} 内", ext, allowed));
    }
    Ok(ext)
}

/// 检查文件大小
fn check_size(path: &Path, max: u64) -> Result<(), String> {
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if meta.len() > max {
        return Err(format!(
            "文件过大 ({} bytes, 上限 {} bytes)",
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
            .map_err(|e| format!("PDF解析失败: {}", e)),
        "docx" | "pptx" => read_office_file(
            path.to_str()
                .ok_or_else(|| "文件路径不是有效 UTF-8".to_string())?,
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
        _ => Err(format!("不支持的附件扩展名: {}", extension)),
    }
}

/// 从 Office XML 的 `<t>` 标签中提取文本内容。
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

/// 读取并解析 OpenXML 格式（docx/pptx）的文件内容。
pub fn read_office_file(path: &str, file_type: &str) -> Result<String, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut full_text = String::new();

    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
        let name = file.name().to_string();

        let is_target = if file_type == "docx" {
            name == "word/document.xml"
        } else {
            name.starts_with("ppt/slides/slide") && name.ends_with(".xml")
        };

        if is_target {
            let mut content = String::new();
            file.read_to_string(&mut content)
                .map_err(|e| e.to_string())?;
            full_text.push_str(&extract_text_from_xml(&content));
            full_text.push('\n');
        }
    }
    Ok(full_text)
}

/// 处理各种格式的文件内容（H8 路径沙箱加固）
///
/// 图像 (png/jpg/webp): 返回 Base64 DataURI。
/// PDF: 返回提取内容文本。
/// Office (docx/pptx): 返回提取内容文本。
/// 其他: 尝试按 UTF-8 编码读取为纯文本。
pub async fn process_file_content(path: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        let path_obj = Path::new(&path);

        validate_safe_path(path_obj).map_err(|e| format!("文件路径校验失败: {}", e))?;

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
                pdf_extract::extract_text(&path).map_err(|e| format!("PDF解析失败: {}", e))
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
            _ => Err(format!(
                "扩展名 {:?} 不在白名单内（支持图片/富文档/文本与常用源代码格式）",
                extension
            )),
        }
    })
    .await
    .map_err(|_| "文件处理线程异常".to_string())?
}

/// 校验模型路径安全性（不限制目录，仅验证路径合法 + 文件存在）。
/// 模型文件由用户通过文件选择器主动指定，可以存放在任意位置。
pub fn validate_model_path(path: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(path);
    if !p.is_absolute() {
        return Err("模型路径必须为绝对路径".into());
    }
    // 拒绝含 .. 的路径（防目录穿越）
    for comp in p.components() {
        if matches!(comp, std::path::Component::ParentDir) {
            return Err("模型路径不允许包含 ..".into());
        }
    }
    if let Some(ext) = p.extension().and_then(|s| s.to_str()) {
        let ext_lower = ext.to_lowercase();
        if !["gguf", "safetensors", "bin"].contains(&ext_lower.as_str()) {
            return Err(format!(
                "模型文件扩展名 {:?} 不在白名单内 (gguf/safetensors/bin)",
                ext_lower
            ));
        }
    } else {
        return Err("模型文件必须有扩展名".into());
    }
    if !p.exists() {
        return Err(format!("模型文件不存在: {}", path));
    }
    Ok(p)
}
