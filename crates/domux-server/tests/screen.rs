//! The first whole screen: the top bar, the tab row and one pane box, drawn through the
//! headless harness so a failure prints the picture that is wrong.

use domux_core::config::Config;
use domux_server::testing::{row, Harness};
use domux_term::Size;
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

#[tokio::test]
async fn erased_pane_row_keeps_its_background_through_the_right_edge() {
    let mut h = Harness::start(Config::default(), 40, 10).await;
    let pane = h.focused_pane(h.client.clone());
    h.feed_pane(pane, b"\x1b[48;2;10;20;30m\x1b[2K").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("r2 c1-38 bg=#0a141e"),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(
        row(&f, 2),
        "|│                                      │|",
        "{f}"
    );
}

#[tokio::test]
async fn a_second_client_sees_the_same_tab_and_the_smaller_client_sizes_the_pane() {
    let mut h = Harness::start(Config::default(), 60, 20).await;
    let second = h.attach(40, 10).await;
    let pane = h.focused_pane(h.client.clone());
    let f2 = h.frame(second.clone()).await;
    assert_eq!(
        row(&f2, 1),
        "|┌ sh ──────────────────────────────────┐|",
        "{f2}"
    );
    let f1 = h.frame(h.client.clone()).await;
    assert!(
        row(&f1, 1).starts_with("|┌ sh ──────────────────────────────────┐"),
        "the box is the small client's size:\n{f1}"
    );
    // The pane's own screen is the box's inside: 40 - 2 columns, and 10 rows less the top
    // bar and the two rules.
    assert_eq!(h.pane_size(&pane), Size { cols: 38, rows: 7 });
}

#[tokio::test]
async fn a_screen_below_the_minimum_says_what_it_needs() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let pane = h.focused_pane(h.client.clone());
    assert_eq!(h.pane_size(&pane), Size { cols: 78, rows: 21 });
    let small = h.attach(30, 8).await;
    assert_eq!(h.pane_size(&pane), Size { cols: 78, rows: 21 });
    let f = h.frame(small.clone()).await;
    // The sentence is 43 cells and the screen is 30, so it wraps. Clipping it would drop
    // `40x10.` - the size the reader has to reach, which is the point of the notice.
    assert_eq!(row(&f, 0), "|Screen is 30x8. domux needs at|", "{f}");
    assert_eq!(row(&f, 1), "|least 40x10.                  |", "{f}");
    h.detach(small.clone()).await;
    assert_eq!(h.pane_size(&pane), Size { cols: 78, rows: 21 });
    let resized = h.attach(40, 10).await;
    h.resize(resized.clone(), 40, 10).await;
    let f = h
        .wait_for(resized, |f| f.contains("┌ sh"), Duration::from_secs(2))
        .await;
    assert_eq!(
        row(&f, 0),
        "| proj › main  1 │ + │ 14:32   Fri 4 Sep |",
        "{f}"
    );
}

/// Far below the minimum the notice degrades rather than dying: it wraps to as many rows as
/// the screen has and stops, with no panic and nothing drawn past an edge. At 10x5 the last
/// line does not fit, which is the honest outcome at that size.
#[tokio::test]
async fn a_screen_far_below_the_minimum_draws_what_fits_and_does_not_panic() {
    let mut h = Harness::start(Config::default(), 10, 5).await;
    let f = h.frame(h.client.clone()).await;
    assert_eq!(row(&f, 0), "|Screen is |", "{f}");
    assert_eq!(row(&f, 1), "|10x5.     |", "{f}");
    assert_eq!(row(&f, 2), "|domux     |", "{f}");
    assert_eq!(row(&f, 3), "|needs at  |", "{f}");
    assert_eq!(row(&f, 4), "|least     |", "{f}");
}
