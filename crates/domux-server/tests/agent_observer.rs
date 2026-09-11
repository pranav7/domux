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

/// One `SessionStart` on the record the observer made, so it holds a session id.
///
/// `report_agent` adopts the record already on the pane rather than making a second one, so the
/// id and the process this binds to are the
/// observer's own.
async fn hooked(h: &mut Harness, pane: &domux_core::ids::PaneId, kind: AgentKind, session: &str) {
    h.report(
        pane.clone(),
        kind,
        &format!("{{\"hook_event_name\":\"SessionStart\",\"session_id\":\"{session}\"}}"),
    )
    .await;
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
    hooked(&mut h, &pane, AgentKind::Claude, "s1").await;
    h.kill_process(pid).await;
    h.set_foreground_for(&pane, Some("zsh")).await;
    tokio::time::sleep(A_TICK).await;
    assert!(
        agents(&mut h).await.is_empty(),
        "the process went, so the record went with it"
    );
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
async fn a_pane_that_exits_ends_its_agents_session_without_waiting_for_a_tick() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let pane = h.focused_pane(h.client.clone());
    h.report(
        pane.clone(),
        AgentKind::Claude,
        r#"{"hook_event_name":"SessionStart","session_id":"s1"}"#,
    )
    .await;
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
    assert!(
        agents(&mut h).await.is_empty(),
        "the pane exit is immediate, not once a second"
    );
    assert!(!any_working(&h.model()), "nothing is working any more");
}

/// The same exit with `remain_on_exit`, where the pane stays open. Nothing closes the pane, so
/// only the pane exit itself can end the record.
#[tokio::test]
async fn a_pane_whose_child_exits_ends_its_agents_session_while_the_pane_stays() {
    let mut config = Config::default();
    config.terminal.remain_on_exit = true;
    let mut h = Harness::start(config, 80, 24).await;
    let pane = h.focused_pane(h.client.clone());
    h.report(
        pane.clone(),
        AgentKind::Claude,
        r#"{"hook_event_name":"SessionStart","session_id":"s1"}"#,
    )
    .await;
    h.report(
        pane.clone(),
        AgentKind::Claude,
        r#"{"hook_event_name":"UserPromptSubmit","session_id":"s1"}"#,
    )
    .await;
    assert_eq!(one_agent(&mut h).await.state, AgentState::Working);
    h.exit_pane(pane.clone(), Some(0)).await;
    assert!(agents(&mut h).await.is_empty(), "the session is over");
    assert!(
        h.model().pane(&pane).is_some(),
        "remain_on_exit keeps the pane, so the record ended on the child and not on a close"
    );
}

/// The case a user hits: claude runs codex through a tool, so another agent's name is in front
/// of the pane while claude is alive and working. Reading that name as claude's exit would set a
/// red dot on a running agent, and a later hook on an exited record changes nothing, so the
/// record would stay dead until the agent was restarted.
#[tokio::test]
async fn an_agent_that_runs_another_agent_as_a_tool_keeps_its_record() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let pane = h.focused_pane(h.client.clone());
    let claude = h.set_foreground_for(&pane, Some("claude")).await;
    tokio::time::sleep(A_TICK).await;
    h.report(
        pane.clone(),
        AgentKind::Claude,
        r#"{"hook_event_name":"PreToolUse","session_id":"s1"}"#,
    )
    .await;
    assert_eq!(one_agent(&mut h).await.state, AgentState::Working);
    // The tool is codex, which is a kind domux knows. Nothing killed claude.
    h.set_foreground_for(&pane, Some("codex")).await;
    tokio::time::sleep(A_TICK * 2).await;
    let a = one_agent(&mut h).await;
    assert_eq!(
        a.state,
        AgentState::Working,
        "claude is alive, so the agent in front of the pane is a tool it is running"
    );
    assert_eq!(a.kind, AgentKind::Claude);
    assert_eq!(a.pane.as_ref(), Some(&pane), "it never left its pane");
    assert_eq!(a.pid, Some(claude), "and it still names its own process");
    assert!(!a.unseen, "nothing happened, so nothing needs looking at");
}

