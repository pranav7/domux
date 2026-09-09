//! The switcher's filter, the keys overlay over it, and the extra lines the width buys.
//!
//! Everything here is read through a real frame. The rows, the truncation and the footer's
//! words all have unit tests of their own in `render::projects_box` and `render::overlay`;
//! what those cannot answer is whether the switcher's own geometry, its filter and its help
//! reach the screen the reader is looking at.
//!
//! Rows are read out of the box's own columns rather than out of the whole frame. Every test
//! that builds slots leaves `Created workspace-2` in the footer, and the top bar carries the
//! workspace name, so `f.contains("workspace-2")` is answered by two things that are not the
//! row (a second cause for the same observation).

use domux_core::config::Config;
use domux_core::model::{Focus, Overlay, RegionKind};
use domux_server::testing::{Harness, HarnessOptions};
use domux_server::FixedClock;
use serde_json::json;
use std::time::Duration;

/// One row of the frame's `|...|` block, without the bars.
fn row(frame: &str, y: usize) -> String {
    frame
        .lines()
        .filter(|l| l.starts_with('|'))
        .nth(y)
        .unwrap_or_else(|| panic!("no row {y} in the frame"))
        .trim_matches('|')
        .to_string()
}

/// Cells `from` to `to` of a row, counted in cells. A frame row is full of box drawing and a
/// byte offset into one lands inside a glyph.
fn cells(row: &str, from: usize, to: usize) -> String {
    row.chars().skip(from).take(to + 1 - from).collect()
}

/// The inside of the switcher's box on an 80 column screen: it is 60 wide at x=10, so its
/// border sits in cells 10 and 69 and its text lives in 11 to 68.
fn box_row(frame: &str, y: usize) -> String {
    cells(&row(frame, y), 11, 68).trim_end().to_string()
}

/// Every row of the box's inside, joined, for an assertion about what is not on the screen.
fn box_text(frame: &str) -> String {
    frame
        .lines()
        .filter(|l| l.starts_with('|'))
        .map(|l| cells(l.trim_matches('|'), 11, 68))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The style of one cell, read out of the frame's own style dump, whose lines are
/// `r{row} c{from}-{to} [attrs] fg=# bg=#`.
fn style_at(frame: &str, y: usize, x: usize) -> String {
    for line in frame.lines() {
        let Some(rest) = line.strip_prefix(&format!("r{y} c")) else {
            continue;
        };
        let (span, style) = rest.split_once(' ').unwrap_or((rest, ""));
        let (from, to) = span.split_once('-').expect("a style run is c<from>-<to>");
        let (from, to) = (from.parse().unwrap(), to.parse().unwrap());
        if (from..=to).contains(&x) {
            return style.to_string();
        }
    }
    panic!("no style covers r{y} c{x} in:\n{frame}")
}

fn overlays(h: &Harness) -> (Option<Overlay>, Option<Overlay>) {
    let view = h
        .model()
        .client(&h.client)
        .expect("the first client is attached")
        .clone();
    (view.overlay, view.overlay_under)
}

/// The switcher, open, with `audrey-app` holding `main`, a named `workspace-1` and an
/// unnamed `workspace-2`, and the harness's own `proj` under it.
///
/// The name is on one slot only and the two slots are otherwise alike, so a filter that kept
/// everything and one that kept nothing both fail, and so does one that matched the handle
/// when the fixture meant the name.
async fn switcher_with_a_named_slot(h: &mut Harness) -> String {
    let (_root, w1, _w2) = h.git_project_with_two_slots().await;
    h.api(
        "workspace.rename",
        json!({"workspace": w1.as_str(), "name": "auth cleanup"}),
    )
    .await
    .unwrap();
    h.api("switcher.open", json!({})).await.unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains("┌ Projects"),
        Duration::from_secs(3),
    )
    .await
}

