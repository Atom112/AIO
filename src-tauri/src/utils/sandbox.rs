//! Windows Job Object 沙箱 —— 限制 Agent 命令子进程的能力。
//!
//! 当 Agent 通过 `execute_command` 执行系统命令时，为**每次执行**创建独立的
//! Job Object 并将子进程分配进去（返回守卫），实现以下限制：
//! - 守卫 Drop 时（超时/正常返回/提前放弃）自动清理整棵子进程树（KILL_ON_JOB_CLOSE）
//! - 活跃进程数上限 256（允许真实构建工具链并行，同时兜底 fork bomb）
//! - 禁止子进程访问桌面/创建窗口（UILIMIT_DESKTOP）
//!
//! 注意：不限制文件读写（工具需要）和网络访问（web_tools 有独立防护）。
//!
//! 非 Windows 平台此模块为空实现（no-op 守卫）。

#[cfg(windows)]
mod imp {
    use std::process::Child;

    // Win32 类型
    type Handle = isize;
    type Bool = i32;
    type Dword = u32;

    const FALSE: Bool = 0;
    const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: Dword = 0x00002000;
    const JOB_OBJECT_LIMIT_ACTIVE_PROCESS: Dword = 0x00000008;
    const JOB_OBJECT_UILIMIT_DESKTOP: Dword = 0x00000040;
    const JOB_OBJECT_UILIMIT_HANDLES: Dword = 0x00000001;
    #[allow(non_upper_case_globals)]
    const JobObjectExtendedLimitInformation: Dword = 9;
    #[allow(non_upper_case_globals)]
    const JobObjectBasicUIRestrictions: Dword = 4;

    extern "system" {
        fn CreateJobObjectW(lpJobAttributes: *const std::ffi::c_void, lpName: *const u16)
            -> Handle;

        fn SetInformationJobObject(
            hJob: Handle,
            JobObjectInformationClass: Dword,
            lpJobObjectInformation: *const std::ffi::c_void,
            cbJobObjectInformationLength: Dword,
        ) -> Bool;

        fn AssignProcessToJobObject(hJob: Handle, hProcess: Handle) -> Bool;

        fn CloseHandle(hObject: Handle) -> Bool;

        fn OpenProcess(dwDesiredAccess: Dword, bInheritHandle: Bool, dwProcessId: Dword) -> Handle;
    }

    const PROCESS_SET_QUOTA: Dword = 0x0100;
    const PROCESS_TERMINATE: Dword = 0x0001;

    #[repr(C)]
    struct IoCounters {
        read_operation_count: u64,
        write_operation_count: u64,
        other_operation_count: u64,
        read_transfer_count: u64,
        write_transfer_count: u64,
        other_transfer_count: u64,
    }

    #[repr(C)]
    struct JobObjectExtendedLimitInformation {
        basic_limit_information: JobObjectBasicLimitInformation,
        io_info: IoCounters,
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory_used: usize,
        peak_job_memory_used: usize,
    }

    #[repr(C)]
    struct JobObjectBasicLimitInformation {
        per_process_user_time_limit: u64,
        per_job_user_time_limit: u64,
        limit_flags: Dword,
        minimum_working_set_size: usize,
        maximum_working_set_size: usize,
        active_process_limit: Dword,
        affinity: usize,
        priority_class: Dword,
        scheduling_class: Dword,
    }

    #[repr(C)]
    struct JobObjectBasicUiRestrictions {
        ui_restrictions_class: Dword,
    }

    /// 每次执行独立的 Job Object 守卫：持有唯一句柄，Drop 时关闭句柄，
    /// 触发 KILL_ON_JOB_CLOSE 清理整棵进程树（超时/正常返回统一兜底，
    /// 孙进程不再存活到应用退出——修复旧全局 job 从不关闭、超时只杀
    /// 直接子进程的问题）。
    pub struct JobGuard {
        handle: Handle,
    }

