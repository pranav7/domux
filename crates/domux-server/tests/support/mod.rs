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
    // A bare init that failed here would surface later as a confusing push error, so read
    // its status rather than dropping it.
    let status = Command::new("git")
        .args(["init", "-q", "--bare", "-b", branch])
        .arg(&origin)
        .status()
        .unwrap();
    assert!(status.success(), "git init --bare in {}", origin.display());
    git(&work, &["init", "-q", "-b", branch]);
    git(&work, &["config", "user.email", "test@example.com"]);
    git(&work, &["config", "user.name", "domux test"]);
    commit(&work, "README.md", "hello\n");
    git(
        &work,
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    git(&work, &["push", "-q", "-u", "origin", branch]);
    git(&work, &["remote", "set-head", "origin", branch]);
    (tmp, work)
}
