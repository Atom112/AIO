//! 原子写工具（持久化文件写入的公共实现，HOR-1 的落地基础）。
//!
//! 所有 JSON / 配置类持久化文件统一走 temp + rename：崩溃 / 断电至多留下一个
//! `.tmp` 残留文件，目标文件要么是旧内容要么是新内容，绝不产生截断的半写
//! 状态。`std::fs::rename` 在 Unix 上原子替换、在 Windows 上以
//! `MOVEFILE_REPLACE_EXISTING` 语义替换已存在的目标。
//!
//! 语义对齐既有正确实现（`core/config.rs` 的 save_app_config）：
//! 进程崩溃安全；掉电安全依赖文件系统日志，不做 fsync。

use std::io;
use std::path::{Path, PathBuf};

/// 原子写入：先写同目录隐藏临时文件（`.<name>.tmp`），再 rename 覆盖目标。
///
/// 失败时尽力清理临时文件。同目录保证 rename 不跨卷。
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "目标路径没有父目录"))?;
    std::fs::create_dir_all(parent)?;
    let file_name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "目标路径没有文件名"))?
        .to_string_lossy()
        .into_owned();
    let tmp: PathBuf = path.with_file_name(format!(".{file_name}.tmp"));
    if let Err(e) = std::fs::write(&tmp, bytes) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("aio-fs-util-test-{}-{}", tag, std::process::id()));
        std::fs::create_dir_all(&dir).expect("创建临时目录失败");
        dir.join("target.json")
    }

    #[test]
    fn atomic_write_round_trip_and_overwrite() {
        let path = temp_path("roundtrip");
        atomic_write(&path, b"{\"a\":1}").expect("首次写入失败");
        assert_eq!(std::fs::read(&path).unwrap(), b"{\"a\":1}");
        // 覆盖写：rename 语义必须替换旧内容
        atomic_write(&path, b"{\"a\":2}").expect("覆盖写入失败");
        assert_eq!(std::fs::read(&path).unwrap(), b"{\"a\":2}");
        // 成功后不留临时文件
        let tmp = path.with_file_name(".target.json.tmp");
        assert!(!tmp.exists(), "成功后应清理临时文件");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn atomic_write_cleans_tmp_on_failure() {
        // 指向一个“父目录其实是文件”的路径，让写入必然失败
        let dir = std::env::temp_dir().join(format!("aio-fs-util-file-{}", std::process::id()));
        std::fs::write(&dir, b"i am a file").expect("准备失败路径失败");
        let target = dir.join("child.json");
        assert!(atomic_write(&target, b"x").is_err());
        // 目标不应被创建
        assert!(!target.exists());
        let _ = std::fs::remove_file(&dir);
    }
}
