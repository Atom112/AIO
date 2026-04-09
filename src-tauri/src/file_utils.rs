/// 工具函数组：处理文件内容提取和转换。

use base64::{engine::general_purpose, Engine as _};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use zip::ZipArchive;

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

/// 处理各种格式的文件内容。
///
/// 支持格式
/// 图像 (png/jpg/webp): 返回 Base64 DataURI。
/// PDF: 返回提取内容文本。
/// Office (docx/pptx): 返回提取内容文本。
/// 其他: 尝试按 UTF-8 编码读取为纯文本。
#[tauri::command]
pub async fn process_file_content(path: String) -> Result<String, String> {
    let path_obj = Path::new(&path);
    let extension = path_obj.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();

    match extension.as_str() {
        "png" | "jpg" | "jpeg" | "webp" => {
            let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
            let b64 = general_purpose::STANDARD.encode(bytes);
            Ok(format!("data:image/{};base64,{}", extension, b64))
        }
        "pdf" => pdf_extract::extract_text(&path).map_err(|e| format!("PDF解析失败: {}", e)),
        "docx" | "pptx" => read_office_file(&path, &extension),
        _ => {
            let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
            let (res, _, _) = encoding_rs::UTF_8.decode(&bytes);
            Ok(res.into_owned())
        }
    }
}