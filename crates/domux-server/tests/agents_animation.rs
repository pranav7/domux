//! The working row's animation: one tick every 80 ms while an agent works or compacts, which
//! moves the band along the word and turns the glyph on every second one.
//!
//! What these tests can and cannot see. The animation reaches the screen, so a frame test
//! holds it: the glyph on a working row turns, the band along the word moves, and the word
//! itself does not change. What no frame shows is the core's own counting, because a server
//! with nothing working draws no glyph at all, so the screen stands still whether the core
//! counted or not. The rule that an idle server does no work is pinned where it is visible,
//! in `core`'s own tests (`an_animation_tick_leaves_a_server_with_nothing_working_alone`),
//! and so is the pool of working words.
//!
//! An assertion that something did **not** change passes on a screen that never changes, so
//! the standing half of the first test stands after the turning half, on the same fixture.

use domux_core::config::Config;
use domux_core::model::agent::AgentKind;
use domux_server::agents::labels::GLYPH_FRAMES;
use domux_server::testing::Harness;
use serde_json::json;
use std::collections::HashSet;
use std::time::Duration;

const CLAUDE_WORKS: &str = r#"{"hook_event_name":"UserPromptSubmit","session_id":"c1"}"#;
const CLAUDE_STOPS: &str = r#"{"hook_event_name":"Stop","session_id":"c1"}"#;
const CODEX_WORKS: &str = r#"{"hook_event_name":"UserPromptSubmit","session_id":"x1"}"#;

/// Line 1 of the row for `kind`, which is where the glyph and the word are drawn.
fn row_of<'a>(frame: &'a str, kind: &str) -> &'a str {
    frame
        .lines()
        .find(|l| l.contains(kind))
        .unwrap_or_else(|| panic!("no row holds {kind:?} in:\n{frame}"))
}

/// The working word on a row, without the ellipsis the row draws it with. Read from the
/// screen rather than from the model: what the reader sees is the claim.
fn word_on(line: &str) -> &str {
    let before = line
        .split('…')
        .next()
        .unwrap_or_else(|| panic!("the working word ends with an ellipsis in {line:?}"));
    assert!(before != line, "no working word in {line:?}");
    before
        .split_whitespace()
        .next_back()
        .unwrap_or_else(|| panic!("nothing before the ellipsis in {line:?}"))
}

/// Every glyph frame the row showed over `samples` looks, `gap` apart.
async fn glyphs_over(h: &mut Harness, samples: usize, gap: Duration) -> HashSet<&'static str> {
    let mut seen = HashSet::new();
    for _ in 0..samples {
        let f = h.frame(h.client.clone()).await;
        let line = row_of(&f, "claude ").to_string();
        for g in GLYPH_FRAMES {
            if line.contains(g) {
                seen.insert(g);
            }
        }
        tokio::time::sleep(gap).await;
    }
    seen
}

/// The Navigator, which is where a working row lives now: `leader s` over the whole screen,
/// with the agents nested under the workspaces they run in (decision record 0028).
async fn open_overlay(h: &mut Harness) -> String {
    h.api("switcher.open", json!({})).await.unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains("┌ Navigator"),
        Duration::from_secs(2),
    )
    .await
}

#[tokio::test]
async fn the_glyph_turns_while_an_agent_works_and_stands_still_when_it_stops() {
    let mut h = Harness::start(Config::default(), 100, 24).await;
    let pane = h.focused_pane(h.client.clone());
    h.report(pane.clone(), AgentKind::Claude, CLAUDE_WORKS)
        .await;
    open_overlay(&mut h).await;

    // Fourteen looks 90 ms apart over a glyph that turns every 160 ms: eight turns, so a
    // static glyph leaves one frame in the set and fails here.
    let seen = glyphs_over(&mut h, 14, Duration::from_millis(90)).await;
    assert!(
        seen.len() >= 3,
        "the glyph turned through at least three frames, saw {seen:?}"
    );

    // And it stops. No agent works, so no row draws a glyph and the screen stands still.
    //
    // The whole frame, and not only the row, because nothing else in it moves either: the
    // harness runs the server on a `FixedClock`, which is what `render` hands the top bar and
    // what `Core::tick` measures its minute against, so the clock at the top right reads the
    // same string on both looks. It starts no fact providers, so nothing arrives from outside
    // either. Take either of those away and this comparison becomes a wall-clock race.
    h.report(pane.clone(), AgentKind::Claude, CLAUDE_STOPS)
        .await;
    let quiet = h.frame(h.client.clone()).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    let still = h.frame(h.client.clone()).await;
    assert_eq!(quiet, still, "nothing works, so nothing turns");
}

