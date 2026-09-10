//! The agents overlay: one overlay holding the Agents box and a footer, under `leader a`.
//!
//! Two rules shape the assertions here. A frame assertion that only asks whether a substring
//! is somewhere on the screen passes for many wrong layouts, so where the claim is about a
//! place the test reads the row and the column, and where it is about the fill it reads the
//! frame's own style dump. And an assertion that something is **absent** passes on a blank
//! frame, so every one of those stands beside a positive assertion on the same frame.

use domux_core::config::Config;
use domux_core::ids::PaneId;
use domux_core::model::agent::{AgentKind, AgentState};
use domux_core::model::{Focus, Overlay, RegionKind};
use domux_server::testing::{row, Harness};
use serde_json::json;
use std::time::Duration;

const CLAUDE_STARTS: &str = r#"{"hook_event_name":"SessionStart","session_id":"c1"}"#;
const CLAUDE_WORKS: &str = r#"{"hook_event_name":"UserPromptSubmit","session_id":"c1"}"#;
const CLAUDE_ENDS: &str = r#"{"hook_event_name":"SessionEnd","session_id":"c1"}"#;
const CODEX_STARTS: &str = r#"{"hook_event_name":"SessionStart","session_id":"x1"}"#;
/// Codex's waiting event (`agents::hooks::parse_codex`).
const CODEX_WAITS: &str = r#"{"hook_event_name":"PermissionRequest","session_id":"x1"}"#;

/// The fill's background (interface spec 9.1), as the frame's style dump spells it.
const FILL: &str = "bg=#313244";
/// The box's left border column at the 100 column geometry, and the first column inside it.
const BOX_LEFT: usize = 12;
const BOX_INNER: usize = BOX_LEFT + 1;

/// A working claude in the first pane and a waiting codex in a second, so the box has two
/// rows and the sort order has something to put in front.
async fn two_agents(h: &mut Harness) -> (PaneId, PaneId) {
    let first = h.focused_pane(h.client.clone());
    h.api("pane.split", json!({"dir": "right"})).await.unwrap();
    let second = h.focused_pane(h.client.clone());
    h.report(first.clone(), AgentKind::Claude, CLAUDE_STARTS)
        .await;
    h.report(first.clone(), AgentKind::Claude, CLAUDE_WORKS)
        .await;
    h.report(second.clone(), AgentKind::Codex, CODEX_STARTS)
        .await;
    h.report(second.clone(), AgentKind::Codex, CODEX_WAITS)
        .await;
    (first, second)
}

/// One claude that started and ended. The exited row is the only row that names the resume
/// key, so a fixture without one leaves that half of the row untested.
async fn one_exited_agent(h: &mut Harness) {
    let pane = h.focused_pane(h.client.clone());
    h.report(pane.clone(), AgentKind::Claude, CLAUDE_STARTS)
        .await;
    h.report(pane.clone(), AgentKind::Claude, CLAUDE_ENDS).await;
}

async fn open_overlay(h: &mut Harness) -> String {
    h.api("agents.open", json!({})).await.unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains("┌ Agents"),
        Duration::from_secs(2),
    )
    .await
}

/// The number of the frame's `|...|` row holding `needle`, which is the grid's own row: the
/// frame text has two header lines above its rows, so counting `lines()` would be two out.
fn row_holding(frame: &str, needle: &str) -> usize {
    frame
        .lines()
        .filter(|l| l.starts_with('|'))
        .position(|l| l.contains(needle))
        .unwrap_or_else(|| panic!("no row holds {needle:?} in:\n{frame}"))
}

/// The column a character sits in on one `|...|` row. `str::find` answers in bytes and every
/// glyph the box's border is drawn with takes three of them, so a byte offset is not a column.
/// The leading `|` comes off, and so this is the grid's own column.
fn col_of(line: &str, ch: char) -> usize {
    line.chars()
        .position(|c| c == ch)
        .unwrap_or_else(|| panic!("no {ch:?} in {line:?}"))
        - 1
}

