//! `workspace.clear` and `workspace.delete`: the two operations that run `git reset --hard`,
//! `git clean -fd` and `git worktree remove` on the author's own work.
//!
//! Every repository here is built by `tempfile::tempdir` through `repo_with_origin`, and
//! every path these tests destroy is inside one. Nothing reads or writes a checkout the
//! author has.
//!
//! Two rules shape the assertions. A test for a guard asserts the refusal **and** that the
//! side effect did not happen: a delete that removed the worktree and then complained would
//! carry the same error code as one that refused first. And a slot whose branch equals its
//! handle cannot tell "the branch the worktree is on" from "the branch the handle names", so
//! the tests that are about that distinction check the slot out onto another branch first.

mod support;

use domux_core::api::{ApiError, ErrorCode};
use domux_core::config::Config;
use domux_core::model::{Focus, Model, RegionKind, WorkspaceHandle};
use domux_server::testing::Harness;
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;

/// `Harness::api` with a bound sized for git rather than for the model.
///
/// A delete runs up to six git processes and a clear fetches, against a machine that is also
/// running the rest of the suite. `Harness::api` gives every call five seconds, which is the
/// bound this work trips first, and a test that fails on the clock is worse than a slow one:
/// a spurious failure reads as "the mutant was killed", so a real hole reports as covered.
async fn api(h: &Harness, method: &str, params: Value) -> Result<Value, ApiError> {
    support::api_at(h.socket_path(), method, params).await
}

/// The published model once `pred` holds.
///
/// The core answers a job's caller from inside the batch that changed the model and
/// publishes the snapshot at the end of that batch, so a read taken straight after the
/// answer can be one batch behind.
async fn model_when(h: &Harness, what: &str, pred: impl Fn(&Model) -> bool) -> Model {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let m = h.model();
        if pred(&m) {
            return m;
        }
        assert!(tokio::time::Instant::now() < deadline, "{what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// The id of the project's own checkout.
///
/// By id, not by the name `main`: the harness always holds a second project of its own, so
/// `main` names two workspaces and the resolver answers `ambiguous` rather than picking one.
fn main_of(h: &Harness) -> String {
    h.model()
        .projects
        .iter()
        .find(|p| p.name == "audrey-app")
        .and_then(|p| {
            p.workspaces
                .iter()
                .find(|w| w.handle == WorkspaceHandle::Main)
        })
        .map(|w| w.id.to_string())
        .expect("the git project has a main")
}

fn slot_of(root: &Path, n: u32) -> std::path::PathBuf {
    root.join(format!(".domux/worktrees/workspace-{n}"))
}

fn branches(root: &Path) -> String {
    support::git(root, &["branch", "--list", "--format=%(refname:short)"])
}

// ---------------------------------------------------------------- delete

#[tokio::test]
async fn delete_removes_the_worktree_and_the_branch_and_frees_the_number() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let (root, w1, _w2) = h.git_project_with_two_slots().await;
    api(
        &h,
        "workspace.delete",
        json!({"workspace": "workspace-1", "yes": true}),
    )
    .await
    .unwrap();
    assert!(!slot_of(&root, 1).exists());
    let left = branches(&root);
    assert!(
        !left.lines().any(|b| b == "workspace-1"),
        "the local branch goes with it: {left}"
    );
    let m = model_when(&h, "the record goes", |m| m.workspace(&w1).is_none()).await;
    assert!(
        m.workspace(&w1).is_none(),
        "and the record with the worktree"
    );
    let made = api(&h, "workspace.create", json!({"project": "audrey-app"}))
        .await
        .unwrap();
    assert_eq!(made["handle"], "workspace-1", "a freed number comes back");
}

/// `main` is the project's own checkout, so there is no worktree to remove and no branch that
/// could be deleted without taking the repository (architecture spec: "path = project root;
/// can't be cleared or deleted").
///
/// The refusal comes before the confirmation, which is why this asks with no `yes`: a handler
/// that checked consent first would answer the question instead of the refusal, and the
/// reader would be asked to confirm something that was never going to happen.
#[tokio::test]
async fn deleting_main_is_refused_by_name_and_leaves_the_checkout_alone() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let (root, _w1, _w2) = h.git_project_with_two_slots().await;
    let err = api(&h, "workspace.delete", json!({"workspace": main_of(&h)}))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Refused);
    assert_eq!(
        err.message,
        "main is the project's checkout and cannot be deleted; delete a workspace-N slot instead"
    );
    assert!(
        err.data.is_none(),
        "a refusal by name is not a question: {err:?}"
    );
    assert!(
        root.join("README.md").is_file(),
        "the checkout is untouched"
    );
    assert!(slot_of(&root, 1).is_dir(), "and so are its slots");
}

