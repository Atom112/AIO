//! Windows AppContainer 沙箱（P3.3）—— OS 级文件写管控 + 默认断网。
//!
//! 此前 Windows 仅有 Job Object（进程生命周期/UI 限制，不限制文件与网络），
//! Auto 模式下的 execute_command 实质无沙箱。本模块补齐：
//! - 每项目一个 AppContainer profile（AIO.Agent.<fnv1a(canonical_path)>），
//!   低权限令牌默认拒绝一切，仅按授权放行；
//! - ACL 授权：项目根 RW+X（继承）、专用临时目录 RW；系统目录依赖内置的
//!   ALL APPLICATION PACKAGES ACE；用户工具链目录经配置追加 R+X；
//! - 进程创建：CreateProcessW + PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITY，
//!   无 capability = 默认断网（P3.1 的 Windows 面）；network=true 时注入
//!   internetClient capability（S-1-15-3-1）；
//! - CREATE_SUSPENDED + Job Object + ResumeThread：外层保留进程数/生命周期
//!   限制，超时 Drop 守卫清理整棵进程树；
//! - fail-closed：profile/ACL/进程创建任一失败直接报错，绝不静默回退裸跑
//!   （配置 sandboxWindows=off 可整体关闭）。
//!
//! 注：AppContainer 令牌无法事后附加到已运行进程，必须在进程创建时指定，
//! 因此本模块自带 CreateProcessW 路径（含 stdio 管道与 TEMP 重定向环境块），
//! 与 std::process::Command 的原生路径在 shell_tools 中分流。

#![allow(non_snake_case)]

use crate::ierr;
use std::ffi::c_void;

type Handle = isize;
type Bool = i32;
type Hresult = i32;

const ERROR_SUCCESS: u32 = 0;
const WAIT_TIMEOUT: u32 = 258;
const STILL_ACTIVE: u32 = 259;

// ---- 常量 ----
const SE_FILE_OBJECT: u32 = 1;
const DACL_SECURITY_INFORMATION: u32 = 0x4;
const GRANT_ACCESS: i32 = 1;
const SUB_CONTAINERS_AND_OBJECTS_INHERIT: u32 = 3;
const TRUSTEE_IS_SID: i32 = 0;
const TRUSTEE_IS_UNKNOWN: i32 = 0;
const ACCESS_ALLOWED_ACE_TYPE: u8 = 1;
const ACL_SIZE_INFORMATION: u32 = 2;
// 注意：属性编号以本机 SDK 的 WinBase.h 枚举为准（SecurityCapabilities = 9，
// 9 | PROC_THREAD_ATTRIBUTE_INPUT = 0x20009）。曾误用 21（其他属性），导致
// UpdateProcThreadAttribute 返回 ERROR_BAD_LENGTH。
const PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITY: usize = 0x0002_0009;
const EXTENDED_STARTUPINFO_PRESENT: u32 = 0x0008_0000;
const CREATE_SUSPENDED: u32 = 0x4;
const CREATE_UNICODE_ENVIRONMENT: u32 = 0x400;
const STARTF_USESTDHANDLES: u32 = 0x100;
pub const FILE_GENERIC_READ: u32 = 0x0012_0089;
pub const FILE_GENERIC_WRITE: u32 = 0x0012_0116;
pub const FILE_GENERIC_EXECUTE: u32 = 0x0012_00A0;
/// internetClient capability（S-1-15-3-1）：仅出站 TCP/UDP 客户端。
const INTERNET_CLIENT_SID: &str = "S-1-15-3-1";

// ---- FFI 结构 ----
#[repr(C)]
struct SidAndAttributes {
    sid: *mut c_void,
    attributes: u32,
}

#[repr(C)]
struct SecurityCapabilities {
    app_container_sid: *mut c_void,
    capabilities: *mut SidAndAttributes,
    capability_count: u32,
    reserved: u32,
}

#[repr(C)]
struct SecurityAttributes {
    n_length: u32,
    lp_security_descriptor: *mut c_void,
    b_inherit_handle: Bool,
}

