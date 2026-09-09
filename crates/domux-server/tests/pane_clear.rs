//! `pane.clear`: emptying a pane's screen and its scrollback from domux's side.
//!
//! The key exists because typing `clear` is asking the program in the pane to do it, and the
//! program is exactly what is broken when you want it. A shell whose line editor is holding
//! the reply to a device attributes query - what a binary file printed to a terminal leaves
//! behind - will not run the `clear` you typed, because what it has is not what you typed.

use domux_core::config::Config;
use domux_server::testing::{row, Harness};
use std::time::Duration;

/// A pane holding more output than its screen, so there is scrollback as well as a screen.
async fn pane_with_lines(h: &mut Harness, n: usize) -> domux_core::ids::PaneId {
    let pane = h.focused_pane(h.client.clone());
    let mut bytes = Vec::new();
    for i in 0..n {
        bytes.extend_from_slice(format!("line {i:02}\r\n").as_bytes());
    }
    h.feed_pane(pane.clone(), &bytes).await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains(&format!("line {:02}", n - 1)),
        Duration::from_secs(2),
    )
    .await;
    pane
}

/// The whole text of a pane, scrollback included, as `pane.read` answers it.
async fn everything_in(h: &mut Harness, pane: &domux_core::ids::PaneId) -> String {
    let read = h
        .api(
            "pane.read",
            serde_json::json!({ "pane": pane.to_string(), "lines": 500 }),
        )
        .await
        .expect("pane.read");
    read["text"]
        .as_str()
        .expect("read answers text")
        .to_string()
}

#[tokio::test]
async fn leader_k_empties_the_pane_and_its_scrollback() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    let pane = pane_with_lines(&mut h, 40).await;
    assert!(
        everything_in(&mut h, &pane).await.contains("line 00"),
        "the scrollback holds what scrolled off"
    );
    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), "k").await;
    h.wait_for(
        h.client.clone(),
        |f| !f.contains("line 39"),
        Duration::from_secs(2),
    )
    .await;
    assert_eq!(
        everything_in(&mut h, &pane).await,
        "",
        "nothing is left to scroll back to"
    );
    h.stop().await;
}

/// The same operation from the other two surfaces, because a key, a subcommand and an API
/// call reach one handler.
#[tokio::test]
async fn the_api_call_empties_the_pane_too() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    let pane = pane_with_lines(&mut h, 40).await;
    h.api(
        "pane.clear",
        serde_json::json!({ "pane": pane.to_string() }),
    )
    .await
    .expect("pane.clear");
    h.wait_for(
        h.client.clone(),
        |f| !f.contains("line 39"),
        Duration::from_secs(2),
    )
    .await;
    assert_eq!(everything_in(&mut h, &pane).await, "");
    h.stop().await;
}

/// Nothing is typed at the program. That is the whole point: the pane is emptied whatever
/// its program is doing with its input, and a shell that never saw a keystroke has nothing
/// to echo back.
#[tokio::test]
async fn clearing_sends_the_program_nothing() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    let pane = pane_with_lines(&mut h, 40).await;
    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), "k").await;
    h.wait_for(
        h.client.clone(),
        |f| !f.contains("line 39"),
        Duration::from_secs(2),
    )
    .await;
    assert!(
        h.pane_input(&pane).is_empty(),
        "the pane's program was sent {:?}",
        String::from_utf8_lossy(&h.pane_input(&pane))
    );
    h.stop().await;
}

/// A full screen program owns its screen and will not know to redraw it, and there is no
/// scrollback on the alternate screen to take away.
#[tokio::test]
async fn a_full_screen_program_is_refused() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    let pane = h.focused_pane(h.client.clone());
    h.feed_pane(pane.clone(), b"\x1b[?1049h\x1b[Hedit me").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("edit me"),
        Duration::from_secs(2),
    )
    .await;
    let refused = h
        .api(
            "pane.clear",
            serde_json::json!({ "pane": pane.to_string() }),
        )
        .await
        .expect_err("a full screen program keeps its screen");
    assert!(
        refused.message.contains("full screen program"),
        "{}",
        refused.message
    );
    let f = h.frame(h.client.clone()).await;
    assert!(f.contains("edit me"), "the screen is untouched:\n{f}");
    h.stop().await;
}

/// Copy mode walks the history this just took away, so it ends with it rather than being
/// left pointing at rows that are gone.
#[tokio::test]
async fn clearing_ends_copy_mode() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    let pane = pane_with_lines(&mut h, 40).await;
    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), "[").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains(" copy "),
        Duration::from_secs(2),
    )
    .await;
    assert!(h.model().pane(&pane).unwrap().copy_mode);
    h.api(
        "pane.clear",
        serde_json::json!({ "pane": pane.to_string() }),
    )
    .await
    .expect("pane.clear");
    h.wait_for(
        h.client.clone(),
        |f| !f.contains(" copy "),
        Duration::from_secs(2),
    )
    .await;
    assert!(!h.model().pane(&pane).unwrap().copy_mode);
    h.stop().await;
}

/// The pane is left able to draw: the cursor goes home, so what the program writes next
/// lands at the top rather than where the old output ended.
#[tokio::test]
async fn what_the_program_writes_next_starts_at_the_top() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    let pane = pane_with_lines(&mut h, 40).await;
    h.api(
        "pane.clear",
        serde_json::json!({ "pane": pane.to_string() }),
    )
    .await
    .expect("pane.clear");
    h.feed_pane(pane.clone(), b"$ ").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("$ "),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        row(&f, 2).starts_with("|\u{2502}$ "),
        "the prompt is on the pane's first row:\n{f}"
    );
    h.stop().await;
}

/// A pane the model does not hold is a not found, not a silent success.
#[tokio::test]
async fn clearing_a_pane_that_is_not_there_is_a_failure() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    let refused = h
        .api("pane.clear", serde_json::json!({ "pane": "p_9999" }))
        .await
        .expect_err("no such pane");
    assert!(refused.message.contains("p_9999"), "{}", refused.message);
    h.stop().await;
}