/// Interface spec 12.22: `domux workspace delete workspace-1` prints the confirmation and
/// fails unless `--yes` is given.
///
/// The harness registers no fact provider, so no branch fact has arrived, and the question
/// says "its local branch" rather than naming the handle. Naming the handle would promise to
/// delete a branch this call has not looked at (principle 4); the job reads the real one.
#[tokio::test]
async fn deleting_without_yes_asks_and_changes_nothing() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let (root, w1, _w2) = h.git_project_with_two_slots().await;
    let err = api(&h, "workspace.delete", json!({"workspace": "workspace-1"}))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Refused);
    assert_eq!(
        err.message,
        "Delete workspace-1? Removes the worktree at .domux/worktrees/workspace-1 and its \
         local branch and closes 1 tab. The remote branch and any pull request stay. \
         Answer with --yes"
    );
    // `ApiError::needs_confirmation` writes both halves from one call, so the message and the
    // lists cannot disagree.
    let data = err.data.as_ref().unwrap();
    assert_eq!(
        data["removes"],
        json!([
            "the worktree at .domux/worktrees/workspace-1",
            "its local branch"
        ])
    );
    assert_eq!(
        data["keeps"],
        json!(["the remote branch and any pull request"])
    );
    // Asking is not doing, and an error code alone does not prove that.
    assert!(slot_of(&root, 1).is_dir(), "the worktree is still there");
    assert!(
        branches(&root).lines().any(|b| b == "workspace-1"),
        "and so is its branch"
    );
    assert!(h.model().workspace(&w1).is_some(), "and so is its record");
}

/// The branch that goes is the one the worktree is on, not the one the handle names.
///
/// The slot is checked out onto `feat/auth-cleanup` first, because a slot whose branch equals
/// its handle cannot tell the two apart: every assertion below would pass against a job that
/// read `workspace-1` off the handle and never asked git.
///
/// The job asks git rather than reading the branch fact, and `git::is_dirty` weighs the same
/// answer, so the guard and the action are about one branch. What the fact is for is the
/// question, which is composed on the core task where no git command may run;
/// `core::tests::the_delete_question_names_the_branch_that_was_observed` is that half.
#[tokio::test]
async fn delete_removes_the_branch_the_worktree_is_on_not_the_one_the_handle_names() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let (root, _w1, _w2) = h.git_project_with_two_slots().await;
    let slot = slot_of(&root, 1);
    support::git(&slot, &["checkout", "-q", "-b", "feat/auth-cleanup"]);

    api(
        &h,
        "workspace.delete",
        json!({"workspace": "workspace-1", "yes": true}),
    )
    .await
    .unwrap();
    assert!(!slot.exists());
    let left = branches(&root);
    assert!(
        !left.lines().any(|b| b == "feat/auth-cleanup"),
        "the branch that goes is the one it was on: {left}"
    );
}

