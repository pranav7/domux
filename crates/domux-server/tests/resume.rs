//! Persistence and resume: what survives a restart, and what a broken state file does.

use domux_core::config::Config;
use domux_server::testing::{row, Harness, HarnessOptions};
use serde_json::json;
use std::time::Duration;

#[tokio::test]
async fn state_json_is_written_after_structure_changes_with_the_current_schema_and_a_bak() {
    let mut h = Harness::start(Config::default(), 40, 10).await;
    h.api("tab.create", json!({})).await.unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains(" 2 "),
        Duration::from_secs(2),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(250)).await;
    let path = h.state_dir().join("state.json");
    let text = std::fs::read_to_string(&path).expect("state.json exists");
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    // Deliberately the literal 4, not `state_file::SCHEMA_VERSION`: this assertion exists to
    // pin the format contract, the number that actually reaches the author's disk. Comparing
    // against the constant would make it track a bump instead of catching one; a
    // `SCHEMA_VERSION` change with no new migration rung would go unnoticed here even though
    // domux-core's own tests would fail.
    assert_eq!(v["schema_version"], 5);
    assert_eq!(
        v["projects"][0]["workspaces"][0]["tabs"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    h.api("tab.rename", json!({"name": "x"})).await.unwrap();
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert!(h.state_dir().join("state.json.bak").exists());
    assert!(!h.state_dir().join("state.json.tmp").exists());
}

#[tokio::test]
async fn kill_and_restart_brings_back_tabs_panes_names_layout_and_cwds() {
    let mut h = Harness::start(Config::default(), 60, 12).await;
    let root = h.model().projects[0].root.clone();
    let sub = root.join("crates");
    std::fs::create_dir_all(&sub).unwrap();
    h.api("pane.split", json!({"dir": "right"})).await.unwrap();
    h.frame(h.client.clone()).await;
    let right = h.focused_pane(h.client.clone());
    // The shell in the right pane reports a new directory with OSC 7.
    h.feed_pane(
        right.clone(),
        format!("\x1b]7;file://localhost{}\x1b\\", sub.display()).as_bytes(),
    )
    .await;
    h.api("tab.rename", json!({"name": "code"})).await.unwrap();
    h.api("tab.create", json!({})).await.unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains(" 2 "),
        Duration::from_secs(2),
    )
    .await;
    h.api("tab.select", json!({"tab": "1"})).await.unwrap();
    h.frame(h.client.clone()).await;
    tokio::time::sleep(Duration::from_millis(1200)).await; // an inspector tick records the cwd
    let before = h.model();
    let spawned_before = h.spawner.as_ref().unwrap().requests().len();
    assert_eq!(spawned_before, 3);

    h.restart().await;

    let after = h.model();
    assert_eq!(
        serde_json::to_value(&after.projects).unwrap(),
        serde_json::to_value(&before.projects).unwrap(),
        "the persisted structure came back exactly (facts are skipped by serde)"
    );
    let f = h.frame(h.client.clone()).await;
    assert_eq!(
        row(&f, 0),
        "| proj › main  1 code │ 2 │ + │          14:32   Fri 4 Sep ● |",
        "{f}"
    );
    assert_eq!(
        f.matches('┌').count(),
        2,
        "two panes on the restored current tab"
    );
    let requests = h.spawner.as_ref().unwrap().requests();
    assert_eq!(requests.len(), 6, "three shells respawned");
    let respawned: Vec<_> = requests[3..].iter().collect();
    let for_right = respawned
        .iter()
        .find(|r| r.pane == right)
        .expect("the right pane kept its id");
    assert_eq!(for_right.cwd, sub);
    assert!(for_right
        .env
        .iter()
        .any(|(k, v)| k == "DOMUX_PANE" && *v == right.to_string()));
    assert!(for_right
        .env
        .iter()
        .any(|(k, v)| k == "DOMUX_SOCKET" && v.ends_with("domux.sock")));
    assert_eq!(
        h.model().client(&h.client).unwrap().tab,
        before
            .last_workspace
            .as_ref()
            .and_then(|w| before.workspace(w))
            .unwrap()
            .last_tab
            .clone()
            .unwrap(),
        "the client starts on the last tab"
    );
}

#[tokio::test]
async fn a_missing_directory_falls_back_to_the_workspace_path() {
    let mut h = Harness::start(Config::default(), 40, 10).await;
    let root = h.model().projects[0].root.clone();
    let pane = h.focused_pane(h.client.clone());
    h.feed_pane(
        pane.clone(),
        b"\x1b]7;file://localhost/nonexistent/dir\x1b\\",
    )
    .await;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    h.frame(h.client.clone()).await;
    assert_eq!(
        h.model().pane(&pane).unwrap().cwd,
        std::path::PathBuf::from("/nonexistent/dir")
    );
    h.restart().await;
    let requests = h.spawner.as_ref().unwrap().requests();
    assert_eq!(requests.last().unwrap().cwd, root);
    assert_eq!(
        h.model().pane(&pane).unwrap().cwd,
        root,
        "the model records the fallback"
    );
}

