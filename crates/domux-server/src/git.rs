//! Every git subprocess V2 runs, one function per operation, behaving as V1's
//! `workspaces.go` does. Each function blocks: call it from `tokio::task::spawn_blocking`,
//! never from the core task.

use crate::subprocess;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

/// Where a project's slots live. V1 wrote `.baag/worktrees` before the rename and still
/// reads it, so V2 reads it too and creates only under the current name.
pub const WORKTREE_DIR: &str = ".domux/worktrees";
pub const LEGACY_WORKTREE_DIR: &str = ".baag/worktrees";

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    /// git ran and refused, or this file refused before running it. `message` is git's own
    /// words when git spoke (principle 9).
    #[error("{command} failed: {message}")]
    Failed { command: String, message: String },
    /// The command was still running when its time ran out, so domux stopped it. The reader
    /// gets the one thing they can act on: the command, and where to run it themselves.
    // `{:?}` rather than `as_secs()`: a limit under a second reads as "0 seconds", which is
    // a wrong number in front of a reader who is about to run the command themselves.
    #[error(
        "{command} did not finish within {:?}, so domux stopped it; run it in {dir} to see \
         what it is waiting for",
        .limit
    )]
    TimedOut {
        command: String,
        dir: String,
        limit: Duration,
    },
}

impl GitError {
    fn failed(command: impl Into<String>, message: impl Into<String>) -> GitError {
        GitError::Failed {
            command: command.into(),
            message: message.into(),
        }
    }
}

/// `git -C ""` is a documented no-op: git runs wherever the server was started, which is
/// somebody's checkout, and a relative path resolves against the same place. Every directory
/// this file acts on is refused unless it is absolute, so `clean -fd` and `reset --hard`
/// cannot land somewhere nobody named.
fn not_absolute(command: String, dir: &Path) -> GitError {
    GitError::failed(
        command,
        format!(
            "the directory \"{}\" is not an absolute path; pass the project root",
            dir.display()
        ),
    )
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
    Err(GitError::failed(
        command,
        format!(
            "the {kind} \"{value}\" starts with \"-\", which git reads as an option rather than \
             a name; {advice}"
        ),
    ))
}

/// The base comes from `[worktrees] base` in the author's configuration, so that is what a
/// refusal sends them to. A slot branch comes from `slot_branch`, so a bad one is a caller's
/// mistake rather than a setting.
const BASE_ADVICE: &str = "set [worktrees] base to a branch or ref name";
const BRANCH_ADVICE: &str = "a slot branch is named workspace-<number>";

/// How long any one git command may run before domux stops it.
///
/// **One number for every command, not one per operation.** `run` is the single place a git
/// process is built, and that is what makes a command added later impossible to leave
/// unbounded. A table of per-operation bounds brings the miss straight back: whoever adds
/// `git push` gets whichever entry the table calls the default, and a `push` bounded like a
/// `rev-parse` is a worse defect than the one this fixes. The bound is here to guarantee that
/// a command ends, not to make one quick, so a number that is too generous costs a wait and a
/// number that is too tight costs a create that fails on a repository whose only fault was
/// being large.
///
/// Sized from the slow end, measured on this machine against a repository of 50,000 files:
/// `git reset --hard` restoring all of them 4.8s, `git worktree add` checking them out 4.3s,
/// `git clean -fd` removing 50,000 untracked files 2.0s, `git status --porcelain` 0.2s,
/// `git rev-parse` 0.005s. The cost tracks the number of files, so a 400,000 file checkout
/// extrapolates to around 40 seconds, and a `post-checkout` hook is the author's own code on
/// top of that. 300 seconds is about seven times that extrapolation. `git fetch` is the other
/// end and the only command here that waits on a remote: one branch is normally seconds, but
/// it depends on a link domux does not control, so the bound leaves room for a slow one.
///
/// What this replaces is not a looser bound, it is no bound at all. git applies no stall
/// timeout of its own unless `http.lowSpeedLimit` is set, and it is unset by default; a remote
/// that accepts the connection and then goes quiet leaves git on a socket macOS will not probe
/// for `net.inet.tcp.keepidle`, which ships at 7,200,000 ms. Both checked on the machine these
/// numbers come from, git 2.50.1.
///
/// The bound is per command, so an operation that runs several has a worst case of their sum:
/// `worktree_add` runs up to five, so 25 minutes rather than 5. That is still bounded, which
/// is the property the fact registry and `Core::claims` need, and a deadline for a whole
/// operation belongs to the job lane rather than to this file.
pub const TIME_LIMIT: Duration = Duration::from_secs(300);

/// Runs git in `dir` and returns its trimmed stdout. stderr and stdout are joined in the
/// error so git's own words reach the engineer (principle 9). Refuses a `dir` that is not
/// absolute, which includes the empty path: the check lives here, at the one place every
/// command goes through, so an operation added later cannot miss it. The same reasoning puts
/// `TIME_LIMIT` here.
pub fn run(dir: &Path, args: &[&str]) -> Result<String, GitError> {
    run_within(dir, args, TIME_LIMIT)
}

