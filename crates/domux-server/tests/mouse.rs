//! The pointer: the wheel to a program that asked for it, and the buttons to a selection
//! (decision 0014).

use domux_core::config::Config;
use domux_server::testing::{row, Harness};
use domux_term::MouseAction;
use std::time::Duration;

/// A pane holding `line 00` up to `line NN` and a prompt. On an 80x10 screen the box's inner
/// area starts at column 1, row 2 and is 7 rows tall, so `line 14` is the top visible row.
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

/// The gesture the migration needs: press, drag, let go, and the text is on the clipboard.
/// The selection is drawn while the button is held, so the reader sees what they are taking.
#[tokio::test]
async fn a_drag_selects_what_it_covers_and_copies_it_when_the_button_is_let_go() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    let pane = pane_with_lines(&mut h, 20).await;
    // `line 14` starts at column 1 of row 2; `line 15` ends at column 7 of row 3.
    h.mouse(h.client.clone(), MouseAction::Press, 1, 2, 1).await;
    h.mouse(h.client.clone(), MouseAction::Drag, 7, 3, 1).await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains(" copy ") && f.contains("inverse"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        f.contains("line 14"),
        "the pane is still showing its live rows:\n{f}"
    );
    assert!(h.model().pane(&pane).unwrap().copy_mode);

    h.mouse(h.client.clone(), MouseAction::Release, 7, 3, 1)
        .await;
    h.wait_for(
        h.client.clone(),
        |f| !f.contains(" copy "),
        Duration::from_secs(2),
    )
    .await;
    assert_eq!(
        h.clipboard(h.client.clone()).await,
        vec!["line 14\nline 15".to_string()],
        "the release copied the selection and left copy mode"
    );
    assert!(!h.model().pane(&pane).unwrap().copy_mode);
}

/// A press that never moves is a click. It focuses the pane it landed in and leaves nothing
/// behind: no copy mode the reader did not ask for, and no clipboard write.
#[tokio::test]
async fn a_click_focuses_the_pane_under_it_and_copies_nothing() {
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
    assert_eq!(h.focused_pane(h.client.clone()), right);

    h.mouse(h.client.clone(), MouseAction::Press, 5, 5, 1).await;
    h.mouse(h.client.clone(), MouseAction::Release, 5, 5, 1)
        .await;
    h.wait_for(h.client.clone(), |_| true, Duration::from_secs(2))
        .await;
    assert_eq!(
        h.focused_pane(h.client.clone()),
        left,
        "the click moved the keys to the pane under it"
    );
    assert!(!h.model().pane(&left).unwrap().copy_mode);
    assert!(h.clipboard(h.client.clone()).await.is_empty());
}

/// A double click copies the word under the pointer, by Ghostty's own idea of a word so that
/// domux and the terminal it runs in agree.
#[tokio::test]
async fn a_double_click_copies_the_word_under_it() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    pane_with_lines(&mut h, 20).await;
    // Row 2 reads `line 14`: column 1 is its `l` and column 6 is the `1` of `14`.
    h.mouse(h.client.clone(), MouseAction::Press, 2, 2, 1).await;
    h.mouse(h.client.clone(), MouseAction::Press, 2, 2, 2).await;
    h.wait_for(h.client.clone(), |_| true, Duration::from_secs(2))
        .await;
    assert_eq!(
        h.clipboard(h.client.clone())
            .await
            .last()
            .map(String::as_str),
        Some("line")
    );

    h.mouse(h.client.clone(), MouseAction::Press, 7, 2, 1).await;
    h.mouse(h.client.clone(), MouseAction::Press, 7, 2, 2).await;
    h.wait_for(h.client.clone(), |_| true, Duration::from_secs(2))
        .await;
    assert_eq!(
        h.clipboard(h.client.clone())
            .await
            .last()
            .map(String::as_str),
        Some("14"),
        "the word under the second double click, not the first"
    );
}