/// `/` gives the footer to the filter, the box narrows as the letters arrive, Enter keeps the
/// filter and hands the keys back to the list, and Esc clears it (interface spec 12.10).
///
/// "auth" matches the name of one slot and nothing else in the fixture: not `audrey-app`, not
/// either handle, and not `proj`. So the row that stays could only have been kept by the name,
/// and the three rows that go could only have gone by the filter.
#[tokio::test]
async fn slash_filters_the_switcher_as_you_type_and_enter_keeps_it_and_esc_clears_it() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let f = switcher_with_a_named_slot(&mut h).await;
    assert!(
        box_text(&f).contains("auth cleanup") && box_text(&f).contains("workspace-2"),
        "both slots start visible:\n{f}"
    );

    h.key(h.client.clone(), "/").await;
    h.type_text(h.client.clone(), "auth").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("Filter › auth"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        f.contains("esc clear"),
        "the footer becomes the filter input and says the way out:\n{f}"
    );
    assert!(
        box_text(&f).contains("auth cleanup"),
        "the row the name matches stays:\n{f}"
    );
    assert!(
        !box_text(&f).contains("workspace-2") && !box_text(&f).contains("PROJ"),
        "and the box narrows to it, header and all:\n{f}"
    );

    h.key(h.client.clone(), "Enter").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("⏎ open"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        box_text(&f).contains("auth cleanup") && !box_text(&f).contains("workspace-2"),
        "Enter keeps the filter and gives the keys back to the list:\n{f}"
    );

    h.key(h.client.clone(), "/").await;
    h.key(h.client.clone(), "Esc").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| box_text(f).contains("workspace-2"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        f.contains("⏎ open") && !f.contains("Filter ›"),
        "Esc clears the filter and the hints come back:\n{f}"
    );
    assert_eq!(
        overlays(&h).0,
        Some(Overlay::Switcher),
        "and the switcher is still the overlay: Esc closed the field, not the box"
    );
}

/// A filter that matches nothing names what was searched for and the way out (principle 9),
/// rather than showing the same empty box a domux with no projects shows.
#[tokio::test]
async fn a_filter_that_matches_nothing_names_what_was_searched_for_and_the_way_out() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    switcher_with_a_named_slot(&mut h).await;
    h.key(h.client.clone(), "/").await;
    h.type_text(h.client.clone(), "zzz").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("No workspace matches"),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(
        box_row(&f, 4),
        "No workspace matches \"zzz\". esc clears the filter",
        "the state and the next action, in the box's first row:\n{f}"
    );
    assert!(
        !box_text(&f).contains("auth cleanup") && !box_text(&f).contains("AUDREY-APP"),
        "and nothing of the list is left behind it:\n{f}"
    );
}

/// `?` opens the keys over the switcher and Esc comes back to it, rather than closing both
/// (interface spec 12.7).
///
/// The state is asserted as well as the frame: `overlay_under` is what makes this one press
/// of Esc rather than two, and a help that had replaced the switcher would draw the same
/// `┌ Keys` box.
#[tokio::test]
async fn question_mark_opens_the_keys_over_the_switcher_and_esc_comes_back_to_it() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    switcher_with_a_named_slot(&mut h).await;
    h.key(h.client.clone(), "?").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("┌ Keys"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        f.contains("C-a d      client.detach"),
        "the help renders the configured bindings:\n{f}"
    );
    assert_eq!(
        overlays(&h),
        (Some(Overlay::Help), Some(Overlay::Switcher)),
        "and it went over the switcher rather than replacing it"
    );

    h.key(h.client.clone(), "Esc").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| !f.contains("┌ Keys"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        f.contains("┌ Projects") && box_text(&f).contains("auth cleanup"),
        "one Esc comes back to the switcher, with its rows:\n{f}"
    );
    assert_eq!(
        overlays(&h),
        (Some(Overlay::Switcher), None),
        "and nothing is left stranded under it"
    );
    assert_eq!(
        h.model().client(&h.client).unwrap().focus,
        Focus::Region(RegionKind::Switcher),
        "the keys are in the box that came back, named as the box it is, not on a pane\n\
         behind it and not as a modal with no key table of its own"
    );

    // The keys really are the switcher's again: `/` is a `[keys.list]` key and nothing else
    // in the tree answers it.
    h.key(h.client.clone(), "/").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("Filter ›"),
        Duration::from_secs(2),
    )
    .await;
}

/// Opening the keys twice leaves one of them, so one Esc is the way out.
///
/// Only the API can ask twice: the second `?` is a key, and a key with the help open reaches
/// the help's own arm and closes it.
#[tokio::test]
async fn opening_the_keys_twice_over_the_switcher_still_closes_on_one_esc() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    switcher_with_a_named_slot(&mut h).await;
    h.api("help", json!({})).await.unwrap();
    h.api("help", json!({})).await.unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains("┌ Keys"),
        Duration::from_secs(2),
    )
    .await;
    assert_eq!(
        overlays(&h),
        (Some(Overlay::Help), Some(Overlay::Switcher)),
        "the second call did not push a second help over the first"
    );
    h.key(h.client.clone(), "Esc").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| !f.contains("┌ Keys"),
            Duration::from_secs(2),
        )
        .await;
    assert!(f.contains("┌ Projects"), "and one Esc is the way out:\n{f}");
}

