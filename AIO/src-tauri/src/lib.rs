// src-tauri/src/lib.rs
// 关于此文件：
// 该文件为 Tauri 应用在 Rust 侧的入口/初始化逻辑以及示例命令定义处。
// - 在 Tauri 应用中，Rust 端可以通过 `#[tauri::command]` 导出可被前端调用的函数（命令）。
// - `run()` 函数负责构建并运行 Tauri 应用（插件注册、命令绑定、上下文注入等）。


// `#[tauri::command]` 属性将该函数导出为 Tauri 可调用命令。
// 这意味着前端（例如 JS/TS / Solid）可以通过 Tauri 的 `invoke` API 调用此函数。
// 函数签名使用了借用的字符串切片 `&str`，返回一个 `String`。
// 函数的实现为演示用途，将接收到的 `name` 格式化为一个问候字符串并返回。
#[tauri::command]
fn greet(name: &str) -> String {
    // `format!` 返回一个堆分配的 `String`，适合通过 FFI/JSON 返回给前端。
    format!("Hello, {}! You've been greeted from Rust!", name)
}

// `#[cfg_attr(mobile, tauri::mobile_entry_point)]`：
// - 在启用 `mobile` 配置时，将 `run` 标记为移动平台的入口点（适用于 Tauri 的移动目标）。
// - 在桌面环境下这行宏不会改变函数签名，`run` 照常作为构建和运行应用的函数。
// `pub fn run()`：公开的运行函数，通常由 `main.rs` 或移动入口调用以启动 Tauri 应用。
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 创建 Tauri 应用构建器并按需进行配置：
    // - `tauri::Builder::default()`：获取默认的 Builder 实例。
    // - `.plugin(...)`：注册插件（此处示例为 `tauri_plugin_opener`，用于打开 URL / 外部资源等）。
    // - `.invoke_handler(tauri::generate_handler![greet])`：注册可被前端调用的命令列表。
    //    - `tauri::generate_handler!` 宏会为列出的函数生成一个 invoke handler，
    //      使得这些函数可以在前端通过 `invoke('greet', { name: '...' })` 调用。
    // - `.run(tauri::generate_context!())`：读取应用运行时上下文并启动事件循环。
    //    - `tauri::generate_context!()` 宏会从 `tauri.conf.json`（或相应环境）生成上下文信息，
    //      包括 windows 配置、打包信息等。
    // - `.expect(...)`：若启动过程中发生错误则触发 panic 并输出错误信息（便于调试）。
    tauri::Builder::default()
        // 注册 `tauri_plugin_opener` 插件（如果你使用该插件来打开外部链接或处理 opener 事件）
        .plugin(tauri_plugin_opener::init())
        // 将 `greet` 命令暴露给前端；可以在这里添加更多命令：tauri::generate_handler![greet, other_cmd, ...]
        .invoke_handler(tauri::generate_handler![greet])
        // 读取运行时上下文并运行应用（会阻塞直到应用退出或出错）
        .run(tauri::generate_context!())
        // 如果运行出错，打印错误并 panic（可根据需要改为更优雅的错误处理）
        .expect("error while running tauri application");
}