#[repr(C)]
struct StartupInfoW {
    cb: u32,
    lp_reserved: *mut u16,
    lp_desktop: *mut u16,
    lp_title: *mut u16,
    dw_x: u32,
    dw_y: u32,
    dw_x_size: u32,
    dw_y_size: u32,
    dw_x_count_chars: u32,
    dw_y_count_chars: u32,
    dw_fill_attribute: u32,
    dw_flags: u32,
    w_show_window: u16,
    cb_reserved2: u16,
    lp_reserved2: *mut u8,
    h_std_input: Handle,
    h_std_output: Handle,
    h_std_error: Handle,
}

#[repr(C)]
struct StartupInfoExW {
    startup_info: StartupInfoW,
    lp_attribute_list: *mut c_void,
}

#[repr(C)]
struct ProcessInformation {
    h_process: Handle,
    h_thread: Handle,
    dw_process_id: u32,
    dw_thread_id: u32,
}

#[repr(C)]
struct TrusteeW {
    p_multiple_trustee: *mut TrusteeW,
    multiple_trustee_operation: i32,
    trustee_form: i32,
    trustee_type: i32,
    ptstr_name: *mut u16,
}

#[repr(C)]
struct ExplicitAccessW {
    grf_access_permissions: u32,
    grf_access_mode: i32,
    grf_inheritance: u32,
    trustee: TrusteeW,
}

#[repr(C)]
struct AclSizeInformation {
    acl_bytes_in_use: u32,
    acl_bytes_free: u32,
    ace_count: u32,
}

#[repr(C)]
struct AceHeader {
    ace_type: u8,
    ace_flags: u8,
    ace_size: u16,
}

#[repr(C)]
struct AccessAllowedAce {
    header: AceHeader,
    mask: u32,
    sid_start: u32,
}

extern "system" {
    // userenv.dll
    fn CreateAppContainerProfile(
        psz_app_container_name: *const u16,
        psz_display_name: *const u16,
        psz_description: *const u16,
        p_capabilities: *const SidAndAttributes,
        dw_capability_count: u32,
        pp_sid_app_container_sid: *mut *mut c_void,
    ) -> Hresult;
    fn DeriveAppContainerSidFromAppContainerName(
        psz_app_container_name: *const u16,
        ppsid_app_container_sid: *mut *mut c_void,
    ) -> Hresult;

    // kernel32.dll
    fn CreateProcessW(
        lp_application_name: *const u16,
        lp_command_line: *mut u16,
        lp_process_attributes: *const SecurityAttributes,
        lp_thread_attributes: *const SecurityAttributes,
        b_inherit_handles: Bool,
        dw_creation_flags: u32,
        lp_environment: *const c_void,
        lp_current_directory: *const u16,
        lp_startup_info: *const StartupInfoW,
        lp_process_information: *mut ProcessInformation,
    ) -> Bool;
    fn InitializeProcThreadAttributeList(
        lp_attribute_list: *mut c_void,
        dw_attribute_count: u32,
        dw_flags: u32,
        lp_size: *mut usize,
    ) -> Bool;
    fn UpdateProcThreadAttribute(
        lp_attribute_list: *mut c_void,
        dw_flags: u32,
        attribute: usize,
        lp_value: *const c_void,
        cb_size: usize,
        lp_previous_value: *mut c_void,
        lp_return_size: *mut usize,
    ) -> Bool;
    fn DeleteProcThreadAttributeList(lp_attribute_list: *mut c_void);
    fn CreatePipe(
        h_read_pipe: *mut Handle,
        h_write_pipe: *mut Handle,
        lp_pipe_attributes: *const SecurityAttributes,
        n_size: u32,
    ) -> Bool;
    fn WaitForSingleObject(h_handle: Handle, dw_milliseconds: u32) -> u32;
    fn GetExitCodeProcess(h_process: Handle, lp_exit_code: *mut u32) -> Bool;
    fn TerminateProcess(h_process: Handle, u_exit_code: u32) -> Bool;
    fn ResumeThread(h_thread: Handle) -> u32;
    fn CloseHandle(hObject: Handle) -> Bool;
    fn LocalFree(hmem: *mut c_void) -> *mut c_void;
    fn GetLastError() -> u32;

    // advapi32.dll
    fn ConvertStringSidToSidW(string_sid: *const u16, sid: *mut *mut c_void) -> Bool;
    fn GetNamedSecurityInfoW(
        p_object_name: *const u16,
        object_type: u32,
        security_info: u32,
        ppsid_owner: *mut *mut c_void,
        ppsid_group: *mut *mut c_void,
        pp_dacl: *mut *mut c_void,
        pp_sacl: *mut *mut c_void,
        pp_security_descriptor: *mut *mut c_void,
    ) -> u32;
    fn SetNamedSecurityInfoW(
        p_object_name: *const u16,
        object_type: u32,
        security_info: u32,
        psid_owner: *mut c_void,
        psid_group: *mut c_void,
        psid_dacl: *mut c_void,
        psid_sacl: *mut c_void,
    ) -> u32;
    fn SetEntriesInAclW(
        c_count_of_explicit_entries: u32,
        p_list_of_explicit_entries: *const ExplicitAccessW,
        old_acl: *mut c_void,
        new_acl: *mut *mut c_void,
    ) -> u32;
    fn GetAclInformation(
        pacl: *mut c_void,
        pacl_information: *mut c_void,
        n_acl_information_length: u32,
        dw_acl_information_class: u32,
    ) -> Bool;
    fn GetAce(pacl: *mut c_void, dw_ace_index: u32, p_ace: *mut *mut c_void) -> Bool;
    fn EqualSid(p_sid1: *const c_void, p_sid2: *const c_void) -> Bool;
}