/// An uncommitted change is refused without `--force`, and the refusal happens before
/// anything is removed.
#[tokio::test]
async fn a_dirty_workspace_needs_force_and_says_what_is_in_it() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let (root, w1, _w2) = h.git_project_with_two_slots().await;
    let slot = slot_of(&root, 1);
    std::fs::write(slot.join("scratch.txt"), "work").unwrap();
    let err = api(
        &h,
        "workspace.delete",
        json!({"workspace": "workspace-1", "yes": true}),
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::Refused);
    assert_eq!(
        err.message,
        "workspace-1 has uncommitted or unpushed changes; delete it with --force or commit \
         and push first"
    );
    assert!(slot.is_dir(), "the worktree is still there");
    assert!(
        slot.join("scratch.txt").is_file(),
        "and so is what was in it"
    );
    assert!(
        branches(&root).lines().any(|b| b == "workspace-1"),
        "and so is its branch"
    );
    assert!(h.model().workspace(&w1).is_some(), "and so is its record");

    api(
        &h,
        "workspace.delete",
        json!({"workspace": "workspace-1", "yes": true, "force": true}),
    )
    .await
    .unwrap();
    assert!(!slot.exists());
}

/// A commit that was never pushed is dirty too, and it is the half `git worktree remove
/// --force` cannot see: git refuses a worktree holding modified or untracked files, and a
/// slot holding a week of clean commits looks empty to it.
///
/// So this is the case the guard exists for. `git status` is clean here; only the range
/// comparison in `git::is_dirty` can find the work.
#[tokio::test]
async fn a_commit_that_was_never_pushed_is_dirty_too() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let (root, _w1, _w2) = h.git_project_with_two_slots().await;
    let slot = slot_of(&root, 1);
    support::commit(&slot, "spike.md", "a week of work\n");
    assert_eq!(
        support::git(&slot, &["status", "--porcelain"]),
        "",
        "the tree is clean, so only the commit makes this dirty"
    );
    let err = api(
        &h,
        "workspace.delete",
        json!({"workspace": "workspace-1", "yes": true}),
    )
    .await
    .unwrap_err();
    assert_eq!(err.code, ErrorCode::Refused);
    assert!(
        slot.join("spike.md").is_file(),
        "and the commit is still there"
    );
}

/// Deleting the workspace a client is in moves it somewhere it can be drawn, and the
/// processes in its panes are killed rather than left running behind a record that is gone.
#[tokio::test]
async fn deleting_a_workspace_moves_its_client_and_kills_its_panes() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let (_root, w1, _w2) = h.git_project_with_two_slots().await;
    api(&h, "workspace.focus", json!({"workspace": "workspace-1"}))
        .await
        .unwrap();
    let m = model_when(&h, "the client is in workspace-1", |m| {
        m.client(&h.client).is_some_and(|v| v.workspace == w1)
    })
    .await;
    let panes: Vec<_> = m
        .workspace(&w1)
        .unwrap()
        .tabs
        .iter()
        .flat_map(|t| t.layout.pane_ids())
        .collect();
    assert!(!panes.is_empty(), "the slot has panes to close");
    assert!(
        panes.iter().all(|p| h.pane_is_running(p)),
        "and they are running"
    );

    api(
        &h,
        "workspace.delete",
        json!({"workspace": "workspace-1", "yes": true}),
    )
    .await
    .unwrap();
    let m = model_when(&h, "the record goes", |m| m.workspace(&w1).is_none()).await;
    let view = m.client(&h.client).expect("the client is still attached");
    assert_ne!(view.workspace, w1);
    assert!(
        m.workspace(&view.workspace).is_some() && m.tab(&view.tab).is_some(),
        "and it is somewhere a frame can be drawn for it"
    );
    assert!(
        panes.iter().all(|p| !h.pane_is_running(p)),
        "the processes go with the slot, or they outlive the record on screen"
    );
}

// ---------------------------------------------------------------- clear

