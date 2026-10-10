//! Per-task git worktree isolation for parallel write-capable subagents.
//!
//! Industry-aligned mechanism (Claude Code `isolation: worktree`, Amp per-thread worktrees,
//! Vibe Kanban per-task workspaces): each awaited write-capable delegated task gets its own
//! linked worktree + branch based on the main checkout's current HEAD, so parallel tasks
//! cannot clobber each other's files. On completion the branch is merged back into the main
//! checkout's current branch (Auto mode directly; Normal mode after a batch approval) and
//! the worktree is removed. Conflicting merges abort cleanly and report the conflicted
//! files so the orchestrating agent can retry serially.
//!
//! Layout: worktrees live in a sibling directory of the project root
//! (`<project>.aio-wt/<id>`), mirroring Amp, so the main checkout's `git status` stays
//! clean and tools are sandboxed to the worktree root by the existing path checks.
//! While a task runs, its worktree holds a `git worktree lock` (like Claude Code) so a
//! concurrent sweep cannot remove it.

use crate::ierr;
use crate::utils::git_tools::run_git;
use std::path::{Path, PathBuf};

/// Directory suffix for the worktree container (sibling of the project root).
pub(crate) const WORKTREE_DIR_SUFFIX: &str = ".aio-wt";

/// Branch prefix for task worktree branches.
pub(crate) const WORKTREE_BRANCH_PREFIX: &str = "aio/sub-";

/// One delegated task's workspace resolution outcome.
#[derive(Clone)]
pub(crate) enum TaskWorkspace {
    /// No isolation: tools operate on the shared project root (per-file locks still apply).
    Shared,
    /// Isolated checkout: tools operate on the worktree; changes land on its branch.
    Worktree {
        /// Absolute worktree path (also the tools' root).
        path: PathBuf,
        /// Branch name checked out in the worktree.
        branch: String,
    },
}

/// Outcome of merging a task worktree branch back into the main checkout.
#[derive(Debug)]
pub(crate) enum MergeOutcome {
    /// Merge committed (or fast-forwarded) into the main checkout's current branch.
    Merged { summary: String },
    /// Merge aborted due to conflicts; the worktree + branch are kept for recovery.
    Conflicted { summary: String, files: Vec<String> },
    /// Merge refused (e.g. dirty main working tree overlapping the changed files).
    Refused { reason: String },
}

/// Whether worktree isolation is usable for this project root: git repo present and the
/// root is a local path (network/UNC paths are rejected like Claude Code's worktrees).
pub(crate) fn worktree_supported(main_root: &str) -> bool {
    if main_root.starts_with("\\\\") {
        return false; // UNC / network path: git worktree metadata over UNC is flaky
    }
    crate::utils::git_tools::is_git_repo(main_root)
}

/// Resolves the worktree container directory: `<project_root>.aio-wt` (sibling of the
/// project root, mirroring Amp's layout so the main checkout stays clean).
pub(crate) fn container_dir(main_root: &str) -> PathBuf {
    let root = Path::new(main_root);
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    root.with_file_name(format!("{name}{WORKTREE_DIR_SUFFIX}"))
}

/// Resolves the per-task worktree path: `<container>/<id8>`.
fn worktree_path(main_root: &str, id8: &str) -> PathBuf {
    container_dir(main_root).join(id8)
}

/// Creates an isolated worktree for task `id8`, branching from the main checkout's
/// current HEAD, then locks it against concurrent sweeps. Best-effort stale sweep runs
/// first (older unlocked siblings are removed).
pub(crate) fn create_worktree(main_root: &str, id8: &str) -> Result<(PathBuf, String), String> {
    sweep_stale(main_root);

    let branch = format!("{WORKTREE_BRANCH_PREFIX}{id8}");
    let path = worktree_path(main_root, id8);
    if path.exists() {
        return Err(ierr!(
            "工作区目录已存在: {}",
            "Worktree directory already exists: {}",
            path.display()
        ));
    }
    // Branch from the current HEAD of the main checkout (like Amp; the caller's in-progress
    // work is the base, so isolated tasks build on what the user sees)
    let path_s = path.to_string_lossy().to_string();
    run_git(main_root, &["worktree", "add", "-b", &branch, &path_s])?;
    run_git(main_root, &["worktree", "lock", &path_s])?;
    Ok((path, branch))
}

/// Outcome of collecting a worktree's changes onto its branch.
#[derive(Debug)]
pub(crate) enum CommitOutcome {
    /// No tracked changes in the worktree (nothing to merge or clean up beyond removal).
    NoChanges,
    /// All changes committed on the worktree branch.
    Committed { summary: String },
}

