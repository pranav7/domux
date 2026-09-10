//! The top bar's left end. MUX-23 took the folded agent count off it: the number said nothing
//! the Agents box does not say, and the dot beside it was a second attention marker with no
//! row behind it. The bar now opens with the location label whatever the records say.

use domux_core::config::Config;
use domux_core::model::agent::AgentKind;
use domux_server::testing::{row, Harness};
use serde_json::json;

/// A waiting agent is the strongest case the count ever had, and the bar still opens with the
/// place. Both a bare screen and a waiting record, so a bar that had lost its label would not
/// pass by having nothing at the left end at all.
#[tokio::test]
async fn the_top_bar_opens_with_the_place_and_never_a_count() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    let f = h.frame(h.client.clone()).await;
    assert!(row(&f, 0).starts_with("| proj › main"), "{f}");
    let pane = h.focused_pane(h.client.clone());
    h.report(
        pane,
        AgentKind::Claude,
        r#"{"hook_event_name":"Notification","session_id":"c1","message":"needs permission"}"#,
    )
    .await;
    // The record really is waiting, so the assertion below is about the bar and not about a
    // report that never landed.
    assert!(h
        .model()
        .agents
        .iter()
        .any(|a| a.state == domux_core::model::agent::AgentState::Waiting));
    let f = h.frame(h.client.clone()).await;
    assert!(
        row(&f, 0).starts_with("| proj › main"),
        "a waiting agent adds nothing to the left end:\n{f}"
    );
    assert!(!f.contains("● 1"), "{f}");
}

/// A second waiting agent in another tab, which is what the count grew for. The tab row still
/// starts where the label ends, so a click lands on the tab the reader sees.
#[tokio::test]
async fn two_waiting_agents_still_leave_the_tab_row_where_it_was() {
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
    let f = h.frame(h.client.clone()).await;
    assert!(row(&f, 0).starts_with("| proj › main"), "{f}");
    assert!(!f.contains("● 2"), "{f}");
}