/// Clearing `main` would run `git reset --hard` and `git clean -fd` in the author's own
/// checkout. The architecture spec says `main` "can't be cleared or deleted"; this is that.
#[tokio::test]
async fn clearing_main_is_refused_by_name_and_leaves_the_checkout_alone() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let (root, _w1, _w2) = h.git_project_with_two_slots().await;
    std::fs::write(root.join("uncommitted.txt"), "the author's work").unwrap();
    let err = api(&h, "workspace.clear", json!({"workspace": main_of(&h)}))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Refused);
    assert_eq!(
        err.message,
        "main is the project's checkout and cannot be cleared; clear a workspace-N slot instead"
    );
    assert!(
        root.join("uncommitted.txt").is_file(),
        "and nothing in the checkout was thrown away"
    );
}

#[tokio::test]
async fn clear_resets_the_branch_and_keeps_the_slot_its_number_and_its_name() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let (root, w1, _w2) = h.git_project_with_two_slots().await;
    api(
        &h,
        "workspace.rename",
        json!({"workspace": "workspace-1", "name": "auth cleanup"}),
    )
    .await
    .unwrap();
    let slot = slot_of(&root, 1);
    support::commit(&slot, "spike.md", "spike\n");
    std::fs::write(slot.join("scratch.txt"), "x").unwrap();
    let tabs = h.model().workspace(&w1).unwrap().tabs.len();

    let err = api(&h, "workspace.clear", json!({"workspace": "auth cleanup"}))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Refused);
    assert_eq!(
        err.message,
        "auth cleanup has uncommitted or unpushed changes; clear it with --yes to throw them away"
    );
    assert!(slot.join("spike.md").is_file(), "asking is not doing");
    assert!(slot.join("scratch.txt").is_file(), "for either of them");

    api(
        &h,
        "workspace.clear",
        json!({"workspace": "auth cleanup", "yes": true}),
    )
    .await
    .unwrap();
    assert!(
        !slot.join("spike.md").exists(),
        "the branch is back at the base"
    );
    assert!(!slot.join("scratch.txt").exists(), "and the tree is clean");
    assert!(slot.is_dir(), "the slot stays");
    let m = h.model();
    let w = m.workspace(&w1).expect("and so does its record");
    assert_eq!(w.name.as_deref(), Some("auth cleanup"), "and its name");
    assert_eq!(w.handle.to_string(), "workspace-1", "and its number");
    assert_eq!(w.tabs.len(), tabs, "and its tabs");
}

/// A clear resets the branch the worktree is on, and leaves it on that branch.
///
/// Same reason as the delete above: with the slot on `workspace-1`, a clear that checked out
/// the handle and one that read the branch do the same thing. With the slot on
/// `feat/auth-cleanup`, a clear that used the handle would check `workspace-1` out into it -
/// the slot would come back on a different branch and the work on `feat/auth-cleanup` would
/// still be sitting there, unreachable from the slot the author is looking at.
#[tokio::test]
async fn clear_resets_the_branch_the_worktree_is_on_not_the_one_the_handle_names() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let (root, _w1, _w2) = h.git_project_with_two_slots().await;
    let slot = slot_of(&root, 1);
    support::git(&slot, &["checkout", "-q", "-b", "feat/auth-cleanup"]);
    support::commit(&slot, "spike.md", "spike\n");

    api(
        &h,
        "workspace.clear",
        json!({"workspace": "workspace-1", "yes": true}),
    )
    .await
    .unwrap();
    assert_eq!(
        support::git(&slot, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "feat/auth-cleanup",
        "the slot is still on the branch it was on"
    );
    assert!(
        !slot.join("spike.md").exists(),
        "and that branch is the one that was put back at the base"
    );
}

