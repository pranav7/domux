//! `workspace.create`: the worktree, the branch and the `worktree.conf` setup of one slot.
//!
//! Every test here builds its own repository under `tempfile::tempdir`, because this is the
//! call that makes directories and branches.

mod support;

use domux_core::api::ErrorCode;
use domux_core::config::{Config, WorktreesConfig};
use domux_core::ids::WorkspaceId;
use domux_server::facts::branch::BranchProvider;
use domux_server::git;
use domux_server::testing::{Harness, HarnessOptions};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use support::{api_at, next_event, repo_with_origin, subscribe};

/// The workspaces of the project at `root`, by handle, in the order the model holds them.
fn handles(h: &Harness, root: &Path) -> Vec<String> {
    let canonical = root.canonicalize().expect("the project root is there");
    h.model()
        .project_at(&canonical)
        .map(|p| p.workspaces.iter().map(|w| w.handle.to_string()).collect())
        .unwrap_or_default()
}

/// A harness with the branch provider running, which is what the untouched glyph needs: a
/// slot draws as `◌ workspace-1` only while its branch fact equals its handle
/// (`Workspace::is_untouched`), and an unknown branch equals nothing (principle 4). 120
/// columns is the width at which the sidebar draws (interface spec 12.1).
async fn harness_with_branches(config: Config) -> Harness {
    Harness::start_with(HarnessOptions {
        providers: vec![Arc::new(BranchProvider)],
        ..HarnessOptions::new(config, 120, 24)
    })
    .await
}

/// The whole of a create: the worktree on disk, the fresh branch, the record, the tab whose
/// pane sits in the slot, and the second call taking the next number.
#[tokio::test]
async fn create_makes_the_worktree_on_a_fresh_branch_at_the_lowest_free_number() {
    let (_tmp, repo) = repo_with_origin("main");
    // The model records the canonical path, because `project.add`'s job resolves it before
    // anything is registered: a temp directory reaches here through `/var`, which on macOS is
    // a link to `/private/var`.
    let canonical = repo.canonicalize().unwrap();
    let mut h = harness_with_branches(Config::default()).await;
    h.api("project.add", json!({"path": repo.to_str().unwrap()}))
        .await
        .unwrap();

    let made = h
        .api("workspace.create", json!({"project": "audrey-app"}))
        .await
        .unwrap();
    let slot = canonical.join(".domux/worktrees/workspace-1");
    assert_eq!(made["handle"], "workspace-1");
    assert_eq!(made["branch"], "workspace-1");
    assert_eq!(made["path"], slot.to_str().unwrap());
    assert_eq!(made["base"], "origin/main", "from origin/HEAD");
    assert_eq!(
        made["setup"],
        Value::Null,
        "this project has no worktree.conf at all, which is absent rather than a summary of \
         nothing"
    );
    assert_eq!(
        made["tabs"], 1,
        "a fresh workspace has one tab with a shell in its worktree"
    );
    assert!(slot.join("README.md").is_file(), "the base is checked out");
    assert_eq!(
        git::branch_of(&slot).unwrap(),
        "workspace-1",
        "on its own branch, not on the base"
    );
    // The pane starts in the slot, not in the main checkout: a shell in the wrong directory
    // is the whole point of a worktree missed.
    let workspace = WorkspaceId(made["id"].as_str().unwrap().to_string());
    let pane = h.first_pane_of(made["id"].as_str().unwrap()).await;
    assert_eq!(h.model().pane(&pane).unwrap().cwd, slot);
    assert_eq!(
        h.model().workspace(&workspace).unwrap().path,
        slot,
        "and the record points at it"
    );

    let again = h
        .api("workspace.create", json!({"project": "audrey-app"}))
        .await
        .unwrap();
    assert_eq!(
        again["handle"], "workspace-2",
        "the next lowest free number"
    );
    assert_ne!(again["id"], made["id"]);
    assert_eq!(handles(&h, &repo), ["main", "workspace-1", "workspace-2"]);

    // A fresh model starts with the top bar (Task 2), so open the surface that draws the box.
    h.api("sidebar.show", json!({})).await.unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("◌ workspace-2"),
            Duration::from_secs(10),
        )
        .await;
    assert!(
        f.contains("◌ workspace-1"),
        "both slots draw as untouched (interface spec 12.23):\n{f}"
    );
}

