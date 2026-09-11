//! Copy mode: entering it, walking the scrollback, selecting and copying.

use domux_core::config::Config;
use domux_server::testing::{row, Harness};
use std::time::Duration;

async fn pane_with_lines(h: &mut Harness, n: usize) -> domux_core::ids::PaneId {
    let pane = h.focused_pane(h.client.clone());
    let mut bytes = Vec::new();
    for i in 0..n {
        bytes.extend_from_slice(format!("line {i:02}\r\n").as_bytes());
    }
    bytes.extend_from_slice(b"$ ");
    h.feed_pane(pane.clone(), &bytes).await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("$ "),
        Duration::from_secs(2),
    )
    .await;
    pane
}

#[tokio::test]
async fn leader_bracket_enters_copy_mode_and_marks_the_border() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    let pane = pane_with_lines(&mut h, 20).await;
    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), "[").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains(" copy "),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(
        row(&f, 1),
        "|┌ sh ──────────────────────────────────────────────────────────────────── copy ┐|",
        "{f}"
    );
    assert!(
        row(&f, 0).ends_with("v select · ⏎ copy · esc leave ● |"),
        "{f}"
    );
    assert!(h.model().pane(&pane).unwrap().copy_mode);
    // Copy mode is a focused region inside the pane the keys already went to, not a second
    // one: the accent fill is still the current tab's cell and only it. Style runs are
    // collapsed by style, so one line ending in the accent background is one run
    // (principle 2).
    assert_eq!(
        f.lines().filter(|l| l.ends_with("bg=#cba6f7")).count(),
        1,
        "{f}"
    );
    h.key(h.client.clone(), "j").await;
    h.frame(h.client.clone()).await;
    assert!(h.pane_input(&pane).is_empty(), "keys stay in copy mode");
    h.key(h.client.clone(), "Esc").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| !f.contains(" copy "),
            Duration::from_secs(2),
        )
        .await;
    assert!(f.contains("14:32"), "{f}");
    assert!(!h.model().pane(&pane).unwrap().copy_mode);
}

#[tokio::test]
async fn moving_above_the_top_scrolls_into_the_scrollback_and_the_flag_counts() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    pane_with_lines(&mut h, 20).await;
    // 7 visible rows: line 14 to line 19 and the prompt; 14 lines are in the scrollback.
    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), "[").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains(" copy "),
        Duration::from_secs(2),
    )
    .await;
    for _ in 0..8 {
        h.key(h.client.clone(), "k").await;
    }
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("copy 2/14"),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(
        row(&f, 2),
        "|│line 12                                                                       │|",
        "{f}"
    );
    assert!(
        f.contains("cursor row=2 col=3"),
        "the copy cursor kept its column:\n{f}"
    );
    h.key(h.client.clone(), "g").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("copy 14/14"),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(
        row(&f, 2),
        "|│line 00                                                                       │|"
    );
    h.key(h.client.clone(), "G").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains(" copy ┐"),
            Duration::from_secs(2),
        )
        .await;
    assert!(f.contains("│$ "), "{f}");
}

#[tokio::test]
async fn wheel_scroll_moves_the_pane_under_the_pointer_immediately() {
    let mut h = Harness::start(Config::default(), 40, 10).await;
    let left = pane_with_lines(&mut h, 20).await;
    let right = h
        .api("pane.split", serde_json::json!({"dir": "right"}))
        .await
        .unwrap()["id"]
        .as_str()
        .map(|id| domux_core::ids::PaneId(id.into()))
        .expect("the split returns its pane");
    h.frame(h.client.clone()).await;

    // The right pane has keyboard focus, but column 5 is inside the left pane's box.
    // A wheel step scrolls the pointed-at pane at once rather than walking its copy cursor
    // through the visible rows first.
    h.scroll(h.client.clone(), 5, 5, 3).await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("copy 3/14"),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(h.focused_pane(h.client.clone()), left);
    assert!(h.model().pane(&left).unwrap().copy_mode);
    assert!(!h.model().pane(&right).unwrap().copy_mode);
    assert!(
        f.contains("line 11"),
        "the viewport moved into history:\n{f}"
    );

    h.scroll(h.client.clone(), 5, 5, -2).await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("copy 1/14"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        f.contains("line 13"),
        "the viewport moved towards live:\n{f}"
    );
}