    impl Drop for JobGuard {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.handle);
            }
        }
    }

    /// 创建并配置限制性 Job Object（每次执行独立创建）。
    fn configure_job_object() -> Result<Handle, String> {
        unsafe {
            // 创建未命名的 Job Object
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle == 0 {
                return Err("CreateJobObjectW 失败".to_string());
            }

            // 设置扩展限制：KILL_ON_JOB_CLOSE + 活跃进程限制
            let extended = JobObjectExtendedLimitInformation {
                basic_limit_information: JobObjectBasicLimitInformation {
                    per_process_user_time_limit: 0,
                    per_job_user_time_limit: 0,
                    limit_flags: JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
                        | JOB_OBJECT_LIMIT_ACTIVE_PROCESS,
                    minimum_working_set_size: 0,
                    maximum_working_set_size: 0,
                    // 允许真实工具链运行（cargo build 并行 rustc、npm 工作进程等），
                    // 同时兜底 fork bomb（配合危险命令词法检测）。旧全局 job 上限 2
                    // 会让任何派生 2 个以上子进程的构建工具直接失败，且并发命令共享
                    // 同一个 job 相互挤压。
                    active_process_limit: 256,
                    affinity: 0,
                    priority_class: 0,
                    scheduling_class: 0,
                },
                io_info: IoCounters {
                    read_operation_count: 0,
                    write_operation_count: 0,
                    other_operation_count: 0,
                    read_transfer_count: 0,
                    write_transfer_count: 0,
                    other_transfer_count: 0,
                },
                process_memory_limit: 0,
                job_memory_limit: 0,
                peak_process_memory_used: 0,
                peak_job_memory_used: 0,
            };

            if SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &extended as *const _ as *const _,
                std::mem::size_of::<JobObjectExtendedLimitInformation>() as Dword,
            ) == FALSE
            {
                CloseHandle(handle);
                return Err("SetInformationJobObject (limits) 失败".to_string());
            }

            // 设置 UI 限制：禁止桌面/窗口访问
            let ui_restrictions = JobObjectBasicUiRestrictions {
                ui_restrictions_class: JOB_OBJECT_UILIMIT_DESKTOP | JOB_OBJECT_UILIMIT_HANDLES,
            };
            if SetInformationJobObject(
                handle,
                JobObjectBasicUIRestrictions,
                &ui_restrictions as *const _ as *const _,
                std::mem::size_of::<JobObjectBasicUiRestrictions>() as Dword,
            ) == FALSE
            {
                CloseHandle(handle);
                return Err("SetInformationJobObject (UI) 失败".to_string());
            }

            Ok(handle)
        }
    }

    /// 为单次执行创建独立 Job Object 并分配子进程，返回守卫。
    ///
    /// # Safety
    /// 此函数调用 Win32 FFI。调用方必须确保 `child` 已成功 spawn。
    pub fn create_job_for_child(child: &Child) -> Result<JobGuard, String> {
        create_job_for_pid_impl(child.id())
    }

    /// 按 PID 创建独立 Job Object 并分配（AppContainer 挂起进程入队复用）。
    pub(crate) fn create_job_for_pid_impl(pid: u32) -> Result<JobGuard, String> {
        let job = configure_job_object()?;

        unsafe {
            // 打开进程句柄（需要 SET_QUOTA 和 TERMINATE 权限）
            let process_handle = OpenProcess(
                PROCESS_SET_QUOTA | PROCESS_TERMINATE,
                0, // 不继承句柄
                pid,
            );
            if process_handle == 0 {
                CloseHandle(job);
                return Err(format!("OpenProcess({pid}) 失败"));
            }

            // 分配到 Job Object
            let result = AssignProcessToJobObject(job, process_handle);
            CloseHandle(process_handle);

            if result == FALSE {
                CloseHandle(job);
                return Err(format!("AssignProcessToJobObject({pid}) 失败"));
            }
        }

        Ok(JobGuard { handle: job })
    }
}

/// Windows Job Object 沙箱守卫（每次执行独立创建，见模块头注释）。
#[cfg(windows)]
pub use imp::JobGuard;

/// 非 Windows 平台的 no-op 守卫（调用方无需平台条件编译）。
#[cfg(not(windows))]
pub struct JobGuard;

/// 将子进程分配到**每次执行独立**的 Windows Job Object 沙箱，返回守卫。
///
/// 守卫必须存活到该次执行结束；超时路径可提前 drop 以立即清理整棵进程树。
///
/// 非 Windows 平台：返回 no-op 守卫，始终成功。
#[allow(unused_variables)]
pub fn assign_to_job(child: &std::process::Child) -> Result<JobGuard, String> {
    #[cfg(windows)]
    {
        imp::create_job_for_child(child)
    }
    #[cfg(not(windows))]
    {
        Ok(JobGuard)
    }
}

/// 按 PID 将进程加入每次执行独立的 Job Object（AppContainer 路径使用：
/// 进程以 CREATE_SUSPENDED 创建后、ResumeThread 之前入队，避免竞态逃逸）。
#[cfg(windows)]
pub fn create_job_for_pid(pid: u32) -> Result<JobGuard, String> {
    imp::create_job_for_pid_impl(pid)
}

