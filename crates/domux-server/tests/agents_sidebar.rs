//! The sidebar's Agents box under its Projects box, and the keys that cross between them
//! (domain model 3.6, interface spec 12.18, 12.27 and 12.29).
//!
//! Two rules shape the assertions here, the same two the agents overlay's tests carry. A
//! frame assertion that only asks whether a substring is somewhere on the screen passes for
//! many wrong layouts, so where the claim is about a place the test reads the row number and
//! the style dump. And an assertion that something is **absent** passes on a blank frame, so
//! every one of those stands beside a positive assertion that the thing it is looking for is
//! on some other surface of the same fixture.
//!
//! Row numbers here are screen rows, which is what the style dump's `r<n>` counts. See
//! `screen_rows`: counting `f.lines()` instead is off by the two header lines the frame
//! carries.

use domux_core::config::Config;
use domux_core::model::agent::AgentKind;
use domux_core::model::{Focus, Overlay, RegionKind};
use domux_server::testing::{row, Harness};
use serde_json::json;
use std::time::Duration;

const CLAUDE_STARTS: &str = r#"{"hook_event_name":"SessionStart","session_id":"c1"}"#;
const CLAUDE_WORKS: &str = r#"{"hook_event_name":"UserPromptSubmit","session_id":"c1"}"#;
const CLAUDE_ENDS: &str = r#"{"hook_event_name":"SessionEnd","session_id":"c1"}"#;

/// The accent and the unfocused border (interface spec 9.1), as the style dump spells them.
/// The literals rather than `theme::ACCENT` and `theme::SURFACE2`, so a token that moved
/// would fail here instead of moving the assertion with it.
const ACCENT: &str = "fg=#cba6f7";
const UNFOCUSED: &str = "fg=#585b70";

/// Columns `from` to `to` of a frame row, without the `|` framing. By column and not by
/// byte: the box drawing characters are three bytes each, so a byte slice cuts one in half.
fn cols(line: &str, from: usize, to: usize) -> String {
    line.chars().skip(1 + from).take(to - from + 1).collect()
}

/// The frame's screen rows, so an index into them is the row the style dump names.
///
/// `Grid::to_text` writes a size line and a cursor line before the rows and the style dump
/// after them, so `f.lines().position(..)` is two more than the screen row and cannot be
/// mixed with an `r<n>` lookup or with `testing::row`.
fn screen_rows(f: &str) -> Vec<&str> {
    f.lines().filter(|l| l.starts_with('|')).collect()
}

/// The screen row holding `needle`, or a panic naming the frame.
fn row_with(f: &str, needle: &str) -> usize {
    screen_rows(f)
        .iter()
        .position(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("no row holds {needle:?}:\n{f}"))
}

/// One working claude, with the sidebar showing. The agent is what puts a row in the Agents
/// box; without one the box draws its empty text and every cursor test would be vacuous.
async fn sidebar_with_an_agent(h: &mut Harness) {
    let pane = h.focused_pane(h.client.clone());
    h.report(pane.clone(), AgentKind::Claude, CLAUDE_STARTS)
        .await;
    h.report(pane, AgentKind::Claude, CLAUDE_WORKS).await;
    h.api("sidebar.show", json!({})).await.unwrap();
}

/// The frame once the Agents box is on it.
async fn sidebar_frame(h: &mut Harness) -> String {
    h.wait_for(
        h.client.clone(),
        |f| f.contains("┌ Agents"),
        Duration::from_secs(2),
    )
    .await
}

fn focus_of(h: &Harness) -> Focus {
    h.model()
        .client(&h.client)
        .expect("the client is attached")
        .focus
        .clone()
}

