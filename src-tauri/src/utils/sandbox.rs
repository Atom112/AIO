//! Windows Job Object sandbox — restricts the capabilities of Agent command subprocesses.
//!
//! When the Agent runs a system command via `execute_command`, an independent Job Object
//! is created **per execution** and the child process assigned into it (returning a guard):
//! - On guard Drop (timeout / normal return / early abandon) the whole child process tree is cleaned up automatically (KILL_ON_JOB_CLOSE)
//! - Active process count capped at 256 (lets real build toolchains run in parallel while still bounding fork bombs)
//! - Child processes are denied desktop access / window creation (UILIMIT_DESKTOP)
//!
//! Note: file reads/writes (needed by tools) and network access (web_tools has its own guard) are not restricted.
//!
//! On non-Windows platforms this module is an empty implementation (no-op guard).

#[cfg(windows)]
mod imp {
    use crate::ierr;
    use std::process::Child;

    // Win32 types
    type Handle = isize;
    type Bool = i32;
    type Dword = u32;

    const FALSE: Bool = 0;
    const TRUE: Bool = 1;
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

        // SEC-8: thread snapshot + resume — used to resume a child that was started
        // with CREATE_SUSPENDED after it has joined its Job Object (assign-then-resume
        // closes the CreateProcess→Assign TOCTOU window in which a fast-spawning child
        // could put grandchildren outside the job).
        fn CreateToolhelp32Snapshot(dwFlags: Dword, th32ProcessID: Dword) -> Handle;

        fn Thread32First(hSnapshot: Handle, lpte: *mut ThreadEntry32) -> Bool;

        fn Thread32Next(hSnapshot: Handle, lpte: *mut ThreadEntry32) -> Bool;

        fn OpenThread(dwDesiredAccess: Dword, bInheritHandle: Bool, dwThreadId: Dword) -> Handle;

