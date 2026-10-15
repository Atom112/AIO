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
        /// Commit the branch was created from — "has changes" checks and the task diff
        /// compare against it, so a subagent that commits itself still counts as work.
        base_commit: String,
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
/// current HEAD, then locks it against concurrent sweeps. Returns the worktree path,
/// its branch, and the base commit the branch started from. Best-effort stale sweep runs
/// first (older unlocked siblings are removed).
pub(crate) fn create_worktree(
    main_root: &str,
    id8: &str,
) -> Result<(PathBuf, String, String), String> {
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
    // Base commit: the worktree's HEAD at creation. `commit_all` and the failure-path
    // preservation check compare the branch against it — a subagent may commit inside the
    // worktree itself, which leaves a clean working tree but real work on the branch.
    let base_commit = run_git(&path_s, &["rev-parse", "HEAD"])
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    Ok((path, branch, base_commit))
}

/// Outcome of collecting a worktree's changes onto its branch.
#[derive(Debug)]
pub(crate) enum CommitOutcome {
    /// Neither a dirty working tree nor commits ahead of the base (nothing to merge).
    NoChanges,
    /// The task's accumulated changes (dirty files committed and/or subagent commits).
    Committed { summary: String },
}

/// Whether the worktree branch has commits beyond `base_commit` (e.g. the subagent ran
/// git_add/git_commit itself — a clean tree with real work on the branch).
pub(crate) fn has_commits(worktree_root: &str, base_commit: &str) -> bool {
    if base_commit.is_empty() {
        return false;
    }
    let range = format!("{base_commit}..HEAD");
    run_git(worktree_root, &["rev-list", "--count", &range])
        .ok()
        .and_then(|out| out.trim().parse::<u32>().ok())
        .map(|n| n > 0)
        .unwrap_or(false)
}

