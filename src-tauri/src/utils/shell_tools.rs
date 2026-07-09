//! 内置系统命令执行工具 — 供 Agent 运行终端命令。
//!
//! 业界参考：Claude Code 的 Bash 工具（简单 schema + 权限记忆）。
//! 由 `run_agent_turn` 在工具分发时直接调用，跳过 MCP 子进程通道。
//!
//! 安全措施：
//! - 路径沙箱：工作目录限定在项目根目录
//! - 输出截断：stdout/stderr 各 50KB
//! - 超时控制：默认 60s，最大 300s
//! - 危险命令检测：拒绝高危模式（rm -rf /, sudo, dd to /dev, fork bomb 等）
//! - 权限系统：复用现有 Allow/Ask/Deny 规则引擎

use crate::core::models::{ToolResult, ToolResultContent, ToolSpec, ToolFunctionSpec};
use serde_json::json;
use std::process::Command;
use std::time::Duration;

// ====== 常量 ======

const DEFAULT_TIMEOUT_MS: u64 = 60_000;
const MAX_TIMEOUT_MS: u64 = 300_000;
const MAX_OUTPUT_BYTES: usize = 50_000;

// ====== 辅助函数 ======

fn tool_ok(text: String) -> ToolResult {
    ToolResult {
        content: vec![ToolResultContent {
            kind: "text".into(),
            data: json!({"text": text}),
        }],
        is_error: false,
    }
}

fn tool_err(msg: &str) -> ToolResult {
    ToolResult {
        content: vec![ToolResultContent {
            kind: "text".into(),
            data: json!({"text": format!("[Error] {msg}")}),
        }],
        is_error: true,
    }
}

/// 截断输出到指定字节数，超限时追加 "(truncated)" 标记。
fn truncate_output(output: &[u8], limit: usize) -> String {
    if output.len() <= limit {
        String::from_utf8_lossy(output).into_owned()
    } else {
        format!(
            "{}\n...(truncated, {} bytes total)",
            String::from_utf8_lossy(&output[..limit]),
            output.len()
        )
    }
}

// ====== 危险命令检测 ======

/// 检测命令是否包含高危操作。
/// 返回 `Some(risk_description)` 表示危险，`None` 表示安全。
pub fn check_dangerous_command(command: &str) -> Option<&'static str> {
    let cmd = command.trim().to_lowercase();

    // rm -rf 危险路径
    if (cmd.contains("rm -rf /") && !cmd.contains("rm -rf /tmp"))
        || cmd.contains("rm -rf /*")
        || cmd.contains("rm -rf ~")
        || cmd.contains("rm -rf ~/")
        || cmd.contains("rm -r /")
        || cmd.contains("rm -rf .") && !cmd.contains("./")
    {
        return Some("危险：递归删除系统关键路径");
    }

    // sudo 提权
    if cmd.starts_with("sudo ") || cmd.contains(" sudo ") || cmd.contains("&& sudo ") || cmd.contains("; sudo ") {
        return Some("危险：尝试提权执行 (sudo)");
    }

    // 直接写入设备文件
    if cmd.contains("> /dev/") || cmd.contains(" >> /dev/") || cmd.contains("dd if=") && cmd.contains("of=/dev/") {
        return Some("危险：直接写入设备文件");
    }

    // fork bomb
    if cmd.contains(":(){ :|:& };:") || cmd.contains(":(){ :|:&};:") {
        return Some("危险：fork bomb 模式");
    }

    // 高危 git 操作
    if cmd.contains("git push --force") || cmd.contains("git push -f") {
        return Some("危险：强制推送 (git push --force)");
    }

    // 系统关机/重启
    if cmd.starts_with("shutdown ") || cmd.starts_with("reboot ") || cmd.starts_with("halt ")
        || cmd == "shutdown" || cmd == "reboot" || cmd == "halt"
        || cmd.contains("shutdown -") || cmd.contains("reboot -")
    {
        return Some("危险：系统关机/重启");
    }

    // Windows 危险操作
    if cfg!(windows) {
        if cmd.contains("del /s /q c:\\") || cmd.contains("del /s /q c:")
            || cmd.contains("rmdir /s /q c:\\") || cmd.contains("rmdir /s /q c:")
            || cmd.starts_with("format c:") || cmd.starts_with("format c:\\")
        {
            return Some("危险：删除/格式化系统盘");
        }
    }

    None
}