/// The help lists `[keys.list]` under `in a list`, from the loaded table rather than from the
/// default spelling (principle 3).
///
/// A tall screen, because the three tables do not fit on 24 rows and this test is about what
/// the block says rather than about the truncation, which
/// `help_keeps_the_footer_when_the_screen_is_too_short_for_every_binding` owns. `/` is
/// rebound and `?` is removed, so a block built from the defaults draws a row this fixture
/// cannot produce and misses one it must.
#[tokio::test]
async fn the_keys_overlay_lists_the_box_keys_from_the_configured_table() {
    let mut cfg = Config::default();
    cfg.keys.list.remove("/");
    cfg.keys.list.insert("g".into(), "list.filter".into());
    cfg.keys.list.remove("?");
    let mut h = Harness::start(cfg, 80, 48).await;
    h.api("help", json!({})).await.unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("┌ Keys"),
            Duration::from_secs(2),
        )
        .await;
    assert!(f.contains("in a list"), "the block has a heading:\n{f}");
    assert!(
        f.contains("j          list.down") && f.contains("Enter      list.activate"),
        "the table is drawn in the shape the other two tables use:\n{f}"
    );
    assert!(
        f.contains("g          list.filter"),
        "the rebound key is what shows:\n{f}"
    );
    assert!(
        !f.contains("/          list.filter"),
        "and the default it replaced is gone:\n{f}"
    );
    assert!(
        !f.contains("?          help"),
        "a list key rebound to nothing drops out of the block:\n{f}"
    );
    assert!(
        !f.contains("more, see domux.toml"),
        "nothing is truncated at this height, so an absence above is a real absence:\n{f}"
    );
    // The block is last, after the two tables that apply wherever the reader is standing.
    let list = f.find("in a list").expect("the heading is on the screen");
    let globals = f.find("focus.left").expect("the globals are on the screen");
    let passthrough = f
        .find("keep C-h")
        .expect("the passthrough rule is on the screen");
    assert!(
        list > globals && list > passthrough,
        "the box keys come after the leader table, the globals and the passthrough rule:\n{f}"
    );
    // By action, like the other two tables, and not by key. `Enter` against `Down` is the
    // pair that separates the two: by action `list.activate` comes before `list.down`, by
    // key `Down` comes before `Enter`. `Enter` against `j` would pass under either, which is
    // what this assertion said first and what the mutant caught.
    assert!(
        f.find("Enter      list.activate") < f.find("Down       list.down"),
        "the block is sorted by action:\n{f}"
    );
    // A blank row above the heading, so the block reads as its own table rather than as more
    // of the passthrough rule above it.
    let y = f
        .lines()
        .filter(|l| l.starts_with('|'))
        .position(|l| l.contains("in a list"))
        .expect("the heading is on a row");
    assert_eq!(
        row(&f, y - 1).trim_matches(|c| c == ' ' || c == '\u{2502}'),
        "",
        "the heading has a blank row over it:\n{f}"
    );
}

/// The switcher has the width for the tab list and the sidebar does not (interface spec 5.5).
///
/// The tab is named, so the line the switcher draws is one no other row of either surface can
/// produce: a bare tab number would also appear under a workspace with an unnamed tab.
#[tokio::test]
async fn the_switcher_shows_the_tab_list_and_the_sidebar_does_not() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let (_root, w1, _w2) = h.git_project_with_two_slots().await;
    h.api("workspace.focus", json!({"workspace": w1.as_str()}))
        .await
        .unwrap();
    h.api("tab.rename", json!({"name": "pr1"})).await.unwrap();
    h.api("switcher.open", json!({})).await.unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("┌ Projects"),
            Duration::from_secs(3),
        )
        .await;
    assert!(
        f.contains("1 pr1"),
        "the switcher has the width for the tab list:\n{f}"
    );

    h.key(h.client.clone(), "Esc").await;
    h.api("sidebar.show", json!({})).await.unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("┌ Projects"),
            Duration::from_secs(3),
        )
        .await;
    let sidebar: String = f
        .lines()
        .filter(|l| l.starts_with('|'))
        .map(|l| cells(l.trim_matches('|'), 0, 37))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        sidebar.contains("auth") || sidebar.contains("workspace-1"),
        "the sidebar is drawing the same workspace:\n{f}"
    );
    assert!(
        !sidebar.contains("1 pr1"),
        "and its compact rows drop the tab list:\n{f}"
    );
}