/// Commits all changes in the worktree on its branch (system bookkeeping commit on a
/// throwaway branch; does not go through the model-facing git tool approval).
pub(crate) fn commit_all(worktree_root: &str, message: &str) -> Result<CommitOutcome, String> {
    let status = run_git(worktree_root, &["status", "--porcelain"])?;
    if status.trim().is_empty() {
        return Ok(CommitOutcome::NoChanges);
    }
    run_git(worktree_root, &["add", "-A"])?;
    run_git(worktree_root, &["commit", "-m", message])?;
    let stat = run_git(worktree_root, &["diff", "--stat", "HEAD~1", "HEAD"]).unwrap_or_default();
    let summary = stat
        .lines()
        .last()
        .map(|l| l.trim().to_string())
        .unwrap_or_default();
    Ok(CommitOutcome::Committed { summary })
}

/// Lists the files the branch changed relative to the main checkout's current HEAD.
pub(crate) fn changed_files(main_root: &str, branch: &str) -> Vec<String> {
    // `HEAD...branch` (three-dot range) must be one argument: changes on the branch
    // since its merge-base with the current HEAD
    let range = format!("HEAD...{branch}");
    run_git(main_root, &["diff", "--name-only", &range])
        .map(|out| {
            out.lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Merges the worktree branch into the main checkout's current branch. On conflict the
/// merge is aborted and the conflicted files are reported (the worktree stays on disk).
pub(crate) fn merge_branch(main_root: &str, branch: &str) -> Result<MergeOutcome, String> {
    match run_git(main_root, &["merge", "--no-edit", branch]) {
        Ok(_) => {
            let stat = run_git(
                main_root,
                &["show", "--stat", "--oneline", "HEAD", "--no-color"],
            )
            .unwrap_or_default();
            Ok(MergeOutcome::Merged {
                summary: stat.lines().take(1).next().unwrap_or_default().to_string(),
            })
        }
        Err(e) => {
            // Collect the conflicted files before aborting
            let files: Vec<String> =
                run_git(main_root, &["diff", "--name-only", "--diff-filter=U"])
                    .map(|out| {
                        out.lines()
                            .map(|l| l.trim().to_string())
                            .filter(|l| !l.is_empty())
                            .collect()
                    })
                    .unwrap_or_default();
            let _ = run_git(main_root, &["merge", "--abort"]);
            if files.is_empty() {
                // Refused rather than conflicted (e.g. dirty main working tree overlap)
                Ok(MergeOutcome::Refused { reason: e })
            } else {
                Ok(MergeOutcome::Conflicted { summary: e, files })
            }
        }
    }
}

/// Removes a task worktree and deletes its branch (unlock first; force removes uncommitted
/// leftovers — call only after the merge outcome has been recorded). Best-effort.
pub(crate) fn remove_worktree(main_root: &str, worktree_path: &Path, branch: &str) {
    let path_s = worktree_path.to_string_lossy().to_string();
    let _ = run_git(main_root, &["worktree", "unlock", &path_s]);
    if run_git(main_root, &["worktree", "remove", "--force", &path_s]).is_err() {
        // Fall back to a plain directory removal (worktree metadata stays but git prunes it lazily)
        let _ = std::fs::remove_dir_all(worktree_path);
        let _ = run_git(main_root, &["worktree", "prune"]);
    }
    let _ = run_git(main_root, &["branch", "-D", branch]);
}

/// Best-effort sweep of stale worktrees: unlocked sibling directories older than
/// `max_age` are removed with their branches (abandoned by crashed runs).
pub(crate) fn sweep_stale(main_root: &str) {
    const MAX_AGE: std::time::Duration = std::time::Duration::from_secs(3 * 24 * 3600);
    let parent = container_dir(main_root);
    let Ok(entries) = std::fs::read_dir(&parent) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if !p.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let Ok(modified) = meta.modified() else {
            continue;
        };
        if modified.elapsed().unwrap_or_default() < MAX_AGE {
            continue;
        }
        // Skip locked worktrees (a live agent may hold them)
        let p_s = p.to_string_lossy().to_string();
        if run_git(main_root, &["worktree", "list", "--porcelain"])
            .map(|out| out.contains(&p_s))
            .unwrap_or(false)
        {
            // Still registered with git: unlock + remove via git (keeps metadata consistent)
            let _ = run_git(main_root, &["worktree", "unlock", &p_s]);
            if run_git(main_root, &["worktree", "remove", "--force", &p_s]).is_ok() {
                let branch = format!("{WORKTREE_BRANCH_PREFIX}{name}");
                let _ = run_git(main_root, &["branch", "-D", &branch]);
            }
        } else {
            let _ = std::fs::remove_dir_all(&p);
        }
        tracing::info!("[worktree] 清理过期工作区: {}", p.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn git_available() -> bool {
        std::process::Command::new("git")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Fresh git repo with one committed file, config set for committing.
    fn fresh_repo(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aio_wt_{tag}_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let dir_s = dir.to_string_lossy().to_string();
        run_git(&dir_s, &["init"]).unwrap();
        run_git(&dir_s, &["config", "user.email", "t@aio.dev"]).unwrap();
        run_git(&dir_s, &["config", "user.name", "AIO Test"]).unwrap();
        fs::write(dir.join("base.txt"), "base\n").unwrap();
        run_git(&dir_s, &["add", "-A"]).unwrap();
        run_git(&dir_s, &["commit", "-m", "init"]).unwrap();
        dir
    }

    #[test]
    fn create_commit_merge_remove_roundtrip() {
        if !git_available() {
            return;
        }
        let repo = fresh_repo("roundtrip");
        let repo_s = repo.to_string_lossy().to_string();
        assert!(worktree_supported(&repo_s));

        let (wt, branch) = create_worktree(&repo_s, "test0001").expect("创建 worktree 应成功");
        let wt_s = wt.to_string_lossy().to_string();
        assert!(wt.join("base.txt").exists(), "worktree 应检出主检出文件");

        // No changes yet
        match commit_all(&wt_s, "task").unwrap() {
            CommitOutcome::NoChanges => {}
            _ => panic!("空 worktree 应返回 NoChanges"),
        }

        // Change + commit
        fs::write(wt.join("new.txt"), "hello\n").unwrap();
        match commit_all(&wt_s, "task: add new").unwrap() {
            CommitOutcome::Committed { summary } => {
                assert!(!summary.is_empty(), "应返回 diff 摘要")
            }
            _ => panic!("有变更时应返回 Committed"),
        }
        assert_eq!(changed_files(&repo_s, &branch), vec!["new.txt".to_string()]);

        // Merge back
        match merge_branch(&repo_s, &branch).unwrap() {
            MergeOutcome::Merged { .. } => {}
            other => panic!("无重叠变更应合并成功: {other:?}"),
        }
        assert!(repo.join("new.txt").exists(), "合并后主检出应包含新文件");

        remove_worktree(&repo_s, &wt, &branch);
        assert!(!wt.exists(), "worktree 应被删除");
        let branches = run_git(&repo_s, &["branch", "--format=%(refname:short)"]).unwrap();
        assert!(!branches.contains(&branch), "分支应被删除: {branches}");
        let _ = fs::remove_dir_all(&repo);
    }

    #[test]
    fn conflicting_merge_aborts_and_reports_files() {
        if !git_available() {
            return;
        }
        let repo = fresh_repo("conflict");
        let repo_s = repo.to_string_lossy().to_string();

        // Two worktrees on the same base, both editing base.txt differently
        let (wt_a, br_a) = create_worktree(&repo_s, "conf0001").unwrap();
        let (wt_b, br_b) = create_worktree(&repo_s, "conf0002").unwrap();
        let wt_a_s = wt_a.to_string_lossy().to_string();
        let wt_b_s = wt_b.to_string_lossy().to_string();
        fs::write(wt_a.join("base.txt"), "from A\n").unwrap();
        fs::write(wt_b.join("base.txt"), "from B\n").unwrap();
        commit_all(&wt_a_s, "a").unwrap();
        commit_all(&wt_b_s, "b").unwrap();

        merge_branch(&repo_s, &br_a).unwrap().merged_or_panic();
        match merge_branch(&repo_s, &br_b).unwrap() {
            MergeOutcome::Conflicted { files, .. } => {
                assert_eq!(files, vec!["base.txt".to_string()], "应报告冲突文件");
            }
            other => panic!("重叠编辑应冲突: {other:?}"),
        }
        // Merge state must be cleaned up (abort); the conflicted worktree stays on disk
        let status = run_git(&repo_s, &["status", "--porcelain"]).unwrap();
        assert!(
            !status.contains("UU ") && !status.contains("AA "),
            "合并状态应已中止: {status}"
        );
        assert!(wt_b.exists(), "冲突的 worktree 应保留待恢复");

        // Cleanup
        remove_worktree(&repo_s, &wt_a, &br_a);
        remove_worktree(&repo_s, &wt_b, &br_b);
        let _ = fs::remove_dir_all(&repo);
    }

    #[test]
    fn non_git_project_is_unsupported() {
        if !git_available() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("aio_wt_nogit_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        assert!(!worktree_supported(&dir.to_string_lossy()));
        let _ = fs::remove_dir_all(&dir);
    }

    /// Test helper: panics when the outcome is not Merged.
    impl super::MergeOutcome {
        fn merged_or_panic(&self) {
            match self {
                MergeOutcome::Merged { .. } => {}
                other => panic!("期望合并成功: {other:?}"),
            }
        }
    }
}
