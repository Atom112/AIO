// src-tauri/src/main.rs
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // --fs-server <path>  启动内置文件系统 MCP server（Agent 模式使用）
    if args.len() >= 3 && args[1] == "--fs-server" {
        aio_lib::mcp_fs_server::run(args[2].clone());
        return;
    }

    aio_lib::run();
}
