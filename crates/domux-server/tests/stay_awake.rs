//! Stay awake through the API, the key and the screen (MUX-15, decision 0029).
//!
//! No test here holds a real machine awake: the harness hands the server a fake command
//! runner, which records what it was asked to run and runs none of it.

mod support;

use domux_core::config::{Config, StayAwakeMode};
use domux_server::testing::{row, Harness, HarnessOptions};
use ratatui::style::Color;
use serde_json::json;
use std::time::Duration;
use support::{next_event, subscribe};

/// The dot at the right end, and the two colours it takes.
const DOT: &str = "●";
const GREEN: Color = Color::Rgb(0xa6, 0xe3, 0xa1);
const GREY: Color = Color::Rgb(0x58, 0x5b, 0x70);

fn on(os: &'static str, mode: StayAwakeMode) -> HarnessOptions {
    let mut config = Config::default();
    config.stay_awake.mode = mode;
    HarnessOptions {
        platform: Some(os),
        ..HarnessOptions::new(config, 100, 24)
    }
}

/// A harness on macOS in partial mode, with the holder installed.
async fn macos() -> Harness {
    let h = Harness::start_with(on("macos", StayAwakeMode::Partial)).await;
    h.runner.on_path("caffeinate");
    h
}

#[tokio::test]
async fn enabling_answers_that_the_machine_is_held_awake() {
    let mut h = macos().await;
    let out = h.api("stay_awake.enable", json!({})).await.unwrap();
    assert_eq!(out["on"], true);
    assert!(out.get("note").is_none() || out["note"].is_null());
    assert!(
        h.runner.ran("caffeinate", &["-dimsu"]),
        "{:?}",
        h.runner.calls()
    );
}

#[tokio::test]
async fn toggling_turns_the_hold_on_and_then_off() {
    let mut h = macos().await;
    assert_eq!(
        h.api("stay_awake.toggle", json!({})).await.unwrap()["on"],
        true
    );
    assert_eq!(
        h.api("stay_awake.toggle", json!({})).await.unwrap()["on"],
        false
    );
    assert!(h.runner.ran("kill", &["4242"]), "{:?}", h.runner.calls());
}

#[tokio::test]
async fn each_change_of_the_hold_is_announced_once() {
    let mut h = macos().await;
    let mut events = subscribe(h.socket_path(), &["stay_awake.*"]).await;
    h.api("stay_awake.enable", json!({})).await.unwrap();
    let e = next_event(&mut events).await;
    assert_eq!(e["event"], "stay_awake.changed");
    assert_eq!(e["on"], true);
    // The second enable changes nothing, so it announces nothing and the next event to
    // arrive is the one after it.
    h.api("stay_awake.enable", json!({})).await.unwrap();
    h.api("stay_awake.disable", json!({})).await.unwrap();
    let e = next_event(&mut events).await;
    assert_eq!(e["on"], false);
}

#[tokio::test]
async fn enabling_a_hold_that_is_already_on_answers_on_and_starts_no_second_holder() {
    let mut h = macos().await;
    h.api("stay_awake.enable", json!({})).await.unwrap();
    let again = h.api("stay_awake.enable", json!({})).await.unwrap();
    assert_eq!(again["on"], true);
    assert_eq!(h.runner.calls_to("caffeinate").len(), 1);
}

#[tokio::test]
async fn a_platform_with_no_way_to_hold_it_awake_answers_off_with_the_reason() {
    let mut h = Harness::start_with(on("freebsd", StayAwakeMode::Partial)).await;
    let out = h.api("stay_awake.enable", json!({})).await.unwrap();
    assert_eq!(out["on"], false, "never a hold that is not there");
    assert_eq!(
        out["note"],
        "stay awake works on macOS and Linux, and this machine runs freebsd"
    );
    assert!(h.runner.calls().is_empty());
}

#[tokio::test]
async fn the_dot_at_the_right_end_is_green_while_the_machine_is_held_awake() {
    let mut h = macos().await;
    let f = h.frame(h.client.clone()).await;
    assert!(
        row(&f, 0).contains(DOT),
        "the dot rests at the right end:\n{f}"
    );
    let grey = h.buffer(&h.client.clone());
    let at = dot_column(&grey);
    assert_eq!(grey[(at, 0)].fg, GREY, "grey while nothing is held");

    h.api("stay_awake.enable", json!({})).await.unwrap();
    h.frame(h.client.clone()).await;
    let green = h.buffer(&h.client.clone());
    assert_eq!(green[(dot_column(&green), 0)].fg, GREEN);

    h.api("stay_awake.disable", json!({})).await.unwrap();
    h.frame(h.client.clone()).await;
    let back = h.buffer(&h.client.clone());
    assert_eq!(back[(dot_column(&back), 0)].fg, GREY);
}