/// A triple click copies the line. A line the screen wrapped is one line, so the copy is what
/// was written rather than the row the pointer happened to be on.
#[tokio::test]
async fn a_triple_click_copies_a_wrapped_line_whole() {
    let mut h = Harness::start(Config::default(), 40, 10).await;
    let pane = h.focused_pane(h.client.clone());
    // The box's inner area is 38 columns wide, so 50 characters take two rows.
    let long = "x".repeat(50);
    h.feed_pane(pane.clone(), format!("{long}\r\ndone").as_bytes())
        .await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("done"),
        Duration::from_secs(2),
    )
    .await;
    // Row 3 is the second row of the wrapped line.
    h.mouse(h.client.clone(), MouseAction::Press, 3, 3, 1).await;
    h.mouse(h.client.clone(), MouseAction::Press, 3, 3, 2).await;
    h.mouse(h.client.clone(), MouseAction::Press, 3, 3, 3).await;
    h.wait_for(h.client.clone(), |_| true, Duration::from_secs(2))
        .await;
    // The three presses are one sequence, so the double click copied a word on the way past.
    // The triple click's answer is the one this test is about, and it is the last.
    assert_eq!(
        h.clipboard(h.client.clone()).await.last(),
        Some(&long),
        "the whole line, not the row the pointer was on"
    );
}

/// The report the wheel becomes for a program that asked for the mouse, and the mode that does
/// not open over it. This is the pane a full screen program owns: it scrolls its own view, and
/// copy mode has no history to walk there anyway.
#[tokio::test]
async fn the_wheel_reaches_a_program_that_asked_for_the_mouse_and_copy_mode_stays_shut() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    let pane = pane_with_lines(&mut h, 20).await;
    h.feed_pane(pane.clone(), b"\x1b[?1000h\x1b[?1006h").await;
    h.frame(h.client.clone()).await;
    let before = h.pane_input(&pane).len();

    h.scroll(h.client.clone(), 5, 5, 3).await;
    h.frame(h.client.clone()).await;
    let sent = h.pane_input(&pane);
    assert_eq!(
        String::from_utf8_lossy(&sent[before..]),
        "\x1b[<64;5;4M",
        "wheel up at the pane's own cell, in the format the program asked for"
    );
    assert!(
        !h.model().pane(&pane).unwrap().copy_mode,
        "the program owns the gesture, so copy mode does not open"
    );

    h.scroll(h.client.clone(), 5, 5, -3).await;
    h.frame(h.client.clone()).await;
    let sent = h.pane_input(&pane);
    assert!(
        String::from_utf8_lossy(&sent).ends_with("\x1b[<65;5;4M"),
        "wheel down is the other report"
    );
}

/// A pane whose program asked for nothing keeps the old gesture: the wheel opens copy mode and
/// moves its viewport.
#[tokio::test]
async fn the_wheel_still_opens_copy_mode_over_a_pane_whose_program_wants_no_mouse() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    let pane = pane_with_lines(&mut h, 20).await;
    let before = h.pane_input(&pane).len();
    h.scroll(h.client.clone(), 5, 5, 3).await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("copy 3/14"),
        Duration::from_secs(2),
    )
    .await;
    assert_eq!(
        h.pane_input(&pane).len(),
        before,
        "nothing was written to a program that did not ask for the mouse"
    );
}

/// A click into a pane while the keys are in a box takes them back. Without this the pointer
/// could move the pane the tab draws as focused while the sidebar kept the keys, and the frame
/// would mark one thing while another read the keys (principle 2).
#[tokio::test]
async fn a_click_in_a_pane_takes_the_keys_back_from_a_box() {
    let mut h = Harness::start(Config::default(), 80, 20).await;
    pane_with_lines(&mut h, 5).await;
    h.api("sidebar.show", serde_json::json!({})).await.unwrap();
    h.api(
        "focus.region",
        serde_json::json!({"region": "sidebar_projects"}),
    )
    .await
    .unwrap();
    h.frame(h.client.clone()).await;
    assert!(
        matches!(
            h.model().client(&h.client).unwrap().focus,
            domux_core::model::Focus::Region(_)
        ),
        "the box has the keys"
    );

    // Column 45 is inside the pane box, right of the sidebar.
    h.mouse(h.client.clone(), MouseAction::Press, 45, 5, 1)
        .await;
    h.mouse(h.client.clone(), MouseAction::Release, 45, 5, 1)
        .await;
    h.wait_for(h.client.clone(), |_| true, Duration::from_secs(2))
        .await;
    assert!(
        matches!(
            h.model().client(&h.client).unwrap().focus,
            domux_core::model::Focus::Pane(_)
        ),
        "the click took the keys back to the pane"
    );
}