// ---- 工具 ----

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// FNV-1a（确定性，跨进程稳定）：profile 名与临时目录名都持久化在用户系统上，
/// 不能用带随机种子的 DefaultHasher（每次启动不同会导致 profile 泄漏）。
fn fnv1a_hex(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// 已完成 ACL 授权的 (profile, 目录) 缓存：同一进程内避免每条命令重复读 DACL。
fn grants_cache() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()))
}

// ---- profile / SID ----

/// 取（必要时创建）项目对应 AppContainer profile 的 SID。
/// profile 已存在时 Create 返回失败 HRESULT，回落到 Derive 取 SID。
/// 返回的 SID 由 LocalFree 释放（调用方负责）。
fn ensure_profile_sid(profile_name: &str) -> Result<*mut c_void, String> {
    let name_w = wide(profile_name);
    let display_w = wide("AIO Agent Sandbox");
    let desc_w = wide("AIO agent command sandbox profile (per-project)");
    let mut sid: *mut c_void = std::ptr::null_mut();
    let hr = unsafe {
        CreateAppContainerProfile(
            name_w.as_ptr(),
            display_w.as_ptr(),
            desc_w.as_ptr(),
            std::ptr::null(),
            0,
            &mut sid,
        )
    };
    tracing::debug!(
        "[appcontainer] CreateAppContainerProfile({profile_name}) hr=0x{hr:08X} sid_null={}",
        sid.is_null()
    );
    if hr == 0 && !sid.is_null() {
        return Ok(sid);
    }
    // 已存在（或创建失败但可派生）：按名派生 SID
    let mut derived: *mut c_void = std::ptr::null_mut();
    let hr2 = unsafe { DeriveAppContainerSidFromAppContainerName(name_w.as_ptr(), &mut derived) };
    if hr2 == 0 && !derived.is_null() {
        return Ok(derived);
    }
    Err(ierr!(
        "AppContainer profile 创建/派生失败: create=0x{:08X} derive=0x{:08X}",
        "Failed to create/derive AppContainer profile: create=0x{:08X} derive=0x{:08X}",
        hr,
        hr2
    ))
}

// ---- ACL ----

/// 判断 DACL 中是否已存在针对指定 SID 的 allow-ACE（幂等去重，防止 ACE 累积）。
fn dacl_has_sid_ace(dacl: *mut c_void, sid: *const c_void) -> bool {
    if dacl.is_null() {
        return false;
    }
    let mut info = AclSizeInformation {
        acl_bytes_in_use: 0,
        acl_bytes_free: 0,
        ace_count: 0,
    };
    if unsafe {
        GetAclInformation(
            dacl,
            &mut info as *mut _ as *mut c_void,
            std::mem::size_of::<AclSizeInformation>() as u32,
            ACL_SIZE_INFORMATION,
        )
    } == 0
    {
        return false;
    }
    for i in 0..info.ace_count {
        let mut ace: *mut c_void = std::ptr::null_mut();
        if unsafe { GetAce(dacl, i, &mut ace) } == 0 || ace.is_null() {
            continue;
        }
        let ace_hdr = unsafe { &*(ace as *const AccessAllowedAce) };
        if ace_hdr.header.ace_type != ACCESS_ALLOWED_ACE_TYPE {
            continue;
        }
        let ace_sid =
            unsafe { &(*ace.cast::<AccessAllowedAce>()).sid_start } as *const u32 as *const c_void;
        if unsafe { EqualSid(ace_sid, sid) } != 0 {
            return true;
        }
    }
    false
}