/// The links and the copies are in the slot before the pane starts, and the run lines are
/// typed into that pane rather than run behind its back.
#[tokio::test]
async fn worktree_conf_links_and_copies_before_the_pane_starts_and_types_its_run_lines_into_it() {
    let (_tmp, repo) = repo_with_origin("main");
    std::fs::create_dir_all(repo.join(".domux")).unwrap();
    std::fs::write(
        repo.join(".domux/worktree.conf"),
        "link .env\ncopy setup.cfg\nrun bin/setup --fast\n",
    )
    .unwrap();
    std::fs::write(repo.join(".env"), "SECRET=1").unwrap();
    std::fs::write(repo.join("setup.cfg"), "x=1").unwrap();
    let mut h = Harness::start(Config::default(), 120, 24).await;
    h.api("project.add", json!({"path": repo.to_str().unwrap()}))
        .await
        .unwrap();

    let made = h
        .api("workspace.create", json!({"project": "audrey-app"}))
        .await
        .unwrap();
    let slot = repo
        .canonicalize()
        .unwrap()
        .join(".domux/worktrees/workspace-1");
    assert!(slot.join(".env").is_symlink());
    assert_eq!(
        std::fs::read_to_string(slot.join("setup.cfg")).unwrap(),
        "x=1"
    );
    assert!(
        !slot.join("setup.cfg").is_symlink(),
        "a copy is a file of its own, not a second link"
    );
    assert_eq!(made["setup"], "linked 1, copied 1, ran 1");
    let pane = h.first_pane_of(made["id"].as_str().unwrap()).await;
    assert_eq!(
        String::from_utf8_lossy(&h.pane_input(&pane)),
        "bin/setup --fast\n",
        "the run line is typed into the workspace's first pane, not run behind its back"
    );
    assert!(
        !slot.join("bin").exists(),
        "and `run` writes nothing itself"
    );
}

/// A setup line that cannot be applied is counted and the slot is still made, and a line
/// domux does not understand is not counted at all.
///
/// Three states in one file, because the summary is one string and a fixture with one kind of
/// line in it cannot tell the three apart: `copy` works and is counted, `link` names a file
/// the main checkout does not have and is skipped, and `sideways` is not a directive, which
/// `parse` drops with a warning rather than counting as a skip.
#[tokio::test]
async fn a_setup_line_that_cannot_be_applied_is_counted_as_skipped_and_the_slot_is_still_made() {
    let (_tmp, repo) = repo_with_origin("main");
    std::fs::create_dir_all(repo.join(".domux")).unwrap();
    std::fs::write(
        repo.join(".domux/worktree.conf"),
        "copy setup.cfg
link not-here.env
sideways whatever
",
    )
    .unwrap();
    std::fs::write(repo.join("setup.cfg"), "x=1").unwrap();
    let mut h = Harness::start(Config::default(), 120, 24).await;
    h.api("project.add", json!({"path": repo.to_str().unwrap()}))
        .await
        .unwrap();

    let made = h
        .api("workspace.create", json!({"project": "audrey-app"}))
        .await
        .unwrap();
    assert_eq!(made["setup"], "copied 1, 1 skipped");
    assert_eq!(
        handles(&h, &repo),
        ["main", "workspace-1"],
        "the slot is made"
    );
    let slot = PathBuf::from(made["path"].as_str().unwrap());
    assert!(slot.join("setup.cfg").is_file(), "and the rest still ran");
    assert!(!slot.join("not-here.env").exists());
}

/// A `worktree.conf` that asks for nothing summarises as the empty string, which is still a
/// different answer from a project that has no `worktree.conf` at all.
///
/// This is the distinction the whole `Option<Setup>` type exists for (principle 4), and the
/// first test asserts the absent half. Without this one nothing holds the present-but-empty
/// half, so collapsing `Some("")` to `None` would pass the suite and quietly undo the fix.
#[tokio::test]
async fn a_worktree_conf_that_asks_for_nothing_is_an_empty_summary_and_not_an_absent_one() {
    let (_tmp, repo) = repo_with_origin("main");
    std::fs::create_dir_all(repo.join(".domux")).unwrap();
    std::fs::write(
        repo.join(".domux/worktree.conf"),
        "# nothing to do here yet\n\n",
    )
    .unwrap();
    let mut h = Harness::start(Config::default(), 120, 24).await;
    h.api("project.add", json!({"path": repo.to_str().unwrap()}))
        .await
        .unwrap();

    let made = h
        .api("workspace.create", json!({"project": "audrey-app"}))
        .await
        .unwrap();
    assert_eq!(
        made["setup"], "",
        "the file is there and it did nothing, which is not the same as having no file"
    );
    assert!(
        !made["setup"].is_null(),
        "and absent is what a project with no worktree.conf answers"
    );
}

