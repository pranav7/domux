//! `agent.report`: one hook payload in, one record change out, and the SessionStart context
//! block on the way back.
//!
//! The records are read back through `Harness::model()`, the published snapshot, rather than
//! through `agent.list`, which Task 12 builds. Every assertion here runs today.

use domux_core::config::Config;
use domux_core::model::agent::{Agent, AgentKind, AgentState};
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

/// The record the reports landed on. Read from the published model, the way the committed
/// tests read a write straight after awaiting the call that made it.
fn only_agent(h: &Harness) -> Agent {
    let m = h.model();
    assert_eq!(m.agents.len(), 1, "one record");
    m.agents[0].clone()
}

#[tokio::test]
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
    let a = only_agent(&h);
    assert_eq!(a.state, AgentState::Idle);
    assert_eq!(a.pane.as_ref(), Some(&pane));
    assert_eq!(
        a.session_id.as_deref(),
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
    let a = only_agent(&h);
    assert_eq!(a.state, AgentState::Idle);
    assert!(a.unseen, "working to idle set unseen");
    assert_eq!(a.reason, None, "the reason went with the waiting state");
}

#[tokio::test]
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
    let a = only_agent(&h);
    assert_eq!(a.recap.as_deref(), Some("Session check cleanup"));
    assert_eq!(a.name, None, "no rename yet, so the kind stands in");
    std::fs::write(&transcript, "{\"type\":\"ai-title\",\"aiTitle\":\"Session check cleanup\"}\n{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"<command-name>/rename</command-name>\\n<command-args>auth-cleanup</command-args>\"}}\n{\"type\":\"system\",\"subtype\":\"away_summary\",\"timestamp\":\"2026-09-04T10:21:00.000Z\",\"content\":\"Replaced three session checks with one guard.\"}\n").unwrap();
    h.report(pane.clone(), AgentKind::Claude, &with_path("Stop"))
        .await;
    let a = only_agent(&h);
    assert_eq!(
        a.recap.as_deref(),
        Some("Replaced three session checks with one guard")
    );
    assert_eq!(a.name.as_deref(), Some("auth-cleanup"));
}

/// Plan assumption 6, which the architecture spec does not name: the transcript is re-read on
/// `SessionStart` too, because a resumed session already has a recap and a name and would
/// otherwise show neither until its first turn ended. Only `SessionStart` is sent here, so
/// neither of the two events the spec does name can be what read the file.
#[tokio::test]
async fn a_session_start_alone_reads_the_recap_and_the_name_a_resumed_session_already_has() {
    let dir = tempfile::tempdir().unwrap();
    let transcript = dir.path().join("s.jsonl");
    std::fs::write(&transcript, "{\"type\":\"ai-title\",\"aiTitle\":\"Session check cleanup\"}\n{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"<command-name>/rename</command-name>\\n<command-args>auth-cleanup</command-args>\"}}\n").unwrap();
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let pane = h.focused_pane(h.client.clone());
    h.report(
        pane.clone(),
        AgentKind::Claude,
        &payload(
            "SessionStart",
            json!({"transcript_path": transcript.to_str().unwrap()}),
        ),
    )
    .await;
    let a = only_agent(&h);
    assert_eq!(a.recap.as_deref(), Some("Session check cleanup"));
    assert_eq!(a.name.as_deref(), Some("auth-cleanup"));
}

#[tokio::test]
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
    let a = only_agent(&h);
    assert_eq!(a.kind, AgentKind::Codex);
    assert_eq!(a.reason.as_deref(), Some("Codex wants to run `git push`"));
    assert_eq!(a.recap, None, "codex has no transcript recap in V2.0");
}

/// The Codex manifest's `RecapSource::None` on its own. The event is one the recap is re-read
/// on, and the payload names a transcript that is really there and really holds a recap, so
/// the manifest is the only thing left that can keep the recap absent.
#[tokio::test]
async fn a_codex_stop_reads_no_recap_though_its_payload_names_a_transcript() {
    let dir = tempfile::tempdir().unwrap();
    let transcript = dir.path().join("s.jsonl");
    std::fs::write(
        &transcript,
        "{\"type\":\"ai-title\",\"aiTitle\":\"Session check cleanup\"}\n",
    )
    .unwrap();
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let pane = h.focused_pane(h.client.clone());
    let text = format!(
        r#"{{"hook_event_name":"Stop","session_id":"t-1","rollout_path":"{}"}}"#,
        transcript.to_str().unwrap()
    );
    h.report(pane.clone(), AgentKind::Codex, &text).await;
    let a = only_agent(&h);
    assert_eq!(
        a.transcript_path.as_deref(),
        Some(transcript.as_path()),
        "the payload's path did reach the record"
    );
    assert_eq!(
        a.recap, None,
        "the codex manifest declares no recap source, so nothing read that file"
    );
}

