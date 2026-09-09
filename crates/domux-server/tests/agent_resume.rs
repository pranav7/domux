//! Resume: `agent.resume`, `workspace.resume`, `[resume] agents = "auto"`, and Enter on an
//! exited row. The line is typed into a shell in the agent's own pane and not run for it
//! (architecture spec section 5).

use domux_core::api::ErrorCode;
use domux_core::config::{Config, ResumeMode};
use domux_core::model::agent::{AgentKind, AgentState};
use domux_core::model::Overlay;
use domux_server::testing::{Harness, HarnessOptions};
use serde_json::json;
use std::time::Duration;

/// Longer than the core's one-second tick, so a sleep of this length has seen at least one.
/// The same constant `agent_observer.rs` uses, for the same reason.
const A_TICK: Duration = Duration::from_millis(1200);

/// A pill's background, as the frame's style dump spells it: green when it worked and red when
/// it did not (interface spec 7.3 and the theme table).
const PILL_OK: &str = "bg=#a6e3a1";
const PILL_REFUSED: &str = "bg=#f38ba8";

/// The frame row directly under the overlay's box, which is the footer.
///
/// Asked for by position rather than by searching the frame for the words, because the same
/// words are also in the top bar: a failed key puts its message in the hint there as well, so a
/// test that only asked whether the reason is somewhere on the screen would pass with no footer
/// at all. That is what this exists to tell apart.
fn footer_row(frame: &str) -> usize {
    frame
        .lines()
        .filter(|l| l.starts_with('|'))
        .position(|l| l.contains('└'))
        .map(|bottom| bottom + 1)
        .unwrap_or_else(|| panic!("no box bottom in:\n{frame}"))
}

/// The first column of the footer's pill: one inside the left edge of the Agents box, which is
/// the padding `overlay::footer` leaves. Read off the box's own top border rather than written
/// down, because the overlay is centred and the number moves with the screen width.
fn pill_col(frame: &str) -> usize {
    let top = frame
        .lines()
        .filter(|l| l.starts_with('|'))
        .find(|l| l.contains("┌ Agents"))
        .unwrap_or_else(|| panic!("no Agents box in:\n{frame}"));
    // The dump's leading `|` shifts every column by one, and the pill starts one cell inside the
    // border. The two cancel, so the border's index in the line is the pill's own column.
    top.chars().position(|c| c == '┌').expect("a top border")
}

fn row_text(frame: &str, y: usize) -> String {
    frame
        .lines()
        .filter(|l| l.starts_with('|'))
        .nth(y)
        .unwrap_or_else(|| panic!("no row {y} in:\n{frame}"))
        .to_string()
}

/// The style of one cell, out of the frame's own style dump, whose lines are
/// `r{row} c{from}-{to} [attrs] fg=# bg=#`. The same reader `agents_overlay.rs` uses.
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

