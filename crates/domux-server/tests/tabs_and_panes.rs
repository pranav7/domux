//! The tab and pane methods, each one checked by the frame it produces.

use domux_core::config::Config;
use domux_server::testing::{row, Harness};
use serde_json::json;
use std::time::Duration;

#[tokio::test]
async fn split_right_draws_two_boxes_edge_to_edge_and_focuses_the_new_pane() {
    let mut h = Harness::start(Config::default(), 40, 10).await;
    let first = h.focused_pane(h.client.clone());
    let info = h.api("pane.split", json!({"dir": "right"})).await.unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.matches('┌').count() == 2,
            Duration::from_secs(2),
        )
        .await;
    // The two boxes touch, and 40 columns divide evenly between them.
    assert_eq!(
        row(&f, 1),
        "|┌ sh ──────────────┐┌ sh ──────────────┐|",
        "{f}"
    );
    // Both boxes stand on the screen's last row, so neither draws a bottom rule and both
    // give that row to their program (MUX-14).
    assert_eq!(
        row(&f, 9),
        "|│                  ││                  │|",
        "{f}"
    );
    assert!(
        f.contains("r1 c25-39 fg=#cba6f7"),
        "the right box is focused:\n{f}"
    );
    assert!(
        f.contains("r1 c0-0 fg=#585b70"),
        "the left box is plain:\n{f}"
    );
    assert_eq!(info["focused"], true);
    assert_ne!(info["id"], first.as_str());
    assert_eq!(
        h.model().pane(&first).unwrap().cwd,
        h.model()
            .pane(&domux_core::ids::PaneId(
                info["id"].as_str().unwrap().into()
            ))
            .unwrap()
            .cwd,
        "the new shell starts in the split pane's cwd"
    );
}

#[tokio::test]
async fn split_down_stacks_with_the_boxes_touching() {
    let mut h = Harness::start(Config::default(), 40, 10).await;
    h.api("pane.split", json!({"dir": "down"})).await.unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.matches('┌').count() == 2,
            Duration::from_secs(2),
        )
        .await;
    // 9 rows do not divide evenly, so the first box takes the odd one.
    assert_eq!(row(&f, 1), "|┌ sh ──────────────────────────────────┐|");
    // The upper box keeps its bottom rule: that rule is what parts it from the box under it.
    assert_eq!(row(&f, 5), "|└──────────────────────────────────────┘|");
    assert_eq!(
        row(&f, 6),
        "|┌ sh ──────────────────────────────────┐|",
        "the next box starts on the row below, with nothing between them"
    );
    // The lower one stands on the screen's last row and does not (MUX-14).
    assert_eq!(
        row(&f, 9),
        "|│                                      │|",
        "{f}"
    );
}

#[tokio::test]
async fn zoom_takes_the_whole_workpanel_and_shows_the_flag() {
    let mut h = Harness::start(Config::default(), 40, 10).await;
    h.api("pane.split", json!({"dir": "right"})).await.unwrap();
    let r = h.api("pane.zoom", json!({})).await.unwrap();
    assert!(r["zoomed"].is_string());
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("zoomed"),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(
        row(&f, 1),
        "|┌ sh ────────────────────────── zoomed ┐|",
        "{f}"
    );
    assert_eq!(f.matches('┌').count(), 1);
    let r = h.api("pane.zoom", json!({})).await.unwrap();
    assert!(r["zoomed"].is_null());
    h.wait_for(
        h.client.clone(),
        |f| f.matches('┌').count() == 2,
        Duration::from_secs(2),
    )
    .await;
}

#[tokio::test]
async fn tab_create_select_rename_and_close_update_the_tab_row() {
    let mut h = Harness::start(Config::default(), 60, 10).await;
    h.api("tab.create", json!({})).await.unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains(" 2 "),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(
        row(&f, 0),
        "| proj › main  1 │ 2 │ + │                 14:32   Fri 4 Sep |",
        "{f}"
    );
    assert!(
        f.contains("r0 c17-19 bold fg=#1e1e2e bg=#cba6f7"),
        "tab 2 is current:\n{f}"
    );
    h.api("tab.rename", json!({"name": "tests"})).await.unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("2 tests"),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(
        row(&f, 0),
        "| proj › main  1 │ 2 tests │ + │           14:32   Fri 4 Sep |"
    );
    h.api("tab.select", json!({"tab": "1"})).await.unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("r0 c13-15 bold"),
            Duration::from_secs(2),
        )
        .await;
    assert!(
        f.contains("r0 c17-25 fg=#7f849c bg=#181825"),
        "tab 2 is dim:\n{f}"
    );
    h.api("tab.clear_name", json!({"tab": "2"})).await.unwrap();
    h.wait_for(
        h.client.clone(),
        |f| !f.contains("tests"),
        Duration::from_secs(2),
    )
    .await;
    h.api("tab.close", json!({"tab": "2"})).await.unwrap();
    let f = h
        .wait_for(
            h.client.clone(),
            |f| !f.contains(" 2 "),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(
        row(&f, 0),
        "| proj › main  1 │ + │                     14:32   Fri 4 Sep |"
    );
    let err = h.api("tab.select", json!({"tab": "5"})).await.unwrap_err();
    assert_eq!(
        err.message,
        "tab 5 does not exist; this workspace has 1 tabs"
    );
}

