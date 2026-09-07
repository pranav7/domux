//! Closing a tab: the key asks first, the API does not.

use domux_core::config::Config;
use domux_server::testing::Harness;
use serde_json::json;
use std::time::Duration;

/// A second tab, which `tab.create` makes the current one.
async fn two_tabs() -> Harness {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    h.api("tab.create", json!({})).await.unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains(" 2 "),
        Duration::from_secs(2),
    )
    .await;
    h
}

/// The question names the tab by the number the tab row shows, so the reader can match the
/// two, and offers both answers (principle 9). Nothing has closed while it stands.
#[tokio::test]
async fn the_key_asks_before_it_closes_and_y_closes() {
    let mut h = two_tabs().await;
    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), "x").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("close tab 2?"),
            Duration::from_secs(2),
        )
        .await;
    assert!(f.contains("y close"), "the way to say yes:\n{f}");
    assert!(f.contains("esc keep"), "the way to say no:\n{f}");
    assert!(f.contains(" 2 "), "the tab is still open:\n{f}");

    h.key(h.client.clone(), "y").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| !f.contains(" 2 "),
            Duration::from_secs(2),
        )
        .await;
    assert!(!f.contains("close tab"), "the question is gone:\n{f}");
}

/// `y` is the only key that closes. Anything else cancels rather than waiting for one of two
/// right answers: a stray keystroke should reach the outcome that keeps the work.
#[tokio::test]
async fn every_other_key_keeps_the_tab() {
    for answer in ["n", "Esc", "q", "z"] {
        let mut h = two_tabs().await;
        h.key(h.client.clone(), "C-a").await;
        h.key(h.client.clone(), "x").await;
        h.wait_for(
            h.client.clone(),
            |f| f.contains("close tab 2?"),
            Duration::from_secs(2),
        )
        .await;
        h.key(h.client.clone(), answer).await;
        let f = h
            .wait_for(
                h.client.clone(),
                |f| !f.contains("close tab 2?"),
                Duration::from_secs(2),
            )
            .await;
        assert!(f.contains(" 2 "), "{answer} kept the tab:\n{f}");
    }
}

/// A caller that sent `tab.close` has already decided. The question protects a person at a
/// keyboard, not a script, and asking one would leave the call waiting for a key that never
/// comes.
#[tokio::test]
async fn the_api_closes_without_asking() {
    let mut h = two_tabs().await;
    h.api("tab.close", json!({})).await.unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| !f.contains(" 2 "),
            Duration::from_secs(2),
        )
        .await;
    assert!(!f.contains("close tab"), "nothing was asked:\n{f}");
}

/// A named tab is named in the question. The number alone would make the reader count cells
/// to check they are about to close the right one.
#[tokio::test]
async fn the_question_carries_the_tabs_name() {
    let mut h = two_tabs().await;
    h.api("tab.rename", json!({"name": "codex"})).await.unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains("2 codex"),
        Duration::from_secs(2),
    )
    .await;
    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), "x").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("close tab 2 codex?"),
        Duration::from_secs(2),
    )
    .await;
}

/// A binding may carry its own target, and then the question has to name that tab rather than
/// the one the reader is looking at.
#[tokio::test]
async fn a_binding_with_a_target_asks_about_that_tab() {
    let mut cfg = Config::default();
    cfg.keys.bindings.insert("x".into(), "tab.close 1".into());
    let mut h = Harness::start(cfg, 80, 10).await;
    h.api("tab.create", json!({})).await.unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains(" 2 "),
        Duration::from_secs(2),
    )
    .await;
    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), "x").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("close tab 1?"),
        Duration::from_secs(2),
    )
    .await;
    h.key(h.client.clone(), "y").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| !f.contains(" 2 "),
            Duration::from_secs(2),
        )
        .await;
    // Tab 1 went, so the tab that was 2 is now the only one and is numbered 1.
    assert!(f.contains(" 1 "), "one tab left:\n{f}");
}
