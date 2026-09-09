//! The observer: what the once-a-second walk over the process inspector does to the records,
//! and what a pane exit does to them at once.
//!
//! Each test that asks what a tick did waits for real ticks, several seconds of them across
//! the file even though the tests run beside each other. The timing is the part of the observer
//! most likely to be wrong, and a mocked clock would test the mock.

use domux_core::config::Config;
use domux_core::model::agent::{Agent, AgentKind, AgentSource, AgentState};
use domux_core::model::Model;
use domux_server::agents::observer::any_working;
use domux_server::testing::Harness;
use std::time::Duration;

/// Longer than the core's one-second tick, so a sleep of this length has seen at least one.
const A_TICK: Duration = Duration::from_millis(1200);

/// Every record the core has published.
///
/// The read goes through one control API call first, and that call is what makes the answer
/// current: the core takes its messages in order, so a method that has answered proves every
/// message sent before it was handled. `agent.list` arrives in Task 12, and the records
/// themselves are what this file is about, so it reads the model the server publishes.
async fn agents(h: &mut Harness) -> Vec<Agent> {
    h.api("server.info", serde_json::json!({}))
        .await
        .expect("server.info");
    h.model().agents
}

async fn one_agent(h: &mut Harness) -> Agent {
    let agents = agents(h).await;
    assert_eq!(agents.len(), 1, "one record, got {agents:?}");
    agents.into_iter().next().unwrap()
}

#[tokio::test]
async fn a_known_agent_command_with_no_hooks_creates_an_unknown_record() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let pane = h.focused_pane(h.client.clone());
    let pid = h.set_foreground_for(&pane, Some("claude")).await;
    tokio::time::sleep(A_TICK).await;
    let a = one_agent(&mut h).await;
    assert_eq!(a.state, AgentState::Unknown);
    assert_eq!(a.kind, AgentKind::Claude);
    assert_eq!(a.pane.as_ref(), Some(&pane));
    assert_eq!(a.session_id, None, "no hook, so no session id");
    assert_eq!(a.source, AgentSource::Observer);
    assert_eq!(
        a.pid,
        Some(pid),
        "the record holds the process it was seen as"
    );
    assert!(
        !a.unseen,
        "an agent starting is not an attention transition"
    );
}

#[tokio::test]
async fn a_second_tick_does_not_create_a_second_record() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let pane = h.focused_pane(h.client.clone());
    h.set_foreground_for(&pane, Some("claude")).await;
    tokio::time::sleep(A_TICK * 3).await;
    assert_eq!(agents(&mut h).await.len(), 1);
}

#[tokio::test]
async fn a_shell_or_an_editor_in_the_foreground_creates_nothing() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let pane = h.focused_pane(h.client.clone());
    // `claude-code-review` is the trap: a name that starts with an agent's name is not that
    // agent, so the manifest matches the whole name and never a part of it.
    for name in ["zsh", "nvim", "cargo", "claude-code-review"] {
        h.set_foreground_for(&pane, Some(name)).await;
        tokio::time::sleep(A_TICK).await;
        assert!(agents(&mut h).await.is_empty(), "{name} is not an agent");
    }
}

#[tokio::test]
async fn a_record_whose_process_is_gone_is_marked_exited() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let pane = h.focused_pane(h.client.clone());
    let claude = h.set_foreground_for(&pane, Some("claude")).await;
    tokio::time::sleep(A_TICK).await;
    let pid = one_agent(&mut h)
        .await
        .pid
        .expect("the observer recorded the pid");
    assert_eq!(pid, claude);
    h.kill_process(pid).await;
    h.set_foreground_for(&pane, Some("zsh")).await;
    tokio::time::sleep(A_TICK).await;
    let a = one_agent(&mut h).await;
    assert_eq!(a.state, AgentState::Exited);
    assert_eq!(a.pane, None);
    assert!(a.unseen, "an exit is an attention transition");
}