fn last_col_of(line: &str, ch: char) -> usize {
    line.chars()
        .rev()
        .position(|c| c == ch)
        .map(|from_end| line.chars().count() - 1 - from_end)
        .unwrap_or_else(|| panic!("no {ch:?} in {line:?}"))
        - 1
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
        let (from, to): (usize, usize) = (from.parse().unwrap(), to.parse().unwrap());
        if (from..=to).contains(&x) {
            return style.to_string();
        }
    }
    panic!("no style covers r{y} c{x} in:\n{frame}")
}

#[tokio::test]
async fn leader_a_opens_one_box_with_a_footer_and_the_cursor_on_the_first_row() {
    let mut h = Harness::start(Config::default(), 100, 24).await;
    two_agents(&mut h).await;
    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), "a").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("┌ Agents"),
            Duration::from_secs(2),
        )
        .await;
    assert!(f.contains("┌ Agents"), "one box, titled Agents:\n{f}");
    assert!(
        !f.contains("┌ Projects"),
        "the agents overlay holds the Agents box alone:\n{f}"
    );
    assert!(
        f.contains("⏎ open · / filter · esc close"),
        "the footer:\n{f}"
    );
    let view = h.model().client(&h.client).unwrap().clone();
    assert_eq!(view.overlay, Some(Overlay::Agents));
    assert_eq!(view.focus, Focus::Region(RegionKind::AgentsOverlay));
    let agents = h.agents().await;
    assert_eq!(
        view.agents_cursor.as_ref(),
        Some(&agents[0].id),
        "the cursor starts on the first row (12.32)"
    );
    // And the frame marks that row: the fill is on the cursor's line and on no other
    // (interface spec 5.3), so a cursor the box never read would fail here too.
    let codex = row_holding(&f, "● codex");
    let claude = row_holding(&f, "● claude");
    assert!(
        style_at(&f, codex, BOX_INNER).contains(FILL),
        "the fill is on the cursor's row:\n{f}"
    );
    assert!(
        !style_at(&f, claude, BOX_INNER).contains(FILL),
        "and on no other row:\n{f}"
    );
}

#[tokio::test]
async fn the_rows_are_the_three_line_form_with_the_waiting_agent_first() {
    let mut h = Harness::start(Config::default(), 100, 24).await;
    two_agents(&mut h).await;
    let f = open_overlay(&mut h).await;
    let codex_line = row_holding(&f, "● codex");
    let claude_line = row_holding(&f, "● claude");
    assert!(codex_line < claude_line, "waiting sorts first:\n{f}");
    // Line 2 is the place **with its tab**, which is what tells the overlay's row form from
    // the sidebar's: `project › workspace › tab` has two separators where the sidebar's
    // `project › workspace` has one.
    assert_eq!(
        row(&f, codex_line + 1).matches(" › ").count(),
        2,
        "line 2 is the place with its tab:\n{f}"
    );
    // One blank row between the two agents and nothing else between them (interface spec
    // 6.2): two lines of codex, then the blank, then claude.
    assert_eq!(
        claude_line - codex_line,
        3,
        "one blank row between the agents:\n{f}"
    );
}

#[tokio::test]
async fn j_and_k_move_the_cursor_and_enter_opens_the_agents_pane_and_clears_unseen() {
    let mut h = Harness::start(Config::default(), 100, 24).await;
    let (first, _) = two_agents(&mut h).await;
    let waiting = h.agents().await[0].id.clone();
    // Away from the codex's own pane, so the dot it carries is still there to be cleared.
    h.api("pane.focus", json!({"pane": first.to_string()}))
        .await
        .unwrap();
    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), "a").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("┌ Agents"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        h.agents().await.iter().any(|a| a.id == waiting && a.unseen),
        "the waiting codex still carries its dot before Enter:\n{f}"
    );
    h.key(h.client.clone(), "j").await;
    let f = h.frame(h.client.clone()).await;
    assert_ne!(
        h.model().client(&h.client).unwrap().agents_cursor.as_ref(),
        Some(&waiting)
    );
    // The fill went with the cursor, so the reader can see which row `Enter` will act on.
    assert!(
        style_at(&f, row_holding(&f, "● claude"), BOX_INNER).contains(FILL),
        "the fill followed the cursor:\n{f}"
    );
    h.key(h.client.clone(), "k").await;
    h.frame(h.client.clone()).await;
    assert_eq!(
        h.model().client(&h.client).unwrap().agents_cursor.as_ref(),
        Some(&waiting)
    );
    h.key(h.client.clone(), "Enter").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| !f.contains("┌ Agents"),
            Duration::from_secs(2),
        )
        .await;
    let a = h
        .agents()
        .await
        .into_iter()
        .find(|x| x.id == waiting)
        .expect("the record is still listed");
    assert!(!a.unseen, "Enter cleared unseen");
    assert_eq!(
        h.focused_pane(h.client.clone()),
        a.pane.clone().expect("a live record has a pane"),
        "and focused its pane"
    );
    assert!(!f.contains("┌ Agents"), "the overlay closed:\n{f}");
    assert_eq!(
        h.model().client(&h.client).unwrap().overlay,
        None,
        "and the model says so too"
    );
}