/// The same case with the tool being the agent's own kind, which is what `claude -p` in a Bash
/// tool looks like: the record must keep the process it holds rather than follow the child, or
/// the child's exit would read as the agent's.
#[tokio::test]
async fn an_agent_that_runs_its_own_kind_as_a_tool_keeps_its_own_process() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let pane = h.focused_pane(h.client.clone());
    let parent = h.set_foreground_for(&pane, Some("claude")).await;
    tokio::time::sleep(A_TICK).await;
    assert_eq!(one_agent(&mut h).await.pid, Some(parent));
    // A second claude, in front of the pane while the first is alive.
    let child = h.set_foreground_for(&pane, Some("claude")).await;
    assert_ne!(child, parent);
    tokio::time::sleep(A_TICK).await;
    let a = one_agent(&mut h).await;
    assert_eq!(
        a.pid,
        Some(parent),
        "the record keeps its own process, not the one in front"
    );
    // Now the tool ends and its shell is in front. The record's own process is untouched, so
    // the child's death is not the agent's.
    h.kill_process(child).await;
    h.set_foreground_for(&pane, Some("bash")).await;
    tokio::time::sleep(A_TICK).await;
    let a = one_agent(&mut h).await;
    assert_eq!(a.state, AgentState::Unknown, "still running, still unknown");
    assert_eq!(a.pane.as_ref(), Some(&pane));
}

/// An agent that really did go, replaced by a fresh one of the same kind between two ticks. The
/// old record exits rather than being rebound, so the session that ended keeps its own record.
#[tokio::test]
async fn a_restarted_agent_of_the_same_kind_gets_a_record_of_its_own() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let pane = h.focused_pane(h.client.clone());
    let first_pid = h.set_foreground_for(&pane, Some("claude")).await;
    tokio::time::sleep(A_TICK).await;
    let first = one_agent(&mut h).await;
    hooked(&mut h, &pane, AgentKind::Claude, "s1").await;
    h.kill_process(first_pid).await;
    let second_pid = h.set_foreground_for(&pane, Some("claude")).await;
    tokio::time::sleep(A_TICK).await;
    let agents = agents(&mut h).await;
    assert_eq!(
        agents.len(),
        1,
        "the session that ended took its record with it"
    );
    let new = agents.iter().find(|a| a.id != first.id).unwrap();
    assert_eq!(new.kind, AgentKind::Claude);
    assert_eq!(new.state, AgentState::Unknown);
    assert_eq!(new.pid, Some(second_pid));
    assert_eq!(new.pane.as_ref(), Some(&pane));
}

/// The two answers disagreeing: the inspector names a process in front of the pane and also says
/// that process is gone. The observer must not read that as an agent leaving and a new one
/// arriving, or it would exit and create a record once a second for as long as it lasted.
#[tokio::test]
async fn a_process_that_is_named_in_front_and_reported_gone_does_not_multiply_records() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let pane = h.focused_pane(h.client.clone());
    let pid = h.set_foreground_for(&pane, Some("claude")).await;
    tokio::time::sleep(A_TICK).await;
    let first = one_agent(&mut h).await;
    hooked(&mut h, &pane, AgentKind::Claude, "s1").await;
    // Still in the foreground, and gone.
    h.kill_process(pid).await;
    tokio::time::sleep(A_TICK * 3).await;
    let a = one_agent(&mut h).await;
    assert_eq!(a.id, first.id, "the same one record, three ticks later");
    // And the record is not stuck: the moment the pane stops naming it, it goes.
    h.set_foreground_for(&pane, Some("zsh")).await;
    tokio::time::sleep(A_TICK).await;
    assert!(agents(&mut h).await.is_empty(), "the session is over");
}

