//! Naming a tab in its own cell, the chord indicator, and the keys help overlay, plus the
//! three top bar defects earlier reviews deferred to Task 18: the prompt drawn on the cell of
//! the tab it names, the tab row eliding around a current tab that must stay visible, and a
//! right end that elides instead of running off the screen edge.

use domux_core::config::Config;
use domux_server::testing::{row, Harness};
use std::time::Duration;

/// Every cell of `frame` whose background is the accent, as `(row, column)`.
///
/// Reads the frame's own style dump, whose lines are `r{row} c{from}-{to} [attrs] fg=# bg=# ul=#`,
/// so what it counts is what a terminal would paint. Accent as a *foreground* - a focused box's
/// border and title - is a different treatment and is deliberately not counted.
///
/// The background is matched where it sits in the line rather than at the end of it. `ul=` is
/// dumped after `bg=`, so an `ends_with` here stopped seeing a cell the moment anything gave it
/// an underline colour - and this guard protects a ruling, so it must not be defeatable by a
/// later change nobody connects to it. Every dumped style is preceded by a space, so the leading
/// space keeps `ul=#cba6f7` from matching as a fill.
fn accent_filled_cells(frame: &str) -> Vec<(u16, u16)> {
    let mut out = Vec::new();
    for line in frame.lines().filter(|l| l.contains(" bg=#cba6f7")) {
        let mut parts = line.split(' ');
        let Some(Ok(row)) = parts
            .next()
            .and_then(|r| r.strip_prefix('r'))
            .map(str::parse)
        else {
            continue;
        };
        let Some((from, to)) = parts
            .next()
            .and_then(|c| c.strip_prefix('c'))
            .and_then(|c| c.split_once('-'))
        else {
            continue;
        };
        let (Ok(from), Ok(to)) = (from.parse::<u16>(), to.parse::<u16>()) else {
            continue;
        };
        out.extend((from..=to).map(|x| (row, x)));
    }
    out.sort_unstable();
    out
}

/// Principle 2 as a rule that can be checked rather than argued (ruled 2026-09-07): the accent
/// fill means "your input goes here", so at most one run of cells may carry it, and it is the
/// run that owns the keys. Panics unless the accent-filled cells are exactly one contiguous run
/// on one row, and returns it as `(row, first, last)`.
fn one_accent_run(frame: &str) -> (u16, u16, u16) {
    let cells = accent_filled_cells(frame);
    assert!(!cells.is_empty(), "nothing is accent-filled:\n{frame}");
    let (row, first) = cells[0];
    let (last_row, last) = cells[cells.len() - 1];
    assert_eq!(row, last_row, "the accent fill spans two rows:\n{frame}");
    assert_eq!(
        cells.len(),
        (last - first + 1) as usize,
        "the accent fill is more than one run - two marks meaning two different things:\n{frame}"
    );
    (row, first, last)
}

#[tokio::test]
async fn leader_comma_opens_the_prompt_in_the_tab_cell_and_the_clock_gives_way() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), ",").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("Name tab 1 ›"),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(
        row(&f, 0),
        "| proj › main  Name tab 1 ›   │ + │           ⏎ save · esc cancel · empty clears |",
        "{f}"
    );
    assert!(
        f.contains("r0 c13-26 dim fg=#1e1e2e bg=#cba6f7"),
        "the label at reduced weight in the accent cell:\n{f}"
    );
    assert!(
        f.contains("r0 c27-27 inverse fg=#1e1e2e bg=#cba6f7"),
        "the block caret:\n{f}"
    );
    assert!(f.contains("fg=#89b4fa"), "keys in blue:\n{f}");
    h.type_text(h.client.clone(), "tests").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("› tests"),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(
        row(&f, 0),
        "| proj › main  Name tab 1 › tests  │ + │      ⏎ save · esc cancel · empty clears |",
        "{f}"
    );
    let pane = h.focused_pane(h.client.clone());
    assert!(h.pane_input(&pane).is_empty(), "the prompt owns the keys");
    h.key(h.client.clone(), "Enter").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains(" 1 tests "),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(
        row(&f, 0),
        "| proj › main  1 tests │ + │                                   14:32   Fri 4 Sep |",
        "{f}"
    );
    assert_eq!(
        h.model().client(&h.client).unwrap().focus,
        domux_core::model::Focus::Pane(pane),
        "focus returned to the pane"
    );
}