/// 向目录 DACL 追加对 AppContainer SID 的授权（不覆盖既有 ACE——System32 等
/// 依赖 ALL APPLICATION PACKAGES ACE，覆盖会破坏系统可执行文件的可见性）。
/// 幂等：进程内缓存 + DACL 遍历去重。
fn grant_dir_access(
    path: &str,
    sid: *mut c_void,
    permissions: u32,
    cache_key_suffix: &str,
) -> Result<(), String> {
    let canon = std::path::Path::new(path)
        .canonicalize()
        .map_err(|e| ierr!("路径不存在 {}: {}", "Path does not exist {}: {}", path, e))?;
    let canon_s = canon.to_string_lossy().into_owned();
    let cache_key = format!("{cache_key_suffix}:{}", canon_s);
    {
        let cache = grants_cache().lock().unwrap();
        if cache.contains(&cache_key) {
            return Ok(());
        }
    }
    let path_w = wide(&canon_s);

    // 读现有 DACL
    let mut dacl: *mut c_void = std::ptr::null_mut();
    let mut sd: *mut c_void = std::ptr::null_mut();
    let rc = unsafe {
        GetNamedSecurityInfoW(
            path_w.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut dacl,
            std::ptr::null_mut(),
            &mut sd,
        )
    };
    if rc != ERROR_SUCCESS {
        return Err(ierr!(
            "GetNamedSecurityInfoW({}) 失败: {}",
            "GetNamedSecurityInfoW({}) failed: {}",
            canon_s,
            rc
        ));
    }

    if !dacl_has_sid_ace(dacl, sid) {
        let ea = ExplicitAccessW {
            grf_access_permissions: permissions,
            grf_access_mode: GRANT_ACCESS,
            grf_inheritance: SUB_CONTAINERS_AND_OBJECTS_INHERIT,
            trustee: TrusteeW {
                p_multiple_trustee: std::ptr::null_mut(),
                multiple_trustee_operation: 0,
                trustee_form: TRUSTEE_IS_SID,
                trustee_type: TRUSTEE_IS_UNKNOWN,
                ptstr_name: sid as *mut u16,
            },
        };
        let mut new_dacl: *mut c_void = std::ptr::null_mut();
        let rc2 = unsafe { SetEntriesInAclW(1, &ea, dacl, &mut new_dacl) };
        if rc2 != ERROR_SUCCESS {
            if !sd.is_null() {
                unsafe { LocalFree(sd) };
            }
            return Err(ierr!(
                "SetEntriesInAclW({}) 失败: {}",
                "SetEntriesInAclW({}) failed: {}",
                canon_s,
                rc2
            ));
        }
        let rc3 = unsafe {
            SetNamedSecurityInfoW(
                path_w.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                new_dacl,
                std::ptr::null_mut(),
            )
        };
        if !new_dacl.is_null() {
            unsafe { LocalFree(new_dacl) };
        }
        if rc3 != ERROR_SUCCESS {
            if !sd.is_null() {
                unsafe { LocalFree(sd) };
            }
            return Err(ierr!(
                "SetNamedSecurityInfoW({}) 失败: {}",
                "SetNamedSecurityInfoW({}) failed: {}",
                canon_s,
                rc3
            ));
        }
    }
    if !sd.is_null() {
        unsafe { LocalFree(sd) };
    }
    grants_cache().lock().unwrap().insert(cache_key);
    Ok(())
}

// ---- 环境块 ----