#[tokio::test]
async fn closing_the_last_pane_in_a_tab_closes_the_tab_and_the_last_tab_is_replaced() {
    let mut h = Harness::start(Config::default(), 40, 10).await;
    let first_tab = h.current_tab(h.client.clone());
    let first_pane = h.focused_pane(h.client.clone());
    h.api("tab.create", json!({})).await.unwrap();
    h.wait_for(
        h.client.clone(),
        |f| f.contains(" 2 "),
        Duration::from_secs(2),
    )
    .await;
    let second_pane = h.focused_pane(h.client.clone());
    h.exit_pane(second_pane.clone(), Some(0)).await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| !f.contains(" 2 "),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(
        h.current_tab(h.client.clone()),
        first_tab,
        "the client moved to the surviving tab"
    );
    assert_eq!(row(&f, 0), "| proj › main  1 │ + │ 14:32   Fri 4 Sep |");
    h.exit_pane(first_pane.clone(), Some(0)).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let f = h.frame(h.client.clone()).await;
    assert_eq!(
        row(&f, 0),
        "| proj › main  1 │ + │ 14:32   Fri 4 Sep |",
        "a fresh tab replaced the last one:\n{f}"
    );
    assert_ne!(h.focused_pane(h.client.clone()), first_pane);
    assert_eq!(
        h.spawner.as_ref().unwrap().requests().len(),
        3,
        "first shell, tab 2 shell, replacement shell"
    );
}

#[tokio::test]
async fn remain_on_exit_keeps_the_pane_with_an_exit_flag() {
    let mut cfg = Config::default();
    cfg.terminal.remain_on_exit = true;
    let mut h = Harness::start(cfg, 40, 10).await;
    let pane = h.focused_pane(h.client.clone());
    h.exit_pane(pane.clone(), Some(3)).await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("exited 3"),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(
        row(&f, 1),
        "|┌ sh ──────────────────────── exited 3 ┐|",
        "{f}"
    );
    // Task 17 makes Enter close an exited pane; until then the API does.
    h.api("pane.close", json!({"pane": pane.as_str()}))
        .await
        .unwrap();
    h.wait_for(
        h.client.clone(),
        |f| !f.contains("exited 3"),
        Duration::from_secs(2),
    )
    .await;
    assert_ne!(
        h.focused_pane(h.client.clone()),
        pane,
        "a fresh shell replaced the last pane"
    );
}

#[tokio::test]
async fn send_text_send_key_and_read_go_through_the_pane() {
    let mut h = Harness::start(Config::default(), 40, 10).await;
    let pane = h.focused_pane(h.client.clone());
    h.api("pane.send_text", json!({"text": "ls"}))
        .await
        .unwrap();
    h.api("pane.send_key", json!({"key": "Enter"}))
        .await
        .unwrap();
    h.api("pane.send_key", json!({"key": "C-c"})).await.unwrap();
    h.frame(h.client.clone()).await;
    assert_eq!(h.pane_input(&pane), b"ls\r\x03".to_vec());
    h.feed_pane(pane.clone(), b"one\r\ntwo\r\nthree").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("three"),
        Duration::from_secs(2),
    )
    .await;
    let r = h.api("pane.read", json!({"lines": 2})).await.unwrap();
    assert_eq!(r["text"], "two\nthree");
    let err = h
        .api("pane.send_key", json!({"key": "Ctrl-c"}))
        .await
        .unwrap_err();
    assert_eq!(err.code, domux_core::api::ErrorCode::InvalidParams);
}