#[tokio::test]
async fn esc_closes_the_overlay_and_gives_focus_back_to_the_pane() {
    let mut h = Harness::start(Config::default(), 100, 24).await;
    two_agents(&mut h).await;
    let before = h.focused_pane(h.client.clone());
    open_overlay(&mut h).await;
    h.key(h.client.clone(), "Esc").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| !f.contains("┌ Agents"),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(
        h.model().client(&h.client).unwrap().focus,
        Focus::Pane(before),
        "{f}"
    );
}

/// The API form of the same close (principle 12), so the overlay has a way out that is not a
/// keystroke and the key and the call reach one handler.
#[tokio::test]
async fn agents_close_closes_the_overlay_and_gives_focus_back_to_the_pane() {
    let mut h = Harness::start(Config::default(), 100, 24).await;
    two_agents(&mut h).await;
    let before = h.focused_pane(h.client.clone());
    open_overlay(&mut h).await;
    h.api("agents.close", json!({})).await.unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| !f.contains("┌ Agents"),
            Duration::from_secs(2),
        )
        .await;
    let view = h.model().client(&h.client).unwrap().clone();
    assert_eq!(view.overlay, None, "{f}");
    assert_eq!(view.focus, Focus::Pane(before));
    // Closing what is already closed is not an error, as `switcher.close` is not.
    h.api("agents.close", json!({})).await.unwrap();
}

#[tokio::test]
async fn slash_filters_the_box_and_esc_restores_the_footer() {
    let mut h = Harness::start(Config::default(), 100, 24).await;
    two_agents(&mut h).await;
    open_overlay(&mut h).await;
    h.key(h.client.clone(), "/").await;
    h.type_text(h.client.clone(), "codex").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("Filter › codex"),
            Duration::from_secs(2),
        )
        .await;
    assert!(f.contains("● codex"), "{f}");
    assert!(
        !f.contains("● claude"),
        "the claude row was filtered out:\n{f}"
    );
    assert!(
        f.contains("esc clear"),
        "the footer became the filter input:\n{f}"
    );
    h.key(h.client.clone(), "Esc").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("⏎ open"),
            Duration::from_secs(2),
        )
        .await;
    assert!(f.contains("● claude"), "esc cleared the filter:\n{f}");
    assert!(
        f.contains("┌ Agents"),
        "the first esc clears the filter and does not close the overlay:\n{f}"
    );
}

#[tokio::test]
async fn a_filter_with_no_matches_says_so_and_offers_the_key_that_clears_it() {
    let mut h = Harness::start(Config::default(), 100, 24).await;
    two_agents(&mut h).await;
    open_overlay(&mut h).await;
    h.key(h.client.clone(), "/").await;
    h.type_text(h.client.clone(), "zzz").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("No agent matches"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        f.contains("No agent matches \"zzz\". esc clears the filter"),
        "{f}"
    );
    assert!(
        !f.contains("● codex"),
        "and no row is left under the message:\n{f}"
    );
}

#[tokio::test]
async fn with_no_agents_the_box_says_so_and_names_how_to_get_one() {
    let mut h = Harness::start(Config::default(), 100, 24).await;
    let f = open_overlay(&mut h).await;
    assert!(
        f.contains("No agents yet. Start claude or codex in a pane."),
        "{f}"
    );
}