        fn ResumeThread(hThread: Handle) -> Dword;
    }

    const PROCESS_SET_QUOTA: Dword = 0x0100;
    const PROCESS_TERMINATE: Dword = 0x0001;
    /// Toolhelp thread-snapshot flag + thread access for ResumeThread.
    const TH32CS_SNAPTHREAD: Dword = 0x0000_0004;
    const THREAD_SUSPEND_RESUME: Dword = 0x0002;
    const INVALID_HANDLE_VALUE: Handle = -1;

    #[repr(C)]
    struct ThreadEntry32 {
        dw_size: Dword,
        cnt_usage: Dword,
        th32_thread_id: Dword,
        th32_owner_process_id: Dword,
        tp_base_pri: i32,
        tp_delta_pri: i32,
        dw_flags: Dword,
    }

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

    /// Per-execution Job Object guard: holds the unique handle; closing it on Drop
    /// triggers KILL_ON_JOB_CLOSE and tears down the whole process tree (uniform
    /// backstop for both timeout and normal return, so grandchild processes
    /// never outlive the execution).
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

    /// Creates and configures a restrictive Job Object (one per execution).
    fn configure_job_object() -> Result<Handle, String> {
        unsafe {
            // Create an unnamed Job Object
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle == 0 {
                return Err(ierr!("CreateJobObjectW 失败", "CreateJobObjectW failed"));
            }

            // Set extended limits: KILL_ON_JOB_CLOSE + active process limit
            let extended = JobObjectExtendedLimitInformation {
                basic_limit_information: JobObjectBasicLimitInformation {
                    per_process_user_time_limit: 0,
                    per_job_user_time_limit: 0,
                    limit_flags: JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
                        | JOB_OBJECT_LIMIT_ACTIVE_PROCESS,
                    minimum_working_set_size: 0,
                    maximum_working_set_size: 0,
                    // Lets real toolchains run (cargo build spawning parallel rustc,
                    // npm worker processes, etc.) while still bounding fork bombs
                    // (together with dangerous-command lexical screening). Per-execution
                    // jobs keep concurrent commands from squeezing each other's process quota.
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
                return Err(ierr!(
                    "SetInformationJobObject (limits) 失败",
                    "SetInformationJobObject (limits) failed"
                ));
            }

            // Set UI restrictions: deny desktop / window access
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
                return Err(ierr!(
                    "SetInformationJobObject (UI) 失败",
                    "SetInformationJobObject (UI) failed"
                ));
            }

            Ok(handle)
        }
    }

    /// Creates an independent Job Object for a single execution, assigns the child process, and returns the guard.
    ///
    /// # Safety
    /// This function calls Win32 FFI. The caller must ensure `child` was spawned successfully.
    pub fn create_job_for_child(child: &Child) -> Result<JobGuard, String> {
        create_job_for_pid_impl(child.id())
    }

    /// Creates an independent Job Object and assigns the process by PID (reused to enqueue AppContainer suspended processes).
    pub(crate) fn create_job_for_pid_impl(pid: u32) -> Result<JobGuard, String> {
        let job = configure_job_object()?;

        unsafe {
            // Open the process handle (needs SET_QUOTA and TERMINATE access rights)
            let process_handle = OpenProcess(
                PROCESS_SET_QUOTA | PROCESS_TERMINATE,
                0, // do not inherit the handle
                pid,
            );
            if process_handle == 0 {
                CloseHandle(job);
                return Err(ierr!("OpenProcess({}) 失败", "OpenProcess({}) failed", pid));
            }

            // Assign to the Job Object
            let result = AssignProcessToJobObject(job, process_handle);
            CloseHandle(process_handle);

            if result == FALSE {
                CloseHandle(job);
                return Err(ierr!(
                    "AssignProcessToJobObject({}) 失败",
                    "AssignProcessToJobObject({}) failed",
                    pid
                ));
            }
        }

        Ok(JobGuard { handle: job })
    }

    /// SEC-8: resumes the primary thread of a child spawned with CREATE_SUSPENDED.
    ///
    /// Called by the native command path after the child has joined its Job Object
    /// (assign-then-resume): without the suspension there is a TOCTOU window in
    /// which a fast-spawning child can put grandchildren outside the job before
    /// `AssignProcessToJobObject` lands. A suspended process has exactly one
    /// (never-run) thread — the snapshot below targets precisely it; if the thread
    /// cannot be found or resumed the child is left frozen, so the caller must
    /// treat an error as fatal for that execution (kill + error, never run-on).
    pub(crate) fn resume_suspended_child_impl(pid: u32) -> Result<(), String> {
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
            if snapshot == INVALID_HANDLE_VALUE || snapshot == 0 {
                return Err(ierr!(
                    "CreateToolhelp32Snapshot 失败 (pid {pid})",
                    "CreateToolhelp32Snapshot failed (pid {pid})"
                ));
            }
            let mut entry = ThreadEntry32 {
                dw_size: std::mem::size_of::<ThreadEntry32>() as Dword,
                cnt_usage: 0,
                th32_thread_id: 0,
                th32_owner_process_id: 0,
                tp_base_pri: 0,
                tp_delta_pri: 0,
                dw_flags: 0,
            };
            let mut thread_handle: Handle = 0;
            if Thread32First(snapshot, &mut entry) == TRUE {
                loop {
                    if entry.th32_owner_process_id == pid {
                        thread_handle = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32_thread_id);
                        if thread_handle != 0 {
                            break;
                        }
                    }
                    if Thread32Next(snapshot, &mut entry) == FALSE {
                        break;
                    }
                }
            }
            CloseHandle(snapshot);
            if thread_handle == 0 {
                return Err(ierr!(
                    "未找到可恢复的子进程线程 (pid {pid})",
                    "no resumable thread found for the child (pid {pid})"
                ));
            }
            let prev = ResumeThread(thread_handle);
            CloseHandle(thread_handle);
            // 0xFFFFFFFF = the thread could not be resumed (invalid handle etc.)
            if prev == Dword::MAX {
                return Err(ierr!(
                    "ResumeThread 失败 (pid {pid})",
                    "ResumeThread failed (pid {pid})"
                ));
            }
        }
        Ok(())
    }
}

/// Windows Job Object sandbox guard (created per execution; see the module header).
#[cfg(windows)]
pub use imp::JobGuard;

/// No-op guard for non-Windows platforms (callers need no platform-specific cfg).
#[cfg(not(windows))]
pub struct JobGuard;

/// Assigns the child process to a Windows Job Object sandbox created **per execution** and returns the guard.
///
/// The guard must stay alive until that execution finishes; the timeout path may drop it early to clean up the whole process tree immediately.
///
/// Non-Windows platforms: returns a no-op guard and always succeeds.
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