#[tokio::test]
async fn the_sidebar_holds_the_agents_box_under_the_projects_box_with_one_row_between() {
    let mut h = Harness::start(Config::default(), 120, 30).await;
    sidebar_with_an_agent(&mut h).await;
    let f = sidebar_frame(&mut h).await;
    let rows = screen_rows(&f);
    let projects_top = row_with(&f, "┌ Projects");
    let agents_top = row_with(&f, "┌ Agents");
    assert!(agents_top > projects_top, "Agents is the lower box:\n{f}");
    let projects_bottom = rows[..agents_top]
        .iter()
        .rposition(|l| l.starts_with("|└"))
        .expect("the Projects box's bottom border");
    assert_eq!(
        agents_top,
        projects_bottom + 2,
        "one row between the boxes (interface spec 12.18):\n{f}"
    );
    // The row between them is the sidebar's own background and nothing else: a gap that
    // still carried a border would satisfy the row arithmetic above.
    assert_eq!(
        cols(rows[projects_bottom + 1], 0, 37),
        " ".repeat(38),
        "the row between the boxes is empty:\n{f}"
    );
    // The whole geometry, not just the gap: a split that gave one box every row it could and
    // the other three would pass every assertion above.
    assert_eq!(
        (projects_top, projects_bottom, agents_top),
        (0, 13, 15),
        "half the column each on a 30 row screen:\n{f}"
    );
    assert_eq!(
        cols(row(&f, 28), 0, 37),
        format!("└{}┘", "─".repeat(36)),
        "the Agents box ends one row above the hint row:\n{f}"
    );
    assert!(
        f.contains("leader b hide · leader s search"),
        "the hint row is under both boxes:\n{f}"
    );
}

/// The sidebar's row is the two-line form: no tab on line two and no recap line at all
/// (interface spec 6.3, invariant 10).
///
/// The same fixture is read twice, once in the sidebar and once in the agents overlay, so
/// each "the sidebar does not show this" stands beside a frame that does show it. Without
/// that half the test would pass on a fixture that never had a tab or a recap to drop.
#[tokio::test]
async fn the_sidebar_rows_are_two_lines_with_no_tab_and_no_recap() {
    let mut h = Harness::start(Config::default(), 120, 30).await;
    h.api("tab.rename", json!({"name": "pr1"})).await.unwrap();
    let pane = h.focused_pane(h.client.clone());
    let dir = tempfile::tempdir().unwrap();
    let transcript = dir.path().join("s.jsonl");
    std::fs::write(
        &transcript,
        "{\"type\":\"ai-title\",\"aiTitle\":\"Session check cleanup\"}\n",
    )
    .unwrap();
    let payload = format!(
        "{{\"hook_event_name\":\"Stop\",\"session_id\":\"c1\",\"transcript_path\":\"{}\"}}",
        transcript.display()
    );
    h.report(pane, AgentKind::Claude, &payload).await;
    h.api("sidebar.show", json!({})).await.unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("● claude"),
            Duration::from_secs(2),
        )
        .await;
    let line = row_with(&f, "● claude");
    assert!(
        row(&f, line + 1).contains("proj › main"),
        "line two is the place:\n{f}"
    );
    assert!(
        !row(&f, line + 1).contains("pr1"),
        "and the sidebar drops the tab from it:\n{f}"
    );
    assert!(
        !f.contains("※"),
        "no recap in the sidebar (invariant 10):\n{f}"
    );
    assert!(
        !f.contains("Session check cleanup"),
        "not the recap's text either:\n{f}"
    );

    // The other half: the overlay's wider form keeps both, so the fixture did have a tab and
    // a recap for the sidebar to drop.
    h.api("agents.open", json!({})).await.unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("※"),
            Duration::from_secs(2),
        )
        .await;
    let line = row_with(&f, "● claude");
    assert!(
        row(&f, line + 1).contains("proj › main › pr1"),
        "the overlay's line two carries the tab:\n{f}"
    );
    assert!(
        row(&f, line + 2).contains("※ Session check cleanup"),
        "and the recap under it:\n{f}"
    );
}