#[tokio::test]
async fn an_event_domux_does_not_track_changes_nothing_and_is_not_an_error() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let pane = h.focused_pane(h.client.clone());
    h.report(
        pane.clone(),
        AgentKind::Claude,
        &payload("SessionStart", json!({})),
    )
    .await;
    let before = only_agent(&h);
    let out = h
        .report(
            pane.clone(),
            AgentKind::Claude,
            r#"{"hook_event_name":"SubagentStop","session_id":"x"}"#,
        )
        .await;
    assert_eq!(out.agent, None);
    assert_eq!(out.state, None);
    let after = only_agent(&h);
    assert_eq!(after, before, "no field of the record moved");
    assert_eq!(after.state, AgentState::Idle);
}

/// Plan assumption 7: every hook but `SessionStart` leaves a record whose session is over
/// alone. The hook still finds a live pane, so it is not refused; it must simply change
/// nothing, the recap and the session name included.
#[tokio::test]
async fn a_hook_that_arrives_after_the_session_ended_leaves_the_exited_record_alone() {
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
    h.report(pane.clone(), AgentKind::Claude, &with_path("SessionEnd"))
        .await;
    let exited = only_agent(&h);
    assert_eq!(exited.state, AgentState::Exited);
    assert_eq!(exited.recap.as_deref(), Some("Session check cleanup"));
    // The session is over, and the transcript then grows a recap and a name it never had.
    std::fs::write(&transcript, "{\"type\":\"ai-title\",\"aiTitle\":\"Session check cleanup\"}\n{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"<command-name>/rename</command-name>\\n<command-args>auth-cleanup</command-args>\"}}\n{\"type\":\"system\",\"subtype\":\"away_summary\",\"timestamp\":\"2026-09-04T10:21:00.000Z\",\"content\":\"Replaced three session checks with one guard.\"}\n").unwrap();
    let out = h
        .report(pane.clone(), AgentKind::Claude, &with_path("Stop"))
        .await;
    assert_eq!(out.state, Some(AgentState::Exited));
    let after = only_agent(&h);
    assert_eq!(after, exited, "no field of the exited record moved");
}

/// Never fabricate cuts both ways: a session name is durable, and a read that did not find
/// one is not evidence that the agent cleared it. A transcript over `recap::FULL_SCAN_BYTES`
/// is read as a head and a tail, so an early `/rename` can fall outside the window.
#[tokio::test]
async fn a_session_name_survives_a_transcript_read_that_does_not_name_it() {
    let dir = tempfile::tempdir().unwrap();
    let transcript = dir.path().join("s.jsonl");
    std::fs::write(&transcript, "{\"type\":\"ai-title\",\"aiTitle\":\"Session check cleanup\"}\n{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"<command-name>/rename</command-name>\\n<command-args>auth-cleanup</command-args>\"}}\n").unwrap();
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
    assert_eq!(only_agent(&h).name.as_deref(), Some("auth-cleanup"));
    // The window the reader sees no longer holds the rename.
    std::fs::write(&transcript, "{\"type\":\"system\",\"subtype\":\"away_summary\",\"timestamp\":\"2026-09-04T10:21:00.000Z\",\"content\":\"Replaced three session checks with one guard.\"}\n").unwrap();
    h.report(pane.clone(), AgentKind::Claude, &with_path("Stop"))
        .await;
    let a = only_agent(&h);
    assert_eq!(
        a.name.as_deref(),
        Some("auth-cleanup"),
        "the name the agent set is durable"
    );
    assert_eq!(
        a.recap.as_deref(),
        Some("Replaced three session checks with one guard"),
        "the recap summarises the last turn, so it does follow the transcript"
    );
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
