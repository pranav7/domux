//! `config.reload`: a good file replaces the config and the keymap, a bad one keeps the
//! previous good config and says which line to fix until it is fixed.

use domux_core::config::Config;
use domux_server::testing::{row, Harness};
use serde_json::json;
use std::time::Duration;

#[tokio::test]
async fn a_good_reload_changes_the_leader_and_the_hint_follows() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    std::fs::write(h.config_path(), "[keys]\nleader = \"C-b\"\n").unwrap();
    let r = h.api("config.reload", json!({})).await.unwrap();
    assert!(r["error"].is_null(), "{r}");
    h.key(h.client.clone(), "C-b").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("C-b  ? keys"),
            Duration::from_secs(2),
        )
        .await;
    assert!(!f.contains("C-a"), "{f}");
    h.key(h.client.clone(), "c").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains(" 2 "),
        Duration::from_secs(2),
    )
    .await;
}

#[tokio::test]
async fn a_bad_reload_keeps_the_old_config_and_shows_the_line_until_fixed() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    std::fs::write(
        h.config_path(),
        "[keys]\nleader = \"C-b\"\n[terminal]\nscrollback = \n",
    )
    .unwrap();
    let r = h.api("config.reload", json!({})).await.unwrap();
    assert_eq!(
        r["error"].as_str().unwrap().split(':').next().unwrap(),
        "domux.toml line 4"
    );
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("domux.toml line 4"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        f.contains("domux2 config reload"),
        "the next action is on screen:\n{f}"
    );
    assert!(!f.contains("14:32"), "{f}");
    let info = h.api("server.info", json!({})).await.unwrap();
    assert!(info["config_error"]
        .as_str()
        .unwrap()
        .starts_with("domux.toml line 4"));
    // The old leader still works: the previous good config stayed.
    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), "c").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains(" 2 "),
        Duration::from_secs(2),
    )
    .await;
    std::fs::write(h.config_path(), "[keys]\nleader = \"C-b\"\n").unwrap();
    h.api("config.reload", json!({})).await.unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains("14:32"),
        Duration::from_secs(2),
    )
    .await;
}

#[tokio::test]
async fn a_bad_file_at_startup_uses_defaults_and_shows_the_notice() {
    let dir = tempfile::tempdir().unwrap();
    let state_dir = dir.path().join("state");
    std::fs::create_dir_all(&state_dir).unwrap();
    std::fs::write(state_dir.join("domux.toml"), "[keys\nleader = 1\n").unwrap();
    let opts = domux_server::testing::HarnessOptions {
        state_dir: Some(state_dir),
        ..domux_server::testing::HarnessOptions::new(Config::default(), 80, 10)
    };
    let mut h = Harness::start_with(opts).await;
    let f = h.frame(h.client.clone()).await;
    assert!(f.contains("domux.toml line 1"), "{f}");
    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), "c").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains(" 2 "),
        Duration::from_secs(2),
    )
    .await;
}

#[tokio::test]
async fn unknown_tables_are_warnings_in_the_reload_result_not_errors() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    std::fs::write(h.config_path(), "[worktrees]\nbase = \"origin/main\"\n").unwrap();
    let r = h.api("config.reload", json!({})).await.unwrap();
    assert!(r["error"].is_null());
    assert_eq!(
        r["warnings"][0],
        "unknown table [worktrees] (line 1) is ignored until the milestone that reads it"
    );
}

/// The notice as it is drawn, cell for cell, at the width the tests run at. Its three parts
/// are the object (`domux.toml line 4`), the state (the parser's own words, cut to the room
/// with `…`) and the next action (`domux2 config reload`) - principle 9. The message is what
/// gives way when the room runs short; the next action never is.
#[tokio::test]
async fn the_notice_names_the_file_the_line_and_the_next_action_in_the_room_it_has() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    std::fs::write(
        h.config_path(),
        "[keys]\nleader = \"C-b\"\n[terminal]\nscrollback = \n",
    )
    .unwrap();
    h.api("config.reload", json!({})).await.unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("domux.toml line 4"),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(
        row(&f, 0),
        "| proj › main  1 │ + │ domux.toml line 4: invalid string… · domux2 config reload |"
    );
    // The same notice whole when there is room for it: the message is elastic, not truncated
    // at the source.
    let wide = h.attach(120, 10).await;
    let f = h
        .wait_for(
            wide,
            |f| f.contains("domux.toml line 4"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        row(&f, 0).ends_with(
            "domux.toml line 4: invalid string; expected `\"`, `\'` · domux2 config reload |"
        ),
        "{f}"
    );
}