#[tokio::test]
async fn a_corrupt_state_file_starts_fresh_and_leaves_the_file_for_inspection() {
    let dir = tempfile::tempdir().unwrap();
    let state_dir = dir.path().join("state");
    std::fs::create_dir_all(&state_dir).unwrap();
    std::fs::write(state_dir.join("state.json"), "{not json").unwrap();
    let opts = HarnessOptions {
        state_dir: Some(state_dir.clone()),
        ..HarnessOptions::new(Config::default(), 40, 10)
    };
    let mut h = Harness::start_with(opts).await;
    let f = h.frame(h.client.clone()).await;
    assert_eq!(row(&f, 0), "| proj › main  1 │ + │ 14:32   Fri 4 … ● |");
    h.api("tab.create", json!({})).await.unwrap();
    h.api("tab.create", json!({})).await.unwrap();
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(
        std::fs::read_to_string(state_dir.join("state.json.rejected")).unwrap(),
        "{not json",
        "the bad file is moved aside at load, before any write can rotate it"
    );
}

#[tokio::test]
async fn real_shells_resume_in_their_directories() {
    let opts = HarnessOptions {
        real_ptys: true,
        ..HarnessOptions::new(Config::default(), 60, 12)
    };
    let mut h = Harness::start_with(opts).await;
    let root = h.model().projects[0].root.clone();
    let sub = root.join("deep");
    std::fs::create_dir_all(&sub).unwrap();
    let pane = h.focused_pane(h.client.clone());
    h.spawn_in_pane(
        pane.clone(),
        &[
            "cd",
            sub.to_str().unwrap(),
            "&&",
            "printf",
            "'\\033]7;file://localhost'\"$PWD\"'\\033\\\\'",
        ],
    )
    .await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        h.frame(h.client.clone()).await;
        if h.model().pane(&pane).unwrap().cwd.canonicalize().ok() == sub.canonicalize().ok() {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "cwd never updated: {:?}",
            h.model().pane(&pane).unwrap().cwd
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    h.restart().await;
    let pane_after = h.focused_pane(h.client.clone());
    assert_eq!(pane_after, pane);
    h.spawn_in_pane(pane.clone(), &["pwd"]).await;
    let sub_text = sub.canonicalize().unwrap().display().to_string();
    h.wait_for(
        h.client.clone(),
        |f| unwrapped(f).contains(&sub_text),
        Duration::from_secs(5),
    )
    .await;
}

/// A frame's rows joined back into one string, so a path the pane soft-wrapped is
/// contiguous again. A temp directory on macOS is longer than a 60-column pane is wide, so
/// `pwd` prints a path that arrives on screen in two pieces; a row that filled the pane has
/// no trailing spaces, so joining the rows as they are puts the pieces back together.
fn unwrapped(frame: &str) -> String {
    frame
        .lines()
        .filter(|l| l.starts_with('|'))
        .map(|l| l.trim_matches('|').trim_matches('\u{2502}'))
        .collect()
}

/// Runs a server on `state_dir`, names its one tab, and stops it so the state file is on
/// disk. The harness comes back so the temp project root the file names outlives the call:
/// a state file pointing at a deleted directory would take the cwd fallback instead.
async fn a_saved_state(state_dir: &std::path::Path, tab_name: &str) -> Harness {
    let opts = HarnessOptions {
        state_dir: Some(state_dir.to_path_buf()),
        ..HarnessOptions::new(Config::default(), 40, 10)
    };
    let mut h = Harness::start_with(opts).await;
    h.api("tab.rename", json!({ "name": tab_name }))
        .await
        .unwrap();
    h.frame(h.client.clone()).await;
    h.stop().await;
    h
}

/// A second server on the same state directory, with its own socket and its own project
/// root, so nothing but the state file carries over.
async fn a_server_on(state_dir: &std::path::Path) -> Harness {
    Harness::start_with(HarnessOptions {
        state_dir: Some(state_dir.to_path_buf()),
        ..HarnessOptions::new(Config::default(), 40, 10)
    })
    .await
}

fn state_json(state_dir: &std::path::Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(state_dir.join("state.json")).unwrap()).unwrap()
}

fn tab_names(h: &Harness) -> Vec<Option<String>> {
    h.model().projects[0].workspaces[0]
        .tabs
        .iter()
        .map(|t| t.name.clone())
        .collect()
}

