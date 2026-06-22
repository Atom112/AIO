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

/// 校验路径在沙箱内
/// 允许的根：用户 home、AppData/config、AppData、临时目录
fn path_in_sandbox(path: &Path) -> Result<(), String> {
    // 必须为绝对路径且无 ParentDir 段
    if !path.is_absolute() {
        return Err("路径必须为绝对路径".into());
    }
    for comp in path.components() {
        if matches!(comp, Component::ParentDir) {
            return Err("路径不允许包含 ..".into());
        }
    }

    let canonical = std::fs::canonicalize(path).map_err(|e| format!("路径无法解析: {}", e))?;
    let canonical_str = canonical.to_string_lossy().to_lowercase();

    let mut allowed_roots: Vec<PathBuf> = Vec::new();
    if let Some(home) = dirs::home_dir() {
        allowed_roots.push(home);
    }
    if let Ok(appdata) = std::env::var("APPDATA") {
        allowed_roots.push(PathBuf::from(appdata));
    }
    if let Some(config) = dirs::config_dir() {
        allowed_roots.push(config);
    }
    if let Ok(xdg_data) = std::env::var("XDG_DATA_HOME") {
        allowed_roots.push(PathBuf::from(xdg_data));
    }
    // Tauri 标准 app_data_dir
    if let Some(local) = dirs::data_local_dir() {
        allowed_roots.push(local);
    }

    for root in allowed_roots {
        let root_canon = std::fs::canonicalize(&root).unwrap_or(root);
        let root_str = root_canon.to_string_lossy().to_lowercase();
        if canonical_str.starts_with(&root_str) {
            return Ok(());
        }
    }
    Err("路径不在允许的沙箱目录内".into())
}

/// 扩展名白名单校验
fn check_extension(path: &Path, allowed: &[&str]) -> Result<String, String> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    if !allowed.iter().any(|a| *a == ext) {
        return Err(format!(
            "扩展名 {:?} 不在白名单 {:?} 内",
            ext, allowed
        ));
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
        _ => "application/octet-stream",
    }
}

/// Validates a user-selected attachment path, extension, sandbox location, and size.
pub fn validate_attachment_path(path: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(path);
    path_in_sandbox(&path)?;
    let extension = check_extension(
        &path,
        &[
            "png", "jpg", "jpeg", "webp", "pdf", "docx", "pptx", "txt", "md", "json",
            "csv", "log", "xml", "yaml", "yml", "ini", "tsv",
        ],
    )?;
    let max = if ["png", "jpg", "jpeg", "webp"].contains(&extension.as_str()) {
        MAX_IMAGE_BYTES
    } else if ["pdf", "docx", "pptx"].contains(&extension.as_str()) {
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
            path.to_str().ok_or_else(|| "文件路径不是有效 UTF-8".to_string())?,
            extension,
        )
        .map(Some),
        "txt" | "md" | "json" | "csv" | "log" | "xml" | "yaml" | "yml" | "ini"
        | "tsv" => {
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
                if name.local_name == "t" { in_text_tag = true; }
            }
            Ok(xml::reader::XmlEvent::Characters(content)) => {
                if in_text_tag { out.push_str(&content); }
            }
            Ok(xml::reader::XmlEvent::EndElement { name, .. }) => {
                if name.local_name == "t" { in_text_tag = false; }
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
            file.read_to_string(&mut content).map_err(|e| e.to_string())?;
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
#[tauri::command]
pub async fn process_file_content(path: String) -> Result<String, String> {
    let path_obj = Path::new(&path);

    // 沙箱校验
    if let Err(e) = path_in_sandbox(path_obj) {
        return Err(format!("文件路径沙箱拒绝: {}", e));
    }

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
        _ => Err(format!(
            "扩展名 {:?} 不在白名单内（支持 png/jpg/jpeg/webp/pdf/docx/pptx/txt/md/json/csv/log/xml/yaml/ini/tsv）",
            extension
        )),
    }
}

/// 校验模型路径在沙箱内（H8 强化）
pub fn validate_model_path(path: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(path);
    if !p.is_absolute() {
        return Err("模型路径必须为绝对路径".into());
    }
    if let Some(ext) = p.extension().and_then(|s| s.to_str()) {
        let ext_lower = ext.to_lowercase();
        if !["gguf", "safetensors", "bin"].contains(&ext_lower.as_str()) {
            return Err(format!("模型文件扩展名 {:?} 不在白名单内 (gguf/safetensors/bin)", ext_lower));
        }
    } else {
        return Err("模型文件必须有扩展名".into());
    }
    path_in_sandbox(&p)?;
    Ok(p)
}