#[tokio::test]
async fn tab_and_c_j_and_c_k_cross_between_the_two_boxes() {
    let mut h = Harness::start(Config::default(), 120, 30).await;
    sidebar_with_an_agent(&mut h).await;
    h.api("focus.region", json!({"region": "sidebar_projects"}))
        .await
        .unwrap();
    h.key(h.client.clone(), "Tab").await;
    h.frame(h.client.clone()).await;
    assert_eq!(focus_of(&h), Focus::Region(RegionKind::SidebarAgents));
    h.key(h.client.clone(), "Tab").await;
    h.frame(h.client.clone()).await;
    assert_eq!(
        focus_of(&h),
        Focus::Region(RegionKind::SidebarProjects),
        "Tab crosses back"
    );
    h.key(h.client.clone(), "C-j").await;
    h.frame(h.client.clone()).await;
    assert_eq!(focus_of(&h), Focus::Region(RegionKind::SidebarAgents));
    h.key(h.client.clone(), "C-k").await;
    h.frame(h.client.clone()).await;
    assert_eq!(focus_of(&h), Focus::Region(RegionKind::SidebarProjects));
}

/// The upper box has nothing above it and the lower box nothing below it, so those two
/// presses change nothing rather than wrapping round.
///
/// Its own test because `tab_and_c_j_and_c_k_cross_between_the_two_boxes` only ever presses
/// the direction that moves: an implementation that ignored the direction entirely and
/// swapped the boxes on every press would pass that one.
#[tokio::test]
async fn c_k_in_the_upper_box_and_c_j_in_the_lower_box_change_nothing() {
    let mut h = Harness::start(Config::default(), 120, 30).await;
    sidebar_with_an_agent(&mut h).await;
    h.api("focus.region", json!({"region": "sidebar_projects"}))
        .await
        .unwrap();
    h.key(h.client.clone(), "C-k").await;
    h.frame(h.client.clone()).await;
    assert_eq!(
        focus_of(&h),
        Focus::Region(RegionKind::SidebarProjects),
        "nothing is above the Projects box"
    );
    h.api("focus.region", json!({"region": "sidebar_agents"}))
        .await
        .unwrap();
    h.key(h.client.clone(), "C-j").await;
    h.frame(h.client.clone()).await;
    assert_eq!(
        focus_of(&h),
        Focus::Region(RegionKind::SidebarAgents),
        "and nothing below the Agents box"
    );
}

/// One visible focus target (principle 2): the box with the keys takes the accent and the
/// other takes the unfocused border.
///
/// Both halves are positive assertions on the same frame. "The Projects border is not the
/// accent" alone would pass on a frame that drew no Projects box at all.
#[tokio::test]
async fn only_the_focused_box_takes_the_accent() {
    let mut h = Harness::start(Config::default(), 120, 30).await;
    sidebar_with_an_agent(&mut h).await;
    h.api("focus.region", json!({"region": "sidebar_agents"}))
        .await
        .unwrap();
    let f = sidebar_frame(&mut h).await;
    let agents_top = row_with(&f, "┌ Agents");
    let projects_top = row_with(&f, "┌ Projects");
    assert!(
        f.contains(&format!("r{agents_top} c0-0 {ACCENT}\n")),
        "the Agents border is the accent while it has the keys:\n{f}"
    );
    assert!(
        f.contains(&format!("r{projects_top} c0-0 {UNFOCUSED}\n")),
        "and the Projects border is not:\n{f}"
    );

    h.api("focus.region", json!({"region": "sidebar_projects"}))
        .await
        .unwrap();
    let f = sidebar_frame(&mut h).await;
    assert!(
        f.contains(&format!("r{projects_top} c0-0 {ACCENT}\n")),
        "and the other way round:\n{f}"
    );
    assert!(
        f.contains(&format!("r{agents_top} c0-0 {UNFOCUSED}\n")),
        "and the other way round:\n{f}"
    );
}

