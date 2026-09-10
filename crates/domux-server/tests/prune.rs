//! A record whose path is gone, and the facts the switcher opens with.
//!
//! The author who removes a worktree or a project folder with `rm -rf` is the reader every
//! test here is about: domux did not see it happen, so the next start is the only place the
//! model can be made to match the disk again (architecture spec section 5).
//!
//! Every path any of these tests deletes is one it built itself inside a `tempfile::tempdir`,
//! reached through the harness's own `git_project` helpers or through a directory the test
//! made. Nothing here computes a path from the machine it runs on.

use domux_core::config::Config;
use domux_core::facts::{FactKey, FACT_PR};
use domux_core::ids::WorkspaceId;
use domux_server::testing::{Harness, HarnessOptions};
use domux_server::FixedClock;
use serde_json::json;
use std::path::PathBuf;
use std::time::Duration;

/// The clock every harness server runs on. A cached fact stamped from anything else is
/// either stale or in the future, and `FactRegistry::load_cache` drops both.
const HARNESS_NOW: &str = "2026-09-04T14:32:00";

/// The handles of one project's workspaces, in the order the model holds them.
fn handles(h: &Harness, project: &str) -> Vec<String> {
    h.model()
        .projects
        .iter()
        .find(|p| p.name == project)
        .unwrap_or_else(|| panic!("no project named {project}"))
        .workspaces
        .iter()
        .map(|w| w.handle.to_string())
        .collect()
}

/// The path of one workspace of one project, as the model records it.
fn path_of(h: &Harness, project: &str, handle: &str) -> PathBuf {
    h.model()
        .projects
        .iter()
        .find(|p| p.name == project)
        .unwrap_or_else(|| panic!("no project named {project}"))
        .workspaces
        .iter()
        .find(|w| w.handle.to_string() == handle)
        .unwrap_or_else(|| panic!("no workspace {handle} in {project}"))
        .path
        .clone()
}

/// One row of the frame's `|...|` block, without the bars.
fn row(frame: &str, y: usize) -> String {
    frame
        .lines()
        .filter(|l| l.starts_with('|'))
        .nth(y)
        .unwrap_or_else(|| panic!("no row {y} in the frame"))
        .trim_matches('|')
        .to_string()
}

/// Cells `from` to `to` of one row, counted in cells. Every helper here counts characters
/// rather than bytes: a frame row is full of box drawing, and a byte offset into one lands
/// inside a glyph.
fn cells(row: &str, from: usize, to: usize) -> String {
    row.chars().skip(from).take(to + 1 - from).collect()
}

/// The number of the first row whose text holds `needle`.
fn row_holding(frame: &str, needle: &str) -> usize {
    frame
        .lines()
        .filter(|l| l.starts_with('|'))
        .position(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("no row holds {needle:?} in:\n{frame}"))
}

/// The style of one cell, read out of the frame's own style dump, whose lines are
/// `r{row} c{from}-{to} [attrs] fg=# bg=#`. Reading the run that covers the cell rather than
/// matching a whole line keeps the assertion off the length of the text in it, which is the
/// message of whatever job happened to fail.
fn style_at(frame: &str, y: usize, x: usize) -> String {
    for line in frame.lines() {
        let Some(rest) = line.strip_prefix(&format!("r{y} c")) else {
            continue;
        };
        let (span, style) = rest.split_once(' ').unwrap_or((rest, ""));
        let (from, to) = span.split_once('-').expect("a style run is c<from>-<to>");
        let (from, to) = (from.parse().unwrap(), to.parse().unwrap());
        if (from..=to).contains(&x) {
            return style.to_string();
        }
    }
    panic!("no style covers r{y} c{x} in:\n{frame}")
}

/// The cell `needle` starts at in `row`.
fn cell_of(row: &str, needle: &str) -> usize {
    let byte = row
        .find(needle)
        .unwrap_or_else(|| panic!("no {needle:?} in {row:?}"));
    row[..byte].chars().count()
}

