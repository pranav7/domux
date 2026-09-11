//! The Navigator: one box holding projects, their workspaces, and the agents running in each
//! (decision record 0030; artboard 9 is the specification for every row grammar here).
//!
//! `[navigator] enabled` is on by default, so these tests take the default config. The two
//! boxes it replaces are tested in `agents_sidebar.rs` and `agents_overlay.rs`, which turn it
//! off, and both files go when it does.

mod support;

use domux_core::config::Config;
use domux_core::ids::PaneId;
use domux_core::model::agent::AgentKind;
use domux_core::model::{Focus, Overlay, RegionKind, RowTarget};
use domux_server::testing::{row, Harness};
use serde_json::json;
use std::time::Duration;

const WAIT: Duration = Duration::from_secs(2);

/// The frame's screen rows, without its size and cursor lines.
fn screen_rows(f: &str) -> Vec<&str> {
    f.lines().filter(|l| l.starts_with('|')).collect()
}

/// The screen row holding `needle`, or a panic naming the frame.
fn row_with(f: &str, needle: &str) -> usize {
    screen_rows(f)
        .iter()
        .position(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("no row holds {needle:?}:\n{f}"))
}

const WORKS: &str = r#"{"hook_event_name":"UserPromptSubmit","session_id":"s1"}"#;
const WAITS: &str =
    r#"{"hook_event_name":"Notification","session_id":"s1","message":"needs permission"}"#;
const STOPS: &str = r#"{"hook_event_name":"Stop","session_id":"s1"}"#;
const ENDS: &str = r#"{"hook_event_name":"SessionEnd","session_id":"s1"}"#;

/// The sidebar with the Navigator open and one claude working in the client's own pane.
async fn one_agent(h: &mut Harness) -> PaneId {
    let pane = h.focused_pane(h.client.clone());
    h.report(pane.clone(), AgentKind::Claude, WORKS).await;
    h.api("sidebar.show", json!({})).await.unwrap();
    h.wait_for(h.client.clone(), |f| f.contains("Navigator"), WAIT)
        .await;
    pane
}

/// The box's title says what it holds, and it is the same title on both surfaces.
#[tokio::test]
async fn one_box_called_navigator_takes_the_column_and_the_switcher() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    one_agent(&mut h).await;
    let f = h.frame(h.client.clone()).await;
    assert!(f.contains("┌ Navigator"), "{f}");
    assert!(
        !f.contains("┌ Agents"),
        "the Agents box is gone, so the one box takes the column:\n{f}"
    );
    // Its border runs to the hint row, which is the last row of the column.
    let bottom = row_with(&f, "└─────");
    assert_eq!(
        bottom, 22,
        "the box ends one row above the hint row on a 24-row screen:\n{f}"
    );

    h.api("switcher.open", json!({})).await.unwrap();
    let f = h
        .wait_for(h.client.clone(), |f| f.contains("┌ Navigator ──"), WAIT)
        .await;
    assert!(f.contains("┌ Navigator"), "{f}");
}

/// The row grammar of artboard 9, frame 9.2: four cells of lead, the arrow, the label, then
/// the activity. Its project and workspace are the rows above it and are never repeated.
#[tokio::test]
async fn an_agent_is_a_row_under_the_workspace_it_runs_in() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    one_agent(&mut h).await;
    let f = h.frame(h.client.clone()).await;
    let main = row_with(&f, "  main");
    let agent = row_with(&f, "⌞ claude");
    assert_eq!(agent, main + 1, "directly under its workspace:\n{f}");
    let line = row(&f, agent);
    assert!(
        line.starts_with("|│   ⌞ claude  "),
        "two cells of workspace indent, then the arrow, then the label: {line:?}"
    );
    assert!(
        !line.contains("proj"),
        "the project is the header above, not on the row: {line:?}"
    );
}

/// The dot is drawn only while an agent is waiting, and it sits where the working word sits.
#[tokio::test]
async fn only_a_waiting_row_draws_a_dot_and_it_follows_the_name() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let pane = one_agent(&mut h).await;
    let f = h.frame(h.client.clone()).await;
    assert!(
        !row(&f, row_with(&f, "⌞ claude")).contains('●'),
        "a working row says so with its glyph and its word:\n{f}"
    );

    h.report(pane.clone(), AgentKind::Claude, WAITS).await;
    let f = h
        .wait_for(h.client.clone(), |f| f.contains("claude  ●"), WAIT)
        .await;
    let line = row(&f, row_with(&f, "⌞ claude"));
    assert!(
        line.contains("⌞ claude  ●"),
        "the dot is two cells after the label, in the working word's slot: {line:?}"
    );

    h.report(pane, AgentKind::Claude, STOPS).await;
    let f = h
        .wait_for(h.client.clone(), |f| !f.contains("claude  ●"), WAIT)
        .await;
    assert!(
        f.contains("⌞ claude"),
        "an idle row is still there and says nothing:\n{f}"
    );
}