/// Task 15 checks the same fact from the other side, through `Harness::pane_size`. The two
/// paths are checked separately on purpose: one reads what the core published beside the
/// model, this one reads what the API answers.
#[tokio::test]
async fn pane_list_reports_the_size_the_smallest_client_gives_each_pane() {
    let mut h = Harness::start(Config::default(), 60, 20).await;
    let second = h.attach(40, 10).await;
    let pane = h.focused_pane(h.client.clone());
    h.frame(second.clone()).await;
    let list = h.api("pane.list", json!({})).await.unwrap();
    let panes = list.as_array().unwrap();
    assert_eq!(panes.len(), 1, "{list}");
    assert_eq!(panes[0]["id"], pane.as_str());
    assert_eq!(panes[0]["focused"], true);
    assert_eq!(panes[0]["cols"].as_u64(), Some(38), "{list}");
    assert_eq!(panes[0]["rows"].as_u64(), Some(8), "{list}");
    let published = h.pane_size(&pane);
    assert_eq!(
        (panes[0]["cols"].as_u64(), panes[0]["rows"].as_u64()),
        (Some(published.cols as u64), Some(published.rows as u64)),
        "the API and the published sizes agree"
    );
}

#[tokio::test]
async fn focus_moves_by_geometry_and_last_toggles() {
    let mut h = Harness::start(Config::default(), 60, 12).await;
    let a = h.focused_pane(h.client.clone());
    h.api("pane.split", json!({"dir": "right"})).await.unwrap();
    h.frame(h.client.clone()).await;
    let b = h.focused_pane(h.client.clone());
    h.api("pane.split", json!({"dir": "down"})).await.unwrap();
    h.frame(h.client.clone()).await;
    let c = h.focused_pane(h.client.clone());
    h.api("focus.left", json!({})).await.unwrap();
    h.frame(h.client.clone()).await;
    assert_eq!(h.focused_pane(h.client.clone()), a);
    h.api("focus.right", json!({})).await.unwrap();
    h.frame(h.client.clone()).await;
    assert_eq!(
        h.focused_pane(h.client.clone()),
        b,
        "equal overlap and distance: the first in reading order wins"
    );
    h.api("focus.down", json!({})).await.unwrap();
    h.frame(h.client.clone()).await;
    assert_eq!(h.focused_pane(h.client.clone()), c);
    h.api("focus.last", json!({})).await.unwrap();
    h.frame(h.client.clone()).await;
    assert_eq!(h.focused_pane(h.client.clone()), b);
    let r = h.api("focus.up", json!({})).await.unwrap();
    assert_eq!(
        r["focus"]["kind"], "pane",
        "no pane above: focus stays and the call still answers"
    );
}

#[tokio::test]
async fn client_detach_sends_detached_and_removes_the_client() {
    let mut h = Harness::start(Config::default(), 40, 10).await;
    let second = h.attach(40, 10).await;
    h.api("client.detach", json!({"client": second.as_str()}))
        .await
        .unwrap();
    assert_eq!(
        h.detached_reason(second.clone()).await.as_deref(),
        Some("detached")
    );
    let info = h.api("server.info", json!({})).await.unwrap();
    assert_eq!(info["clients"].as_array().unwrap().len(), 1);
}

/// `Ack.ok` says the call ran, never whether the geometry moved. A single pane has no split
/// to resize, so `Model::resize_pane` answers false and the boxes do not move - and the
/// response is still `ok: true`, because the request was valid and was carried out.
#[tokio::test]
async fn resize_answers_ok_even_when_no_split_owns_the_axis() {
    let mut h = Harness::start(Config::default(), 40, 10).await;
    let before = h.frame(h.client.clone()).await;
    let r = h
        .api("pane.resize", json!({"dir": "right", "cells": 5}))
        .await
        .unwrap();
    assert_eq!(r["ok"], true, "{r}");
    let after = h.frame(h.client.clone()).await;
    assert_eq!(row(&after, 1), row(&before, 1), "nothing moved:\n{after}");
}

/// `pane.read` counts screen rows and answers in logical lines, which is not the same number.
/// A line the screen wrapped comes back as the one line it was written as (task 19's emulator
/// decision), so a caller reading a pane gets a URL it can follow rather than two halves of one.
/// `lines` bounds what is read, not what is returned.
#[tokio::test]
async fn pane_read_rejoins_a_line_the_screen_wrapped() {
    let mut h = Harness::start(Config::default(), 40, 10).await;
    let pane = h.focused_pane(h.client.clone());
    // 55 cells in a 38 cell pane: two screen rows, one line.
    let long = "https://example.com/a-path-long-enough-to-wrap";
    h.feed_pane(pane, format!("{long}\r\nend").as_bytes()).await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("end"),
        Duration::from_secs(2),
    )
    .await;
    let r = h.api("pane.read", json!({"lines": 3})).await.unwrap();
    assert_eq!(r["text"], format!("{long}\nend"));
}
