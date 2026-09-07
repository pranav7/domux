//! Key routing: the leader chord, global keys with passthrough, and everything else
//! reaching the pane unchanged (design principle 1).

use domux_core::config::Config;
use domux_server::testing::Harness;
use std::time::Duration;

#[tokio::test]
async fn printable_control_and_modified_keys_reach_the_pane_unchanged() {
    let mut h = Harness::start(Config::default(), 40, 10).await;
    let pane = h.focused_pane(h.client.clone());
    h.type_text(h.client.clone(), "ls -la").await;
    h.key(h.client.clone(), "Enter").await;
    h.key(h.client.clone(), "C-c").await;
    h.key(h.client.clone(), "M-b").await;
    h.key(h.client.clone(), "Up").await;
    h.paste(h.client.clone(), "two\nlines").await;
    h.frame(h.client.clone()).await;
    assert_eq!(
        h.pane_input(&pane),
        b"ls -la\r\x03\x1bb\x1b[Atwo\nlines".to_vec()
    );
}

#[tokio::test]
async fn leader_then_binding_runs_the_action_and_nothing_reaches_the_pane() {
    let mut h = Harness::start(Config::default(), 40, 10).await;
    let pane = h.focused_pane(h.client.clone());
    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), "\\").await;
    h.wait_for(
        h.client.clone(),
        |f| f.matches('┌').count() == 2,
        Duration::from_secs(2),
    )
    .await;
    assert!(h.pane_input(&pane).is_empty());
    assert!(
        h.model().client(&h.client).unwrap().chord.is_none(),
        "the chord is over"
    );
}

#[tokio::test]
async fn leader_twice_sends_the_leader_and_an_unbound_key_is_dropped() {
    let mut h = Harness::start(Config::default(), 40, 10).await;
    let pane = h.focused_pane(h.client.clone());
    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), "C-a").await;
    h.frame(h.client.clone()).await;
    assert_eq!(h.pane_input(&pane), b"\x01".to_vec());
    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), "x").await;
    h.frame(h.client.clone()).await;
    assert_eq!(
        h.pane_input(&pane),
        b"\x01".to_vec(),
        "x after the leader went nowhere"
    );
    assert!(h.model().client(&h.client).unwrap().chord.is_none());
}

#[tokio::test]
async fn global_focus_keys_move_focus_unless_the_foreground_is_a_passthrough_command() {
    let mut h = Harness::start(Config::default(), 60, 12).await;
    let left = h.focused_pane(h.client.clone());
    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), "\\").await;
    h.wait_for(
        h.client.clone(),
        |f| f.matches('┌').count() == 2,
        Duration::from_secs(2),
    )
    .await;
    let right = h.focused_pane(h.client.clone());
    h.key(h.client.clone(), "C-h").await;
    h.frame(h.client.clone()).await;
    assert_eq!(h.focused_pane(h.client.clone()), left);
    assert!(h.pane_input(&left).is_empty() && h.pane_input(&right).is_empty());
    h.set_foreground("nvim");
    tokio::time::sleep(Duration::from_millis(1100)).await; // one inspector tick
    h.frame(h.client.clone()).await;
    h.key(h.client.clone(), "C-l").await;
    h.frame(h.client.clone()).await;
    assert_eq!(
        h.focused_pane(h.client.clone()),
        left,
        "C-l passed through to nvim"
    );
    assert_eq!(h.pane_input(&left), b"\x0c".to_vec());
    h.key(h.client.clone(), "S-Right").await;
    h.frame(h.client.clone()).await;
    assert_eq!(
        h.pane_input(&left),
        b"\x0c".to_vec(),
        "S-Right is not a passthrough key, so domux took it"
    );
    h.key(h.client.clone(), "C-\\").await;
    h.frame(h.client.clone()).await;
    assert_eq!(
        h.pane_input(&left),
        b"\x0c\x1c".to_vec(),
        "C-\\ passes through too"
    );
}

#[tokio::test]
async fn a_configured_leader_replaces_the_default() {
    let mut cfg = Config::default();
    cfg.keys.leader = "C-b".into();
    let mut h = Harness::start(cfg, 40, 10).await;
    let pane = h.focused_pane(h.client.clone());
    h.key(h.client.clone(), "C-a").await;
    h.frame(h.client.clone()).await;
    assert_eq!(
        h.pane_input(&pane),
        b"\x01".to_vec(),
        "C-a is an ordinary key now"
    );
    h.key(h.client.clone(), "C-b").await;
    h.key(h.client.clone(), "c").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains(" 2 "),
        Duration::from_secs(2),
    )
    .await;
}

#[tokio::test]
async fn focus_keys_reach_the_pane_when_the_binding_is_removed() {
    let mut cfg = Config::default();
    cfg.keys.global.remove("C-h");
    let mut h = Harness::start(cfg, 40, 10).await;
    let pane = h.focused_pane(h.client.clone());
    h.key(h.client.clone(), "C-h").await;
    h.frame(h.client.clone()).await;
    assert_eq!(h.pane_input(&pane), b"\x08".to_vec());
}

#[tokio::test]
async fn the_prompt_takes_every_key_and_enter_names_the_tab() {
    let mut h = Harness::start(Config::default(), 40, 10).await;
    let pane = h.focused_pane(h.client.clone());
    let tab = h.current_tab(h.client.clone());
    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), ",").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("Name tab 1"),
        Duration::from_secs(2),
    )
    .await;
    h.type_text(h.client.clone(), "pr1").await;
    h.key(h.client.clone(), "Enter").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains(" 1 pr1 "),
        Duration::from_secs(2),
    )
    .await;
    assert_eq!(h.model().tab(&tab).unwrap().name.as_deref(), Some("pr1"));
    assert!(h.model().client(&h.client).unwrap().overlay.is_none());
    assert!(
        h.pane_input(&pane).is_empty(),
        "nothing typed at the prompt reached the pane"
    );
    // Focus is back on the pane, so the next key is the pane's again.
    h.key(h.client.clone(), "y").await;
    h.frame(h.client.clone()).await;
    assert_eq!(h.pane_input(&pane), b"y".to_vec());
}

#[tokio::test]
async fn the_help_overlay_swallows_keys_until_it_closes() {
    let mut h = Harness::start(Config::default(), 40, 10).await;
    let pane = h.focused_pane(h.client.clone());
    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), "?").await;
    h.frame(h.client.clone()).await;
    assert!(h.model().client(&h.client).unwrap().overlay.is_some());
    h.key(h.client.clone(), "x").await;
    h.frame(h.client.clone()).await;
    assert!(h.pane_input(&pane).is_empty(), "the overlay took the key");
    h.key(h.client.clone(), "Esc").await;
    h.frame(h.client.clone()).await;
    assert!(h.model().client(&h.client).unwrap().overlay.is_none());
    h.key(h.client.clone(), "y").await;
    h.frame(h.client.clone()).await;
    assert_eq!(h.pane_input(&pane), b"y".to_vec());
}

/// Principle 8: a key that cannot do what it says still answers. `C-a 9` selects the ninth
/// tab, and a workspace with one tab has none, so the failure is named where the clock is
/// and stands until the next key.
#[tokio::test]
async fn a_failed_action_names_the_failure_until_the_next_key() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), "9").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("tab 9 does not exist; this workspace has 1 tabs"),
        Duration::from_secs(2),
    )
    .await;
    h.key(h.client.clone(), "y").await;
    let frame = h.frame(h.client.clone()).await;
    assert!(!frame.contains("tab 9 does not exist"), "{frame}");
    assert!(frame.contains("14:32"), "the clock is back:\n{frame}");
}
