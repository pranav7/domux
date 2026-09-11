//! Records `git describe` at build time so `domux --version` can say which commit built it.
//!
//! The release pipeline sets `DOMUX_GIT_DESCRIBE` to the release tag, because a shallow checkout
//! has no tags to describe. Without the variable and without git the value is `unknown`: never a
//! guess.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=DOMUX_GIT_DESCRIBE");
    // A commit moves HEAD in the worktree's git dir; a new tag lands under the common dir.
    if let Some(dir) = git(&["rev-parse", "--git-dir"]).map(PathBuf::from) {
        println!("cargo:rerun-if-changed={}", dir.join("HEAD").display());
    }
    if let Some(common) = git(&["rev-parse", "--git-common-dir"]).map(PathBuf::from) {
        println!(
            "cargo:rerun-if-changed={}",
            common.join("packed-refs").display()
        );
        println!(
            "cargo:rerun-if-changed={}",
            common.join("refs/tags").display()
        );
    }
    let describe = env::var("DOMUX_GIT_DESCRIBE")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| git(&["describe", "--tags", "--always", "--dirty"]))
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=DOMUX_GIT_DESCRIBE={describe}");
}

/// Runs git with `args` and returns trimmed stdout, or `None` when git is missing or fails.
fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}
