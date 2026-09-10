//! What the server registers when it starts with an empty model.
//!
//! The seed is the one registration nobody types, so it is the one that used to differ from
//! `project.add`: it called `add_folder_project` and asked git nothing. A server started in a
//! repository therefore held a folder project, which `facts::targets` skips - no branch on
//! the sidebar row - and which adopted none of the `workspace-N` worktrees already on disk.

use domux_core::config::Config;
use domux_core::facts::{FactKey, FACT_BRANCH};
use domux_core::model::{ProjectKind, WorkspaceHandle};
use domux_server::facts::branch::BranchProvider;
use domux_server::testing::{git, repo_with_origin, Harness, HarnessOptions};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// A harness whose server starts in `root`, with the branch provider registered so the fact
/// the sidebar row draws is really fetched.
async fn started_in(root: &Path) -> Harness {
    let mut opts = HarnessOptions::new(Config::default(), 80, 24);
    opts.project_root = Some(root.to_path_buf());
    opts.providers = vec![Arc::new(BranchProvider)];
    Harness::start_with(opts).await
}

/// The one project the seed made, as the model holds it.
fn seeded(h: &Harness) -> domux_core::model::Project {
    let model = h.model();
    assert_eq!(
        model.projects.len(),
        1,
        "the seed makes exactly one project on an empty model"
    );
    model.projects[0].clone()
}

/// Adds a worktree under `dir` the way V1 left them: a directory named for its slot, on a
/// branch of the same name. Answers with the path resolved, because the seed canonicalizes
/// the root it registers - `project.add` has always done that, and the seed now reads the
/// path the same way - so a temp path under `/tmp` is under `/private/tmp` in the model.
fn worktree(root: &Path, dir: &str, slot: &str) -> PathBuf {
    let path = root.join(dir).join(slot);
    git(
        root,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            slot,
            path.to_str().expect("a temp path is utf-8"),
        ],
    );
    path.canonicalize().expect("the worktree is there")
}

#[tokio::test]
async fn a_repository_the_server_starts_in_is_a_git_project() {
    let (_tmp, repo) = repo_with_origin("main");
    let mut h = started_in(&repo).await;
    let project = seeded(&h);
    assert_eq!(
        project.kind,
        ProjectKind::Git {
            default_branch: "main".into()
        },
        "the seed asks git what the directory is"
    );
    h.stop().await;
}

#[tokio::test]
async fn a_folder_the_server_starts_in_is_still_a_folder_project() {
    let tmp = tempfile::tempdir().unwrap();
    let plain = tmp.path().join("notes");
    std::fs::create_dir_all(&plain).unwrap();
    let mut h = started_in(&plain).await;
    let project = seeded(&h);
    assert_eq!(
        project.kind,
        ProjectKind::Folder,
        "a directory that is not a repository has no branch to ask for"
    );
    assert_eq!(
        project.workspaces.len(),
        1,
        "a folder project has main and no slots"
    );
    h.stop().await;
}

/// The defect MUX-7 reported: the row for the project the server was started in showed no
/// branch, because a folder project is skipped by `facts::targets` and so is never asked.
#[tokio::test]
async fn the_seeded_project_has_a_branch_to_draw() {
    let (_tmp, repo) = repo_with_origin("main");
    let mut h = started_in(&repo).await;
    let main = seeded(&h)
        .workspaces
        .iter()
        .find(|w| w.handle == WorkspaceHandle::Main)
        .expect("every project has main")
        .id
        .clone();
    let fact = h
        .wait_for_fact(
            &FactKey::workspace(&main, FACT_BRANCH),
            |f| f.is_some(),
            Duration::from_secs(5),
        )
        .await
        .expect("the provider answered");
    assert_eq!(fact.text, "main");
    h.stop().await;
}

/// The defect MUX-8 reported: worktrees V1 made are on disk beside the repository and the
/// seed walked past them, so switching to V2 meant building every workspace again.
#[tokio::test]
async fn the_worktrees_already_beside_a_repository_are_adopted() {
    let (_tmp, repo) = repo_with_origin("main");
    let one = worktree(&repo, ".domux/worktrees", "workspace-1");
    let two = worktree(&repo, ".domux/worktrees", "workspace-2");
    let mut h = started_in(&repo).await;
    let project = seeded(&h);
    let handles: Vec<String> = project
        .workspaces
        .iter()
        .map(|w| w.handle.to_string())
        .collect();
    assert_eq!(handles, vec!["main", "workspace-1", "workspace-2"]);
    let paths: Vec<&PathBuf> = project.workspaces.iter().map(|w| &w.path).collect();
    assert_eq!(paths[1], &one);
    assert_eq!(paths[2], &two);
    h.stop().await;
}

/// V1 wrote `.baag/worktrees` before the rename, and `slot_directory` answers with the path a
/// slot is really at rather than the path V2 would have made. The seed goes through the same
/// reading as `project.add`, so it inherits that.
#[tokio::test]
async fn a_worktree_under_the_name_v1_used_before_the_rename_is_adopted_where_it_is() {
    let (_tmp, repo) = repo_with_origin("main");
    let legacy = worktree(&repo, ".baag/worktrees", "workspace-1");
    let mut h = started_in(&repo).await;
    let project = seeded(&h);
    let slot = project
        .workspaces
        .iter()
        .find(|w| w.handle != WorkspaceHandle::Main)
        .expect("the legacy worktree is a slot");
    assert_eq!(slot.path, legacy);
    h.stop().await;
}
