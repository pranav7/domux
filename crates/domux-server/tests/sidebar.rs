//! The sidebar's geometry and API, the tab row on the panes, and the remembered state.
//!
//! The waits below are on the full-width top bar going away rather than on the box arriving,
//! because `on_the_panes` states the layout positively: a client that has drawn nothing
//! satisfies "no top bar" and would pass a wait written the other way round.

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

/// `leader b` reaches `sidebar.toggle`, the full-width top bar goes, the sidebar takes the
/// left 38 columns, and the tab row moves onto the panes with the right end's pieces at its
/// end (interface spec 4.2).
#[tokio::test]
async fn leader_b_replaces_the_top_bar_with_the_sidebar_and_puts_the_tab_row_on_the_panes() {
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
        "  1 │ + │                                                     14:32   Fri 4 Sep ● ",
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
    // `cols(line, 0, 37)` takes 38 cells, the sidebar's whole width.
    assert_eq!(
        cols(row(&f, 0), 0, 37),
        "┌ Navigator ─────────────────────────┐",
        "{f}"
    );
    assert_eq!(
        cols(row(&f, 1), 0, 37),
        "│ PROJ ───────────────────────────── │"
    );
    assert_eq!(
        cols(row(&f, 2), 0, 37),
        "│   main                             │"
    );
    assert_eq!(
        cols(row(&f, 22), 0, 37),
        "│ leader b hide · leader s search    │",
        "the hint row is the box's own footer now, inside its border:\n{f}"
    );
    assert_eq!(
        cols(row(&f, 23), 0, 37),
        "└────────────────────────────────────┘",
        "so the box's bottom border reaches the same row the workpanel's does:\n{f}"
    );
    // The fill marks the row the keys act on, which with focus in a pane is the workspace
    // this client is in (domain model, section 3.3). `rows` is given the key and hands back
    // where it put the fill, so the band and the brightening are one answer: the band runs
    // the row's whole inner width and the text on it is brightened to `text`.
    assert!(
        f.contains("r2 c2-7 fg=#cdd6f4 bg=#313244"),
        "`main` is the current workspace, so its row carries the fill, its indent \
         included:\n{f}"
    );
    assert!(
        f.contains("r2 c1-1 bg=#313244") && f.contains("r2 c8-36 bg=#313244"),
        "and the band runs to the border rather than stopping at the text:\n{f}"
    );
    assert!(
        f.contains("r1 c2-6 bold fg=#7f849c\n"),
        "the project header is not a row the keys can act on, so it takes no fill:\n{f}"
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
    // Asking for it at this width gets it: `leader b` shows the sidebar at any width, and
    // the auto-hide is an override the reader can override in turn (interface spec 12.1).
    let result = h.api("sidebar.show", serde_json::json!({})).await.unwrap();
    assert_eq!(
        (result["open"].clone(), result["visible"].clone()),
        (serde_json::json!(true), serde_json::json!(true))
    );
    let f = h
        .wait_for(h.client.clone(), on_the_panes, Duration::from_secs(2))
        .await;
    assert_eq!(
        cols(row(&f, 0), 0, 37),
        "┌ Navigator ─────────────────────────┐",
        "the sidebar the reader asked for, on a screen that would not show it alone:\n{f}"
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
        "  1 │ + │                                                           C-a  ? keys ● ",
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
    // Named, not left to `most_recent_client`: an explicit ask from the 81-column client
    // would force the sidebar onto it, which resizes the panes and destroys the premise.
    let me = serde_json::json!({ "client": h.client.to_string() });
    h.api("sidebar.show", me.clone()).await.unwrap();
    h.wait_for(h.client.clone(), on_the_panes, Duration::from_secs(2))
        .await;
    let pane = h.focused_pane(h.client.clone());
    let before = h.pane_size(&pane);
    assert_eq!((before.cols, before.rows), (79, 21));
    h.api("sidebar.hide", me).await.unwrap();
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
/// missing mid-run write leaves a stale `false` here rather than no file at all.
#[tokio::test]
async fn the_sidebar_reaches_the_state_file_while_the_server_is_still_running() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let _second = h.attach(81, 24).await;
    let pane = h.focused_pane(h.client.clone());
    let before = h.pane_size(&pane);
    // Named for the same reason as the redraw test above: an ask from the narrow client
    // would force the sidebar onto it and resize the panes.
    h.api(
        "sidebar.show",
        serde_json::json!({ "client": h.client.to_string() }),
    )
    .await
    .unwrap();
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

/// The narrow-screen override is this client's own.
///
/// The remembered state is the server's, so both clients below want the sidebar. The
/// auto-hide is each client's own, so the one that asked for it at this width gets it and
/// the one that did not stays hidden (interface spec 12.1). One bit could not do this: it
/// would either show the sidebar on both or on neither.
#[tokio::test]
async fn asking_for_the_sidebar_on_a_narrow_screen_leaves_another_narrow_client_hidden() {
    let mut h = Harness::start(Config::default(), 119, 24).await;
    h.api(
        "sidebar.show",
        serde_json::json!({ "client": h.client.to_string() }),
    )
    .await
    .unwrap();
    // Attached after the show, deliberately. A client attaching onto a sidebar that is
    // already remembered open must auto-hide on a narrow screen, and every fixture that
    // attaches first hides that: `set` re-assigns `sidebar_forced` for every attached
    // client, so it overwrites whatever the attach default was.
    let other = h.attach(119, 24).await;
    let f = h
        .wait_for(h.client.clone(), on_the_panes, Duration::from_secs(2))
        .await;
    assert!(
        f.contains("┌ Navigator"),
        "the client that asked gets it at 119 columns:\n{f}"
    );
    let g = h.frame(other).await;
    assert!(
        g.contains("proj › main"),
        "the client that did not ask is still auto-hidden:\n{g}"
    );
    assert!(
        h.model().sidebar_open,
        "and the remembered state is open for both of them"
    );
}

/// A client that did not ask does not carry an override across another client's show.
///
/// Named for what it proves rather than for the line that motivated it. The `open &&` in
/// `api::sidebar::set` is not what makes this pass - the `asked` gate on the second show is,
/// and removing `open &&` leaves this green. That term is equivalent and `api/sidebar.rs`
/// says why; this is the property that is real: after the wide client asks, the narrow one
/// is auto-hidden because it never asked, whatever it was doing before.
#[tokio::test]
async fn a_client_that_did_not_ask_keeps_no_override_across_another_clients_show() {
    let mut h = Harness::start(Config::default(), 119, 24).await;
    let me = serde_json::json!({ "client": h.client.to_string() });
    h.api("sidebar.show", me.clone()).await.unwrap();
    h.wait_for(h.client.clone(), on_the_panes, Duration::from_secs(2))
        .await;
    h.api("sidebar.hide", me).await.unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains("proj › main"),
        Duration::from_secs(2),
    )
    .await;
    let wide = h.attach(120, 24).await;
    h.api(
        "sidebar.show",
        serde_json::json!({ "client": wide.to_string() }),
    )
    .await
    .unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("proj › main"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        f.contains("proj › main"),
        "the narrow client is auto-hidden again, not still forced from before:\n{f}"
    );
    let g = h.wait_for(wide, on_the_panes, Duration::from_secs(2)).await;
    assert!(
        g.contains("┌ Navigator"),
        "while the wide client that asked has it:\n{g}"
    );
}

/// The event says which client and which way it went.
///
/// Persistence keys off the event's class, not its contents, so every other test here passes
/// with the payload inverted: they observe that an event was raised, never that it said the
/// right thing. A subscriber to `sidebar.*` - `domux api events`, and every later task that
/// watches for structural changes - would be told the sidebar closed when it opened, which is
/// a fabricated fact rather than a missing one (principle 4).
#[tokio::test]
async fn the_sidebar_event_carries_the_client_and_the_state_it_moved_to() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixStream;
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let stream = UnixStream::connect(h.socket_path()).await.unwrap();
    let (r, mut w) = stream.into_split();
    w.write_all(
        b"{\"id\":1,\"method\":\"events.subscribe\",\"params\":{\"filter\":[\"sidebar.*\"]}}\n",
    )
    .await
    .unwrap();
    let mut lines = BufReader::new(r).lines();
    // Every read is bounded: a subscription that never answers must fail this test in
    // seconds rather than stall the suite, which a test binary cannot report.
    let ack = tokio::time::timeout(Duration::from_secs(2), lines.next_line())
        .await
        .expect("events.subscribe was acknowledged")
        .unwrap()
        .unwrap();
    assert!(
        serde_json::from_str::<serde_json::Value>(&ack).unwrap()["error"].is_null(),
        "{ack}"
    );
    let asked = h.client.to_string();
    h.api(
        "sidebar.show",
        serde_json::json!({ "client": asked.clone() }),
    )
    .await
    .unwrap();
    let line = tokio::time::timeout(Duration::from_secs(2), lines.next_line())
        .await
        .expect("the sidebar event arrived")
        .unwrap()
        .unwrap();
    let event: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(event["event"], "sidebar.toggled", "{event}");
    assert_eq!(
        event["open"],
        serde_json::json!(true),
        "it opened, so the event says so: {event}"
    );
    assert_eq!(
        event["client"],
        serde_json::json!(asked),
        "and names the client that asked: {event}"
    );
}

/// The current tab's cell while an overlay owns the keys: bold text, no accent fill, and no
/// background of its own on the row over the panes.
///
/// `cell_for` has three arms and each needs its own fixture. Keys in a pane gives the accent
/// fill, a tab that is not the current one gives the dim arm, and only an open overlay gives
/// this one. `overlay::frame` starts at row 1, so the tab row stays visible under it and a
/// mantle band here would draw across the top of the panes.
#[tokio::test]
async fn the_current_tab_carries_no_background_while_an_overlay_owns_the_keys() {
    let mut h = Harness::start(Config::default(), 120, 34).await;
    h.api("sidebar.show", serde_json::json!({})).await.unwrap();
    h.wait_for(h.client.clone(), on_the_panes, Duration::from_secs(2))
        .await;
    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), "?").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("Keys"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        f.contains("r0 c39-41 bold fg=#cdd6f4\n"),
        "the current tab is bold text on nothing: not the accent fill, and not a mantle band:\n{f}"
    );
}
