//! Passthrough keys: which programs get `C-h`, `C-j`, `C-k`, `C-l` and `C-\` instead of
//! domux. A passthrough command gets them by name, a program that claimed them gets them while
//! it is in front of its pane, and a program hands focus back with its own pane (decision
//! 0054).

use domux_core::api::ErrorCode;
use domux_core::config::Config;
use domux_core::ids::PaneId;
use domux_core::model::{Focus, RegionKind};
use domux_server::testing::Harness;
use serde_json::json;
use std::time::Duration;

/// Longer than the inspector's once-a-second tick, so a pane's foreground command has been
/// read.
const ONE_TICK: Duration = Duration::from_millis(1100);

/// Splits the client's one pane and answers the left pane and the right one. The right one
/// has the keys. It reads the model rather than counting box corners, so it works at any
/// width, with the sidebar showing or not.
async fn two_panes(h: &mut Harness) -> (PaneId, PaneId) {
    let left = h.focused_pane(h.client.clone());
    h.api("pane.split", json!({"dir": "right"})).await.unwrap();
    h.frame(h.client.clone()).await;
    assert_eq!(h.model().all_pane_ids().len(), 2, "the split made a pane");
    let right = h.focused_pane(h.client.clone());
    assert_ne!(left, right, "the split took the keys");
    (left, right)
}

fn focus(h: &Harness) -> Focus {
    h.model()
        .client(&h.client)
        .expect("the first client is attached")
        .focus
        .clone()
}

/// Shows the sidebar and puts the keys in its Projects box.
async fn keys_in_the_sidebar(h: &mut Harness) {
    h.api("sidebar.show", json!({})).await.unwrap();
    h.api("focus.region", json!({"region": "sidebar_projects"}))
        .await
        .unwrap();
    h.frame(h.client.clone()).await;
    assert_eq!(focus(h), Focus::Region(RegionKind::SidebarProjects));
}

/// Puts `name` in front of `pane`, in a process group that holds this test process. Every
/// `h.api` call comes from the test process, so a claim the test makes then holds in that
/// pane.
async fn claimant_in_front(h: &mut Harness, pane: &PaneId, name: &str) {
    let group = h.set_foreground_for(pane, Some(name)).await;
    h.inspector.set_group(std::process::id(), group);
}

async fn claim(h: &mut Harness, pane: &PaneId) {
    h.api("pane.claim_passthrough", json!({"pane": pane.to_string()}))
        .await
        .unwrap();
}

