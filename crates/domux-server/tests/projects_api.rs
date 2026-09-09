//! `project.add`, `project.list` and `project.remove`: registering a path, adopting the
//! worktrees beside it, and letting a project go without touching anything on disk.

mod support;

use domux_core::api::ErrorCode;
use domux_core::config::Config;
use domux_core::model::{Focus, RegionKind};
use domux_server::facts::branch::BranchProvider;
use domux_server::git;
use domux_server::testing::{Harness, HarnessOptions};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use support::{api_at, next_event, repo_with_origin, subscribe};

/// Past the 100 ms persist debounce, the same bound `sidebar.rs` uses for the same reason:
/// a regression is a failure in a quarter of a second rather than a test that hangs.
const PAST_THE_DEBOUNCE: Duration = Duration::from_millis(250);

fn state_json(h: &Harness) -> String {
    std::fs::read_to_string(h.state_dir().join("state.json")).expect("state.json exists")
}

fn project_names(projects: &serde_json::Value) -> Vec<String> {
    projects
        .as_array()
        .expect("project.list answers with an array")
        .iter()
        .map(|p| p["name"].as_str().unwrap_or_default().to_string())
        .collect()
}

/// The whole of `project.add` on a repository the author has been using: `main` from the
/// checkout, `origin/HEAD` for the base, and a record for every worktree that is already
/// there.
///
/// The branch provider is registered because the hollow glyph depends on it. A slot is
/// untouched only when its branch equals its handle (`Workspace::is_untouched`), and an
/// unknown branch is not equal to anything (principle 4), so without a provider these rows
/// would draw as live workspaces in teal and the frame assertion below would be about the
/// wrong thing.
#[tokio::test]
async fn adding_a_repository_registers_main_and_adopts_the_worktrees_on_disk() {
    let (_tmp, repo) = repo_with_origin("develop");
    // What the model records is the canonical path, because the job resolves it before
    // anything is registered: on macOS a temp directory reaches here through `/var`, which
    // is a symlink to `/private/var`, and two spellings of one folder would register twice.
    let canonical = repo.canonicalize().unwrap();
    git::worktree_add(
        &repo,
        &git::slot_path(&repo, 1),
        "workspace-1",
        "origin/develop",
    )
    .unwrap();
    git::worktree_add(
        &repo,
        &git::slot_path(&repo, 3),
        "workspace-3",
        "origin/develop",
    )
    .unwrap();
    let mut h = Harness::start_with(HarnessOptions {
        providers: vec![Arc::new(BranchProvider)],
        ..HarnessOptions::new(Config::default(), 120, 24)
    })
    .await;
    let added = h
        .api("project.add", json!({"path": repo.to_str().unwrap()}))
        .await
        .unwrap();
    assert_eq!(added["name"], "audrey-app");
    assert_eq!(added["kind"], "git");
    assert_eq!(added["adopted"], json!(["workspace-1", "workspace-3"]));
    let projects = h.api("project.list", json!({})).await.unwrap();
    let p = projects
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "audrey-app")
        .unwrap()
        .clone();
    assert_eq!(
        p["default_branch"], "develop",
        "the base comes from origin/HEAD"
    );
    assert_eq!(p["workspaces"], 3);
    assert_eq!(p["root"], canonical.to_str().unwrap());
    // The answer names `main`, not whichever workspace happens to be last. On this fixture
    // the two differ: the last one is `workspace-3`.
    let main = h
        .model()
        .projects
        .iter()
        .find(|p| p.name == "audrey-app")
        .and_then(|p| p.workspaces.iter().find(|w| w.handle.to_string() == "main"))
        .map(|w| w.id.to_string())
        .expect("audrey-app has a main workspace");
    assert_eq!(added["workspace"], main);
    // Adding a registered path again adopts nothing here, because there is nothing left to
    // adopt: both worktrees are registered already. `adopted` names what the call registered,
    // not what it found beside the root, so an implementation reporting the second would
    // answer `["workspace-1", "workspace-3"]` and claim to have made records it did not make.
    // A worktree that is on disk and *not* registered is the other case, and
    // `a_worktree_made_after_the_project_was_registered_is_adopted_by_adding_it_again` has it.
    let again = h
        .api("project.add", json!({"path": repo.to_str().unwrap()}))
        .await
        .unwrap();
    assert_eq!(again["project"], added["project"]);
    assert_eq!(
        again["adopted"],
        json!([]),
        "the second add adopted nothing"
    );
    let listed = h.api("project.list", json!({})).await.unwrap();
    let after = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "audrey-app")
        .unwrap()
        .clone();
    assert_eq!(
        after["workspaces"], 3,
        "and registered no second copy of them"
    );
    // Nothing draws the Projects box until a surface holding it is open, and a fresh model
    // starts with the top bar (Task 2).
    h.api("sidebar.show", json!({})).await.unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("◌ workspace-1"),
            Duration::from_secs(5),
        )
        .await;
    assert!(
        f.contains("AUDREY-APP") && f.contains("◌ workspace-3"),
        "and the numbers keep their gap:\n{f}"
    );
    // Slot 2 was never on disk, so nothing invented it.
    assert!(!f.contains("workspace-2"), "{f}");
}