/// `C-h` from a pane enters the box whose rows overlap the pane's most (interface spec
/// 12.29).
#[tokio::test]
async fn c_h_from_a_pane_enters_the_box_whose_rows_overlap_it_most() {
    let mut h = Harness::start(Config::default(), 120, 30).await;
    sidebar_with_an_agent(&mut h).await;
    h.api("pane.split", json!({"dir": "down"})).await.unwrap();
    let low = h.focused_pane(h.client.clone());
    h.key(h.client.clone(), "C-h").await;
    h.frame(h.client.clone()).await;
    assert_eq!(
        focus_of(&h),
        Focus::Region(RegionKind::SidebarAgents),
        "a pane low in the screen enters Agents (interface spec 12.29)"
    );
    h.key(h.client.clone(), "C-l").await;
    h.frame(h.client.clone()).await;
    assert_eq!(focus_of(&h), Focus::Pane(low.clone()));

    let high = h
        .model()
        .tab(&h.current_tab(h.client.clone()))
        .unwrap()
        .layout
        .pane_ids()[0]
        .clone();
    assert_ne!(high, low, "the split made two panes");
    h.api("pane.focus", json!({"pane": high.to_string()}))
        .await
        .unwrap();
    h.key(h.client.clone(), "C-h").await;
    h.frame(h.client.clone()).await;
    assert_eq!(focus_of(&h), Focus::Region(RegionKind::SidebarProjects));
    h.key(h.client.clone(), "C-l").await;
    h.frame(h.client.clone()).await;
    assert_eq!(
        focus_of(&h),
        Focus::Pane(high),
        "and `C-l` hands the keys back from either box"
    );
}

/// `C-h` from the one pane of an unsplit tab enters the Projects box, because the pane covers
/// both boxes equally and a tie goes to the upper one (interface spec 12.29, plan assumption
/// 25).
///
/// Its own test, and here rather than in `regions.rs`, because this is the case that decided
/// how `region_for_rows` measures. M2's tests reach the Projects box this way too and so cover
/// it by accident; this one covers it on purpose, with the Agents box drawn and holding a row,
/// so the tie is between two boxes that are both really on the screen rather than between a
/// box and an empty rectangle.
///
/// It is also the case that can never fire if the comparison is made against the two boxes as
/// drawn: the tab row takes screen row 0, so the pane misses the Projects box's first row and
/// leans to Agents by exactly one. See `render::sidebar::region_for_rows`.
#[tokio::test]
async fn c_h_from_the_one_pane_of_an_unsplit_tab_enters_projects_on_the_tie() {
    // Four heights and not one, two of each parity. The rule lived in the geometry of the
    // split before it lived in `region_for_rows`, and the first version of it answered
    // Projects on an even screen and Agents on an odd one; every fixture in this suite was
    // even, so nothing failed. `render::sidebar::tests::a_pane_filling_the_workpanel_enters_projects_at_every_height`
    // walks the whole range at the unit level, and these four carry the same claim through
    // the server, the keymap and the renderer.
    for rows in [24, 25, 30, 31] {
        let mut h = Harness::start(Config::default(), 120, rows).await;
        sidebar_with_an_agent(&mut h).await;
        let f = sidebar_frame(&mut h).await;
        assert!(
            row_with(&f, "● claude") > row_with(&f, "┌ Agents"),
            "at {rows} rows the Agents box has a row, so the tie is between two boxes that \
             are both drawn:\n{f}"
        );
        let pane = h.focused_pane(h.client.clone());
        assert_eq!(
            h.model()
                .tab(&h.current_tab(h.client.clone()))
                .unwrap()
                .layout
                .pane_ids()
                .len(),
            1,
            "one pane, so it fills the workpanel and covers both boxes"
        );
        h.key(h.client.clone(), "C-h").await;
        h.frame(h.client.clone()).await;
        assert_eq!(
            focus_of(&h),
            Focus::Region(RegionKind::SidebarProjects),
            "at {rows} rows"
        );
        h.key(h.client.clone(), "C-l").await;
        h.frame(h.client.clone()).await;
        assert_eq!(
            focus_of(&h),
            Focus::Pane(pane),
            "and `C-l` hands them back at {rows} rows"
        );
    }
}