/// A press over the chrome, and one over an overlay, are not a pane's. The sidebar's own rows
/// answer a click in Task 4 of this change; until then nothing happens rather than something
/// happening to the pane behind them.
#[tokio::test]
async fn a_press_over_an_overlay_does_not_reach_the_pane_under_it() {
    let mut h = Harness::start(Config::default(), 80, 20).await;
    let pane = pane_with_lines(&mut h, 20).await;
    h.api("switcher.open", serde_json::json!({})).await.unwrap();
    h.frame(h.client.clone()).await;
    h.mouse(h.client.clone(), MouseAction::Press, 40, 10, 1)
        .await;
    h.mouse(h.client.clone(), MouseAction::Drag, 45, 11, 1)
        .await;
    h.mouse(h.client.clone(), MouseAction::Release, 45, 11, 1)
        .await;
    h.frame(h.client.clone()).await;
    assert!(!h.model().pane(&pane).unwrap().copy_mode);
    assert!(h.clipboard(h.client.clone()).await.is_empty());
}

/// The screen column of the first `needle` in row `n` of a frame. The row is printed as
/// `|cells|`, so the column is the position inside those pipes, counted in characters because
/// the bar's own label carries a multi-byte one.
fn column_of(frame: &str, n: usize, needle: char) -> u16 {
    let printed = row(frame, n);
    let cells: Vec<char> = printed.chars().collect();
    let at = cells
        .iter()
        .skip(1)
        .position(|c| *c == needle)
        .unwrap_or_else(|| panic!("row {n} has no {needle:?}:\n{frame}"));
    at as u16
}

/// A click on a tab's cell selects that tab, which is what `tab.select` and the number keys do.
#[tokio::test]
async fn a_click_on_a_tab_selects_it() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    h.api("tab.create", serde_json::json!({})).await.unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains(" 2 "),
            Duration::from_secs(2),
        )
        .await;
    let second = h.current_tab(h.client.clone());
    let at = column_of(&f, 0, '1');
    h.mouse(h.client.clone(), MouseAction::Press, at, 0, 1)
        .await;
    h.mouse(h.client.clone(), MouseAction::Release, at, 0, 1)
        .await;
    h.wait_for(h.client.clone(), |_| true, Duration::from_secs(2))
        .await;
    assert_ne!(
        h.current_tab(h.client.clone()),
        second,
        "the click moved to the first tab"
    );
}

/// A click on the `+` makes a tab, which is what `tab.create` and `leader t` do.
#[tokio::test]
async fn a_click_on_the_plus_makes_a_tab() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    let f = h.frame(h.client.clone()).await;
    assert!(!f.contains(" 2 "), "one tab to start with:\n{f}");
    let at = column_of(&f, 0, '+');
    h.mouse(h.client.clone(), MouseAction::Press, at, 0, 1)
        .await;
    h.mouse(h.client.clone(), MouseAction::Release, at, 0, 1)
        .await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains(" 2 "),
            Duration::from_secs(2),
        )
        .await;
    assert!(f.contains(" 2 "), "the click made a second tab:\n{f}");
}

/// A click on a separator in the tab row acts on nothing. The cells between the tabs are a rule,
/// not a target, and a click that fell on one must not select the tab beside it.
#[tokio::test]
async fn a_click_on_the_tab_rows_rule_does_nothing() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    h.api("tab.create", serde_json::json!({})).await.unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains(" 2 "),
            Duration::from_secs(2),
        )
        .await;
    let before = h.current_tab(h.client.clone());
    let at = column_of(&f, 0, '│');
    h.mouse(h.client.clone(), MouseAction::Press, at, 0, 1)
        .await;
    h.mouse(h.client.clone(), MouseAction::Release, at, 0, 1)
        .await;
    h.wait_for(h.client.clone(), |_| true, Duration::from_secs(2))
        .await;
    assert_eq!(h.current_tab(h.client.clone()), before);
    assert!(
        !h.frame(h.client.clone()).await.contains(" 3 "),
        "and it made no tab"
    );
}

/// A click on a workspace's row in the sidebar switches to that workspace, which is what Enter
/// on the row does. The row is found on the screen rather than counted: the box draws a header
/// and a blank between projects, and the click has to land on the row the reader sees.
#[tokio::test]
async fn a_click_on_a_sidebar_row_switches_to_that_workspace() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let (_root, w1, _w2) = h.git_project_with_two_slots().await;
    h.api("sidebar.show", serde_json::json!({})).await.unwrap();
    let client = h.client.clone();
    assert_ne!(
        h.model().client(&client).unwrap().workspace,
        w1,
        "the client starts somewhere else"
    );
    // The two slots draw as `workspace-1` and `workspace-2`, under their project's header.
    let f = h
        .wait_for(
            client.clone(),
            |f| f.contains("Navigator") && f.contains("workspace-1"),
            Duration::from_secs(2),
        )
        .await;
    let at = f
        .lines()
        .filter(|l| l.starts_with('|'))
        .position(|l| l.contains("workspace-1"))
        .expect("the sidebar draws the first slot") as u16;

    h.mouse(client.clone(), MouseAction::Press, 5, at, 1).await;
    h.mouse(client.clone(), MouseAction::Release, 5, at, 1)
        .await;
    h.wait_for(client.clone(), |_| true, Duration::from_secs(2))
        .await;
    assert_eq!(
        h.model().client(&client).unwrap().workspace,
        w1,
        "the click switched to the workspace on that row"
    );
}

