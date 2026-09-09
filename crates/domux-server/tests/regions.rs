//! Reaching the boxes: `C-h` into the sidebar's Projects box, `C-l` and `Esc` back out, and
//! the `[keys.list]` routing that serves the box and the switcher from one table.
//!
//! Every test here that could be written with one pane, one project or one workspace is
//! written with two, because that is where the two implementations differ: with one pane
//! "geometry first" and "the sidebar first" agree, with one project the cursor never crosses
//! a header, and with one workspace it never crosses a blank.

use domux_core::config::Config;
use domux_core::ids::WorkspaceId;
use domux_core::model::{Focus, Model, RegionKind};
use domux_server::testing::{row, Harness};
use serde_json::json;
use std::time::Duration;

/// Columns `from` to `to` of a frame row, without the `|` framing.
fn cols(line: &str, from: usize, to: usize) -> String {
    line.chars().skip(1 + from).take(to - from + 1).collect()
}

/// Row `n` of the sidebar's own 38 columns.
///
/// The whole frame is the wrong thing to search for a workspace's handle here: every test
/// that builds slots leaves `Created workspace-2` in the hint row, so `f.contains(...)` would
/// be answered by the pill as readily as by the row (a second cause for the same observation).
/// The box's rows are r1 upwards, r0 and r22 are its border, and r23 is the hint row.
fn box_row(f: &str, n: usize) -> String {
    cols(row(f, n), 0, 37)
}

fn focus(h: &Harness) -> Focus {
    h.model()
        .client(&h.client)
        .expect("the first client is attached")
        .focus
        .clone()
}

fn cursor(h: &Harness) -> Option<WorkspaceId> {
    h.model()
        .client(&h.client)
        .expect("the first client is attached")
        .projects_cursor
        .clone()
}

fn filter(h: &Harness) -> (String, bool) {
    let view = h
        .model()
        .client(&h.client)
        .expect("the first client is attached")
        .clone();
    (view.filter, view.filtering)
}

/// The workspace of the project the harness roots itself at, which is where a fresh client
/// starts. It is a plain folder called `proj`, so it sorts after the git project's
/// `audrey-app` and its `main` is the last row the cursor can rest on.
fn proj_main(model: &Model) -> WorkspaceId {
    model
        .projects
        .iter()
        .find(|p| p.name == "proj")
        .and_then(|p| p.workspaces.first())
        .map(|w| w.id.clone())
        .expect("the harness roots a project at <tmp>/proj")
}

/// A 120 column client with the sidebar showing and the keys in the Projects box.
async fn in_the_box(h: &mut Harness) -> String {
    h.api("sidebar.show", json!({})).await.unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains("Projects"),
        Duration::from_secs(2),
    )
    .await;
    h.key(h.client.clone(), "C-h").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("r0 c0-0 fg=#cba6f7"),
        Duration::from_secs(2),
    )
    .await
}

#[tokio::test]
async fn c_h_from_the_leftmost_pane_enters_the_projects_box_and_c_l_and_esc_return() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let pane = h.focused_pane(h.client.clone());
    let f = in_the_box(&mut h).await;
    assert_eq!(focus(&h), Focus::Region(RegionKind::SidebarProjects), "{f}");
    assert!(
        !f.contains("r1 c39-39 fg=#cba6f7"),
        "no pane carries the accent while the box has it:\n{f}"
    );
    assert!(
        row(&f, 23).contains("⏎ open"),
        "the hint row shows the cursor row's keys:\n{}",
        row(&f, 23)
    );

    h.key(h.client.clone(), "C-l").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("r1 c39-39 fg=#cba6f7"),
        Duration::from_secs(2),
    )
    .await;
    assert_eq!(focus(&h), Focus::Pane(pane.clone()), "C-l comes back");

    h.key(h.client.clone(), "C-h").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("r0 c0-0 fg=#cba6f7"),
        Duration::from_secs(2),
    )
    .await;
    h.key(h.client.clone(), "Esc").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("r1 c39-39 fg=#cba6f7"),
        Duration::from_secs(2),
    )
    .await;
    assert_eq!(focus(&h), Focus::Pane(pane), "and so does Esc");
}

