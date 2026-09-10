//! The count at the left end of the top bar: the agent list folded to one cell (interface
//! spec 6.8). Only while the sidebar is not drawn and only while there is something to count.

use domux_core::config::Config;
use domux_core::model::agent::AgentKind;
use domux_server::testing::{row, Harness};
use serde_json::json;
use std::time::Duration;

#[tokio::test]
async fn the_count_appears_at_the_left_end_only_when_an_agent_needs_you() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    let f = h.frame(h.client.clone()).await;
    assert!(!f.contains("● "), "nothing to count, so no cell:\n{f}");
    let pane = h.focused_pane(h.client.clone());
    h.report(
        pane.clone(),
        AgentKind::Claude,
        r#"{"hook_event_name":"SessionStart","session_id":"c1"}"#,
    )
    .await;
    let f = h.frame(h.client.clone()).await;
    assert!(!f.contains("● "), "an idle seen agent has no red dot:\n{f}");
    h.report(
        pane.clone(),
        AgentKind::Claude,
        r#"{"hook_event_name":"Notification","session_id":"c1","message":"needs permission"}"#,
    )
    .await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("● 1"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        row(&f, 0).starts_with("| ● 1 "),
        "the cell is at the left end:\n{f}"
    );
    assert!(
        f.contains("r0 c1-1 fg=#f38ba8 bg=#313244"),
        "a surface0 fill with a red dot:\n{f}"
    );
}

#[tokio::test]
async fn the_count_is_every_project_not_this_workspace() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    let first = h.focused_pane(h.client.clone());
    h.api("tab.create", json!({})).await.unwrap();
    let second = h.focused_pane(h.client.clone());
    for (pane, sid) in [(first, "c1"), (second, "c2")] {
        h.report(
            pane.clone(),
            AgentKind::Claude,
            &format!(
                "{{\"hook_event_name\":\"Notification\",\"session_id\":\"{sid}\",\"message\":\"needs permission\"}}"
            ),
        )
        .await;
    }
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("● 2"),
            Duration::from_secs(2),
        )
        .await;
    assert!(row(&f, 0).starts_with("| ● 2 "), "{f}");
}

#[tokio::test]
async fn the_count_goes_when_the_sidebar_is_open_because_the_box_is_there() {
    let mut h = Harness::start(Config::default(), 120, 30).await;
    let pane = h.focused_pane(h.client.clone());
    h.report(
        pane,
        AgentKind::Claude,
        r#"{"hook_event_name":"Notification","session_id":"c1","message":"needs permission"}"#,
    )
    .await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("● 1"),
        Duration::from_secs(2),
    )
    .await;
    h.api("sidebar.show", json!({})).await.unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("┌ Agents"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        !row(&f, 0).contains("● 1"),
        "the sidebar replaces the count:\n{f}"
    );
}

/// The gate is `sidebar_visible`, not the remembered `sidebar_open` intent. A client too
/// narrow for the sidebar still draws this bar even after the sidebar is asked for on
/// another client, and that client's own frame is where the count has to survive: nothing
/// else on that screen shows the agent needs attention. Using `sidebar_open` instead would
/// pass every other test in this file - both clients below end up with `sidebar_open ==
/// true` - and still be wrong, because the box that is supposed to displace the count is not
/// the one drawn here.
#[tokio::test]
async fn a_client_that_did_not_ask_for_the_sidebar_still_shows_the_count() {
    let mut h = Harness::start(Config::default(), 119, 24).await;
    let pane = h.focused_pane(h.client.clone());
    h.report(
        pane,
        AgentKind::Claude,
        r#"{"hook_event_name":"Notification","session_id":"c1","message":"needs permission"}"#,
    )
    .await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("● 1"),
        Duration::from_secs(2),
    )
    .await;
    h.api("sidebar.show", json!({ "client": h.client.to_string() }))
        .await
        .unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains("┌ Projects"),
        Duration::from_secs(2),
    )
    .await;
    // Attached after the show, deliberately: a client attaching onto a sidebar that is
    // already remembered open still auto-hides on a narrow screen it never asked on (see
    // `api::sidebar::set` and `sidebar.rs`'s
    // `asking_for_the_sidebar_on_a_narrow_screen_leaves_another_narrow_client_hidden`).
    let other = h.attach(119, 24).await;
    let f = h
        .wait_for(other.clone(), |f| f.contains("● 1"), Duration::from_secs(2))
        .await;
    assert!(
        f.contains("proj › main"),
        "this client draws the top bar, not the sidebar:\n{f}"
    );
    assert!(
        row(&f, 0).starts_with("| ● 1 "),
        "so the count is still at the left end:\n{f}"
    );
    assert!(
        h.model().sidebar_open,
        "the remembered state really is open, for both clients"
    );
}

#[tokio::test]
async fn focusing_an_unseen_agents_pane_takes_the_count_down() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    let first = h.focused_pane(h.client.clone());
    h.api("pane.split", json!({"dir": "right"})).await.unwrap();
    let second = h.focused_pane(h.client.clone());
    h.api("pane.focus", json!({"pane": first.to_string()}))
        .await
        .unwrap();
    // Working to idle while you are looking somewhere else: the dot is unseen, not waiting.
    h.report(
        second.clone(),
        AgentKind::Claude,
        r#"{"hook_event_name":"UserPromptSubmit","session_id":"c1"}"#,
    )
    .await;
    h.report(
        second.clone(),
        AgentKind::Claude,
        r#"{"hook_event_name":"Stop","session_id":"c1"}"#,
    )
    .await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("● 1"),
        Duration::from_secs(2),
    )
    .await;
    h.api("pane.focus", json!({"pane": second.to_string()}))
        .await
        .unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| !f.contains("● 1"),
            Duration::from_secs(2),
        )
        .await;
    assert!(!f.contains("● 1"), "{f}");
}

#[tokio::test]
async fn a_waiting_agent_keeps_the_count_up_even_after_you_look_at_it() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    let pane = h.focused_pane(h.client.clone());
    h.report(
        pane.clone(),
        AgentKind::Claude,
        r#"{"hook_event_name":"Notification","session_id":"c1","message":"needs permission"}"#,
    )
    .await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("● 1"),
        Duration::from_secs(2),
    )
    .await;
    h.api("pane.focus", json!({"pane": pane.to_string()}))
        .await
        .unwrap();
    let f = h.frame(h.client.clone()).await;
    assert!(
        f.contains("● 1"),
        "waiting is still waiting; only the unseen flag was cleared:\n{f}"
    );
    h.report(
        pane.clone(),
        AgentKind::Claude,
        r#"{"hook_event_name":"PostToolUse","session_id":"c1"}"#,
    )
    .await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| !f.contains("● 1"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        !f.contains("● 1"),
        "the agent answered and went back to work:\n{f}"
    );
}