/// A click on the Projects box's own rows that name no workspace - the project header, the blank
/// between two projects, the border - acts on nothing.
#[tokio::test]
async fn a_click_on_a_sidebar_header_does_nothing() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let (_root, _w1, _w2) = h.git_project_with_two_slots().await;
    h.api("sidebar.show", serde_json::json!({})).await.unwrap();
    let client = h.client.clone();
    h.wait_for(
        client.clone(),
        |f| f.contains("Navigator"),
        Duration::from_secs(2),
    )
    .await;
    let before = h.model().client(&client).unwrap().workspace.clone();
    // Row 0 is the box's own top border.
    h.mouse(client.clone(), MouseAction::Press, 5, 0, 1).await;
    h.mouse(client.clone(), MouseAction::Release, 5, 0, 1).await;
    h.wait_for(client.clone(), |_| true, Duration::from_secs(2))
        .await;
    assert_eq!(h.model().client(&client).unwrap().workspace, before);
}

// ------------------------------------------------------------------ links (MUX-13)

/// Waits for the opener to have been handed something, and answers with everything it has.
async fn opened(h: &Harness) -> Vec<String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        let seen = h.opener.opened();
        if !seen.is_empty() {
            return seen;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "nothing was opened within two seconds"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// The pill this client is showing, which is where the reader is told what was opened. It is
/// read from the view rather than from the frame because a pill only draws in the sidebar's
/// hint row, and these tests run with the sidebar hidden.
async fn pill(h: &mut Harness) -> Option<String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        let said = h
            .model()
            .client(&h.client)
            .and_then(|v| v.pill.clone())
            .map(|p| p.text);
        if said.is_some() {
            return said;
        }
        if tokio::time::Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Presses and releases on one cell, which is a click.
async fn click(h: &mut Harness, column: u16, row: u16) {
    h.mouse(h.client.clone(), MouseAction::Press, column, row, 1)
        .await;
    h.mouse(h.client.clone(), MouseAction::Release, column, row, 1)
        .await;
}

/// MUX-13: a click on an OSC 8 hyperlink opens what the program marked, which is not the text
/// the reader can see. That difference is the whole assertion: a click that opened the word
/// under it would open `link`, and a click that opened nothing would leave the opener empty.
#[tokio::test]
async fn a_click_on_a_hyperlink_opens_the_target_the_program_marked() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    let pane = h.focused_pane(h.client.clone());
    h.feed_pane(
        pane,
        b"see \x1b]8;;https://example.com/artifact\x1b\\link\x1b]8;;\x1b\\ here",
    )
    .await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("see link here"),
        Duration::from_secs(2),
    )
    .await;

    // Column 5 of the screen is column 4 of the pane's grid, which is the `l` of `link`.
    click(&mut h, 5, 2).await;

    assert_eq!(opened(&h).await, vec!["https://example.com/artifact"]);
    assert_eq!(
        pill(&mut h).await.as_deref(),
        Some("Opened https://example.com/artifact"),
        "the reader is told, because the answer is on another application's window"
    );
}

/// A bare URL in the text opens too, which is the other half of the screenshot on MUX-13:
/// Claude Code prints the address as plain text beside the marked one.
#[tokio::test]
async fn a_click_on_a_url_in_plain_text_opens_it() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    let pane = h.focused_pane(h.client.clone());
    h.feed_pane(pane, b"Published https://claude.ai/code/artifact/8fc.")
        .await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("Published https"),
        Duration::from_secs(2),
    )
    .await;

    click(&mut h, 20, 2).await;

    assert_eq!(
        opened(&h).await,
        vec!["https://claude.ai/code/artifact/8fc"],
        "and the full stop the sentence ended with is not part of it"
    );
}