/// A workspace record whose worktree the author removed goes at the next start, and the
/// switcher's footer says so.
///
/// The fixture holds three records the prune must leave alone - the git project's `main` and
/// `workspace-1`, and the harness's own folder project - so an implementation that removed
/// every slot, or every record, or the whole project, fails here rather than passing on the
/// one record it was supposed to take.
#[tokio::test]
async fn a_worktree_removed_outside_domux_is_pruned_at_start_and_noted_in_the_switcher() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (root, _w1, _w2) = h.git_project_with_two_slots().await;
    assert_eq!(
        handles(&h, "audrey-app"),
        vec!["main", "workspace-1", "workspace-2"],
        "the fixture starts with all three"
    );
    h.stop().await;
    std::fs::remove_dir_all(root.join(".domux/worktrees/workspace-2")).unwrap();
    h.restart().await;

    assert_eq!(
        handles(&h, "audrey-app"),
        vec!["main", "workspace-1"],
        "the record for a path that is gone goes with it, and the two that are still there stay"
    );
    assert_eq!(
        handles(&h, "proj"),
        vec!["main"],
        "and a project the prune had no business touching is untouched"
    );

    h.api("switcher.open", json!({})).await.unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("Navigator"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        f.contains("Pruned workspace-2: its worktree is gone"),
        "the footer says what happened:\n{f}"
    );
    assert!(
        !f.contains("Removed"),
        "and says only that: the project is still there\n{f}"
    );
}

/// A project whose root is gone leaves with every workspace under it.
#[tokio::test]
async fn a_project_whose_root_is_gone_leaves_with_its_workspaces() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("scratch");
    std::fs::create_dir_all(&root).unwrap();
    let mut h = Harness::start(Config::default(), 80, 24).await;
    h.api("project.add", json!({"path": root.to_str().unwrap()}))
        .await
        .unwrap();
    assert!(
        h.model().projects.iter().any(|p| p.name == "scratch"),
        "the fixture registered it"
    );
    h.stop().await;
    std::fs::remove_dir_all(&root).unwrap();
    h.restart().await;

    let projects = h.api("project.list", json!({})).await.unwrap();
    assert!(
        projects
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["name"] != "scratch"),
        "{projects}"
    );
    assert!(
        h.model()
            .projects
            .iter()
            .flat_map(|p| p.workspaces.iter())
            .all(|w| !w.path.starts_with(&root)),
        "and no workspace of it is left behind under a project that is gone"
    );
}

/// The project loop runs before the workspace loop, so a project whose root went away is
/// reported once, as a project, and not slot by slot.
///
/// This is the fixture that separates the two orders. With a folder project they agree: its
/// only workspace is `main`, `prune_workspace` refuses `main`, and the workspace loop is
/// silent either way. A git project with two slots is where they part, because both slots
/// live under the root: run the workspace loop first and the reader gets three lines about a
/// project that has one thing wrong with it.
#[tokio::test]
async fn a_git_project_whose_root_is_gone_is_reported_once_and_not_slot_by_slot() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (root, _w1, _w2) = h.git_project_with_two_slots().await;
    assert_eq!(
        handles(&h, "audrey-app").len(),
        3,
        "both slots are under the root that is about to go"
    );
    h.stop().await;
    std::fs::remove_dir_all(&root).unwrap();
    h.restart().await;

    assert!(
        !h.model().projects.iter().any(|p| p.name == "audrey-app"),
        "the project went with its root"
    );
    h.api("switcher.open", json!({})).await.unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("Navigator"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        f.contains("Removed audrey-app: its folder is gone"),
        "one line, about the project:\n{f}"
    );
    assert!(
        !f.contains("Pruned"),
        "and not one line per slot: a project is its root\n{f}"
    );
}

/// The pull request survives a restart, so the switcher opens with the last known number
/// rather than a blank line while `gh` is still running.
///
/// No provider is registered, so nothing here can fetch: the number on the screen came off
/// the cache file or it came from nowhere.
#[tokio::test]
async fn the_pull_request_cache_is_read_at_start_so_the_switcher_is_not_blank() {
    let tmp = tempfile::tempdir().unwrap();
    let state = tmp.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let mut h = Harness::start_with(HarnessOptions {
        state_dir: Some(state.clone()),
        ..HarnessOptions::new(Config::default(), 80, 24)
    })
    .await;
    let (_root, w1, _w2) = h.git_project_with_two_slots().await;
    h.stop().await;
    // Stamped from the clock the server runs on, not from the wall clock: a fact from the
    // future is not fresh, and `Local::now()` is in the future of a fixed clock.
    let at = FixedClock::at(HARNESS_NOW).0.to_rfc3339();
    std::fs::write(
        state.join("pr-cache.json"),
        format!(
            r#"{{"schema_version":1,"facts":{{"{w1}/pr":{{"text":"PR#212","state":"OPEN","fetched_at":"{at}","ttl":600}}}}}}"#
        ),
    )
    .unwrap();
    h.restart().await;

    h.api("switcher.open", json!({})).await.unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("PR#212"),
            Duration::from_secs(3),
        )
        .await;
    assert!(f.contains("PR#212"), "{f}");
    let y = row_holding(&f, "PR#212");
    let from = cell_of(&row(&f, y), "PR#212");
    assert!(
        f.contains(&format!("r{y} c{from}-{} fg=#a6e3a1", from + 5)),
        "an open pull request is green, in those cells and no others:\n{f}"
    );
}