/// `ran` counts the lines that were really typed, so a pane whose process never started
/// reports nothing ran rather than a number nobody could have seen (principle 4).
///
/// The input is one a reader reaches with a `terminal.shell` that cannot start: `spawn_pane`
/// logs the failure and carries on, so the workspace and its tab exist and the pane has no
/// runtime behind it. Every other fixture here has `ran` equal to the number of run lines, so
/// this is the only one that separates "typed" from "asked for".
#[tokio::test]
async fn a_run_line_that_could_not_be_typed_is_not_counted_as_having_run() {
    let (_tmp, repo) = repo_with_origin("main");
    std::fs::create_dir_all(repo.join(".domux")).unwrap();
    std::fs::write(
        repo.join(".domux/worktree.conf"),
        "copy setup.cfg\nrun bin/setup --fast\n",
    )
    .unwrap();
    std::fs::write(repo.join("setup.cfg"), "x=1").unwrap();
    let mut h = Harness::start(Config::default(), 120, 24).await;
    h.api("project.add", json!({"path": repo.to_str().unwrap()}))
        .await
        .unwrap();
    // From here no pane gets a process, which is what a shell that cannot start looks like.
    h.spawner
        .as_ref()
        .expect("the fake spawner")
        .refuse_spawns();

    let made = h
        .api("workspace.create", json!({"project": "audrey-app"}))
        .await
        .unwrap();
    assert_eq!(
        made["setup"], "copied 1",
        "the copy happened and the run line did not: nothing typed it"
    );
    let pane = h.first_pane_of(made["id"].as_str().unwrap()).await;
    assert!(
        h.pane_input(&pane).is_empty(),
        "and nothing reached the pane"
    );
    assert!(
        !h.pane_is_running(&pane),
        "which is because it has no process, the condition this test is about"
    );
}

/// A create that fails says so on the screen too, in a red pill (interface spec 7.3).
///
/// The green half is the test above it; this is the half that gives `ok: false` a writer. The
/// colour is the assertion that matters, because a pill that reported a failure in green would
/// carry the same words.
#[tokio::test]
async fn a_create_that_fails_puts_a_red_pill_in_the_hint_row() {
    let dir = tempfile::tempdir().unwrap();
    support::git(dir.path(), &["init", "-q", "-b", "main"]);
    support::git(dir.path(), &["config", "user.email", "t@example.com"]);
    support::git(dir.path(), &["config", "user.name", "t"]);
    support::commit(dir.path(), "README.md", "hi\n");
    let name = dir
        .path()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let mut h = Harness::start(Config::default(), 120, 24).await;
    h.api("project.add", json!({"path": dir.path().to_str().unwrap()}))
        .await
        .unwrap();
    h.api("sidebar.show", json!({})).await.unwrap();
    h.api("workspace.create", json!({"project": name}))
        .await
        .unwrap_err();

    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("git fetch"),
            Duration::from_secs(5),
        )
        .await;
    // Catppuccin base on red: the refusal pill of interface spec 7.3 and the theme table.
    assert!(
        f.contains("bold fg=#1e1e2e bg=#f38ba8"),
        "and it is red, not the green a result gets:\n{f}"
    );
}

/// The two-slot helper builds what it says: two slots the server itself made, in slot order,
/// with the model and the disk agreeing. Tasks 14, 15, 18, 19 and 20 all start from it, so a
/// helper that handed back the ids the other way round, or a model a create could not have
/// produced, would seed every one of them wrong.
#[tokio::test]
async fn the_two_slot_helper_builds_two_slots_that_agree_on_disk_and_in_the_model() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let (root, first, second) = h.git_project_with_two_slots().await;
    assert_eq!(handles(&h, &root), ["main", "workspace-1", "workspace-2"]);
    let handle_of = |id: &WorkspaceId| {
        h.model()
            .workspace(id)
            .map(|w| w.handle.to_string())
            .expect("the helper's workspace is in the model")
    };
    assert_eq!(handle_of(&first), "workspace-1");
    assert_eq!(handle_of(&second), "workspace-2");
    assert_eq!(
        git::existing_slots(&root.canonicalize().unwrap()).unwrap(),
        vec![1, 2],
        "and both worktrees are on disk"
    );
}