/// A file that parses but says something the keymap cannot use. toml found nothing wrong, so
/// there is no span and no line - and the notice says `domux.toml:` rather than sending the
/// reader to line 1, which is not where the mistake is.
#[tokio::test]
async fn a_config_that_parses_but_is_not_a_keymap_keeps_the_old_config_and_invents_no_line() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    std::fs::write(
        h.config_path(),
        "# a comment\n[keys]\nleader = \"Ctrl-Q\"\n",
    )
    .unwrap();
    let r = h.api("config.reload", json!({})).await.unwrap();
    let error = r["error"].as_str().expect("an error");
    assert!(
        error.starts_with("domux.toml: [keys] leader: unknown key \"Ctrl-Q\""),
        "{error}"
    );
    assert!(
        !error.contains("line "),
        "no line arrived, so none is shown: {error}"
    );
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("domux.toml:"),
            Duration::from_secs(2),
        )
        .await;
    assert!(f.contains("domux2 config reload"), "{f}");
    // The previous good config stayed: the old leader still opens the chord.
    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), "c").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains(" 2 "),
        Duration::from_secs(2),
    )
    .await;
}

/// The file is gone by the time the reload reads it. No file is not an error - a fresh install
/// has none - so the reload succeeds with the defaults and the notice goes.
#[tokio::test]
async fn a_config_file_that_vanished_reloads_the_defaults_and_clears_the_notice() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    std::fs::write(h.config_path(), "[keys\nleader = \"C-b\"\n").unwrap();
    let r = h.api("config.reload", json!({})).await.unwrap();
    assert!(r["error"]
        .as_str()
        .unwrap()
        .starts_with("domux.toml line 1"));
    h.wait_for(
        h.client.clone(),
        |f| f.contains("domux.toml line 1"),
        Duration::from_secs(2),
    )
    .await;
    std::fs::remove_file(h.config_path()).unwrap();
    let r = h.api("config.reload", json!({})).await.unwrap();
    assert!(r["error"].is_null(), "{r}");
    h.wait_for(
        h.client.clone(),
        |f| f.contains("14:32"),
        Duration::from_secs(2),
    )
    .await;
    let info = h.api("server.info", json!({})).await.unwrap();
    assert!(info["config_error"].is_null(), "{info}");
    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), "c").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains(" 2 "),
        Duration::from_secs(2),
    )
    .await;
}

/// A string that is never closed, so the error's span sits at the end of the input. The line
/// is the file's last line, not one past it and not the first.
#[tokio::test]
async fn a_syntax_error_at_the_end_of_the_file_names_the_last_line() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    std::fs::write(h.config_path(), "[keys]\nleader = \"C-b").unwrap();
    let r = h.api("config.reload", json!({})).await.unwrap();
    assert_eq!(
        r["error"].as_str().unwrap(),
        "domux.toml line 2: invalid basic string"
    );
}

/// The path is there but nothing can be read from it. There is no line in a file that never
/// opened, so the notice names the file and what happened to it.
#[tokio::test]
async fn a_config_that_cannot_be_read_says_so_without_a_line() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    std::fs::create_dir(h.config_path()).unwrap();
    let r = h.api("config.reload", json!({})).await.unwrap();
    let error = r["error"].as_str().expect("an error");
    assert!(
        error.starts_with("domux.toml: could not read the file:"),
        "{error}"
    );
    // The old config stayed, so the server is still usable while the path is wrong.
    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), "c").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains(" 2 "),
        Duration::from_secs(2),
    )
    .await;
}