#[tokio::test]
async fn esc_cancels_and_an_empty_name_clears_and_leader_r_clears_without_a_prompt() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    h.api("tab.rename", serde_json::json!({"name": "old"}))
        .await
        .unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains(" 1 old "),
        Duration::from_secs(2),
    )
    .await;
    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), ",").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("Name tab 1 ›"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        f.contains("› old"),
        "the prompt starts with the current name:\n{f}"
    );
    h.key(h.client.clone(), "Esc").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| !f.contains("Name tab"),
            Duration::from_secs(2),
        )
        .await;
    assert!(f.contains(" 1 old "), "esc changed nothing:\n{f}");
    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), ",").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("Name tab 1 ›"),
        Duration::from_secs(2),
    )
    .await;
    for _ in 0..3 {
        h.key(h.client.clone(), "Backspace").await;
    }
    h.key(h.client.clone(), "Enter").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| !f.contains("Name tab"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        f.contains("main  1 │"),
        "an empty name gave the tab its number back:\n{f}"
    );
    h.api("tab.rename", serde_json::json!({"name": "again"}))
        .await
        .unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains("again"),
        Duration::from_secs(2),
    )
    .await;
    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), "R").await;
    h.wait_for(
        h.client.clone(),
        |f| !f.contains("again"),
        Duration::from_secs(2),
    )
    .await;
}

#[tokio::test]
async fn the_chord_indicator_shows_the_leader_and_the_help_key_in_the_clocks_place() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    h.key(h.client.clone(), "C-a").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("C-a"),
            Duration::from_secs(2),
        )
        .await;
    assert!(row(&f, 0).ends_with("C-a  ? keys |"), "{f}");
    assert!(!f.contains("14:32"), "the clock gave way:\n{f}");
    h.key(h.client.clone(), "Esc").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("14:32"),
        Duration::from_secs(2),
    )
    .await;
}

#[tokio::test]
async fn help_lists_the_configured_bindings_and_esc_closes_it() {
    let mut cfg = Config::default();
    cfg.keys.leader = "C-b".into();
    cfg.keys
        .bindings
        .insert("g".into(), "pane.split right".into());
    cfg.keys.bindings.insert("|".into(), "".into());
    // Tall enough that the full list fits with no truncation: M2 added four leader
    // bindings (switcher.open, sidebar.toggle, workspace.rename, workspace.clear_name)
    // and M3 added agents.open and the box keys' Tab, so this grew from the 80x30 M1
    // needed. The 80x24 case is the next test, which is where truncation is the behaviour
    // under test.
    let mut h = Harness::start(cfg, 80, 48).await;
    h.key(h.client.clone(), "C-b").await;
    h.key(h.client.clone(), "?").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("┌ Keys"),
            Duration::from_secs(2),
        )
        .await;
    assert!(f.contains("C-b g      pane.split right"), "{f}");
    assert!(
        f.contains("C-b 1-9    tab.select <n>"),
        "the nine digits collapse into one row:\n{f}"
    );
    assert!(!f.contains("C-b |"), "the unbound default is gone:\n{f}");
    assert!(f.contains("C-h        focus.left"), "{f}");
    assert!(
        f.contains("nvim, vim, fzf keep C-h, C-j, C-k, C-l, C-\\"),
        "{f}"
    );
    assert!(f.contains("esc close"), "{f}");
    assert!(
        !f.contains("more, see domux.toml"),
        "the screen is tall enough for the whole list, which is what this case is about:\n{f}"
    );
    let pane = h.focused_pane(h.client.clone());
    h.key(h.client.clone(), "j").await;
    h.frame(h.client.clone()).await;
    assert!(h.pane_input(&pane).is_empty(), "the overlay owns the keys");
    h.key(h.client.clone(), "Esc").await;
    h.wait_for(
        h.client.clone(),
        |f| !f.contains("┌ Keys"),
        Duration::from_secs(2),
    )
    .await;
    assert_eq!(
        h.model().client(&h.client).unwrap().focus,
        domux_core::model::Focus::Pane(pane)
    );
}