/// The pane to the left is what `C-h` is for. The sidebar is what lies past the workpanel's
/// edge, not what lies past the pane, so the box is reached only from a pane that has no left
/// neighbour.
///
/// Two panes and not one: with a single pane every pane is the leftmost one, and an
/// implementation that entered the sidebar before looking at the geometry would pass.
#[tokio::test]
async fn c_h_takes_the_pane_to_the_left_before_it_takes_the_sidebar() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let left = h.focused_pane(h.client.clone());
    h.api("pane.split", json!({"dir": "right"})).await.unwrap();
    h.api("sidebar.show", json!({})).await.unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains("Projects"),
        Duration::from_secs(2),
    )
    .await;
    let right = h.focused_pane(h.client.clone());
    assert_ne!(right, left, "the split focused the new pane");

    h.key(h.client.clone(), "C-h").await;
    h.frame(h.client.clone()).await;
    assert_eq!(
        focus(&h),
        Focus::Pane(left.clone()),
        "the first C-h crosses to the pane on the left"
    );

    h.key(h.client.clone(), "C-h").await;
    h.frame(h.client.clone()).await;
    assert_eq!(
        focus(&h),
        Focus::Region(RegionKind::SidebarProjects),
        "and the second one, from the leftmost pane, enters the box"
    );
}

/// `C-h` reaches a box the reader can see. The sidebar the client is not drawing is not one,
/// whether it is closed or whether the screen is too narrow for it (interface spec 12.1).
#[tokio::test]
async fn c_h_leaves_the_keys_on_the_pane_while_no_sidebar_is_drawn() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let pane = h.focused_pane(h.client.clone());
    h.key(h.client.clone(), "C-h").await;
    h.frame(h.client.clone()).await;
    assert_eq!(
        focus(&h),
        Focus::Pane(pane),
        "the sidebar is closed, so there is no box to enter"
    );

    // 119 columns and not 120, and shown from the wide client rather than from this one:
    // `sidebar_visible` is `sidebar_open` AND either the room for it or `sidebar_forced`, and
    // asking for the sidebar on a narrow screen sets `sidebar_forced` and gets it anyway
    // (interface spec 12.1). So the client that asks cannot tell `sidebar_open` from
    // `sidebar_visible`; only a narrow client that did not ask can.
    let narrow = h.attach(119, 24).await;
    h.api("sidebar.show", json!({"client": h.client.as_str()}))
        .await
        .unwrap();
    let narrow_pane = h.focused_pane(narrow.clone());
    h.key(narrow.clone(), "C-h").await;
    h.frame(narrow.clone()).await;
    assert!(
        h.model().sidebar_open,
        "the sidebar is open; this client's screen is what has no room for it"
    );
    assert_eq!(
        h.model().client(&narrow).unwrap().focus,
        Focus::Pane(narrow_pane),
        "a sidebar this client is not drawing is not a box it can move into"
    );
}

/// Tab, `C-j` and `C-k` have nowhere to go until M3 puts the Agents box under Projects. They
/// must not move the tab's focused pane and must not reach the pane either.
///
/// The tab is split downwards first, so the pane the keys would move to exists: with one pane
/// `C-j` does nothing whatever the region rule is.
#[tokio::test]
async fn tab_and_c_j_and_c_k_do_nothing_in_the_sidebar_until_m3_adds_the_agents_box() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let top = h.focused_pane(h.client.clone());
    h.api("pane.split", json!({"dir": "down"})).await.unwrap();
    let bottom = h.focused_pane(h.client.clone());
    assert_ne!(bottom, top, "the split made a pane below");
    in_the_box(&mut h).await;
    let before = h
        .model()
        .tab(&h.current_tab(h.client.clone()))
        .unwrap()
        .focused
        .clone();

    for key in ["Tab", "C-j", "C-k"] {
        h.key(h.client.clone(), key).await;
        h.frame(h.client.clone()).await;
        assert_eq!(
            focus(&h),
            Focus::Region(RegionKind::SidebarProjects),
            "{key} has nothing to cross to yet"
        );
        assert_eq!(
            h.model()
                .tab(&h.current_tab(h.client.clone()))
                .unwrap()
                .focused,
            before,
            "{key} does not move the focused pane under the box either"
        );
    }
    assert!(
        h.pane_input(&top).is_empty() && h.pane_input(&bottom).is_empty(),
        "and none of them reaches a pane"
    );
}

