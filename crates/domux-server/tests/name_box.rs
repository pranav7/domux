//! The name box: `leader N`, `n` on a row, and `leader n` (interface spec 7.1 and 12.9).
//!
//! The fixture every test here starts from is `Harness::git_project_with_two_slots`, which
//! leaves the model holding two projects: the harness's own `proj`, a plain folder with just
//! `main`, and the git project `audrey-app` with `main`, `workspace-1` and `workspace-2`. The
//! attached client is in `proj`'s `main`, so "the workspace the client is in" and "the row the
//! cursor is on" are different workspaces in different projects unless a test moves one of
//! them. Every test below depends on that: it is the only fixture that can tell the two rules
//! apart.
//!
//! The box is sorted alphabetically and blank rows separate the workspaces, so the switcher
//! lists `audrey-app`'s `main`, `workspace-1` and `workspace-2`, then `proj`'s `main`, and the
//! cursor starts on the last of those.

use domux_core::api::ErrorCode;
use domux_core::config::Config;
use domux_core::model::{Focus, Overlay, RegionKind};
use domux_server::testing::{row, Harness};
use serde_json::json;
use std::time::Duration;

const WAIT: Duration = Duration::from_secs(5);

/// `leader N` from a pane names the workspace the client is in, and the title says which one
/// that is, so the reader never names the wrong slot (interface spec 7.1).
#[tokio::test]
async fn leader_n_names_the_workspace_the_client_is_in_and_the_title_shows_its_handle() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (_root, w1, _w2) = h.git_project_with_two_slots().await;
    let client = h.client.clone();
    h.api(
        "workspace.focus",
        json!({"workspace": w1.as_str(), "client": client.as_str()}),
    )
    .await
    .unwrap();

    h.key(client.clone(), "C-a").await;
    h.key(client.clone(), "N").await;

    let f = h
        .wait_for(client.clone(), |f| f.contains("Name workspace-1"), WAIT)
        .await;
    // Row 10 is the box's top border: 58 by 5 cells centred over a workpanel that starts one
    // row under the top bar, on an 80 by 24 screen.
    assert!(row(&f, 10).contains("┌ Name workspace-1 "), "{f}");
    assert!(
        f.contains("⏎ save    esc cancel    an empty name clears it"),
        "{f}"
    );

    h.type_text(client.clone(), "auth cleanup").await;
    // Nothing else on this screen can be drawing those words: the workspace still answers to
    // its handle, so the only place `auth cleanup` can be is the field (principle 5).
    h.wait_for(client.clone(), |f| f.contains("auth cleanup"), WAIT)
        .await;

    h.key(client.clone(), "Enter").await;
    h.wait_for(client.clone(), |f| !f.contains("Name workspace-1"), WAIT)
        .await;

    assert_eq!(
        h.model().workspace(&w1).unwrap().name.as_deref(),
        Some("auth cleanup")
    );
    let m = h.model();
    let view = m.client(&client).expect("the client is attached");
    assert_eq!(view.overlay, None, "the box closed");
    assert_eq!(view.overlay_under, None, "and left nothing behind it");
    assert!(
        matches!(view.focus, Focus::Pane(_)),
        "the keys go back to the pane the box was opened from: {:?}",
        view.focus
    );
}

/// Esc leaves the name it found, including the name the box was opened on.
#[tokio::test]
async fn esc_closes_the_box_and_leaves_the_name_alone() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (_root, w1, _w2) = h.git_project_with_two_slots().await;
    let client = h.client.clone();
    h.api(
        "workspace.rename",
        json!({"workspace": w1.as_str(), "name": "auth cleanup"}),
    )
    .await
    .unwrap();
    h.api(
        "workspace.focus",
        json!({"workspace": w1.as_str(), "client": client.as_str()}),
    )
    .await
    .unwrap();

    h.key(client.clone(), "C-a").await;
    h.key(client.clone(), "N").await;
    h.wait_for(client.clone(), |f| f.contains("Name workspace-1"), WAIT)
        .await;
    // Typed over, so Esc has something to throw away: a box the reader never touched would
    // leave the name alone under any implementation of Esc at all.
    h.type_text(client.clone(), " and tests").await;
    h.wait_for(client.clone(), |f| f.contains("and tests"), WAIT)
        .await;

    h.key(client.clone(), "Esc").await;

    h.wait_for(client.clone(), |f| !f.contains("Name workspace-1"), WAIT)
        .await;
    assert_eq!(
        h.model().workspace(&w1).unwrap().name.as_deref(),
        Some("auth cleanup"),
        "esc changes nothing"
    );
    let m = h.model();
    assert_eq!(m.client(&client).unwrap().overlay, None);
}