/// `run` with the bound spelled out, which only the tests do. It stays private on purpose:
/// the limit is not a caller's choice, so `run` is the one way in and a later operation
/// cannot quietly pick a bound of its own. The tests need it because the alternative is a
/// test that waits out `TIME_LIMIT`.
fn run_within(dir: &Path, args: &[&str], limit: Duration) -> Result<String, GitError> {
    let command = format!("git {}", args.join(" "));
    if !dir.is_absolute() {
        return Err(not_absolute(command, dir));
    }
    tracing::debug!(dir = %dir.display(), %command, "git");
    let mut process = Command::new("git");
    process.arg("-C").arg(dir).args(args);
    // `subprocess::output_within` sets the three streams and drains both pipes as the child
    // writes, so a git command printing a long error cannot fill a pipe and stall forever.
    // It closes stdin too, which `Command::output` already did: git asking a question nobody
    // is there to answer now ends at the bound rather than never.
    let out = subprocess::output_within(process, limit).map_err(|e| {
        if e.kind() == std::io::ErrorKind::TimedOut {
            GitError::TimedOut {
                command: command.clone(),
                dir: dir.display().to_string(),
                limit,
            }
        } else {
            // Everything else is git failing to start at all, which is what it was before.
            GitError::failed(command.clone(), e.to_string())
        }
    })?;
    if !out.status.success() {
        let mut message = String::from_utf8_lossy(&out.stderr).trim().to_string();
        if message.is_empty() {
            message = String::from_utf8_lossy(&out.stdout).trim().to_string();
        }
        return Err(GitError::Failed { command, message });
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
                return Err(GitError::failed(
                    format!("read {}", dir.display()),
                    e.to_string(),
                ))
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
        return Err(GitError::failed(
            format!("git worktree add {}", path.display()),
            "the slot directory is already there; remove it or pick another slot",
        ));
    }
    fetch(root, base)?;
    prune(root)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| GitError::failed(format!("mkdir {}", parent.display()), e.to_string()))?;
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