/// A note is gone once the reader has been in a box with it on the screen.
#[tokio::test]
async fn a_note_clears_on_the_first_key_in_a_box() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (root, _w1, _w2) = h.git_project_with_two_slots().await;
    h.stop().await;
    std::fs::remove_dir_all(root.join(".domux/worktrees/workspace-2")).unwrap();
    h.restart().await;

    h.api("switcher.open", json!({})).await.unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("Navigator"),
            Duration::from_secs(2),
        )
        .await;
    assert!(f.contains("Pruned workspace-2"), "the note is there:\n{f}");

    // `j` is bound to `list.down`, which Task 14 has not built, so this key does nothing at
    // all except be a key in a box. The note going is the whole visible result.
    h.key(h.client.clone(), "j").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| !f.contains("Pruned workspace-2"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        f.contains("filter") && f.contains("close"),
        "and the footer goes back to the keys:\n{f}"
    );
}

/// The same rule in the sidebar's Agents box, which M3 added as a second place a key can land
/// in a box.
///
/// The note going is only half of what the reader sees. The hint row prints the note in place
/// of the keys, so a note that will not clear leaves the box refusing to say what to press and
/// refusing to stop hiding it, which is worse than a stale line: the reader is stuck. Both
/// halves are asserted, because the first on its own passes on a box whose keys never come
/// back.
#[tokio::test]
async fn a_note_clears_on_the_first_key_in_the_sidebar_agents_box() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (root, _w1, _w2) = h.git_project_with_two_slots().await;
    h.stop().await;
    std::fs::remove_dir_all(root.join(".domux/worktrees/workspace-2")).unwrap();
    h.restart().await;

    h.api("sidebar.show", json!({})).await.unwrap();
    h.api("focus.region", json!({"region": "sidebar_agents"}))
        .await
        .unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("Pruned workspace-2"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        !f.contains("more"),
        "the note has the hint row, so the box is showing no keys:\n{f}"
    );

    // `j` is `list.down` over an empty box, so it does nothing at all except be a key in a
    // box. What follows is the whole visible result.
    h.key(h.client.clone(), "j").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| !f.contains("Pruned workspace-2"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        f.contains("open") && f.contains("more"),
        "and the hint row goes back to the keys the box was hiding:\n{f}"
    );
}

/// And in the agents overlay, the other place M3 added. Its footer orders the note and the
/// keys the way the sidebar's hint row does, so it hides them the same way.
#[tokio::test]
async fn a_note_clears_on_the_first_key_in_the_agents_overlay() {
    // The agents overlay is the surface the two-box layout keeps (decision record 0028).
    let mut config = Config::default();
    config.navigator.enabled = false;
    let mut h = Harness::start(config, 80, 24).await;
    let (root, _w1, _w2) = h.git_project_with_two_slots().await;
    h.stop().await;
    std::fs::remove_dir_all(root.join(".domux/worktrees/workspace-2")).unwrap();
    h.restart().await;

    h.api("agents.open", json!({})).await.unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("Pruned workspace-2"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        f.contains("┌ Agents") && !f.contains("filter"),
        "the note has the footer, so the overlay is showing no keys:\n{f}"
    );

    h.key(h.client.clone(), "j").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| !f.contains("Pruned workspace-2"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        f.contains("open") && f.contains("filter") && f.contains("close"),
        "and the footer goes back to the keys it was hiding:\n{f}"
    );
}