/// A reload landing between the leader and the key it opened. The core handles both in turn,
/// so the chord finishes against the keymap in force when its second key arrives, and the new
/// leader is what opens the next one.
#[tokio::test]
async fn a_reload_in_the_middle_of_a_chord_finishes_it_and_then_uses_the_new_leader() {
    let mut h = Harness::start(Config::default(), 80, 10).await;
    h.key(h.client.clone(), "C-a").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("C-a  ? keys"),
        Duration::from_secs(2),
    )
    .await;
    std::fs::write(h.config_path(), "[keys]\nleader = \"C-b\"\n").unwrap();
    let r = h.api("config.reload", json!({})).await.unwrap();
    assert!(r["error"].is_null(), "{r}");
    h.key(h.client.clone(), "c").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains(" 2 "),
        Duration::from_secs(2),
    )
    .await;
    h.key(h.client.clone(), "C-b").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("C-b  ? keys"),
        Duration::from_secs(2),
    )
    .await;
}

/// A reload does more than reload: it is the only way out of a respawn block. The guard stops
/// replacing a workspace's pane when its shell keeps exiting at once, and the notice it leaves
/// names `terminal.shell` - which is only an action if setting it has an effect short of a
/// restart. A reload that failed lifts nothing: the config that tripped the guard is still the
/// one in force.
#[tokio::test]
async fn a_reload_is_the_way_out_of_a_respawn_block_and_a_failed_one_is_not() {
    let mut h = Harness::start(Config::default(), 100, 10).await;
    // The workspace's only pane exits the moment it starts, over and over, until the guard
    // stops replacing it.
    for _ in 0..5 {
        let pane = h.focused_pane(h.client.clone());
        h.exit_pane(pane, Some(1)).await;
        h.frame(h.client.clone()).await;
    }
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("exited immediately"),
            Duration::from_secs(2),
        )
        .await;
    assert!(f.contains("terminal.shell"), "{f}");
    let blocked = h.model().all_pane_ids().len();

    std::fs::write(h.config_path(), "[keys\nleader = \"C-b\"\n").unwrap();
    let r = h.api("config.reload", json!({})).await.unwrap();
    assert!(r["error"]
        .as_str()
        .unwrap()
        .starts_with("domux.toml line 1"));
    // The bar shows the config error now, which outranks the shell notice - see
    // `a_config_error_outranks_the_shell_failure_notice`. What says the block was not lifted is
    // the pane still retained rather than replaced.
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("domux.toml line 1"),
            Duration::from_secs(2),
        )
        .await;
    assert!(!f.contains("exited immediately"), "{f}");
    assert_eq!(
        h.model().all_pane_ids().len(),
        blocked,
        "a reload that failed lifts no block"
    );

    std::fs::write(h.config_path(), "[terminal]\nremain_on_exit = false\n").unwrap();
    let r = h.api("config.reload", json!({})).await.unwrap();
    assert!(r["error"].is_null(), "{r}");
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("14:32"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        !f.contains("exited immediately"),
        "both notices are withdrawn:\n{f}"
    );
}

/// Two true things at once: the shell keeps exiting and the file the user just saved does not
/// parse. The config error is what the bar shows (ruled 2026-09-07).
///
/// It is the newer of the two - their last edit was rejected, so the config in force is still
/// the old one - and it is a prerequisite for the other: `terminal.shell` cannot be set until
/// the file parses at all. Hidden behind the shell notice, the reader would believe they had
/// fixed the shell, find it still broken, and have no way on screen to learn why.
#[tokio::test]
async fn a_config_error_outranks_the_shell_failure_notice() {
    let mut h = Harness::start(Config::default(), 100, 10).await;
    for _ in 0..5 {
        let pane = h.focused_pane(h.client.clone());
        h.exit_pane(pane, Some(1)).await;
        h.frame(h.client.clone()).await;
    }
    h.wait_for(
        h.client.clone(),
        |f| f.contains("exited immediately"),
        Duration::from_secs(2),
    )
    .await;
    let blocked = h.model().all_pane_ids().len();
    std::fs::write(h.config_path(), "[keys\nleader = \"C-b\"\n").unwrap();
    let r = h.api("config.reload", json!({})).await.unwrap();
    assert!(r["error"]
        .as_str()
        .unwrap()
        .starts_with("domux.toml line 1"));
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("domux.toml line 1"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        !f.contains("exited immediately"),
        "the config error has the bar:\n{f}"
    );
    assert!(f.contains("domux2 config reload"), "{f}");
    // Both notices are still true: a reload that failed lifts no respawn block.
    assert_eq!(h.model().all_pane_ids().len(), blocked);
}