// ====== Windows AppContainer 沙箱（P3.3）======
//
// OS 级文件写管控 + 默认断网：每项目独立 profile，ACL 仅授权项目根/专用临时
// 目录/配置的工具链目录；进程经 SECURITY_CAPABILITY 属性在 AppContainer 令牌
// 下创建，无 capability = 无网络出口（network=true 注入 internetClient）。
// 与 Job Object 叠加（进程数/生命周期限制 + 整树清理）。详见模块头注释。
#[cfg(windows)]
pub(crate) mod win_appcontainer;
#[cfg(windows)]
pub use win_appcontainer::{spawn_in_appcontainer, terminate, wait_with_timeout};

// ====== Linux / macOS 命令沙箱 ======
//
// 在 Unix 上 Agent 命令默认无进程隔离。这里做 best-effort 包装：
// - Linux：若 `bwrap`(bubblewrap) 可用，以“系统只读 + 仅项目目录可写 + 私有 /tmp +
//   独立 PID 命名空间”的方式执行；命令可正常联网/写项目，无法写到系统其它位置。
// - macOS：若 `sandbox-exec` 可用，以 SBPL 配置“仅项目目录可写”。
// 两者都仅在对应工具存在时启用；不存在则回退原生执行（由 `setpgid` 进程组清理兜底），
// 保证任何环境都不会因缺失依赖而回归。

/// 在 PATH 中查找可执行文件（仅 Unix 使用）。
#[cfg(unix)]
fn find_in_path(bin: &str) -> bool {
    let path = match std::env::var_os("PATH") {
        Some(p) => p,
        None => return false,
    };
    std::env::split_paths(&path).any(|d| d.join(bin).is_file())
}

/// Linux bubblewrap 包装 argv（完整替换“sh -c cmd”的启动 argv）。纯函数，跨平台可单测。
///
/// 网络出口管控（P3.1）：`allow_network=false`（默认）追加 `--unshare-net`——
/// 命令在隔离网络命名空间中运行，无任何外部出口（含 localhost）；
/// `true` 保持宿主网络（下载依赖、API 调用等场景由模型显式声明）。
#[allow(dead_code)]
pub fn build_bwrap_argv(
    project_root: &str,
    shell: &str,
    shell_arg: &str,
    command: &str,
    allow_network: bool,
) -> Option<Vec<String>> {
    let root = std::path::Path::new(project_root)
        .canonicalize()
        .ok()?
        .to_string_lossy()
        .into_owned();
    let mut argv = vec![
        "bwrap".to_string(),
        "--die-with-parent".to_string(),
        "--new-session".to_string(),
        "--ro-bind".to_string(),
        "/".to_string(),
        "/".to_string(),
        "--bind".to_string(),
        root.clone(),
        root,
        "--tmpfs".to_string(),
        "/tmp".to_string(),
        "--dev".to_string(),
        "/dev".to_string(),
        "--proc".to_string(),
        "/proc".to_string(),
        "--unshare-pid".to_string(),
        "--unshare-user".to_string(),
    ];
    if !allow_network {
        argv.push("--unshare-net".to_string());
    }
    argv.push("--".to_string());
    argv.push(shell.to_string());
    argv.push(shell_arg.to_string());
    argv.push(command.to_string());
    Some(argv)
}

/// macOS sandbox-exec 包装 argv（写保护：仅项目目录可写）。纯函数，跨平台可单测。
///
/// 网络出口管控（P3.1）：`allow_network=false`（默认）在 SBPL 中追加
/// `(deny network*)`——阻断全部网络出口；`true` 保持宿主网络。
#[allow(dead_code)]
pub fn build_sandbox_exec_argv(
    project_root: &str,
    shell: &str,
    shell_arg: &str,
    command: &str,
    allow_network: bool,
) -> Option<Vec<String>> {
    let root = std::path::Path::new(project_root)
        .canonicalize()
        .ok()?
        .to_string_lossy()
        .into_owned();
    let network_rule = if allow_network {
        String::new()
    } else {
        "(deny network*)\n".to_string()
    };
    let profile = format!(
        "(version 1)\n(allow default)\n(deny file-write*)\n(allow file-write* (subpath \"{}\"))\n{}",
        root, network_rule
    );
    Some(vec![
        "sandbox-exec".into(),
        "-p".into(),
        profile,
        shell.to_string(),
        shell_arg.to_string(),
        command.to_string(),
    ])
}