/// A note is the server's, and the box a key was in is one client's. A key another client
/// sent from a pane does not read a note on behalf of the reader looking at the switcher.
///
/// Two clients, because with one the client the key came from and the only client there is
/// are the same client, and a rule reading either would pass.
#[tokio::test]
async fn a_key_another_client_sent_from_a_pane_does_not_clear_the_note() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (root, _w1, _w2) = h.git_project_with_two_slots().await;
    h.stop().await;
    std::fs::remove_dir_all(root.join(".domux/worktrees/workspace-2")).unwrap();
    h.restart().await;
    let reader = h.client.clone();
    let other = h.attach(80, 24).await;

    // The switcher belongs to the first client; the second is in its pane.
    h.api("switcher.open", json!({"client": reader.to_string()}))
        .await
        .unwrap();
    let f = h
        .wait_for(
            reader.clone(),
            |f| f.contains("Navigator"),
            Duration::from_secs(2),
        )
        .await;
    assert!(f.contains("Pruned workspace-2"), "the note is there:\n{f}");

    h.key(other.clone(), "x").await;
    let f = h
        .wait_for(
            reader.clone(),
            |f| f.contains("Navigator"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        f.contains("Pruned workspace-2: its worktree is gone"),
        "the other client typed into a pane, so the note is still unread:\n{f}"
    );

    h.key(reader.clone(), "j").await;
    let f = h
        .wait_for(
            reader.clone(),
            |f| !f.contains("Pruned workspace-2"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        f.contains("filter"),
        "and the reader's own key in the box clears it:\n{f}"
    );
}

/// Every record whose path is gone goes, not the first one. `rm -rf .domux` takes every
/// worktree of a project at once, which is the fixture here: with one gone slot a prune that
/// stopped after the first would look exactly like one that did not.
#[tokio::test]
async fn every_worktree_that_is_gone_is_pruned_and_named_not_only_the_first() {
    // 120 columns so the footer has room for both notes whole: at 80 the second is cut at
    // "workspa…", which is a line that a prune of only the first could also have produced.
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let (root, _w1, _w2) = h.git_project_with_two_slots().await;
    h.stop().await;
    std::fs::remove_dir_all(root.join(".domux/worktrees")).unwrap();
    h.restart().await;

    assert_eq!(
        handles(&h, "audrey-app"),
        vec!["main"],
        "both slots went, and main, whose path is the project's root, stayed"
    );
    h.api("switcher.open", json!({})).await.unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("Pruned"),
            Duration::from_secs(2),
        )
        .await;
    let y = row_holding(&f, "Pruned");
    // The footer is inside the box now (MUX-16), so the pane's border, the screen behind it,
    // the box's own border and its two pad cells all come before the note.
    assert_eq!(
        cells(&row(&f, y), 0, 97),
        "│           │  Pruned workspace-1: its worktree is gone · Pruned workspace-2: its worktree is gone",
        "both are named, in slot order:\n{f}"
    );
}

/// A key in a pane is not a key in a box. The sidebar stands beside the pane the reader is
/// typing in, so a note that cleared on any key at all would go during the first keystroke
/// of the day, which is the one moment nobody is looking at it.
#[tokio::test]
async fn a_note_survives_a_key_that_went_to_a_pane() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (root, _w1, _w2) = h.git_project_with_two_slots().await;
    h.stop().await;
    std::fs::remove_dir_all(root.join(".domux/worktrees/workspace-2")).unwrap();
    h.restart().await;

    h.key(h.client.clone(), "x").await;
    h.key(h.client.clone(), "y").await;
    h.api("switcher.open", json!({})).await.unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("Navigator"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        f.contains("Pruned workspace-2: its worktree is gone"),
        "two keys went to the pane and the note is still waiting to be read:\n{f}"
    );
}

