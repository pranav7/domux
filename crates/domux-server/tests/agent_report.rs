//! `agent.report`: one hook payload in, one record change out, and the SessionStart context
//! block on the way back.
//!
//! The tests that read the records back through `agent.list` are ignored until Task 12 builds
//! it; the reason string on each says so.

use domux_core::config::Config;
use domux_core::model::agent::{AgentKind, AgentState};
use domux_server::testing::{Harness, HarnessOptions};
use serde_json::json;

fn payload(event: &str, extra: serde_json::Value) -> String {
    let mut v = json!({
        "session_id": "3f6a1c22-8d4e-4b90-9c7f-2a1b5e6d0f31",
        "cwd": "/Users/pranav/projects/domux",
        "hook_event_name": event
    });
    for (k, val) in extra.as_object().unwrap() {
        v[k] = val.clone();
    }
    v.to_string()
}

#[tokio::test]
#[ignore = "agent.list arrives in Task 12"]
async fn a_session_start_creates_an_idle_record_and_answers_with_the_context_block() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let pane = h.focused_pane(h.client.clone());
    let out = h
        .report(
            pane.clone(),
            AgentKind::Claude,
            &payload("SessionStart", json!({"source": "startup"})),
        )
        .await;
    assert_eq!(out.state, Some(AgentState::Idle));
    let agent = out.agent.expect("a record was created");
    let block = out
        .context
        .expect("SessionStart answers with the context block");
    assert!(
        block.starts_with(&format!("[domux] You are agent {agent} (claude) in ")),
        "{block}"
    );
    assert!(block.contains("domux2 peek"), "{block}");
    assert!(block.contains("domux2 whoami"), "{block}");
    assert!(block.contains("messaging arrives in M4"), "{block}");
    let agents = h.agents().await;
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].state, AgentState::Idle);
    assert_eq!(agents[0].pane.as_ref(), Some(&pane));
    assert_eq!(
        agents[0].session_id.as_deref(),
        Some("3f6a1c22-8d4e-4b90-9c7f-2a1b5e6d0f31")
    );
}

#[tokio::test]
async fn only_a_session_start_answers_with_a_context_block() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let pane = h.focused_pane(h.client.clone());
    h.report(
        pane.clone(),
        AgentKind::Claude,
        &payload("SessionStart", json!({})),
    )
    .await;
    for event in ["UserPromptSubmit", "PreToolUse", "Stop"] {
        let out = h
            .report(pane.clone(), AgentKind::Claude, &payload(event, json!({})))
            .await;
        assert_eq!(out.context, None, "{event}");
    }
}

#[tokio::test]
#[ignore = "agent.list arrives in Task 12"]
async fn the_hook_sequence_of_one_turn_walks_the_state_machine() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let pane = h.focused_pane(h.client.clone());
    let states = [
        "SessionStart",
        "UserPromptSubmit",
        "PreToolUse",
        "Notification",
        "PostToolUse",
        "Stop",
    ];
    let expected = [
        AgentState::Idle,
        AgentState::Working,
        AgentState::Working,
        AgentState::Waiting,
        AgentState::Working,
        AgentState::Idle,
    ];
    for (event, want) in states.iter().zip(expected) {
        let out = h
            .report(
                pane.clone(),
                AgentKind::Claude,
                &payload(
                    event,
                    json!({"message": "Claude needs your permission to use Bash"}),
                ),
            )
            .await;
        assert_eq!(out.state, Some(want), "{event}");
    }
    let a = &h.agents().await[0];
    assert_eq!(a.state, AgentState::Idle);
    assert!(a.unseen, "working to idle set unseen");
    assert_eq!(a.reason, None, "the reason went with the waiting state");
}