/// A folder is a project with `main` and nothing else, adding a registered path answers
/// with the project it already is, and a path that is not there registers nothing.
#[tokio::test]
async fn a_plain_folder_becomes_a_project_with_main_only_and_adding_it_twice_is_the_same_project() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let before = project_names(&h.api("project.list", json!({})).await.unwrap());
    let added = h
        .api("project.add", json!({"path": dir.path().to_str().unwrap()}))
        .await
        .unwrap();
    assert_eq!(added["kind"], "folder");
    assert_eq!(added["adopted"], json!([]));
    let listed = h.api("project.list", json!({})).await.unwrap();
    let p = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == added["project"])
        .expect("the folder is registered")
        .clone();
    assert_eq!(p["workspaces"], 1, "a folder gets main and no slots");
    assert_eq!(
        p["default_branch"],
        json!(null),
        "a folder has no default branch, and absent is not a guess at main"
    );
    let again = h
        .api("project.add", json!({"path": dir.path().to_str().unwrap()}))
        .await
        .unwrap();
    assert_eq!(
        again["project"], added["project"],
        "adding a registered path answers with the project it already is"
    );
    assert_eq!(
        again["workspace"], added["workspace"],
        "and with the same main workspace, not a second one"
    );
    assert_eq!(
        project_names(&h.api("project.list", json!({})).await.unwrap()).len(),
        before.len() + 1,
        "adding it twice registered one project"
    );

    let missing = h
        .api("project.add", json!({"path": "/nonexistent/place"}))
        .await
        .unwrap_err();
    assert_eq!(missing.code, ErrorCode::NotFound);
    assert_eq!(missing.message, "/nonexistent/place does not exist");
    assert_eq!(
        project_names(&h.api("project.list", json!({})).await.unwrap()).len(),
        before.len() + 1,
        "and the path that is not there registered nothing"
    );
}

/// A worktree that appeared after the project was registered is adopted by adding the path
/// again.
///
/// This is what moving over from V1 looks like: V1 makes `workspace-2` beside a repository V2
/// already holds, and until this the record could never learn about it. `import v1` calls
/// `project.add` for every project it plans, so the import is the reader who meets this
/// first - it looks the slot up in `workspace.list` and reports "is not registered under"
/// when it is not there.
#[tokio::test]
async fn a_worktree_made_after_the_project_was_registered_is_adopted_by_adding_it_again() {
    let (_tmp, repo) = repo_with_origin("main");
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let added = h
        .api("project.add", json!({"path": repo.to_str().unwrap()}))
        .await
        .unwrap();
    assert_eq!(
        added["adopted"],
        json!([]),
        "nothing was beside it yet to adopt"
    );

    git::worktree_add(
        &repo,
        &git::slot_path(&repo, 2),
        "workspace-2",
        "origin/main",
    )
    .unwrap();

    let again = h
        .api("project.add", json!({"path": repo.to_str().unwrap()}))
        .await
        .unwrap();
    assert_eq!(
        again["project"], added["project"],
        "the same project, not a second one"
    );
    assert_eq!(again["adopted"], json!(["workspace-2"]));
    let listed = h.api("project.list", json!({})).await.unwrap();
    let after = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == added["project"])
        .unwrap()
        .clone();
    assert_eq!(after["workspaces"], 2, "main and the slot it just adopted");
    let held = h
        .api("workspace.list", json!({"project": added["project"]}))
        .await
        .unwrap();
    let slot = held
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["handle"] == "workspace-2")
        .expect("workspace-2 is registered")
        .clone();
    assert_eq!(
        slot["path"],
        git::slot_path(&repo, 2)
            .canonicalize()
            .unwrap()
            .to_str()
            .unwrap(),
        "at the path the worktree is really at"
    );
    assert_eq!(
        slot["tabs"], 1,
        "and it has its tab, like every other workspace"
    );
}

/// A folder record at a path that is a repository becomes a git project, with the worktrees
/// beside it adopted.
///
/// Every state file written before decision record 0010 holds this shape, because the seed
/// registered the directory the server started in without asking git: the author's own state
/// held `audrey-app` as a folder at a repository root with four `workspace-N` worktrees on
/// disk and none of them registered. `facts::targets` skips a folder, so the sidebar row had
/// no branch either.
#[tokio::test]
async fn a_folder_record_at_a_repository_becomes_a_git_project_when_it_is_added_again() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("audrey-app");
    std::fs::create_dir_all(&root).unwrap();
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let added = h
        .api("project.add", json!({"path": root.to_str().unwrap()}))
        .await
        .unwrap();
    assert_eq!(added["kind"], "folder", "git had nothing to say about it");

    // The record is now the one the old seed wrote: a folder at a path that is a repository
    // with a worktree beside it.
    domux_server::testing::repo_with_origin_at(&root, "develop");
    git::worktree_add(
        &root,
        &git::slot_path(&root, 1),
        "workspace-1",
        "origin/develop",
    )
    .unwrap();

    let again = h
        .api("project.add", json!({"path": root.to_str().unwrap()}))
        .await
        .unwrap();
    assert_eq!(again["project"], added["project"], "the same project");
    assert_eq!(again["kind"], "git");
    assert_eq!(again["adopted"], json!(["workspace-1"]));
    let listed = h.api("project.list", json!({})).await.unwrap();
    let after = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == added["project"])
        .unwrap()
        .clone();
    assert_eq!(
        after["default_branch"], "develop",
        "read from origin/HEAD, like any other git project"
    );
    assert_eq!(after["workspaces"], 2);
}