/// One press moves one workspace. The blank row between two workspaces and the header of the
/// next project are stepped over, and neither end wraps.
///
/// The client is moved into the git project's `main` first, so the cursor starts at the top of
/// the list and the walk crosses a project boundary: that is the only place in this fixture
/// where a step has to skip two rows rather than one, and an implementation that stepped over
/// blanks but not headers would pass a walk inside one project.
#[tokio::test]
async fn j_and_k_move_the_cursor_over_workspaces_and_never_onto_a_header_or_a_blank_row() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let (_root, w1, w2) = h.git_project_with_two_slots().await;
    let git_main = h
        .model()
        .project_of_workspace(&w1)
        .and_then(|p| p.workspaces.first().map(|w| w.id.clone()))
        .expect("the git project holds main");
    let proj_main = proj_main(&h.model());
    h.api("workspace.focus", json!({"workspace": git_main.as_str()}))
        .await
        .unwrap();
    in_the_box(&mut h).await;
    assert_eq!(
        cursor(&h),
        Some(git_main.clone()),
        "the cursor starts on the workspace this client is in"
    );

    for expected in [&w1, &w2, &proj_main] {
        h.key(h.client.clone(), "j").await;
        h.frame(h.client.clone()).await;
        assert_eq!(cursor(&h), Some(expected.clone()));
    }
    h.key(h.client.clone(), "j").await;
    h.frame(h.client.clone()).await;
    assert_eq!(
        cursor(&h),
        Some(proj_main),
        "the last row holds; nothing wraps"
    );

    for expected in [&w2, &w1, &git_main] {
        h.key(h.client.clone(), "k").await;
        h.frame(h.client.clone()).await;
        assert_eq!(cursor(&h), Some(expected.clone()));
    }
    h.key(h.client.clone(), "k").await;
    h.frame(h.client.clone()).await;
    assert_eq!(cursor(&h), Some(git_main), "the first row holds too");
}

/// The key and the API call are one handler, so pressing `j` and calling `list.down` leave the
/// cursor in the same place.
///
/// The test would pass on two separate implementations if the cursor never moved, so it
/// asserts the move first and then the agreement.
#[tokio::test]
async fn pressing_the_list_key_and_calling_the_api_move_the_cursor_the_same_way() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let (_root, w1, _w2) = h.git_project_with_two_slots().await;
    let git_main = h
        .model()
        .project_of_workspace(&w1)
        .and_then(|p| p.workspaces.first().map(|w| w.id.clone()))
        .expect("the git project holds main");
    h.api("workspace.focus", json!({"workspace": git_main.as_str()}))
        .await
        .unwrap();
    in_the_box(&mut h).await;

    h.key(h.client.clone(), "j").await;
    h.frame(h.client.clone()).await;
    let by_key = cursor(&h);
    assert_eq!(by_key, Some(w1), "the key moved the cursor at all");

    h.key(h.client.clone(), "k").await;
    h.frame(h.client.clone()).await;
    assert_eq!(cursor(&h), Some(git_main), "back where it started");

    h.api("list.down", json!({})).await.unwrap();
    h.frame(h.client.clone()).await;
    assert_eq!(cursor(&h), by_key, "and the API call lands on the same row");
}

/// `list.*` act on a box, so they refuse when the keys are not in one, and refuse without
/// touching the state. An error code alone would not say that: the refusal has to leave the
/// view where it found it.
///
/// Every method, `list.filter` included. `filter` is the one that looks harmless - it writes a
/// single flag - and it is the worst of the four to leave unguarded, because a filter field
/// opened over no box takes every key the reader presses next.
#[tokio::test]
async fn the_list_methods_refuse_and_change_nothing_when_the_keys_are_not_in_a_box() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    h.api("sidebar.show", json!({})).await.unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains("Projects"),
        Duration::from_secs(2),
    )
    .await;
    let before = h.model().client(&h.client).unwrap().clone();
    for method in ["list.down", "list.up", "list.activate", "list.filter"] {
        let err = h.api(method, json!({})).await.expect_err(method);
        assert_eq!(
            err.code,
            domux_core::api::ErrorCode::Refused,
            "{method}: {err}"
        );
        assert!(err.message.contains("not in a box"), "{method}: {err}");
    }
    let after = h.model().client(&h.client).unwrap().clone();
    assert_eq!(after.projects_cursor, before.projects_cursor);
    assert_eq!(after.workspace, before.workspace);
    assert_eq!(after.focus, before.focus);
    assert!(!after.filtering, "and no filter field was opened");
    assert_eq!(after.filter, before.filter);
}

