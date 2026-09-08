//! Temporary git repositories for the M2 tests. Each test builds its own, so no test
//! touches the author's checkouts.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Runs git and returns its trimmed stdout, panicking with stderr on failure.
pub fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("git {args:?}: {e}"));
    assert!(
        out.status.success(),
        "git {args:?} in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A commit with one file, so a repository has history to branch from.
pub fn commit(dir: &Path, name: &str, body: &str) {
    std::fs::write(dir.join(name), body).unwrap();
    git(dir, &["add", name]);
    git(dir, &["commit", "-q", "-m", &format!("Add {name}")]);
}

/// A bare origin and a clone of it with one commit on `branch` and `origin/HEAD` set, which
/// is what `git::default_branch` reads. Returns the temp dir (keep it alive) and the clone.
pub fn repo_with_origin(branch: &str) -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let origin = tmp.path().join("origin.git");
    let work = tmp.path().join("audrey-app");
    std::fs::create_dir_all(&origin).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    // Every other git call in these tests names its directory with `-C`. This one takes the
    // repository as an argument instead, so it is given an explicit working directory as well:
    // without one it would run in the test binary's own directory, inside a real checkout. A
    // bare init that failed would surface later as a confusing push error, so read its status
    // rather than dropping it.
    let status = Command::new("git")
        .current_dir(tmp.path())
        .args(["init", "-q", "--bare", "-b", branch])
        .arg(&origin)
        .status()
        .unwrap();
    assert!(status.success(), "git init --bare in {}", origin.display());
    git(&work, &["init", "-q", "-b", branch]);
    git(&work, &["config", "user.email", "test@example.com"]);
    git(&work, &["config", "user.name", "domux test"]);
    // The author's own git configuration reaches these repositories otherwise, and a global
    // `commit.gpgsign` would have these tests try to sign, a global `core.hooksPath` would run
    // that machine's hooks inside them. Repository configuration wins over global for every
    // command against this repository, including the ones that go through `git::run` and the
    // ones that run in its worktrees, so the isolation belongs here and not in production code.
    let no_hooks = tmp.path().join("no-hooks");
    git(
        &work,
        &["config", "core.hooksPath", no_hooks.to_str().unwrap()],
    );
    git(&work, &["config", "commit.gpgsign", "false"]);
    // The push below runs origin's receive hooks, so origin needs the same.
    git(
        &origin,
        &["config", "core.hooksPath", no_hooks.to_str().unwrap()],
    );
    commit(&work, "README.md", "hello\n");
    git(
        &work,
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    git(&work, &["push", "-q", "-u", "origin", branch]);
    git(&work, &["remote", "set-head", "origin", branch]);
    (tmp, work)
}