#[tokio::test]
async fn v_enter_copies_the_selection_to_the_client_and_leaves() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    pane_with_lines(&mut h, 20).await;
    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), "[").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains(" copy "),
        Duration::from_secs(2),
    )
    .await;
    // The cursor starts at the prompt (row 6, col 2). Go up two lines to the start of line 18.
    h.key(h.client.clone(), "k").await;
    h.key(h.client.clone(), "k").await;
    h.key(h.client.clone(), "0").await;
    h.key(h.client.clone(), "v").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("⏎ copy · esc leave ● |") && !f.contains("v select"),
            Duration::from_secs(2),
        )
        .await;
    assert!(f.contains("inverse"), "the selection is reversed:\n{f}");
    h.key(h.client.clone(), "j").await;
    h.key(h.client.clone(), "$").await;
    h.key(h.client.clone(), "Enter").await;
    h.wait_for(
        h.client.clone(),
        |f| !f.contains(" copy "),
        Duration::from_secs(2),
    )
    .await;
    assert_eq!(
        h.clipboard(h.client.clone()).await,
        vec!["line 18\nline 19".to_string()]
    );
}

#[tokio::test]
async fn enter_without_a_selection_leaves_and_copies_nothing() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    pane_with_lines(&mut h, 3).await;
    h.api("pane.copy_mode", serde_json::json!({}))
        .await
        .unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains(" copy "),
        Duration::from_secs(2),
    )
    .await;
    h.key(h.client.clone(), "Enter").await;
    h.wait_for(
        h.client.clone(),
        |f| !f.contains(" copy "),
        Duration::from_secs(2),
    )
    .await;
    assert!(h.clipboard(h.client.clone()).await.is_empty());
}

/// A selection can be made and still hold no text. Nothing is copied, and the reason is
/// named rather than left to a clipboard message that never arrives (principles 8 and 9).
#[tokio::test]
async fn enter_over_blank_cells_copies_nothing_and_says_so() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    pane_with_lines(&mut h, 3).await;
    h.api("pane.copy_mode", serde_json::json!({}))
        .await
        .unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains(" copy "),
        Duration::from_secs(2),
    )
    .await;
    // The cursor sits on the prompt at column 2; every cell to the right of it is blank.
    h.key(h.client.clone(), "l").await;
    h.key(h.client.clone(), "v").await;
    h.key(h.client.clone(), "l").await;
    h.key(h.client.clone(), "l").await;
    h.key(h.client.clone(), "Enter").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| !f.contains(" copy "),
            Duration::from_secs(2),
        )
        .await;
    assert!(f.contains("nothing to copy: the selection is blank"), "{f}");
    assert!(h.clipboard(h.client.clone()).await.is_empty());
}

/// The alternate screen has no history of its own, and the primary screen's history is not
/// its to walk. Movement stays on the visible screen and the flag keeps no count, but the
/// screen itself still copies: copy mode inside a full-screen program is not an empty box.
#[tokio::test]
async fn copy_mode_on_the_alternate_screen_stays_on_the_visible_screen() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    let pane = pane_with_lines(&mut h, 20).await;
    h.feed_pane(pane.clone(), b"\x1b[?1049h\x1b[Hedit me").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("edit me"),
        Duration::from_secs(2),
    )
    .await;
    h.api("pane.copy_mode", serde_json::json!({}))
        .await
        .unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains(" copy "),
        Duration::from_secs(2),
    )
    .await;
    for _ in 0..10 {
        h.key(h.client.clone(), "k").await;
    }
    h.key(h.client.clone(), "g").await;
    let f = h.frame(h.client.clone()).await;
    assert!(
        f.contains(" copy ┐"),
        "no scrollback to walk, so the flag counts nothing:\n{f}"
    );
    assert_eq!(
        row(&f, 2),
        "|│edit me                                                                       │|",
        "{f}"
    );
    h.key(h.client.clone(), "0").await;
    h.key(h.client.clone(), "v").await;
    h.key(h.client.clone(), "$").await;
    h.key(h.client.clone(), "Enter").await;
    h.wait_for(
        h.client.clone(),
        |f| !f.contains(" copy "),
        Duration::from_secs(2),
    )
    .await;
    assert_eq!(
        h.clipboard(h.client.clone()).await,
        vec!["edit me".to_string()]
    );
}