#[tokio::test]
async fn the_overlay_is_centred_between_60_and_120_columns_and_three_rows_down() {
    let mut h = Harness::start(Config::default(), 100, 24).await;
    two_agents(&mut h).await;
    let f = open_overlay(&mut h).await;
    let top = row_holding(&f, "┌ Agents");
    assert_eq!(top, 3, "three rows from the top (12.13):\n{f}");
    let line = row(&f, top);
    let left = col_of(line, '┌');
    let right = last_col_of(line, '┐');
    assert_eq!(right - left + 1, 76, "100 columns minus 24:\n{f}");
    assert_eq!(left, BOX_LEFT, "and centred, so 12 columns each side:\n{f}");
    // What the overlay does not cover reads as being behind it (interface spec 7.1), and what
    // it does cover is not dimmed with the screen.
    assert!(
        style_at(&f, top, 1).contains("dim"),
        "the screen beside the box is dimmed:\n{f}"
    );
    assert!(
        !style_at(&f, top, BOX_LEFT).contains("dim"),
        "and the box itself is not:\n{f}"
    );
    h.resize(h.client.clone(), 200, 40).await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("┌ Agents"),
            Duration::from_secs(2),
        )
        .await;
    let line = row(&f, row_holding(&f, "┌ Agents"));
    let left = col_of(line, '┌');
    assert_eq!(
        last_col_of(line, '┐') - left + 1,
        120,
        "capped at 120:\n{f}"
    );
    assert_eq!(left, 40, "and still centred:\n{f}");
}

/// The exited row is the one row that names the resume key, and it names the configured one
/// (principle 3). Nothing else in M3 draws `AgentsView::resume_key`, so without this fixture
/// blanking that field would break no test until Task 18.
#[tokio::test]
async fn an_exited_row_names_the_configured_resume_key() {
    let mut h = Harness::start(Config::default(), 100, 24).await;
    one_exited_agent(&mut h).await;
    let f = open_overlay(&mut h).await;
    let line = row(&f, row_holding(&f, "● claude"));
    assert!(
        line.contains("exited"),
        "the activity names the state:\n{f}"
    );
    assert!(
        line.contains("⏎ resume"),
        "and then the key that resumes it:\n{f}"
    );
}

/// Enter on an exited row runs `agent.resume` and keeps the overlay open: resume is not a
/// navigation, so leaving the list is wrong (assumption 28). Which method the row reached is
/// pinned by the result, which carries the command `agent.resume` typed; what it must never
/// reach is `agent.focus`, which refuses an exited record.
///
/// This test read the refusal `agent.resume` answered before Task 18 built it, and pinned the
/// code word to say which method the row had reached. `agent_resume.rs` owns the resuming; what
/// is still this file's is that the row goes to that method and that the list stays up.
#[tokio::test]
async fn enter_on_an_exited_row_reaches_resume_and_leaves_the_overlay_open() {
    let mut h = Harness::start(Config::default(), 100, 24).await;
    one_exited_agent(&mut h).await;
    let before = h.focused_pane(h.client.clone());
    open_overlay(&mut h).await;
    let exited = h.agents().await[0].id.clone();
    assert_eq!(h.agents().await[0].state, AgentState::Exited);
    let out = h.api("list.activate", json!({})).await.unwrap();
    assert_eq!(
        out["agent"],
        exited.to_string(),
        "the row reached agent.resume, with the record under the cursor: {out}"
    );
    assert!(
        out["command"]
            .as_str()
            .expect("the result carries the command")
            .contains("claude --resume 'c1'"),
        "and it resumed that record's own session: {out}"
    );
    let f = h.frame(h.client.clone()).await;
    assert!(f.contains("┌ Agents"), "the overlay stayed open:\n{f}");
    // Over the API, so nothing else composed this frame. A key gets one from `Core::key`
    // whatever the handler did, and every other test of this footer presses one; here the
    // result line is only on the screen because setting it marked the view (principle 8).
    assert!(
        f.contains("Resumed claude in "),
        "and the footer carries the result, with no key to have redrawn it:\n{f}"
    );
    let view = h.model().client(&h.client).unwrap().clone();
    assert_eq!(view.overlay, Some(Overlay::Agents));
    assert_eq!(view.agents_cursor, Some(exited), "the cursor did not move");
    assert_eq!(
        h.focused_pane(h.client.clone()),
        before,
        "and the keys did not go into a pane"
    );
}