#[tokio::test(flavor = "current_thread")]
async fn help_keeps_the_footer_when_the_screen_is_too_short_for_every_binding() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), "?").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("┌ Keys"),
            Duration::from_secs(2),
        )
        .await;
    // The list does not fit, so it says how much it dropped and still shows the way out.
    // An overlay that swallowed `esc close` would be a dead end (principle 9).
    assert!(
        f.contains("more, see domux.toml"),
        "the truncation is named:\n{f}"
    );
    assert!(
        f.contains("esc close"),
        "the footer survives truncation:\n{f}"
    );
    h.key(h.client.clone(), "Esc").await;
    h.wait_for(
        h.client.clone(),
        |f| !f.contains("┌ Keys"),
        Duration::from_secs(2),
    )
    .await;
}

/// A defect the Task 16 review found and deferred here: `tab.rename` with no name on a tab
/// that is not the current one drew the prompt over the *current* tab's cell and labelled it
/// with the *current* tab's number, while the prompt held the other tab's id - wrong cell,
/// wrong label, right target, so the reader could not tell what was about to be renamed.
///
/// The current-tab case hides the bug, so the case under test is the non-current one: the
/// client is on tab 2 and the prompt names tab 1.
#[tokio::test]
async fn the_prompt_draws_in_the_cell_of_the_tab_it_names_not_the_current_one() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    h.api("tab.create", serde_json::json!({})).await.unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains(" 1 │ 2 │"),
        Duration::from_secs(2),
    )
    .await;
    h.api("tab.rename", serde_json::json!({"tab": "1"}))
        .await
        .unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("Name tab"),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(
        row(&f, 0),
        "| proj › main  Name tab 1 ›   │ 2 │ + │       ⏎ save · esc cancel · empty clears |",
        "{f}"
    );
    assert!(
        f.contains("r0 c13-26 dim fg=#1e1e2e bg=#cba6f7"),
        "the prompt fills tab 1's cell, not tab 2's:\n{f}"
    );
    // The accent fill is the prompt's cell and nothing else: c13 to c28 is the label, the caret
    // and the trailing pad. Tab 2 is where the two marks used to collide.
    assert_eq!(one_accent_run(&f), (0, 13, 28), "{f}");
    assert!(
        f.contains("r0 c30-32 bold fg=#cdd6f4 bg=#181825"),
        "tab 2 is still the current tab, bright and bold against the dim others, but it does not \
         wear the mark that means the keys go to it:\n{f}"
    );
    h.type_text(h.client.clone(), "one").await;
    h.key(h.client.clone(), "Enter").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("1 one"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        f.contains(" 1 one │ 2 │"),
        "the name landed on the tab the prompt named:\n{f}"
    );
}

/// The tab row's window is seeded on the cell the keys go to, not on the current tab.
///
/// Found by rendering the row rather than by reasoning about it: with a wide prompt on tab 1 and
/// the view on tab 3, the window seeded at tab 3 and elided tab 1 away, so the row showed
/// `…│ 2 │ 3 │ + │` beside `⏎ save · esc cancel` - the keys to save a name, and no name on screen
/// to save. A prompt that cannot be seen is worse than a prompt that cannot be opened.
///
/// It also pins the order the cells go in when they cannot all fit: the prompt first, then the
/// current tab, then the `+`. A control to create a tab is worth less than either.
#[tokio::test]
async fn the_prompt_cell_is_never_elided_off_the_row() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    for _ in 0..2 {
        h.api("tab.create", serde_json::json!({})).await.unwrap();
    }
    h.api(
        "tab.rename",
        serde_json::json!({"tab": "1", "name": "auth"}),
    )
    .await
    .unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains(" 1 auth "),
        Duration::from_secs(2),
    )
    .await;
    h.api("tab.rename", serde_json::json!({"tab": "1"}))
        .await
        .unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("Name tab 1 ›"),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(
        row(&f, 0),
        "| proj › main  Name tab 1 › auth  │ 2 │ 3 │   ⏎ save · esc cancel · empty clears |",
        "{f}"
    );
    // The prompt's whole cell, c13 to c32, and nothing else.
    assert_eq!(one_accent_run(&f), (0, 13, 32), "{f}");
    assert!(
        f.contains("r0 c38-40 bold fg=#cdd6f4 bg=#181825"),
        "tab 3 is still on the row as the current tab, and the `+` gave up its cells for it:\n{f}"
    );
}