/// A row too wide for the box is cut inside it, wide graphemes included (interface spec 5.6).
///
/// The name is 40 CJK characters, 80 cells in a box that has 58. `truncate_with_ellipsis`
/// spends 57 of them, because the 29th wide grapheme would need cells 57 and 58 and only one
/// is left, so the row is 28 graphemes and an ellipsis and the last cell of the box stays
/// blank. That blank is the whole point of the assertion: it is the cell an implementation
/// that measured in characters, or that let a wide grapheme straddle the border, would have
/// written into. The row is asserted whole rather than sliced because a frame collapses a
/// wide glyph and its spacer into one character, so counting characters into a row of CJK
/// lands in the wrong cell - which is the mistake this test exists to catch.
#[tokio::test]
async fn a_name_of_wide_graphemes_is_cut_inside_the_switchers_box() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (_root, w1, _w2) = h.git_project_with_two_slots().await;
    h.api(
        "workspace.rename",
        json!({"workspace": w1.as_str(), "name": "設定".repeat(20)}),
    )
    .await
    .unwrap();
    h.api("switcher.open", json!({})).await.unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("設定"),
            Duration::from_secs(3),
        )
        .await;
    let y = f
        .lines()
        .filter(|l| l.starts_with('|'))
        .position(|l| l.contains("設定"))
        .expect("the name is on the screen");
    assert_eq!(
        row(&f, y),
        format!("│         │{}… │         │", "設定".repeat(14)),
        "28 wide graphemes and an ellipsis fill 57 of the box's 58 cells, and the 58th is\n\
         left blank because a 29th cannot be half drawn:\n{f}"
    );
}

/// Line 2 is cut to the switcher's own width, and the pull request number is never what goes
/// (interface spec 5.6 and 12.16).
///
/// The row builder's ordering has unit tests of its own. What only a real frame can answer is
/// which width the switcher hands it: the box is 58 cells here, so the title has 29 to live in
/// and loses one to the ellipsis. A switcher that passed the sidebar's 36, or the screen's 80,
/// or the box's outer 60, cuts the title somewhere else and this row changes.
///
/// The branch, the number and the title are three separate facts of three different lengths,
/// so the one that was shortened is named by the row rather than inferred from it.
#[tokio::test]
async fn the_switcher_cuts_the_pull_request_title_to_its_own_width_and_never_the_number() {
    let tmp = tempfile::tempdir().unwrap();
    let state = tmp.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let mut h = Harness::start_with(HarnessOptions {
        state_dir: Some(state.clone()),
        ..HarnessOptions::new(Config::default(), 80, 24)
    })
    .await;
    let (_root, w1, _w2) = h.git_project_with_two_slots().await;
    h.stop().await;
    // Stamped from the clock the server runs on: a fact from the future is not fresh, and
    // `Local::now()` is in the future of a fixed clock.
    let at = FixedClock::at("2026-09-04T14:32:00").0.to_rfc3339();
    std::fs::write(
        state.join("pr-cache.json"),
        format!(
            r#"{{"schema_version":1,"facts":{{"{w1}/branch":{{"text":"feat/auth-cleanup","fetched_at":"{at}","ttl":600}},"{w1}/pr":{{"text":"PR#212","state":"OPEN","url":"Consolidate the auth middleware into one guard","fetched_at":"{at}","ttl":600}}}}}}"#
        ),
    )
    .unwrap();
    h.restart().await;

    h.api("switcher.open", json!({})).await.unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("PR#212"),
            Duration::from_secs(3),
        )
        .await;
    let y = f
        .lines()
        .filter(|l| l.starts_with('|'))
        .position(|l| l.contains("PR#212"))
        .expect("the pull request is on the screen");
    assert_eq!(
        row(&f, y),
        "│         │feat/auth-cleanup · PR#212 · Consolidate the auth middlew…│         │",
        "the title takes what the branch and the number leave and loses its tail:\n{f}"
    );

    h.key(h.client.clone(), "Esc").await;
    h.api("sidebar.show", json!({"client": h.client.as_str()}))
        .await
        .unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("PR#212"),
            Duration::from_secs(3),
        )
        .await;
    let y = f
        .lines()
        .filter(|l| l.starts_with('|'))
        .position(|l| l.contains("PR#212"))
        .expect("the pull request is on the screen");
    assert_eq!(
        cells(&row(&f, y), 0, 37),
        "│feat/auth-cleanup · PR#212          │",
        "and the sidebar drops the title whole rather than shortening it:\n{f}"
    );
}