/// The sidebar's hint row prints the note in place of its keys, and gives it up for a pill.
///
/// The pill here is a real one, from a real `workspace.create`, so what is being ordered is
/// the two things a running server can actually put in that row at the same time.
#[tokio::test]
async fn the_sidebar_hint_row_shows_the_note_under_a_pill_and_over_the_keys() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let (root, _w1, _w2) = h.git_project_with_two_slots().await;
    h.stop().await;
    std::fs::remove_dir_all(root.join(".domux/worktrees/workspace-2")).unwrap();
    h.restart().await;

    h.api("sidebar.show", json!({})).await.unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("Pruned workspace-2"),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(
        cells(&row(&f, 23), 0, 37),
        " Pruned workspace-2: its worktree is… ",
        "the note takes the hint row, cut to the sidebar's width:\n{f}"
    );
    assert_eq!(
        style_at(&f, 23, 1),
        "fg=#cdd6f4",
        "in text, which is neither a pill's colours nor the blue of a key:\n{f}"
    );

    // A job that fails answers with a red pill, and the note has not been read yet: the row
    // can only say one of them.
    let _ = h
        .api("project.add", json!({"path": "/no/such/folder"}))
        .await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| !f.contains("Pruned workspace-2"),
            Duration::from_secs(5),
        )
        .await;
    assert_eq!(
        style_at(&f, 23, 1),
        "bold fg=#1e1e2e bg=#f38ba8",
        "a pill is an answer to what the reader just did, so it goes over the note:\n{f}"
    );
}

/// The switcher's footer orders the same three the same way, and cuts a note too long for it.
///
/// The mutant this is here for lives in `overlay::footer`, and every test of the ordering in
/// the sidebar's hint row is on the far side of a boundary from it: the two rows are drawn by
/// different functions. Both worktrees are gone rather than one, so the note is 83 cells in a
/// 54 cell row and the cut is visible: `put_within` clips at the row's end whatever it is
/// handed, so only the ellipsis tells a note that was shortened from one that was chopped.
#[tokio::test]
async fn the_switcher_footer_shows_the_note_under_a_pill_and_over_the_keys() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (root, _w1, _w2) = h.git_project_with_two_slots().await;
    h.stop().await;
    std::fs::remove_dir_all(root.join(".domux/worktrees")).unwrap();
    h.restart().await;

    h.api("switcher.open", json!({})).await.unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("Pruned workspace-1"),
            Duration::from_secs(2),
        )
        .await;
    let y = row_holding(&f, "Pruned workspace-1");
    assert_eq!(
        row(&f, y),
        "│         │  Pruned workspace-1: its worktree is gone · Pruned wor…  │         │",
        "the note has the footer row, not a share of it, and ends in an ellipsis:\n{f}"
    );
    assert_eq!(
        style_at(&f, y, 13),
        "fg=#cdd6f4 bg=#1e1e2e",
        "drawn in text, from the footer's first column:\n{f}"
    );

    let _ = h
        .api("project.add", json!({"path": "/no/such/folder"}))
        .await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| !f.contains("Pruned workspace-1"),
            Duration::from_secs(5),
        )
        .await;
    assert_eq!(
        style_at(&f, y, 13),
        "bold fg=#1e1e2e bg=#f38ba8",
        "and gives the row up to a pill:\n{f}"
    );
}

/// Everything that went at one start is named in one line, so a reader who was away for a
/// week is told all of it rather than the first thing and a count.
///
/// Three records, of both kinds: two gone folders and one gone worktree. Two projects rather
/// than one because a project loop that stopped after the first would still have removed
/// something and still have written a note; the second folder is what separates "every
/// project whose root is gone" from "the first one". The screen is 150 columns so all three
/// notes fit whole, since a line cut short is one a shorter list of notes could have made.
#[tokio::test]
async fn every_record_gone_at_one_start_is_named_in_the_note_row() {
    let dir = tempfile::tempdir().unwrap();
    let alpha = dir.path().join("alpha");
    let beta = dir.path().join("beta");
    std::fs::create_dir_all(&alpha).unwrap();
    std::fs::create_dir_all(&beta).unwrap();
    let mut h = Harness::start(Config::default(), 150, 24).await;
    let (root, _w1, _w2) = h.git_project_with_two_slots().await;
    for folder in [&alpha, &beta] {
        h.api("project.add", json!({"path": folder.to_str().unwrap()}))
            .await
            .unwrap();
    }
    h.stop().await;
    std::fs::remove_dir_all(&alpha).unwrap();
    std::fs::remove_dir_all(&beta).unwrap();
    std::fs::remove_dir_all(root.join(".domux/worktrees/workspace-2")).unwrap();
    h.restart().await;

    h.api("switcher.open", json!({})).await.unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("Removed alpha"),
            Duration::from_secs(2),
        )
        .await;
    let y = row_holding(&f, "Removed alpha");
    assert_eq!(
        row(&f, y),
        "│              │  Removed alpha: its folder is gone · Removed beta: its folder is gone · Pruned workspace-2: its worktree is gone     │              │",
        "all three, in one line, the projects first and in the order the model holds them:\n{f}"
    );
}