/// One Claude record that started in `audrey-app` and ended, in the client's focused pane. The
/// directory comes from the payload, which is what the resume line has to `cd` to.
async fn an_exited_claude(h: &mut Harness) -> (domux_core::ids::PaneId, domux_core::ids::AgentId) {
    let pane = h.focused_pane(h.client.clone());
    h.report(pane.clone(), AgentKind::Claude, r#"{"hook_event_name":"SessionStart","session_id":"sess-1","cwd":"/Users/pranav/projects/audrey-app"}"#).await;
    h.report(
        pane.clone(),
        AgentKind::Claude,
        r#"{"hook_event_name":"SessionEnd","session_id":"sess-1"}"#,
    )
    .await;
    let id = h.agents().await[0].id.clone();
    (pane, id)
}

/// One exited record of `kind` in `pane`, by hook, with the session id given.
async fn an_exited(h: &mut Harness, pane: &domux_core::ids::PaneId, kind: AgentKind, sid: &str) {
    let (start, end) = start_and_end(kind);
    h.report(
        pane.clone(),
        kind,
        &format!(r#"{{"hook_event_name":"{start}","session_id":"{sid}"}}"#),
    )
    .await;
    h.report(
        pane.clone(),
        kind,
        &format!(r#"{{"hook_event_name":"{end}","session_id":"{sid}"}}"#),
    )
    .await;
}

/// The names one kind's own hooks give the start and the end of a session. Claude and Codex
/// share their spelling and OpenCode's plugin sends OpenCode's own (plan assumptions 13 and 14),
/// so a helper that sent Claude's names to all three would build no OpenCode record at all.
fn start_and_end(kind: AgentKind) -> (&'static str, &'static str) {
    match kind {
        AgentKind::Claude | AgentKind::Codex => ("SessionStart", "SessionEnd"),
        AgentKind::Opencode => ("session.created", "session.deleted"),
    }
}

/// The overlay this client has open, which is what says whether Enter left the list up.
fn overlay(h: &Harness) -> Option<Overlay> {
    h.model()
        .client(&h.client)
        .expect("the client is attached")
        .overlay
        .clone()
}

#[tokio::test]
async fn resume_types_v1s_line_into_the_agents_own_pane_and_does_not_run_it() {
    let mut h = Harness::start(Config::default(), 100, 24).await;
    let (pane, id) = an_exited_claude(&mut h).await;
    let result = h
        .api("agent.resume", json!({"agent": id.to_string()}))
        .await
        .unwrap();
    assert_eq!(result["pane"], pane.to_string());
    assert_eq!(result["agent"], id.to_string());
    assert_eq!(
        result["command"],
        "cd '/Users/pranav/projects/audrey-app' && command -v claude >/dev/null 2>&1 && claude --resume 'sess-1'"
    );
    let typed = String::from_utf8(h.pane_input(&pane)).unwrap();
    assert_eq!(
        typed,
        "cd '/Users/pranav/projects/audrey-app' && command -v claude >/dev/null 2>&1 && claude --resume 'sess-1'\r",
        "typed with one carriage return, so the shell has it and runs it as one line"
    );
}

/// The record stays exited. Only a hook can say a session is running, so a resume that typed a
/// line has not started an agent and must not claim to have (principle 4).
#[tokio::test]
async fn a_resumed_record_stays_exited_until_a_hook_says_otherwise() {
    let mut h = Harness::start(Config::default(), 100, 24).await;
    let (_, id) = an_exited_claude(&mut h).await;
    h.api("agent.resume", json!({"agent": id.to_string()}))
        .await
        .unwrap();
    let after = &h.agents().await[0];
    assert_eq!(after.id, id);
    assert_eq!(after.state, AgentState::Exited);
    assert_eq!(after.session_id.as_deref(), Some("sess-1"));
}

#[tokio::test]
async fn a_session_id_with_a_quote_in_it_is_quoted_safely() {
    assert_eq!(
        domux_server::agents::resume::shell_quote("a'b"),
        "'a'\\''b'"
    );
    assert_eq!(
        domux_server::agents::resume::shell_quote("plain"),
        "'plain'"
    );
}

/// The quoting is in the line the pane receives, not only in the helper. A session id that
/// would end the shell word early still reaches the pane as one word.
#[tokio::test]
async fn a_quote_in_the_session_id_reaches_the_pane_quoted() {
    let mut h = Harness::start(Config::default(), 100, 24).await;
    let pane = h.focused_pane(h.client.clone());
    h.report(
        pane.clone(),
        AgentKind::Claude,
        r#"{"hook_event_name":"SessionStart","session_id":"a'b","cwd":"/repo"}"#,
    )
    .await;
    h.report(
        pane.clone(),
        AgentKind::Claude,
        r#"{"hook_event_name":"SessionEnd","session_id":"a'b"}"#,
    )
    .await;
    let id = h.agents().await[0].id.clone();
    h.api("agent.resume", json!({"agent": id.to_string()}))
        .await
        .unwrap();
    assert_eq!(
        String::from_utf8(h.pane_input(&pane)).unwrap(),
        "cd '/repo' && command -v claude >/dev/null 2>&1 && claude --resume 'a'\\''b'\r"
    );
}

#[tokio::test]
async fn resuming_a_record_with_no_session_id_says_what_is_missing() {
    let mut h = Harness::start(Config::default(), 100, 24).await;
    let pane = h.focused_pane(h.client.clone());
    let pid = h.set_foreground_for(&pane, Some("claude")).await;
    tokio::time::sleep(A_TICK).await;
    let id = h.agents().await[0].id.clone();
    // The process, not the foreground. A record exits when its own process id goes away and
    // never because something else came to the front, so clearing the foreground alone leaves
    // this record `unknown`, and `unknown` is live. Without the kill and the tick after it the
    // refusal below would be the live one, and this test would pass on a message it does not
    // name.
    h.set_foreground_for(&pane, None).await;
    h.kill_process(pid).await;
    tokio::time::sleep(A_TICK).await;
    assert_eq!(h.agents().await[0].state, AgentState::Exited);
    let err = h
        .api("agent.resume", json!({"agent": id.to_string()}))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Unavailable);
    assert_eq!(err.message, "this agent has no session id, so there is nothing to resume; it was seen by the observer and never reported a hook. Run domux2 install claude to install the hooks");
    assert!(
        h.pane_input(&pane).is_empty(),
        "a refusal types nothing into the pane"
    );
}

/// A Codex record the observer made has two reasons it cannot resume: its kind carries no
/// resume command, and it has no session id. The kind is the one worth reading, because nothing
/// the reader does about the session id would help, so the kind is asked about first. This pins
/// that order; without it the two checks could be swapped and every other test would pass.
#[tokio::test]
async fn a_kind_that_cannot_resume_says_so_even_when_it_also_has_no_session_id() {
    let mut h = Harness::start(Config::default(), 100, 24).await;
    let pane = h.focused_pane(h.client.clone());
    let pid = h.set_foreground_for(&pane, Some("codex")).await;
    tokio::time::sleep(A_TICK).await;
    let record = h.agents().await[0].clone();
    assert_eq!(record.kind, AgentKind::Codex);
    assert_eq!(
        record.session_id, None,
        "the observer records no session id"
    );
    h.set_foreground_for(&pane, None).await;
    h.kill_process(pid).await;
    tokio::time::sleep(A_TICK).await;
    let err = h
        .api("agent.resume", json!({"agent": record.id.to_string()}))
        .await
        .unwrap_err();
    assert_eq!(
        err.message, "codex does not resume yet; only claude does. Start it yourself in its pane",
        "the kind is asked about before the session id"
    );
}

/// The pane a record last ran in can be closed while the record stays in the list. The line
/// still exists and there is nowhere to type it, so the refusal says that rather than reporting
/// a terminal that is missing, which is what the write would have said one step later.
#[tokio::test]
async fn resuming_a_record_whose_pane_is_gone_says_the_pane_is_gone() {
    let mut h = Harness::start(Config::default(), 100, 24).await;
    h.api("pane.split", json!({"dir": "right"})).await.unwrap();
    let second = h.focused_pane(h.client.clone());
    an_exited(&mut h, &second, AgentKind::Claude, "sess-1").await;
    let id = h.agents().await[0].id.clone();
    h.api("pane.close", json!({"pane": second.as_str()}))
        .await
        .unwrap();
    let err = h
        .api("agent.resume", json!({"agent": id.to_string()}))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
    assert_eq!(
        err.message,
        format!("pane {second} is gone; open a pane and run the command yourself")
    );
}

/// The narrower case the check above cannot reach: a pane the model still holds whose process
/// never started, which is what a `terminal.shell` that cannot run looks like. There is nowhere
/// to type the line, and the write is what says so.
///
/// Named for `workspace.resume` as much as for this: its loop puts the write's failure in
/// `skipped` rather than ending the run, and this is the fixture that makes that branch
/// reachable, so the comment claiming it is checked rather than asserted.
#[tokio::test]
async fn resuming_into_a_pane_whose_process_never_started_is_refused_and_skipped() {
    let mut h = Harness::start(Config::default(), 100, 24).await;
    // From here no new pane gets a process.
    h.spawner
        .as_ref()
        .expect("the fake spawner")
        .refuse_spawns();
    h.api("pane.split", json!({"dir": "right"})).await.unwrap();
    let second = h.focused_pane(h.client.clone());
    an_exited(&mut h, &second, AgentKind::Claude, "sess-1").await;
    let id = h.agents().await[0].id.clone();
    let err = h
        .api("agent.resume", json!({"agent": id.to_string()}))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
    assert_eq!(err.message, format!("pane {second} has no terminal"));
    // And the same record inside a workspace-wide resume is skipped, not fatal.
    let out = h.api("workspace.resume", json!({})).await.unwrap();
    assert_eq!(out["resumed"].as_array().unwrap().len(), 0, "{out}");
    assert_eq!(
        out["skipped"].as_array().unwrap(),
        &vec![serde_json::Value::from(format!(
            "{id}: pane {second} has no terminal"
        ))],
        "the write's failure joins the skipped list: {out}"
    );
}

#[tokio::test]
async fn codex_and_opencode_do_not_resume_in_v2_0_and_the_message_says_so() {
    let mut h = Harness::start(Config::default(), 100, 24).await;
    let pane = h.focused_pane(h.client.clone());
    an_exited(&mut h, &pane, AgentKind::Codex, "x1").await;
    let id = h.agents().await[0].id.clone();
    let err = h
        .api("agent.resume", json!({"agent": id.to_string()}))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Unavailable);
    assert_eq!(
        err.message,
        "codex does not resume yet; only claude does. Start it yourself in its pane"
    );
    // The kind leads its own message, so an OpenCode row names OpenCode. One assertion on one
    // kind would pass with the kind hard coded.
    h.api("pane.split", json!({"dir": "right"})).await.unwrap();
    let second = h.focused_pane(h.client.clone());
    an_exited(&mut h, &second, AgentKind::Opencode, "o1").await;
    let opencode = h
        .agents()
        .await
        .into_iter()
        .find(|a| a.kind == AgentKind::Opencode)
        .expect("the opencode record");
    let err = h
        .api("agent.resume", json!({"agent": opencode.id.to_string()}))
        .await
        .unwrap_err();
    assert_eq!(
        err.message,
        "opencode does not resume yet; only claude does. Start it yourself in its pane"
    );
}

#[tokio::test]
async fn resuming_a_live_agent_is_refused_and_names_what_to_do_instead() {
    let mut h = Harness::start(Config::default(), 100, 24).await;
    let pane = h.focused_pane(h.client.clone());
    h.report(
        pane.clone(),
        AgentKind::Claude,
        r#"{"hook_event_name":"SessionStart","session_id":"s"}"#,
    )
    .await;
    let id = h.agents().await[0].id.clone();
    let err = h
        .api("agent.resume", json!({"agent": id.to_string()}))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Refused);
    assert!(
        err.message.contains("is idle, not exited"),
        "{}",
        err.message
    );
    assert!(
        err.message.contains("agent focus"),
        "it names the verb that does work: {}",
        err.message
    );
    assert!(
        h.pane_input(&pane).is_empty(),
        "a live agent's pane is not typed into"
    );
}

