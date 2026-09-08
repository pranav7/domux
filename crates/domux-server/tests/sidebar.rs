//! The sidebar's geometry and API, the tab row on the panes, and the remembered state.
//!
//! Task 13a. Nothing draws the Projects box yet - its rows are `render::projects_box`, which
//! task 11 builds - so the sidebar's own 38 columns are blank here and every test below waits
//! on the top bar going away rather than on the box arriving. Task 13b adds
//! `render::sidebar::draw` and the test that reads the box.

use domux_core::config::Config;
use domux_server::testing::{row, Harness};
use std::time::Duration;

/// Columns `from` to `to` of a frame row, without the `|` framing.
fn cols(line: &str, from: usize, to: usize) -> String {
    line.chars().skip(1 + from).take(to - from + 1).collect()
}

/// True once this client draws the sidebar's layout: no full-width top bar, and the pane box
/// starts right of the sidebar's 38 columns and the one gap column.
///
/// Positive on purpose. `!f.contains("proj › main")` alone is true of a client that has
/// drawn nothing at all, so a client that never got its frame would pass it.
fn on_the_panes(f: &str) -> bool {
    f.lines().filter(|l| l.starts_with('|')).count() > 2
        && !f.contains("proj › main")
        && cols(row(f, 1), 38, 40) == " ┌ "
}

/// `leader b` reaches `sidebar.toggle`, the full-width top bar goes, and the tab row moves
/// onto the panes with the right end's pieces at its end (interface spec 4.2).
#[tokio::test]
async fn leader_b_takes_the_top_bar_away_and_moves_the_tab_row_onto_the_panes() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), "b").await;
    let f = h
        .wait_for(h.client.clone(), on_the_panes, Duration::from_secs(2))
        .await;
    assert!(h.model().sidebar_open);
    // `cols(line, 38, 119)` takes 82 cells: the one gap column, then the 81-column
    // workpanel (120 screen - 38 sidebar - 1 gap). Every literal below is exactly 82.
    // 82 = "  1 │ + │" (9) + 55 spaces + "14:32   Fri 4 Sep " (18)
    assert_eq!(
        cols(row(&f, 0), 38, 119),
        "  1 │ + │                                                       14:32   Fri 4 Sep ",
        "the tab row sits on the panes with the clock at its end:\n{f}"
    );
    // 82 = " ┌ sh " (6) + 75 dashes + "┐" (1)
    assert_eq!(
        cols(row(&f, 1), 38, 119),
        " ┌ sh ───────────────────────────────────────────────────────────────────────────┐"
    );
    // 82 = " └" (2) + 79 dashes + "┘" (1)
    assert_eq!(
        cols(row(&f, 23), 38, 119),
        " └───────────────────────────────────────────────────────────────────────────────┘"
    );
    // The seam. Task 13b draws the Projects box into these columns and replaces this
    // assertion with the brief's own `┌ Projects ─…─┐` literals.
    assert_eq!(
        cols(row(&f, 0), 0, 37),
        " ".repeat(38),
        "task 13a leaves the sidebar's own columns blank:\n{f}"
    );
}

#[tokio::test]
async fn hiding_the_sidebar_brings_the_top_bar_back_and_the_server_remembers_the_state() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    assert_eq!(
        h.api("sidebar.toggle", serde_json::json!({}))
            .await
            .unwrap()["open"],
        true
    );
    h.wait_for(h.client.clone(), on_the_panes, Duration::from_secs(2))
        .await;
    assert!(
        h.model().sidebar_open,
        "the model remembers it (roadmap decision 4)"
    );
    let second = h.attach(120, 24).await;
    let f = h
        .wait_for(second, on_the_panes, Duration::from_secs(2))
        .await;
    assert!(
        on_the_panes(&f),
        "a new client starts in the remembered state:\n{f}"
    );
    h.api("sidebar.hide", serde_json::json!({})).await.unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("proj › main"),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(cols(row(&f, 0), 0, 20), " proj › main  1 │ + │");
    assert!(!h.model().sidebar_open);
}

#[tokio::test]
async fn a_screen_narrower_than_the_sidebar_plus_a_pane_hides_it_without_forgetting_it() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    h.api("sidebar.show", serde_json::json!({})).await.unwrap();
    h.wait_for(h.client.clone(), on_the_panes, Duration::from_secs(2))
        .await;
    h.resize(h.client.clone(), 119, 24).await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("proj › main"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        f.contains("proj › main"),
        "below 120 columns the sidebar hides itself and the top bar returns:\n{f}"
    );
    assert!(
        h.model().sidebar_open,
        "auto-hide never changes the remembered state (interface spec 12.1)"
    );
    // Open but not visible: the two answers are different questions, and on this screen
    // they have different answers.
    let result = h.api("sidebar.show", serde_json::json!({})).await.unwrap();
    assert_eq!(
        (result["open"].clone(), result["visible"].clone()),
        (serde_json::json!(true), serde_json::json!(false))
    );
    let result = h
        .api("sidebar.toggle", serde_json::json!({}))
        .await
        .unwrap();
    assert_eq!(
        (result["open"].clone(), result["visible"].clone()),
        (serde_json::json!(false), serde_json::json!(false))
    );
    h.resize(h.client.clone(), 120, 24).await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("proj › main"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        f.contains("proj › main"),
        "a sidebar you hid yourself stays hidden when the screen widens:\n{f}"
    );
}

