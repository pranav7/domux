//! `workspace.focus`, `workspace.list`, the half of `workspace.rename` that takes a name,
//! `workspace.clear_name` and the `workspace.resume` stub.
//!
//! These call the handlers through the API. Task 14 wires `Enter` in the Projects box to
//! `workspace.focus` and asserts the same outcomes through a key press; there is one handler
//! under both, so what is pinned here is what the key does.

mod support;

use domux_core::api::ErrorCode;
use domux_core::config::Config;
use domux_core::facts::{Fact, FactKey, FactState, FACT_BRANCH, FACT_PR};
use domux_core::ids::WorkspaceId;
use domux_core::model::{Focus, RegionKind};
use domux_server::facts::branch::BranchProvider;
use domux_server::facts::{FactProvider, FactTarget, ProviderScope};
use domux_server::git;
use domux_server::testing::{row, Harness, HarnessOptions};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use support::repo_with_origin;

/// A pull request for one workspace and none for the other, so `pr` and `pr_state` each have
/// a row that could only have come from the fact and a row where the fact is absent.
///
/// A fixture where every workspace had a pull request could not tell "reads the fact" from
/// "reports the same string for everyone", and one where none did could not tell `null` from
/// a value the code never reaches. `workspace-2` is the one with the pull request, so the
/// row the other assertions are about is the row without one.
struct OnePrProvider;

impl FactProvider for OnePrProvider {
    fn name(&self) -> &str {
        FACT_PR
    }

    fn interval(&self) -> Duration {
        Duration::from_millis(50)
    }

    fn scope(&self) -> ProviderScope {
        ProviderScope::Workspace
    }

    fn fetch(&self, target: &FactTarget) -> Result<Option<Fact>, String> {
        if !target.path.ends_with("workspace-2") {
            return Ok(None);
        }
        Ok(Some(Fact::new(
            "PR#212",
            Some(FactState::Draft),
            target.now.to_rfc3339(),
            Duration::from_secs(600),
        )))
    }
}

/// The pane the keys would land in if this client switched to `workspace`, read before the
/// switch so the assertion after it names a pane that is not the one the client is on.
fn landing_pane(h: &Harness, workspace: &WorkspaceId) -> Focus {
    let m = h.model();
    let w = m
        .workspace(workspace)
        .expect("the workspace is in the model");
    let tab = w
        .last_tab
        .clone()
        .or_else(|| w.tabs.first().map(|t| t.id.clone()))
        .expect("the workspace has a tab");
    Focus::Pane(
        m.tab(&tab)
            .expect("the tab is in the model")
            .focused
            .clone(),
    )
}

#[tokio::test]
async fn focusing_a_workspace_from_the_switcher_switches_it_and_closes_the_switcher() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (_root, w1, _w2) = h.git_project_with_two_slots().await;
    let client = h.client.clone();
    h.api("switcher.open", json!({"client": client.as_str()}))
        .await
        .unwrap();
    h.wait_for(
        client.clone(),
        |f| f.contains("Projects"),
        Duration::from_secs(2),
    )
    .await;
    // The keys are in the switcher and the client is in another project's `main`, so every
    // assertion below is about a value the call had to change rather than one it left alone.
    assert_eq!(
        h.model().client(&client).unwrap().focus,
        Focus::Region(RegionKind::Switcher)
    );
    assert_ne!(h.model().client(&client).unwrap().workspace, w1);
    let landing = landing_pane(&h, &w1);

    h.api(
        "workspace.focus",
        json!({"workspace": w1.as_str(), "client": client.as_str()}),
    )
    .await
    .unwrap();
    let f = h
        .wait_for(
            client.clone(),
            |f| !f.contains("┌ Projects"),
            Duration::from_secs(3),
        )
        .await;

    assert_eq!(h.model().client(&client).unwrap().workspace, w1);
    assert_eq!(
        h.model().client(&client).unwrap().overlay,
        None,
        "the switcher closes (interface spec 5.4)"
    );
    // `testing::row` returns the whole framed line: "|" + 80 terminal cells + "|" = 82.
    // 82 = "|" (1) + " audrey-app › workspace-1  1 │ + │" (34) + 28 spaces
    //      + "14:32   Fri 4 Sep " (18) + "|" (1)
    assert_eq!(
        row(&f, 0),
        "| audrey-app › workspace-1  1 │ + │                            14:32   Fri 4 Sep |",
        "{f}"
    );
    assert_eq!(
        h.model().client(&client).unwrap().focus,
        landing,
        "the keys go to the target's own focused pane, not to the one the client was on"
    );
    assert_eq!(h.model().last_workspace, Some(w1));
}