#[tokio::test]
async fn enter_on_an_exited_row_resumes_and_leaves_the_overlay_open_with_the_result() {
    let mut h = Harness::start(Config::default(), 100, 24).await;
    let (pane, _) = an_exited_claude(&mut h).await;
    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), "a").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("⏎ resume"),
        Duration::from_secs(2),
    )
    .await;
    h.key(h.client.clone(), "Enter").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| f.contains("Resumed"),
            Duration::from_secs(2),
        )
        .await;
    let footer = footer_row(&f);
    assert!(
        row_text(&f, footer).contains("Resumed claude in "),
        "the footer row says what happened (principle 8):\n{f}"
    );
    assert!(
        style_at(&f, footer, pill_col(&f)).contains(PILL_OK),
        "in green, because it worked:\n{f}"
    );
    assert_eq!(
        overlay(&h),
        Some(Overlay::Agents),
        "the overlay is still open behind the result (plan assumption 28)"
    );
    assert!(
        String::from_utf8(h.pane_input(&pane))
            .unwrap()
            .contains("claude --resume"),
        "the command was typed"
    );
}

/// The refusal path of the same key. Enter on a row that cannot resume changes nothing else on
/// the screen, so without this line the key would look broken (principle 9).
///
/// It reads the footer row and its colour rather than asking whether the reason is anywhere on
/// the frame. A failed key also puts its message in the top bar's hint, so the loose question
/// answers yes with no footer pill at all - the overlay dims the top bar, which is why the
/// footer is the row that carries this.
#[tokio::test]
async fn enter_on_a_row_that_cannot_resume_puts_the_reason_in_the_footer() {
    let mut h = Harness::start(Config::default(), 100, 24).await;
    let pane = h.focused_pane(h.client.clone());
    an_exited(&mut h, &pane, AgentKind::Codex, "x1").await;
    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), "a").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("⏎ resume"),
        Duration::from_secs(2),
    )
    .await;
    h.key(h.client.clone(), "Enter").await;
    let f = h
        .wait_for(
            h.client.clone(),
            |f| row_text(f, footer_row(f)).contains("does not resume"),
            Duration::from_secs(2),
        )
        .await;
    let footer = footer_row(&f);
    assert!(
        row_text(&f, footer).contains("codex does not resume"),
        "the footer row names the kind:\n{f}"
    );
    assert!(
        style_at(&f, footer, pill_col(&f)).contains(PILL_REFUSED),
        "in red, because it refused:\n{f}"
    );
    assert_eq!(overlay(&h), Some(Overlay::Agents), "and the list stays up");
    assert!(h.pane_input(&pane).is_empty(), "and nothing was typed");
}