/// `[worktrees] base` decides what a slot branches from, and the call's own `base` beats it.
///
/// The two bases hold different commits, so the answer is not the only thing that moves: a
/// create that reported `origin/release` and branched from `origin/main` would fail on the
/// file.
#[tokio::test]
async fn create_branches_from_the_configured_base_and_the_call_can_name_another() {
    let (_tmp, repo) = repo_with_origin("main");
    support::git(&repo, &["checkout", "-q", "-b", "release"]);
    support::commit(&repo, "release-only.md", "released\n");
    support::git(&repo, &["push", "-q", "origin", "release"]);
    support::git(&repo, &["checkout", "-q", "main"]);
    support::git(&repo, &["checkout", "-q", "-b", "spike"]);
    support::commit(&repo, "spike-only.md", "spiked\n");
    support::git(&repo, &["push", "-q", "origin", "spike"]);
    support::git(&repo, &["checkout", "-q", "main"]);
    let config = Config {
        worktrees: WorktreesConfig {
            base: Some("origin/release".into()),
        },
        ..Config::default()
    };
    let mut h = Harness::start(config, 120, 24).await;
    h.api("project.add", json!({"path": repo.to_str().unwrap()}))
        .await
        .unwrap();

    let made = h
        .api("workspace.create", json!({"project": "audrey-app"}))
        .await
        .unwrap();
    assert_eq!(made["base"], "origin/release");
    let first = PathBuf::from(made["path"].as_str().unwrap());
    assert!(
        first.join("release-only.md").is_file(),
        "and that is the commit it checked out"
    );

    let named = h
        .api(
            "workspace.create",
            json!({"project": "audrey-app", "base": "origin/spike"}),
        )
        .await
        .unwrap();
    assert_eq!(named["base"], "origin/spike", "the call beats the setting");
    let second = PathBuf::from(named["path"].as_str().unwrap());
    assert!(second.join("spike-only.md").is_file());
    assert!(!second.join("release-only.md").exists());
}

/// A project that is not a repository is refused, and refused before anything happens: no
/// directory, no branch, no record. A handler that refused after `git worktree add` would
/// answer with this same error code.
#[tokio::test]
async fn creating_in_a_plain_folder_is_refused_and_makes_no_worktree() {
    let folder = tempfile::tempdir().unwrap();
    let name = folder
        .path()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let mut h = Harness::start(Config::default(), 120, 24).await;
    h.api(
        "project.add",
        json!({"path": folder.path().to_str().unwrap()}),
    )
    .await
    .unwrap();

    let err = h
        .api("workspace.create", json!({"project": name}))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Refused);
    assert!(
        err.message.starts_with("a plain folder has only main"),
        "{}",
        err.message
    );
    assert!(
        err.message.contains(folder.path().to_str().unwrap())
            || err
                .message
                .contains(folder.path().canonicalize().unwrap().to_str().unwrap()),
        "and it says which folder: {}",
        err.message
    );
    assert_eq!(handles(&h, folder.path()), ["main"], "no record was made");
    // Worth asserting and worth not leaning on: this half cannot fail even with the guard
    // moved into the job, because a folder that is not a repository fails `git fetch` before
    // `create_dir_all` runs. What carries this test is the code and the first words of the
    // message, which say which guard refused.
    assert!(
        !folder.path().join(".domux").exists(),
        "and nothing was created on disk"
    );
}

/// A create whose git work fails says why in git's own words, adds no record, and leaves no
/// directory behind.
///
/// The repository has no remote, so `git fetch` is the step that fails, and it fails before
/// `git worktree add` has made anything.
#[tokio::test]
async fn a_create_that_fails_leaves_no_half_made_workspace_and_says_why() {
    let dir = tempfile::tempdir().unwrap();
    support::git(dir.path(), &["init", "-q", "-b", "main"]);
    support::git(dir.path(), &["config", "user.email", "t@example.com"]);
    support::git(dir.path(), &["config", "user.name", "t"]);
    support::commit(dir.path(), "README.md", "hi\n");
    let name = dir
        .path()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let mut h = Harness::start(Config::default(), 120, 24).await;
    h.api("project.add", json!({"path": dir.path().to_str().unwrap()}))
        .await
        .unwrap();

    let err = h
        .api("workspace.create", json!({"project": name}))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Unavailable);
    assert!(
        err.message.contains("git fetch"),
        "the reason is git's own: {}",
        err.message
    );
    assert_eq!(
        handles(&h, dir.path()),
        ["main"],
        "only main; the slot record was never made"
    );
    assert!(
        !dir.path().join(".domux/worktrees/workspace-1").exists(),
        "and no directory was left behind"
    );
}

