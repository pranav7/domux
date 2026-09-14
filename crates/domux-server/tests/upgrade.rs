//! `server.upgrade`: the server hands its panes, their screens, the agent records and the
//! socket to a new build of itself, and nothing a pane was running ends (decision 0045).
//!
//! The exec is the one step a test cannot take, so `Harness::upgrade` takes the rest: the old
//! server writes its handover and asks to exec, and the harness starts the new server from that
//! handover in this process, with the same spawner.

use domux_core::api::{ErrorCode, ServerInfo};
use domux_core::config::Config;
use domux_core::model::agent::{AgentKind, AgentState};
use domux_core::proto::SERVER_UPGRADING;
use domux_server::testing::{Harness, HarnessOptions};
use serde_json::json;
use std::path::Path;
use std::time::Duration;

fn info(value: serde_json::Value) -> ServerInfo {
    serde_json::from_value(value).expect("ServerInfo")
}

#[tokio::test]
async fn an_upgrade_adopts_every_pane_rather_than_spawning_it_again() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    h.api("pane.split", json!({"dir": "right"})).await.unwrap();
    h.frame(h.client.clone()).await;
    let spawner = h.spawner.clone().unwrap();
    let spawned = spawner.requests().len();
    let panes: Vec<_> = h.model().all_pane_ids();
    assert_eq!(panes.len(), 2);

    h.upgrade().await;

    assert_eq!(
        spawner.requests().len(),
        spawned,
        "the new server spawned nothing"
    );
    let mut adopted: Vec<_> = spawner.adopted().into_iter().map(|(p, _)| p).collect();
    adopted.sort();
    let mut expected = panes.clone();
    expected.sort();
    assert_eq!(adopted, expected, "it adopted every pane the old one had");
    for (pane, pty) in spawner.adopted() {
        assert_eq!(
            Some(pty.fd),
            spawner.raw_fd(&pane),
            "each pane took back its own PTY"
        );
    }
    assert_eq!(h.model().all_pane_ids(), panes, "the layout is the same");
}

#[tokio::test]
async fn an_upgraded_pane_shows_the_screen_it_had() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let pane = h.focused_pane(h.client.clone());
    h.feed_pane(
        pane.clone(),
        b"written before the upgrade\r\n\x1b[31mred\x1b[0m",
    )
    .await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("written before the upgrade"),
        Duration::from_secs(2),
    )
    .await;

    h.upgrade().await;

    h.wait_for(
        h.client.clone(),
        |f| f.contains("written before the upgrade") && f.contains("red"),
        Duration::from_secs(2),
    )
    .await;
    // And the pane goes on taking output after it.
    h.feed_pane(pane, b" and after").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("red and after"),
        Duration::from_secs(2),
    )
    .await;
}

#[tokio::test]
async fn an_upgrade_keeps_every_agent_record_with_its_state_and_session_name() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let pane = h.focused_pane(h.client.clone());
    let session = json!({"session_id": "3f6a1c22-8d4e-4b90-9c7f-2a1b5e6d0f31", "cwd": "/tmp"});
    let with = |event: &str| {
        let mut v = session.clone();
        v["hook_event_name"] = json!(event);
        v.to_string()
    };
    h.report(pane.clone(), AgentKind::Claude, &with("SessionStart"))
        .await;
    h.report(pane.clone(), AgentKind::Claude, &with("UserPromptSubmit"))
        .await;
    let before = h.model().agents;
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].state, AgentState::Working);

    h.upgrade().await;

    let after = h.model().agents;
    assert_eq!(
        after.len(),
        1,
        "the record crossed, and no second one was made"
    );
    assert_eq!(after[0].id, before[0].id);
    assert_eq!(after[0].state, AgentState::Working);
    assert_eq!(after[0].session_id, before[0].session_id);
    assert_eq!(after[0].pane, before[0].pane);
    // A hook after the upgrade lands on the same record.
    h.report(pane, AgentKind::Claude, &with("Stop")).await;
    h.frame(h.client.clone()).await;
    assert_eq!(h.model().agents[0].state, AgentState::Idle);
}

#[tokio::test]
async fn an_attached_client_is_told_the_server_is_upgrading() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let client = h.client.clone();
    let binary = std::env::current_exe().unwrap();
    assert!(h.request_upgrade(&binary).await.is_none());
    assert_eq!(
        h.detached_reason(client).await.as_deref(),
        Some(SERVER_UPGRADING)
    );
}

#[tokio::test]
async fn server_info_says_when_the_server_was_upgraded_and_keeps_when_it_started() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let before = info(h.api("server.info", json!({})).await.unwrap());
    assert_eq!(
        before.upgraded_at, None,
        "a server that never upgraded says nothing"
    );
    h.clock.advance(Duration::from_secs(60));

    h.upgrade().await;

    let after = info(h.api("server.info", json!({})).await.unwrap());
    assert_eq!(after.started_at, before.started_at);
    assert!(after.upgraded_at.is_some());
    assert_ne!(
        after.upgraded_at.as_deref(),
        Some(before.started_at.as_str())
    );
}