/// The realistic damage of a write that was not atomic: a file cut off part way. It reads
/// as absent, not as an empty structure that was really saved, and it is kept.
#[tokio::test]
async fn a_truncated_state_file_starts_fresh_and_keeps_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let state_dir = dir.path().join("state");
    let _first = a_saved_state(&state_dir, "code").await;
    let whole = std::fs::read_to_string(state_dir.join("state.json")).unwrap();
    let cut = &whole[..whole.len() / 2];
    std::fs::write(state_dir.join("state.json"), cut).unwrap();

    let mut h = a_server_on(&state_dir).await;
    assert_eq!(tab_names(&h), vec![None], "a fresh tab, not the saved one");
    // Two structure changes, because one was never the problem. `write_atomic` rotates the
    // current file into `.bak` on every write, so a refused file left in place survives the
    // first write and is destroyed by the second. It is moved somewhere nothing rotates.
    h.api("tab.create", json!({})).await.unwrap();
    h.api("tab.create", json!({})).await.unwrap();
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(
        std::fs::read_to_string(state_dir.join("state.json.rejected")).unwrap(),
        cut,
        "the truncated file is kept for inspection, past any number of writes"
    );
}

/// A file this build does not know how to read is refused rather than read with today's
/// field names. Starting fresh then overwrites it, so it has to survive somewhere the
/// writer never rotates.
#[tokio::test]
async fn a_state_file_from_a_newer_domux_starts_fresh_and_keeps_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let state_dir = dir.path().join("state");
    let _first = a_saved_state(&state_dir, "code").await;
    let mut v = state_json(&state_dir);
    v["schema_version"] = json!(99);
    let newer = serde_json::to_string_pretty(&v).unwrap();
    std::fs::write(state_dir.join("state.json"), &newer).unwrap();

    let mut h = a_server_on(&state_dir).await;
    assert_eq!(tab_names(&h), vec![None], "a fresh tab, not the saved one");
    h.api("tab.create", json!({})).await.unwrap();
    h.api("tab.create", json!({})).await.unwrap();
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(
        std::fs::read_to_string(state_dir.join("state.json.rejected")).unwrap(),
        newer,
        "the newer file is kept rather than overwritten away, past any number of writes"
    );
}

/// `.bak` is the previous good file, never a source. A newer one is not preferred, or a
/// restart would resurrect the state before the last change.
#[tokio::test]
async fn a_bak_newer_than_the_state_file_is_not_read_in_its_place() {
    let dir = tempfile::tempdir().unwrap();
    let state_dir = dir.path().join("state");
    let _first = a_saved_state(&state_dir, "kept").await;
    let stale = std::fs::read_to_string(state_dir.join("state.json"))
        .unwrap()
        .replace("\"kept\"", "\"stale\"");
    // Written after the state file, so its modification time is the later of the two.
    std::fs::write(state_dir.join("state.json.bak"), &stale).unwrap();

    let h = a_server_on(&state_dir).await;
    assert_eq!(tab_names(&h), vec![Some("kept".into())]);
}

/// `last_workspace` names a workspace the file does not hold. The client is seated in one
/// that exists rather than refused with nowhere to go.
#[tokio::test]
async fn a_last_workspace_that_is_gone_seats_the_client_in_a_workspace_that_exists() {
    let dir = tempfile::tempdir().unwrap();
    let state_dir = dir.path().join("state");
    let _first = a_saved_state(&state_dir, "code").await;
    let mut v = state_json(&state_dir);
    v["last_workspace"] = json!("w_dead");
    std::fs::write(
        state_dir.join("state.json"),
        serde_json::to_string_pretty(&v).unwrap(),
    )
    .unwrap();

    let h = a_server_on(&state_dir).await;
    let real = h.model().projects[0].workspaces[0].id.clone();
    assert_eq!(h.model().client(&h.client).unwrap().workspace, real);
    assert_eq!(tab_names(&h), vec![Some("code".into())], "the tab survived");
}

/// `last_tab` names a tab the workspace does not hold. The client starts on a tab that
/// exists: a client that could not be seated cannot attach at all, so a stale id here
/// would leave the server unusable with no way back but deleting the file.
#[tokio::test]
async fn a_last_tab_that_is_gone_seats_the_client_on_a_tab_that_exists() {
    let dir = tempfile::tempdir().unwrap();
    let state_dir = dir.path().join("state");
    let _first = a_saved_state(&state_dir, "code").await;
    let mut v = state_json(&state_dir);
    v["projects"][0]["workspaces"][0]["last_tab"] = json!("t_dead");
    std::fs::write(
        state_dir.join("state.json"),
        serde_json::to_string_pretty(&v).unwrap(),
    )
    .unwrap();

    let mut h = a_server_on(&state_dir).await;
    let real = h.model().projects[0].workspaces[0].tabs[0].id.clone();
    assert_eq!(h.model().client(&h.client).unwrap().tab, real);
    // The name costs the top bar five columns, so at 40 the date truncates. Both rows are
    // 40 columns wide: the restored name is what this pins, not the truncation.
    assert_eq!(
        row(&h.frame(h.client.clone()).await, 0),
        "| proj › main  1 code │ + │ 14:32   F… ● |"
    );
}