/// 构造 Unicode 环境块：继承父环境，覆盖 TEMP/TMP 指向 AC 专用临时目录
/// （AppContainer 令牌默认无权写用户 %TEMP%，npm/cargo 等依赖临时目录的工具
/// 需要重定向；未重定向则全部报 Permission denied）。
fn build_env_block(ac_temp: &std::path::Path) -> Vec<u16> {
    let mut block: Vec<u16> = Vec::new();
    let temp_s = ac_temp.to_string_lossy().into_owned();
    for (k, v) in std::env::vars_os() {
        let key = k.to_string_lossy().to_ascii_uppercase();
        if key == "TEMP" || key == "TMP" {
            continue; // 用覆盖值替换
        }
        block.extend(k.to_string_lossy().encode_utf16());
        block.push('=' as u16);
        block.extend(v.to_string_lossy().encode_utf16());
        block.push(0);
    }
    for (k, v) in [("TEMP", temp_s.as_str()), ("TMP", temp_s.as_str())] {
        block.extend(k.encode_utf16());
        block.push('=' as u16);
        block.extend(v.encode_utf16());
        block.push(0);
    }
    block.push(0); // 块结尾双 NUL
    block
}

// ---- 进程 ----

/// AppContainer 进程创建结果（shell_tools 消费）。
pub struct AcChild {
    /// 进程句柄（Drop 时关闭）
    pub process: Handle,
    pub pid: u32,
    /// stdout 读取端（File 实现了 Read；take() 接入读取器线程，Option 便于移出所有权）
    pub stdout_read: Option<std::fs::File>,
    pub stderr_read: Option<std::fs::File>,
}

impl Drop for AcChild {
    fn drop(&mut self) {
        if self.process != 0 {
            unsafe { CloseHandle(self.process) };
        }
    }
}