/// A note and an open filter want the same row. The filter has it, and gives it up to a pill
/// (interface spec 12.10 and 12.12).
///
/// A text field with no marker on the screen is a mode the reader cannot see they are in
/// (principle 2), where a note kept waiting behind one is only late: closing the field brings
/// it back, which is what the last step asserts and what separates outranked from cleared.
///
/// Reached over the API throughout, and that is the point rather than a convenience. A key in
/// a box clears the notes before it is routed, so `/` pressed by hand can never meet one.
/// `list.filter` opens the same field without a key, and Task 21's comment here said the state
/// was unreachable on the strength of the key alone.
#[tokio::test]
async fn the_switchers_filter_takes_the_footer_from_a_note_and_gives_it_to_a_pill() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (root, _w1, _w2) = h.git_project_with_two_slots().await;
    h.stop().await;
    std::fs::remove_dir_all(root.join(".domux/worktrees")).unwrap();
    h.restart().await;

    h.api("switcher.open", json!({})).await.unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("Pruned workspace-1"),
            Duration::from_secs(2),
        )
        .await;
    let y = f
        .lines()
        .filter(|l| l.starts_with('|'))
        .position(|l| l.contains("Pruned workspace-1"))
        .expect("the note is on the screen");

    h.api("list.filter", json!({})).await.unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("Filter ›"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        !f.contains("Pruned workspace-1"),
        "the filter field has the row, not a share of it:\n{f}"
    );

    // Outranked, not read. Closing the field over the API touches no note, so the row goes
    // back to saying what the start-up prune took away.
    h.api("switcher.close", json!({})).await.unwrap();
    h.api("switcher.open", json!({})).await.unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains("Pruned workspace-1"),
        Duration::from_secs(2),
    )
    .await;

    h.api("list.filter", json!({})).await.unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains("Filter ›"),
        Duration::from_secs(2),
    )
    .await;
    let _ = h
        .api("project.add", json!({"path": "/no/such/folder"}))
        .await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| !f.contains("Filter ›"),
            Duration::from_secs(5),
        )
        .await;
    assert_eq!(
        style_at(&f, y, 11),
        "bold fg=#1e1e2e bg=#f38ba8",
        "and the field itself gives the row up to a pill, which answers what the reader\n\
         just did:\n{f}"
    );
}

/// The sidebar's hint row answers the same two the same way, because it is the same question
/// asked on the other surface.
///
/// It did not before Task 20: the note came first there and second in `overlay::footer`, and
/// the comment beside each said the two agreed. So this is the assertion that would have
/// failed, and the reason it is written on the sidebar rather than on the switcher.
#[tokio::test]
async fn the_sidebars_hint_row_orders_the_note_and_the_filter_as_the_footer_does() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let (root, _w1, _w2) = h.git_project_with_two_slots().await;
    h.stop().await;
    std::fs::remove_dir_all(root.join(".domux/worktrees")).unwrap();
    h.restart().await;

    h.api("sidebar.show", json!({"client": h.client.as_str()}))
        .await
        .unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("Pruned workspace-1"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        cells(&row(&f, 23), 0, 37).contains("Pruned workspace-1"),
        "the note starts in the sidebar's hint row:\n{f}"
    );

    h.api(
        "focus.region",
        json!({"region": "sidebar_projects", "client": h.client.as_str()}),
    )
    .await
    .unwrap();
    h.api("list.filter", json!({"client": h.client.as_str()}))
        .await
        .unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| cells(&row(f, 23), 0, 37).contains("Filter ›"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        !row(&f, 23).contains("Pruned"),
        "and the open field takes it from the note:\n{}",
        row(&f, 23)
    );
}