#[tokio::test]
async fn the_pane_is_narrower_by_the_sidebar_and_the_smallest_client_still_sizes_it() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    h.api("sidebar.show", serde_json::json!({})).await.unwrap();
    h.wait_for(h.client.clone(), on_the_panes, Duration::from_secs(2))
        .await;
    let panes = h.api("pane.list", serde_json::json!({})).await.unwrap();
    assert_eq!(
        (
            panes[0]["cols"].as_u64().unwrap(),
            panes[0]["rows"].as_u64().unwrap()
        ),
        (79, 21),
        "120 minus 38 for the sidebar, 1 for the gap, 2 for the box border"
    );
}

/// One remembered state, not one per client: `Model::sidebar_open` is a single value and it
/// is what a client attaching later starts in, so a toggle on one screen reaches the other.
#[tokio::test]
async fn a_toggle_on_one_client_reaches_every_other_client() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let second = h.attach(120, 24).await;
    // Answered for the most recently active client, which is the one that just attached.
    h.api("sidebar.show", serde_json::json!({})).await.unwrap();
    let f = h
        .wait_for(h.client.clone(), on_the_panes, Duration::from_secs(2))
        .await;
    assert!(on_the_panes(&f), "the client that did not ask:\n{f}");
    let f = h
        .wait_for(second, on_the_panes, Duration::from_secs(2))
        .await;
    assert!(on_the_panes(&f), "the client that asked:\n{f}");
}

#[tokio::test]
async fn the_remembered_sidebar_survives_a_restart() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    h.api("sidebar.show", serde_json::json!({})).await.unwrap();
    h.wait_for(h.client.clone(), on_the_panes, Duration::from_secs(2))
        .await;
    h.restart().await;
    let f = h
        .wait_for(h.client.clone(), on_the_panes, Duration::from_secs(2))
        .await;
    assert!(
        on_the_panes(&f),
        "the state file carried it across the restart:\n{f}"
    );
    assert!(h.model().sidebar_open);
}

/// The sidebar takes its columns from the pane and the auto-hide gives them back, so the
/// program in the pane is resized to what is actually drawn either way.
#[tokio::test]
async fn the_pane_takes_the_sidebars_columns_back_when_the_sidebar_hides_itself() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    h.api("sidebar.show", serde_json::json!({})).await.unwrap();
    h.wait_for(h.client.clone(), on_the_panes, Duration::from_secs(2))
        .await;
    let pane = h.focused_pane(h.client.clone());
    assert_eq!((h.pane_size(&pane).cols, h.pane_size(&pane).rows), (79, 21));
    h.resize(h.client.clone(), 119, 24).await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("proj › main"),
        Duration::from_secs(2),
    )
    .await;
    assert_eq!(
        (h.pane_size(&pane).cols, h.pane_size(&pane).rows),
        (117, 21),
        "119 columns less the box border, with no sidebar to pay for"
    );
}

/// The right end keeps its floor on the tab row over the panes too, because both rows share
/// the cells by one rule: the leader indicator is a live key, not the clock.
#[tokio::test]
async fn the_leader_indicator_reaches_the_end_of_the_tab_row_on_the_panes() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    h.api("sidebar.show", serde_json::json!({})).await.unwrap();
    h.wait_for(h.client.clone(), on_the_panes, Duration::from_secs(2))
        .await;
    h.key(h.client.clone(), "C-a").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("keys"),
            Duration::from_secs(2),
        )
        .await;
    // 82 = "  1 │ + │" (9) + 61 spaces + "C-a  ? keys " (12)
    assert_eq!(
        cols(row(&f, 0), 38, 119),
        "  1 │ + │                                                             C-a  ? keys ",
        "{f}"
    );
    assert!(
        !f.contains("14:32"),
        "the chord displaces the clock, here as on the full-width bar:\n{f}"
    );
}

/// The tab row on the panes has no background of its own (interface spec 4.2): the same
/// cells on the full-width bar sit on mantle.
#[tokio::test]
async fn the_tab_row_on_the_panes_carries_no_background() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let f = h.frame(h.client.clone()).await;
    assert!(
        f.contains("r0 c16-16 fg=#313244 bg=#181825"),
        "the separator after the current tab sits on mantle on the full-width bar:\n{f}"
    );
    h.api("sidebar.show", serde_json::json!({})).await.unwrap();
    let f = h
        .wait_for(h.client.clone(), on_the_panes, Duration::from_secs(2))
        .await;
    assert!(
        f.contains("r0 c42-42 fg=#313244\n"),
        "and on nothing over the panes:\n{f}"
    );
    assert!(
        f.contains("r0 c39-41 bold fg=#1e1e2e bg=#cba6f7"),
        "while the fill on the tab that owns the keys keeps its own background:\n{f}"
    );
}