/// Enter on a **live** row is the other method, and it lands the reader in the pane with no
/// overlay left open (plan assumption 28). Here so the two halves of one key are pinned
/// together: a change that made the exited path close the overlay, or the live path keep it
/// open, breaks one of these two tests.
#[tokio::test]
async fn enter_on_a_live_row_lands_in_the_pane_instead_of_keeping_the_overlay() {
    let mut h = Harness::start(Config::default(), 100, 24).await;
    let pane = h.focused_pane(h.client.clone());
    h.report(
        pane.clone(),
        AgentKind::Claude,
        r#"{"hook_event_name":"SessionStart","session_id":"live-1"}"#,
    )
    .await;
    h.key(h.client.clone(), "C-a").await;
    h.key(h.client.clone(), "a").await;
    h.wait_for(
        h.client.clone(),
        |f| f.contains("⏎ open"),
        Duration::from_secs(2),
    )
    .await;
    h.key(h.client.clone(), "Enter").await;
    // Polled rather than through `wait_for`, whose predicate takes the frame: what this is
    // waiting for is the client's overlay, and the frame behind an overlay that has just closed
    // is the workpanel, which has no words of its own to wait for.
    let mut closed = false;
    for _ in 0..40 {
        if overlay(&h).is_none() {
            closed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(closed, "Enter on a live row leaves no overlay open");
    assert!(
        h.pane_input(&pane).is_empty(),
        "a live row is not resumed, so its pane is not typed into"
    );
}

#[tokio::test]
async fn workspace_resume_resumes_every_resumable_record_and_lists_what_it_skipped() {
    let mut h = Harness::start(Config::default(), 100, 24).await;
    let (_, _) = an_exited_claude(&mut h).await;
    h.api("pane.split", json!({"dir": "right"})).await.unwrap();
    let second = h.focused_pane(h.client.clone());
    an_exited(&mut h, &second, AgentKind::Codex, "x1").await;
    let ws = h.model().client(&h.client).unwrap().workspace.to_string();
    let out = h
        .api("workspace.resume", json!({"workspace": ws}))
        .await
        .unwrap();
    assert_eq!(out["resumed"].as_array().unwrap().len(), 1);
    assert_eq!(out["skipped"].as_array().unwrap().len(), 1);
    assert!(
        out["skipped"][0]
            .as_str()
            .unwrap()
            .contains("codex does not resume"),
        "{out}"
    );
    // The skipped line names its record, so a workspace with several says which is which.
    let codex = h
        .agents()
        .await
        .into_iter()
        .find(|a| a.kind == AgentKind::Codex)
        .expect("the codex record");
    assert!(
        out["skipped"][0]
            .as_str()
            .unwrap()
            .starts_with(&format!("{}: ", codex.id)),
        "{out}"
    );
}

/// The one that cannot resume must not stop the one that can (plan assumption 29). The Codex
/// record exited last, and exited records sort newest first, so it is reached before the Claude
/// one: a loop that stopped at the first failure would type nothing at all.
#[tokio::test]
async fn a_record_that_cannot_resume_does_not_stop_the_ones_that_can() {
    let mut h = Harness::start(Config::default(), 100, 24).await;
    let (claude_pane, _) = an_exited_claude(&mut h).await;
    h.api("pane.split", json!({"dir": "right"})).await.unwrap();
    let second = h.focused_pane(h.client.clone());
    an_exited(&mut h, &second, AgentKind::Codex, "x1").await;
    let out = h.api("workspace.resume", json!({})).await.unwrap();
    assert_eq!(
        out["skipped"].as_array().unwrap().len(),
        1,
        "the codex record was reached and refused: {out}"
    );
    assert!(
        String::from_utf8(h.pane_input(&claude_pane))
            .unwrap()
            .contains("claude --resume 'sess-1'"),
        "and the claude record was still resumed after it"
    );
    assert!(
        h.pane_input(&second).is_empty(),
        "nothing was typed into the codex one"
    );
}

/// A live record is not resumed by a workspace-wide resume, and it is not reported as skipped
/// either: it was never a candidate.
#[tokio::test]
async fn workspace_resume_leaves_a_live_record_alone_and_does_not_list_it() {
    let mut h = Harness::start(Config::default(), 100, 24).await;
    let pane = h.focused_pane(h.client.clone());
    h.report(
        pane.clone(),
        AgentKind::Claude,
        r#"{"hook_event_name":"SessionStart","session_id":"live-1"}"#,
    )
    .await;
    let out = h.api("workspace.resume", json!({})).await.unwrap();
    assert_eq!(out["resumed"].as_array().unwrap().len(), 0);
    assert_eq!(
        out["skipped"].as_array().unwrap().len(),
        0,
        "a live record is not a candidate, so it is not a failure either: {out}"
    );
    assert!(h.pane_input(&pane).is_empty());
}

/// The records of another workspace are not this one's to resume. Without the workspace filter
/// this call would resume the Claude record in the first project's workspace.
#[tokio::test]
async fn workspace_resume_only_touches_the_workspace_it_was_named() {
    let mut h = Harness::start(Config::default(), 100, 24).await;
    let (pane, _) = an_exited_claude(&mut h).await;
    let root = h.git_project("main").await;
    let canonical = root.canonicalize().expect("the project root is there");
    let mine = h.model().client(&h.client).unwrap().workspace.clone();
    let other = h
        .model()
        .project_at(&canonical)
        .and_then(|p| p.workspaces.first().map(|w| w.id.clone()))
        .expect("the registered project has a main workspace");
    assert_ne!(other, mine, "the second project is not the client's own");
    let out = h
        .api("workspace.resume", json!({"workspace": other.to_string()}))
        .await
        .unwrap();
    assert_eq!(out["resumed"].as_array().unwrap().len(), 0, "{out}");
    assert_eq!(out["skipped"].as_array().unwrap().len(), 0, "{out}");
    assert!(
        h.pane_input(&pane).is_empty(),
        "the other workspace's resume left this record alone"
    );
}

#[tokio::test]
async fn resume_agents_auto_resumes_every_record_when_the_server_starts() {
    let mut cfg = Config::default();
    cfg.resume.agents = ResumeMode::Auto;
    let mut h = Harness::start(cfg, 100, 24).await;
    let (pane, _) = an_exited_claude(&mut h).await;
    h.stop().await;
    h.restart().await;
    let a = &h.agents().await[0];
    assert_eq!(
        a.state,
        AgentState::Exited,
        "the record is still exited until a hook says otherwise"
    );
    let typed = String::from_utf8(h.pane_input(&pane)).unwrap();
    assert!(
        typed.contains("claude --resume 'sess-1'"),
        "auto resume typed the line: {typed:?}"
    );
}

#[tokio::test]
async fn the_default_is_manual_so_a_restart_types_nothing() {
    let mut h = Harness::start_with(HarnessOptions::new(Config::default(), 100, 24)).await;
    let (pane, _) = an_exited_claude(&mut h).await;
    h.stop().await;
    h.restart().await;
    assert!(
        String::from_utf8(h.pane_input(&pane)).unwrap().is_empty(),
        "manual is the default"
    );
}

/// `auto` resumes at start and not on every attach (plan assumption 30). A second terminal
/// attaching to a running server has not asked for anything to be relaunched, and a resume
/// there would type a second line into a pane where the first one is already running.
#[tokio::test]
async fn auto_does_not_resume_again_when_another_client_attaches() {
    let mut cfg = Config::default();
    cfg.resume.agents = ResumeMode::Auto;
    let mut h = Harness::start(cfg, 100, 24).await;
    let (pane, _) = an_exited_claude(&mut h).await;
    h.stop().await;
    h.restart().await;
    let after_start = String::from_utf8(h.pane_input(&pane)).unwrap();
    assert!(after_start.contains("claude --resume 'sess-1'"));
    h.attach(100, 24).await;
    assert_eq!(
        String::from_utf8(h.pane_input(&pane)).unwrap(),
        after_start,
        "the attach typed nothing more"
    );
}