/// The rest of the movement keys on the same 7-row pane: `C-u` and `C-d` are half of it,
/// PageUp and PageDown the whole of it, and the arrows are `j` and `k`. Each walks the cursor
/// across the screen first and scrolls only what is left over at the edge, so a page from the
/// middle of the screen does not scroll a page.
#[tokio::test]
async fn the_half_screen_and_page_keys_walk_the_scrollback() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    pane_with_lines(&mut h, 20).await;
    h.api("pane.copy_mode", serde_json::json!({}))
        .await
        .unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains(" copy "),
        Duration::from_secs(2),
    )
    .await;
    // The cursor starts on row 6 of 7. Half a screen is 3: the first two presses walk it to
    // row 0 without scrolling, the third has nothing left to walk and scrolls 3.
    for _ in 0..3 {
        h.key(h.client.clone(), "C-u").await;
    }
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("copy 3/14"),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(
        row(&f, 2),
        "|│line 11                                                                       │|",
        "{f}"
    );
    // The cursor is already on the top row, so the arrow scrolls one more line.
    h.key(h.client.clone(), "Up").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("copy 4/14"),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(
        row(&f, 2),
        "|│line 10                                                                       │|",
        "{f}"
    );
    // A page down from the top row walks the cursor to the bottom row and scrolls the one
    // line that is left over; the next page has the whole screen to give back.
    h.key(h.client.clone(), "PageDown").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("copy 3/14"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        f.contains("cursor row=8 col=3"),
        "the cursor walked to the bottom row:\n{f}"
    );
    h.key(h.client.clone(), "PageDown").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains(" copy ┐"),
            Duration::from_secs(2),
        )
        .await;
    assert!(f.contains("│$ "), "back on the live screen:\n{f}");
    // Up from the bottom row: one line of scroll, then a whole screen of it.
    h.key(h.client.clone(), "PageUp").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("copy 1/14"),
        Duration::from_secs(2),
    )
    .await;
    h.key(h.client.clone(), "PageUp").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("copy 8/14"),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(
        row(&f, 2),
        "|│line 06                                                                       │|",
        "{f}"
    );
    // Half a screen down from the top row stays on the screen: the cursor moves, the view
    // does not.
    h.key(h.client.clone(), "C-d").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("cursor row=5 col=3"),
            Duration::from_secs(2),
        )
        .await;
    assert!(f.contains("copy 8/14"), "{f}");
    // `q` leaves without copying, like Esc.
    h.key(h.client.clone(), "q").await;
    h.wait_for(
        h.client.clone(),
        |f| !f.contains(" copy "),
        Duration::from_secs(2),
    )
    .await;
    assert!(h.clipboard(h.client.clone()).await.is_empty());
}