/// Three things the bar keeps at the widths where the notice does not fit whole.
///
/// The tab row fills its budget to the last cell whenever it is cut, so without a reserved gap
/// its own elision mark abuts the right end's and the two read as one run of text. The message
/// is elided to a mark rather than dropped, because a right end showing only `domux2 config
/// reload` reads as an offer rather than as an error, and the mark carries the message's own
/// red. Its separator narrows with it, because a right end that opens with a bare ` · ` reads
/// as a sentence with its subject cut off. And the right end stays flush to the edge on the
/// width it came back with, not on the cells reserved for it: an elastic piece shrinks below
/// its reservation, and a notice floating short of the edge reads as a label dropped mid-bar.
#[tokio::test]
async fn a_config_notice_too_wide_to_fit_keeps_a_gap_a_mark_and_the_right_edge() {
    // Both shapes of tab row: one tab too wide for the row, and more tabs than fit. The second
    // is what floats the right end - the message shrinks further than the cells reserved for
    // it, so a right end placed at its reservation stops short of the edge.
    for (cols, tabs) in [
        // 74 is the width where the message shrinks a cell further than the room reserved for
        // it, so it is the one that catches a right end placed at its reservation.
        (74u16, 1usize),
        (74, 9),
        (70, 1),
        (60, 1),
        (55, 1),
        (50, 1),
        (46, 1),
        (42, 1),
        (70, 9),
        (60, 9),
        (50, 9),
    ] {
        let mut h = Harness::start(Config::default(), cols, 10).await;
        for i in 1..tabs {
            h.api("tab.create", json!({})).await.unwrap();
            h.api("tab.rename", json!({"name": format!("long name {i}")}))
                .await
                .unwrap();
        }
        h.api("tab.rename", json!({"name": "a very long tab name indeed"}))
            .await
            .unwrap();
        std::fs::write(h.config_path(), "[keys\nleader = \"C-b\"\n").unwrap();
        let r = h.api("config.reload", json!({})).await.unwrap();
        assert!(r["error"]
            .as_str()
            .unwrap()
            .starts_with("domux.toml line 1"));
        let f = h
            .wait_for(
                h.client.clone(),
                |f| row(f, 0).contains("domux"),
                Duration::from_secs(2),
            )
            .await;
        let bar = row(&f, 0);
        assert!(!bar.contains("……"), "{cols} columns, {tabs} tabs: {bar}");
        // The narrowest of these widths cuts the action itself, so this is `domux` and not
        // `domux2 config reload`: what is pinned is the shape of the right end, not its length.
        assert!(
            bar.contains("… domux"),
            "{cols} columns, {tabs} tabs: {bar}"
        );
        assert!(
            !bar.contains(" · domux"),
            "{cols} columns, {tabs} tabs: {bar}"
        );
        // `row` returns the screen row between two `|` delimiters. One blank column at the
        // right edge is the bar's own, and there is only one: the right end sits flush on the
        // width it came back with, not on the cells that were reserved for it.
        let inner = bar
            .strip_prefix('|')
            .and_then(|b| b.strip_suffix('|'))
            .unwrap();
        assert_eq!(
            domux_core::text::display_width(inner) as u16,
            cols,
            "{cols} columns, {tabs} tabs: {bar}"
        );
        assert!(
            inner.ends_with(' ') && !inner.ends_with("  "),
            "{cols} columns, {tabs} tabs: {bar}"
        );
        h.stop().await;
    }
}