/// 若可用则为命令返回替换后的启动 argv（argv[0] 为沙箱二进制）；否则 None（保持原生执行）。
/// Windows 走 Job Object，恒返回 None。
///
/// `allow_network`：false（默认）= 沙箱内无网络出口（Linux `--unshare-net` /
/// macOS SBPL `deny network*`）；true = 保持宿主网络（模型显式声明，审批可见）。
#[allow(unused_variables)]
pub fn sandbox_command_prefix(
    shell: &str,
    shell_arg: &str,
    command: &str,
    project_root: &str,
    allow_network: bool,
) -> Option<Vec<String>> {
    #[cfg(target_os = "linux")]
    {
        if !find_in_path("bwrap") {
            return None;
        }
        build_bwrap_argv(project_root, shell, shell_arg, command, allow_network)
    }
    #[cfg(target_os = "macos")]
    {
        if !find_in_path("sandbox-exec") {
            return None;
        }
        build_sandbox_exec_argv(project_root, shell, shell_arg, command, allow_network)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

/// 沙箱是否可用（P3.2 fail-closed 判定）：
/// - Linux：PATH 中存在 bwrap
/// - macOS：PATH 中存在 sandbox-exec（系统自带）
/// - Windows：恒 true（Job Object 无外部依赖；AppContainer 见 P3.3）
pub fn sandbox_available() -> bool {
    #[cfg(target_os = "linux")]
    {
        find_in_path("bwrap")
    }
    #[cfg(target_os = "macos")]
    {
        find_in_path("sandbox-exec")
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bwrap_argv_shape() {
        let root = std::env::temp_dir();
        let root = root.to_string_lossy().into_owned();
        let argv = build_bwrap_argv(&root, "sh", "-c", "echo hi", true).expect("argl");
        assert_eq!(argv[0], "bwrap");
        assert!(argv.contains(&"--die-with-parent".to_string()));
        assert!(argv.contains(&"--ro-bind".to_string()));
        assert!(argv.contains(&"--unshare-pid".to_string()));
        let n = argv.len();
        assert_eq!(
            &argv[n - 4..],
            &[
                "--".to_string(),
                "sh".to_string(),
                "-c".to_string(),
                "echo hi".to_string()
            ]
        );
    }

    /// 网络出口管控（P3.1）：默认（allow_network=false）追加 --unshare-net。
    #[test]
    fn bwrap_network_isolation() {
        let root = std::env::temp_dir().to_string_lossy().into_owned();
        let no_net = build_bwrap_argv(&root, "sh", "-c", "curl x", false).expect("argv");
        assert!(
            no_net.contains(&"--unshare-net".to_string()),
            "默认应隔离网络: {no_net:?}"
        );
        let with_net = build_bwrap_argv(&root, "sh", "-c", "curl x", true).expect("argv");
        assert!(
            !with_net.contains(&"--unshare-net".to_string()),
            "显式声明网络时不应隔离: {with_net:?}"
        );
    }

    #[test]
    fn sandbox_exec_argv_shape() {
        let root = std::env::temp_dir();
        let root = root.to_string_lossy().into_owned();
        let argv = build_sandbox_exec_argv(&root, "sh", "-c", "echo hi", true).expect("argv");
        assert_eq!(argv[0], "sandbox-exec");
        assert_eq!(argv[1], "-p");
        assert!(argv[2].contains("deny file-write*"));
        assert!(argv[2].contains("(subpath \""));
        let n = argv.len();
        assert_eq!(
            &argv[n - 3..],
            &["sh".to_string(), "-c".to_string(), "echo hi".to_string()]
        );
    }

    /// 网络出口管控（P3.1）：默认（allow_network=false）SBPL 含 deny network*。
    #[test]
    fn sandbox_exec_network_isolation() {
        let root = std::env::temp_dir().to_string_lossy().into_owned();
        let no_net = build_sandbox_exec_argv(&root, "sh", "-c", "curl x", false).expect("argv");
        assert!(
            no_net[2].contains("(deny network*)"),
            "默认应阻断网络: {}",
            no_net[2]
        );
        let with_net = build_sandbox_exec_argv(&root, "sh", "-c", "curl x", true).expect("argv");
        assert!(
            !with_net[2].contains("(deny network*)"),
            "显式声明网络时不应阻断: {}",
            with_net[2]
        );
    }
}