/// Adds a process by PID to a per-execution Job Object (used by the AppContainer path:
/// the process is enqueued after CREATE_SUSPENDED creation and before ResumeThread, avoiding race-condition escapes).
#[cfg(windows)]
pub fn create_job_for_pid(pid: u32) -> Result<JobGuard, String> {
    imp::create_job_for_pid_impl(pid)
}

/// SEC-8: resumes the primary thread of a child spawned with CREATE_SUSPENDED, after
/// it has joined its per-execution Job Object (assign-then-resume — see
/// [`resume_suspended_child_impl`]). The caller must kill the child and fail the
/// execution when this returns Err: an unreached resume leaves the child frozen.
#[cfg(windows)]
pub fn resume_suspended_child(pid: u32) -> Result<(), String> {
    imp::resume_suspended_child_impl(pid)
}

// ====== Windows AppContainer sandbox ======
//
// OS-level file-write control + network denied by default: one profile per project,
// ACLs grant only the project root / dedicated temp dir / configured toolchain dirs;
// processes are created under an AppContainer token via the SECURITY_CAPABILITY
// attribute — no capability = no network egress (network=true injects internetClient).
// Layered on top of the Job Object (process count / lifetime limits + whole-tree cleanup). See the module header.
#[cfg(windows)]
pub(crate) mod win_appcontainer;
#[cfg(windows)]
pub use win_appcontainer::{
    add_session_rx_grant, revoke_project_grants, session_rx_grants, spawn_in_appcontainer,
    terminate, wait_with_timeout,
};

// ====== Linux / macOS command sandbox ======
//
// On Unix, Agent commands have no process isolation by default. This is a best-effort wrapper:
// - Linux: when `bwrap` (bubblewrap) is available, runs with "system read-only + only the
//   project dir writable + private /tmp + separate PID namespace + $HOME read boundary
//   (tmpfs over the home dir, only common toolchain subdirs re-mounted read-only)". The
//   command can use the network and write the project, but cannot write anywhere else on
//   the system, and cannot read home-dir secrets (~/.ssh, cloud tokens, etc.).
// - macOS: when `sandbox-exec` is available, uses an SBPL profile configured "only the
//   project dir writable + home reads denied except toolchain subpaths".
// Both are enabled only when the tool exists; otherwise execution falls back to native
// (backstopped by `setpgid` process-group cleanup), so no environment regresses on a missing dependency.

/// $HOME 下需要**只读回绑**的工具链子目录（命令运行需要它们）。
/// 其余 home 内容被 tmpfs 遮蔽、对命令不可见——`~/.ssh`、云凭据、
/// `~/.config` 里其他工具的 token 等一律读不到（SEC-01 读边界）。
/// 注意刻意不包含 `.local`（`share` 下有多种工具的凭据）与整个 `.config`
/// （gh / npm / gcloud 的 token 都在里面），git 配置只放行 `.config/git`。
const HOME_TOOLCHAIN_SUBDIRS: &[&str] = &[
    ".cargo",
    ".rustup",
    ".nvm",
    ".volta",
    ".bun",
    ".deno",
    ".pyenv",
    ".rbenv",
    ".go",
    ".m2",
    ".gradle",
    ".config/git",
];

/// $HOME 下需要只读回绑的单个文件（git 全局配置）。
const HOME_TOOLCHAIN_FILES: &[&str] = &[".gitconfig"];