#[tokio::test]
async fn an_upgrade_that_cannot_exec_takes_its_panes_back_and_goes_on() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let pane = h.focused_pane(h.client.clone());
    h.feed_pane(pane.clone(), b"still here").await;
    h.frame(h.client.clone()).await;
    h.exec.fail_with("No such file or directory (os error 2)");
    let binary = std::env::current_exe().unwrap();

    let refused = h
        .request_upgrade(&binary)
        .await
        .expect("a failed exec is answered");

    assert!(
        refused.message.contains("did not upgrade"),
        "{}",
        refused.message
    );
    assert!(
        refused.message.contains("No such file"),
        "{}",
        refused.message
    );
    assert!(
        refused.message.contains("still running"),
        "{}",
        refused.message
    );
    let spawner = h.spawner.clone().unwrap();
    assert_eq!(
        spawner
            .adopted()
            .into_iter()
            .map(|(p, _)| p)
            .collect::<Vec<_>>(),
        vec![pane.clone()],
        "the server took its pane back"
    );
    assert!(
        !domux_core::paths::handoff_dir_under(h.state_dir()).exists(),
        "and removed the handover it wrote"
    );
    // The socket accepts again and the pane still draws.
    let client = h.attach(80, 24).await;
    h.feed_pane(pane.clone(), b" and working").await;
    h.wait_for(
        client,
        |f| f.contains("still here and working"),
        Duration::from_secs(2),
    )
    .await;
    h.api("pane.send_text", json!({"pane": pane, "text": "typed"}))
        .await
        .unwrap();
    assert!(
        String::from_utf8_lossy(&h.pane_input(&pane)).contains("typed"),
        "input reaches the pane it took back"
    );
}

#[tokio::test]
async fn an_upgrade_to_a_file_that_cannot_run_is_refused_before_anything_changes() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let refused = h
        .request_upgrade(Path::new("/no/such/domux"))
        .await
        .expect("refused");
    assert_eq!(refused.code, ErrorCode::InvalidParams);
    assert!(
        refused.message.contains("/no/such/domux"),
        "{}",
        refused.message
    );
    assert!(h.exec.calls().is_empty());
    let refused = h
        .request_upgrade(Path::new("relative/domux"))
        .await
        .expect("refused");
    assert!(refused.message.contains("absolute"), "{}", refused.message);
    // Nothing paused: the server still answers and still takes a client.
    h.api("server.info", json!({})).await.unwrap();
    h.attach(80, 24).await;
}

#[tokio::test]
async fn an_upgrade_to_a_binary_that_reads_another_handover_format_is_refused() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let binary = std::env::current_exe().unwrap();
    let refused = h
        .api("server.upgrade", json!({"binary": binary, "handoff": 99}))
        .await
        .unwrap_err();
    assert_eq!(refused.code, ErrorCode::Conflict);
    assert!(refused.message.contains("format 99"), "{}", refused.message);
    assert!(
        refused.message.contains("server restart"),
        "{}",
        refused.message
    );
    assert!(h.exec.calls().is_empty());
}

#[tokio::test]
async fn a_server_can_be_upgraded_twice() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let pane = h.focused_pane(h.client.clone());
    h.feed_pane(pane.clone(), b"first").await;
    h.frame(h.client.clone()).await;
    h.upgrade().await;
    h.feed_pane(pane.clone(), b" second").await;
    h.frame(h.client.clone()).await;
    h.upgrade().await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("first second"),
        Duration::from_secs(2),
    )
    .await;
    assert_eq!(h.exec.calls().len(), 2);
}

/// The whole handover with real PTYs and a real shell: only the exec is left out. The shell
/// that printed before the upgrade is the process that answers after it.
#[tokio::test]
async fn a_real_shell_goes_on_running_across_an_upgrade() {
    let opts = HarnessOptions {
        real_ptys: true,
        ..HarnessOptions::new(Config::default(), 80, 24)
    };
    let mut h = Harness::start_with(opts).await;
    let pane = h.focused_pane(h.client.clone());
    h.spawn_in_pane(pane.clone(), &["echo", "before-$((20+1))"])
        .await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("before-21"),
        Duration::from_secs(5),
    )
    .await;
    let pid = h.model().pane(&pane).unwrap().pid;
    assert!(pid.is_some());

    h.upgrade().await;

    assert_eq!(h.model().pane(&pane).unwrap().pid, pid, "the same process");
    h.spawn_in_pane(pane.clone(), &["echo", "after-$((40+2))"])
        .await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("before-21") && f.contains("after-42"),
        Duration::from_secs(5),
    )
    .await;
}