/// `git::clean` is `-fd`, not `-fdx`: the files git ignores are where a project's
/// `worktree.conf` setup puts `.env` and its friends, and clearing a slot must not undo its
/// setup.
#[tokio::test]
async fn clear_keeps_the_files_git_ignores() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let (root, _w1, _w2) = h.git_project_with_two_slots().await;
    // The ignore rule goes on the base, not on the slot's own branch. A rule the slot alone
    // carries is thrown away by the reset before `git clean` runs, so `.env` would stop being
    // ignored on the way and the test would be about the reset rather than about `-fd`.
    std::fs::write(root.join(".gitignore"), ".env\n").unwrap();
    support::git(&root, &["add", ".gitignore"]);
    support::git(&root, &["commit", "-q", "-m", "Ignore .env"]);
    support::git(&root, &["push", "-q", "origin", "main"]);
    let slot = slot_of(&root, 1);
    std::fs::write(slot.join(".env"), "TOKEN=1\n").unwrap();
    std::fs::write(slot.join("scratch.txt"), "x").unwrap();

    api(
        &h,
        "workspace.clear",
        json!({"workspace": "workspace-1", "yes": true}),
    )
    .await
    .unwrap();
    assert!(
        !slot.join("scratch.txt").exists(),
        "an untracked file goes with the clear"
    );
    assert!(
        slot.join(".env").is_file(),
        "and a file git ignores does not, or the slot loses the setup that built it"
    );
}

/// A clear asks only when there is something to lose, and `git::is_dirty` is the only thing
/// that can know. So a pristine slot clears without `--yes`.
///
/// That is the difference from `delete`, which asks every time: a delete takes the slot
/// itself, so even a pristine one is a change the caller may not have meant.
#[tokio::test]
async fn a_clean_slot_clears_without_yes() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let (root, _w1, _w2) = h.git_project_with_two_slots().await;
    let slot = slot_of(&root, 1);
    api(&h, "workspace.clear", json!({"workspace": "workspace-1"}))
        .await
        .expect("a slot with nothing in it has nothing to confirm");
    assert!(slot.is_dir());
}

/// Each operation says on the screen that it worked, in the words it did it under and in the
/// green a result gets (interface spec 7.3 and 12.12).
///
/// A clear and a delete in one test on purpose: with only one of them, a pill that always
/// said the same word would pass. And a named slot, so "Cleared auth cleanup" is the sentence
/// rather than "Cleared workspace-1", which is what the handle would have produced.
#[tokio::test]
async fn a_clear_and_a_delete_each_say_what_they_did_in_the_hint_row() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let (_root, _w1, _w2) = h.git_project_with_two_slots().await;
    api(
        &h,
        "workspace.rename",
        json!({"workspace": "workspace-1", "name": "auth cleanup"}),
    )
    .await
    .unwrap();
    // The pill is drawn in the sidebar's hint row and the overlay's footer, so with neither
    // open this test could not tell an absent pill from an undrawn one.
    api(&h, "sidebar.show", json!({})).await.unwrap();

    api(
        &h,
        "workspace.clear",
        json!({"workspace": "auth cleanup", "yes": true}),
    )
    .await
    .unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("Cleared auth cleanup"),
            Duration::from_secs(15),
        )
        .await;
    // Catppuccin base on green, bold: a result pill, not a plain hint.
    assert!(
        f.contains("bold fg=#1e1e2e bg=#a6e3a1"),
        "and it is green:\n{f}"
    );

    api(
        &h,
        "workspace.delete",
        json!({"workspace": "auth cleanup", "yes": true}),
    )
    .await
    .unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("Deleted auth cleanup"),
            Duration::from_secs(15),
        )
        .await;
    assert!(
        f.contains("bold fg=#1e1e2e bg=#a6e3a1"),
        "and so is this:\n{f}"
    );
}

// ---------------------------------------------------------------- events