/// The dot keeps its place while the right end has something to say, which is the whole
/// reason it is not one of the pieces in that priority chain (decision 0029).
#[tokio::test]
async fn the_dot_stays_while_the_right_end_shows_something_else() {
    let mut h = macos().await;
    h.api("stay_awake.enable", json!({})).await.unwrap();
    h.key(h.client.clone(), "C-a").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("keys"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        row(&f, 0).contains(DOT),
        "the chord indicator did not take it:\n{f}"
    );
    let buf = h.buffer(&h.client.clone());
    assert_eq!(buf[(dot_column(&buf), 0)].fg, GREEN);
}

#[tokio::test]
async fn a_toast_says_which_way_stay_awake_went() {
    let mut h = macos().await;
    h.api("stay_awake.enable", json!({})).await.unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("Stay awake turned on"),
            Duration::from_secs(2),
        )
        .await;
    // The bottom right of the workpanel, above the last row rather than on it.
    let rows: Vec<usize> = (0..24)
        .filter(|n| row(&f, *n).contains("Stay awake turned on"))
        .collect();
    assert_eq!(rows.len(), 1, "one toast:\n{f}");
    assert!(rows[0] > 12, "the lower half of the screen:\n{f}");

    h.api("stay_awake.disable", json!({})).await.unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains("Stay awake turned off"),
        Duration::from_secs(2),
    )
    .await;
}

#[tokio::test]
async fn the_toast_goes_away_once_its_time_is_up() {
    let mut h = macos().await;
    h.api("stay_awake.enable", json!({})).await.unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains("Stay awake turned on"),
        Duration::from_secs(2),
    )
    .await;
    h.advance(Duration::from_secs(7));
    let f = h
        .wait_for(
            h.client.clone(),
            |f| !f.contains("Stay awake turned on"),
            Duration::from_secs(3),
        )
        .await;
    assert!(!f.contains("Stay awake"), "{f}");
}

#[tokio::test]
async fn the_key_toggles_the_hold() {
    let mut h = macos().await;
    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), "A").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("Stay awake turned on"),
        Duration::from_secs(2),
    )
    .await;
    assert!(
        h.runner.ran("caffeinate", &["-dimsu"]),
        "{:?}",
        h.runner.calls()
    );
}

#[tokio::test]
async fn the_hold_is_given_back_when_the_server_stops() {
    let mut h = macos().await;
    h.api("stay_awake.enable", json!({})).await.unwrap();
    assert!(h
        .state_dir()
        .join(domux_server::stay_awake::PID_FILE_NAME)
        .exists());
    h.stop().await;
    assert!(h.runner.ran("kill", &["4242"]), "{:?}", h.runner.calls());
    assert!(!h
        .state_dir()
        .join(domux_server::stay_awake::PID_FILE_NAME)
        .exists());
}

#[tokio::test]
async fn a_hold_the_state_file_remembers_is_taken_again_at_start() {
    let mut h = macos().await;
    h.api("stay_awake.enable", json!({})).await.unwrap();
    h.frame(h.client.clone()).await;
    h.restart().await;
    let out = h.api("server.info", json!({})).await.unwrap();
    assert_eq!(out["stay_awake"], true, "the switch was left on");
    assert_eq!(
        h.runner.calls_to("caffeinate").len(),
        2,
        "the first hold ended with the first server, so the second takes its own"
    );
}

#[tokio::test]
async fn a_machine_nobody_asked_to_hold_awake_starts_with_no_hold() {
    let mut h = macos().await;
    let out = h.api("server.info", json!({})).await.unwrap();
    assert_eq!(out["stay_awake"], false);
    assert!(h.runner.calls().is_empty(), "{:?}", h.runner.calls());
}

/// The column the dot stands in, which is the last cell of the top bar that holds one.
fn dot_column(buf: &ratatui::buffer::Buffer) -> u16 {
    (0..buf.area.width)
        .rev()
        .find(|x| buf[(*x, 0)].symbol() == DOT)
        .expect("a dot on the top bar")
}