/// 在 AppContainer 中执行命令（完整 shell 命令行，由调用方拼好 shell -c 形态）。
///
/// 参数：
/// - command_line：完整命令行（如 "cmd /C echo hi"）
/// - working_dir：工作目录 = 项目根（须已 canonicalize 可解析）
/// - allow_network：true 时注入 internetClient capability
/// - extra_rx_dirs：额外授予 R+X 的目录（工具链；仅在存在时授予）
///
/// 返回 (AcChild, JobGuard)：守卫 Drop 清理整棵进程树（超时路径提前 drop）。
pub fn spawn_in_appcontainer(
    command_line: &str,
    working_dir: &str,
    allow_network: bool,
    extra_rx_dirs: &[String],
) -> Result<(AcChild, crate::utils::sandbox::JobGuard), String> {
    // 1) profile + SID
    // canonicalize() 在 Windows 返回 \? 前缀扩展路径，CreateProcessW 的
    // lpCurrentDirectory 不接受（ERROR_FILE_NOT_FOUND）——统一剥除。
    let canon_root = std::path::Path::new(working_dir)
        .canonicalize()
        .map_err(|e| {
            ierr!(
                "项目根不可解析 {}: {}",
                "Failed to resolve project root {}: {}",
                working_dir,
                e
            )
        })?;
    let canon_s =
        crate::utils::file_tools::strip_windows_extended_prefix(&canon_root.to_string_lossy());
    let hash = fnv1a_hex(canon_s.to_lowercase().as_bytes());
    let profile_name = format!("AIO.Agent.{hash}");
    let sid = ensure_profile_sid(&profile_name)?;

    // 2) ACL：项目根 RW+X；AC 专用临时目录 RW；工具链目录 R+X
    grant_dir_access(
        &canon_s,
        sid,
        FILE_GENERIC_READ | FILE_GENERIC_WRITE | FILE_GENERIC_EXECUTE,
        &hash,
    )?;
    let ac_temp = std::env::temp_dir().join("aio-ac").join(&hash);
    std::fs::create_dir_all(&ac_temp).map_err(|e| {
        ierr!(
            "创建 AC 临时目录失败: {}",
            "Failed to create the AC temp directory: {}",
            e
        )
    })?;
    grant_dir_access(
        ac_temp.to_str().unwrap_or_default(),
        sid,
        FILE_GENERIC_READ | FILE_GENERIC_WRITE,
        &format!("{hash}-tmp"),
    )?;
    for dir in extra_rx_dirs {
        let p = std::path::Path::new(dir);
        if p.exists() {
            grant_dir_access(
                dir,
                sid,
                FILE_GENERIC_READ | FILE_GENERIC_EXECUTE,
                &format!("{hash}-rx"),
            )?;
        }
    }

    // 3) capability（network=true → internetClient）
    let mut internet_sid: *mut c_void = std::ptr::null_mut();
    let mut caps: Vec<SidAndAttributes> = Vec::new();
    if allow_network {
        let sid_w = wide(INTERNET_CLIENT_SID);
        if unsafe { ConvertStringSidToSidW(sid_w.as_ptr(), &mut internet_sid) } == 0 {
            unsafe { LocalFree(sid) };
            return Err(ierr!(
                "ConvertStringSidToSidW(internetClient) 失败",
                "ConvertStringSidToSidW(internetClient) failed"
            ));
        }
        caps.push(SidAndAttributes {
            sid: internet_sid,
            attributes: 0,
        });
    }

    // 4) stdio 管道（两端可继承；写端交给子进程，读端留在父进程）
    let sa = SecurityAttributes {
        n_length: std::mem::size_of::<SecurityAttributes>() as u32,
        lp_security_descriptor: std::ptr::null_mut(),
        b_inherit_handle: 1,
    };
    let (out_read, mut out_write) = create_pipe(&sa)?;
    let (err_read, mut err_write) = create_pipe(&sa)?;

    // 5) 属性表 + SECURITY_CAPABILITIES
    let sec_caps = SecurityCapabilities {
        app_container_sid: sid,
        capabilities: if caps.is_empty() {
            std::ptr::null_mut()
        } else {
            caps.as_mut_ptr()
        },
        capability_count: caps.len() as u32,
        reserved: 0,
    };
    let mut attr_size: usize = 0;
    unsafe {
        InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut attr_size);
    }
    if attr_size == 0 {
        cleanup_spawn(sid, internet_sid, out_read, out_write, err_read, err_write);
        return Err(ierr!(
            "InitializeProcThreadAttributeList 查询大小失败",
            "InitializeProcThreadAttributeList size query failed"
        ));
    }
    // 属性表缓冲必须指针对齐（Vec<u8> 的 1 字节对齐会导致
    // UpdateProcThreadAttribute 返回 ERROR_BAD_LENGTH —— 实测诊断确认）
    let mut attr_buf: Vec<u64> = vec![0u64; attr_size.div_ceil(8)];
    let attr_list = attr_buf.as_mut_ptr() as *mut c_void;
    if unsafe { InitializeProcThreadAttributeList(attr_list, 1, 0, &mut attr_size) } == 0 {
        cleanup_spawn(sid, internet_sid, out_read, out_write, err_read, err_write);
        return Err(ierr!(
            "InitializeProcThreadAttributeList 失败",
            "InitializeProcThreadAttributeList failed"
        ));
    }
    let attr_ok = unsafe {
        UpdateProcThreadAttribute(
            attr_list,
            0,
            PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITY,
            &sec_caps as *const SecurityCapabilities as *const c_void,
            std::mem::size_of::<SecurityCapabilities>(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if attr_ok == 0 {
        let err = unsafe { GetLastError() };
        unsafe { DeleteProcThreadAttributeList(attr_list) };
        cleanup_spawn(sid, internet_sid, out_read, out_write, err_read, err_write);
        return Err(ierr!(
            "UpdateProcThreadAttribute(SECURITY_CAPABILITY) 失败: Win32 错误 {} (attr_size={}, sid_null={}, caps={})",
            "UpdateProcThreadAttribute(SECURITY_CAPABILITY) failed: Win32 error {} (attr_size={}, sid_null={}, caps={})",
            err,
            attr_size,
            sid.is_null(),
            caps.len()
        ));
    }

    // 6) 环境块（TEMP/TMP 重定向）+ CreateProcessW（挂起启动，先入 Job 再恢复）
    let env_block = build_env_block(&ac_temp);
    let mut cmdw = command_line.encode_utf16().collect::<Vec<u16>>();
    cmdw.push(0);
    let cwd_w = wide(&canon_s);
    let siex = StartupInfoExW {
        startup_info: StartupInfoW {
            cb: std::mem::size_of::<StartupInfoExW>() as u32,
            lp_reserved: std::ptr::null_mut(),
            lp_desktop: std::ptr::null_mut(),
            lp_title: std::ptr::null_mut(),
            dw_x: 0,
            dw_y: 0,
            dw_x_size: 0,
            dw_y_size: 0,
            dw_x_count_chars: 0,
            dw_y_count_chars: 0,
            dw_fill_attribute: 0,
            dw_flags: STARTF_USESTDHANDLES,
            w_show_window: 0,
            cb_reserved2: 0,
            lp_reserved2: std::ptr::null_mut(),
            h_std_input: 0,
            h_std_output: out_write,
            h_std_error: err_write,
        },
        lp_attribute_list: attr_list,
    };
    let mut pi = ProcessInformation {
        h_process: 0,
        h_thread: 0,
        dw_process_id: 0,
        dw_thread_id: 0,
    };
    let created = unsafe {
        CreateProcessW(
            std::ptr::null(),
            cmdw.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1, // 继承句柄（管道写端）
            CREATE_SUSPENDED | EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
            env_block.as_ptr() as *const c_void,
            cwd_w.as_ptr(),
            &siex.startup_info,
            &mut pi,
        )
    };
    unsafe { DeleteProcThreadAttributeList(attr_list) };
    // 父进程侧关闭管道写端（EOF 语义：子进程退出后读端结束）。
    // 置零防止错误路径的 cleanup_spawn 二次关闭（句柄值复用会误关无关句柄）。
    unsafe {
        CloseHandle(out_write);
        CloseHandle(err_write);
    }
    out_write = 0;
    err_write = 0;
    if created == 0 {
        let err = unsafe { GetLastError() };
        cleanup_spawn(sid, internet_sid, out_read, out_write, err_read, err_write);
        return Err(ierr!(
            "CreateProcessW(AppContainer) 失败: Win32 错误 {} (cmdline={}, cwd={})",
            "CreateProcessW(AppContainer) failed: Win32 error {} (cmdline={}, cwd={})",
            err,
            command_line,
            canon_s
        ));
    }

    // 7) Job Object（挂起态先入队，避免竞态派生进程逃逸）→ ResumeThread → 关线程句柄
    let job = crate::utils::sandbox::create_job_for_pid(pi.dw_process_id).map_err(|e| {
        // 入队失败：终止刚创建的进程，fail-closed 报错
        unsafe {
            TerminateProcess(pi.h_process, 1);
            CloseHandle(pi.h_process);
        }
        cleanup_spawn(sid, internet_sid, out_read, out_write, err_read, err_write);
        ierr!(
            "AppContainer 进程无法加入 Job Object: {}",
            "Failed to add the AppContainer process to the Job Object: {}",
            e
        )
    })?;
    if unsafe { ResumeThread(pi.h_thread) } == u32::MAX {
        // 恢复失败：Job 守卫仍在，drop 时清理；此处显式终止更直接
        let _ = unsafe { TerminateProcess(pi.h_process, 1) };
    }
    unsafe { CloseHandle(pi.h_thread) };
    unsafe { LocalFree(sid) };
    if !internet_sid.is_null() {
        unsafe { LocalFree(internet_sid) };
    }
    Ok((
        AcChild {
            process: pi.h_process,
            pid: pi.dw_process_id,
            stdout_read: Some(unsafe {
                std::os::windows::io::FromRawHandle::from_raw_handle(out_read as *mut c_void)
            }),
            stderr_read: Some(unsafe {
                std::os::windows::io::FromRawHandle::from_raw_handle(err_read as *mut c_void)
            }),
        },
        job,
    ))
}

fn create_pipe(sa: &SecurityAttributes) -> Result<(Handle, Handle), String> {
    let mut read: Handle = 0;
    let mut write: Handle = 0;
    if unsafe { CreatePipe(&mut read, &mut write, sa, 0) } == 0 {
        return Err(ierr!("CreatePipe 失败", "CreatePipe failed"));
    }
    Ok((read, write))
}

fn cleanup_spawn(
    sid: *mut c_void,
    internet_sid: *mut c_void,
    r1: Handle,
    w1: Handle,
    r2: Handle,
    w2: Handle,
) {
    unsafe {
        if !sid.is_null() {
            LocalFree(sid);
        }
        if !internet_sid.is_null() {
            LocalFree(internet_sid);
        }
        if r1 != 0 {
            CloseHandle(r1);
        }
        if w1 != 0 {
            CloseHandle(w1);
        }
        if r2 != 0 {
            CloseHandle(r2);
        }
        if w2 != 0 {
            CloseHandle(w2);
        }
    }
}

/// 等待进程退出（毫秒），返回 (退出码, 是否超时)。
pub fn wait_with_timeout(child: &AcChild, timeout_ms: u32) -> (Option<i32>, bool) {
    let r = unsafe { WaitForSingleObject(child.process, timeout_ms) };
    if r == WAIT_TIMEOUT {
        return (None, true);
    }
    let mut code: u32 = 0;
    if unsafe { GetExitCodeProcess(child.process, &mut code) } == 0 || code == STILL_ACTIVE {
        return (None, false);
    }
    (Some(code as i32), false)
}

/// 终止进程（配合 Job 守卫 Drop 清理整棵树）。
pub fn terminate(child: &AcChild) {
    unsafe { TerminateProcess(child.process, 1) };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 在 AppContainer 中跑真实命令并收集输出（本机 Windows 集成验证）。
    fn run_ac(cmd: &str, project: &std::path::Path, network: bool) -> (String, Option<i32>) {
        std::fs::create_dir_all(project).unwrap();
        let cmd_path = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".to_string());
        let (mut child, _job) = spawn_in_appcontainer(
            &format!("{cmd_path} /C {cmd}"),
            project.to_str().unwrap(),
            network,
            &[],
        )
        .expect("AppContainer spawn 失败");
        use std::io::Read;
        let mut out = String::new();
        if let Some(f) = child.stdout_read.take() {
            let mut f = f;
            let _ = f.read_to_string(&mut out);
        }
        let (code, _to) = wait_with_timeout(&child, 30_000);
        (out, code)
    }

    /// 基本执行：cmd echo 在 AC 内正常输出。
    #[test]
    fn appcontainer_basic_exec() {
        let project = std::env::temp_dir().join("aio-ac-test-basic");
        let (out, code) = run_ac("echo aio_ac_ok", &project, false);
        assert_eq!(code, Some(0), "exit code 应为 0, out={out}");
        assert!(out.contains("aio_ac_ok"), "应捕获输出: {out}");
        let _ = std::fs::remove_dir_all(&project);
    }

    /// 文件写越界：AC 令牌对项目外路径无权写入（P3.3 核心安全属性）。
    #[test]
    fn appcontainer_blocks_write_outside_project() {
        let project = std::env::temp_dir().join("aio-ac-test-escape");
        std::fs::create_dir_all(&project).unwrap();
        let escape_target = std::env::temp_dir().join("aio_ac_escape_marker.txt");
        let _ = std::fs::remove_file(&escape_target);
        let (out, code) = run_ac(
            &format!("echo pwned > \"{}\"", escape_target.to_str().unwrap()),
            &project,
            false,
        );
        assert!(
            !escape_target.exists(),
            "项目外写入必须被阻止 (code={code:?}, out={out})"
        );
        let _ = std::fs::remove_dir_all(&project);
    }

    /// 项目内写入正常（授权路径可用，防止过度封锁）。
    #[test]
    fn appcontainer_allows_write_inside_project() {
        let project = std::env::temp_dir().join("aio-ac-test-inside");
        std::fs::create_dir_all(&project).unwrap();
        let (out, code) = run_ac("echo ok > inside_write_test.txt", &project, false);
        assert!(
            project.join("inside_write_test.txt").exists(),
            "项目内写入应成功 (code={code:?}, out={out})"
        );
        let _ = std::fs::remove_dir_all(&project);
    }

    /// 默认断网：无 capability 时外部网络访问被拒（P3.1 Windows 面）。
    #[test]
    fn appcontainer_blocks_network_by_default() {
        let project = std::env::temp_dir().join("aio-ac-test-net");
        let (out, code) = run_ac("ping -n 1 -w 500 127.0.0.1", &project, false);
        assert!(code != Some(0), "无 capability 时 ping 应失败 (out={out})");
        let _ = std::fs::remove_dir_all(&project);
    }
}