/// Closing the switcher hands the keys back to what was underneath, which is the pane unless
/// the switcher was opened over another overlay (interface spec 12.7). The help overlay is
/// still drawn after this switch, so the keys must stay in it: a frame with the keys in a
/// region nothing on the screen marks is what principle 2 forbids.
#[tokio::test]
async fn focusing_from_a_switcher_opened_over_an_overlay_leaves_the_keys_in_that_overlay() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (_root, w1, _w2) = h.git_project_with_two_slots().await;
    let client = h.client.clone();
    h.api("help", json!({"client": client.as_str()}))
        .await
        .unwrap();
    h.api("switcher.open", json!({"client": client.as_str()}))
        .await
        .unwrap();

    h.api(
        "workspace.focus",
        json!({"workspace": w1.as_str(), "client": client.as_str()}),
    )
    .await
    .unwrap();
    let _ = h.frame(client.clone()).await;

    let m = h.model();
    let view = m.client(&client).unwrap();
    assert_eq!(view.workspace, w1, "the switch still happened");
    assert_eq!(
        view.overlay,
        Some(domux_core::model::Overlay::Help),
        "one overlay closed, not both"
    );
    assert_eq!(view.focus, Focus::Region(RegionKind::Overlay));
}

/// The switcher closes, and **only** the switcher. An overlay the reader opened for something
/// else is not what switching was asked for, so it stays.
///
/// This is the far side of the mutant the test above is on the near side of. That one opens
/// the switcher over the help overlay, so a handler treating any overlay as the switcher
/// behaves identically there: the switcher is what is open. Here there is no switcher at all,
/// and a handler that popped whatever was open would close the help overlay and hand the keys
/// to the pane.
#[tokio::test]
async fn focusing_a_workspace_with_a_help_overlay_open_leaves_the_overlay_open() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (_root, w1, _w2) = h.git_project_with_two_slots().await;
    let client = h.client.clone();
    h.api("help", json!({"client": client.as_str()}))
        .await
        .unwrap();
    let _ = h.frame(client.clone()).await;
    assert_eq!(
        h.model().client(&client).unwrap().overlay,
        Some(domux_core::model::Overlay::Help),
        "the fixture holds an overlay that is not the switcher"
    );

    h.api(
        "workspace.focus",
        json!({"workspace": w1.as_str(), "client": client.as_str()}),
    )
    .await
    .unwrap();
    let _ = h.frame(client.clone()).await;

    let m = h.model();
    let view = m.client(&client).unwrap();
    assert_eq!(view.workspace, w1, "the switch still happened");
    assert_eq!(
        view.overlay,
        Some(domux_core::model::Overlay::Help),
        "and the overlay the reader opened is still open"
    );
    assert_eq!(
        view.focus,
        Focus::Region(RegionKind::Overlay),
        "so the keys are still in it"
    );
}

#[tokio::test]
async fn focusing_a_workspace_with_the_sidebar_open_keeps_it_open_and_moves_the_fill() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let (_root, w1, _w2) = h.git_project_with_two_slots().await;
    let client = h.client.clone();
    h.api("sidebar.show", json!({"client": client.as_str()}))
        .await
        .unwrap();
    let before = h
        .wait_for(
            client.clone(),
            |f| f.contains("Projects"),
            Duration::from_secs(2),
        )
        .await;
    // `workspace-1` is the third row of the box: the header, `main`, a blank line, then this
    // one. The fill there is two style runs rather than one, because the handle standing in
    // the name position brightens and the rest of the row only takes the band. It is not
    // there before the switch, so the assertion after it is about a row the call filled.
    assert!(
        !before.contains("r4 c12-36 bg=#313244"),
        "the fill starts on the row the client is in, which is not this one:\n{before}"
    );
    let landing = landing_pane(&h, &w1);

    h.api(
        "workspace.focus",
        json!({"workspace": w1.as_str(), "client": client.as_str()}),
    )
    .await
    .unwrap();
    let f = h
        .wait_for(
            client.clone(),
            |f| f.contains("r4 c12-36 bg=#313244"),
            Duration::from_secs(3),
        )
        .await;

    assert!(
        f.contains("r4 c1-11 bold fg=#93e2d5 bg=#313244"),
        "the filled row's handle brightens (interface spec 5.3):\n{f}"
    );
    assert!(
        f.contains("┌ Projects"),
        "the sidebar stays open (interface spec 12.26):\n{f}"
    );
    assert!(
        h.model().sidebar_open,
        "and the remembered state is untouched"
    );
    assert_eq!(h.model().client(&client).unwrap().workspace, w1);
    assert_eq!(
        h.model().client(&client).unwrap().projects_cursor,
        Some(w1),
        "the fill follows the workspace"
    );
    assert_eq!(
        h.model().client(&client).unwrap().focus,
        landing,
        "and the keys go to the target's pane"
    );
}