/// Hiding the sidebar from the Agents box gives the keys back to the pane, the same as hiding
/// it from the Projects box (principle 2).
///
/// `sidebar.toggle` is a global binding, so `leader b` fires from inside a box at step 3 of
/// `input::route_key`, ahead of the box's own table. Without this the keys stayed in a region
/// the screen no longer draws and every key after it was swallowed by `list_key`, because
/// `api::list::surface` refuses once `sidebar_visible` is false: no key reached the pane until
/// the reader found `Esc`.
#[tokio::test]
async fn hiding_the_sidebar_from_either_box_gives_the_keys_back_to_the_pane() {
    for region in ["sidebar_projects", "sidebar_agents"] {
        let mut h = Harness::start(Config::default(), 120, 30).await;
        sidebar_with_an_agent(&mut h).await;
        let pane = h.focused_pane(h.client.clone());
        h.api("focus.region", json!({ "region": region }))
            .await
            .unwrap();
        h.key(h.client.clone(), "C-a").await;
        h.key(h.client.clone(), "b").await;
        h.frame(h.client.clone()).await;
        assert!(
            !h.model().sidebar_open,
            "leader b hid the sidebar from {region}"
        );
        assert_eq!(
            focus_of(&h),
            Focus::Pane(pane.clone()),
            "and the keys came back to the pane from {region}"
        );
        // The half that says the keys are usable again: a key reaches the pane rather than
        // being swallowed by the box's table.
        h.key(h.client.clone(), "x").await;
        h.frame(h.client.clone()).await;
        assert_eq!(
            h.pane_input(&pane),
            b"x".to_vec(),
            "and the next key reaches the pane from {region}"
        );
    }
}

/// The hint row names the keys of the row the cursor is on (interface spec 12.11), and an
/// exited row's key is the resume key the row itself has no room for (interface spec 12.6,
/// plan assumption 23).
#[tokio::test]
async fn the_hint_row_shows_the_cursor_rows_key_while_focus_is_in_the_agents_box() {
    let mut h = Harness::start(Config::default(), 120, 30).await;
    let pane = h.focused_pane(h.client.clone());
    sidebar_with_an_agent(&mut h).await;
    h.api("focus.region", json!({"region": "sidebar_agents"}))
        .await
        .unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("⏎ open"),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(
        cols(row(&f, 29), 0, 37),
        format!(" ⏎ open · ? more{}", " ".repeat(22)),
        "a live row offers open (interface spec 12.11):\n{f}"
    );
    h.report(pane, AgentKind::Claude, CLAUDE_ENDS).await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("⏎ resume"),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(
        cols(row(&f, 29), 0, 37),
        format!(" ⏎ resume · ? more{}", " ".repeat(20)),
        "an exited row offers resume in the hint row (interface spec 12.6):\n{f}"
    );
    // The row itself stops after the age: the key is the hint row's, not the row's
    // (plan assumption 23). Beside a positive assertion, so a blank frame cannot pass it.
    let line = row_with(&f, "● claude");
    assert!(
        row(&f, line).contains("exited"),
        "the row says it exited:\n{f}"
    );
    assert!(
        !row(&f, line).contains("resume"),
        "and does not carry the key too:\n{f}"
    );
}

#[tokio::test]
async fn an_empty_agents_box_still_draws_its_border_and_says_why_it_is_empty() {
    let mut h = Harness::start(Config::default(), 120, 30).await;
    h.api("sidebar.show", json!({})).await.unwrap();
    let f = sidebar_frame(&mut h).await;
    let agents_top = row_with(&f, "┌ Agents");
    assert_eq!(agents_top, 15, "the border is always there (principle 14)");
    // Wrapped over two rows and not cut: 36 columns inside the border and 47 cells of text,
    // and the action is the second half of the sentence (principle 9, plan assumption 22).
    assert_eq!(
        cols(row(&f, agents_top + 1), 1, 36),
        "No agents yet. Start claude or codex",
        "{f}"
    );
    assert_eq!(
        cols(row(&f, agents_top + 2), 1, 36),
        "in a pane.                          ",
        "the action is wrapped rather than dropped:\n{f}"
    );
}