/// A path that is a file names the state and what to do about it, and it too registers
/// nothing. Separate from the missing path above so the two refusals cannot be read off one
/// assertion: they are different guards with different advice.
#[tokio::test]
async fn adding_a_file_rather_than_a_folder_says_so_and_registers_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("notes.md");
    std::fs::write(&file, "hello\n").unwrap();
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let before = project_names(&h.api("project.list", json!({})).await.unwrap());
    let err = h
        .api("project.add", json!({"path": file.to_str().unwrap()}))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidParams, "{err}");
    assert!(
        err.message
            .ends_with("is a file; name the folder that holds the project"),
        "{err}"
    );
    assert_eq!(
        project_names(&h.api("project.list", json!({})).await.unwrap()),
        before
    );
}

/// The removal asks first, says what goes and what stays, and leaves the worktrees where
/// they are (interface spec 12.8).
#[tokio::test]
async fn removing_a_project_needs_yes_names_what_goes_and_leaves_the_folder_on_disk() {
    let (_tmp, repo) = repo_with_origin("main");
    git::worktree_add(
        &repo,
        &git::slot_path(&repo, 1),
        "workspace-1",
        "origin/main",
    )
    .unwrap();
    let mut h = Harness::start(Config::default(), 120, 24).await;
    h.api("project.add", json!({"path": repo.to_str().unwrap()}))
        .await
        .unwrap();
    let err = h
        .api("project.remove", json!({"project": "audrey-app"}))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Refused);
    assert_eq!(
        err.message,
        "Remove audrey-app? It has 2 workspaces. The folder and its worktrees stay on disk. Answer with --yes"
    );
    // `needs_confirmation` fills `data` with the same content structured, for a caller that
    // lays it out rather than printing the sentence.
    assert_eq!(
        err.data.as_ref().unwrap()["removes"],
        json!(["the record of audrey-app and its 2 workspaces"])
    );
    assert_eq!(
        err.data.as_ref().unwrap()["keeps"],
        json!([format!(
            "the folder at {} and every worktree under it",
            repo.canonicalize().unwrap().display()
        )])
    );
    assert_eq!(
        h.api("project.list", json!({}))
            .await
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        2,
        "nothing was removed"
    );
    // Every pane under the project, before the records go: `remove_project` takes the tabs
    // and the panes with it without passing through `close_pane`, so nothing else would
    // ever kill these PTYs.
    let panes: Vec<_> = h
        .model()
        .projects
        .iter()
        .filter(|p| p.name == "audrey-app")
        .flat_map(|p| p.workspaces.iter())
        .flat_map(|w| w.tabs.iter())
        .flat_map(|t| t.layout.pane_ids())
        .collect();
    assert!(!panes.is_empty(), "the project has panes to close");
    assert!(
        panes.iter().all(|p| h.pane_is_running(p)),
        "and they are running"
    );
    let ack = h
        .api(
            "project.remove",
            json!({"project": "audrey-app", "yes": true}),
        )
        .await
        .unwrap();
    assert_eq!(ack, json!({"ok": true}), "a removal that worked says so");
    h.frame(h.client.clone()).await;
    assert!(
        panes.iter().all(|p| !h.pane_is_running(p)),
        "and the processes went with the records"
    );
    assert!(
        repo.join(".domux/worktrees/workspace-1").is_dir(),
        "the worktrees stay (interface spec 12.8)"
    );
    assert!(repo.join("README.md").is_file(), "and so does the checkout");
    assert!(
        !project_names(&h.api("project.list", json!({})).await.unwrap())
            .contains(&"audrey-app".to_string())
    );
}

/// A project with one workspace is asked about in the singular. The count is the only thing
/// that moves between this and the two-workspace question above, so a fixture with one of
/// them cannot tell a right grammar from `It has 1 workspaces.`
#[tokio::test]
async fn a_project_with_one_workspace_is_asked_about_in_the_singular() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    // A repository with no worktrees beside it: `main` and nothing else, through the
    // harness helper that registers one.
    h.git_project("main").await;
    let name = "audrey-app";
    let err = h
        .api("project.remove", json!({"project": name}))
        .await
        .unwrap_err();
    assert_eq!(
        err.message,
        format!(
            "Remove {name}? It has 1 workspace. The folder and its worktrees stay on disk. \
             Answer with --yes"
        )
    );
    assert_eq!(
        err.data.as_ref().unwrap()["removes"],
        json!([format!("the record of {name} and its 1 workspace")])
    );
}

/// The refusal is a refusal: no record goes, no process is killed, no frame changes and the
/// state file is not rewritten. An error code alone says none of that - a handler that
/// removed the project and then refused would answer with exactly the same code.
#[tokio::test]
async fn refusing_a_removal_without_yes_leaves_the_model_the_frame_and_the_file_alone() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = Harness::start(Config::default(), 120, 24).await;
    h.api("project.add", json!({"path": dir.path().to_str().unwrap()}))
        .await
        .unwrap();
    let name = folder_name(dir.path());
    // Let the add's own writes finish, so what is on disk below is a settled file rather
    // than one the previous call was still writing.
    tokio::time::sleep(PAST_THE_DEBOUNCE).await;
    let panes_before = h.model().all_pane_ids();
    let frame_before = h.frame(h.client.clone()).await;
    let file_before = state_json(&h);

    let err = h
        .api("project.remove", json!({"project": name}))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Refused, "{err}");
    assert!(err.message.ends_with("Answer with --yes"), "{err}");

    tokio::time::sleep(PAST_THE_DEBOUNCE).await;
    assert_eq!(
        h.model().all_pane_ids(),
        panes_before,
        "no pane was closed by a call that refused"
    );
    for pane in &panes_before {
        // Its runtime is still there: `pane_size` reads the published sizes, and a killed
        // pane has no entry to read.
        h.pane_size(pane);
    }
    assert_eq!(
        h.frame(h.client.clone()).await,
        frame_before,
        "and nothing on the screen moved"
    );
    assert_eq!(state_json(&h), file_before, "and nothing was written");
}

