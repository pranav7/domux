//! The switcher overlay: `leader s` opens the Projects box over the screen with its footer
//! under it, Esc closes it and gives the keys back to the pane.

use domux_core::config::Config;
use domux_server::testing::{row, Harness};
use std::time::Duration;

/// Columns `from` to `to` of a frame row, without the `|` framing.
fn cols(line: &str, from: usize, to: usize) -> String {
    line.chars().skip(1 + from).take(to - from + 1).collect()
}

/// The style runs the frame lists for row `r`.
fn styles(frame: &str, r: usize) -> Vec<&str> {
    let prefix = format!("r{r} c");
    frame
        .lines()
        .filter(|l| l.starts_with(prefix.as_str()))
        .collect()
}

/// The column `ch` sits in, without the `|` framing. By character and not by `str::find`,
/// which answers in bytes: every border glyph on the row ahead of it is three bytes wide and
/// one cell wide, so the two answers differ by twice the number of borders passed.
fn col_of(line: &str, ch: char) -> usize {
    line.chars()
        .position(|c| c == ch)
        .unwrap_or_else(|| panic!("no {ch:?} in {line:?}"))
        - 1
}

async fn open_switcher(h: &mut Harness) -> String {
    h.api("switcher.open", serde_json::json!({})).await.unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains("Projects"),
        Duration::from_secs(2),
    )
    .await
}

#[tokio::test]
async fn leader_s_opens_the_switcher_with_the_projects_box_and_the_footer() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), "s").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("Projects"),
            Duration::from_secs(2),
        )
        .await;
    // `cols(line, 10, 69)` takes 60 display cells, so every literal here is exactly 60.
    // Each is one border cell, its content, and one border cell; no `.take` is needed and
    // none is used, because a `.take` that trims nothing hides a literal that is too short.
    // 60 = "┌ Projects " (11) + 48 dashes + "┐" (1)
    assert_eq!(
        cols(row(&f, 3), 10, 69),
        "┌ Projects ────────────────────────────────────────────────┐",
        "{f}"
    );
    // 60 = "│PROJ " (6) + 53 dashes + "│" (1)
    assert_eq!(
        cols(row(&f, 4), 10, 69),
        "│PROJ ─────────────────────────────────────────────────────│"
    );
    // 60 = "│main" (5) + 54 spaces + "│" (1)
    assert_eq!(
        cols(row(&f, 5), 10, 69),
        "│main                                                      │"
    );
    // The tab list, which only the switcher's width asks for (interface spec 5.5): the
    // sidebar's `Extras::compact` leaves this line out, so it is what separates the two
    // surfaces and it is the reason the box is five rows and not four.
    // 60 = "│1" (2) + 57 spaces + "│" (1)
    assert_eq!(
        cols(row(&f, 6), 10, 69),
        "│1                                                         │"
    );
    // 60 = "└" (1) + 58 dashes + "┘" (1)
    assert_eq!(
        cols(row(&f, 7), 10, 69),
        "└──────────────────────────────────────────────────────────┘"
    );
    // `cols(line, 11, 48)` takes 38 cells, and the footer is 38.
    assert_eq!(
        cols(row(&f, 8), 11, 48),
        "⏎ open · / filter · ? help · esc close"
    );
    assert!(
        f.contains("r3 c10-10 fg=#cba6f7"),
        "the switcher's box is the focused region, and the overlay is not dimmed with the screen behind it:\n{f}"
    );
    assert!(
        f.contains("r1 c1-4 dim fg=#7f849c"),
        "and the pane behind it is not: the keys are in the box, so nothing else is drawn as a focus target (principle 2):\n{f}"
    );
    assert!(
        f.contains("r8 c11-11 fg=#89b4fa"),
        "the footer belongs to the overlay, not to the screen behind it, so its keys are not dimmed either:\n{f}"
    );
    assert_eq!(
        h.model().client(&h.client).unwrap().overlay,
        Some(domux_core::model::Overlay::Switcher)
    );
}