/// A path that is really on disk opens, and a word that is not a link opens nothing and says
/// nothing: a click with no link under it is how the reader moves the focus between panes.
#[tokio::test]
async fn a_click_opens_a_path_that_exists_and_ignores_a_word_that_is_not_a_link() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("notes.md");
    std::fs::write(&file, "x").unwrap();
    let pane = h.focused_pane(h.client.clone());
    h.feed_pane(pane, format!("wrote {}", file.display()).as_bytes())
        .await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("notes.md"),
        Duration::from_secs(2),
    )
    .await;

    // `wrote` is columns 1 to 5 of the screen, and the path starts at column 7.
    click(&mut h, 2, 2).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        h.opener.opened().is_empty(),
        "a word is not a link, and nothing was opened"
    );
    assert_eq!(
        h.model().client(&h.client).and_then(|v| v.pill.clone()),
        None,
        "and nothing was said about it either"
    );

    click(&mut h, 8, 2).await;
    assert_eq!(opened(&h).await, vec![file.display().to_string()]);
    assert_eq!(
        pill(&mut h).await.as_deref(),
        Some("Opened notes.md"),
        "a path says its file name, because the row is narrow"
    );
}

/// A drag over a link selects and copies it, and opens nothing. The two gestures share a
/// button, so this is the line between them: a press that moved is a selection.
#[tokio::test]
async fn a_drag_over_a_link_copies_it_and_opens_nothing() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    let pane = h.focused_pane(h.client.clone());
    h.feed_pane(pane, b"https://example.com/x").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("https://example.com/x"),
        Duration::from_secs(2),
    )
    .await;

    h.mouse(h.client.clone(), MouseAction::Press, 1, 2, 1).await;
    h.mouse(h.client.clone(), MouseAction::Drag, 21, 2, 1).await;
    h.mouse(h.client.clone(), MouseAction::Release, 21, 2, 1)
        .await;
    h.wait_for(
        h.client.clone(),
        |f| !f.contains(" copy "),
        Duration::from_secs(2),
    )
    .await;

    assert_eq!(
        h.clipboard(h.client.clone()).await,
        vec!["https://example.com/x".to_string()]
    );
    assert!(
        h.opener.opened().is_empty(),
        "the drag selected it rather than opening it"
    );
}

/// An opener that will not open says so, in the words of the link rather than of the target:
/// an OSC 8 link says one thing and points at another, and a reader told only the target
/// cannot tell which of the things on their screen failed.
#[tokio::test]
async fn a_link_the_desktop_will_not_open_says_what_could_not_be_opened_and_why() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    h.opener.refuse_with("no application knows this");
    let pane = h.focused_pane(h.client.clone());
    h.feed_pane(pane, b"https://example.com/x").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("https://example.com/x"),
        Duration::from_secs(2),
    )
    .await;

    click(&mut h, 2, 2).await;

    assert_eq!(opened(&h).await, vec!["https://example.com/x"]);
    assert_eq!(
        pill(&mut h).await.as_deref(),
        Some("could not open https://example.com/x: no application knows this")
    );
}

/// The tab row starts after the location label, and the pointer measures from the same place.
///
/// `top_bar::bar_tab_hit` finds where the row begins by measuring what is drawn before it. A hit
/// test that measured nothing, or measured a piece the bar no longer draws, would put every
/// tab's cells away from where the reader sees them and the `+` at the end of the row would
/// answer nothing at all. The label is the only piece in front of the row now that MUX-23 has
/// taken the agent count off the bar, and a waiting agent is here to prove it: the bar this
/// clicks on is one that would have carried a count.
#[tokio::test]
async fn a_click_on_the_plus_lands_on_it_after_the_location_label() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    let pane = h.focused_pane(h.client.clone());
    h.report(
        pane,
        domux_core::model::agent::AgentKind::Claude,
        r#"{"hook_event_name":"Notification","session_id":"c1","message":"needs permission"}"#,
    )
    .await;
    let f = h.frame(h.client.clone()).await;
    assert!(
        row(&f, 0).starts_with("| proj › main"),
        "the label opens the bar, with nothing in front of it:\n{f}"
    );
    assert!(!f.contains(" 2 "), "one tab to start with:\n{f}");
    let at = column_of(&f, 0, '+');
    h.mouse(h.client.clone(), MouseAction::Press, at, 0, 1)
        .await;
    h.mouse(h.client.clone(), MouseAction::Release, at, 0, 1)
        .await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains(" 2 "),
            Duration::from_secs(2),
        )
        .await;
    assert!(f.contains(" 2 "), "the click made a second tab:\n{f}");
}