/// Naming a project that is not there is a different refusal from declining to confirm one
/// that is, and it says which.
#[tokio::test]
async fn removing_a_project_that_is_not_registered_says_so() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let err = h
        .api("project.remove", json!({"project": "not-a-project"}))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound, "{err}");
    assert_eq!(
        err.message,
        "no project called not-a-project; run domux2 project list to see them"
    );
}

/// The removal reaches the state file while the server is still running, on its own event.
///
/// `Harness::restart` cannot show this: `Core::shutdown` persists unconditionally, so a
/// "survives a restart" test proves only that stopping writes the file. The other writers
/// are ruled out rather than assumed: no pane changes size, so no `PaneResized` writes it,
/// and the file is checked to have gone quiet before the removal, so the once-a-second tick
/// is not writing it either. The harness has already written the file, so a missing mid-run
/// write leaves the project in it rather than leaving no file at all.
#[tokio::test]
async fn removing_a_project_reaches_the_state_file_while_the_server_is_still_running() {
    let dir = tempfile::tempdir().unwrap();
    let name = folder_name(dir.path());
    let mut h = Harness::start(Config::default(), 120, 24).await;
    h.api("project.add", json!({"path": dir.path().to_str().unwrap()}))
        .await
        .unwrap();
    let pane = h.focused_pane(h.client.clone());
    let size_before = h.pane_size(&pane);
    // Two reads more than a tick apart: equal means nothing else is writing this file, so
    // the write after the removal below has one possible author.
    tokio::time::sleep(PAST_THE_DEBOUNCE).await;
    let quiet = state_json(&h);
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert_eq!(
        state_json(&h),
        quiet,
        "the file went quiet before the removal"
    );
    assert!(quiet.contains(&name), "and it holds the project to remove");

    h.api(
        "project.remove",
        json!({"project": name.clone(), "yes": true}),
    )
    .await
    .unwrap();
    tokio::time::sleep(PAST_THE_DEBOUNCE).await;
    let after = h.pane_size(&pane);
    assert_eq!(
        (after.cols, after.rows),
        (size_before.cols, size_before.rows),
        "no pane resized, so no `PaneResized` wrote the file"
    );
    assert!(
        !state_json(&h).contains(&name),
        "the removal wrote the file while the server was still running"
    );
}

/// Removing the project a client is in seats that client somewhere it can draw, rather than
/// leaving it pointing at a tab the model no longer holds - and it lands on the first
/// surviving workspace's first tab, not on the last one.
///
/// Two projects survive, so "the first" and "the last" are different answers. With one, the
/// landing tab is the only tab and every rule for choosing it is the same rule.
#[tokio::test]
async fn removing_the_project_a_client_is_in_seats_it_on_the_first_project_that_is_left() {
    let first = tempfile::tempdir().unwrap();
    let last = tempfile::tempdir().unwrap();
    let first_name = folder_name(first.path());
    let last_name = folder_name(last.path());
    let mut h = Harness::start(Config::default(), 120, 24).await;
    // Registered in this order, which is the order the model holds them in.
    h.api(
        "project.add",
        json!({"path": first.path().to_str().unwrap()}),
    )
    .await
    .unwrap();
    h.api(
        "project.add",
        json!({"path": last.path().to_str().unwrap()}),
    )
    .await
    .unwrap();
    // The harness's own project, which is the one the client is seated in.
    let f = h.frame(h.client.clone()).await;
    assert!(f.contains("proj › main"), "the client starts in proj:\n{f}");
    let doomed = h.model().client(&h.client).map(|v| v.tab.clone()).unwrap();

    h.api("project.remove", json!({"project": "proj", "yes": true}))
        .await
        .unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains(&format!("{first_name} › main")),
            Duration::from_secs(5),
        )
        .await;
    assert!(
        !f.contains(&format!("{last_name} › main")),
        "it landed in the first project that was left, not the last:\n{f}"
    );
    let view = h
        .model()
        .client(&h.client)
        .cloned()
        .expect("still attached");
    assert_ne!(view.tab, doomed, "the client left the tab that went:\n{f}");
    assert!(
        h.model().tab(&view.tab).is_some(),
        "and the tab it is on is one the model holds"
    );
}