/// A tab that does not own the keys takes the row's background, and on the panes that is
/// nothing.
///
/// One tab cannot show this. The only cell would be the current one, which carries the
/// accent fill, and the fill would cover a wrong background rather than reveal it.
#[tokio::test]
async fn a_tab_that_does_not_own_the_keys_carries_no_background_on_the_panes() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    h.api("sidebar.show", serde_json::json!({})).await.unwrap();
    h.api("tab.create", serde_json::json!({})).await.unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| on_the_panes(f) && f.contains(" 2 "),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(cols(row(&f, 0), 38, 50), "  1 │ 2 │ + │", "{f}");
    assert!(
        f.contains("r0 c39-41 fg=#7f849c\n"),
        "tab 1 does not own the keys, so its cell sits on nothing:\n{f}"
    );
    assert!(
        f.contains("r0 c43-45 bold fg=#1e1e2e bg=#cba6f7"),
        "tab 2 owns them and keeps its own fill:\n{f}"
    );
}

/// Hiding the sidebar redraws a client even when it changes no pane's size.
///
/// The frame is the handler's own business. Every other test here toggles the sidebar on a
/// screen where the panes resize too, and a pane resize marks the pane dirty, which draws a
/// frame on its own: that second cause would hide a handler that never marked the view.
/// The second client below is exactly as wide as the first one's workpanel beside the
/// sidebar, so the size every client agrees on is the same open or hidden.
#[tokio::test]
async fn hiding_the_sidebar_redraws_a_client_whose_panes_do_not_change_size() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let _second = h.attach(81, 24).await;
    h.api("sidebar.show", serde_json::json!({})).await.unwrap();
    h.wait_for(h.client.clone(), on_the_panes, Duration::from_secs(2))
        .await;
    let pane = h.focused_pane(h.client.clone());
    let before = h.pane_size(&pane);
    assert_eq!((before.cols, before.rows), (79, 21));
    h.api("sidebar.hide", serde_json::json!({})).await.unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("proj › main"),
            Duration::from_secs(2),
        )
        .await;
    let after = h.pane_size(&pane);
    assert_eq!(
        (after.cols, after.rows),
        (before.cols, before.rows),
        "no pane changed size, so the redraw came from the handler:\n{f}"
    );
}

/// The state file carries the sidebar while the server is still running, on the toggle's own
/// event.
///
/// Three writers had to be ruled out to make this say what it claims. `Core::shutdown`
/// persists unconditionally, so stopping and restarting the server proves only that stopping
/// writes the file. A pane resize publishes `PaneResized`, which is structural and persists
/// too, so a toggle that changes the pane geometry would write the file whether or not it
/// raised an event of its own. The second client below is exactly as wide as the first one's
/// workpanel beside the sidebar, so showing the sidebar resizes nothing and the toggle's own
/// event is the only writer left. The harness has already written the file at startup, so a
/// missing mid-session write leaves a stale `false` here rather than no file at all.
#[tokio::test]
async fn the_sidebar_reaches_the_state_file_while_the_server_is_still_running() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let _second = h.attach(81, 24).await;
    let pane = h.focused_pane(h.client.clone());
    let before = h.pane_size(&pane);
    h.api("sidebar.show", serde_json::json!({})).await.unwrap();
    h.wait_for(h.client.clone(), on_the_panes, Duration::from_secs(2))
        .await;
    // Past the 100 ms persist debounce. Bounded, as `resume.rs` does it, so a regression is
    // a failure in a quarter of a second rather than a test that never finishes.
    tokio::time::sleep(Duration::from_millis(250)).await;
    let after = h.pane_size(&pane);
    assert_eq!(
        (after.cols, after.rows),
        (before.cols, before.rows),
        "no pane resized, so no `PaneResized` wrote the file"
    );
    let text =
        std::fs::read_to_string(h.state_dir().join("state.json")).expect("state.json exists");
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["sidebar_open"], serde_json::json!(true), "{text}");
}

/// A call naming a client that is not attached changes nothing.
///
/// The state is the server's, so `set` would otherwise write it and mirror it into every
/// real client's view on behalf of a client that does not exist, and answer as though that
/// had worked. The guard refuses before anything is written, and this checks that nothing
/// was rather than trusting the error code to mean it.
#[tokio::test]
async fn a_call_naming_a_client_that_is_not_attached_changes_nothing() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let err = h
        .api("sidebar.show", serde_json::json!({"client": "c_9999"}))
        .await
        .unwrap_err();
    assert_eq!(err.code, domux_core::api::ErrorCode::NotFound, "{err}");
    assert!(!h.model().sidebar_open, "the remembered state is untouched");
    let f = h.frame(h.client.clone()).await;
    assert!(
        f.contains("proj › main"),
        "and the attached client keeps its top bar:\n{f}"
    );
}
