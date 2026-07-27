//! Windows Job Object 沙箱 —— 限制 Agent 命令子进程的能力。
//!
//! 当 Agent 通过 `execute_command` 执行系统命令时，将子进程分配到受限的
//! Job Object，实现以下限制：
//! - 父进程退出时自动清理所有子进程树（KILL_ON_JOB_CLOSE）
//! - 限制子进程创建新的子进程（ACTIVE_PROCESS = 1）
//! - 禁止子进程访问桌面/创建窗口（UILIMIT_DESKTOP）
//!
//! 注意：不限制文件读写（工具需要）和网络访问（web_tools 有独立防护）。
//!
//! 非 Windows 平台此模块为空实现。

#[cfg(windows)]
mod imp {
    use std::process::Child;
    use std::sync::OnceLock;

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

    /// Job Object 句柄和错误状态的懒初始化存储。
    /// 如果创建 Job Object 失败，后续所有 assign 调用都会返回该错误。
    static JOB_STATE: OnceLock<Result<Handle, String>> = OnceLock::new();

    /// 获取全局限制性 Job Object，首次调用时创建。
    fn ensure_job_handle() -> Result<Handle, String> {
        match JOB_STATE.get_or_init(create_job_object) {
            Ok(handle) => Ok(*handle),
            Err(e) => Err(e.clone()),
        }
    }

    /// 创建并配置限制性 Job Object。
    fn create_job_object() -> Result<Handle, String> {
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
                    active_process_limit: 2, // 允许 shell 本身 + 1 个子进程
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

    /// 将已启动的子进程分配到限制性 Job Object。
    ///
    /// # Safety
    /// 此函数调用 Win32 FFI。调用方必须确保 `child` 已成功 spawn。
    pub fn assign_child_to_job(child: &Child) -> Result<(), String> {
        let job = ensure_job_handle()?;
        let pid = child.id();

        unsafe {
            // 打开进程句柄（需要 SET_QUOTA 和 TERMINATE 权限）
            let process_handle = OpenProcess(
                PROCESS_SET_QUOTA | PROCESS_TERMINATE,
                0, // 不继承句柄
                pid,
            );
            if process_handle == 0 {
                return Err(format!("OpenProcess({pid}) 失败"));
            }

            // 分配到 Job Object
            let result = AssignProcessToJobObject(job, process_handle);
            CloseHandle(process_handle);

            if result == FALSE {
                return Err(format!("AssignProcessToJobObject({pid}) 失败"));
            }
        }

        Ok(())
    }
}

/// 将子进程分配到 Windows Job Object 沙箱。
///
/// 限制：KILL_ON_JOB_CLOSE（父进程退出时自动清理）、
/// ACTIVE_PROCESS 限制、禁止桌面/窗口访问。
///
/// 非 Windows 平台：空操作，始终成功。
#[allow(unused_variables)]
pub fn assign_to_job(child: &std::process::Child) -> Result<(), String> {
    #[cfg(windows)]
    {
        imp::assign_child_to_job(child)
    }
    #[cfg(not(windows))]
    {
        Ok(())
    }
}
