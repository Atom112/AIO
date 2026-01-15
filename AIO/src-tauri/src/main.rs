// src-tauri/src/main.rs
// 在发布（release）构建中防止额外的控制台窗口，切勿移除！！
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// 从标准库和外部 crate 导入必要的模块
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::env; // 用于环境变量（作为可选的回退）

// 定义与前端结构匹配的 Assistant 结构体
#[derive(Serialize, Deserialize, Clone)]
struct Assistant {
    id: String,
    name: String,
    // 根据需要添加其他字段
}

// --- 命令 ---

/// 从 JSON 文件加载 assistants 的命令。
/// 在 Tauri v2 中，宏更简单：#[tauri::command]
#[tauri::command]
async fn load_assistants() -> Result<Vec<Assistant>, String> { // 已添加 'async'
    let file_path = get_data_file_path()?;

    // 检查文件是否存在
    if !file_path.exists() {
        println!(
            "Data file does not exist at {:?}, returning empty list.",
            file_path
        );
        // 如果文件尚不存在则返回空列表。
        // 前端负责创建初始的 assistant。
        return Ok(vec![]);
    }

    // 读取文件内容
    let contents = fs::read_to_string(&file_path)
        .map_err(|e| format!("Failed to read file '{:?}': {}", file_path, e))?;

    // 将 JSON 内容解析为 Vec<Assistant>
    let assistants: Vec<Assistant> = serde_json::from_str(&contents)
        .map_err(|e| format!("Failed to parse JSON from '{:?}': {}", file_path, e))?;

    Ok(assistants)
}

/// 将 assistant 列表保存到 JSON 文件的命令。
#[tauri::command]
async fn save_assistants(assistants: Vec<Assistant>) -> Result<(), String> { // 已添加 'async'
    let file_path = get_data_file_path()?;

    // 确保父目录存在
    if let Some(parent) = file_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create directory '{:?}': {}", parent, e))?;
    }

    // 将 assistants 向量序列化为格式化的 JSON 字符串
    let json = serde_json::to_string_pretty(&assistants)
        .map_err(|e| format!("Failed to serialize assistants to JSON: {}", e))?;

    // 将 JSON 字符串写入文件
    fs::write(&file_path, json)
        .map_err(|e| format!("Failed to write file '{:?}': {}", file_path, e))?;

    println!("Assistants successfully saved to {:?}", file_path);
    Ok(())
}

/// 帮助函数：确定 assistants 数据文件的路径。
/// 使用 `dirs` crate 提供的系统配置目录。
fn get_data_file_path() -> Result<PathBuf, String> {
    // 尝试使用 `dirs` crate 获取配置目录
    match dirs::config_dir() {
        Some(mut path) => {
            // 附加应用的特定文件夹和文件名
            // TODO: 为你的应用自定义这些标识符
            path.push("YourCompanyNameOrName"); // 例如 "MyCompany"
            path.push("YourAppName");          // 例如 "MyAwesomeAIApp"
            path.push("assistants.json");
            Ok(path)
        }
        None => {
            // 当 dirs::config_dir 失败时的回退机制（罕见）
            eprintln!("Warning: Could not determine config directory. Using current working directory.");
            let mut path = env::current_dir()
                .map_err(|e| format!("Failed to get current directory: {}", e))?;
            path.push("assistants_fallback.json"); // 使用不同名称以避免混淆
            Ok(path)
        }
    }
}

// --- 命令结束 ---

fn main() {
    tauri::Builder::default()
        // 注册自定义命令，以便从前端调用
        // 注意：在 v2 中直接传入函数名
        .invoke_handler(tauri::generate_handler![
            load_assistants,
            save_assistants
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}