/// A `[keys.list]` table the reader has emptied draws no block at all, rather than a heading
/// with nothing under it.
///
/// The same rule the passthrough block already follows, and the reason it needs its own test
/// is that the heading and the blank row are pushed before the rows are: an unguarded block
/// leaves two lines behind that name a table with nothing in it.
#[tokio::test]
async fn an_empty_list_table_draws_no_heading_in_the_keys_overlay() {
    let mut cfg = Config::default();
    cfg.keys.list.clear();
    let mut h = Harness::start(cfg, 80, 48).await;
    h.api("help", json!({})).await.unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("┌ Keys"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        !f.contains("in a list"),
        "no heading with nothing under it:\n{f}"
    );
    assert!(
        f.contains("C-a s      switcher.open") && f.contains("esc close"),
        "and the rest of the overlay is where it was:\n{f}"
    );
}

/// The row that says the overlay is truncated, wherever the keys are. Read out of the box's
/// own inside so nothing else on the screen can answer for it.
fn more_row(frame: &str) -> bool {
    frame
        .lines()
        .filter(|l| l.starts_with('|'))
        .any(|l| l.contains("more, see domux.toml"))
}

/// The line number of the row holding `needle`, among the frame's `|...|` rows.
fn row_holding(frame: &str, needle: &str) -> usize {
    frame
        .lines()
        .filter(|l| l.starts_with('|'))
        .position(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("no row holds {needle:?} in:\n{frame}"))
}

/// `?` from inside a box lists the box's keys first, on both boxes, because it is one
/// question about where the reader's keys are.
///
/// A 24 row screen on purpose: the overlay shows 17 of its 38 lines there, so the order is
/// the whole of what the reader gets, and a block placed last would be entirely gone. Both
/// halves are in one test because the claim is that the two surfaces answer alike; separated,
/// each half would pass under an implementation that special-cased its own surface.
#[tokio::test]
async fn the_keys_overlay_lists_the_box_keys_first_from_either_box() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    h.api("switcher.open", json!({})).await.unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains("┌ Projects"),
        Duration::from_secs(3),
    )
    .await;
    h.key(h.client.clone(), "?").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("┌ Keys"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        f.contains("j          list.down"),
        "the switcher's box keys are on the screen at 24 rows:\n{f}"
    );
    assert!(
        row_holding(&f, "in a list") < row_holding(&f, "client.detach"),
        "and they come before the leader table:\n{f}"
    );
    assert!(
        more_row(&f),
        "and the overlay says the rest of it did not fit:\n{f}"
    );
    // A blank row under the block, so it reads as its own table rather than running into the
    // leader table below it. The pane ordering pins the blank on the other side of the block,
    // and the two are separate lines of code: the mutant for this one survived a sweep with
    // only that assertion in the suite. The anchor is whichever leader binding sorts first,
    // not `client.detach` by name: M3's `agents.open` took that place, and the claim is about
    // the blank row above the leader table rather than about which action heads it.
    let first_leader = f
        .lines()
        .filter(|l| l.starts_with('|'))
        .position(|l| l.contains("C-a ") && !l.contains("leader"))
        .unwrap_or_else(|| panic!("no leader binding row in:\n{f}"));
    assert_eq!(
        row(&f, first_leader - 1).trim_matches(|c| c == ' ' || c == '\u{2502}'),
        "",
        "the block ends with a blank row before the leader table:\n{f}"
    );

    // The sidebar's box, reached with no overlay in the way, gets the same answer.
    h.key(h.client.clone(), "Esc").await;
    h.key(h.client.clone(), "Esc").await;
    h.api("sidebar.show", json!({"client": h.client.as_str()}))
        .await
        .unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains("┌ Projects"),
        Duration::from_secs(3),
    )
    .await;
    h.key(h.client.clone(), "C-h").await;
    h.frame(h.client.clone()).await;
    assert_eq!(
        h.model().client(&h.client).unwrap().focus,
        Focus::Region(RegionKind::SidebarProjects),
        "the keys are in the sidebar's box, which is the state this half is about"
    );
    h.key(h.client.clone(), "?").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("┌ Keys"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        f.contains("j          list.down")
            && row_holding(&f, "in a list") < row_holding(&f, "client.detach"),
        "the sidebar's box gets the same order, from the same rule:\n{f}"
    );
    assert!(more_row(&f), "and the same truncation row:\n{f}");
}