/// Switching lands somewhere you can type: a tab, with a running shell, in that workspace's
/// own worktree rather than in the project root or in the pane the client came from.
///
/// The fixture is a worktree that no domux made, adopted by `project.add`, so the workspace
/// has never been shown and nothing about it was arranged by the switch.
///
/// **What made the tab is not what this test proves.** `Core::apply_side_effects` runs
/// `ensure_every_workspace_has_a_tab` at the end of every dispatch, so by the time any
/// handler runs there is no workspace in the model without a tab, and the branch in
/// `api::workspace::focus` that would make one cannot be reached from here. The tab is the
/// core's. What is pinned here is the outcome the reader gets, which is what a switch has to
/// deliver whichever layer delivers it.
#[tokio::test]
async fn switching_to_a_workspace_lands_on_a_tab_with_a_shell_in_its_own_worktree() {
    let (_tmp, repo) = repo_with_origin("main");
    let slot = git::slot_path(&repo, 1);
    git::worktree_add(&repo, &slot, "workspace-1", "origin/main").unwrap();
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let client = h.client.clone();
    h.api("project.add", json!({"path": repo.to_str().unwrap()}))
        .await
        .unwrap();
    let _ = h.frame(client.clone()).await;
    let w1 = h
        .model()
        .projects
        .iter()
        .flat_map(|p| p.workspaces.iter())
        .find(|w| w.handle.to_string() == "workspace-1")
        .map(|w| w.id.clone())
        .expect("project.add adopted the worktree on disk");
    let came_from = h.focused_pane(client.clone());

    let info = h
        .api(
            "workspace.focus",
            json!({"workspace": w1.as_str(), "client": client.as_str()}),
        )
        .await
        .unwrap();

    assert_eq!(info["tabs"], 1);
    let tab = h.current_tab(client.clone());
    let pane = h.model().tab(&tab).unwrap().focused.clone();
    assert_ne!(
        pane, came_from,
        "and not the pane the client was already on"
    );
    assert_eq!(
        h.model().pane(&pane).unwrap().cwd,
        slot.canonicalize().unwrap(),
        "the shell starts in the workspace's own worktree, not in the project root"
    );
    assert!(
        h.pane_is_running(&pane),
        "there is a shell behind it to type into"
    );
}

/// The tab the switch lands on is the one that workspace was last on, not its first.
///
/// Two tabs, and the second selected before switching away: with one tab, or with the last
/// one being the first one, `last_tab` and `tabs.first()` are the same answer and the two
/// implementations cannot be told apart.
#[tokio::test]
async fn switching_back_to_a_workspace_lands_on_the_tab_it_was_last_on() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (_root, w1, w2) = h.git_project_with_two_slots().await;
    let client = h.client.clone();
    h.api(
        "workspace.focus",
        json!({"workspace": w1.as_str(), "client": client.as_str()}),
    )
    .await
    .unwrap();
    let first = h.current_tab(client.clone());
    h.api(
        "tab.create",
        json!({"workspace": w1.as_str(), "client": client.as_str()}),
    )
    .await
    .unwrap();
    let _ = h.frame(client.clone()).await;
    let second = h.current_tab(client.clone());
    assert_ne!(
        first, second,
        "the workspace has two tabs and is on the second"
    );

    h.api(
        "workspace.focus",
        json!({"workspace": w2.as_str(), "client": client.as_str()}),
    )
    .await
    .unwrap();
    h.api(
        "workspace.focus",
        json!({"workspace": w1.as_str(), "client": client.as_str()}),
    )
    .await
    .unwrap();
    let _ = h.frame(client.clone()).await;

    assert_eq!(h.current_tab(client.clone()), second);
}