/// `list.down` keeps the filled row in view by moving `ClientView.agents_scroll`, the same way
/// M2's handlers keep `projects_scroll` (interface spec 12.2). A 12 row screen leaves the box
/// two lines for rows, once its border and its end pads are off (decision record 0023), which
/// two agents of two lines each overflow by three.
#[tokio::test]
async fn list_down_scrolls_the_box_to_keep_the_filled_row_in_view() {
    let mut h = Harness::start(Config::default(), 100, 12).await;
    two_agents(&mut h).await;
    let f = open_overlay(&mut h).await;
    assert_eq!(
        h.model().client(&h.client).unwrap().agents_scroll,
        0,
        "the overlay opens at the top:\n{f}"
    );
    assert!(f.contains("● codex"), "the first row is in view:\n{f}");
    h.key(h.client.clone(), "j").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("● claude"),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(
        h.model().client(&h.client).unwrap().agents_scroll,
        3,
        "the box scrolled to bring the second row in:\n{f}"
    );
    assert!(
        !f.contains("● codex"),
        "and the first row's first line went with it:\n{f}"
    );
}

/// `agents.close` names the agents overlay, so it closes that one and leaves anything else
/// alone, the way `switcher.close` does.
#[tokio::test]
async fn agents_close_leaves_another_overlay_alone() {
    let mut h = Harness::start(Config::default(), 100, 24).await;
    two_agents(&mut h).await;
    h.api("switcher.open", json!({})).await.unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("┌ Projects"),
            Duration::from_secs(2),
        )
        .await;
    h.api("agents.close", json!({})).await.unwrap();
    let view = h.model().client(&h.client).unwrap().clone();
    assert_eq!(
        view.overlay,
        Some(Overlay::Switcher),
        "the switcher is not the agents overlay:\n{f}"
    );
    let f = h.frame(h.client.clone()).await;
    assert!(f.contains("┌ Projects"), "and it is still drawn:\n{f}");
}

/// The Agents box holds the `[keys.list]` table, and `n` in that table is `workspace.rename`,
/// which acts on the row under the cursor while the keys are in a **Projects** box and on the
/// client's own workspace otherwise. The Agents box is not a Projects box: `projects_cursor`
/// outlives the switcher, so a plain "the keys are in a box" here would rename a workspace
/// whose row is nowhere on the screen.
#[tokio::test]
async fn n_in_the_agents_overlay_names_the_clients_own_workspace_and_not_a_projects_row() {
    let mut h = Harness::start(Config::default(), 100, 24).await;
    two_agents(&mut h).await;
    let mine = h.model().client(&h.client).unwrap().workspace.clone();
    // A second project, so the Projects cursor has somewhere to go that is not the client's
    // own workspace.
    h.git_project("main").await;
    h.api("switcher.open", json!({})).await.unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains("┌ Projects"),
        Duration::from_secs(3),
    )
    .await;
    // The client's own workspace may be the first selectable row or the last, and a step off
    // either end clamps, so this steps whichever way there is room for. The assertion below is
    // what says the fixture actually worked.
    h.key(h.client.clone(), "k").await;
    h.frame(h.client.clone()).await;
    if h.model()
        .client(&h.client)
        .unwrap()
        .projects_cursor
        .as_ref()
        == Some(&mine)
    {
        h.key(h.client.clone(), "j").await;
        h.frame(h.client.clone()).await;
    }
    let elsewhere = h
        .model()
        .client(&h.client)
        .unwrap()
        .projects_cursor
        .clone()
        .expect("the switcher put a cursor on a row");
    assert_ne!(
        elsewhere, mine,
        "the fixture moved the Projects cursor off the client's own workspace"
    );
    h.key(h.client.clone(), "Esc").await;
    h.wait_for(
        h.client.clone(),
        |f| !f.contains("┌ Projects"),
        Duration::from_secs(2),
    )
    .await;
    open_overlay(&mut h).await;
    h.key(h.client.clone(), "n").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("┌ Name"),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(
        h.model().client(&h.client).unwrap().overlay,
        Some(Overlay::NameWorkspace(mine)),
        "the name box names the workspace the client is in:\n{f}"
    );
}