/// The switcher's box is the sidebar's box, so the same keys drive it: `overlay_key`'s
/// switcher arm calls the same `list_key`.
#[tokio::test]
async fn the_same_keys_work_in_the_switcher_because_it_is_the_same_box() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (_root, w1, _w2) = h.git_project_with_two_slots().await;
    let git_main = h
        .model()
        .project_of_workspace(&w1)
        .and_then(|p| p.workspaces.first().map(|w| w.id.clone()))
        .expect("the git project holds main");
    h.api("workspace.focus", json!({"workspace": git_main.as_str()}))
        .await
        .unwrap();
    h.api("switcher.open", json!({})).await.unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains("Projects"),
        Duration::from_secs(2),
    )
    .await;
    h.key(h.client.clone(), "j").await;
    h.frame(h.client.clone()).await;
    assert_eq!(
        cursor(&h),
        Some(w1),
        "j moves the cursor inside the overlay"
    );
    h.key(h.client.clone(), "k").await;
    h.frame(h.client.clone()).await;
    assert_eq!(cursor(&h), Some(git_main), "and k moves it back");

    h.key(h.client.clone(), "Esc").await;
    h.wait_for(
        h.client.clone(),
        |f| !f.contains("Projects"),
        Duration::from_secs(2),
    )
    .await;
    assert_eq!(h.model().client(&h.client).unwrap().overlay, None);
    assert!(matches!(focus(&h), Focus::Pane(_)));
}

/// Enter switches to the row under the cursor on both surfaces, and both reach
/// `api::workspace::focus`: the same outcomes Task 19's tests assert through the API, reached
/// by pressing keys. One operation, one handler, two ways in (principle 3).
#[tokio::test]
async fn enter_activates_the_row_under_the_cursor_in_both_surfaces() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (_root, w1, _w2) = h.git_project_with_two_slots().await;
    let git_main = h
        .model()
        .project_of_workspace(&w1)
        .and_then(|p| p.workspaces.first().map(|w| w.id.clone()))
        .expect("the git project holds main");
    h.api("workspace.focus", json!({"workspace": git_main.as_str()}))
        .await
        .unwrap();
    h.api("switcher.open", json!({})).await.unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains("Projects"),
        Duration::from_secs(2),
    )
    .await;
    h.key(h.client.clone(), "j").await;
    h.key(h.client.clone(), "Enter").await;
    h.wait_for(
        h.client.clone(),
        |f| !f.contains("┌ Projects"),
        Duration::from_secs(3),
    )
    .await;
    assert_eq!(
        h.model().client(&h.client).unwrap().workspace,
        w1,
        "the switcher closes on the row you chose"
    );
    assert!(matches!(focus(&h), Focus::Pane(_)));

    let mut h = Harness::start(Config::default(), 120, 24).await;
    let (_root, w1, _w2) = h.git_project_with_two_slots().await;
    let git_main = h
        .model()
        .project_of_workspace(&w1)
        .and_then(|p| p.workspaces.first().map(|w| w.id.clone()))
        .expect("the git project holds main");
    h.api("workspace.focus", json!({"workspace": git_main.as_str()}))
        .await
        .unwrap();
    in_the_box(&mut h).await;
    h.key(h.client.clone(), "j").await;
    h.key(h.client.clone(), "Enter").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("workspace-1"),
            Duration::from_secs(3),
        )
        .await;
    assert!(f.contains("┌ Projects"), "the sidebar stays open:\n{f}");
    assert_eq!(h.model().client(&h.client).unwrap().workspace, w1);
    assert!(
        matches!(focus(&h), Focus::Pane(_)),
        "and the keys go back to the pane:\n{f}"
    );
}

/// `/` opens the filter, the typing narrows the box as it goes, and the row says so. Esc
/// clears the filter and closes it; the hints come back with it (interface spec 12.10).
///
/// The rows are read out of the box's own columns, and both slots are named, so a filter that
/// dropped everything and one that dropped nothing cannot both pass.
#[tokio::test]
async fn slash_filters_the_box_as_you_type_and_esc_clears_it() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let (_root, _w1, _w2) = h.git_project_with_two_slots().await;
    let f = in_the_box(&mut h).await;
    assert!(
        box_row(&f, 3).contains("workspace-1") && box_row(&f, 4).contains("workspace-2"),
        "both slots start visible:\n{f}"
    );

    h.key(h.client.clone(), "/").await;
    h.type_text(h.client.clone(), "workspace-1").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| !box_row(f, 4).contains("workspace-2"),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(filter(&h), ("workspace-1".to_string(), true));
    assert!(
        box_row(&f, 2).contains("workspace-1"),
        "the row that matches stays, under its own header:\n{f}"
    );
    assert!(box_row(&f, 1).contains("AUDREY-APP"), "{f}");
    assert!(
        row(&f, 23).contains("Filter › workspace-1"),
        "the hint row says the box is filtering:\n{}",
        row(&f, 23)
    );

    h.key(h.client.clone(), "Esc").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| box_row(f, 4).contains("workspace-2"),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(filter(&h), (String::new(), false));
    assert!(
        row(&f, 23).contains("⏎ open"),
        "and the keys come back:\n{}",
        row(&f, 23)
    );
    assert_eq!(
        focus(&h),
        Focus::Region(RegionKind::SidebarProjects),
        "Esc closed the filter, not the box"
    );
}