/// The other arm of the one-accent-fill rule. An overlay that is not a prompt has no cell in the
/// tab row, so nothing there is filled at all: the overlay marks itself the way any focused box
/// does, with an accent border and a bold accent title. The current tab still has to be
/// identifiable, which is the whole reason the fill is not simply dropped.
#[tokio::test]
async fn an_open_overlay_takes_the_accent_fill_off_the_tab_row() {
    let mut h = Harness::start(Config::default(), 80, 30).await;
    h.api("tab.create", serde_json::json!({})).await.unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains(" 1 │ 2 │"),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(
        one_accent_run(&f),
        (0, 17, 19),
        "with the keys on a pane the current tab is the accent-filled run:\n{f}"
    );
    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), "?").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("┌ Keys"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        accent_filled_cells(&f).is_empty(),
        "the keys go to the overlay, so no cell claims them with a fill:\n{f}"
    );
    assert!(
        f.contains("r0 c17-19 bold fg=#cdd6f4 bg=#181825"),
        "tab 2 is still bright and bold:\n{f}"
    );
    assert!(
        f.contains("r0 c13-15 fg=#7f849c bg=#181825"),
        "and tab 1 is still dim, so which tab is current is still legible:\n{f}"
    );
    // The overlay says the keys are its own the way every focused box does.
    assert!(
        f.contains("fg=#cba6f7"),
        "the overlay's border and title carry the accent as a foreground:\n{f}"
    );
    h.key(h.client.clone(), "Esc").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| !f.contains("┌ Keys"),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(
        one_accent_run(&f),
        (0, 17, 19),
        "closing it gives the fill back to the current tab:\n{f}"
    );
}

/// A defect the Task 15 review found and deferred here: the top bar dropped tabs, the `+` and
/// the whole right end with no indicator, and could hide the current tab. The current tab is
/// the one visible focus target (principle 2), so it stays; each elided end says so with `…`
/// rather than the row just ending (principle 6).
#[tokio::test]
async fn the_tab_row_elides_both_ends_around_the_current_tab_when_the_tabs_do_not_fit() {
    let mut h = Harness::start(Config::default(), 60, 10).await;
    for _ in 0..8 {
        h.api("tab.create", serde_json::json!({})).await.unwrap();
    }
    h.api("tab.select", serde_json::json!({"tab": "5"}))
        .await
        .unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("│ 5 │"),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(
        row(&f, 0),
        "| proj › main …│ 3 │ 4 │ 5 │ 6 │ 7 │…│ + │ 14:32   Fri 4 Sep |",
        "{f}"
    );
    assert!(
        f.contains("r0 c23-25 bold fg=#1e1e2e bg=#cba6f7"),
        "tab 5 is the current cell:\n{f}"
    );
}

/// The same defect at the width where even `+` has to go: nine tabs on a 40 column screen.
/// The current tab and the mark that says tabs were dropped are the last two things to go.
#[tokio::test]
async fn the_tab_row_drops_the_plus_before_the_current_tab_on_a_narrow_screen() {
    let mut h = Harness::start(Config::default(), 40, 10).await;
    for _ in 0..7 {
        h.api("tab.create", serde_json::json!({})).await.unwrap();
    }
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("│ 8 │"),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(
        row(&f, 0),
        "| proj › main …│ 8 │   14:32   Fri 4 Sep |",
        "{f}"
    );
    assert!(
        f.contains("r0 c15-17 bold fg=#1e1e2e bg=#cba6f7"),
        "the current tab is what the row keeps:\n{f}"
    );
}

/// A defect the Task 17 review found and deferred here: the right end had no collision
/// handling beyond `max(x + 1)`, so a hint too long for the room beside the tab row started
/// one cell after the tabs and ran off the screen edge. It elides instead (principle 6), and
/// a cell of gap keeps it off the tab row.
#[tokio::test]
async fn a_hint_too_long_for_the_room_beside_the_tabs_elides() {
    let mut h = Harness::start(Config::default(), 40, 10).await;
    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), "9").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("tab 9 does not"),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(
        row(&f, 0),
        "| proj › main  1 │ + │ tab 9 does not e… |",
        "{f}"
    );
}