/// Switching is one client's move. The other client keeps its workspace, its tab and its
/// pane: with a single client attached, "the calling client" and "every client" are the same
/// set and a handler that moved everyone would pass every other test in this file.
///
/// The client that asks is the **second** one attached, and the one checked afterwards is
/// the first. `Model::clients` is in attach order, so a handler that reached for the first
/// client rather than the one it was handed would move the wrong view here; with the roles
/// the other way round the two would be the same client and the test would prove nothing.
#[tokio::test]
async fn focusing_a_workspace_moves_the_client_that_asked_and_no_other() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (_root, w1, _w2) = h.git_project_with_two_slots().await;
    let first = h.client.clone();
    let asked = h.attach(80, 24).await;
    let before = h.model().client(&first).unwrap().clone();

    h.api(
        "workspace.focus",
        json!({"workspace": w1.as_str(), "client": asked.as_str()}),
    )
    .await
    .unwrap();
    let _ = h.frame(asked.clone()).await;

    assert_eq!(h.model().client(&asked).unwrap().workspace, w1);
    let after = h.model().client(&first).unwrap().clone();
    assert_eq!(after.workspace, before.workspace);
    assert_eq!(after.tab, before.tab);
    assert_eq!(after.focus, before.focus);
    assert_eq!(after.projects_cursor, before.projects_cursor);
}

/// A subscriber learns that a client changed workspace from `workspace.switched` and from
/// nothing else, so a handler that moved the view without publishing would leave every
/// stream reader looking at the wrong workspace.
#[tokio::test]
async fn focusing_a_workspace_publishes_workspace_switched_with_the_tab_it_landed_on() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (_root, w1, _w2) = h.git_project_with_two_slots().await;
    let client = h.client.clone();
    let socket = h.socket_path().to_path_buf();
    let mut events = support::subscribe(&socket, &["workspace.switched"]).await;

    h.api(
        "workspace.focus",
        json!({"workspace": w1.as_str(), "client": client.as_str()}),
    )
    .await
    .unwrap();
    let _ = h.frame(client.clone()).await;

    let event = support::next_event(&mut events).await;
    assert_eq!(event["event"], "workspace.switched", "{event}");
    assert_eq!(event["client"], client.as_str(), "{event}");
    assert_eq!(event["workspace"], w1.as_str(), "{event}");
    assert_eq!(
        event["tab"],
        h.current_tab(client.clone()).as_str(),
        "{event}"
    );
}

/// A target nothing matches is refused, and the refusal is the whole outcome: the view, the
/// model's last workspace and the frame are all as they were. An error code on its own does
/// not prove that, because a handler that had already moved `last_workspace` before failing
/// would return the same code.
#[tokio::test]
async fn focusing_a_workspace_that_does_not_exist_refuses_and_changes_nothing() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (_root, _w1, _w2) = h.git_project_with_two_slots().await;
    let client = h.client.clone();
    let before_frame = h.frame(client.clone()).await;
    let before_view = h.model().client(&client).unwrap().clone();
    let before_last = h.model().last_workspace.clone();

    let err = h
        .api(
            "workspace.focus",
            json!({"workspace": "workspace-9", "client": client.as_str()}),
        )
        .await
        .unwrap_err();

    assert_eq!(err.code, ErrorCode::NotFound);
    assert_eq!(h.model().client(&client).unwrap(), &before_view);
    assert_eq!(h.model().last_workspace, before_last);
    assert_eq!(h.frame(client.clone()).await, before_frame);
}

/// The same, for a client that is not attached. Without the guard the handler would still
/// move `last_workspace` and give the target a tab and a shell on behalf of a client that
/// does not exist, and answer `not_found` afterwards.
#[tokio::test]
async fn focusing_a_workspace_for_a_client_that_is_not_attached_changes_nothing() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (_root, w1, _w2) = h.git_project_with_two_slots().await;
    let before_last = h.model().last_workspace.clone();
    let before_tabs = h.model().workspace(&w1).unwrap().tabs.len();

    let err = h
        .api(
            "workspace.focus",
            json!({"workspace": w1.as_str(), "client": "c_9999"}),
        )
        .await
        .unwrap_err();

    assert_eq!(err.code, ErrorCode::NotFound);
    assert_eq!(err.message, "client c_9999 is not attached");
    assert_eq!(h.model().last_workspace, before_last);
    assert_eq!(h.model().workspace(&w1).unwrap().tabs.len(), before_tabs);
}

