//! Every git subprocess V2 runs, one function per operation, behaving as V1's
//! `workspaces.go` does. Each function blocks: call it from `tokio::task::spawn_blocking`,
//! never from the core task.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Where a project's slots live. V1 wrote `.baag/worktrees` before the rename and still
/// reads it, so V2 reads it too and creates only under the current name.
pub const WORKTREE_DIR: &str = ".domux/worktrees";
pub const LEGACY_WORKTREE_DIR: &str = ".baag/worktrees";

#[derive(Debug, thiserror::Error)]
#[error("{command} failed: {message}")]
pub struct GitError {
    pub command: String,
    pub message: String,
}

/// Runs git in `dir` and returns its trimmed stdout. stderr and stdout are joined in the
/// error so git's own words reach the engineer (principle 9).
pub fn run(dir: &Path, args: &[&str]) -> Result<String, GitError> {
    let command = format!("git {}", args.join(" "));
    tracing::debug!(dir = %dir.display(), %command, "git");
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|e| GitError {
            command: command.clone(),
            message: e.to_string(),
        })?;
    if !out.status.success() {
        let mut message = String::from_utf8_lossy(&out.stderr).trim().to_string();
        if message.is_empty() {
            message = String::from_utf8_lossy(&out.stdout).trim().to_string();
        }
        return Err(GitError { command, message });
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn is_repo(dir: &Path) -> bool {
    run(dir, &["rev-parse", "--is-inside-work-tree"])
        .map(|s| s == "true")
        .unwrap_or(false)
}

/// The short name `origin/HEAD` points at, falling back to `main` when it is not set
/// (V1's `defaultBaseBranch`). A directory that is not a repository at all answers `main`
/// too, so callers that need to tell the two apart ask `is_repo` first.
pub fn default_branch(root: &Path) -> String {
    match run(
        root,
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
    ) {
        Ok(reference) => {
            let short = reference
                .strip_prefix("origin/")
                .unwrap_or(&reference)
                .trim()
                .to_string();
            if short.is_empty() {
                "main".to_string()
            } else {
                short
            }
        }
        Err(_) => "main".to_string(),
    }
}

/// The ref a slot branches from: `[worktrees] base` when the author set it, else
/// `origin/<default branch>`.
pub fn base_ref(root: &Path, configured: Option<&str>) -> String {
    match configured {
        Some(base) if !base.trim().is_empty() => base.trim().to_string(),
        _ => format!("origin/{}", default_branch(root)),
    }
}

pub fn slot_branch(slot: u32) -> String {
    format!("workspace-{slot}")
}

pub fn slot_path(root: &Path, slot: u32) -> PathBuf {
    root.join(WORKTREE_DIR).join(slot_branch(slot))
}

/// The slot numbers whose directories exist under either worktree directory, sorted. This
/// is V1's `lowestFreeWorkspaceSlot` inverted: the model holds the records, and this tells
/// it what is already on disk (Task 16 adopts them). A worktree directory that is not there
/// is no slots; one that is there and cannot be read is an error, because reporting it as
/// empty would hand out a slot number that is already taken.
pub fn existing_slots(root: &Path) -> Result<Vec<u32>, GitError> {
    let mut slots = Vec::new();
    for dir in [WORKTREE_DIR, LEGACY_WORKTREE_DIR] {
        let dir = root.join(dir);
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                return Err(GitError {
                    command: format!("read {}", dir.display()),
                    message: e.to_string(),
                })
            }
        };
        for entry in entries.flatten() {
            if !entry.path().is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(n) = name
                .strip_prefix("workspace-")
                .and_then(|s| s.parse::<u32>().ok())
            {
                if n >= 1 && !slots.contains(&n) {
                    slots.push(n);
                }
            }
        }
    }
    slots.sort_unstable();
    Ok(slots)
}

pub fn fetch(root: &Path, base: &str) -> Result<(), GitError> {
    let remote = base.split_once('/').map(|(r, _)| r).unwrap_or("origin");
    let branch = base.split_once('/').map(|(_, b)| b).unwrap_or(base);
    run(root, &["fetch", "-q", remote, branch]).map(|_| ())
}

/// `git worktree add` on a fresh branch from `base`. A branch of that name that already
/// exists is force-reset to the base first, which is V1's rule: the slot number is the
/// identity, and a stale branch from a deleted slot must not decide what the new one holds.
/// Prunes registrations for directories removed outside git before it starts.
pub fn worktree_add(root: &Path, path: &Path, branch: &str, base: &str) -> Result<(), GitError> {
    fetch(root, base)?;
    prune(root)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| GitError {
            command: format!("mkdir {}", parent.display()),
            message: e.to_string(),
        })?;
    }
    let exists = run(
        root,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    )
    .is_ok();
    let path = path.to_string_lossy().into_owned();
    if exists {
        run(root, &["branch", "-f", branch, base])?;
        run(root, &["worktree", "add", &path, branch])?;
    } else {
        run(root, &["worktree", "add", "-b", branch, &path, base])?;
    }
    Ok(())
}

/// Removes the worktree and then its branch. A branch that is already gone is not an error:
/// a create that crashed halfway leaves one or the other (V1 tolerates the same). Without
/// `force` git refuses a worktree holding uncommitted or untracked work, and that refusal
/// reaches the caller: this is the call that deletes a directory the author was in.
pub fn worktree_remove(
    root: &Path,
    path: &Path,
    branch: &str,
    force: bool,
) -> Result<(), GitError> {
    let path = path.to_string_lossy().into_owned();
    let mut args = vec!["worktree", "remove"];
    if force {
        args.push("--force");
    }
    args.push(&path);
    run(root, &args)?;
    match run(root, &["branch", "-D", branch]) {
        Ok(_) => Ok(()),
        Err(e) if e.message.contains("not found") => Ok(()),
        Err(e) => Err(e),
    }
}

pub fn prune(root: &Path) -> Result<(), GitError> {
    run(root, &["worktree", "prune"]).map(|_| ())
}

/// The branch checked out at `path`. A detached HEAD reads as `HEAD`, which is git's way of
/// saying there is no branch; a repository before its first commit is an error, not `HEAD`.
pub fn branch_of(path: &Path) -> Result<String, GitError> {
    run(path, &["rev-parse", "--abbrev-ref", "HEAD"])
}

/// V1's `workspaceIsDirty`: uncommitted changes, or commits the upstream does not have.
/// With no upstream it compares against the base's remote branch.
pub fn is_dirty(path: &Path, branch: &str) -> Result<bool, GitError> {
    if !run(path, &["status", "--porcelain"])?.is_empty() {
        return Ok(true);
    }
    let upstream = format!("{branch}@{{u}}");
    let range = if run(path, &["rev-parse", &upstream]).is_ok() {
        format!("{upstream}..{branch}")
    } else {
        format!("origin/{}..{branch}", default_branch(path))
    };
    Ok(!run(path, &["log", "--oneline", &range])?.is_empty())
}

/// Checks the slot's own branch out, fetches, and resets it hard to the base. V1's
/// `resetGitWorkspace`.
pub fn reset_to_base(path: &Path, branch: &str, base: &str) -> Result<(), GitError> {
    run(path, &["checkout", "-q", branch])?;
    fetch(path, base)?;
    run(path, &["reset", "--hard", base]).map(|_| ())
}

/// Removes untracked files and directories. Not `-x`: files git ignores are where the
/// `.domux/worktree.conf` setup puts `.env` and its friends, and clearing a slot must not
/// undo its setup.
pub fn clean(path: &Path) -> Result<(), GitError> {
    run(path, &["clean", "-fd"]).map(|_| ())
}