#[tokio::test]
async fn an_agent_running_a_tool_keeps_its_state() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let pane = h.focused_pane(h.client.clone());
    h.set_foreground_for(&pane, Some("claude")).await;
    tokio::time::sleep(A_TICK).await;
    h.report(
        pane.clone(),
        AgentKind::Claude,
        r#"{"hook_event_name":"PreToolUse","session_id":"s1"}"#,
    )
    .await;
    assert_eq!(one_agent(&mut h).await.state, AgentState::Working);
    // claude spawned a shell for the tool, so the foreground is that shell while it runs.
    h.set_foreground_for(&pane, Some("bash")).await;
    tokio::time::sleep(A_TICK * 2).await;
    assert_eq!(
        one_agent(&mut h).await.state,
        AgentState::Working,
        "the tool is in the foreground, and that is not the agent's exit"
    );
    h.set_foreground_for(&pane, Some("claude")).await;
    tokio::time::sleep(A_TICK).await;
    let a = one_agent(&mut h).await;
    assert_eq!(a.state, AgentState::Working);
    assert_eq!(a.pane.as_ref(), Some(&pane), "it never left its pane");
}

#[tokio::test]
async fn a_pane_that_exits_marks_its_agent_exited_without_waiting_for_a_tick() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let pane = h.focused_pane(h.client.clone());
    h.report(
        pane.clone(),
        AgentKind::Claude,
        r#"{"hook_event_name":"UserPromptSubmit","session_id":"s1"}"#,
    )
    .await;
    assert_eq!(one_agent(&mut h).await.state, AgentState::Working);
    h.exit_pane(pane.clone(), Some(0)).await;
    // Well inside one tick, and the barrier is the pane going rather than the record, so the
    // record is still a question this test asks and not one it waits for.
    let gone = tokio::time::timeout(
        Duration::from_millis(200),
        until_pane_is_gone(&mut h, &pane),
    )
    .await
    .is_ok();
    assert!(gone, "the exited pane was never closed");
    let a = one_agent(&mut h).await;
    assert_eq!(
        a.state,
        AgentState::Exited,
        "the pane exit is immediate, not once a second"
    );
    assert_eq!(a.pane, None);
    assert!(!any_working(&h.model()), "nothing is working any more");
}

/// The same exit with `remain_on_exit`, where the pane stays open. Nothing closes the pane, so
/// only the pane exit itself can end the record.
#[tokio::test]
async fn a_pane_whose_child_exits_marks_its_agent_exited_while_the_pane_stays() {
    let mut config = Config::default();
    config.terminal.remain_on_exit = true;
    let mut h = Harness::start(config, 80, 24).await;
    let pane = h.focused_pane(h.client.clone());
    h.report(
        pane.clone(),
        AgentKind::Claude,
        r#"{"hook_event_name":"UserPromptSubmit","session_id":"s1"}"#,
    )
    .await;
    assert_eq!(one_agent(&mut h).await.state, AgentState::Working);
    h.exit_pane(pane.clone(), Some(0)).await;
    let a = one_agent(&mut h).await;
    assert_eq!(a.state, AgentState::Exited);
    assert_eq!(a.pane, None);
    assert!(
        h.model().pane(&pane).is_some(),
        "remain_on_exit keeps the pane, so the record exited on the child and not on a close"
    );
}

/// A pane closed on purpose, which is a different path from a child that exited: `pane.close`
/// and `tab.close` change the model themselves and never call the core's `close_pane`, so the
/// records end on the kill list every one of those paths puts its panes on.
#[tokio::test]
async fn a_pane_that_is_closed_marks_its_agent_exited() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let first = h.focused_pane(h.client.clone());
    h.api("pane.split", serde_json::json!({"dir": "right"}))
        .await
        .expect("pane.split");
    h.report(
        first.clone(),
        AgentKind::Claude,
        r#"{"hook_event_name":"UserPromptSubmit","session_id":"s1"}"#,
    )
    .await;
    assert_eq!(one_agent(&mut h).await.state, AgentState::Working);
    h.api("pane.close", serde_json::json!({"pane": first.as_str()}))
        .await
        .expect("pane.close");
    let a = one_agent(&mut h).await;
    assert_eq!(a.state, AgentState::Exited, "its pane was closed");
    assert_eq!(a.pane, None);
    assert_eq!(
        a.last_pane.as_ref(),
        Some(&first),
        "the record still says where it ran"
    );
}