/// `leader ?` from a pane lists the leader table first, which is the other half of the one
/// rule: the reader is not holding the box keys, so they are the table that gives way.
///
/// The same 24 row screen as the box case, so the two differ in where the keys are and in
/// nothing else.
#[tokio::test]
async fn the_keys_overlay_lists_the_leader_table_first_from_a_pane() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
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
        f.contains("C-a d      client.detach") && f.contains("C-a s      switcher.open"),
        "the leader table is what a reader on a pane gets:\n{f}"
    );
    assert!(
        !f.contains("in a list"),
        "and the box keys are what the truncation drops:\n{f}"
    );
    assert!(
        more_row(&f),
        "which the overlay says rather than presenting a partial list as the whole of it:\n{f}"
    );
}

/// `?` from the sidebar's Projects box comes back to the box, the same way `?` over the
/// switcher comes back to the switcher (interface spec 12.7 applied to the other surface).
///
/// The focus is asserted and then used: `j` moves the cursor, which is what proves the box
/// has the keys rather than only being labelled as though it does. Esc is not the check here,
/// because Esc in the box is `focus.pane` and that method's whole job is to leave.
#[tokio::test]
async fn question_mark_from_the_sidebars_box_comes_back_to_the_box() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    h.git_project_with_two_slots().await;
    h.api("sidebar.show", json!({"client": h.client.as_str()}))
        .await
        .unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains("┌ Projects"),
        Duration::from_secs(3),
    )
    .await;
    h.key(h.client.clone(), "C-h").await;
    h.frame(h.client.clone()).await;
    assert_eq!(
        h.model().client(&h.client).unwrap().focus,
        Focus::Region(RegionKind::SidebarProjects),
        "the keys start in the box"
    );
    let before = h.model().client(&h.client).unwrap().projects_cursor.clone();

    h.key(h.client.clone(), "?").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("┌ Keys"),
        Duration::from_secs(2),
    )
    .await;
    h.key(h.client.clone(), "Esc").await;
    h.wait_for(
        h.client.clone(),
        |f| !f.contains("┌ Keys"),
        Duration::from_secs(2),
    )
    .await;
    assert_eq!(
        h.model().client(&h.client).unwrap().focus,
        Focus::Region(RegionKind::SidebarProjects),
        "and come back to it rather than to a pane behind the sidebar"
    );
    // `k` and not `j`: entering the box puts the cursor on the workspace this client is in,
    // which is the harness's own `proj` main, and `proj` sorts after `audrey-app` so that is
    // the last row the cursor can rest on. `j` clamps there and moves nothing, and would have
    // passed against a pane that had swallowed the key.
    h.key(h.client.clone(), "k").await;
    h.frame(h.client.clone()).await;
    assert_ne!(
        h.model().client(&h.client).unwrap().projects_cursor,
        before,
        "which `k` proves: the box has the keys, not just the label"
    );
}

/// A box that is gone by the time the help closes hands the keys to the pane instead.
///
/// The client is narrowed below `SIDEBAR_MIN_COLS` while the help is open, which is the one
/// way to reach it: `sidebar.show` at 120 columns leaves `sidebar_forced` false, so the resize
/// really does take the sidebar off the screen. Without the `sidebar_visible` half of the
/// rule the keys land in a region nothing on the screen marks (principle 2).
#[tokio::test]
async fn a_help_opened_in_the_sidebars_box_gives_the_keys_to_the_pane_when_the_box_is_gone() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    h.api("sidebar.show", json!({"client": h.client.as_str()}))
        .await
        .unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains("┌ Projects"),
        Duration::from_secs(3),
    )
    .await;
    h.key(h.client.clone(), "C-h").await;
    h.frame(h.client.clone()).await;
    let pane = h.focused_pane(h.client.clone());
    h.key(h.client.clone(), "?").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("┌ Keys"),
        Duration::from_secs(2),
    )
    .await;

    h.resize(h.client.clone(), 80, 24).await;
    h.frame(h.client.clone()).await;
    assert!(
        !h.model().client(&h.client).unwrap().sidebar_visible(),
        "the sidebar is off the screen, which is what makes this the discriminating state"
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
        h.model().client(&h.client).unwrap().focus,
        Focus::Pane(pane),
        "with no box left to come back to the keys go to the pane:\n{f}"
    );
}
