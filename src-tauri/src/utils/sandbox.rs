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
pub use win_appcontainer::{spawn_in_appcontainer, terminate, wait_with_timeout};

// ====== Linux / macOS command sandbox ======
//
// On Unix, Agent commands have no process isolation by default. This is a best-effort wrapper:
// - Linux: when `bwrap` (bubblewrap) is available, runs with "system read-only + only the
//   project dir writable + private /tmp + separate PID namespace"; the command can use
//   the network and write the project, but cannot write anywhere else on the system.
// - macOS: when `sandbox-exec` is available, uses an SBPL profile configured "only the project dir writable".
// Both are enabled only when the tool exists; otherwise execution falls back to native
// (backstopped by `setpgid` process-group cleanup), so no environment regresses on a missing dependency.

/// Looks up an executable in PATH (Unix only).
#[cfg(unix)]
fn find_in_path(bin: &str) -> bool {
    let path = match std::env::var_os("PATH") {
        Some(p) => p,
        None => return false,
    };
    std::env::split_paths(&path).any(|d| d.join(bin).is_file())
}

/// Builds the Linux bubblewrap wrapper argv (fully replaces the "sh -c cmd" launch argv). Pure function, unit-testable cross-platform.
///
/// Network egress control: `allow_network=false` (default) appends `--unshare-net` —
/// the command runs in an isolated network namespace with no external egress (including localhost);
/// `true` keeps the host network (dependency downloads, API calls, etc., declared explicitly by the model).
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

/// Builds the macOS sandbox-exec wrapper argv (write protection: only the project dir is writable). Pure function, unit-testable cross-platform.
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

/// Returns the replaced launch argv for a command when a sandbox is available (argv[0] is the sandbox binary); otherwise None (run natively).
/// Windows uses the Job Object path and always returns None.
///
/// `allow_network`: false (default) = no network egress inside the sandbox (Linux `--unshare-net` /
/// macOS SBPL `deny network*`); true = keep the host network (explicitly declared by the model, visible in approvals).
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

/// Whether the sandbox is available (fail-closed check):
/// - Linux: bwrap exists in PATH
/// - macOS: sandbox-exec exists in PATH (system-provided)
/// - Windows: always true (Job Object has no external dependency; AppContainer is covered separately)
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

    /// Network egress control: default (allow_network=false) appends --unshare-net.
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

    /// Network egress control: default (allow_network=false) makes the SBPL contain deny network*.
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
