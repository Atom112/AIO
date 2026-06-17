use crate::LocalLlamaState;
use std::io::{BufRead, BufReader};
use tauri::path::BaseDirectory;
use tauri::Manager;
use tokio::task;
use tokio::time::{sleep, Duration};

// 仅在 Windows 系统下导入，用于隐藏控制台窗口
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

/// 启动本地大模型服务器
/// @param model_path 模型文件的绝对路径 (.gguf)
/// @param port 指定服务器运行的端口
/// @param gpu_layers 卸载到 GPU 的模型层数 (用于加速)
#[tauri::command]
pub async fn start_local_server(
    app: tauri::AppHandle,
    state: tauri::State<'_, LocalLlamaState>,
    model_path: String,
    port: u16,
    gpu_layers: i32,
) -> Result<String, String> {
    println!(
        "[DEBUG] 启动参数 - 模型: {}, 端口: {}, GPU层数: {}",
        model_path, port, gpu_layers
    );

    // 1. 参数验证
    if gpu_layers <= 0 {
        return Err("GPU 层数必须大于 0，建议设置为 99 或 999".to_string());
    }

    // 2. 启动前清理：如果已经有一个正在运行的服务器，先关闭它
    stop_local_server(state.clone()).await?;

    // 给操作系统一点时间释放端口
    sleep(Duration::from_millis(500)).await;

    // 3. 路径解析：获取侧载 (sidecar) 的可执行文件路径
    // 预期路径: resources/llama-backend/llama-server.exe
    let resource_dir = app
        .path()
        .resolve("resources/llama-backend", BaseDirectory::Resource)
        .map_err(|e| format!("无法解析资源路径: {}", e))?;

    let exe_path = resource_dir.join("llama-server.exe");

    // 检查文件是否存在
    if !exe_path.exists() {
        return Err(format!("找不到执行文件: {:?}", exe_path));
    }

    if !std::path::Path::new(&model_path).exists() {
        return Err(format!("模型文件不存在: {}", model_path));
    }

    // 4. 构建命令行指令
    let mut cmd = std::process::Command::new(&exe_path);
    cmd.current_dir(&resource_dir) // 设置执行目录
        .args([
            "-m",
            &model_path, // 模型路径
            "--port",
            &port.to_string(), // 监听端口
            "-ngl",
            &gpu_layers.to_string(), // GPU 层数
            "-c",
            "4096", // 上下文窗口大小
            "--host",
            "127.0.0.1", // 仅监听本地地址
        ])
        .stdout(std::process::Stdio::piped()) // 捕获标准输出
        .stderr(std::process::Stdio::piped()); // 捕获标准错误（llama.cpp 默认将日志输出到 stderr）

    // 5. Windows 平台特殊处理：隐藏黑色控制台窗口
    #[cfg(target_os = "windows")]
    cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW 标志

    // 6. 启动进程
    let mut child = cmd.spawn().map_err(|e| format!("启动失败: {}", e))?;

    // 7. 日志实时监控：新开一个线程读取服务器输出日志
    let stderr = child.stderr.take().expect("无法获取 stderr");
    task::spawn_blocking(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines() {
            if let Ok(line) = line {
                // 将本地模型的日志打印到后端控制台，方便调试
                println!("[llama-server] {}", line);

                // 关键词监控：可以根据日志输出判断 GPU 是否挂载成功
                if line.contains("offloaded") {
                    println!("GPU 卸载状态: {}", line);
                }
                if line.contains("CUDA") {
                    println!("CUDA 信息: {}", line);
                }
                if line.contains("error") || line.contains("Error") || line.contains("failed") {
                    println!("LLAMA 错误: {}", line);
                }
            }
        }
    });

    // 8. 等待并检查进程是否崩溃
    sleep(Duration::from_millis(2000)).await;
    match child.try_wait() {
        Ok(None) => println!("进程正常运行中"),
        Ok(Some(status)) => {
            return Err(format!("进程启动后立即退出，退出码: {}", status));
        }
        Err(e) => return Err(format!("无法检查进程状态: {}", e)),
    }

    // 9. 健康检查：通过 HTTP 请求确认服务真正可用
    let client = reqwest::Client::new();
    let health_url = format!("http://127.0.0.1:{}/health", port);

    match client
        .get(&health_url)
        .timeout(Duration::from_secs(5))
        .send()
        .await
    {
        Ok(_) => println!("健康检查通过"),
        Err(_) => {
            let _ = child.kill(); // 如果访问不到健康接口，杀掉进程
            return Err("服务未响应健康检查，可能启动失败".to_string());
        }
    }

    // 10. 全局状态存储：保留子进程句柄以便后续关闭
    {
        let mut lock = state.child_process.lock().unwrap();
        *lock = Some(child);
    }

    // 返回 API 基础地址给前端
    Ok(format!("http://127.0.0.1:{}/v1", port))
}

/// 停止本地服务器
#[tauri::command]
pub async fn stop_local_server(state: tauri::State<'_, LocalLlamaState>) -> Result<(), String> {
    let mut lock = state.child_process.lock().unwrap();
    // take() 会把所有权取出并置空
    if let Some(mut child) = lock.take() {
        println!("[DEBUG] 正在停止本地服务器...");
        let _ = child.kill(); // 强制杀死进程
    }
    Ok(())
}

/// 检查本地服务器是否正在运行
#[tauri::command]
pub fn is_local_server_running(state: tauri::State<'_, LocalLlamaState>) -> bool {
    let mut lock = state.child_process.lock().unwrap();
    if let Some(child) = lock.as_mut() {
        // try_wait 不会阻塞，若返回 None 表示进程还在跑
        match child.try_wait() {
            Ok(None) => return true,
            _ => return false,
        }
    }
    false
}