/// Entering the Agents box puts the cursor on its first row, so the fill and the hint row
/// have a row to read (interface spec 12.32, plan assumption 26).
///
/// A model assertion and not a frame one: the fill is a background colour, and a frame test
/// for it would pass just as well with the cursor on a different agent.
#[tokio::test]
async fn entering_the_agents_box_puts_the_cursor_on_the_first_row() {
    let mut h = Harness::start(Config::default(), 120, 30).await;
    sidebar_with_an_agent(&mut h).await;
    let first = h.agents().await[0].id.clone();
    assert_eq!(
        h.model().client(&h.client).unwrap().agents_cursor,
        None,
        "nothing has put a cursor in the box yet"
    );
    h.api("focus.region", json!({"region": "sidebar_agents"}))
        .await
        .unwrap();
    assert_eq!(
        h.model().client(&h.client).unwrap().agents_cursor,
        Some(first.clone())
    );
    // The keys move it, so the box is a list and not one row that cannot be left. A second
    // agent, so `list.down` has somewhere to go.
    h.api("pane.split", json!({"dir": "right"})).await.unwrap();
    let second_pane = h.focused_pane(h.client.clone());
    h.report(
        second_pane,
        AgentKind::Codex,
        r#"{"hook_event_name":"SessionStart","session_id":"x1"}"#,
    )
    .await;
    h.api("focus.region", json!({"region": "sidebar_agents"}))
        .await
        .unwrap();
    h.key(h.client.clone(), "j").await;
    h.frame(h.client.clone()).await;
    let moved = h.model().client(&h.client).unwrap().agents_cursor.clone();
    assert!(
        moved.is_some() && moved != Some(first),
        "`j` moves the cursor to the other agent, not off the list: {moved:?}"
    );
}

/// The cursor keys measure the sidebar's Agents box by the rows it draws, which are the
/// two-line ones: a list that fits the box does not scroll.
///
/// Four agents each carrying a recap. In the sidebar's form that is eleven lines in a box
/// twelve rows deep, so nothing scrolls; in the agents overlay's three-line form the same
/// four agents are fifteen lines and would. The row form and the rectangle are the only two
/// things `api::list` has to take from the sidebar rather than the overlay, and this is where
/// both show: with no recap both forms are two lines and a box measured either way answers
/// the same.
#[tokio::test]
async fn the_cursor_keys_measure_the_sidebars_agents_box_by_the_rows_it_draws() {
    let mut h = Harness::start(Config::default(), 120, 30).await;
    let dir = tempfile::tempdir().unwrap();
    for n in 0..4 {
        if n > 0 {
            h.api("pane.split", json!({"dir": "down"})).await.unwrap();
        }
        let pane = h.focused_pane(h.client.clone());
        let transcript = dir.path().join(format!("s{n}.jsonl"));
        std::fs::write(
            &transcript,
            "{\"type\":\"ai-title\",\"aiTitle\":\"Session check cleanup\"}\n",
        )
        .unwrap();
        let payload = format!(
            "{{\"hook_event_name\":\"Stop\",\"session_id\":\"c{n}\",\"transcript_path\":\"{}\"}}",
            transcript.display()
        );
        h.report(pane, AgentKind::Claude, &payload).await;
    }
    h.api("sidebar.show", json!({})).await.unwrap();
    let f = sidebar_frame(&mut h).await;
    let agents_top = row_with(&f, "┌ Agents");
    assert_eq!(
        h.agents().await.len(),
        4,
        "four records, which is what makes the two row forms different heights"
    );

    h.api("focus.region", json!({"region": "sidebar_agents"}))
        .await
        .unwrap();
    for _ in 0..3 {
        h.key(h.client.clone(), "j").await;
    }
    let f = h.frame(h.client.clone()).await;
    assert_eq!(
        h.model().client(&h.client).unwrap().agents_scroll,
        0,
        "the whole list fits the box, so the cursor never pushed it:\n{f}"
    );
    assert!(
        row(&f, agents_top + 1).contains("● claude"),
        "and the first agent is still the box's top row:\n{f}"
    );
}