/// The whole tab going, which takes panes the model drops without naming them one at a time.
#[tokio::test]
async fn a_tab_that_closes_marks_the_agents_in_its_panes_exited() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let pane = h.focused_pane(h.client.clone());
    // A second tab, so closing the first is not closing the workspace's only one.
    h.api("tab.create", serde_json::json!({}))
        .await
        .expect("tab.create");
    h.report(
        pane.clone(),
        AgentKind::Claude,
        r#"{"hook_event_name":"UserPromptSubmit","session_id":"s1"}"#,
    )
    .await;
    assert_eq!(one_agent(&mut h).await.state, AgentState::Working);
    let tab = h
        .model()
        .pane_location(&pane)
        .expect("the pane is in a tab")
        .tab;
    h.api("tab.close", serde_json::json!({"tab": tab.as_str()}))
        .await
        .expect("tab.close");
    let a = one_agent(&mut h).await;
    assert_eq!(a.state, AgentState::Exited);
    assert_eq!(a.pane, None);
}

#[tokio::test]
async fn a_second_kind_taking_the_pane_exits_the_first_record_and_creates_a_second() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let pane = h.focused_pane(h.client.clone());
    h.set_foreground_for(&pane, Some("claude")).await;
    tokio::time::sleep(A_TICK).await;
    let first = one_agent(&mut h).await;
    h.kill_process(first.pid.expect("the observer recorded the pid"))
        .await;
    let codex_pid = h.set_foreground_for(&pane, Some("codex")).await;
    tokio::time::sleep(A_TICK).await;
    let agents = agents(&mut h).await;
    assert_eq!(agents.len(), 2);
    let claude = agents.iter().find(|a| a.id == first.id).unwrap();
    assert_eq!(claude.state, AgentState::Exited);
    assert_eq!(claude.pane, None);
    let codex = agents.iter().find(|a| a.id != first.id).unwrap();
    assert_eq!(codex.kind, AgentKind::Codex);
    assert_eq!(codex.state, AgentState::Unknown);
    assert_eq!(codex.pane.as_ref(), Some(&pane));
    assert_eq!(codex.pid, Some(codex_pid));
}

#[tokio::test]
async fn the_observer_fills_in_the_pid_of_a_record_a_hook_created() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let pane = h.focused_pane(h.client.clone());
    h.report(
        pane.clone(),
        AgentKind::Claude,
        r#"{"hook_event_name":"SessionStart","session_id":"s1"}"#,
    )
    .await;
    let before = one_agent(&mut h).await;
    assert_eq!(before.pid, None);
    assert_eq!(before.source, AgentSource::Hook);
    let pid = h.set_foreground_for(&pane, Some("claude")).await;
    tokio::time::sleep(A_TICK).await;
    let after = one_agent(&mut h).await;
    assert_eq!(
        after.pid,
        Some(pid),
        "the observer bound the process to the record"
    );
    assert_eq!(after.id, before.id, "the same record, not a second one");
    assert_eq!(
        after.state,
        AgentState::Idle,
        "the observer never overwrites a hook's state"
    );
}

/// What the animation ticker asks: is there anything for a turning glyph to report on.
#[tokio::test]
async fn any_working_is_true_only_while_a_record_works_or_compacts() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let pane = h.focused_pane(h.client.clone());
    assert!(!any_working(&h.model()), "no records at all");
    for (event, working, why) in [
        ("SessionStart", false, "idle is not working"),
        ("UserPromptSubmit", true, "a turn started"),
        ("PreCompact", true, "compacting turns the glyph too"),
        ("Notification", false, "waiting is not working"),
    ] {
        h.report(
            pane.clone(),
            AgentKind::Claude,
            &format!(r#"{{"hook_event_name":"{event}","session_id":"s1"}}"#),
        )
        .await;
        assert_eq!(any_working(&h.model()), working, "after {event}: {why}");
    }
}

/// Waits until the model no longer holds `pane`.
async fn until_pane_is_gone(h: &mut Harness, pane: &domux_core::ids::PaneId) {
    loop {
        let model: Model = h.model();
        if model.pane(pane).is_none() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}