#[tokio::test]
async fn clearing_and_deleting_a_workspace_each_report_what_they_did() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let (_root, w1, _w2) = h.git_project_with_two_slots().await;
    let socket = h.socket_path().to_path_buf();
    let mut events = support::subscribe(&socket, &["workspace.cleared", "workspace.deleted"]).await;

    api(
        &h,
        "workspace.clear",
        json!({"workspace": "workspace-1", "yes": true}),
    )
    .await
    .unwrap();
    let cleared = support::next_event(&mut events).await;
    assert_eq!(cleared["event"], "workspace.cleared");
    assert_eq!(cleared["workspace"], w1.as_str());
    assert_eq!(
        cleared["base"], "origin/main",
        "the base it was put back at, which is the one thing about the reset that was decided"
    );

    api(
        &h,
        "workspace.delete",
        json!({"workspace": "workspace-1", "yes": true}),
    )
    .await
    .unwrap();
    let deleted = support::next_event(&mut events).await;
    assert_eq!(deleted["event"], "workspace.deleted");
    assert_eq!(deleted["workspace"], w1.as_str());
    assert_eq!(deleted["handle"], "workspace-1");
    assert_eq!(
        deleted["pruned"], false,
        "somebody asked for this one; a prune is the record going because its path did not"
    );
}

// ---------------------------------------------------------------- the keys

/// A key bound to `workspace.delete` asks on the screen rather than answering "add --yes" to
/// somebody who has no command line to add it to, and `y` is what deletes.
#[tokio::test]
async fn a_key_bound_to_workspace_delete_asks_in_an_overlay_and_y_deletes() {
    let mut config = Config::default();
    config
        .keys
        .bindings
        .insert("D".into(), "workspace.delete workspace-1".into());
    let mut h = Harness::start(config, 120, 24).await;
    let (root, w1, _w2) = h.git_project_with_two_slots().await;
    let slot = slot_of(&root, 1);

    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), "D").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("Delete workspace-1?"),
            Duration::from_secs(5),
        )
        .await;
    assert!(
        f.contains("Removes the worktree at .domux/worktrees/workspace-1"),
        "the box says what goes:\n{f}"
    );
    assert!(
        f.contains("The remote branch and any pull request stay."),
        "and what stays:\n{f}"
    );
    assert!(
        f.contains("y delete workspace    esc keep workspace"),
        "and how to answer:\n{f}"
    );
    assert_eq!(
        h.model().client(&h.client).map(|v| v.focus.clone()),
        Some(Focus::Region(RegionKind::Overlay)),
        "and the keys are in the box, not in the pane behind it"
    );
    assert!(
        f.contains("bold fg=#f38ba8 bg=#1e1e2e"),
        "the question is in the border in red (interface spec 7.3):\n{f}"
    );
    assert!(slot.is_dir(), "asking is not doing");

    h.key(h.client.clone(), "y").await;
    h.wait_for(
        h.client.clone(),
        |f| !f.contains("Delete workspace-1?"),
        Duration::from_secs(5),
    )
    .await;
    model_when(&h, "y is what deletes it", |m| m.workspace(&w1).is_none()).await;
    assert!(!slot.exists(), "and the worktree goes with the record");
}

