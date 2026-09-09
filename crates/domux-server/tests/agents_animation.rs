//! The working glyph animation: one frame every 80 ms while an agent works or compacts.
//!
//! What these tests can and cannot see. The animation reaches the screen, so a frame test
//! holds it: the glyph on a working row turns, and the word beside it does not. What no
//! frame shows is the core's own counting, because a server with nothing working draws no
//! glyph at all, so the screen stands still whether the core counted or not. The rule that
//! an idle server does no work is pinned where it is visible, in `core`'s own tests
//! (`an_animation_tick_leaves_a_server_with_nothing_working_alone`), and so is the pool of
//! working words.
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
        let line = row_of(&f, "● claude").to_string();
        for g in GLYPH_FRAMES {
            if line.contains(g) {
                seen.insert(g);
            }
        }
        tokio::time::sleep(gap).await;
    }
    seen
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

#[tokio::test]
async fn the_glyph_turns_while_an_agent_works_and_stands_still_when_it_stops() {
    let mut h = Harness::start(Config::default(), 100, 24).await;
    let pane = h.focused_pane(h.client.clone());
    h.report(pane.clone(), AgentKind::Claude, CLAUDE_WORKS)
        .await;
    open_overlay(&mut h).await;

    // Fourteen looks 90 ms apart over an 80 ms animation: the glyph moves on between every
    // pair, so a static glyph leaves one frame in the set and fails here.
    let seen = glyphs_over(&mut h, 14, Duration::from_millis(90)).await;
    assert!(
        seen.len() >= 3,
        "the glyph turned through at least three frames, saw {seen:?}"
    );

    // And it stops. No agent works, so no row draws a glyph and the screen stands still.
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
    let first = word_on(row_of(&f, "● claude")).to_string();

    let mut glyphs = HashSet::new();
    for _ in 0..8 {
        tokio::time::sleep(Duration::from_millis(90)).await;
        let f = h.frame(h.client.clone()).await;
        let line = row_of(&f, "● claude");
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
            |f| f.contains("● codex") && f.contains("● claude"),
            Duration::from_secs(2),
        )
        .await;
    let claude = word_on(row_of(&f, "● claude")).to_string();
    let codex = word_on(row_of(&f, "● codex")).to_string();
    assert_ne!(claude, codex, "never the same word twice on screen:\n{f}");
}