/// Enter in the sidebar's Agents box reaches the same handler the agents overlay's Enter
/// does, so it lands the reader in the agent's pane (interface spec 6.8).
///
/// The sidebar is a third surface for `list.activate`; without its arm in `api::list` the key
/// would refuse in a box whose own hint row offers `⏎ open`.
#[tokio::test]
async fn enter_in_the_sidebars_agents_box_switches_to_the_agents_pane() {
    let mut h = Harness::start(Config::default(), 120, 30).await;
    let first = h.focused_pane(h.client.clone());
    h.api("pane.split", json!({"dir": "right"})).await.unwrap();
    let second = h.focused_pane(h.client.clone());
    h.report(second.clone(), AgentKind::Claude, CLAUDE_STARTS)
        .await;
    h.api("sidebar.show", json!({})).await.unwrap();
    sidebar_frame(&mut h).await;
    h.api("pane.focus", json!({"pane": first.to_string()}))
        .await
        .unwrap();
    h.api("focus.region", json!({"region": "sidebar_agents"}))
        .await
        .unwrap();
    h.key(h.client.clone(), "Enter").await;
    h.frame(h.client.clone()).await;
    assert_eq!(
        focus_of(&h),
        Focus::Pane(second),
        "Enter left the box for the pane the agent is in"
    );
}

/// And Enter on an **exited** row in the sidebar's box resumes it, the same as in the agents
/// overlay, with the result in the sidebar's hint row rather than the overlay's footer.
///
/// The sidebar is the third surface `list.activate` serves, and the test above it only ever
/// activates a live row: that one would pass with the exited arm broken. This is the other half,
/// and it reads the pane to prove the line was typed rather than trusting the row.
#[tokio::test]
async fn enter_on_an_exited_row_in_the_sidebars_agents_box_resumes_it() {
    let mut h = Harness::start(Config::default(), 120, 30).await;
    let first = h.focused_pane(h.client.clone());
    h.api("pane.split", json!({"dir": "right"})).await.unwrap();
    let second = h.focused_pane(h.client.clone());
    h.report(second.clone(), AgentKind::Claude, CLAUDE_STARTS)
        .await;
    h.report(second.clone(), AgentKind::Claude, CLAUDE_ENDS)
        .await;
    h.api("sidebar.show", json!({})).await.unwrap();
    sidebar_frame(&mut h).await;
    h.api("pane.focus", json!({"pane": first.to_string()}))
        .await
        .unwrap();
    h.api("focus.region", json!({"region": "sidebar_agents"}))
        .await
        .unwrap();

    h.key(h.client.clone(), "Enter").await;

    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("Resumed"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        f.contains("Resumed claude in "),
        "the sidebar's hint row carries the result (principle 8):\n{f}"
    );
    assert_eq!(
        focus_of(&h),
        Focus::Region(RegionKind::SidebarAgents),
        "and the keys stayed in the box, because resume is not a navigation"
    );
    assert!(
        String::from_utf8(h.pane_input(&second))
            .unwrap()
            .contains("claude --resume 'c1'"),
        "the line was typed into the agent's own pane"
    );
}