/// The box opens on the name the workspace already has, so fixing a typo does not mean
/// retyping the name, and a blank field clears it (interface spec 7.1).
///
/// The field is emptied and then filled with two spaces, because a blank name is what
/// `Model::rename_workspace` clears on: a field of nothing but spaces has to reach the same
/// answer, and the pill has to agree with it or it claims a name the model did not keep.
#[tokio::test]
async fn the_box_opens_on_the_current_name_and_a_blank_field_clears_it() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (_root, w1, _w2) = h.git_project_with_two_slots().await;
    let client = h.client.clone();
    h.api(
        "workspace.rename",
        json!({"workspace": w1.as_str(), "name": "auth"}),
    )
    .await
    .unwrap();
    h.api(
        "workspace.focus",
        json!({"workspace": w1.as_str(), "client": client.as_str()}),
    )
    .await
    .unwrap();

    h.key(client.clone(), "C-a").await;
    h.key(client.clone(), "N").await;
    h.wait_for(client.clone(), |f| f.contains("Name workspace-1"), WAIT)
        .await;
    assert_eq!(
        h.model().client(&client).unwrap().input.text,
        "auth",
        "the field starts on the name the workspace has"
    );

    for _ in 0..4 {
        h.key(client.clone(), "Backspace").await;
    }
    h.type_text(client.clone(), "  ").await;
    h.key(client.clone(), "Enter").await;

    let f = h
        .wait_for(client.clone(), |f| !f.contains("Name workspace-1"), WAIT)
        .await;
    assert_eq!(
        h.model().workspace(&w1).unwrap().name,
        None,
        "a blank name clears it:\n{f}"
    );
    let m = h.model();
    assert_eq!(
        m.client(&client)
            .unwrap()
            .pill
            .as_ref()
            .map(|p| (p.text.as_str(), p.ok)),
        Some(("Cleared the name on workspace-1", true)),
        "and says which slot it cleared (principle 8)"
    );
}

/// `n` in the sidebar's Projects box is the same operation as `n` in the switcher: it names
/// the row under the cursor. The keys leave the sidebar while the box is open, so the box is
/// the one thing on the screen carrying the accent (principle 2).
#[tokio::test]
async fn n_in_the_sidebar_box_names_the_cursor_row_and_takes_the_keys_off_the_sidebar() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let (_root, w1, _w2) = h.git_project_with_two_slots().await;
    let client = h.client.clone();
    h.api("sidebar.show", serde_json::json!({})).await.unwrap();
    h.wait_for(client.clone(), |f| f.contains("Projects"), WAIT)
        .await;
    h.key(client.clone(), "C-h").await;
    h.wait_for(client.clone(), |f| f.contains("r0 c0-0 fg=#cba6f7"), WAIT)
        .await;
    // The cursor starts on `proj`'s `main`, the last row: two steps up is `workspace-1`.
    h.key(client.clone(), "k").await;
    h.key(client.clone(), "k").await;
    h.frame(client.clone()).await;
    assert_eq!(
        h.model().client(&client).unwrap().projects_cursor.as_ref(),
        Some(&w1),
        "the cursor is on workspace-1 before the key under test"
    );

    h.key(client.clone(), "n").await;

    let f = h
        .wait_for(client.clone(), |f| f.contains("Name workspace-1"), WAIT)
        .await;
    let m = h.model();
    let view = m.client(&client).unwrap();
    assert_eq!(view.overlay, Some(Overlay::NameWorkspace(w1.clone())));
    assert_eq!(
        view.focus,
        Focus::Region(RegionKind::Overlay),
        "the keys are in the box now, not in the sidebar"
    );
    // The dim is the name box's own, over everything it did not draw, so this line reads
    // "dimmed and unfocused" rather than "dimmed", which a focused border would also be.
    assert!(
        f.contains("r0 c0-0 dim fg=#585b70"),
        "so the sidebar's box gives up the accent:\n{f}"
    );
}

