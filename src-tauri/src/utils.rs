//! 文件内容提取工具模块
//!
//! 该模块提供了从多种文件格式（如 .docx, .pptx, .pdf 和纯文本）中提取文本的功能。
//! 它主要用于支持 Tauri 应用的文件处理流程。

use base64::{engine::general_purpose, Engine as _};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use zip::ZipArchive;
/// 从 XML 字符串中提取位于 `<t>` 标签内的文本。
///
/// 在 Office Open XML 格式（如 .docx 和 .pptx）中，文本内容通常封装在 `<t>` (text) 标签内。
///
/// # 参数
/// * `xml` - 包含文本的原始 XML 字符串。
///
/// # 返回值
/// 返回提取并拼接后的纯文本字符串。
pub fn extract_text_from_xml(xml: &str) -> String {
    let reader = xml::EventReader::new(xml.as_bytes());
    let mut out = String::new();
    let mut in_text_tag = false;

    for e in reader {
        match e {
            // 检测开始标签是否为 <t>
            Ok(xml::reader::XmlEvent::StartElement { name, .. }) => {
                if name.local_name == "t" {
                    in_text_tag = true;
                }
            }
            // 如果在 <t> 标签内，则抓取字符内容
            Ok(xml::reader::XmlEvent::Characters(content)) => {
                if in_text_tag {
                    out.push_str(&content);
                }
            }
            // 检测结束标签并关闭标志位
            Ok(xml::reader::XmlEvent::EndElement { name, .. }) => {
                if name.local_name == "t" {
                    in_text_tag = false;
                }
            }
            _ => {}
        }
    }
    out
}

/// 读取并解析 Office 文件（DOCX 或 PPTX）。
///
/// 由于这些格式本质上是 ZIP 压缩包，此函数会解压并查找特定的 XML 文件来提取文本。
///
/// # 参数
/// * `path` - 文件的磁盘路径。
/// * `file_type` - 文件类型标识，支持 "docx" 或 "pptx"。
///
/// # 错误
/// 如果文件无法打开、不是有效的 ZIP 格式或读取失败，将返回包含错误信息的 `String`。
pub fn read_office_file(path: &str, file_type: &str) -> Result<String, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    // 将文件作为 ZIP 存档打开
    let mut archive = ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut full_text = String::new();

    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
        let name = file.name().to_string();

        // 根据文件类型匹配目标 XML 文件
        // Word 核心内容在 word/document.xml
        // PowerPoint 核心内容在 ppt/slides/slideN.xml
        let is_target = if file_type == "docx" {
            name == "word/document.xml"
        } else {
            name.starts_with("ppt/slides/slide") && name.ends_with(".xml")
        };

        if is_target {
            let mut content = String::new();
            file.read_to_string(&mut content)
                .map_err(|e| e.to_string())?;
            // 提取并追加文本
            full_text.push_str(&extract_text_from_xml(&content));
            full_text.push('\n');
        }
    }
    Ok(full_text)
}

/// 处理文件内容的 Tauri 命令函数。
///
/// 根据文件扩展名自动选择合适的提取逻辑。
/// 支持的格式：
/// - `.pdf`: 使用 `pdf_extract` 库提取。
/// - `.docx`, `.pptx`: 使用 `read_office_file` 逻辑处理。
/// - 其他: 尝试作为 UTF-8 编码的纯文本读取。
///
/// # 参数
/// * `path` - 需要处理的文件完整路径。
///
/// # 返回值
/// 成功时返回提取出的所有文本，失败时返回错误描述字符串。
#[tauri::command]
pub async fn process_file_content(path: String) -> Result<String, String> {
    let path_obj = Path::new(&path);
    let extension = path_obj
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();

    match extension.as_str() {
        // --- 新增图片处理逻辑 ---
        "png" | "jpg" | "jpeg" | "webp" => {
            let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
            let b64 = general_purpose::STANDARD.encode(bytes);
            Ok(format!("data:image/{};base64,{}", extension, b64))
        }

        // 原有的 PDF/Office/Text 处理保持不变...
        "pdf" => pdf_extract::extract_text(&path).map_err(|e| format!("PDF解析失败: {}", e)),
        "docx" | "pptx" => read_office_file(&path, &extension),
        _ => {
            let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
            let (res, _, _) = encoding_rs::UTF_8.decode(&bytes);
            Ok(res.into_owned())
        }
    }
}