/// A `worktree.conf` that is there and cannot be read fails the create and takes the worktree
/// and its branch back off disk.
///
/// Carrying on would build a slot without the files its setup names and report that as a
/// success. This is the one failure that happens after `git worktree add` has worked, so it
/// is the one that proves the rollback: without it the directory and the branch stay, with
/// nothing in the model pointing at them.
#[tokio::test]
async fn a_worktree_conf_that_cannot_be_read_fails_the_create_and_takes_the_worktree_back() {
    let (_tmp, repo) = repo_with_origin("main");
    // A directory where the file should be fails the read the way an unreadable file does,
    // and needs no permission bits to arrange.
    std::fs::create_dir_all(repo.join(".domux/worktree.conf")).unwrap();
    let mut h = Harness::start(Config::default(), 120, 24).await;
    h.api("project.add", json!({"path": repo.to_str().unwrap()}))
        .await
        .unwrap();

    let err = h
        .api("workspace.create", json!({"project": "audrey-app"}))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Internal);
    assert!(
        err.message.contains(".domux/worktree.conf"),
        "it names the file it could not read: {}",
        err.message
    );
    assert_eq!(handles(&h, &repo), ["main"]);
    let canonical = repo.canonicalize().unwrap();
    assert!(
        !canonical.join(".domux/worktrees/workspace-1").exists(),
        "the worktree it had already made is gone"
    );
    assert!(
        git::existing_slots(&canonical).unwrap().is_empty(),
        "and git holds no registration for it"
    );
    assert!(
        !support::git(&repo, &["branch", "--list", "workspace-1"]).contains("workspace-1"),
        "and so is the branch"
    );
}

/// Two creates in flight at once make two slots, not one slot and one failure.
///
/// This is the shape decision record 0006 names: the slot number is **chosen** in the
/// handler and only reaches the model when that create's job finishes, so a second call
/// arriving in between reads a model that still says the number is free. Both would then run
/// `git worktree add` for `workspace-1`, and the loser would fail inside git.
///
/// It also shows the core did not stop for the first job: the second handler can only have
/// seen `workspace-1` spoken for while that first job was still running.
///
/// Both requests go over their own sockets through `tokio::join!`, so neither is waiting on
/// the other: `Harness::api` takes `&mut self` and would serialise them into two calls that
/// never overlap, which is a test that proves nothing.
#[tokio::test]
async fn two_creates_in_flight_at_once_take_two_different_slot_numbers() {
    let (_tmp, repo) = repo_with_origin("main");
    let mut h = Harness::start(Config::default(), 120, 24).await;
    h.api("project.add", json!({"path": repo.to_str().unwrap()}))
        .await
        .unwrap();
    let socket = h.socket_path().to_path_buf();
    let params = json!({"project": "audrey-app"});
    let (one, two) = tokio::join!(
        api_at(&socket, "workspace.create", params.clone()),
        api_at(&socket, "workspace.create", params.clone()),
    );
    let one = one.expect("the first create");
    let two = two.expect("the second create");

    let mut made = [
        one["handle"].as_str().unwrap(),
        two["handle"].as_str().unwrap(),
    ];
    made.sort_unstable();
    assert_eq!(made, ["workspace-1", "workspace-2"]);
    assert_eq!(handles(&h, &repo), ["main", "workspace-1", "workspace-2"]);
    let canonical = repo.canonicalize().unwrap();
    assert_eq!(
        git::existing_slots(&canonical).unwrap(),
        vec![1, 2],
        "and both worktrees are really there"
    );
}

/// A create that failed gives its number back, so the next one is `workspace-1` again.
///
/// The number is held only while the job runs. Without the release the model would still say
/// 1 is free and the claim would still say it is taken, so the next create would build
/// `workspace-2` and the project would have no `workspace-1` at all. A second create after a
/// create that *worked* cannot show this: the model holds slot 1 by then, so a leaked claim
/// and a released one give the same answer.
#[tokio::test]
async fn a_create_that_failed_gives_its_slot_number_back() {
    let (tmp, repo) = repo_with_origin("main");
    // Point the remote at nothing so the fetch inside the create fails, then put it back.
    let real_origin = support::git(&repo, &["remote", "get-url", "origin"]);
    support::git(
        &repo,
        &[
            "remote",
            "set-url",
            "origin",
            tmp.path().join("gone.git").to_str().unwrap(),
        ],
    );
    let mut h = Harness::start(Config::default(), 120, 24).await;
    h.api("project.add", json!({"path": repo.to_str().unwrap()}))
        .await
        .unwrap();
    h.api("workspace.create", json!({"project": "audrey-app"}))
        .await
        .expect_err("the fetch fails");

    support::git(&repo, &["remote", "set-url", "origin", &real_origin]);
    let made = h
        .api("workspace.create", json!({"project": "audrey-app"}))
        .await
        .unwrap();
    assert_eq!(made["handle"], "workspace-1");
    assert_eq!(handles(&h, &repo), ["main", "workspace-1"]);
}