/// A pane closed on purpose, which is a different path from a child that exited: `pane.close`
/// and `tab.close` change the model themselves and never call the core's `close_pane`, so the
/// records end on the kill list every one of those paths puts its panes on.
#[tokio::test]
async fn a_pane_that_is_closed_ends_its_agents_session() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let first = h.focused_pane(h.client.clone());
    h.api("pane.split", serde_json::json!({"dir": "right"}))
        .await
        .expect("pane.split");
    h.report(
        first.clone(),
        AgentKind::Claude,
        r#"{"hook_event_name":"SessionStart","session_id":"s1"}"#,
    )
    .await;
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
    assert!(agents(&mut h).await.is_empty(), "its pane was closed");
}

/// The whole tab going, which takes panes the model drops without naming them one at a time.
#[tokio::test]
async fn a_tab_that_closes_ends_the_sessions_in_its_panes() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let pane = h.focused_pane(h.client.clone());
    // A second tab, so closing the first is not closing the workspace's only one.
    h.api("tab.create", serde_json::json!({}))
        .await
        .expect("tab.create");
    h.report(
        pane.clone(),
        AgentKind::Claude,
        r#"{"hook_event_name":"SessionStart","session_id":"s1"}"#,
    )
    .await;
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
    assert!(agents(&mut h).await.is_empty(), "its tab was closed");
}

#[tokio::test]
async fn a_second_kind_taking_the_pane_ends_the_first_record_and_creates_a_second() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let pane = h.focused_pane(h.client.clone());
    h.set_foreground_for(&pane, Some("claude")).await;
    tokio::time::sleep(A_TICK).await;
    hooked(&mut h, &pane, AgentKind::Claude, "s1").await;
    let first = one_agent(&mut h).await;
    h.kill_process(first.pid.expect("the observer recorded the pid"))
        .await;
    let codex_pid = h.set_foreground_for(&pane, Some("codex")).await;
    tokio::time::sleep(A_TICK).await;
    let agents = agents(&mut h).await;
    assert_eq!(agents.len(), 1, "the claude session is over");
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

/// Every session that ends takes its record with it, whatever it reported and whatever kind it
/// was (decision record 0028). Two records, one hooked and one the observer found alone, and
/// neither is left behind.
#[tokio::test]
async fn a_hooked_record_and_a_bare_one_both_go_when_their_processes_do() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let first = h.focused_pane(h.client.clone());
    h.api("pane.split", serde_json::json!({"dir": "right"}))
        .await
        .expect("pane.split");
    let second = h.focused_pane(h.client.clone());

    let bare = h.set_foreground_for(&first, Some("claude")).await;
    let reported = h.set_foreground_for(&second, Some("claude")).await;
    tokio::time::sleep(A_TICK).await;
    assert_eq!(agents(&mut h).await.len(), 2);
    hooked(&mut h, &second, AgentKind::Claude, "s1").await;

    h.kill_process(bare).await;
    h.kill_process(reported).await;
    h.set_foreground_for(&first, Some("zsh")).await;
    h.set_foreground_for(&second, Some("zsh")).await;
    tokio::time::sleep(A_TICK * 2).await;

    assert!(
        agents(&mut h).await.is_empty(),
        "a session id is not a reason to keep a record whose session is over"
    );
}

/// A kind with no hooks installed goes the same way: the observer made the record and the
/// observer's own evidence takes it away.
#[tokio::test]
async fn a_codex_record_goes_when_its_process_does() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let pane = h.focused_pane(h.client.clone());
    let pid = h.set_foreground_for(&pane, Some("codex")).await;
    tokio::time::sleep(A_TICK).await;
    hooked(&mut h, &pane, AgentKind::Codex, "c1").await;
    h.kill_process(pid).await;
    h.set_foreground_for(&pane, Some("zsh")).await;
    tokio::time::sleep(A_TICK * 2).await;
    assert!(agents(&mut h).await.is_empty());
}