/// A cursor left on a record that has gone starts again at the first row rather than naming
/// a row the box is not showing (principle 2).
///
/// Its own test because `entering_the_agents_box_puts_the_cursor_on_the_first_row` only ever
/// enters a box whose cursor is empty, where keeping whatever was held and replacing it look
/// identical.
#[tokio::test]
async fn a_cursor_left_on_a_record_that_is_gone_starts_again_at_the_first_row() {
    let mut h = Harness::start(Config::default(), 120, 30).await;
    let first_pane = h.focused_pane(h.client.clone());
    h.api("pane.split", json!({"dir": "right"})).await.unwrap();
    let second_pane = h.focused_pane(h.client.clone());
    h.report(first_pane.clone(), AgentKind::Claude, CLAUDE_STARTS)
        .await;
    h.report(first_pane, AgentKind::Claude, CLAUDE_ENDS).await;
    h.report(
        second_pane,
        AgentKind::Codex,
        r#"{"hook_event_name":"SessionStart","session_id":"x1"}"#,
    )
    .await;
    h.api("sidebar.show", json!({})).await.unwrap();
    sidebar_frame(&mut h).await;

    // The exited claude sorts last, so the cursor has to be walked onto it.
    let exited = h
        .agents()
        .await
        .into_iter()
        .find(|a| a.kind == AgentKind::Claude)
        .expect("the claude record")
        .id;
    h.api("focus.region", json!({"region": "sidebar_agents"}))
        .await
        .unwrap();
    h.key(h.client.clone(), "j").await;
    h.frame(h.client.clone()).await;
    assert_eq!(
        h.model().client(&h.client).unwrap().agents_cursor,
        Some(exited.clone()),
        "the cursor is on the record this test is about to remove"
    );

    h.api("focus.pane", json!({})).await.unwrap();
    h.api("agent.dismiss", json!({"agent": exited.to_string()}))
        .await
        .unwrap();
    h.api("focus.region", json!({"region": "sidebar_agents"}))
        .await
        .unwrap();
    let cursor = h.model().client(&h.client).unwrap().agents_cursor.clone();
    assert!(
        cursor.is_some() && cursor != Some(exited),
        "the cursor moved to the row that is still there: {cursor:?}"
    );
}

/// `?` over a box comes back to the box, and the sidebar's Agents box is a box
/// (`ClientView::focus_returning_from_overlay`).
#[tokio::test]
async fn help_over_the_agents_box_gives_the_keys_back_to_it() {
    let mut h = Harness::start(Config::default(), 120, 30).await;
    sidebar_with_an_agent(&mut h).await;
    h.api("focus.region", json!({"region": "sidebar_agents"}))
        .await
        .unwrap();
    h.key(h.client.clone(), "?").await;
    h.frame(h.client.clone()).await;
    assert_eq!(
        h.model().client(&h.client).unwrap().overlay,
        Some(Overlay::Help)
    );
    assert_eq!(
        focus_of(&h),
        Focus::Region(RegionKind::SidebarAgents),
        "the box keeps its region while the help is over it"
    );
    h.key(h.client.clone(), "Esc").await;
    h.frame(h.client.clone()).await;
    assert_eq!(h.model().client(&h.client).unwrap().overlay, None);
    assert_eq!(
        focus_of(&h),
        Focus::Region(RegionKind::SidebarAgents),
        "and gets them back when it closes"
    );
}

/// Tab in an overlay that holds one box changes nothing (interface spec 12.27): there is no
/// second box in it to cross to.
#[tokio::test]
async fn tab_in_a_one_box_overlay_changes_nothing() {
    let mut h = Harness::start(Config::default(), 120, 30).await;
    sidebar_with_an_agent(&mut h).await;
    h.api("agents.open", json!({})).await.unwrap();
    h.frame(h.client.clone()).await;
    assert_eq!(focus_of(&h), Focus::Region(RegionKind::AgentsOverlay));
    h.key(h.client.clone(), "Tab").await;
    h.frame(h.client.clone()).await;
    assert_eq!(focus_of(&h), Focus::Region(RegionKind::AgentsOverlay));
    assert_eq!(
        h.model().client(&h.client).unwrap().overlay,
        Some(Overlay::Agents),
        "and the overlay is still open"
    );
}

/// A region nothing on the screen marks is refused rather than entered (principle 2).
#[tokio::test]
async fn focus_region_refuses_the_agents_box_while_the_sidebar_is_hidden() {
    let mut h = Harness::start(Config::default(), 120, 30).await;
    let err = h
        .api("focus.region", json!({"region": "sidebar_agents"}))
        .await
        .expect_err("the sidebar is hidden");
    assert_eq!(err.code, domux_core::api::ErrorCode::Refused, "{err}");
    assert!(err.message.contains("sidebar"), "{err}");
    h.api("sidebar.show", json!({})).await.unwrap();
    h.api("focus.region", json!({"region": "sidebar_agents"}))
        .await
        .expect("and it is entered once the sidebar shows");
}
