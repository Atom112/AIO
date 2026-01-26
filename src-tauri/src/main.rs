// src-tauri/src/main.rs
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // 调用 lib.rs 里的 run 函数
    // 注意：这里的 aio_app 应该替换为你 Cargo.toml 里的项目名称
    // 通常默认为 app_lib::run() 或 [your_project_name]::run()
    aio_lib::run(); 
}