/// Removing a project a client is **not** in leaves that client exactly where it was.
///
/// The reseat picks the clients whose workspace has gone. Without that filter it would take
/// every attached client and drag it to the landing tab, and a reader watching a pane in an
/// untouched project would have the screen pulled out from under them. One client is enough
/// to see it, but only if it is somewhere the landing tab is not, so the client gets a
/// second tab first: "left alone" and "moved to the first tab" are then different answers.
#[tokio::test]
async fn removing_a_project_a_client_is_not_in_leaves_that_client_where_it_was() {
    let dir = tempfile::tempdir().unwrap();
    let name = folder_name(dir.path());
    let mut h = Harness::start(Config::default(), 120, 24).await;
    h.api("project.add", json!({"path": dir.path().to_str().unwrap()}))
        .await
        .unwrap();
    h.api("tab.create", json!({})).await.unwrap();
    let before = h.model().client(&h.client).cloned().expect("attached");
    let landing = h
        .model()
        .projects
        .iter()
        .find(|p| p.name == "proj")
        .and_then(|p| p.workspaces.first())
        .and_then(|w| w.tabs.first())
        .map(|t| t.id.clone())
        .expect("proj has a tab");
    assert_ne!(
        before.tab, landing,
        "the client is on a tab the reseat would move it off"
    );

    h.api("project.remove", json!({"project": name, "yes": true}))
        .await
        .unwrap();
    let f = h.frame(h.client.clone()).await;
    let after = h
        .model()
        .client(&h.client)
        .cloned()
        .expect("still attached");
    assert_eq!(
        (after.tab, after.workspace),
        (before.tab, before.workspace),
        "a client in a project that was not removed does not move:\n{f}"
    );
}

/// The last component of a temp directory's path, which is the name the project takes.
fn folder_name(path: &std::path::Path) -> String {
    path.file_name()
        .expect("a temp directory has a name")
        .to_string_lossy()
        .into_owned()
}

/// A key bound to `project.remove` asks on the screen rather than answering "add --yes" to
/// somebody who has no command line to add it to, and `y` is what removes the project.
#[tokio::test]
async fn a_key_bound_to_project_remove_asks_in_an_overlay_and_y_removes_the_project() {
    let dir = tempfile::tempdir().unwrap();
    let name = folder_name(dir.path());
    let mut config = Config::default();
    config
        .keys
        .bindings
        .insert("X".into(), format!("project.remove {name}"));
    let mut h = Harness::start(config, 120, 24).await;
    h.api("project.add", json!({"path": dir.path().to_str().unwrap()}))
        .await
        .unwrap();

    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), "X").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains(&format!("Remove {name}?")),
            Duration::from_secs(5),
        )
        .await;
    assert!(
        f.contains("The folder and its worktrees stay on disk."),
        "the box says what stays:\n{f}"
    );
    assert!(
        f.contains(&format!(
            "Removes the record of {name} and its 1 workspace."
        )),
        "and what goes:\n{f}"
    );
    assert!(
        f.contains("y remove project    esc keep project"),
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
    assert!(
        f.lines().any(|l| l.starts_with('r') && l.contains(" dim")),
        "and the screen beneath it dims (interface spec 7.1):\n{f}"
    );
    assert!(
        project_names(&h.api("project.list", json!({})).await.unwrap()).contains(&name),
        "asking is not doing"
    );

    h.key(h.client.clone(), "y").await;
    h.wait_for(
        h.client.clone(),
        |f| !f.contains(&format!("Remove {name}?")),
        Duration::from_secs(5),
    )
    .await;
    assert!(
        !project_names(&h.api("project.list", json!({})).await.unwrap()).contains(&name),
        "and y is what removes it"
    );
}

/// Anything other than `y` closes the question and keeps the project: the safe outcome is
/// the one a stray keystroke reaches.
#[tokio::test]
async fn a_key_other_than_y_closes_the_question_and_keeps_the_project() {
    let dir = tempfile::tempdir().unwrap();
    let name = folder_name(dir.path());
    let mut config = Config::default();
    config
        .keys
        .bindings
        .insert("X".into(), format!("project.remove {name}"));
    let mut h = Harness::start(config, 120, 24).await;
    h.api("project.add", json!({"path": dir.path().to_str().unwrap()}))
        .await
        .unwrap();
    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), "X").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains(&format!("Remove {name}?")),
        Duration::from_secs(5),
    )
    .await;
    h.key(h.client.clone(), "Esc").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| !f.contains(&format!("Remove {name}?")),
            Duration::from_secs(5),
        )
        .await;
    assert!(
        project_names(&h.api("project.list", json!({})).await.unwrap()).contains(&name),
        "esc keeps it:\n{f}"
    );
    // And the dimming went with the box. This is also what keeps the dim assertion in the
    // test above from being about nothing: with no overlay open, no frame in these fixtures
    // has a dim cell at all.
    assert!(
        !f.lines().any(|l| l.starts_with('r') && l.contains(" dim")),
        "the screen is not dimmed once the question is gone:\n{f}"
    );
}

/// A worktree V1 left under its old directory name is adopted where it really is.
///
/// `git::existing_slots` reads both directories, so the slot is found either way; what this
/// pins is the path the record gets. Built at the legacy name only, so `git::slot_path`'s
/// answer does not exist and a record built from it would point at nothing.
#[tokio::test]
async fn a_worktree_under_the_legacy_directory_is_adopted_at_the_path_it_is_at() {
    let (_tmp, repo) = repo_with_origin("main");
    let legacy = repo.join(".baag/worktrees/workspace-2");
    git::worktree_add(&repo, &legacy, "workspace-2", "origin/main").unwrap();
    let canonical = repo.canonicalize().unwrap();
    assert!(
        !canonical.join(".domux/worktrees/workspace-2").exists(),
        "the current directory name is not where this worktree is"
    );
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let added = h
        .api("project.add", json!({"path": repo.to_str().unwrap()}))
        .await
        .unwrap();
    assert_eq!(added["adopted"], json!(["workspace-2"]));
    let path = h
        .model()
        .projects
        .iter()
        .find(|p| p.name == "audrey-app")
        .and_then(|p| {
            p.workspaces
                .iter()
                .find(|w| w.handle.to_string() == "workspace-2")
        })
        .map(|w| w.path.clone())
        .expect("workspace-2 is registered");
    assert_eq!(path, canonical.join(".baag/worktrees/workspace-2"));
    assert!(
        path.is_dir(),
        "and that is a directory that is really there"
    );
}