#[tokio::test]
async fn workspace_list_carries_the_branch_and_the_pull_request_and_resume_says_it_is_m3_s() {
    let mut h = Harness::start_with(HarnessOptions {
        providers: vec![Arc::new(BranchProvider), Arc::new(OnePrProvider)],
        ..HarnessOptions::new(Config::default(), 120, 24)
    })
    .await;
    let (_root, w1, w2) = h.git_project_with_two_slots().await;
    h.wait_for_fact(
        &FactKey::workspace(&w1, FACT_BRANCH),
        |f| f.is_some(),
        Duration::from_secs(5),
    )
    .await
    .expect("the branch provider answered for workspace-1");
    h.wait_for_fact(
        &FactKey::workspace(&w2, FACT_PR),
        |f| f.is_some(),
        Duration::from_secs(5),
    )
    .await
    .expect("the pull request provider answered for workspace-2");

    let list = h
        .api("workspace.list", json!({"project": "audrey-app"}))
        .await
        .unwrap();
    let rows = list.as_array().unwrap();

    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0]["handle"], "main");
    assert_eq!(rows[1]["handle"], "workspace-1");
    assert_eq!(rows[1]["branch"], "workspace-1");
    // Two different wrong shapes, and `rows[1]["pr"]` cannot tell them apart on its own:
    // `serde_json`'s `Index` answers `Value::Null` for a key that is not there as well as for
    // a key whose value is null. So the key is asserted separately from the value. A caller
    // reading this row has the same problem, which is why the wire shape is the contract.
    let row = rows[1].as_object().expect("a row is an object");
    for field in ["pr", "pr_state"] {
        assert!(
            row.contains_key(field),
            "{field} is written, not omitted: {row:?}"
        );
        assert!(
            row[field].is_null(),
            "and it is null, not an empty string: {row:?}"
        );
    }
    assert_eq!(rows[2]["handle"], "workspace-2");
    assert_eq!(rows[2]["pr"], "PR#212", "the row carries the fact's text");
    assert_eq!(
        rows[2]["pr_state"], "DRAFT",
        "and the state that colours it"
    );
    assert_eq!(rows[2]["tabs"], 1);

    let err = h
        .api("workspace.resume", json!({"workspace": "workspace-1"}))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Unavailable);
    assert_eq!(err.message, "resume arrives with agents in M3");
    // And for a target that matches nothing, which is the input that separates "refuses
    // without looking" from "resolves first and then refuses": a resume that resolved would
    // answer `NotFound` here and the same `Unavailable` above.
    let err = h
        .api("workspace.resume", json!({"workspace": "workspace-9"}))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Unavailable);
    assert_eq!(err.message, "resume arrives with agents in M3");
}

/// A project the caller named scopes the list to that project, and naming none lists them
/// all. The harness holds a project of its own beside the git one, so the two answers differ
/// here: with a single project in the model they would be the same list.
#[tokio::test]
async fn workspace_list_scopes_to_a_named_project_and_lists_every_project_without_one() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (_root, _w1, _w2) = h.git_project_with_two_slots().await;

    let scoped = h
        .api("workspace.list", json!({"project": "audrey-app"}))
        .await
        .unwrap();
    let all = h.api("workspace.list", json!({})).await.unwrap();

    let scoped = scoped.as_array().unwrap();
    let all = all.as_array().unwrap();
    assert_eq!(scoped.len(), 3);
    assert!(
        all.len() > scoped.len(),
        "the harness's own project has a main too: {all:?}"
    );
    for row in scoped {
        assert!(all.contains(row), "{row} is missing from the whole list");
    }

    let err = h
        .api("workspace.list", json!({"project": "no-such-project"}))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
}