#[tokio::test]
#[ignore = "agent.list arrives in Task 12"]
async fn a_stop_reads_the_recap_and_the_session_name_from_the_transcript() {
    let dir = tempfile::tempdir().unwrap();
    let transcript = dir.path().join("s.jsonl");
    std::fs::write(
        &transcript,
        "{\"type\":\"ai-title\",\"aiTitle\":\"Session check cleanup\"}\n",
    )
    .unwrap();
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let pane = h.focused_pane(h.client.clone());
    let with_path = |event: &str| {
        payload(
            event,
            json!({"transcript_path": transcript.to_str().unwrap()}),
        )
    };
    h.report(pane.clone(), AgentKind::Claude, &with_path("SessionStart"))
        .await;
    h.report(
        pane.clone(),
        AgentKind::Claude,
        &with_path("UserPromptSubmit"),
    )
    .await;
    h.report(pane.clone(), AgentKind::Claude, &with_path("Stop"))
        .await;
    let a = &h.agents().await[0];
    assert_eq!(a.recap.as_deref(), Some("Session check cleanup"));
    assert_eq!(a.name, None, "no rename yet, so the kind stands in");
    std::fs::write(&transcript, "{\"type\":\"ai-title\",\"aiTitle\":\"Session check cleanup\"}\n{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"<command-name>/rename</command-name>\\n<command-args>auth-cleanup</command-args>\"}}\n{\"type\":\"system\",\"subtype\":\"away_summary\",\"timestamp\":\"2026-09-04T10:21:00.000Z\",\"content\":\"Replaced three session checks with one guard.\"}\n").unwrap();
    h.report(pane.clone(), AgentKind::Claude, &with_path("Stop"))
        .await;
    let a = &h.agents().await[0];
    assert_eq!(
        a.recap.as_deref(),
        Some("Replaced three session checks with one guard")
    );
    assert_eq!(a.name.as_deref(), Some("auth-cleanup"));
}

#[tokio::test]
#[ignore = "agent.list arrives in Task 12"]
async fn a_codex_payload_reaches_the_same_record_path_and_reads_no_transcript() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let pane = h.focused_pane(h.client.clone());
    let out = h
        .report(
            pane.clone(),
            AgentKind::Codex,
            r#"{"hook_event_name":"PermissionRequest","session_id":"t-1","message":"Codex wants to run `git push`"}"#,
        )
        .await;
    assert_eq!(out.state, Some(AgentState::Waiting));
    let a = &h.agents().await[0];
    assert_eq!(a.kind, AgentKind::Codex);
    assert_eq!(a.reason.as_deref(), Some("Codex wants to run `git push`"));
    assert_eq!(a.recap, None, "codex has no transcript recap in V2.0");
}

#[tokio::test]
#[ignore = "agent.list arrives in Task 12"]
async fn an_event_domux_does_not_track_changes_nothing_and_is_not_an_error() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let pane = h.focused_pane(h.client.clone());
    h.report(
        pane.clone(),
        AgentKind::Claude,
        &payload("SessionStart", json!({})),
    )
    .await;
    let out = h
        .report(
            pane.clone(),
            AgentKind::Claude,
            r#"{"hook_event_name":"SubagentStop","session_id":"x"}"#,
        )
        .await;
    assert_eq!(out.agent, None);
    assert_eq!(out.state, None);
    assert_eq!(h.agents().await[0].state, AgentState::Idle);
}

#[tokio::test]
async fn a_report_without_a_pane_says_which_variable_is_missing() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let err = h
        .api(
            "agent.report",
            serde_json::json!({"kind": "claude", "payload": {"hook_event_name": "Stop"}}),
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, domux_core::api::ErrorCode::InvalidParams);
    assert_eq!(err.message, "agent.report needs a pane; the hook ran without DOMUX_PANE set, so it is not inside a domux pane");
}

#[tokio::test]
async fn a_report_for_a_pane_that_closed_says_so_and_does_not_panic() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let err = h
        .api(
            "agent.report",
            serde_json::json!({"pane": "p_dead", "kind": "claude", "payload": {"hook_event_name": "Stop"}}),
        )
        .await
        .unwrap_err();
    assert_eq!(err.code, domux_core::api::ErrorCode::NotFound);
    assert!(err.message.contains("p_dead"), "{}", err.message);
}

#[tokio::test]
async fn the_context_block_names_this_agents_place_and_working_directory() {
    let opts = HarnessOptions::new(Config::default(), 80, 24);
    let mut h = Harness::start_with(opts).await;
    let pane = h.focused_pane(h.client.clone());
    let out = h
        .report(
            pane.clone(),
            AgentKind::Claude,
            &payload("SessionStart", serde_json::json!({})),
        )
        .await;
    let block = out.context.unwrap();
    let model = h.model();
    let ws = model
        .workspace(&model.client(&h.client).unwrap().workspace)
        .unwrap();
    assert!(
        block.contains(&ws.display_name()),
        "the place names the workspace:\n{block}"
    );
    assert!(
        block.contains(&pane.to_string()),
        "the place names the pane:\n{block}"
    );
    assert!(
        block.contains("/Users/pranav/projects/domux"),
        "the working directory from the payload:\n{block}"
    );
}