/// Two projects with the same folder name are told apart by id, and a name that matches both
/// answers `ambiguous` with the candidates rather than picking one.
#[tokio::test]
async fn a_name_two_projects_share_is_ambiguous_and_an_id_still_names_one() {
    let one = tempfile::tempdir().unwrap();
    let two = tempfile::tempdir().unwrap();
    let first = one.path().join("audrey-app");
    let second = two.path().join("audrey-app");
    std::fs::create_dir_all(&first).unwrap();
    std::fs::create_dir_all(&second).unwrap();
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let a = h
        .api("project.add", json!({"path": first.to_str().unwrap()}))
        .await
        .unwrap();
    h.api("project.add", json!({"path": second.to_str().unwrap()}))
        .await
        .unwrap();

    let err = h
        .api("project.remove", json!({"project": "audrey-app"}))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Ambiguous, "{err}");
    assert_eq!(
        err.data,
        Some(json!([
            a["project"],
            h.model()
                .projects
                .iter()
                .filter(|p| p.name == "audrey-app")
                .nth(1)
                .unwrap()
                .id
                .to_string()
        ])),
        "{err}"
    );

    let id = a["project"].as_str().unwrap().to_string();
    let refusal = h
        .api("project.remove", json!({"project": id}))
        .await
        .unwrap_err();
    assert_eq!(
        refusal.code,
        ErrorCode::Refused,
        "an id names one of them: {refusal}"
    );
}

/// A job a key started has no caller waiting on it, so its failure lands in the hint row,
/// where a failed key's message goes (principle 8). Without that the key would answer with
/// nothing at all.
#[tokio::test]
async fn a_job_a_key_started_reports_its_failure_in_the_hint_row() {
    let mut config = Config::default();
    config
        .keys
        .bindings
        .insert("A".into(), "project.add /nonexistent/place".into());
    let mut h = Harness::start(config, 120, 24).await;
    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), "A").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("/nonexistent/place does not exist"),
            Duration::from_secs(5),
        )
        .await;
    assert!(
        f.contains("proj › main"),
        "and the screen is otherwise itself:\n{f}"
    );
}

/// Two `project.add` calls for one unregistered path, both in flight, register one project
/// and both get the same one back.
///
/// The window this looks for is real in shape: the check that makes `project.add`
/// idempotent runs when the job finishes, not when the handler queues it, so two handlers
/// can both queue a job for a path no project holds yet. What closes it is that the two
/// `JobFinished` arms run on the core task, one after the other, so the second arm sees the
/// project the first one registered.
///
/// Both requests go over their own sockets through `tokio::join!`, so neither is waiting on
/// the other: `Harness::api` takes `&mut self` and would serialise them into two calls that
/// never overlap, which is a test that proves nothing.
#[tokio::test]
async fn two_project_add_calls_in_flight_at_once_register_one_project() {
    let dir = tempfile::tempdir().unwrap();
    let name = folder_name(dir.path());
    let h = Harness::start(Config::default(), 120, 24).await;
    let socket = h.socket_path().to_path_buf();
    let params = json!({"path": dir.path().to_str().unwrap()});
    let (one, two) = tokio::join!(
        api_at(&socket, "project.add", params.clone()),
        api_at(&socket, "project.add", params.clone()),
    );
    let one = one.expect("the first add");
    let two = two.expect("the second add");
    assert_eq!(
        one["project"], two["project"],
        "both calls name one project"
    );
    let registered: Vec<_> = h
        .model()
        .projects
        .iter()
        .filter(|p| p.name == name)
        .map(|p| p.id.clone())
        .collect();
    assert_eq!(
        registered.len(),
        1,
        "and the model holds one: {registered:?}"
    );
}

/// `project.add` reports `project.added` once and `workspace.created` for every worktree it
/// adopts, and `project.remove` reports `project.removed`.
///
/// Nothing else in the suite reads the event stream, and the state file cannot stand in for
/// it: `publish_events` treats every event outside a four-name list as structural, and
/// registering a project raises `tab.created` in the same batch, so the file would be
/// written with the right contents whether or not these events were ever published.
#[tokio::test]
async fn adding_and_removing_a_project_report_what_they_did() {
    let (_tmp, repo) = repo_with_origin("main");
    git::worktree_add(
        &repo,
        &git::slot_path(&repo, 1),
        "workspace-1",
        "origin/main",
    )
    .unwrap();
    git::worktree_add(
        &repo,
        &git::slot_path(&repo, 2),
        "workspace-2",
        "origin/main",
    )
    .unwrap();
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let mut events = subscribe(h.socket_path(), &["project.*", "workspace.*"]).await;

    h.api("project.add", json!({"path": repo.to_str().unwrap()}))
        .await
        .unwrap();
    let added = next_event(&mut events).await;
    assert_eq!(added["event"], "project.added");
    assert_eq!(added["name"], "audrey-app");
    assert_eq!(
        added["root"],
        repo.canonicalize().unwrap().to_str().unwrap()
    );
    // One `workspace.created` per adopted slot, in slot order. `main` is not one: it is the
    // checkout the project already is, and `add_git_project` reports the project, not it.
    let mut adopted = Vec::new();
    for _ in 0..2 {
        let created = next_event(&mut events).await;
        assert_eq!(created["event"], "workspace.created");
        assert_eq!(created["project"], added["project"]);
        adopted.push(created["handle"].as_str().unwrap().to_string());
    }
    assert_eq!(adopted, ["workspace-1", "workspace-2"]);

    h.api(
        "project.remove",
        json!({"project": "audrey-app", "yes": true}),
    )
    .await
    .unwrap();
    let removed = next_event(&mut events).await;
    assert_eq!(removed["event"], "project.removed");
    assert_eq!(removed["project"], added["project"]);
    assert_eq!(removed["name"], "audrey-app");
}