/// When the prune leaves nothing at all, the server seeds a project from the directory it was
/// started in. A server with no workspace refuses every client at `attach`, so without the
/// seed this test does not fail an assertion, it fails to get a client back.
#[tokio::test]
async fn a_server_whose_every_project_is_gone_still_takes_a_client() {
    let tmp = tempfile::tempdir().unwrap();
    let state = tmp.path().join("state");
    let only = tmp.path().join("only");
    std::fs::create_dir_all(&state).unwrap();
    std::fs::create_dir_all(&only).unwrap();
    let mut h = Harness::start_with(HarnessOptions {
        state_dir: Some(state),
        project_root: Some(only.clone()),
        ..HarnessOptions::new(Config::default(), 80, 24)
    })
    .await;
    assert_eq!(
        h.model().projects.len(),
        1,
        "the fixture holds exactly the one project whose folder is about to go"
    );
    h.stop().await;
    std::fs::remove_dir_all(&only).unwrap();
    h.restart().await;

    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("only"),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(
        h.model().projects.len(),
        1,
        "the record that named a missing folder went, and one was seeded in its place:\n{f}"
    );
}

/// The prune runs before anything is spawned, so no shell is started in a directory that is
/// not there.
#[tokio::test]
async fn no_pane_is_started_in_a_worktree_that_is_gone() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (root, _w1, _w2) = h.git_project_with_two_slots().await;
    let gone = path_of(&h, "audrey-app", "workspace-2");
    let spawner = h.spawner.clone().expect("the fake spawner");
    let before = spawner.requests().len();
    h.stop().await;
    std::fs::remove_dir_all(root.join(".domux/worktrees/workspace-2")).unwrap();
    h.restart().await;

    let after = spawner.requests();
    assert!(
        after.len() > before,
        "the restart did spawn the panes of the workspaces that are still there"
    );
    assert!(
        after[before..].iter().all(|r| r.cwd != gone),
        "and none of them in the worktree that is gone: {:?}",
        after[before..].iter().map(|r| &r.cwd).collect::<Vec<_>>()
    );
}

/// What the cache knew about a pruned workspace goes with the record, and what it knew about
/// the one beside it stays. Without the second half this passes against a prune that forgets
/// every fact it has.
///
/// What this does **not** establish is that the start is what forgot it. `start_due_fetches`
/// calls `forget_deleted` on every tick, and a tick has run by the time this reads the
/// published facts, so the two orders are indistinguishable here. The claim that the start
/// does it, before a tick has had the chance, is carried by
/// `core::tests::the_start_forgets_a_pruned_workspaces_facts_before_any_tick_runs`, which
/// reads a `Core` that has never ticked.
#[tokio::test]
async fn the_facts_of_a_pruned_workspace_are_forgotten_and_its_neighbour_keeps_its_own() {
    let tmp = tempfile::tempdir().unwrap();
    let state = tmp.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let mut h = Harness::start_with(HarnessOptions {
        state_dir: Some(state.clone()),
        ..HarnessOptions::new(Config::default(), 80, 24)
    })
    .await;
    let (root, w1, w2) = h.git_project_with_two_slots().await;
    h.stop().await;
    let at = FixedClock::at(HARNESS_NOW).0.to_rfc3339();
    let pr =
        |text: &str| format!(r#"{{"text":"{text}","state":"OPEN","fetched_at":"{at}","ttl":600}}"#);
    std::fs::write(
        state.join("pr-cache.json"),
        format!(
            r#"{{"schema_version":1,"facts":{{"{w1}/pr":{},"{w2}/pr":{}}}}}"#,
            pr("PR#101"),
            pr("PR#202")
        ),
    )
    .unwrap();
    std::fs::remove_dir_all(root.join(".domux/worktrees/workspace-2")).unwrap();
    h.restart().await;

    assert_eq!(
        h.fact(&FactKey::workspace(&WorkspaceId(w1.to_string()), FACT_PR))
            .map(|f| f.text),
        Some("PR#101".to_string()),
        "the workspace that is still there keeps what the cache knew"
    );
    assert_eq!(
        h.fact(&FactKey::workspace(&WorkspaceId(w2.to_string()), FACT_PR)),
        None,
        "and the pruned one's pull request is not kept for a record nothing holds"
    );
}