/// Builds the Linux bubblewrap wrapper argv (fully replaces the "sh -c cmd" launch argv). Pure function, unit-testable cross-platform.
///
/// Network egress control: `allow_network=false` (default) appends `--unshare-net` —
/// the command runs in an isolated network namespace with no external egress (including localhost);
/// `true` keeps the host network (dependency downloads, API calls, etc., declared explicitly by the model).
///
/// Read boundary (SEC-01): after the whole-root read-only bind, `$HOME` is covered by a tmpfs
/// and only [`HOME_TOOLCHAIN_SUBDIRS`] / [`HOME_TOOLCHAIN_FILES`] are re-mounted read-only —
/// the rest of the home directory (credentials) is invisible to the command.
#[allow(dead_code)]
pub fn build_bwrap_argv(
    project_root: &str,
    shell: &str,
    shell_arg: &str,
    command: &str,
    allow_network: bool,
    protect_git_ro: bool,
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
        root.clone(),
    ];
    // Codex 式仓库历史保护：read-only 回绑盖在 RW 根绑定之上（argv 顺序即挂载
    // 叠加顺序），.git 子树对沙箱内命令只读。--ro-bind-try：无 .git 的目录无害跳过。
    if protect_git_ro {
        let git_dir = format!("{root}/.git");
        argv.push("--ro-bind-try".to_string());
        argv.push(git_dir.clone());
        argv.push(git_dir);
    }
    argv.extend([
        "--tmpfs".to_string(),
        "/tmp".to_string(),
        "--dev".to_string(),
        "/dev".to_string(),
        "--proc".to_string(),
        "/proc".to_string(),
        "--unshare-pid".to_string(),
        "--unshare-user".to_string(),
    ]);
    // SEC-01: home read boundary. Ordering matters — bwrap applies mounts in argv
    // order, so the tmpfs shadows the real home bound by --ro-bind / /, and the
    // toolchain re-binds land on top of the tmpfs. (Pure string building — cross-
    // platform testable; the argv itself is only consumed on Linux.)
    if let Some(home) = std::env::var("HOME").ok().filter(|h| !h.is_empty()) {
        argv.push("--tmpfs".to_string());
        argv.push(home.clone());
        for sub in HOME_TOOLCHAIN_SUBDIRS {
            let path = format!("{home}/{sub}");
            argv.push("--ro-bind-try".to_string());
            argv.push(path.clone());
            argv.push(path);
        }
        for file in HOME_TOOLCHAIN_FILES {
            let path = format!("{home}/{file}");
            argv.push("--ro-bind-try".to_string());
            argv.push(path.clone());
            argv.push(path);
        }
    }
    if !allow_network {
        argv.push("--unshare-net".to_string());
    }
    argv.push("--".to_string());
    argv.push(shell.to_string());
    argv.push(shell_arg.to_string());
    argv.push(command.to_string());
    Some(argv)
}