/// A plain folder reports `project.added` too. `Model::add_folder_project` is M1's and
/// reports nothing, so this is the branch that raises the event by hand; the repository
/// above goes through `add_git_project`, which raises its own.
#[tokio::test]
async fn adding_a_plain_folder_reports_project_added() {
    let dir = tempfile::tempdir().unwrap();
    let name = folder_name(dir.path());
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let mut events = subscribe(h.socket_path(), &["project.*"]).await;
    h.api("project.add", json!({"path": dir.path().to_str().unwrap()}))
        .await
        .unwrap();
    let added = next_event(&mut events).await;
    assert_eq!(added["event"], "project.added");
    assert_eq!(added["name"], name);
    assert_eq!(
        added["root"],
        dir.path().canonicalize().unwrap().to_str().unwrap()
    );
}

/// A worktree directory that is there and cannot be read refuses the whole add, rather than
/// answering "no slots".
///
/// `git::existing_slots`'s own doc comment is about this: answering "no slots" hands out a
/// slot number that is already taken, and `workspace.create` then runs `git branch -f`,
/// which moves that branch off the author's commit before `git worktree add` finds the
/// directory in the way. The commit is then in the reflog and nowhere else. A file where
/// the directory should be is the input this test uses, because it fails the same way a
/// directory nobody can read does and it needs no permission bits to arrange.
#[tokio::test]
async fn a_worktree_directory_that_cannot_be_read_refuses_the_add() {
    let (_tmp, repo) = repo_with_origin("main");
    std::fs::create_dir_all(repo.join(".domux")).unwrap();
    std::fs::write(repo.join(".domux/worktrees"), "not a directory\n").unwrap();
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let before = project_names(&h.api("project.list", json!({})).await.unwrap());
    let err = h
        .api("project.add", json!({"path": repo.to_str().unwrap()}))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Internal, "{err}");
    assert!(
        err.message.contains(".domux/worktrees"),
        "the refusal names the directory it could not read: {err}"
    );
    assert_eq!(
        project_names(&h.api("project.list", json!({})).await.unwrap()),
        before,
        "and nothing was registered"
    );
}

/// The confirmation names where the project is, which is the only line that tells two
/// projects of the same name apart - a state this task made real, since `resolve_project`
/// now answers `ambiguous` for it.
#[tokio::test]
async fn the_confirmation_names_the_project_s_root_so_two_of_one_name_are_told_apart() {
    let one = tempfile::tempdir().unwrap();
    let two = tempfile::tempdir().unwrap();
    let first = one.path().join("audrey-app");
    let second = two.path().join("audrey-app");
    std::fs::create_dir_all(&first).unwrap();
    std::fs::create_dir_all(&second).unwrap();
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let added = h
        .api("project.add", json!({"path": first.to_str().unwrap()}))
        .await
        .unwrap();
    h.api("project.add", json!({"path": second.to_str().unwrap()}))
        .await
        .unwrap();
    // The name is ambiguous, so the key has to name the id - which is only known now, after
    // the add. `config.reload` is how a binding is written after the server has started.
    let id = added["project"].as_str().unwrap().to_string();
    std::fs::write(
        h.config_path(),
        format!("[keys.bindings]\nX = \"project.remove {id}\"\n"),
    )
    .unwrap();
    h.api("config.reload", json!({})).await.unwrap();

    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), "X").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("Remove audrey-app?"),
            Duration::from_secs(5),
        )
        .await;
    let first_root = first.canonicalize().unwrap();
    let second_root = second.canonicalize().unwrap();
    assert!(
        f.contains(first_root.to_str().unwrap()),
        "the box says which audrey-app:\n{f}"
    );
    assert!(
        !f.contains(second_root.to_str().unwrap()),
        "and not the other one:\n{f}"
    );
}

/// `--all` is the way back to an empty domux, for an author who wants to register everything
/// again from scratch rather than remove one project at a time.
#[tokio::test]
async fn removing_every_project_leaves_the_model_empty_and_the_folders_alone() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let other = tempfile::tempdir().unwrap();
    h.api(
        "project.add",
        json!({"path": other.path().to_str().unwrap()}),
    )
    .await
    .unwrap();
    assert_eq!(h.model().projects.len(), 2);

    h.api("project.remove", json!({"all": true, "yes": true}))
        .await
        .unwrap();
    assert!(
        h.model().projects.is_empty(),
        "every project's records are gone"
    );
    assert!(
        other.path().is_dir(),
        "the folders it registered are still on disk"
    );
    h.stop().await;
}