/// Naming a workspace is what a name is for: it replaces the handle on the row and it
/// resolves the workspace afterwards, which is how `workspace.clear` and `workspace.delete`
/// take a target the reader typed.
#[tokio::test]
async fn naming_a_workspace_sets_the_name_and_the_name_then_resolves_it() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (_root, w1, w2) = h.git_project_with_two_slots().await;
    let client = h.client.clone();

    h.api(
        "workspace.rename",
        json!({"workspace": "workspace-1", "name": "auth cleanup"}),
    )
    .await
    .unwrap();
    let _ = h.frame(client.clone()).await;

    assert_eq!(
        h.model().workspace(&w1).unwrap().name.as_deref(),
        Some("auth cleanup")
    );
    assert_eq!(
        h.model().workspace(&w2).unwrap().name,
        None,
        "and only the workspace that was named"
    );
    // The resolution key, which is what Task 18's calls depend on. `workspace-2` is still
    // there under its handle, so a resolver that answered with the first workspace it saw
    // rather than the one the name belongs to would fail here.
    let info = h
        .api(
            "workspace.focus",
            json!({"workspace": "auth cleanup", "client": client.as_str()}),
        )
        .await
        .unwrap();
    assert_eq!(info["id"], w1.as_str());
    assert_eq!(info["handle"], "workspace-1");
    assert_eq!(info["name"], "auth cleanup");
}

/// The name comes off two ways, and both leave the handle standing on its own again.
#[tokio::test]
async fn clearing_a_name_by_either_route_brings_the_handle_back() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (_root, w1, w2) = h.git_project_with_two_slots().await;
    let client = h.client.clone();
    for target in [&w1, &w2] {
        h.api(
            "workspace.rename",
            json!({"workspace": target.as_str(), "name": "auth cleanup"}),
        )
        .await
        .unwrap();
    }
    let _ = h.frame(client.clone()).await;

    // A blank name clears it, the rule `Model::rename_workspace` holds.
    h.api(
        "workspace.rename",
        json!({"workspace": w1.as_str(), "name": "   "}),
    )
    .await
    .unwrap();
    // And so does the method whose whole job it is.
    h.api("workspace.clear_name", json!({"workspace": w2.as_str()}))
        .await
        .unwrap();
    let _ = h.frame(client.clone()).await;

    assert_eq!(h.model().workspace(&w1).unwrap().name, None);
    assert_eq!(h.model().workspace(&w2).unwrap().name, None);
    let list = h
        .api("workspace.list", json!({"project": "audrey-app"}))
        .await
        .unwrap();
    let rows = list.as_array().unwrap();
    assert_eq!(rows[1]["name"], serde_json::Value::Null);
    assert_eq!(rows[2]["name"], serde_json::Value::Null);
}

/// `workspace.clear_name` with no target means the workspace the caller is in, which is the
/// shape `leader n` presses. The client is moved to `workspace-1` first and `workspace-2`
/// keeps its name, so "the view's workspace" and "some workspace" are different answers.
#[tokio::test]
async fn clearing_a_name_with_no_target_clears_the_one_the_view_is_in() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (_root, w1, w2) = h.git_project_with_two_slots().await;
    let client = h.client.clone();
    for target in [&w1, &w2] {
        h.api(
            "workspace.rename",
            json!({"workspace": target.as_str(), "name": "auth cleanup"}),
        )
        .await
        .unwrap();
    }
    h.api(
        "workspace.focus",
        json!({"workspace": w1.as_str(), "client": client.as_str()}),
    )
    .await
    .unwrap();
    let _ = h.frame(client.clone()).await;

    // No `client` here: `WorkspaceTargetParams` has no such field, so this resolves through
    // `Ctx::view`, which is the only client attached. A test with two would have to name it.
    h.api("workspace.clear_name", json!({})).await.unwrap();
    let _ = h.frame(client.clone()).await;

    assert_eq!(h.model().workspace(&w1).unwrap().name, None);
    assert_eq!(
        h.model().workspace(&w2).unwrap().name.as_deref(),
        Some("auth cleanup"),
        "the workspace the view is not in keeps its name"
    );
}

// A call with no name at all is the reader asking to be prompted, and Task 15 built the box
// that asks: `renaming_with_no_name_opens_the_box_and_changes_nothing` in `tests/name_box.rs`
// took over from the test that stood here, which pinned the refusal this file's `rename` gave
// while that box did not exist.