#[tokio::test]
async fn the_cursor_starts_on_the_current_workspace_and_the_fill_marks_it() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let f = open_switcher(&mut h).await;
    // Two runs, not one: the fill band is `#313244` across the whole inner width, and the
    // filled row's own text goes from `main`'s dim `#7f849c` to `text` (interface spec 5.3),
    // which is what splits the band into a styled run and a bare one.
    assert!(
        f.contains("r5 c11-14 fg=#cdd6f4 bg=#313244"),
        "main is filled because this client is in it, and its text brightened:\n{f}"
    );
    assert!(
        f.contains("r5 c15-68 bg=#313244"),
        "the fill covers the whole inner width, not just the text:\n{f}"
    );
    assert!(
        styles(&f, 6).iter().all(|l| !l.contains("#313244")),
        "the fill is on line 1 of the row, not on the tab list under it:\n{f}"
    );
    let view = h.model().client(&h.client).unwrap().clone();
    assert_eq!(view.projects_cursor.as_ref(), Some(&view.workspace));
}

#[tokio::test]
async fn the_screen_under_the_switcher_dims_and_comes_back_when_it_closes() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let f = open_switcher(&mut h).await;
    assert!(
        f.contains("r1 c0-0 dim"),
        "the pane border beneath is dimmed:\n{f}"
    );
    h.key(h.client.clone(), "Esc").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| !f.contains("Projects"),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(
        row(&f, 0),
        "| proj › main  1 │ + │                                         14:32   Fri 4 Sep |",
        "{f}"
    );
    assert!(
        f.contains("r1 c1-4 bold fg=#cba6f7"),
        "focus is back on the pane, and nothing is left dimmed:\n{f}"
    );
    assert_eq!(h.model().client(&h.client).unwrap().overlay, None);
}

/// The overlay's whole job: what was on the screen is covered, and only inside its own
/// rectangle.
///
/// The pane is filled with text first. Every other render test in this milestone starts
/// from an empty buffer, where a draw that clears nothing looks exactly like one that does.
#[tokio::test]
async fn the_switcher_covers_the_panes_text_and_leaves_the_rest_of_the_screen_where_it_was() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let pane = h.focused_pane(h.client.clone());
    let filled_line = "X".repeat(78);
    let mut text = String::new();
    for _ in 0..21 {
        text.push_str(&filled_line);
        text.push_str("\r\n");
    }
    h.feed_pane(pane, text.as_bytes()).await;
    h.wait_for(
        h.client.clone(),
        |f| row(f, 5).contains("XXXX"),
        Duration::from_secs(2),
    )
    .await;
    let f = open_switcher(&mut h).await;
    // Rows 4 to 6 are the ones the box leaves partly blank inside: the header's rule reaches
    // the border, but `main` and the tab list stop after a few cells, so an overlay that
    // drew without clearing would show the pane's text in the rest of the line.
    assert_eq!(
        cols(row(&f, 5), 10, 69),
        "│main                                                      │",
        "the box's own row, with nothing of the pane left in it:\n{f}"
    );
    assert_eq!(
        cols(row(&f, 6), 10, 69),
        "│1                                                         │",
        "{f}"
    );
    assert_eq!(
        cols(row(&f, 8), 11, 48),
        "⏎ open · / filter · ? help · esc close",
        "and the footer's row is the footer's:\n{f}"
    );
    assert_eq!(
        cols(row(&f, 8), 49, 69),
        " ".repeat(21),
        "the rest of the footer's row is cleared too, not left showing the pane:\n{f}"
    );
    assert_eq!(
        cols(row(&f, 8), 70, 78),
        "XXXXXXXXX",
        "and the footer clears its own 60 columns and no more:\n{f}"
    );
    // The other half: outside the overlay the screen is still there, dimmed rather than
    // painted over.
    assert_eq!(cols(row(&f, 5), 1, 9), "XXXXXXXXX", "{f}");
    assert_eq!(cols(row(&f, 5), 70, 78), "XXXXXXXXX", "{f}");
    assert_eq!(
        cols(row(&f, 2), 1, 78),
        "X".repeat(78),
        "the row above the overlay is untouched:\n{f}"
    );
    assert_eq!(
        cols(row(&f, 9), 1, 78),
        "X".repeat(78),
        "and the row under the footer:\n{f}"
    );
    assert!(
        f.contains("r5 c1-9 dim"),
        "what the overlay did not cover reads as being behind it:\n{f}"
    );
}