/// While the filter is open a letter is typed, not run. `j` is bound to `list.down`, so a
/// filter that read the table first would move the cursor and never match a workspace with a
/// `j` in its name.
#[tokio::test]
async fn a_letter_typed_into_the_filter_does_not_run_its_list_action() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let (_root, _w1, _w2) = h.git_project_with_two_slots().await;
    in_the_box(&mut h).await;
    let before = cursor(&h);

    h.key(h.client.clone(), "/").await;
    h.key(h.client.clone(), "j").await;
    h.frame(h.client.clone()).await;
    assert_eq!(filter(&h), ("j".to_string(), true), "the letter was typed");
    assert_eq!(cursor(&h), before, "and `list.down` did not run");

    h.key(h.client.clone(), "Backspace").await;
    h.frame(h.client.clone()).await;
    assert_eq!(filter(&h), (String::new(), true), "Backspace deletes");

    h.type_text(h.client.clone(), "workspace").await;
    h.key(h.client.clone(), "Enter").await;
    h.frame(h.client.clone()).await;
    assert_eq!(
        filter(&h),
        ("workspace".to_string(), false),
        "Enter keeps the filter and hands the keys back to the list"
    );
    h.key(h.client.clone(), "j").await;
    h.frame(h.client.clone()).await;
    assert_ne!(cursor(&h), before, "which is what `j` proves");
}

/// The filter belongs to the box's hold on the keys. Leaving with a filter committed gives
/// the sidebar its whole list back, and coming back starts a fresh one, so the sidebar never
/// draws a shortened list with nothing on the screen to say why (principle 2).
#[tokio::test]
async fn leaving_the_box_gives_the_sidebar_its_whole_list_back() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let (_root, _w1, _w2) = h.git_project_with_two_slots().await;
    in_the_box(&mut h).await;
    h.key(h.client.clone(), "/").await;
    h.type_text(h.client.clone(), "workspace-1").await;
    h.key(h.client.clone(), "Enter").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| !box_row(f, 4).contains("workspace-2"),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(
        filter(&h),
        ("workspace-1".to_string(), false),
        "the filter is committed, not open:\n{f}"
    );

    h.key(h.client.clone(), "C-l").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| box_row(f, 4).contains("workspace-2"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        matches!(focus(&h), Focus::Pane(_)),
        "the keys are on the pane:\n{f}"
    );
    assert_eq!(
        filter(&h).0,
        "workspace-1",
        "the field still holds the text; what changed is that the box no longer applies it"
    );
    assert!(
        row(&f, 23).contains("hide"),
        "and the hint row is the sidebar's again:\n{}",
        row(&f, 23)
    );

    h.key(h.client.clone(), "C-h").await;
    h.frame(h.client.clone()).await;
    assert_eq!(
        filter(&h),
        (String::new(), false),
        "and coming back starts a fresh filter"
    );
}

/// `switcher.open` starts with an empty filter every time, which nothing could observe until
/// `/` could type into one (`api::switcher::open` says so and leaves the test here).
///
/// The frame is searched whole here because the switcher's rows have no fixed row number, and
/// the pill that `workspace.create` left is gone by then: the first `/` cleared it, and no key
/// after it sets another.
#[tokio::test]
async fn the_switcher_opens_with_an_empty_filter_each_time() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (_root, _w1, _w2) = h.git_project_with_two_slots().await;
    h.api("switcher.open", json!({})).await.unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains("Projects"),
        Duration::from_secs(2),
    )
    .await;
    h.key(h.client.clone(), "/").await;
    h.type_text(h.client.clone(), "workspace-1").await;
    h.key(h.client.clone(), "Enter").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| !f.contains("workspace-2"),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(filter(&h), ("workspace-1".to_string(), false), "{f}");

    h.key(h.client.clone(), "Esc").await;
    h.wait_for(
        h.client.clone(),
        |f| !f.contains("Projects"),
        Duration::from_secs(2),
    )
    .await;
    h.api("switcher.open", json!({})).await.unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("workspace-2"),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(filter(&h), (String::new(), false), "{f}");
}

