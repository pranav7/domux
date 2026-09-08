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

/// `git -C ""` is a documented no-op: git runs wherever the server was started, which is
/// somebody's checkout, and a relative path resolves against the same place. Every directory
/// this file acts on is refused unless it is absolute, so `clean -fd` and `reset --hard`
/// cannot land somewhere nobody named.
fn not_absolute(command: String, dir: &Path) -> GitError {
    GitError {
        command,
        message: format!(
            "the directory \"{}\" is not an absolute path; pass the project root",
            dir.display()
        ),
    }
}

/// git reads a positional argument that begins with `-` as an option, and an option like
/// `--upload-pack` names a command for git to run. Every base and branch this file hands to
/// git positionally goes through here first. Refuse the name rather than escape it: a ref
/// whose name starts with `-` is not one git itself would create.
fn refuse_option_like(
    command: &str,
    kind: &str,
    value: &str,
    advice: &str,
) -> Result<(), GitError> {
    if !value.starts_with('-') {
        return Ok(());
    }
    Err(GitError {
        command: command.to_string(),
        message: format!(
            "the {kind} \"{value}\" starts with \"-\", which git reads as an option rather than \
             a name; {advice}"
        ),
    })
}

/// The base comes from `[worktrees] base` in the author's configuration, so that is what a
/// refusal sends them to. A slot branch comes from `slot_branch`, so a bad one is a caller's
/// mistake rather than a setting.
const BASE_ADVICE: &str = "set [worktrees] base to a branch or ref name";
const BRANCH_ADVICE: &str = "a slot branch is named workspace-<number>";