/// A session that ends takes its row with it, and the workspace above it closes up.
#[tokio::test]
async fn a_session_that_ends_takes_its_row_out_of_the_list() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let pane = one_agent(&mut h).await;
    h.report(pane, AgentKind::Claude, ENDS).await;
    let f = h
        .wait_for(h.client.clone(), |f| !f.contains("⌞ claude"), WAIT)
        .await;
    assert!(f.contains("  main"), "the workspace stays:\n{f}");
    assert!(h.agents().await.is_empty(), "and the record is gone");
}

/// One cursor walks both kinds of row, and Enter acts on whichever kind is under it.
#[tokio::test]
async fn the_cursor_walks_workspaces_and_agents_and_enter_acts_on_either() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let pane = one_agent(&mut h).await;
    h.key(h.client.clone(), "C-h").await;
    h.frame(h.client.clone()).await;
    let cursor = |h: &Harness| {
        h.model()
            .client(&h.client)
            .unwrap()
            .navigator_cursor
            .clone()
    };
    assert!(
        matches!(cursor(&h), Some(RowTarget::Workspace(_))),
        "it starts on the workspace this client is in"
    );

    h.key(h.client.clone(), "j").await;
    h.frame(h.client.clone()).await;
    let on_the_agent = cursor(&h);
    assert!(
        matches!(on_the_agent, Some(RowTarget::Agent(_))),
        "one step down is the agent row under it, got {on_the_agent:?}"
    );

    // Enter on it focuses the pane that agent runs in, which is what the issue asked for.
    h.key(h.client.clone(), "Enter").await;
    h.frame(h.client.clone()).await;
    assert_eq!(h.focused_pane(h.client.clone()), pane);
    assert!(
        matches!(h.model().client(&h.client).unwrap().focus, Focus::Pane(_)),
        "and the keys go back to the pane"
    );
}

/// `leader a` opens the agents overlay with the Navigator on: the agents alone, grouped under
/// a header per project, over the one list rather than in place of it (decision record 0033).
#[tokio::test]
async fn leader_a_opens_the_agents_overlay_over_the_navigator() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    one_agent(&mut h).await;
    h.api("agents.open", json!({})).await.expect("it opens");
    let f = h
        .wait_for(h.client.clone(), |f| f.contains("┌ Agents"), WAIT)
        .await;
    // The overlay's grouping: the project is a header over the group, where the Navigator
    // makes it one of the rows above the agent.
    let header = row_with(&f, "PROJ");
    let agent = row_with(&f, "claude");
    assert!(header < agent, "the project heads the group:\n{f}");
    let view = h.model().client(&h.client).unwrap().clone();
    assert_eq!(view.overlay, Some(Overlay::Agents));
    assert_eq!(view.focus, Focus::Region(RegionKind::AgentsOverlay));
}

/// The switcher has the width for what the sidebar drops: the tab, and the recap.
#[tokio::test]
async fn the_switcher_adds_the_tab_and_the_recap_the_sidebar_has_no_room_for() {
    let dir = tempfile::tempdir().unwrap();
    let transcript = dir.path().join("s.jsonl");
    std::fs::write(
        &transcript,
        "{\"type\":\"system\",\"subtype\":\"away_summary\",\"timestamp\":\"2026-09-04T10:21:00.000Z\",\"content\":\"Replaced three session checks with one guard.\"}\n",
    )
    .unwrap();
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let pane = h.focused_pane(h.client.clone());
    let payload = format!(
        r#"{{"hook_event_name":"Stop","session_id":"s1","transcript_path":"{}"}}"#,
        transcript.to_str().unwrap()
    );
    h.report(pane, AgentKind::Claude, &payload).await;
    h.api("switcher.open", json!({})).await.unwrap();
    // The recap reaches the record on the core's tick rather than on the hook above (decision
    // record 0034), so the row arrives before the line under it does.
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("Replaced three session checks"),
            Duration::from_secs(5),
        )
        .await;
    assert!(
        f.contains("⌞ claude"),
        "the recap is a line of the agent's own row:\n{f}"
    );
}