/// The overlay opens with an empty filter and the box at the top, every time (interface spec
/// 12.32). `ClientView::filter` and `agents_scroll` both outlive an overlay, so an overlay
/// reopened showing only what the last search matched, scrolled to where the last cursor left
/// it, would hide the agent the reader came for.
#[tokio::test]
async fn reopening_the_overlay_starts_with_no_filter_and_the_box_at_the_top() {
    let mut h = Harness::start(Config::default(), 100, 12).await;
    two_agents(&mut h).await;
    open_overlay(&mut h).await;
    h.key(h.client.clone(), "j").await;
    h.frame(h.client.clone()).await;
    h.key(h.client.clone(), "/").await;
    h.type_text(h.client.clone(), "claude").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("Filter › claude"),
            Duration::from_secs(2),
        )
        .await;
    let view = h.model().client(&h.client).unwrap().clone();
    assert_eq!(view.filter, "claude", "{f}");
    assert_eq!(view.agents_scroll, 3, "the box is scrolled:\n{f}");
    // `agents.close` and not Esc: Esc while `/` is open clears the filter itself, and what
    // this is about is the filter that survives the overlay.
    h.api("agents.close", json!({})).await.unwrap();
    assert_eq!(
        h.model().client(&h.client).unwrap().filter,
        "claude",
        "the filter outlives the overlay, which is why open has to clear it"
    );
    let f = open_overlay(&mut h).await;
    let view = h.model().client(&h.client).unwrap().clone();
    assert_eq!(view.filter, "", "the filter is gone:\n{f}");
    assert_eq!(
        view.agents_scroll, 0,
        "and the box is back at the top:\n{f}"
    );
    assert!(f.contains("● codex"), "so the first row is in view:\n{f}");
}

/// An overlay opened over another comes back when this one closes (interface spec 12.7). Only
/// the API can reach this: an open overlay takes every key, and `a` is not in `[keys.list]`.
#[tokio::test]
async fn the_overlay_opens_over_another_and_gives_it_back_when_it_closes() {
    let mut h = Harness::start(Config::default(), 100, 24).await;
    two_agents(&mut h).await;
    h.api("switcher.open", json!({})).await.unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains("┌ Projects"),
        Duration::from_secs(2),
    )
    .await;
    open_overlay(&mut h).await;
    let view = h.model().client(&h.client).unwrap().clone();
    assert_eq!(view.overlay, Some(Overlay::Agents));
    assert_eq!(
        view.overlay_under,
        Some(Overlay::Switcher),
        "the switcher is still there under it"
    );
    h.api("agents.close", json!({})).await.unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("┌ Projects"),
            Duration::from_secs(2),
        )
        .await;
    let view = h.model().client(&h.client).unwrap().clone();
    assert_eq!(view.overlay, Some(Overlay::Switcher), "{f}");
    assert_eq!(
        view.focus,
        Focus::Region(RegionKind::Switcher),
        "and its box has the keys again"
    );
}