/// The box scrolls to keep the cursor in view.
///
/// The box scrolls to keep the cursor in view.
///
/// A third slot and the shortest screen domux draws on, which leaves the box 7 lines to draw
/// 8 in: on a 24 row screen the whole list fits and a hard-coded scroll of 0 passes. The
/// field can only be tested under the condition that makes it matter.
#[tokio::test]
async fn the_box_scrolls_to_keep_the_cursor_in_view_when_it_cannot_show_every_row() {
    let mut h = Harness::start(Config::default(), 120, 10).await;
    let (_root, _w1, w2) = h.git_project_with_two_slots().await;
    let project = h
        .model()
        .project_of_workspace(&w2)
        .map(|p| p.id.to_string())
        .expect("the git project holds the slot");
    h.api("workspace.create", json!({ "project": project }))
        .await
        .expect("a third slot");
    let git_main = h
        .model()
        .project_of_workspace(&w2)
        .and_then(|p| p.workspaces.first().map(|w| w.id.clone()))
        .expect("the git project holds main");
    h.api("workspace.focus", json!({"workspace": git_main.as_str()}))
        .await
        .unwrap();
    h.api("sidebar.show", json!({})).await.unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains("Projects"),
        Duration::from_secs(2),
    )
    .await;
    h.key(h.client.clone(), "C-h").await;
    h.frame(h.client.clone()).await;
    assert_eq!(
        h.model().client(&h.client).unwrap().projects_scroll,
        0,
        "the box starts at the top"
    );

    // Down to the last row, which is the other project's `main`.
    for _ in 0..4 {
        h.key(h.client.clone(), "j").await;
    }
    let f = h.frame(h.client.clone()).await;
    // 1 exactly, and not merely "more than 0". The eight lines are a header, `main`, three
    // slots, the blank before the next header, that header and its `main`, so the last row
    // ends on line 8 and the box shows 7: one line has to go. That number is what tells the
    // compact rows from the switcher's wider ones, which give every workspace a tab list line
    // and would put the same cursor five lines further down.
    assert_eq!(
        h.model().client(&h.client).unwrap().projects_scroll,
        1,
        "the cursor left the view, so the box scrolled by exactly what it had to:\n{f}"
    );
    assert!(
        f.contains("PROJ"),
        "and the row it scrolled to is drawn:\n{f}"
    );
    assert!(
        !f.contains("AUDREY-APP"),
        "while the top of the list has gone off it:\n{f}"
    );

    // Back one row, which is still inside the window the box is showing. `scroll_to_show` is
    // given the scroll the box already has and moves it as little as it can, so this must not
    // move at all: a step handed 0 instead would answer 0, and the whole list would jump back
    // to the top under a `k` that never left the view.
    h.key(h.client.clone(), "k").await;
    let f = h.frame(h.client.clone()).await;
    assert_eq!(
        h.model().client(&h.client).unwrap().projects_scroll,
        1,
        "a step inside the window leaves the view where it is:\n{f}"
    );
}

/// `focus.region` names a region rather than a direction, and every M2 kind refuses when the
/// thing it names is not on the screen.
#[tokio::test]
async fn focus_region_takes_the_m2_kinds_and_refuses_the_ones_that_are_not_showing() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let err = h
        .api("focus.region", json!({"region": "sidebar_projects"}))
        .await
        .expect_err("the sidebar is closed");
    assert_eq!(err.code, domux_core::api::ErrorCode::Refused, "{err}");
    assert!(err.message.contains("sidebar.show"), "{err}");

    let err = h
        .api("focus.region", json!({"region": "switcher"}))
        .await
        .expect_err("the switcher is closed");
    assert_eq!(err.code, domux_core::api::ErrorCode::Refused, "{err}");
    assert!(err.message.contains("switcher.open"), "{err}");

    let err = h
        .api("focus.region", json!({"region": "sidebar_agents"}))
        .await
        .expect_err("M3 builds the agents box");
    assert_eq!(err.code, domux_core::api::ErrorCode::Unavailable, "{err}");
    assert!(err.message.contains("M3"), "{err}");

    h.api("sidebar.show", json!({})).await.unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains("Projects"),
        Duration::from_secs(2),
    )
    .await;
    h.api("focus.region", json!({"region": "sidebar_projects"}))
        .await
        .unwrap();
    assert_eq!(focus(&h), Focus::Region(RegionKind::SidebarProjects));
    assert_eq!(
        cursor(&h),
        Some(h.model().client(&h.client).unwrap().workspace.clone()),
        "and it puts the cursor where the fill already was, as C-h does"
    );

    h.api("switcher.open", json!({})).await.unwrap();
    h.api("focus.region", json!({"region": "switcher"}))
        .await
        .unwrap();
    assert_eq!(focus(&h), Focus::Region(RegionKind::Switcher));
}