#[tokio::test]
async fn the_word_stands_still_while_the_glyph_turns() {
    let mut h = Harness::start(Config::default(), 100, 24).await;
    let pane = h.focused_pane(h.client.clone());
    h.report(pane.clone(), AgentKind::Claude, CLAUDE_WORKS)
        .await;
    let f = open_overlay(&mut h).await;
    let first = word_on(row_of(&f, "claude ")).to_string();

    // Fourteen looks rather than eight, for the same eight turns the test above samples: the
    // glyph holds each frame for two ticks (MUX-26), so eight looks see half as many.
    let mut glyphs = HashSet::new();
    for _ in 0..14 {
        tokio::time::sleep(Duration::from_millis(90)).await;
        let f = h.frame(h.client.clone()).await;
        let line = row_of(&f, "claude ");
        assert_eq!(
            word_on(line),
            first,
            "the word is the agent's, not the frame's"
        );
        for g in GLYPH_FRAMES {
            if line.contains(g) {
                glyphs.insert(g);
            }
        }
    }
    // Without this the test above passes on a row that draws nothing at all: the word holds
    // still only because the animation is running under it.
    assert!(
        glyphs.len() >= 3,
        "the glyph was turning throughout, saw {glyphs:?}"
    );
}

/// The band reaches the screen, which no test above can see: the characters of the working
/// word are one string however they are lit, so only the styles say whether it is drawn.
///
/// Two claims, and the second is the one that costs anything. That the characters differ from
/// each other says a band is drawn; that the whole run differs a moment later says it moves.
/// A word painted in one colour fails the first, and one lit by a band nothing advances fails
/// the second.
#[tokio::test]
async fn the_band_lights_the_working_word_and_moves_along_it() {
    let mut h = Harness::start(Config::default(), 100, 24).await;
    let pane = h.focused_pane(h.client.clone());
    h.report(pane.clone(), AgentKind::Claude, CLAUDE_WORKS)
        .await;
    let f = open_overlay(&mut h).await;

    // Where the word is: the screen row the claude row is drawn on, and the cells from the
    // one after the glyph's space to the ellipsis. A frame row is bracketed by `|`, so cell
    // zero is character one.
    let rows: Vec<&str> = f.lines().filter(|l| l.starts_with('|')).collect();
    let row = rows
        .iter()
        .position(|l| l.contains("claude "))
        .expect("a claude row is on the screen") as u16;
    let cells: Vec<char> = rows[row as usize].chars().collect();
    let last = cells
        .iter()
        .position(|c| *c == '…')
        .expect("the word ends with an ellipsis");
    let first = cells[..last]
        .iter()
        .rposition(|c| *c == ' ')
        .expect("a space parts the glyph from the word")
        + 1;
    let word = |h: &Harness| {
        let buf = h.buffer(&h.client.clone());
        (first..=last)
            .map(|c| buf[(c as u16 - 1, row)].fg)
            .collect::<Vec<_>>()
    };

    // Ten looks 90 ms apart, because a pass leaves the word dark at each end of its travel
    // and one look can land there: what is claimed is that the band crosses the word over a
    // pass, not that it is on it at every moment.
    let mut looks = Vec::new();
    for _ in 0..10 {
        h.frame(h.client.clone()).await;
        looks.push(word(&h));
        tokio::time::sleep(Duration::from_millis(90)).await;
    }
    assert!(
        looks
            .iter()
            .any(|w| w.iter().collect::<HashSet<_>>().len() > 3),
        "the word is lit unevenly, so a band is on it: {looks:?}"
    );
    assert!(
        looks.windows(2).any(|pair| pair[0] != pair[1]),
        "and it moves along the word: {looks:?}"
    );
}

/// Two agents on screen, two different words.
///
/// This holds the rule at the surface and no further. The words are picked from a hash of the
/// agent id, so two records the model numbered differently take different words whether or not
/// `word_for` walks past the ones already taken, and a build that dropped that walk passes
/// here. What holds the walk is `labels::tests::no_two_agents_on_screen_share_a_word`, which
/// picks forty ids and so meets the collision this fixture cannot force.
#[tokio::test]
async fn two_working_agents_never_share_a_word() {
    let mut h = Harness::start(Config::default(), 100, 24).await;
    let first = h.focused_pane(h.client.clone());
    h.api("pane.split", json!({"dir": "right"})).await.unwrap();
    let second = h.focused_pane(h.client.clone());
    h.report(first, AgentKind::Claude, CLAUDE_WORKS).await;
    h.report(second, AgentKind::Codex, CODEX_WORKS).await;
    open_overlay(&mut h).await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("codex ") && f.contains("claude "),
            Duration::from_secs(2),
        )
        .await;
    let claude = word_on(row_of(&f, "claude ")).to_string();
    let codex = word_on(row_of(&f, "codex ")).to_string();
    assert_ne!(claude, codex, "never the same word twice on screen:\n{f}");
}