/// Commits all changes in the worktree on its branch (system bookkeeping commit on a
/// throwaway branch; does not go through the model-facing git tool approval).
///
/// "Changes" means: a dirty working tree OR commits the subagent made itself (relative to
/// `base_commit`) — judging by `status --porcelain` alone would misclassify a
/// self-committed task as empty and delete its branch (P0-3).
pub(crate) fn commit_all(
    worktree_root: &str,
    base_commit: &str,
    message: &str,
) -> Result<CommitOutcome, String> {
    let dirty = !run_git(worktree_root, &["status", "--porcelain"])?
        .trim()
        .is_empty();
    let ahead = has_commits(worktree_root, base_commit);
    if !dirty && !ahead {
        return Ok(CommitOutcome::NoChanges);
    }
    if dirty {
        run_git(worktree_root, &["add", "-A"])?;
        // Inline identity: a bookkeeping commit must not depend on the user having
        // configured git user.name/user.email
        run_git(
            worktree_root,
            &[
                "-c",
                "user.name=AIO Agent",
                "-c",
                "user.email=aio-agent@local",
                "commit",
                "-m",
                message,
            ],
        )?;
    }
    // Summary of the whole task's accumulated diff (subagent commits + bookkeeping commit)
    let range = format!("{base_commit}..HEAD");
    let stat = run_git(worktree_root, &["diff", "--stat", &range]).unwrap_or_default();
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

/// Names an in-progress operation in the main checkout (merge / rebase / cherry-pick /
/// revert), if any. Merging into a repo mid-operation would either fail or — far worse —
/// `merge --abort` could reset the USER's half-resolved conflict state (P1-1), so the
/// caller refuses up front when this returns Some.
fn in_progress_op(main_root: &str) -> Option<String> {
    let ref_exists =
        |name: &str| run_git(main_root, &["rev-parse", "-q", "--verify", name]).is_ok();
    if ref_exists("MERGE_HEAD") {
        return Some("a merge".into());
    }
    if ref_exists("CHERRY_PICK_HEAD") {
        return Some("a cherry-pick".into());
    }
    if ref_exists("REVERT_HEAD") {
        return Some("a revert".into());
    }
    for dir in ["rebase-merge", "rebase-apply"] {
        if let Ok(p) = run_git(main_root, &["rev-parse", "--git-path", dir]) {
            let p = p.trim();
            if !p.is_empty() && std::path::Path::new(p).exists() {
                return Some("a rebase".into());
            }
        }
    }
    None
}

/// Merges the worktree branch into the main checkout's current branch. Refused up front
/// while the user has a merge/rebase/cherry-pick in progress (never aborts their state);
/// on a conflict of OUR merge, aborts it and reports the conflicted files (the worktree
/// stays on disk for recovery).
pub(crate) fn merge_branch(main_root: &str, branch: &str) -> Result<MergeOutcome, String> {
    if let Some(op) = in_progress_op(main_root) {
        return Ok(MergeOutcome::Refused {
            reason: ierr!(
                "主检出正在进行{}，暂缓子任务合并以免破坏其状态；请先完成或中止当前操作",
                "The main checkout is in the middle of {}; deferring the subagent merge to protect that state — finish or abort the current operation first",
                op
            ),
        });
    }
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
            // Only treat this as OUR conflicted merge when the pre-check was clean and a
            // MERGE_HEAD now exists (i.e. the merge we just started left conflict state).
            // Without the MERGE_HEAD guard, --diff-filter=U could list the files of a
            // concurrently started USER merge and aborting would destroy their work.
            let ours = run_git(main_root, &["rev-parse", "-q", "--verify", "MERGE_HEAD"]).is_ok();
            if !ours {
                // Refused rather than conflicted (e.g. dirty main working tree overlap)
                return Ok(MergeOutcome::Refused { reason: e });
            }
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

    // Parse `git worktree list --porcelain` exactly: records separated by blank lines,
    // each starting with `worktree <path>` and optionally carrying a `locked` line.
    // Substring matching over the raw output would misfire on prefix paths (P2-2).
    let mut registered: Vec<(PathBuf, bool /*locked*/)> = Vec::new();
    // SEC-17: without the registered list, EVERY old sibling directory would be
    // classified as an unregistered orphan and plain-removed — a transient git
    // failure must abort the sweep, not delete live worktrees as "orphans".
    let out = match run_git(main_root, &["worktree", "list", "--porcelain"]) {
        Ok(out) => out,
        Err(e) => {
            tracing::warn!("[worktree] worktree list 失败，跳过过期清理（防孤儿误删）: {e}");
            return;
        }
    };
    {
        let mut current: Option<PathBuf> = None;
        let mut locked = false;
        for line in out.lines() {
            let line = line.trim();
            if line.is_empty() {
                if let Some(p) = current.take() {
                    registered.push((p, locked));
                }
                locked = false;
                continue;
            }
            if let Some(path) = line.strip_prefix("worktree ") {
                current = Some(PathBuf::from(path));
            } else if line == "locked" || line.starts_with("locked ") {
                locked = true;
            }
        }
        if let Some(p) = current.take() {
            registered.push((p, locked));
        }
    }

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
        match registered.iter().find(|(rp, _)| *rp == p) {
            // A live agent holds a lock on its worktree — never sweep it
            Some((_, true)) => continue,
            Some((_, false)) => {
                let p_s = p.to_string_lossy().to_string();
                let _ = run_git(main_root, &["worktree", "unlock", &p_s]);
                if run_git(main_root, &["worktree", "remove", "--force", &p_s]).is_ok() {
                    let branch = format!("{WORKTREE_BRANCH_PREFIX}{name}");
                    let _ = run_git(main_root, &["branch", "-D", &branch]);
                }
            }
            // Not registered with git anymore: orphan directory, plain removal
            None => {
                let _ = std::fs::remove_dir_all(&p);
            }
        }
        tracing::info!("[worktree] 清理过期工作区: {}", p.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn git_available() -> bool {
        // SEC-06: PATH-only resolution, matching the production run_git path
        match crate::utils::exec_resolve::resolve_executable("git") {
            Some(git) => std::process::Command::new(git)
                .arg("--version")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false),
            None => false,
        }
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

        let (wt, branch, base) =
            create_worktree(&repo_s, "test0001").expect("创建 worktree 应成功");
        let wt_s = wt.to_string_lossy().to_string();
        assert!(!base.is_empty(), "应记录基线提交");
        assert!(wt.join("base.txt").exists(), "worktree 应检出主检出文件");

        // No changes yet
        match commit_all(&wt_s, &base, "task").unwrap() {
            CommitOutcome::NoChanges => {}
            _ => panic!("空 worktree 应返回 NoChanges"),
        }

        // Change + commit
        fs::write(wt.join("new.txt"), "hello\n").unwrap();
        match commit_all(&wt_s, &base, "task: add new").unwrap() {
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

    /// P0-3 regression: a subagent that runs git_add/git_commit inside its worktree
    /// leaves a CLEAN working tree — commit_all must still detect the branch's commits
    /// (vs the creation base), or the branch would be deleted and the work silently lost.
    #[test]
    fn subagent_own_commits_are_detected_and_merged() {
        if !git_available() {
            return;
        }
        let repo = fresh_repo("selfcommit");
        let repo_s = repo.to_string_lossy().to_string();

        let (wt, branch, base) = create_worktree(&repo_s, "selfc001").unwrap();
        let wt_s = wt.to_string_lossy().to_string();

        // Simulate the subagent finishing with its own commit (clean tree afterwards)
        fs::write(wt.join("feature.txt"), "done by subagent\n").unwrap();
        run_git(&wt_s, &["add", "-A"]).unwrap();
        run_git(&wt_s, &["commit", "-m", "subagent work"]).unwrap();
        assert!(
            run_git(&wt_s, &["status", "--porcelain"])
                .unwrap()
                .trim()
                .is_empty(),
            "前置条件：工作树应为干净（改动已在提交里）"
        );

        assert!(has_commits(&wt_s, &base), "分支领先基线必须被识别");
        match commit_all(&wt_s, &base, "task").unwrap() {
            CommitOutcome::Committed { summary } => assert!(!summary.is_empty()),
            _ => panic!("子智能体自己的提交必须算作变更，不得判为 NoChanges"),
        }
        merge_branch(&repo_s, &branch).unwrap().merged_or_panic();
        assert!(
            repo.join("feature.txt").exists(),
            "子智能体的提交成果必须合并回主检出"
        );

        remove_worktree(&repo_s, &wt, &branch);
        let _ = fs::remove_dir_all(&repo);
    }

    /// P1-1 regression: while the user has an unresolved conflict merge in the main
    /// checkout (MERGE_HEAD present), subagent merges must be REFUSED and the user's
    /// merge state must remain untouched (no abort, conflicted files still marked).
    #[test]
    fn merge_refused_and_user_state_kept_during_user_merge() {
        if !git_available() {
            return;
        }
        let repo = fresh_repo("usermerge");
        let repo_s = repo.to_string_lossy().to_string();

        // Create the user's conflicting in-progress merge in the main checkout
        let default_branch = run_git(&repo_s, &["rev-parse", "--abbrev-ref", "HEAD"])
            .unwrap()
            .trim()
            .to_string();
        run_git(&repo_s, &["checkout", "-b", "user-feature"]).unwrap();
        fs::write(repo.join("base.txt"), "user line\n").unwrap();
        run_git(&repo_s, &["add", "-A"]).unwrap();
        run_git(&repo_s, &["commit", "-m", "user change"]).unwrap();
        run_git(&repo_s, &["checkout", &default_branch]).unwrap();
        fs::write(repo.join("base.txt"), "master line\n").unwrap();
        run_git(&repo_s, &["add", "-A"]).unwrap();
        run_git(&repo_s, &["commit", "-m", "master change"]).unwrap();
        let merge_failed = run_git(&repo_s, &["merge", "user-feature"]);
        assert!(merge_failed.is_err(), "前置条件：应产生冲突");
        assert!(
            run_git(&repo_s, &["rev-parse", "-q", "--verify", "MERGE_HEAD"]).is_ok(),
            "前置条件：MERGE_HEAD 应存在"
        );

        // A subagent branch ready to merge
        let (wt, branch, base) = create_worktree(&repo_s, "umrg0001").unwrap();
        let wt_s = wt.to_string_lossy().to_string();
        fs::write(wt.join("other.txt"), "x\n").unwrap();
        commit_all(&wt_s, &base, "task").unwrap();

        match merge_branch(&repo_s, &branch).unwrap() {
            MergeOutcome::Refused { reason } => {
                assert!(
                    reason.contains("merge") || reason.contains("合并"),
                    "{reason}"
                )
            }
            other => panic!("用户合并进行中必须拒绝: {other:?}"),
        }

        // The user's merge state must survive untouched
        assert!(
            run_git(&repo_s, &["rev-parse", "-q", "--verify", "MERGE_HEAD"]).is_ok(),
            "用户的 MERGE_HEAD 必须保留（不得被 abort）"
        );
        let status = run_git(&repo_s, &["status", "--porcelain"]).unwrap();
        assert!(status.contains("UU "), "用户的冲突标记必须保留: {status}");

        // Cleanup: conclude the user merge, then merge the subagent branch normally
        run_git(&repo_s, &["merge", "--abort"]).unwrap();
        merge_branch(&repo_s, &branch).unwrap().merged_or_panic();
        assert!(repo.join("other.txt").exists());
        remove_worktree(&repo_s, &wt, &branch);
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
        let (wt_a, br_a, base_a) = create_worktree(&repo_s, "conf0001").unwrap();
        let (wt_b, br_b, base_b) = create_worktree(&repo_s, "conf0002").unwrap();
        let wt_a_s = wt_a.to_string_lossy().to_string();
        let wt_b_s = wt_b.to_string_lossy().to_string();
        fs::write(wt_a.join("base.txt"), "from A\n").unwrap();
        fs::write(wt_b.join("base.txt"), "from B\n").unwrap();
        commit_all(&wt_a_s, &base_a, "a").unwrap();
        commit_all(&wt_b_s, &base_b, "b").unwrap();

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

    /// P2-4 regression: the bookkeeping commit must succeed even when the user has no
    /// global git identity configured.
    #[test]
    fn commit_all_works_without_user_git_identity() {
        if !git_available() {
            return;
        }
        // Repo with NO user.name/user.email (fresh_repo sets them, so build manually)
        let repo = std::env::temp_dir().join(format!("aio_wt_noid_{}", std::process::id()));
        let _ = fs::remove_dir_all(&repo);
        fs::create_dir_all(&repo).unwrap();
        let repo_s = repo.to_string_lossy().to_string();
        run_git(&repo_s, &["init"]).unwrap();
        fs::write(repo.join("base.txt"), "base\n").unwrap();
        run_git(&repo_s, &["add", "-A"]).unwrap();
        // Seed commit also uses the inline identity
        run_git(
            &repo_s,
            &[
                "-c",
                "user.name=seed",
                "-c",
                "user.email=s@local",
                "commit",
                "-m",
                "init",
            ],
        )
        .unwrap();

        let (wt, branch, base) = create_worktree(&repo_s, "noiden01").unwrap();
        let wt_s = wt.to_string_lossy().to_string();
        fs::write(wt.join("x.txt"), "x\n").unwrap();
        match commit_all(&wt_s, &base, "task") {
            Ok(CommitOutcome::Committed { .. }) => {}
            other => panic!("无用户 git 身份时记账提交不应失败: {other:?}"),
        }
        merge_branch(&repo_s, &branch).unwrap().merged_or_panic();
        assert!(repo.join("x.txt").exists());
        remove_worktree(&repo_s, &wt, &branch);
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