/// The same consent every other destructive call asks for, and the same shape: the question,
/// what goes, what stays.
#[tokio::test]
async fn removing_every_project_asks_first_and_changes_nothing_when_it_does() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let refused = h
        .api("project.remove", json!({"all": true}))
        .await
        .expect_err("it asks first");
    assert_eq!(refused.code, ErrorCode::Refused);
    assert!(
        refused
            .message
            .starts_with("Remove every project? There is 1 project: proj."),
        "{}",
        refused.message
    );
    assert!(
        refused
            .message
            .contains("The folder and its worktrees stay on disk."),
        "{}",
        refused.message
    );
    assert_eq!(
        h.model().projects.len(),
        1,
        "a refusal leaves the model as it found it"
    );
    h.stop().await;
}

/// Removing everything twice is not an error the second time. The caller asked for no
/// projects and there are none, which is the state they asked for.
#[tokio::test]
async fn removing_every_project_when_there_are_none_is_quiet() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    h.api("project.remove", json!({"all": true, "yes": true}))
        .await
        .unwrap();
    h.api("project.remove", json!({"all": true, "yes": true}))
        .await
        .expect("nothing to remove is not a failure");
    h.stop().await;
}

/// A removal that names nothing and asks for nothing is a mistake, not "remove everything".
#[tokio::test]
async fn a_removal_with_no_target_and_no_all_names_both_ways_to_ask() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let refused = h
        .api("project.remove", json!({}))
        .await
        .expect_err("it names no project");
    assert_eq!(refused.code, ErrorCode::InvalidParams);
    assert!(
        refused.message.contains("--all"),
        "it names the other way to ask: {}",
        refused.message
    );
    assert_eq!(h.model().projects.len(), 1);
    h.stop().await;
}

/// A client whose workspace has just gone has nowhere to be moved to, and a later attach has
/// nothing to seat a client on. The refusal names the way out rather than stating the state
/// and stopping.
#[tokio::test]
async fn attaching_to_a_server_with_no_project_says_how_to_add_one() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    h.api("project.remove", json!({"all": true, "yes": true}))
        .await
        .unwrap();
    let refused = h.attach_refusal(80, 24).await;
    assert_eq!(
        refused,
        "the server holds no project; run domux2 open <path> to add one"
    );
    h.stop().await;
}

/// The keys are in the sidebar's Projects box, with the cursor on the row this client is in.
async fn in_the_sidebar_box(h: &mut Harness) {
    h.api("sidebar.show", json!({})).await.unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains("Projects"),
        Duration::from_secs(2),
    )
    .await;
    h.key(h.client.clone(), "C-h").await;
    h.frame(h.client.clone()).await;
}

/// `X` in the Projects box removes the project of the row under the cursor.
///
/// The binding carries no project. `leader`-bound `project.remove audrey-app` names one and
/// has its own tests above; what this pins is the target a key in a box has and a shell does
/// not, which is the row the reader can see the fill on (interface spec 7.3).
#[tokio::test]
async fn x_in_the_projects_box_asks_about_the_project_the_cursor_is_in_and_y_removes_it() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let (_root, w1, _w2) = h.git_project_with_two_slots().await;
    let name = h
        .model()
        .project_of_workspace(&w1)
        .map(|p| p.name.clone())
        .expect("the slot is in a project");
    h.api("workspace.focus", json!({"workspace": w1.as_str()}))
        .await
        .unwrap();
    in_the_sidebar_box(&mut h).await;

    h.key(h.client.clone(), "X").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains(&format!("Remove {name}?")),
            Duration::from_secs(5),
        )
        .await;
    assert_eq!(
        h.model().client(&h.client).map(|v| v.focus.clone()),
        Some(Focus::Region(RegionKind::Overlay)),
        "the keys are in the question, not still in the box:\n{f}"
    );
    assert!(
        project_names(&h.api("project.list", json!({})).await.unwrap()).contains(&name),
        "asking is not doing"
    );

    h.key(h.client.clone(), "y").await;
    h.wait_for(
        h.client.clone(),
        |f| !f.contains(&format!("Remove {name}?")),
        Duration::from_secs(5),
    )
    .await;
    assert!(
        !project_names(&h.api("project.list", json!({})).await.unwrap()).contains(&name),
        "and y removes the project the cursor was in"
    );
    assert_eq!(
        h.model()
            .client(&h.client)
            .and_then(|v| v.projects_cursor.clone()),
        None,
        "the cursor is not left on a workspace that has gone, which would draw no fill at all"
    );
    h.stop().await;
}

/// The same key in the switcher, which shares the sidebar's `[keys.list]` table.
#[tokio::test]
async fn x_in_the_switcher_asks_the_same_question_over_the_overlay_it_was_pressed_in() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let (_root, w1, _w2) = h.git_project_with_two_slots().await;
    let name = h
        .model()
        .project_of_workspace(&w1)
        .map(|p| p.name.clone())
        .expect("the slot is in a project");
    h.api("workspace.focus", json!({"workspace": w1.as_str()}))
        .await
        .unwrap();
    h.api("switcher.open", json!({})).await.unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains("Projects"),
        Duration::from_secs(2),
    )
    .await;

    h.key(h.client.clone(), "X").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains(&format!("Remove {name}?")),
        Duration::from_secs(5),
    )
    .await;
    h.key(h.client.clone(), "Esc").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| !f.contains(&format!("Remove {name}?")),
            Duration::from_secs(5),
        )
        .await;
    assert!(
        f.contains("esc close"),
        "esc closes the question and uncovers the switcher it was asked over:\n{f}"
    );
    assert!(
        project_names(&h.api("project.list", json!({})).await.unwrap()).contains(&name),
        "and the project is still there"
    );
    h.stop().await;
}