/// A target resolves by its branch too, which is what lets a call from a shell on
/// `feat/auth-cleanup` name its workspace without an id.
///
/// The fixture is a worktree adopted on a branch that is not its handle, because a slot
/// `workspace.create` made is on branch `workspace-1` and answers to that string in the
/// handle pass: the branch pass would never run and the test would pass against a resolver
/// that had no branch pass at all. Nothing else in the model answers to
/// `feat/auth-cleanup` - `main` is on `main` and the handle is `workspace-1` - so the branch
/// is the only thing that can have resolved it.
#[tokio::test]
async fn a_workspace_resolves_by_the_branch_it_is_on() {
    let (_tmp, repo) = repo_with_origin("main");
    git::worktree_add(
        &repo,
        &git::slot_path(&repo, 1),
        "feat/auth-cleanup",
        "origin/main",
    )
    .unwrap();
    let mut h = Harness::start_with(HarnessOptions {
        providers: vec![Arc::new(BranchProvider)],
        ..HarnessOptions::new(Config::default(), 80, 24)
    })
    .await;
    let client = h.client.clone();
    h.api("project.add", json!({"path": repo.to_str().unwrap()}))
        .await
        .unwrap();
    let _ = h.frame(client.clone()).await;
    let w1 = h
        .model()
        .projects
        .iter()
        .flat_map(|p| p.workspaces.iter())
        .find(|w| w.handle.to_string() == "workspace-1")
        .map(|w| w.id.clone())
        .expect("project.add adopted the worktree on disk");
    h.wait_for_fact(
        &FactKey::workspace(&w1, FACT_BRANCH),
        |f| f.is_some_and(|f| f.text == "feat/auth-cleanup"),
        Duration::from_secs(10),
    )
    .await
    .expect("the branch provider answered");

    let info = h
        .api(
            "workspace.focus",
            json!({"workspace": "feat/auth-cleanup", "client": client.as_str()}),
        )
        .await
        .unwrap();

    assert_eq!(info["id"], w1.as_str());
    assert_eq!(info["handle"], "workspace-1");
    assert_eq!(info["branch"], "feat/auth-cleanup");
}

/// `Model::last_workspace` is persisted state, so a server that dies without a clean stop
/// must not lose where the reader was. `Event::WorkspaceSwitched` is a structural event and
/// `Core::publish_events` writes the state file whenever one is published, which is what
/// makes that true.
///
/// The file is read from the running harness rather than across `Harness::restart`, because
/// `Core::shutdown` persists unconditionally: a restart would prove only that a clean stop
/// writes the file (Task 13a, M25).
///
/// Any other structural event in the same batch would satisfy this assertion, so the one
/// that could occur is removed rather than assumed away. `sync_pane_sizes` publishes
/// `PaneResized` when a pane changes size, and the assertion that no pane did comes first.
#[tokio::test]
async fn switching_workspaces_reaches_the_state_file_while_the_server_is_still_running() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (_root, w1, _w2) = h.git_project_with_two_slots().await;
    let client = h.client.clone();
    let landing = match landing_pane(&h, &w1) {
        Focus::Pane(p) => p,
        other => panic!("a workspace lands on a pane, not {other:?}"),
    };
    let before_size = h.pane_size(&landing);
    let state = h.state_dir().join("state.json");
    // The file may not be there yet. It is written on a debounce after a structural event,
    // so the writes that making the slots triggered may not have landed. Absent is a
    // stronger form of this precondition than present with another value, since it means
    // nothing has persisted a `last_workspace` at all, so both are accepted and neither is
    // assumed. The assertion after the focus still requires the file to exist and to name
    // this workspace, which is what the test is for.
    if let Ok(text) = std::fs::read_to_string(&state) {
        let before: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_ne!(before["last_workspace"], w1.as_str(), "{before}");
    }

    h.api(
        "workspace.focus",
        json!({"workspace": w1.as_str(), "client": client.as_str()}),
    )
    .await
    .unwrap();
    let _ = h.frame(client.clone()).await;
    // Past the 100 ms persist debounce, bounded so a regression fails in a quarter of a
    // second rather than never finishing.
    tokio::time::sleep(Duration::from_millis(250)).await;

    let after_size = h.pane_size(&landing);
    assert_eq!(
        (after_size.cols, after_size.rows),
        (before_size.cols, before_size.rows),
        "no pane resized, so no `PaneResized` wrote the file instead"
    );
    let after: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&state).expect("state.json")).unwrap();
    assert_eq!(after["last_workspace"], w1.as_str(), "{after}");
}