/// V1's `workspaceIsDirty`: uncommitted changes, or commits that exist only here.
///
/// "Only here" needs something to compare against. An upstream is the better answer when the
/// slot has one, because work the reader pushed is not work a delete would lose. With no
/// upstream - which is every slot `worktree_add` builds, since it is `--no-track` - the
/// comparison is against `base`, the ref the slot was branched from.
///
/// **`base` is passed, not guessed.** This function used to fall back to
/// `origin/<default branch>`, and while `worktree add -b` set an upstream pointing at the base,
/// the two were the same range whenever the base *was* the default branch, so the guess was
/// invisible. Dropping the upstream separated them: a slot from `origin/release` compared
/// against `origin/main` counts release's own commits as the slot's work and reads dirty the
/// moment it is created. The caller knows the base - `git::base_ref` is what produced it - so it
/// hands it over rather than letting this guess (decision record 0007).
pub fn is_dirty(path: &Path, branch: &str, base: &str) -> Result<bool, GitError> {
    // `git status` runs first but never sees either name, and the `rev-parse` below is a probe
    // whose failure is expected and ignored. `git log` is the command a bad branch or a bad
    // base actually breaks, so that is the one both refusals name.
    refuse_option_like("git log", "branch", branch, BRANCH_ADVICE)?;
    refuse_option_like("git log", "base", base, BASE_ADVICE)?;
    if !run(path, &["status", "--porcelain"])?.is_empty() {
        return Ok(true);
    }
    let upstream = format!("{branch}@{{u}}");
    let range = if run(path, &["rev-parse", &upstream]).is_ok() {
        format!("{upstream}..{branch}")
    } else {
        format!("{base}..{branch}")
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::finishes_within;
    use std::time::Instant;

    /// A repository where `git <name>` sleeps. An alias rather than a hanging remote: git
    /// itself is the process that takes the time, with no network and no remote helper in the
    /// way, so a test here cannot pass or fail for a reason other than the bound.
    fn repo_where_git_sleeps(name: &str, seconds: u32) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().to_path_buf();
        run(&repo, &["init", "-q"]).unwrap();
        run(
            &repo,
            &[
                "config",
                &format!("alias.{name}"),
                &format!("!sleep {seconds}"),
            ],
        )
        .unwrap();
        (dir, repo)
    }

    /// The bound, and the whole point of this change: a git command that will not finish ends
    /// at the limit with a reason, rather than parking its blocking task for the life of the
    /// server.
    ///
    /// Bounded from the outside as well. The failure this guards against is a hang, and a
    /// test binary cannot report "this did not finish": without `finishes_within` a lost bound
    /// would stall the whole suite silently instead of failing here.
    ///
    /// This is the first of the two links that carry "a timed-out git command releases its
    /// slot claim". It gives the first: a timeout is an `Err` of the same type every other git
    /// failure is. The second is
    /// `workspace_create::a_create_that_failed_gives_its_slot_number_back`, which drives a real
    /// `workspace.create` whose `git fetch` fails all the way through `start_job`, `run_job`,
    /// `create_workspace` and `job_finished`, and shows the number comes back. Nothing between
    /// the two reads the variant: `create_workspace` takes `e.to_string()` from any `Err`, so
    /// the two links compose with no logic in between. Exercising it end to end would mean a
    /// test that waits out `TIME_LIMIT`, which is five minutes.
    #[test]
    fn a_git_command_that_does_not_finish_is_stopped_at_the_limit() {
        let (_dir, repo) = repo_where_git_sleeps("wait", 30);
        let where_it_ran = repo.clone();
        let started = Instant::now();
        let err = finishes_within(
            Duration::from_secs(20),
            "a git command that never answers",
            move || run_within(&where_it_ran, &["wait"], Duration::from_millis(300)).unwrap_err(),
        );
        let GitError::TimedOut {
            command,
            dir,
            limit,
        } = &err
        else {
            panic!("a command that ran out of time is not a plain failure: {err}");
        };
        assert_eq!(command, "git wait", "the command reaches the reader");
        assert_eq!(dir, &repo.display().to_string(), "and so does where it ran");
        assert_eq!(limit, &Duration::from_millis(300));
        // The reader is told what to do, not what happened inside domux.
        let text = err.to_string();
        assert!(
            text.contains("run it in") && text.contains("waiting for"),
            "the message says what to do about it: {text}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "the limit is what ended it, not the command: {:?}",
            started.elapsed()
        );
    }

    /// The bound classifies on the io error's kind, not on "something went wrong". A command
    /// that answers inside the limit is untouched, and one that fails for its own reason is
    /// still a plain failure carrying git's words: without this, a mutant that reported every
    /// git failure as a timeout would tell the author to go and run a command that had already
    /// told them what was wrong.
    #[test]
    fn a_git_command_that_answers_inside_the_bound_is_not_touched_by_it() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().to_path_buf();
        let ample = Duration::from_secs(30);
        run_within(&repo, &["init", "-q"], ample).unwrap();
        assert_eq!(
            run_within(&repo, &["rev-parse", "--is-inside-work-tree"], ample).unwrap(),
            "true"
        );
        let err = run_within(&repo, &["rev-parse", "--verify", "no-such-ref"], ample).unwrap_err();
        assert!(
            matches!(err, GitError::Failed { .. }),
            "a refusal is not a timeout: {err}"
        );
        assert!(
            err.to_string()
                .starts_with("git rev-parse --verify no-such-ref failed: "),
            "{err}"
        );
    }

    /// A git that could not be started at all is a plain failure carrying the reason, never a
    /// timeout. The two arrive by the same door: `output_within` answers both with an
    /// `io::Error`, and only its kind tells them apart.
    ///
    /// Telling the author that git "did not finish, run it yourself" when the truth is that
    /// git is not installed, or is not on the path, sends them to watch a command that will
    /// never start. A mutation sweep found this: with the classification always true, every
    /// test in the workspace still passed, because nothing here had ever failed to start one.
    ///
    /// An argument past `ARG_MAX` rather than a git that is missing. It reaches the same
    /// branch, deterministically, in a test that can run beside every other one: making git
    /// missing means editing `PATH`, which is one setting shared by every test in the binary.
    #[test]
    fn a_git_that_could_not_be_started_is_a_plain_failure_and_never_a_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().to_path_buf();
        // kern.argmax is 1MB on this machine and 2MB on Linux; 4MB is past both.
        let too_long = "x".repeat(4 * 1024 * 1024);
        let err =
            run_within(&repo, &["rev-parse", &too_long], Duration::from_millis(300)).unwrap_err();
        // Nothing prints `err` here: its `command` holds the 4MB argument.
        assert!(
            matches!(err, GitError::Failed { .. }),
            "a git that never started is not a git that ran out of time"
        );
        let GitError::Failed { message, .. } = &err else {
            unreachable!()
        };
        assert!(
            message.to_lowercase().contains("argument list too long"),
            "the reason reaches the reader: {message}"
        );
    }

    /// `run` gives a git command the whole of `TIME_LIMIT`, so a command that takes seconds
    /// is not cut off.
    ///
    /// The mutant this catches lives in `run`, which is why the test goes through `run`.
    /// Asserting something about `TIME_LIMIT` alone leaves `run` free to pass any other
    /// number, and `run` is the only caller there is, so an assertion about the constant on
    /// its own says nothing about what git actually gets. `tests/git.rs` carries the other
    /// half, which is the constant against the measured cost of a real `worktree_add`.
    ///
    /// Two seconds because that is enough to catch a bound of one and cheap enough to run
    /// every time. The band between two seconds and `TIME_LIMIT` is not covered by either
    /// test and cannot be without a test that waits: nothing distinguishes 300 seconds from
    /// 3,000 except sitting through the difference. That half is argued where `TIME_LIMIT`
    /// is declared.
    #[test]
    fn run_gives_a_git_command_the_whole_time_limit() {
        let (_dir, repo) = repo_where_git_sleeps("slow", 2);
        let started = Instant::now();
        run(&repo, &["slow"]).expect("two seconds is nothing against the bound run gives");
        assert!(
            started.elapsed() >= Duration::from_secs(2),
            "the command really did take that long, so the bound really did allow it: {:?}",
            started.elapsed()
        );
    }
}