/// Anything other than `y` closes the question, gives the keys back to the pane and takes the
/// dimming with it.
///
/// **It does not prove the workspace survives**, and the name says so. A delete from a key
/// queues work on a blocking task, so a test that closes the question and then looks at the
/// disk is racing the job it means to say never started, and it wins that race whether the
/// job was queued or not. Measured: `confirmed` forced to `true` - `esc` acting as `y` -
/// leaves this test green. The half it cannot carry is
/// `core::tests::esc_on_the_delete_question_starts_no_job_and_y_starts_one`, which watches
/// for the job itself.
#[tokio::test]
async fn a_key_other_than_y_closes_the_delete_question() {
    let mut config = Config::default();
    config
        .keys
        .bindings
        .insert("D".into(), "workspace.delete workspace-1".into());
    let mut h = Harness::start(config, 120, 24).await;
    let (_root, w1, _w2) = h.git_project_with_two_slots().await;

    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), "D").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("Delete workspace-1?"),
        Duration::from_secs(5),
    )
    .await;
    h.key(h.client.clone(), "Esc").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| !f.contains("Delete workspace-1?"),
            Duration::from_secs(5),
        )
        .await;
    assert!(
        h.model().workspace(&w1).is_some(),
        "the record is still there:\n{f}"
    );
    // With nothing underneath, the keys go back to the pane. Never a frame with the keys in a
    // region nothing on the screen marks (principle 2).
    assert!(
        matches!(
            h.model().client(&h.client).map(|v| v.focus.clone()),
            Some(Focus::Pane(_))
        ),
        "and the keys go back to the pane: {:?}",
        h.model().client(&h.client).map(|v| v.focus.clone())
    );
    // The dimming went with the box, which is also what keeps the red-border assertion in the
    // test above from being about nothing: with no overlay open, no frame here has a dim cell.
    assert!(
        !f.lines().any(|l| l.starts_with('r') && l.contains(" dim")),
        "the screen is not dimmed once the question is gone:\n{f}"
    );
}

/// A key that cannot do what it says still answers, on the screen, in words that name the
/// state and the next action (principles 8 and 9).
///
/// The refusal comes from the job, so this is also the path that proves a dirty slot survives
/// a keyed delete: the assertion waits for the refusal to arrive rather than racing it.
#[tokio::test]
async fn a_key_that_cannot_delete_says_why_on_the_screen() {
    let mut config = Config::default();
    config
        .keys
        .bindings
        .insert("D".into(), "workspace.delete workspace-1".into());
    let mut h = Harness::start(config, 120, 24).await;
    let (root, w1, _w2) = h.git_project_with_two_slots().await;
    let slot = slot_of(&root, 1);
    std::fs::write(slot.join("scratch.txt"), "work").unwrap();

    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), "D").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("Delete workspace-1?"),
        Duration::from_secs(5),
    )
    .await;
    h.key(h.client.clone(), "y").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("has uncommitted or unpushed changes"),
            Duration::from_secs(15),
        )
        .await;
    assert!(
        f.contains("commit and push first"),
        "and it names what to do about it:\n{f}"
    );
    assert!(
        slot.join("scratch.txt").is_file(),
        "the work is still there"
    );
    assert!(h.model().workspace(&w1).is_some(), "and so is the slot");
}

/// A key bound to `workspace.clear` asks every time, because a key press has no `--yes` to
/// add and the only thing that could tell it whether there is anything to lose shells out.
#[tokio::test]
async fn a_key_bound_to_workspace_clear_asks_in_an_overlay_and_y_clears() {
    let mut config = Config::default();
    config
        .keys
        .bindings
        .insert("C".into(), "workspace.clear workspace-1".into());
    let mut h = Harness::start(config, 120, 24).await;
    let (root, w1, _w2) = h.git_project_with_two_slots().await;
    let slot = slot_of(&root, 1);
    std::fs::write(slot.join("scratch.txt"), "x").unwrap();

    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), "C").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("Clear workspace-1?"),
            Duration::from_secs(5),
        )
        .await;
    assert!(
        f.contains("Throws away every commit, change and untracked file"),
        "the box says what goes:\n{f}"
    );
    assert!(
        f.contains("The slot, its number, its name and the files git ignores stay."),
        "and what stays:\n{f}"
    );
    assert!(
        f.contains("y clear workspace    esc keep workspace"),
        "and how to answer:\n{f}"
    );
    assert!(slot.join("scratch.txt").is_file(), "asking is not doing");

    h.key(h.client.clone(), "y").await;
    h.wait_for(
        h.client.clone(),
        |f| !f.contains("Clear workspace-1?"),
        Duration::from_secs(5),
    )
    .await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while slot.join("scratch.txt").exists() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "y is what clears it"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(slot.is_dir(), "and the slot stays");
    assert!(h.model().workspace(&w1).is_some(), "with its record");
}