/// Copy mode owns the keys while it is open, on a pane whose child exited like on any other
/// (ruled 2026-09-07).
///
/// The exited-pane branch ran first, so `esc leave` was dead, there was no way out of the mode
/// at all, and Enter - labelled `copy` on the bar at that very moment - closed the pane instead.
/// A key must never do something its visible label does not say, least of all something
/// destructive (principles 3 and 10). Enter closes an exited pane only when copy mode is not
/// active, which is the state the bar says it is in.
#[tokio::test]
async fn copy_mode_keeps_the_keys_on_a_pane_whose_child_exited() {
    let mut cfg = Config::default();
    cfg.terminal.remain_on_exit = true;
    let mut h = Harness::start(cfg, 80, 12).await;
    let pane = pane_with_lines(&mut h, 20).await;
    h.api("pane.copy_mode", serde_json::json!({}))
        .await
        .unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains(" copy "),
        Duration::from_secs(2),
    )
    .await;
    h.exit_pane(pane.clone(), Some(3)).await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("exited 3"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        row(&f, 0).ends_with("v select · ⏎ copy · esc leave ● |"),
        "the bar still offers the copy mode keys:\n{f}"
    );
    // Esc means what it says: it leaves the mode and the pane is untouched.
    h.key(h.client.clone(), "Esc").await;
    h.wait_for(
        h.client.clone(),
        |f| !f.contains(" copy "),
        Duration::from_secs(2),
    )
    .await;
    assert!(
        h.model().pane(&pane).is_some(),
        "leaving copy mode is not closing the pane"
    );
    // And so does Enter: an exited pane's scrollback is exactly what a reader wants to copy.
    h.api("pane.copy_mode", serde_json::json!({}))
        .await
        .unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains(" copy "),
        Duration::from_secs(2),
    )
    .await;
    h.key(h.client.clone(), "k").await;
    h.key(h.client.clone(), "0").await;
    h.key(h.client.clone(), "v").await;
    h.key(h.client.clone(), "$").await;
    h.key(h.client.clone(), "Enter").await;
    h.wait_for(
        h.client.clone(),
        |f| !f.contains(" copy "),
        Duration::from_secs(2),
    )
    .await;
    assert_eq!(
        h.clipboard(h.client.clone()).await,
        vec!["line 19".to_string()]
    );
    assert!(
        h.model().pane(&pane).is_some(),
        "copying from an exited pane does not close it"
    );
    // With copy mode closed, Enter is the exited pane's own key again and the pane goes.
    h.key(h.client.clone(), "Enter").await;
    h.wait_for(
        h.client.clone(),
        |f| !f.contains("exited 3"),
        Duration::from_secs(2),
    )
    .await;
    assert!(
        h.model().pane(&pane).is_none(),
        "Enter closes an exited pane that is not in copy mode"
    );
}

/// The chord indicator outranks the copy mode keys, because it answers the key just pressed:
/// the leader inside copy mode started a chord the bar never showed, so the next key carried a
/// meaning the screen had not admitted to (principle 8). The chord ends on that next key, and
/// the copy mode keys come back.
#[tokio::test]
async fn the_leader_inside_copy_mode_shows_the_chord_rather_than_the_copy_keys() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    pane_with_lines(&mut h, 3).await;
    h.api("pane.copy_mode", serde_json::json!({}))
        .await
        .unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains(" copy "),
        Duration::from_secs(2),
    )
    .await;
    h.key(h.client.clone(), "C-a").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("C-a  ? keys"),
            Duration::from_secs(2),
        )
        .await;
    assert!(row(&f, 0).ends_with("C-a  ? keys ● |"), "{f}");
    assert!(f.contains(" copy "), "copy mode is still open:\n{f}");
    // An unbound second key ends the chord and changes nothing else.
    h.key(h.client.clone(), "Tab").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("esc leave"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        row(&f, 0).ends_with("v select · ⏎ copy · esc leave ● |"),
        "{f}"
    );
}

/// The wheel hits the pane the reader sees under the pointer, and the sidebar shifts every
/// pane box 39 columns right. Hit testing and drawing have to read the same rectangle: if
/// hit testing kept the sidebar-less one, a scroll over the sidebar would scroll a pane and
/// a scroll over the left pane would land on whatever sits 39 columns further left.
#[tokio::test]
async fn wheel_scroll_finds_the_pane_under_the_pointer_beside_an_open_sidebar() {
    let mut h = Harness::start(Config::default(), 120, 14).await;
    h.api(
        "sidebar.show",
        serde_json::json!({"client": h.client.as_str()}),
    )
    .await
    .unwrap();
    let left = pane_with_lines(&mut h, 20).await;
    h.api("pane.split", serde_json::json!({"dir": "right"}))
        .await
        .unwrap();
    h.frame(h.client.clone()).await;

    // Column 5 is inside the sidebar, which belongs to no pane.
    h.scroll(h.client.clone(), 5, 5, 3).await;
    h.frame(h.client.clone()).await;
    assert!(
        !h.model().pane(&left).unwrap().copy_mode,
        "a scroll over the sidebar scrolls nothing"
    );

    // Column 45 is inside the left pane's box, which starts at column 39.
    h.scroll(h.client.clone(), 45, 5, 3).await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains(" copy "),
            Duration::from_secs(2),
        )
        .await;
    assert!(h.model().pane(&left).unwrap().copy_mode, "{f}");
    assert_eq!(h.focused_pane(h.client.clone()), left);
}
