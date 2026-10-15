//! Canonical executable resolution for every subprocess spawn (SEC-06).
//!
//! Threat model: on Windows, `Command::new("git")` goes through `CreateProcess`, whose
//! search order for a bare name consults the **application directory and the current
//! directory BEFORE PATH**. Agent-facing commands run with cwd = the project root — a
//! tree the model can write into via file tools — so a planted `git.exe`/`cmd.exe`
//! there would be silently executed instead of the real binary. On Unix, a PATH
//! containing an empty or `.` entry makes the cwd searched as well. Therefore every
//! bare-name spawn must resolve here first: the lookup walks PATH (+ platform fallback
//! directories) ONLY — the current directory is never consulted.
//!
//! This module is also the single home for the Windows PATHEXT walk previously
//! duplicated in three places (MCP stdio, LSP client, skills import); all callers now
//! share one implementation.

use std::path::{Path, PathBuf};

/// Candidate suffixes tried for a name in each search directory (Windows PATHEXT
/// semantics). EXE is always forced first so a same-named script (`.cmd`/`.bat`) in an
/// earlier PATH directory cannot shadow the real binary; an already-extensioned name
/// (e.g. `npx.cmd`) is tried as-is. Pure — unit-testable cross-platform.
pub(crate) fn windows_candidate_suffixes(name_has_ext: bool, pathext: Option<&str>) -> Vec<String> {
    if name_has_ext {
        return vec![String::new()];
    }
    let pathext = pathext.unwrap_or(".EXE;.CMD;.BAT;.COM");
    let mut suffixes: Vec<String> = Vec::new();
    for raw in pathext.split(';').map(str::trim) {
        let s = raw.to_ascii_lowercase();
        if !s.is_empty() && s != ".exe" && !suffixes.contains(&s) {
            suffixes.push(s);
        }
    }
    // EXE first regardless of the environment-provided order.
    suffixes.insert(0, ".exe".to_string());
    suffixes
}