/// The other two ways out of the help overlay. Esc has its own test above; `q` and `?` are
/// the ones the footer offers a reader who is already holding the help key, and a way out
/// that only some of the documented keys take is a dead end for the rest (principle 9).
#[tokio::test]
async fn help_closes_on_q_and_on_the_help_key() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    for close in ["q", "?"] {
        h.key(h.client.clone(), "C-a").await;
        h.key(h.client.clone(), "?").await;
        h.wait_for(
            h.client.clone(),
            |f| f.contains("┌ Keys"),
            Duration::from_secs(2),
        )
        .await;
        h.key(h.client.clone(), close).await;
        h.wait_for(
            h.client.clone(),
            |f| !f.contains("┌ Keys"),
            Duration::from_secs(2),
        )
        .await;
    }
    let pane = h.focused_pane(h.client.clone());
    assert!(
        h.pane_input(&pane).is_empty(),
        "no key that closed the overlay reached the pane"
    );
    assert_eq!(
        h.model().client(&h.client).unwrap().focus,
        domux_core::model::Focus::Pane(pane),
        "closing returned focus to the pane"
    );
}

/// The prompt's cursor keys, each pinned by what it changes about the saved name. From
/// `abc`: Home, `1`, End, `2`, Left, `3`, Right, Backspace. Drop any one of the five and the
/// name comes out different, which is what makes this test bite rather than pass along.
#[tokio::test]
async fn the_prompt_moves_its_cursor_with_home_end_and_the_arrows() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    h.api("tab.rename", serde_json::json!({"name": "abc"}))
        .await
        .unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains(" 1 abc "),
        Duration::from_secs(2),
    )
    .await;
    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), ",").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("Name tab 1 › abc"),
        Duration::from_secs(2),
    )
    .await;
    for key in ["Home", "1", "End", "2", "Left", "3", "Right", "Backspace"] {
        h.key(h.client.clone(), key).await;
    }
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("› 1abc3"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        f.contains("Name tab 1 › 1abc3"),
        "the cursor went where each key said:\n{f}"
    );
    h.key(h.client.clone(), "Enter").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| !f.contains("Name tab"),
            Duration::from_secs(2),
        )
        .await;
    assert!(f.contains(" 1 1abc3 "), "the edited name was saved:\n{f}");
}

/// The right end's own floor, and what the bar may drop whole.
///
/// When the anchor cell wants the whole row - a long tab name on a narrow screen - the room left
/// for the right end saturated to none, and the message was dropped with no mark at all: the key
/// was pressed, the action failed, and the row was byte for byte what it had been (principles 8
/// and 9). Everything but the clock now keeps a floor of its own and elides into it, taking the
/// cells off a tab name that already knows how to draw itself cut (ruled 2026-09-07).
#[tokio::test]
async fn an_actionable_right_end_elides_into_a_floor_of_its_own_rather_than_going_whole() {
    let mut h = Harness::start(Config::default(), 40, 10).await;
    h.api(
        "tab.rename",
        serde_json::json!({"name": "a-very-long-branch-name-here-x"}),
    )
    .await
    .unwrap();
    let before = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("a-very-long"),
            Duration::from_secs(2),
        )
        .await;
    let before = row(&before, 0).to_string();
    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), "9").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("tab 9"),
            Duration::from_secs(2),
        )
        .await;
    assert_ne!(row(&f, 0), before, "the failed key changed nothing:\n{f}");
    // A cell of the tab row's budget is the gap before the right end, so the two elisions do
    // not abut: `…tab 9 d…` read as one run of text, with the tab row's cut mark looking like
    // part of the message (ruled 2026-09-07).
    assert_eq!(
        row(&f, 0),
        "| proj › main  1 a-very-long-b… tab 9 d… |",
        "{f}"
    );
}

/// The top bar's own rule, in the case that broke it: neither the tab row nor the right end may
/// reach the last column. The empty cell there keeps the bar reading as a bar rather than as
/// text pressed against the screen edge, so it comes off the room before the two share it -
/// including when the right end is the clock and gives way whole.
#[tokio::test]
async fn the_tab_row_leaves_the_last_column_empty_when_it_wants_the_whole_row() {
    let mut h = Harness::start(Config::default(), 40, 10).await;
    h.api(
        "tab.rename",
        serde_json::json!({"name": "a-very-long-branch-name-here-x"}),
    )
    .await
    .unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("a-very-long"),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(
        row(&f, 0),
        "| proj › main  1 a-very-long-branch-nam… |",
        "{f}"
    );
}