/// The caret moves, so a name is edited rather than only appended to. Every one of the four
/// keys changes the answer: drop any of them and the final name is a different string.
#[tokio::test]
async fn the_arrow_home_and_end_keys_move_the_caret_in_the_field() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (_root, w1, _w2) = h.git_project_with_two_slots().await;
    let client = h.client.clone();
    h.api(
        "workspace.focus",
        json!({"workspace": w1.as_str(), "client": client.as_str()}),
    )
    .await
    .unwrap();
    h.key(client.clone(), "C-a").await;
    h.key(client.clone(), "N").await;
    h.wait_for(client.clone(), |f| f.contains("Name workspace-1"), WAIT)
        .await;

    h.type_text(client.clone(), "auth").await;
    h.key(client.clone(), "Left").await;
    h.type_text(client.clone(), "X").await;
    h.key(client.clone(), "Home").await;
    h.key(client.clone(), "Right").await;
    h.type_text(client.clone(), "1").await;
    h.key(client.clone(), "End").await;
    h.type_text(client.clone(), "9").await;
    h.key(client.clone(), "Enter").await;

    h.wait_for(client.clone(), |f| !f.contains("Name workspace-1"), WAIT)
        .await;
    assert_eq!(
        h.model().workspace(&w1).unwrap().name.as_deref(),
        Some("a1utXh9")
    );
}

/// `leader n` clears the name with no question and no box: the row redrawing with its handle
/// is the whole answer (interface spec 12.9).
#[tokio::test]
async fn leader_n_clears_the_name_with_no_prompt() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (_root, w1, _w2) = h.git_project_with_two_slots().await;
    let client = h.client.clone();
    h.api(
        "workspace.rename",
        json!({"workspace": w1.as_str(), "name": "auth cleanup"}),
    )
    .await
    .unwrap();
    h.api(
        "workspace.focus",
        json!({"workspace": w1.as_str(), "client": client.as_str()}),
    )
    .await
    .unwrap();

    h.key(client.clone(), "C-a").await;
    h.key(client.clone(), "n").await;

    // The top bar reads `project › workspace`, so the handle coming back is visible there.
    h.wait_for(client.clone(), |f| f.contains("› workspace-1"), WAIT)
        .await;
    assert_eq!(h.model().workspace(&w1).unwrap().name, None);
    let m = h.model();
    let view = m.client(&client).unwrap();
    assert_eq!(
        view.overlay, None,
        "and without an overlay (interface spec 12.9)"
    );
    // And with no pill of its own. Asserted against the one string this key could have
    // written rather than against `None`: the fixture's two creates leave `Created
    // workspace-2` in the pill, and nothing here clears a pill it did not set.
    assert_ne!(
        view.pill.as_ref().map(|p| p.text.as_str()),
        Some("Cleared the name on workspace-1"),
        "the row redrawing with its handle is the answer (interface spec 12.9)"
    );
}

