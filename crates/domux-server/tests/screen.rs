//! The first whole screen: the top bar, the tab row and one pane box, drawn through the
//! headless harness so a failure prints the picture that is wrong.

use domux_core::config::Config;
use domux_server::testing::{row, Harness};
use std::time::Duration;

#[tokio::test]
async fn top_bar_shows_location_tabs_plus_and_clock() {
    let mut h = Harness::start(Config::default(), 40, 10).await;
    let f = h.frame(h.client.clone()).await;
    assert_eq!(
        row(&f, 0),
        "| proj › main  1 │ + │ 14:32   Fri 4 Sep |",
        "{f}"
    );
    assert!(
        f.contains("r0 c13-15 bold fg=#1e1e2e bg=#cba6f7"),
        "current tab filled accent with base text:\n{f}"
    );
    assert!(
        f.contains("r0 c0-12 bold fg=#cdd6f4 bg=#181825"),
        "location in bold text on mantle:\n{f}"
    );
    assert!(
        f.contains("r0 c22-38 fg=#a6adc8 bg=#181825"),
        "clock in subtext0:\n{f}"
    );
}

#[tokio::test]
async fn one_pane_box_fills_the_workpanel_with_the_foreground_command_as_title() {
    let mut h = Harness::start(Config::default(), 40, 10).await;
    let f = h.frame(h.client.clone()).await;
    assert_eq!(
        row(&f, 1),
        "|┌ sh ──────────────────────────────────┐|",
        "{f}"
    );
    assert_eq!(row(&f, 2), "|│                                      │|");
    assert_eq!(row(&f, 9), "|└──────────────────────────────────────┘|");
    assert!(
        f.contains("r1 c0-0 fg=#cba6f7"),
        "focused border is accent:\n{f}"
    );
    assert!(
        f.contains("r1 c1-4 bold fg=#cba6f7"),
        "focused title is bold accent:\n{f}"
    );
}

#[tokio::test]
async fn pane_output_appears_inside_the_box_and_moves_the_cursor() {
    let mut h = Harness::start(Config::default(), 40, 10).await;
    let pane = h.focused_pane(h.client.clone());
    h.feed_pane(pane, b"hello").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("│hello"),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(
        row(&f, 2),
        "|│hello                                 │|",
        "{f}"
    );
    assert!(
        f.contains("cursor row=2 col=6 visible=true shape=block"),
        "{f}"
    );
}