/// Builds the macOS sandbox-exec wrapper argv (write protection: only the project dir is writable;
/// read protection: home denied except toolchain subpaths). Pure function, unit-testable cross-platform.
///
/// Network egress control: `allow_network=false` (default) appends
/// `(deny network*)` to the SBPL — blocking all network egress; `true` keeps the host network.
#[allow(dead_code)]
pub fn build_sandbox_exec_argv(
    project_root: &str,
    shell: &str,
    shell_arg: &str,
    command: &str,
    allow_network: bool,
    protect_git_ro: bool,
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
    // Codex 式仓库历史保护：deny 追加在 allow 之后（SBPL 后匹配者胜），.git 子树
    // 在「项目根可写」内被单独抠回只读。
    let git_rule = if protect_git_ro {
        format!("(deny file-write* (subpath \"{root}/.git\"))\n")
    } else {
        String::new()
    };
    // SEC-01: home read boundary — deny home reads, then allow the toolchain
    // subpaths (more specific SBPL subpath rules take precedence). Pure string
    // building — cross-platform testable; the profile is only consumed on macOS.
    let home_rules = std::env::var("HOME")
        .ok()
        .filter(|h| !h.is_empty())
        .map(|home| {
            let mut rules = format!("(deny file-read* (subpath \"{home}\"))\n");
            for sub in HOME_TOOLCHAIN_SUBDIRS {
                rules.push_str(&format!("(allow file-read* (subpath \"{home}/{sub}\"))\n"));
            }
            for file in HOME_TOOLCHAIN_FILES {
                rules.push_str(&format!("(allow file-read* (subpath \"{home}/{file}\"))\n"));
            }
            rules
        })
        .unwrap_or_default();
    let profile = format!(
        "(version 1)\n(allow default)\n(deny file-write*)\n(allow file-write* (subpath \"{}\"))\n{}{}{}",
        root, git_rule, home_rules, network_rule
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

/// Returns the replaced launch argv for a command when a sandbox is available (argv[0] is the sandbox binary); otherwise None (run natively).
/// Windows uses the Job Object path and always returns None.
///
/// `allow_network`: false (default) = no network egress inside the sandbox (Linux `--unshare-net` /
/// macOS SBPL `deny network*`); true = keep the host network (explicitly declared by the model, visible in approvals).
/// `protect_git_ro`: Codex-style `<root>/.git` deny-write inside the sandbox.
/// Windows 构建下无调用方（bwrap/SBPL 仅 Unix）——保留接口供 Unix 目标。
#[allow(unused_variables)]
#[cfg_attr(windows, allow(dead_code))]
pub fn sandbox_command_prefix(
    shell: &str,
    shell_arg: &str,
    command: &str,
    project_root: &str,
    allow_network: bool,
    protect_git_ro: bool,
) -> Option<Vec<String>> {
    #[cfg(target_os = "linux")]
    {
        // SEC-06: resolve the sandbox wrapper PATH-only — spawning a bare `bwrap`
        // would consult untrusted lookup rules; the resolved absolute path is the
        // binary that the existence check validated.
        let Some(bwrap) = crate::utils::exec_resolve::resolve_executable("bwrap") else {
            return None;
        };
        let mut argv = build_bwrap_argv(
            project_root,
            shell,
            shell_arg,
            command,
            allow_network,
            protect_git_ro,
        )?;
        argv[0] = bwrap.to_string_lossy().into_owned();
        Some(argv)
    }
    #[cfg(target_os = "macos")]
    {
        // SEC-06: same PATH-only resolution for the macOS wrapper.
        let Some(sandbox_exec) = crate::utils::exec_resolve::resolve_executable("sandbox-exec")
        else {
            return None;
        };
        let mut argv = build_sandbox_exec_argv(
            project_root,
            shell,
            shell_arg,
            command,
            allow_network,
            protect_git_ro,
        )?;
        argv[0] = sandbox_exec.to_string_lossy().into_owned();
        Some(argv)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

/// Whether the sandbox is available (fail-closed check):
/// - Linux: bwrap resolvable via PATH (SEC-06: same PATH-only lookup the launcher uses)
/// - macOS: sandbox-exec resolvable via PATH (system-provided)
/// - Windows: always true (Job Object has no external dependency; AppContainer is covered separately)
pub fn sandbox_available() -> bool {
    #[cfg(target_os = "linux")]
    {
        crate::utils::exec_resolve::resolve_executable("bwrap").is_some()
    }
    #[cfg(target_os = "macos")]
    {
        crate::utils::exec_resolve::resolve_executable("sandbox-exec").is_some()
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
        let argv = build_bwrap_argv(&root, "sh", "-c", "echo hi", true, false).expect("argl");
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

    /// Network egress control: default (allow_network=false) appends --unshare-net.
    #[test]
    fn bwrap_network_isolation() {
        let root = std::env::temp_dir().to_string_lossy().into_owned();
        let no_net = build_bwrap_argv(&root, "sh", "-c", "curl x", false, false).expect("argv");
        assert!(
            no_net.contains(&"--unshare-net".to_string()),
            "默认应隔离网络: {no_net:?}"
        );
        let with_net = build_bwrap_argv(&root, "sh", "-c", "curl x", true, false).expect("argv");
        assert!(
            !with_net.contains(&"--unshare-net".to_string()),
            "显式声明网络时不应隔离: {with_net:?}"
        );
    }

    /// SEC-01: home is shadowed by a tmpfs and only toolchain subdirs/files are
    /// re-mounted read-only; credential locations (.ssh / .aws / whole .config)
    /// stay invisible to the command.
    #[test]
    fn bwrap_home_read_boundary() {
        let home = match std::env::var("HOME") {
            Ok(h) if !h.is_empty() => h,
            // Windows CI without HOME: the boundary block is skipped by design
            _ => return,
        };
        let root = std::env::temp_dir().to_string_lossy().into_owned();
        let argv = build_bwrap_argv(&root, "sh", "-c", "ls ~", true, false).expect("argv");
        let joined = argv.join(" ");
        assert!(
            joined.contains(&format!("--tmpfs {home}")),
            "home 应被 tmpfs 遮蔽: {joined}"
        );
        assert!(joined.contains(&format!("--ro-bind-try {home}/.cargo {home}/.cargo")));
        assert!(joined.contains(&format!(
            "--ro-bind-try {home}/.gitconfig {home}/.gitconfig"
        )));
        for secret in [".ssh", ".aws", ".gnupg"] {
            assert!(
                !joined.contains(secret),
                "不得回绑凭据目录 {secret}: {joined}"
            );
        }
    }

    /// SEC-01 (macOS): the SBPL denies home reads and re-allows toolchain subpaths only.
    #[test]
    fn sandbox_exec_home_read_boundary() {
        let home = match std::env::var("HOME") {
            Ok(h) if !h.is_empty() => h,
            _ => return,
        };
        let root = std::env::temp_dir().to_string_lossy().into_owned();
        let argv = build_sandbox_exec_argv(&root, "sh", "-c", "ls ~", true, false).expect("argv");
        let profile = &argv[2];
        assert!(profile.contains(&format!("(deny file-read* (subpath \"{home}\"))")));
        assert!(profile.contains(&format!("(allow file-read* (subpath \"{home}/.cargo\"))")));
        assert!(!profile.contains(".ssh"));
    }

    #[test]
    fn sandbox_exec_argv_shape() {
        let root = std::env::temp_dir();
        let root = root.to_string_lossy().into_owned();
        let argv =
            build_sandbox_exec_argv(&root, "sh", "-c", "echo hi", true, false).expect("argv");
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

    /// Network egress control: default (allow_network=false) makes the SBPL contain deny network*.
    #[test]
    fn sandbox_exec_network_isolation() {
        let root = std::env::temp_dir().to_string_lossy().into_owned();
        let no_net =
            build_sandbox_exec_argv(&root, "sh", "-c", "curl x", false, false).expect("argv");
        assert!(
            no_net[2].contains("(deny network*)"),
            "默认应阻断网络: {}",
            no_net[2]
        );
        let with_net =
            build_sandbox_exec_argv(&root, "sh", "-c", "curl x", true, false).expect("argv");
        assert!(
            !with_net[2].contains("(deny network*)"),
            "显式声明网络时不应阻断: {}",
            with_net[2]
        );
    }

    /// Codex 式仓库历史保护（Linux）：.git 以 read-only 回绑盖在 RW 根绑定之上；
    /// 关闭保护时不出现该绑定。
    #[test]
    fn bwrap_git_ro_protection() {
        let root = std::env::temp_dir().to_string_lossy().into_owned();
        // 构建器内部 canonicalize（Windows 产出 \\?\ 前缀路径），期望值须同源
        let root = std::path::Path::new(&root)
            .canonicalize()
            .expect("canon")
            .to_string_lossy()
            .into_owned();
        let protected = build_bwrap_argv(&root, "sh", "-c", "ls", true, true).expect("argv");
        let joined = protected.join(" ");
        let expected = format!("--ro-bind-try {root}/.git {root}/.git");
        assert!(joined.contains(&expected), "应有 .git 只读回绑: {joined}");
        // 叠加顺序：RW 根绑定在前、.git 只读回绑在后（argv 顺序即挂载叠加顺序）
        let rw_pos = joined
            .find(&format!("--bind {root} {root}"))
            .expect("rw bind");
        let ro_pos = joined.find(&expected).expect("ro-bind");
        assert!(ro_pos > rw_pos, "只读回绑必须盖在 RW 绑定之后: {joined}");
        let unprotected = build_bwrap_argv(&root, "sh", "-c", "ls", true, false).expect("argv");
        assert!(
            !unprotected.join(" ").contains(&expected),
            "关闭保护时不应有 .git 回绑"
        );
    }

    /// Codex 式仓库历史保护（macOS）：deny file-write* 追加在项目根 allow 之后
    /// （SBPL 后匹配者胜），关闭保护时不含该规则。
    #[test]
    fn sandbox_exec_git_ro_protection() {
        let root = std::env::temp_dir().to_string_lossy().into_owned();
        let root = std::path::Path::new(&root)
            .canonicalize()
            .expect("canon")
            .to_string_lossy()
            .into_owned();
        let protected = build_sandbox_exec_argv(&root, "sh", "-c", "ls", true, true).expect("argv");
        let expected = format!("(deny file-write* (subpath \"{root}/.git\"))");
        assert!(
            protected[2].contains(&expected),
            "SBPL 应含 .git 拒写规则: {}",
            protected[2]
        );
        let allow_pos = protected[2]
            .find(&format!("(allow file-write* (subpath \"{root}\"))"))
            .expect("root allow");
        let deny_pos = protected[2].find(&expected).expect("git deny");
        assert!(deny_pos > allow_pos, "deny 必须在 allow 之后（后匹配者胜）");
        let unprotected =
            build_sandbox_exec_argv(&root, "sh", "-c", "ls", true, false).expect("argv");
        assert!(
            !unprotected[2].contains(&expected),
            "关闭保护时不应有 .git 拒写规则"
        );
    }
}