async fn release(h: &mut Harness, pane: &PaneId) {
    h.api(
        "pane.release_passthrough",
        json!({"pane": pane.to_string()}),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn claimed_keys_reach_the_pane_while_the_claimant_is_in_front() {
    let mut h = Harness::start(Config::default(), 60, 12).await;
    let (_, right) = two_panes(&mut h).await;
    claimant_in_front(&mut h, &right, "nvim").await;
    claim(&mut h, &right).await;
    h.key(h.client.clone(), "C-h").await;
    h.frame(h.client.clone()).await;
    assert_eq!(h.focused_pane(h.client.clone()), right, "C-h went to nvim");
    assert_eq!(h.pane_input(&right), b"\x08".to_vec());
    h.key(h.client.clone(), "C-\\").await;
    h.frame(h.client.clone()).await;
    assert_eq!(
        h.pane_input(&right),
        b"\x08\x1c".to_vec(),
        "C-\\ is a passthrough key too"
    );
    h.key(h.client.clone(), "S-Left").await;
    h.frame(h.client.clone()).await;
    assert_eq!(
        h.pane_input(&right),
        b"\x08\x1c".to_vec(),
        "S-Left is not a passthrough key, so domux took it"
    );
}

#[tokio::test]
async fn a_claim_from_outside_the_foreground_group_changes_nothing() {
    let mut h = Harness::start(Config::default(), 60, 12).await;
    let (left, right) = two_panes(&mut h).await;
    // nvim is in front, in a group the test process is not in, so the claim is another
    // program's. That is what a claim from outside the pane always is.
    h.set_foreground_for(&right, Some("nvim")).await;
    claim(&mut h, &right).await;
    h.key(h.client.clone(), "C-h").await;
    h.frame(h.client.clone()).await;
    assert_eq!(h.focused_pane(h.client.clone()), left);
    assert!(h.pane_input(&right).is_empty());
}

#[tokio::test]
async fn a_claim_stops_holding_after_release() {
    let mut h = Harness::start(Config::default(), 60, 12).await;
    let (left, right) = two_panes(&mut h).await;
    claimant_in_front(&mut h, &right, "nvim").await;
    claim(&mut h, &right).await;
    release(&mut h, &right).await;
    h.key(h.client.clone(), "C-h").await;
    h.frame(h.client.clone()).await;
    assert_eq!(h.focused_pane(h.client.clone()), left);
}

#[tokio::test]
async fn a_claim_stops_holding_when_the_claimant_leaves_the_foreground() {
    let mut h = Harness::start(Config::default(), 60, 12).await;
    let (left, right) = two_panes(&mut h).await;
    claimant_in_front(&mut h, &right, "nvim").await;
    claim(&mut h, &right).await;
    // C-z in nvim: the shell is in front again, in a group of its own.
    h.set_foreground_for(&right, Some("zsh")).await;
    h.key(h.client.clone(), "C-h").await;
    h.frame(h.client.clone()).await;
    assert_eq!(
        h.focused_pane(h.client.clone()),
        left,
        "the key after C-z is domux's, with no tick in between"
    );
}

#[tokio::test]
async fn c_l_leaves_the_sidebar_while_the_pane_beside_it_holds_a_claim() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let pane = h.focused_pane(h.client.clone());
    claimant_in_front(&mut h, &pane, "nvim").await;
    claim(&mut h, &pane).await;
    keys_in_the_sidebar(&mut h).await;
    h.key(h.client.clone(), "C-l").await;
    h.frame(h.client.clone()).await;
    assert_eq!(focus(&h), Focus::Pane(pane.clone()));
    assert!(h.pane_input(&pane).is_empty(), "nvim got nothing");
}

#[tokio::test]
async fn focus_keys_move_focus_from_nvim_under_the_default_config() {
    let mut h = Harness::start(Config::default(), 60, 12).await;
    let (left, right) = two_panes(&mut h).await;
    h.set_foreground("nvim");
    tokio::time::sleep(ONE_TICK).await;
    h.frame(h.client.clone()).await;
    assert_eq!(
        h.model()
            .pane(&right)
            .and_then(|p| p.command.clone())
            .as_deref(),
        Some("nvim"),
        "the pane's command has been read, so the name is what this tests"
    );
    h.key(h.client.clone(), "C-h").await;
    h.frame(h.client.clone()).await;
    assert_eq!(h.focused_pane(h.client.clone()), left, "domux moved focus");
    assert!(h.pane_input(&right).is_empty(), "nvim got nothing");
}

#[tokio::test]
async fn a_command_the_config_lists_gets_the_keys_without_a_claim() {
    let mut cfg = Config::default();
    cfg.keys.passthrough.commands = vec!["nvim".into()];
    let mut h = Harness::start(cfg, 60, 12).await;
    let (_, right) = two_panes(&mut h).await;
    h.set_foreground("nvim");
    tokio::time::sleep(ONE_TICK).await;
    h.frame(h.client.clone()).await;
    h.key(h.client.clone(), "C-h").await;
    h.frame(h.client.clone()).await;
    assert_eq!(h.focused_pane(h.client.clone()), right, "C-h went to nvim");
    assert_eq!(h.pane_input(&right), b"\x08".to_vec());
}

#[tokio::test]
async fn c_l_leaves_the_sidebar_while_fzf_is_in_front_of_the_pane() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let pane = h.focused_pane(h.client.clone());
    h.set_foreground("fzf");
    tokio::time::sleep(ONE_TICK).await;
    keys_in_the_sidebar(&mut h).await;
    h.key(h.client.clone(), "C-l").await;
    h.frame(h.client.clone()).await;
    assert_eq!(
        focus(&h),
        Focus::Pane(pane.clone()),
        "C-l is the box's key, so it comes back to the pane"
    );
    assert!(h.pane_input(&pane).is_empty(), "fzf got nothing");
}

/// A claim belongs to the process at the other end of the socket, and a key has none.
#[tokio::test]
async fn a_claim_from_a_key_is_refused() {
    let mut cfg = Config::default();
    cfg.keys
        .bindings
        .insert("y".into(), "pane.claim_passthrough p_0000".into());
    let mut h = Harness::start(cfg, 120, 24).await;
    h.key(h.client.clone(), "C-s").await;
    h.key(h.client.clone(), "y").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("only a program in the pane"),
        Duration::from_secs(2),
    )
    .await;
}

#[tokio::test]
async fn a_claim_and_a_release_answer_ok_and_a_missing_pane_is_not_found() {
    let mut h = Harness::start(Config::default(), 60, 12).await;
    let pane = h.focused_pane(h.client.clone()).to_string();
    for method in [
        "pane.claim_passthrough",
        "pane.release_passthrough",
        "pane.release_passthrough",
    ] {
        let answer = h.api(method, json!({"pane": pane})).await.unwrap();
        assert_eq!(answer, json!({"ok": true}), "{method}");
    }
    let err = h
        .api("pane.claim_passthrough", json!({"pane": "p_ffff"}))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound, "{err:?}");
}