// ====== 工具定义 ======

/// 返回 execute_command 的 ToolSpec。
pub fn get_command_tool_spec() -> ToolSpec {
    ToolSpec {
        kind: "function".into(),
        function: ToolFunctionSpec {
            name: "execute_command".into(),
            description: "在项目目录中执行系统命令，返回 stdout/stderr/stdout 和退出码。用于运行构建、测试、git 命令等。".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "要执行的命令。如 'cargo build', 'npm test', 'git status'。避免使用 sudo 或高危操作。"
                    },
                    "description": {
                        "type": "string",
                        "description": "简短描述命令用途，仅用于审计日志。如 '编译 Rust 项目'"
                    },
                    "timeout": {
                        "type": "integer",
                        "description": "超时毫秒，默认 60s，最大 300s（5分钟）。长时间编译适量增大。"
                    }
                },
                "required": ["command"]
            }),
        },
    }
}

// ====== 命令执行 ======

/// 在项目根目录中执行系统命令。
///
/// # 参数
/// - `command` — 要执行的命令字符串
/// - `project_root` — 项目根目录绝对路径（工作目录）
/// - `timeout_ms` — 可选超时毫秒，默认 60s
///
/// # 返回
/// ToolResult，包含 stdout、stderr、exit_code、timed_out 信息。
pub fn execute_command(command: &str, project_root: &str, timeout_ms: Option<u64>) -> ToolResult {
    if command.trim().is_empty() {
        return tool_err("命令为空");
    }

    let timeout = timeout_ms
        .unwrap_or(DEFAULT_TIMEOUT_MS)
        .min(MAX_TIMEOUT_MS);

    // 平台选择 shell
    let (shell, shell_arg) = if cfg!(windows) {
        ("cmd", "/C")
    } else {
        ("sh", "-c")
    };

    let child = match Command::new(shell)
        .arg(shell_arg)
        .arg(command)
        .current_dir(project_root)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return tool_err(&format!("启动命令失败: {e}")),
    };

    // 等待执行（带超时），使用 channel 避免依赖 wait_timeout（未稳定）
    use std::sync::{Arc, Mutex};
    let child_arc = Arc::new(Mutex::new(child));
    let child_clone = Arc::clone(&child_arc);

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut c = child_clone.lock().unwrap();
        let status = c.wait();
        let _ = tx.send(status);
    });

    let (exit_code, timed_out) = match rx.recv_timeout(Duration::from_millis(timeout)) {
        Ok(Ok(status)) => (status.code(), false),
        Ok(Err(_)) => return tool_err("等待命令进程失败"),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            // 超时 → 杀掉进程
            if let Ok(mut c) = child_arc.lock() {
                let _ = c.kill();
                let _ = c.wait();
            }
            (None, true)
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            return tool_err("命令进程意外终止");
        }
    };

    // 读取输出
    let stdout = {
        let mut c = child_arc.lock().unwrap();
        use std::io::Read;
        c.stdout.take().and_then(|mut o| {
            let mut buf = Vec::new();
            std::io::BufReader::new(&mut o).read_to_end(&mut buf).ok()?;
            Some(buf)
        }).unwrap_or_default()
    };

    let stderr = {
        let mut c = child_arc.lock().unwrap();
        use std::io::Read;
        c.stderr.take().and_then(|mut o| {
            let mut buf = Vec::new();
            std::io::BufReader::new(&mut o).read_to_end(&mut buf).ok()?;
            Some(buf)
        }).unwrap_or_default()
    };

    // 构建结果
    let mut result = String::new();
    result.push_str(&format!("$ {command}\n"));

    if timed_out {
        result.push_str(&format!("\n⏱ 命令超时 (>{timeout}ms)，已强制终止\n"));
    }

    if let Some(code) = exit_code {
        result.push_str(&format!("\nexit code: {code}\n"));
    }

    if !stdout.is_empty() {
        result.push_str(&format!("\n[stdout]:\n{}\n", truncate_output(&stdout, MAX_OUTPUT_BYTES)));
    }
    if !stderr.is_empty() {
        result.push_str(&format!("\n[stderr]:\n{}\n", truncate_output(&stderr, MAX_OUTPUT_BYTES)));
    }

    if exit_code.map_or(true, |c| c != 0) || timed_out {
        tool_err(&result)
    } else {
        tool_ok(result)
    }
}