/// The box's border is the one drawn in the accent while the keys are in it, and the sidebar's
/// column is 38 cells wide whether or not it has them.
#[tokio::test]
async fn the_box_takes_the_accent_and_gives_it_back() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let f = in_the_box(&mut h).await;
    assert_eq!(
        cols(row(&f, 0), 0, 37),
        "┌ Projects ──────────────────────────┐",
        "{f}"
    );
    assert!(
        f.contains("r0 c1-10 bold fg=#cba6f7"),
        "the title goes bold in the accent with the border:\n{f}"
    );
    h.key(h.client.clone(), "Esc").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("r1 c39-39 fg=#cba6f7"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        !f.contains("r0 c0-0 fg=#cba6f7"),
        "and hands it back to the pane:\n{f}"
    );
    assert_eq!(
        cols(row(&f, 0), 0, 37),
        "┌ Projects ──────────────────────────┐",
        "the box is still drawn, in its unfocused colours:\n{f}"
    );
}

/// A key in the box clears the last result, so the hint row goes back to the keys
/// (interface spec 12.12).
///
/// `workspace.create` leaves `Created workspace-2` in the hint row, and `C-h` is a global
/// binding so it never reaches `list_key`: the pill is still showing when the box takes the
/// keys, and the next key inside the box is what has to remove it. Nothing else in the test
/// removes it - the clock is fixed, so `PILL_SECONDS` never passes - and the assertion is on
/// the field rather than on the hint row alone, which an empty row would also satisfy.
#[tokio::test]
async fn a_key_in_the_box_clears_the_last_result() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let (_root, _w1, _w2) = h.git_project_with_two_slots().await;
    let f = in_the_box(&mut h).await;
    assert!(
        row(&f, 23).contains("Created workspace-2"),
        "the create's result is still in the hint row:\n{}",
        row(&f, 23)
    );
    assert!(h.model().client(&h.client).unwrap().pill.is_some());

    h.key(h.client.clone(), "j").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| row(f, 23).contains("⏎ open"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        h.model().client(&h.client).unwrap().pill.is_none(),
        "the key cleared it:\n{f}"
    );
}

/// A filter that drops the row the cursor was on leaves the box with no fill. Enter then has
/// nothing to open, and the next step starts at the top of what is left rather than at the
/// row the cursor happened to name.
///
/// The client is in the plain folder's `main`, which is the last row in the list and the only
/// one this filter drops: with the cursor anywhere else the filter would keep it and the
/// question would not arise.
#[tokio::test]
async fn a_step_after_the_filter_drops_the_cursor_starts_at_the_top() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let (_root, _w1, _w2) = h.git_project_with_two_slots().await;
    let git_main = h
        .model()
        .project_of_workspace(&_w1)
        .and_then(|p| p.workspaces.first().map(|w| w.id.clone()))
        .expect("the git project holds main");
    in_the_box(&mut h).await;
    let here = cursor(&h);
    assert_eq!(
        here,
        Some(proj_main(&h.model())),
        "the cursor is on the row the filter is about to drop"
    );

    h.key(h.client.clone(), "/").await;
    h.type_text(h.client.clone(), "audrey-app").await;
    h.key(h.client.clone(), "Enter").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| !box_row(f, 8).contains("PROJ"),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(cursor(&h), here, "the filter did not move the cursor:\n{f}");

    let err = h
        .api("list.activate", json!({}))
        .await
        .expect_err("the cursor names a row the box is not drawing");
    assert_eq!(err.code, domux_core::api::ErrorCode::NotFound, "{err}");
    assert!(err.message.contains("under the cursor"), "{err}");

    h.key(h.client.clone(), "j").await;
    h.frame(h.client.clone()).await;
    assert_eq!(
        cursor(&h),
        Some(git_main),
        "and a step lands on the first row that is left, not on the last one"
    );
}

/// The filter row belongs to the box, so a sidebar whose box does not have the keys shows the
/// keys and not `Filter ›`.
///
/// Hiding the sidebar hands the keys back without closing the filter field, so showing it
/// again is the one way to reach a drawn sidebar with `filtering` set and the keys on a pane.
/// Every other way out of the box goes through `focus.pane`, which closes the field.
#[tokio::test]
async fn the_filter_row_belongs_to_the_box_and_not_to_the_sidebar() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    in_the_box(&mut h).await;
    h.key(h.client.clone(), "/").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| row(f, 23).contains("Filter"),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(filter(&h), (String::new(), true), "{f}");

    h.api("sidebar.hide", json!({"client": h.client.as_str()}))
        .await
        .unwrap();
    h.api("sidebar.show", json!({"client": h.client.as_str()}))
        .await
        .unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("┌ Projects"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        matches!(focus(&h), Focus::Pane(_)),
        "hiding the box gave the keys back:\n{f}"
    );
    assert!(
        h.model().client(&h.client).unwrap().filtering,
        "and left the field open, which is what makes this the discriminating state"
    );
    assert!(
        row(&f, 23).contains("hide") && !row(&f, 23).contains("Filter"),
        "so the row shows the sidebar's keys:\n{}",
        row(&f, 23)
    );
}