/// A create reports `workspace.created` with the handle and the path, and the tab and pane
/// that come with it.
///
/// Nothing else in this file reads the event stream, and the state file cannot stand in for
/// it: `publish_events` treats every event outside a four-name list as structural, so the
/// file is written whether or not these events were ever published.
#[tokio::test]
async fn a_create_reports_workspace_created_and_the_tab_that_came_with_it() {
    let (_tmp, repo) = repo_with_origin("main");
    let mut h = Harness::start(Config::default(), 120, 24).await;
    h.api("project.add", json!({"path": repo.to_str().unwrap()}))
        .await
        .unwrap();
    let mut events = subscribe(h.socket_path(), &["workspace.*", "tab.*", "pane.*"]).await;

    let made = h
        .api("workspace.create", json!({"project": "audrey-app"}))
        .await
        .unwrap();
    let created = next_event(&mut events).await;
    assert_eq!(created["event"], "workspace.created");
    assert_eq!(created["workspace"], made["id"]);
    assert_eq!(created["project"], made["project"]);
    assert_eq!(created["handle"], "workspace-1");
    assert_eq!(created["path"], made["path"]);
    let tab = next_event(&mut events).await;
    assert_eq!(tab["event"], "tab.created");
    assert_eq!(tab["workspace"], made["id"]);
    let pane = next_event(&mut events).await;
    assert_eq!(pane["event"], "pane.spawned");
    assert_eq!(pane["tab"], tab["tab"]);
    assert_eq!(pane["cwd"], made["path"]);
}

/// A create that names no project makes the slot in the project the calling client is looking
/// at, rather than in whichever project the model holds first.
///
/// The harness always registers a folder project of its own and the first client starts in
/// it, so the two answers are different: a create that read the model instead of the view
/// would be refused for a plain folder. `pane.focus` is what moves the client, because it is
/// the one built method that crosses workspaces before `workspace.focus` (Task 19):
/// `tab.select` resolves its target inside the view's own workspace.
#[tokio::test]
async fn a_create_with_no_project_makes_the_slot_where_the_client_is_looking() {
    let (_tmp, repo) = repo_with_origin("main");
    let mut h = Harness::start(Config::default(), 120, 24).await;
    h.api("project.add", json!({"path": repo.to_str().unwrap()}))
        .await
        .unwrap();
    let canonical = repo.canonicalize().unwrap();
    let main_pane = h
        .model()
        .project_at(&canonical)
        .and_then(|p| p.workspaces.first().and_then(|w| w.tabs.first()))
        .map(|t| t.focused.to_string())
        .expect("the repository's main workspace has a tab");
    h.api(
        "pane.focus",
        json!({"pane": main_pane, "client": h.client.to_string()}),
    )
    .await
    .unwrap();

    let made = h
        .api("workspace.create", json!({"client": h.client.to_string()}))
        .await
        .unwrap();
    assert_eq!(made["handle"], "workspace-1");
    assert_eq!(handles(&h, &repo), ["main", "workspace-1"]);
}

/// The create answers on the screen too: a green pill in the hint row, saying what was made.
#[tokio::test]
async fn a_create_puts_a_green_pill_in_the_hint_row() {
    let (_tmp, repo) = repo_with_origin("main");
    let mut h = Harness::start(Config::default(), 120, 24).await;
    h.api("project.add", json!({"path": repo.to_str().unwrap()}))
        .await
        .unwrap();
    h.api("sidebar.show", json!({})).await.unwrap();
    h.api("workspace.create", json!({"project": "audrey-app"}))
        .await
        .unwrap();

    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("Created workspace-1"),
            Duration::from_secs(5),
        )
        .await;
    // Catppuccin base on green, bold: interface spec 7.3's result pill, not a plain hint.
    assert!(
        f.contains("bold fg=#1e1e2e bg=#a6e3a1"),
        "and it is green:\n{f}"
    );
}