/// `n` on a row names the row the cursor is on, and the box opens over the switcher rather
/// than in place of it (interface spec 12.7).
///
/// The client is left in `proj`'s `main` and the cursor is moved to `audrey-app`'s
/// `workspace-1`, so the two candidate answers are workspaces in different projects: a `n`
/// that read the client's own workspace would name `main` here and fail loudly.
#[tokio::test]
async fn n_on_a_row_names_that_row_and_the_box_sits_over_the_switcher_it_came_from() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (_root, w1, _w2) = h.git_project_with_two_slots().await;
    let client = h.client.clone();
    h.api("switcher.open", json!({"client": client.as_str()}))
        .await
        .unwrap();
    // The cursor starts on `proj`'s `main`, the last row in the box. Two steps up is
    // `audrey-app`'s `workspace-1`, past `workspace-2`.
    h.key(client.clone(), "k").await;
    h.key(client.clone(), "k").await;
    h.frame(client.clone()).await;
    assert_eq!(
        h.model().client(&client).unwrap().projects_cursor.as_ref(),
        Some(&w1),
        "the cursor is on workspace-1 before the key under test"
    );

    h.key(client.clone(), "n").await;

    let f = h
        .wait_for(client.clone(), |f| f.contains("Name workspace-1"), WAIT)
        .await;
    assert!(
        f.contains("Projects"),
        "the switcher stays open beneath it (interface spec 12.7):\n{f}"
    );
    {
        let m = h.model();
        let view = m.client(&client).unwrap();
        assert_eq!(view.overlay, Some(Overlay::NameWorkspace(w1.clone())));
        assert_eq!(
            view.overlay_under,
            Some(Overlay::Switcher),
            "the switcher is what it was opened over"
        );
    }

    h.type_text(client.clone(), "spike").await;
    h.key(client.clone(), "Enter").await;

    let f = h
        .wait_for(
            client.clone(),
            |f| f.contains("Named workspace-1 spike"),
            WAIT,
        )
        .await;
    assert_eq!(
        h.model().workspace(&w1).unwrap().name.as_deref(),
        Some("spike")
    );
    let m = h.model();
    let view = m.client(&client).unwrap();
    assert_eq!(
        view.overlay,
        Some(Overlay::Switcher),
        "and it is what Enter returns to"
    );
    assert_eq!(
        view.overlay_under, None,
        "with nothing left stranded under it"
    );
    assert!(
        f.contains("spike"),
        "the row redraws with the name it was given:\n{f}"
    );
}

/// Esc from a box opened over the switcher gives the switcher back, rather than clearing the
/// top overlay and stranding it with nothing drawing it and nothing closing it.
#[tokio::test]
async fn esc_over_the_switcher_gives_the_switcher_back() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (_root, w1, _w2) = h.git_project_with_two_slots().await;
    let client = h.client.clone();
    h.api("switcher.open", json!({"client": client.as_str()}))
        .await
        .unwrap();
    h.key(client.clone(), "k").await;
    h.key(client.clone(), "k").await;
    h.key(client.clone(), "n").await;
    h.wait_for(client.clone(), |f| f.contains("Name workspace-1"), WAIT)
        .await;

    h.key(client.clone(), "Esc").await;

    let f = h
        .wait_for(client.clone(), |f| !f.contains("Name workspace-1"), WAIT)
        .await;
    let m = h.model();
    let view = m.client(&client).unwrap();
    assert_eq!(
        view.overlay,
        Some(Overlay::Switcher),
        "esc goes back one step (interface spec 12.7)"
    );
    assert_eq!(view.overlay_under, None, "and nothing is left under it");
    assert!(f.contains("Projects"), "the switcher is drawn again:\n{f}");
    assert_eq!(
        h.model().workspace(&w1).unwrap().name,
        None,
        "and the row it came from keeps the name it had"
    );
}

