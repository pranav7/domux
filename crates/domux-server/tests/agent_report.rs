//! `agent.report`: one hook payload in, one record change out, and the SessionStart context
//! block on the way back.
//!
//! The records are read back through `Harness::model()`, the published snapshot, rather than
//! through `agent.list`, which Task 12 builds. Every assertion here runs today.

use domux_core::config::Config;
use domux_core::model::agent::{Agent, AgentKind, AgentState};
use domux_server::testing::{Harness, HarnessOptions};
use serde_json::json;
use std::time::Duration;

/// Long enough for the core's once-a-second tick to have run, which is what reads a transcript.
const TICK: Duration = Duration::from_secs(5);

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
    // The three verbs M4 fills in are named, and the paragraph says they fail rather than
    // quoting a sentence: each one refuses in words of its own, naming its own method, so a
    // quotation in the block would be a fourth wording nothing keeps equal to the other three.
    for verb in ["domux2 send", "domux2 wait", "domux2 read"] {
        assert!(block.contains(verb), "{block}");
    }
    assert!(block.contains("fail with an error"), "{block}");
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

/// MUX-28, both halves, at the level the author met them.
///
/// A turn the agent has not recapped shows no recap, however much it has said: M3 filled the
/// slot with the last thing the agent said in words, and a row whose session had never written
/// a recap said a sentence out of the middle of a turn.
///
/// And the recap that does arrive arrives late. Claude Code writes the entry minutes after the
/// hook that ended the turn, so nothing reads it here: the tick does, and the second half of
/// this test sends no hook at all (decision record 0034).
#[tokio::test]
async fn a_recap_the_agent_writes_after_its_last_hook_still_reaches_the_record() {
    let dir = tempfile::tempdir().unwrap();
    let transcript = dir.path().join("s.jsonl");
    let said = "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"tidy the session checks\"}}\n{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"I will read the file first.\"}]}}\n";
    std::fs::write(&transcript, said).unwrap();
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
    let a = h
        .wait_for_agent(|a| a.state == AgentState::Idle, TICK)
        .await;
    assert_eq!(
        a.recap, None,
        "the agent said plenty and recapped none of it"
    );
    assert_eq!(a.name, None, "no rename yet, so the kind stands in");

    // The turn is over and its hooks have all been sent. The agent writes its recap now, and
    // checkpoints the name it was given, and nothing reports either.
    std::fs::write(&transcript, format!("{said}{}", "{\"type\":\"custom-title\",\"customTitle\":\"auth-cleanup\",\"sessionId\":\"c1\"}\n{\"type\":\"system\",\"subtype\":\"away_summary\",\"timestamp\":\"2026-09-04T10:21:00.000Z\",\"content\":\"Replaced three session checks with one guard.\"}\n")).unwrap();
    let a = h.wait_for_agent(|a| a.recap.is_some(), TICK).await;
    assert_eq!(
        a.recap.as_deref(),
        Some("Replaced three session checks with one guard")
    );
    assert_eq!(a.name.as_deref(), Some("auth-cleanup"));
}

/// A resumed session already has a recap and a name, and shows both without waiting for a turn
/// to end. M3 read them on `SessionStart` for this; the tick reads them whether a hook arrives
/// or not, and only `SessionStart` is sent here.
#[tokio::test]
async fn a_resumed_session_shows_the_recap_and_the_name_it_already_has() {
    let dir = tempfile::tempdir().unwrap();
    let transcript = dir.path().join("s.jsonl");
    std::fs::write(&transcript, "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"<command-name>/rename</command-name>\\n<command-args>auth-cleanup</command-args>\"}}\n{\"type\":\"system\",\"subtype\":\"away_summary\",\"content\":\"Replaced three session checks with one guard.\"}\n").unwrap();
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
    let a = h.wait_for_agent(|a| a.recap.is_some(), TICK).await;
    assert_eq!(
        a.recap.as_deref(),
        Some("Replaced three session checks with one guard")
    );
    assert_eq!(a.name.as_deref(), Some("auth-cleanup"));
}

#[tokio::test]
async fn a_codex_payload_reaches_the_same_record_path_and_reads_no_transcript() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let pane = h.focused_pane(h.client.clone());
    h.report(
        pane.clone(),
        AgentKind::Codex,
        r#"{"hook_event_name":"SessionStart","session_id":"t-1"}"#,
    )
    .await;
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
    h.report(
        pane.clone(),
        AgentKind::Codex,
        r#"{"hook_event_name":"SessionStart","session_id":"t-1"}"#,
    )
    .await;
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
        .report_raw(
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

/// A hook that arrives after the session ended finds no record to write to. The pane is still
/// there, so the report is not refused; it answers with no state, and nothing is created to
/// carry the recap and the session name the transcript has grown since.
#[tokio::test]
async fn a_hook_that_arrives_after_the_session_ended_writes_to_nothing() {
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
    assert!(
        h.model().agents.is_empty(),
        "the session is over, so its record is gone (decision record 0030)"
    );
    // The transcript then grows a recap and a name it never had.
    std::fs::write(&transcript, "{\"type\":\"ai-title\",\"aiTitle\":\"Session check cleanup\"}\n{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"<command-name>/rename</command-name>\\n<command-args>auth-cleanup</command-args>\"}}\n{\"type\":\"system\",\"subtype\":\"away_summary\",\"timestamp\":\"2026-09-04T10:21:00.000Z\",\"content\":\"Replaced three session checks with one guard.\"}\n").unwrap();
    let out = h
        .report_raw(pane.clone(), AgentKind::Claude, &with_path("Stop"))
        .await;
    assert_eq!(
        out.agent, None,
        "there was no record for the hook to land on"
    );
    assert_eq!(out.state, None);
    assert!(
        h.model().agents.is_empty(),
        "and no record was invented for a session domux is not tracking"
    );
}

/// Never fabricate cuts both ways: a session name is durable, and a read that did not find
/// one is not evidence that the agent cleared it. A transcript domux meets for the first time
/// is read from `recap::TAIL_BYTES` before its end, so an early `/rename` can fall outside the
/// window.
#[tokio::test]
async fn a_session_name_survives_a_transcript_read_that_does_not_name_it() {
    let dir = tempfile::tempdir().unwrap();
    let transcript = dir.path().join("s.jsonl");
    std::fs::write(&transcript, "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"<command-name>/rename</command-name>\\n<command-args>auth-cleanup</command-args>\"}}\n").unwrap();
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
    h.wait_for_agent(|a| a.name.is_some(), TICK).await;
    // A transcript the reader has to start again on, and this one names no rename.
    std::fs::write(&transcript, "{\"type\":\"system\",\"subtype\":\"away_summary\",\"timestamp\":\"2026-09-04T10:21:00.000Z\",\"content\":\"Replaced three session checks with one guard.\"}\n").unwrap();
    let a = h.wait_for_agent(|a| a.recap.is_some(), TICK).await;
    assert_eq!(
        a.name.as_deref(),
        Some("auth-cleanup"),
        "the name the agent set is durable"
    );
    assert_eq!(
        a.recap.as_deref(),
        Some("Replaced three session checks with one guard"),
        "and the recap is whatever the transcript now holds"
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