#[tokio::test]
async fn the_switcher_is_never_narrower_than_the_sidebar_and_never_taller_than_the_screen() {
    let mut h = Harness::start(Config::default(), 200, 50).await;
    let f = open_switcher(&mut h).await;
    assert_eq!(row(&f, 3).matches('┌').count(), 1);
    let left = col_of(row(&f, 3), '┌');
    assert_eq!(
        left, 40,
        "at 200 columns the overlay is capped at 120 and centred"
    );
    assert_eq!(
        col_of(row(&f, 3), '┐'),
        left + 119,
        "120 columns wide, so 40 columns of screen are left on each side:\n{f}"
    );
    let small = h.attach(60, 12).await;
    h.api(
        "switcher.open",
        serde_json::json!({"client": small.as_str()}),
    )
    .await
    .unwrap();
    let f = h
        .wait_for(small, |f| f.contains("Projects"), Duration::from_secs(2))
        .await;
    assert!(
        row(&f, 3).contains("┌ Projects"),
        "a 60 column screen still gets the box:\n{f}"
    );
    assert_eq!(
        f.lines().filter(|l| l.starts_with('|')).count(),
        12,
        "nothing is drawn past the screen"
    );
    assert!(
        row(&f, 8).contains("⏎ open"),
        "and the footer still has its row inside the screen:\n{f}"
    );
}

/// Esc returns to the overlay the switcher was opened over, not straight to the pane
/// (interface spec 12.7).
#[tokio::test]
async fn closing_the_switcher_returns_to_the_overlay_it_was_opened_over() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    h.api("help", serde_json::json!({})).await.unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains("┌ Keys"),
        Duration::from_secs(2),
    )
    .await;
    let f = open_switcher(&mut h).await;
    assert!(
        f.contains("┌ Keys"),
        "the help overlay is still drawn under the switcher:\n{f}"
    );
    h.key(h.client.clone(), "Esc").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| !f.contains("Projects"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        f.contains("┌ Keys"),
        "and it is what Esc goes back to:\n{f}"
    );
    let view = h.model().client(&h.client).unwrap().clone();
    assert_eq!(view.overlay, Some(domux_core::model::Overlay::Help));
    assert_eq!(view.overlay_under, None);
    assert_eq!(
        view.focus,
        domux_core::model::Focus::Region(domux_core::model::RegionKind::Overlay),
        "the keys are in the overlay that is open, not in the box that closed"
    );
}

/// The footer names the configured keys, a hint whose action is bound to nothing drops out
/// of the row rather than opening it with a separator, and the key that closes the switcher
/// is the configured one and not `Esc` by name (principle 3).
#[tokio::test]
async fn the_switcher_reads_its_keys_from_the_config_rather_than_naming_esc_itself() {
    let mut config = Config::default();
    config.keys.list.remove("Enter");
    config.keys.list.remove("/");
    config.keys.list.insert("f".into(), "list.filter".into());
    config.keys.list.remove("Esc");
    config.keys.list.insert("q".into(), "focus.pane".into());
    let mut h = Harness::start(config, 80, 24).await;
    let f = open_switcher(&mut h).await;
    assert_eq!(
        cols(row(&f, 8), 11, 48),
        "f filter · ? help · q close           ",
        "{f}"
    );
    h.key(h.client.clone(), "Esc").await;
    let f = h.frame(h.client.clone()).await;
    assert!(
        f.contains("Projects"),
        "Esc is bound to nothing here, so it does not close the switcher:\n{f}"
    );
    h.key(h.client.clone(), "q").await;
    h.wait_for(
        h.client.clone(),
        |f| !f.contains("Projects"),
        Duration::from_secs(2),
    )
    .await;
}