/// Runs git in `dir` and returns its trimmed stdout. stderr and stdout are joined in the
/// error so git's own words reach the engineer (principle 9). Refuses a `dir` that is not
/// absolute, which includes the empty path: the check lives here, at the one place every
/// command goes through, so an operation added later cannot miss it.
pub fn run(dir: &Path, args: &[&str]) -> Result<String, GitError> {
    let command = format!("git {}", args.join(" "));
    if !dir.is_absolute() {
        return Err(not_absolute(command, dir));
    }
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
/// is no slots; one that is there and cannot be read is an error. Answering "no slots" for a
/// directory that is there hands out a slot number that is already taken, and the cost is not
/// the clean refusal it looks like: with the directory present but its registration gone,
/// `git branch -f` succeeds and moves the branch off the author's commit before
/// `git worktree add` finds the directory in the way, leaving that commit in the reflog and
/// nowhere else. `worktree_add` refuses an occupied directory before it touches the branch,
/// so this is the second lock on that door rather than the only one.
pub fn existing_slots(root: &Path) -> Result<Vec<u32>, GitError> {
    if !root.is_absolute() {
        return Err(not_absolute(
            format!("read {}", root.join(WORKTREE_DIR).display()),
            root,
        ));
    }
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

/// Fetches the base's branch from its remote. Guarding the two halves covers the whole base:
/// a base beginning with `-` puts it in the remote when it has a slash and in the branch when
/// it does not. `--` is a second belt, verified against git 2.50: `git fetch -- <remote>
/// <refspec>` works and git blocks a strange pathname after it on its own. It is only a belt
/// here, because `git reset --hard --`, `git checkout --` and `git log --` all mean "paths
/// follow", so the separator cannot be added to those and the refusal is what covers them.
pub fn fetch(root: &Path, base: &str) -> Result<(), GitError> {
    let remote = base.split_once('/').map(|(r, _)| r).unwrap_or("origin");
    let branch = base.split_once('/').map(|(_, b)| b).unwrap_or(base);
    refuse_option_like("git fetch", "remote", remote, BASE_ADVICE)?;
    refuse_option_like("git fetch", "branch", branch, BASE_ADVICE)?;
    run(root, &["fetch", "-q", "--", remote, branch]).map(|_| ())
}

/// Anything at `path` that `git worktree add` would refuse: a file, or a directory with
/// something in it. A directory it cannot read counts as occupied, because the alternative is
/// to guess that it is empty.
fn is_occupied(path: &Path) -> bool {
    match std::fs::read_dir(path) {
        Ok(mut entries) => entries.next().is_some(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => true,
    }
}

/// `git worktree add` on a fresh branch from `base`. A branch of that name that already
/// exists is force-reset to the base first, which is V1's rule: the slot number is the
/// identity, and a stale branch from a deleted slot must not decide what the new one holds.
/// Prunes registrations for directories removed outside git before it starts.
///
/// `--no-track`, for two reasons that point the same way. A slot branch that tracks
/// `origin/main` is a branch whose `git push` either goes to `main` or is refused for a name
/// that does not match, and neither is what a workspace wants. And setting the upstream is a
/// write to `.git/config`, which git guards with a lock file: two `workspace.create` calls on
/// one project at the same time then have one of them fail with "could not lock config file",
/// after it has already made the branch, so the slot cannot be built and a stray branch is
/// left behind. Measured, on git 2.50: ten pairs of concurrent adds with `--no-track` all
/// worked and three of three without it failed that way. The branch that resets an existing
/// branch never set an upstream either, so this also makes the two halves agree.
pub fn worktree_add(root: &Path, path: &Path, branch: &str, base: &str) -> Result<(), GitError> {
    if !path.is_absolute() {
        return Err(not_absolute("git worktree add".to_string(), path));
    }
    refuse_option_like("git worktree add", "branch", branch, BRANCH_ADVICE)?;
    refuse_option_like("git worktree add", "base", base, BASE_ADVICE)?;
    // git refuses an occupied path too, but only after `git branch -f` has moved the branch
    // off the commit it held, which leaves that commit reachable from the reflog and nowhere
    // else. Ask first. An empty directory is not occupied: git accepts one, and refusing it
    // would turn a leftover directory into a slot nobody can create.
    if is_occupied(path) {
        return Err(GitError {
            command: format!("git worktree add {}", path.display()),
            message: "the slot directory is already there; remove it or pick another slot"
                .to_string(),
        });
    }
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
        run(
            root,
            &["worktree", "add", "--no-track", "-b", branch, &path, base],
        )?;
    }
    Ok(())
}

/// Removes the worktree and then its branch. A branch that is already gone is not an error:
/// a create that crashed halfway leaves one or the other (V1 tolerates the same). Without
/// `force` git refuses a worktree holding modified or untracked files, and that refusal
/// reaches the caller: this is the call that deletes a directory the author was in.
///
/// git's refusal does not cover committed work that was never pushed. A slot holding a week
/// of commits and nothing uncommitted is removed with `force` false, and the branch goes with
/// it. `is_dirty` in front of this call is the only guard against that.
pub fn worktree_remove(
    root: &Path,
    path: &Path,
    branch: &str,
    force: bool,
) -> Result<(), GitError> {
    if !path.is_absolute() {
        return Err(not_absolute("git worktree remove".to_string(), path));
    }
    // `git worktree remove` takes the path, not the branch. The branch reaches `git branch -D`
    // and nothing else, so naming this one after the function would name a command that never
    // sees the value.
    refuse_option_like("git branch -D", "branch", branch, BRANCH_ADVICE)?;
    let path = path.to_string_lossy().into_owned();
    let mut args = vec!["worktree", "remove"];
    if force {
        args.push("--force");
    }
    args.push(&path);
    run(root, &args)?;
    // Ask git whether the branch is there rather than reading the words of a `branch -D`
    // failure. git translates those, so a machine in another locale would report an error for
    // a slot it removed correctly, and matching on prose also swallows failures that happen to
    // contain the same phrase.
    if run(
        root,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    )
    .is_ok()
    {
        run(root, &["branch", "-D", branch])?;
    }
    Ok(())
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
///
/// With no upstream it compares against `origin/<default branch>`, which is **not** the base
/// the slot was made from, and since `worktree_add` is `--no-track` that is the path every slot
/// takes. With `[worktrees] base` set to anything else, a slot is born with commits that range
/// holds and reads dirty from the moment it is created. Decision record 0007 has the
/// measurement and the remedy: give this function the base rather than letting it guess.
pub fn is_dirty(path: &Path, branch: &str) -> Result<bool, GitError> {
    // `git status` runs first but never sees the branch, and the `rev-parse` below is a probe
    // whose failure is expected and ignored. `git log` is the command a bad branch actually
    // breaks, so that is the one to name.
    refuse_option_like("git log", "branch", branch, BRANCH_ADVICE)?;
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
    refuse_option_like("git checkout", "branch", branch, BRANCH_ADVICE)?;
    refuse_option_like("git reset", "base", base, BASE_ADVICE)?;
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