/// Coming back into the box never lands the reader in a filter field they did not open.
///
/// `sidebar.hide` hands the keys back without closing the field - it is the one exit that does
/// not go through `focus.pane` - so `filtering` is still set when the box is entered again,
/// and `enter_projects_box` is what has to clear it. Nothing else can: `pop_overlay` clears it
/// on the other exits, and this path has no overlay to pop.
#[tokio::test]
async fn coming_back_into_the_box_never_lands_in_a_filter_field_nobody_opened() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    in_the_box(&mut h).await;
    h.key(h.client.clone(), "/").await;
    h.type_text(h.client.clone(), "pro").await;
    h.wait_for(
        h.client.clone(),
        |f| row(f, 23).contains("Filter › pro"),
        Duration::from_secs(2),
    )
    .await;

    h.api("sidebar.hide", json!({"client": h.client.as_str()}))
        .await
        .unwrap();
    h.api("sidebar.show", json!({"client": h.client.as_str()}))
        .await
        .unwrap();
    h.frame(h.client.clone()).await;
    assert!(
        h.model().client(&h.client).unwrap().filtering,
        "hiding the box left the field open, which is what makes the next step the question"
    );

    h.key(h.client.clone(), "C-h").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("r0 c0-0 fg=#cba6f7"),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(
        filter(&h),
        (String::new(), false),
        "and entering the box starts fresh:\n{f}"
    );
    assert!(
        row(&f, 23).contains("⏎ open"),
        "so the row shows the box's keys and not a filter:\n{}",
        row(&f, 23)
    );
    // The key that follows acts, rather than being typed into a field nobody opened.
    h.key(h.client.clone(), "j").await;
    h.frame(h.client.clone()).await;
    assert_eq!(filter(&h).0, "", "`j` moved the cursor rather than typing");
}

/// A chorded letter is not text. Without the guard `C-b` would put a `b` in the filter, which
/// is a key the reader pressed to do something else appearing as a search term.
///
/// `C-b` and not `C-a`: `C-a` is the leader, and a chord is claimed at step 2, so it never
/// reaches the filter at all. `C-b` is bound in `[keys.bindings]`, which is only read after
/// the leader, so it arrives here as an ordinary chorded key.
#[tokio::test]
async fn a_chorded_letter_is_not_typed_into_the_filter() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    in_the_box(&mut h).await;
    h.key(h.client.clone(), "/").await;
    h.key(h.client.clone(), "C-b").await;
    h.key(h.client.clone(), "M-b").await;
    h.frame(h.client.clone()).await;
    assert_eq!(
        filter(&h),
        (String::new(), true),
        "neither chorded letter was typed, and neither closed the field"
    );

    h.key(h.client.clone(), "b").await;
    h.frame(h.client.clone()).await;
    assert_eq!(
        filter(&h),
        ("b".to_string(), true),
        "while the same letter on its own is text"
    );
}

/// `focus.right` from inside an overlay moves nothing: the overlay owns its keys until it
/// closes, and a frame with the keys on a pane under an open overlay marks the wrong thing.
///
/// The switcher's own region and not the sidebar's, which is the only fixture that separates
/// "the sidebar's box hands the keys back" from "any region does".
#[tokio::test]
async fn focus_right_inside_an_overlay_leaves_the_overlay_where_it_is() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    h.api("switcher.open", json!({})).await.unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains("Projects"),
        Duration::from_secs(2),
    )
    .await;
    assert_eq!(focus(&h), Focus::Region(RegionKind::Switcher));

    h.api("focus.right", json!({})).await.unwrap();
    let f = h.frame(h.client.clone()).await;
    assert_eq!(
        h.model().client(&h.client).unwrap().overlay,
        Some(domux_core::model::Overlay::Switcher),
        "the switcher is still open:\n{f}"
    );
    assert_eq!(
        focus(&h),
        Focus::Region(RegionKind::Switcher),
        "and the keys are still in it:\n{f}"
    );
    assert!(f.contains("┌ Projects"), "{f}");
}