/// A name that reads as a handle is refused, so a name can always find the workspace it was
/// given to.
///
/// `Model::resolve_workspace_with` runs the handle pass before the name pass and returns on
/// the first pass with one hit. So without this guard, naming `workspace-1` the string
/// `workspace-2` would leave that string resolving to the **other** workspace, the chosen name
/// unreachable, and two rows in the Projects box reading the same words. `workspace.clear` and
/// `workspace.delete` take their target through that resolver, so the reader who types the
/// name they chose acts on the worktree they did not.
///
/// The fixture has both slots, because that is what makes the shadowing observable: with only
/// `workspace-1` in the model the refused string would resolve to nothing and the two
/// implementations would agree.
#[tokio::test]
async fn naming_a_workspace_with_a_handle_is_refused_so_the_name_stays_findable() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (_root, w1, w2) = h.git_project_with_two_slots().await;
    let client = h.client.clone();

    for shadow in ["workspace-2", "WORKSPACE-2", "main", " workspace-1 "] {
        let err = h
            .api(
                "workspace.rename",
                json!({"workspace": w1.as_str(), "name": shadow}),
            )
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidParams, "{shadow}: {err}");
        assert!(
            err.message.contains("is a handle"),
            "the refusal says what was refused: {err}"
        );
        assert_eq!(
            h.model().workspace(&w1).unwrap().name,
            None,
            "and nothing was written: {shadow}"
        );
    }

    // The near miss on the other side: `workspace-01` parses as a number and no handle prints
    // it, so nothing could ever match it by the handle pass and it is a perfectly good name.
    h.api(
        "workspace.rename",
        json!({"workspace": w1.as_str(), "name": "workspace-01"}),
    )
    .await
    .unwrap();
    let _ = h.frame(client.clone()).await;
    assert_eq!(
        h.model().workspace(&w1).unwrap().name.as_deref(),
        Some("workspace-01")
    );
    // And the handle it nearly spells still answers for the workspace that owns it.
    let info = h
        .api(
            "workspace.focus",
            json!({"workspace": "workspace-1", "client": client.as_str()}),
        )
        .await
        .unwrap();
    assert_eq!(info["id"], w1.as_str());
    let info = h
        .api(
            "workspace.focus",
            json!({"workspace": "workspace-01", "client": client.as_str()}),
        )
        .await
        .unwrap();
    assert_eq!(info["id"], w1.as_str(), "the name resolves too");
    assert_ne!(info["id"], w2.as_str());
}

/// Naming a workspace puts the new name on the screen.
///
/// Only `ctx.view_dirty` can do that here: `dispatch_inner` ORs in the handler's own flag, and
/// `apply_side_effects` raises it only when something was spawned, killed or replaced, which a
/// naming never is. The harness clock is fixed, so nothing redraws on a tick either. So the
/// frame changes because the handler asked for it or it does not change at all, and the wait
/// below fails rather than passing on a redraw something else supplied.
#[tokio::test]
async fn naming_a_workspace_redraws_the_screen_that_shows_its_row() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let (_root, w1, _w2) = h.git_project_with_two_slots().await;
    let client = h.client.clone();
    h.api("sidebar.show", json!({"client": client.as_str()}))
        .await
        .unwrap();
    let before = h
        .wait_for(
            client.clone(),
            |f| f.contains("workspace-1"),
            Duration::from_secs(2),
        )
        .await;
    assert!(!before.contains("auth cleanup"), "{before}");

    h.api(
        "workspace.rename",
        json!({"workspace": w1.as_str(), "name": "auth cleanup"}),
    )
    .await
    .unwrap();

    h.wait_for(
        client.clone(),
        |f| f.contains("auth cleanup"),
        Duration::from_secs(2),
    )
    .await;
}

/// And taking it off puts the handle back on the screen, for the same reason and through the
/// same one flag.
#[tokio::test]
async fn clearing_a_name_redraws_the_screen_that_shows_its_row() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let (_root, w1, _w2) = h.git_project_with_two_slots().await;
    let client = h.client.clone();
    h.api("sidebar.show", json!({"client": client.as_str()}))
        .await
        .unwrap();
    h.api(
        "workspace.rename",
        json!({"workspace": w1.as_str(), "name": "auth cleanup"}),
    )
    .await
    .unwrap();
    let before = h
        .wait_for(
            client.clone(),
            |f| f.contains("auth cleanup"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        !before.contains("workspace-1"),
        "the name has replaced the handle on the row:\n{before}"
    );

    h.api("workspace.clear_name", json!({"workspace": w1.as_str()}))
        .await
        .unwrap();

    h.wait_for(
        client.clone(),
        |f| f.contains("workspace-1"),
        Duration::from_secs(2),
    )
    .await;
}