/// A name that reads as a handle is refused, and the box stays open holding what was typed:
/// the refusal names something to change about the name, so closing would take the name away
/// with the question.
#[tokio::test]
async fn a_name_that_reads_as_a_handle_keeps_the_box_open_with_what_was_typed() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (_root, w1, _w2) = h.git_project_with_two_slots().await;
    let client = h.client.clone();
    h.api(
        "workspace.focus",
        json!({"workspace": w1.as_str(), "client": client.as_str()}),
    )
    .await
    .unwrap();
    h.key(client.clone(), "C-a").await;
    h.key(client.clone(), "N").await;
    h.wait_for(client.clone(), |f| f.contains("Name workspace-1"), WAIT)
        .await;

    h.type_text(client.clone(), "workspace-2").await;
    h.key(client.clone(), "Enter").await;

    let f = h
        .wait_for(client.clone(), |f| f.contains("is a handle"), WAIT)
        .await;
    let m = h.model();
    let view = m.client(&client).unwrap();
    assert_eq!(
        view.overlay,
        Some(Overlay::NameWorkspace(w1.clone())),
        "the box is still open:\n{f}"
    );
    assert_eq!(
        view.input.text, "workspace-2",
        "with what was typed still in it"
    );
    assert_eq!(
        h.model().workspace(&w1).unwrap().name,
        None,
        "and nothing was named"
    );

    // The next key in the box takes the refusal away and the keys come back, so the answer
    // stands until the reader acts on it and no longer (interface spec 12.12).
    h.key(client.clone(), "Backspace").await;
    let f = h
        .wait_for(client.clone(), |f| f.contains("esc cancel"), WAIT)
        .await;
    assert!(!f.contains("is a handle"), "{f}");
}

/// The field takes text and nothing else. An unbound key does not close the box or reach the
/// pane behind it, and a chorded letter is not typed.
///
/// `C-b` and `M-b`, not `C-a`: the leader is claimed at step 2 of the routing and would never
/// reach the box, so it would pass for the wrong reason. A bare `b` beside them shows the same
/// letter is text on its own.
#[tokio::test]
async fn an_unbound_key_and_a_chorded_letter_do_not_type_into_the_box() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (_root, w1, _w2) = h.git_project_with_two_slots().await;
    let client = h.client.clone();
    h.api(
        "workspace.focus",
        json!({"workspace": w1.as_str(), "client": client.as_str()}),
    )
    .await
    .unwrap();
    let pane = h.focused_pane(client.clone());
    h.key(client.clone(), "C-a").await;
    h.key(client.clone(), "N").await;
    h.wait_for(client.clone(), |f| f.contains("Name workspace-1"), WAIT)
        .await;

    h.key(client.clone(), "b").await;
    h.key(client.clone(), "C-b").await;
    h.key(client.clone(), "M-b").await;
    h.key(client.clone(), "Tab").await;
    h.frame(client.clone()).await;

    let m = h.model();
    let view = m.client(&client).unwrap();
    assert_eq!(view.input.text, "b", "only the bare letter is text");
    assert_eq!(
        view.overlay,
        Some(Overlay::NameWorkspace(w1.clone())),
        "and no key of the four closed the box"
    );
    assert!(
        h.pane_input(&pane).is_empty(),
        "nor reached the pane behind it: {:?}",
        h.pane_input(&pane)
    );
}

/// `workspace.rename` with no name opens the box rather than refusing, and rather than reading
/// "no name" as "a blank name", which would clear the name of the workspace the reader was
/// about to name. The key and the API call reach the same handler.
#[tokio::test]
async fn renaming_with_no_name_opens_the_box_and_changes_nothing() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (_root, w1, _w2) = h.git_project_with_two_slots().await;
    let client = h.client.clone();
    h.api(
        "workspace.rename",
        json!({"workspace": w1.as_str(), "name": "auth cleanup"}),
    )
    .await
    .unwrap();

    h.api("workspace.rename", json!({"workspace": w1.as_str()}))
        .await
        .unwrap();

    let f = h
        .wait_for(client.clone(), |f| f.contains("Name workspace-1"), WAIT)
        .await;
    assert_eq!(
        h.model().workspace(&w1).unwrap().name.as_deref(),
        Some("auth cleanup"),
        "opening the box changes no name:\n{f}"
    );
    let m = h.model();
    assert_eq!(
        m.client(&client).unwrap().overlay,
        Some(Overlay::NameWorkspace(w1.clone()))
    );
}