/// A key the switcher has no meaning for yet leaves it open. Task 14 gives `j` and `k` their
/// meaning; until then they must not fall through into "any key closes".
#[tokio::test]
async fn a_key_the_switcher_does_not_answer_leaves_it_open() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    open_switcher(&mut h).await;
    h.key(h.client.clone(), "j").await;
    let f = h.frame(h.client.clone()).await;
    assert!(f.contains("Projects"), "{f}");
    assert_eq!(
        h.model().client(&h.client).unwrap().overlay,
        Some(domux_core::model::Overlay::Switcher)
    );
    assert!(
        h.pane_input(&h.focused_pane(h.client.clone())).is_empty(),
        "and it does not reach the pane either: the overlay takes every key"
    );
}

/// `switcher.close` closes the switcher and nothing else. The API can send it at any time,
/// and an overlay it did not open is not its to close.
#[tokio::test]
async fn closing_the_switcher_when_another_overlay_is_open_leaves_that_one_alone() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    h.api("help", serde_json::json!({})).await.unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains("┌ Keys"),
        Duration::from_secs(2),
    )
    .await;
    h.api("switcher.close", serde_json::json!({}))
        .await
        .unwrap();
    let f = h.frame(h.client.clone()).await;
    assert!(f.contains("┌ Keys"), "the help overlay is still open:\n{f}");
    assert_eq!(
        h.model().client(&h.client).unwrap().overlay,
        Some(domux_core::model::Overlay::Help)
    );
}

/// A call naming a client that is not attached is refused, and nothing is changed on the way
/// to refusing it: the client that is attached keeps the screen it had.
#[tokio::test]
async fn opening_the_switcher_for_a_client_that_is_not_attached_changes_nothing() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let before = h.frame(h.client.clone()).await;
    let err = h
        .api("switcher.open", serde_json::json!({"client": "c_9999"}))
        .await
        .unwrap_err();
    assert_eq!(err.code, domux_core::api::ErrorCode::NotFound, "{err}");
    assert!(err.message.contains("c_9999"), "{err}");
    assert_eq!(h.model().client(&h.client).unwrap().overlay, None);
    assert_eq!(
        h.frame(h.client.clone()).await,
        before,
        "the attached client's screen is untouched"
    );
}

/// Asking for a switcher that is already open leaves the one that is open alone, rather than
/// stacking a second one over it: one Esc has to be enough to get out.
#[tokio::test]
async fn opening_the_switcher_twice_still_closes_on_one_esc() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    open_switcher(&mut h).await;
    h.api("switcher.open", serde_json::json!({})).await.unwrap();
    assert_eq!(h.model().client(&h.client).unwrap().overlay_under, None);
    h.key(h.client.clone(), "Esc").await;
    h.wait_for(
        h.client.clone(),
        |f| !f.contains("Projects"),
        Duration::from_secs(2),
    )
    .await;
    assert_eq!(h.model().client(&h.client).unwrap().overlay, None);
}

/// Closing over the control API redraws the screen on its own.
///
/// Every other close here is a keystroke, and a key marks the view dirty whatever it does, so
/// none of them can tell whether the frame arrived because `switcher.close` asked for it or
/// because a key had been pressed. This one removes that second cause: no key is sent, so the
/// only thing that can redraw is the handler.
#[tokio::test]
async fn closing_the_switcher_over_the_api_redraws_without_a_key() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    open_switcher(&mut h).await;
    h.api("switcher.close", serde_json::json!({}))
        .await
        .unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| !f.contains("Projects"),
            Duration::from_secs(2),
        )
        .await;
    assert!(f.contains("proj › main"), "the screen is back:\n{f}");
    assert_eq!(h.model().client(&h.client).unwrap().overlay, None);
}