/// The box draws from the scroll the model remembers, not from the top of the list.
///
/// `ListBox::render` corrects that number whenever there is a filled row, so the only state in
/// which the remembered scroll reaches the screen is one with no fill and a list taller than
/// the box: the cursor names a record that is no longer in the list. Dismissing the record the
/// cursor is on is how a reader gets there.
#[tokio::test]
async fn the_box_draws_from_the_remembered_scroll_when_the_cursor_names_no_row() {
    // 13 rows and not 12: the box spends its last row on the footer (MUX-16), so a screen one
    // row taller is what leaves it the two lines of rows this fixture is counted in.
    let mut h = Harness::start(Config::default(), 100, 13).await;
    two_agents(&mut h).await;
    h.api("pane.split", json!({"dir": "down"})).await.unwrap();
    let third = h.focused_pane(h.client.clone());
    h.report(
        third.clone(),
        AgentKind::Claude,
        r#"{"hook_event_name":"SessionStart","session_id":"c2"}"#,
    )
    .await;
    h.report(
        third.clone(),
        AgentKind::Claude,
        r#"{"hook_event_name":"SessionEnd","session_id":"c2"}"#,
    )
    .await;
    // Waiting, then working, then exited (interface spec 6.7): three rows of two lines with a
    // blank between them, in a box two lines high.
    let listed = h.agents().await;
    assert_eq!(listed.len(), 3);
    assert_eq!(listed[2].state, AgentState::Exited);
    let doomed = listed[2].id.clone();
    open_overlay(&mut h).await;
    h.key(h.client.clone(), "j").await;
    h.frame(h.client.clone()).await;
    h.key(h.client.clone(), "j").await;
    h.frame(h.client.clone()).await;
    let view = h.model().client(&h.client).unwrap().clone();
    assert_eq!(view.agents_cursor.as_ref(), Some(&doomed));
    assert_eq!(view.agents_scroll, 6, "the box scrolled to the last row");
    h.api("agent.dismiss", json!({"agent": doomed.to_string()}))
        .await
        .unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("● claude"),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(
        h.model().client(&h.client).unwrap().agents_scroll,
        6,
        "nothing reset the remembered scroll:\n{f}"
    );
    assert!(
        !f.contains("● codex"),
        "so the box is still scrolled past the first row's first line:\n{f}"
    );
}

/// Enter on a live row leaves through `Model::select_tab`, which clears the overlay without
/// going through `ClientView::pop_overlay`, so the flag that says `/` is open survives it. The
/// next open has to clear it, or the footer comes back as a filter field with nothing typed in
/// it and every key the reader presses goes into that field.
#[tokio::test]
async fn reopening_after_a_switch_out_of_the_filter_comes_back_with_the_keys_row() {
    let mut h = Harness::start(Config::default(), 100, 24).await;
    two_agents(&mut h).await;
    open_overlay(&mut h).await;
    h.api("list.filter", json!({})).await.unwrap();
    h.api("list.activate", json!({})).await.unwrap();
    assert!(
        h.model().client(&h.client).unwrap().filtering,
        "the switch left the flag standing, which is what open has to answer for"
    );
    let f = open_overlay(&mut h).await;
    assert!(
        f.contains("⏎ open · / filter · esc close"),
        "the footer is the keys row again:\n{f}"
    );
    assert!(
        !f.contains("esc clear"),
        "and not a filter field with nothing in it:\n{f}"
    );
    assert!(!h.model().client(&h.client).unwrap().filtering);
}

/// `?` inside the Agents box lists the box's own keys first, and still does after the help has
/// been opened and closed once.
///
/// Closing the help pops back to the agents overlay, and `ClientView::focus_after_pop` is what
/// says which region the keys land in. Left as `Region(Overlay)` - the one region kind that is
/// not a box - the next `?` drops the `[keys.list]` block to the bottom of the overlay, where a
/// 24 row screen cuts it off entirely: a reader standing in the Agents box asks for help and is
/// not shown the keys they are holding. M2 left that arm for the milestone that made the box
/// reachable.
///
/// A 24 row screen on purpose, for the reason `tests/filter.rs` gives for the switcher's half of
/// this: the overlay does not fit, so the order is the whole of what the reader gets and a block
/// placed last is gone rather than merely late.
#[tokio::test]
async fn help_inside_the_box_lists_the_box_keys_first_after_the_help_has_been_closed_once() {
    let mut h = Harness::start(Config::default(), 100, 24).await;
    two_agents(&mut h).await;
    open_overlay(&mut h).await;
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
        "the box's keys come first the first time:\n{f}"
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
        Focus::Region(RegionKind::AgentsOverlay),
        "the keys came back to the box the help was opened over:\n{f}"
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
        f.contains("j          list.down"),
        "and they still come first the second time:\n{f}"
    );
    // The anchor is whichever leader binding sorts first, not a named action: the leader table
    // is the thing the box's block has to precede, and which action heads it is not the claim.
    let first_leader = f
        .lines()
        .filter(|l| l.starts_with('|'))
        .position(|l| l.contains("C-a ") && !l.contains("leader"))
        .unwrap_or_else(|| panic!("no leader binding row in:\n{f}"));
    assert!(
        row_holding(&f, "in a list") < first_leader,
        "before the leader table:\n{f}"
    );
}