/// A second open replaces the box rather than stacking one on the other, so what the first was
/// opened over is still there to go back to.
///
/// Only a caller can reach this: a key cannot, because the open box takes every key.
#[tokio::test]
async fn opening_the_box_over_an_open_one_replaces_it_and_keeps_the_switcher() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (_root, w1, w2) = h.git_project_with_two_slots().await;
    let client = h.client.clone();
    h.api("switcher.open", json!({"client": client.as_str()}))
        .await
        .unwrap();
    h.key(client.clone(), "k").await;
    h.key(client.clone(), "k").await;
    h.key(client.clone(), "n").await;
    h.wait_for(client.clone(), |f| f.contains("Name workspace-1"), WAIT)
        .await;

    h.api("workspace.rename", json!({"workspace": w2.as_str()}))
        .await
        .unwrap();

    let f = h
        .wait_for(client.clone(), |f| f.contains("Name workspace-2"), WAIT)
        .await;
    {
        let m = h.model();
        let view = m.client(&client).unwrap();
        assert_eq!(view.overlay, Some(Overlay::NameWorkspace(w2.clone())));
        assert_eq!(
            view.overlay_under,
            Some(Overlay::Switcher),
            "the switcher is still what one Esc goes back to:\n{f}"
        );
    }

    h.key(client.clone(), "Esc").await;
    let f = h
        .wait_for(client.clone(), |f| !f.contains("Name workspace-2"), WAIT)
        .await;
    let m = h.model();
    assert_eq!(
        m.client(&client).unwrap().overlay,
        Some(Overlay::Switcher),
        "{f}"
    );
    assert_eq!(h.model().workspace(&w1).unwrap().name, None);
    assert_eq!(h.model().workspace(&w2).unwrap().name, None);
}

/// The cursor row is the target only while the keys are in a Projects box. With the switcher
/// closed the cursor is still set, and `leader N` names the workspace the client is in.
///
/// That is the discriminating case for the rule: the two answers are a slot of `audrey-app`
/// and `main` of `proj`, and only a `leader N` that asks where the keys are can tell them
/// apart.
#[tokio::test]
async fn the_cursor_row_is_the_target_only_while_the_keys_are_in_a_box() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (_root, w1, _w2) = h.git_project_with_two_slots().await;
    let client = h.client.clone();
    let here = h.model().client(&client).unwrap().workspace.clone();
    assert_ne!(
        here, w1,
        "the client is not in the row the cursor will be on"
    );
    h.api("switcher.open", json!({"client": client.as_str()}))
        .await
        .unwrap();
    h.key(client.clone(), "k").await;
    h.key(client.clone(), "k").await;
    h.key(client.clone(), "Esc").await;
    h.wait_for(client.clone(), |f| !f.contains("Projects"), WAIT)
        .await;
    assert_eq!(
        h.model().client(&client).unwrap().projects_cursor.as_ref(),
        Some(&w1),
        "closing the switcher leaves the cursor where it was"
    );

    h.key(client.clone(), "C-a").await;
    h.key(client.clone(), "N").await;

    let f = h
        .wait_for(client.clone(), |f| f.contains("Name main"), WAIT)
        .await;
    let m = h.model();
    assert_eq!(
        m.client(&client).unwrap().overlay,
        Some(Overlay::NameWorkspace(here)),
        "the keys are in a pane, so the box names the workspace the client is in:\n{f}"
    );
}

/// A call on behalf of a client that is not attached opens no box and changes nothing.
#[tokio::test]
async fn opening_the_box_for_a_client_that_is_not_attached_refuses() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (_root, w1, _w2) = h.git_project_with_two_slots().await;
    let client = h.client.clone();

    let err = h
        .api(
            "workspace.rename",
            json!({"workspace": w1.as_str(), "client": "c_9999"}),
        )
        .await
        .unwrap_err();

    assert_eq!(err.code, ErrorCode::NotFound, "{}", err.message);
    let m = h.model();
    assert_eq!(
        m.client(&client).unwrap().overlay,
        None,
        "and no other client's screen took the box"
    );
    assert_eq!(h.model().workspace(&w1).unwrap().name, None);
}