/// Whether the path points to an executable regular file (permission-aware on Unix).
fn is_executable_file(p: &Path) -> bool {
    let Ok(md) = std::fs::metadata(p) else {
        return false;
    };
    if !md.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        md.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Resolves `file_name` inside `dirs` only (injected for tests). Never consults the
/// current directory — the security property this module exists for.
fn search_in_dirs(file_name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    #[cfg(windows)]
    let suffixes = windows_candidate_suffixes(
        Path::new(file_name).extension().is_some(),
        std::env::var("PATHEXT").ok().as_deref(),
    );
    #[cfg(not(windows))]
    let suffixes = vec![String::new()];
    for dir in dirs {
        for suffix in &suffixes {
            let candidate = dir.join(format!("{file_name}{suffix}"));
            if is_executable_file(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

/// Search directories: PATH entries plus platform fallbacks so system binaries resolve
/// even under the minimal environment a GUI app process inherits. `.` and empty entries
/// from PATH are dropped — they would reintroduce the current-directory search.
fn search_dirs() -> Vec<PathBuf> {
    let sep = if cfg!(windows) { ';' } else { ':' };
    let mut dirs: Vec<PathBuf> = std::env::var("PATH")
        .unwrap_or_default()
        .split(sep)
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != ".")
        .map(PathBuf::from)
        .collect();
    let push_unique = |p: PathBuf, dirs: &mut Vec<PathBuf>| {
        if !dirs.contains(&p) {
            dirs.push(p);
        }
    };
    #[cfg(windows)]
    {
        if let Ok(root) = std::env::var("SystemRoot") {
            push_unique(PathBuf::from(&root).join("System32"), &mut dirs);
        }
    }
    #[cfg(not(windows))]
    {
        for fallback in ["/bin", "/usr/bin", "/usr/local/bin", "/sbin", "/usr/sbin"] {
            push_unique(PathBuf::from(fallback), &mut dirs);
        }
    }
    dirs
}

/// Resolves a bare executable name to an absolute path, searching PATH (+ platform
/// fallbacks) only — the current directory is never consulted.
///
/// Names carrying a directory component (relative or absolute) are user-specified
/// full paths: on Windows an extension-less one (e.g. `C:\nodejs\npx`) gets the
/// PATHEXT-augmentation search **within that directory only**; everything else
/// resolves to `None` and callers keep the name verbatim.
pub fn resolve_executable(name: &str) -> Option<PathBuf> {
    let p = Path::new(name);
    let file_name = p.file_name()?.to_string_lossy().into_owned();
    if file_name.is_empty() {
        return None;
    }
    match p.parent() {
        Some(d) if !d.as_os_str().is_empty() => {
            // User-specified directory: augmentation happens inside that directory
            // only (Windows PATHEXT walk for extension-less names like
            // `C:\nodejs\npx`) — never PATH, never the current directory.
            #[cfg(windows)]
            {
                if p.extension().is_none() {
                    return search_in_dirs(&file_name, &[d.to_path_buf()]);
                }
                None
            }
            #[cfg(not(windows))]
            {
                let _ = d;
                None
            }
        }
        _ => search_in_dirs(&file_name, &search_dirs()),
    }
}

/// Resolution with a caller-friendly fallback: names that cannot be resolved are
/// returned unchanged so the OS can produce its native "not found" error. Used for
/// user-configured commands (MCP servers, LSP servers, skill imports) where the bare
/// name is the user's own choice, never for AIO's own system-binary spawns.
pub fn resolve_or_keep(name: &str) -> String {
    resolve_executable(name)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("aio-exec-resolve-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[cfg(windows)]
    fn write_exec(dir: &Path, name: &str) {
        std::fs::write(dir.join(name), b"stub").unwrap();
    }
    #[cfg(not(windows))]
    fn write_exec(dir: &Path, name: &str) {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join(name);
        std::fs::write(&p, b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// PATHEXT order is normalized: EXE first, duplicates case-folded away.
    #[test]
    fn suffixes_force_exe_first() {
        let sfx = windows_candidate_suffixes(false, Some(".CMD;.EXE;.BAT;.COM;.exe"));
        assert_eq!(sfx[0], ".exe");
        assert_eq!(sfx.iter().filter(|s| *s == ".exe").count(), 1);
        assert!(sfx.contains(&".cmd".to_string()));
        // Already-extensioned names are tried as-is.
        assert_eq!(windows_candidate_suffixes(true, None), vec![String::new()]);
        // Default PATHEXT when the env var is absent.
        assert!(windows_candidate_suffixes(false, None).contains(&".com".to_string()));
    }

    /// The search finds the candidate inside the injected dirs and never anywhere else
    /// (the current directory is not part of `dirs`, by construction).
    #[test]
    fn search_finds_candidate_in_dirs() {
        let d = temp_dir("find");
        let exe = if cfg!(windows) { "git.exe" } else { "git" };
        write_exec(&d, exe);
        assert_eq!(
            search_in_dirs("git", std::slice::from_ref(&d)),
            Some(d.join(exe))
        );
        // Extensioned bare names resolve through the same walk.
        assert_eq!(
            search_in_dirs(exe, std::slice::from_ref(&d)),
            Some(d.join(exe))
        );
        assert_eq!(search_in_dirs("missing", std::slice::from_ref(&d)), None);
    }

    #[cfg(not(windows))]
    #[test]
    fn search_requires_exec_bit() {
        let d = temp_dir("xbit");
        std::fs::write(d.join("noperm"), b"x").unwrap();
        assert_eq!(search_in_dirs("noperm", &[d]), None);
    }

    /// Names carrying an absolute path are out of scope — the caller keeps them
    /// verbatim (user-specified full paths). Relative subdirectory names get the
    /// Windows PATHEXT augmentation inside that directory only (covered by
    /// `pathful_name_gets_windows_ext_augmentation`).
    #[test]
    fn absolute_names_are_out_of_scope() {
        assert!(resolve_executable("C:\\Windows\\System32\\cmd.exe").is_none());
        assert!(resolve_executable("/usr/bin/git").is_none());
        assert!(resolve_executable("").is_none());
    }

    /// Windows-only: an extension-less name with a user-specified directory resolves
    /// via PATHEXT inside that directory; an extensioned one stays verbatim.
    #[cfg(windows)]
    #[test]
    fn pathful_name_gets_windows_ext_augmentation() {
        let d = temp_dir("augment");
        write_exec(&d, "mytool.exe");
        let name = d.join("mytool").to_string_lossy().into_owned();
        assert_eq!(resolve_executable(&name), Some(d.join("mytool.exe")));
        let exted = d.join("mytool.exe").to_string_lossy().into_owned();
        assert_eq!(resolve_executable(&exted), None);
    }

    /// System binaries on this machine resolve (git is a hard dev dependency for the
    /// workflow; cmd/sh always exist on their platforms).
    #[test]
    fn system_binaries_resolve() {
        let shell = if cfg!(windows) { "cmd" } else { "sh" };
        let resolved = resolve_executable(shell).expect("system shell must resolve");
        assert!(resolved.is_absolute());
        assert!(resolved.is_file());
    }
}
