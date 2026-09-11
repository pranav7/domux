//! The agent record methods: `agent.list`, `agent.get`, `agent.self`, `agent.focus` and
//! `agent.dismiss`, plus the two acts that clear a dot and the two that take a workspace's
//! records with it.
//!
//! Two rules shape the assertions here, both because the behaviour under test is a flag that
//! is usually off. A test that a dot **survives** something passes on its own if nothing in
//! the server ever clears one, so every such test ends by doing the act that does clear it. A
//! test that a filter narrows a list passes if the filter is ignored and the list happens to
//! be short, so every filter is asked for a set the unfiltered answer does not already equal.

mod support;

use domux_core::api::ErrorCode;
use domux_core::config::Config;
use domux_core::ids::{PaneId, ProjectId, WorkspaceId};
use domux_core::model::agent::{AgentKind, AgentState};
use domux_server::testing::Harness;
use serde_json::json;

/// A claude in the first pane and a codex in a second, both idle, both in the client's
/// workspace.
async fn two_agents(h: &mut Harness) -> (PaneId, PaneId) {
    let first = h.focused_pane(h.client.clone());
    h.api("pane.split", json!({"dir": "right"})).await.unwrap();
    let second = h.focused_pane(h.client.clone());
    h.report(first.clone(), AgentKind::Claude, START_CLAUDE)
        .await;
    h.report(second.clone(), AgentKind::Codex, START_CODEX)
        .await;
    (first, second)
}

const START_CLAUDE: &str = r#"{"hook_event_name":"SessionStart","session_id":"c1"}"#;
const START_CODEX: &str = r#"{"hook_event_name":"SessionStart","session_id":"x1"}"#;
/// Codex's waiting event (`agents::hooks::parse_codex`).
const CODEX_WAITS: &str = r#"{"hook_event_name":"PermissionRequest","session_id":"x1"}"#;
const CODEX_WORKS: &str = r#"{"hook_event_name":"UserPromptSubmit","session_id":"x1"}"#;
const CODEX_STOPS: &str = r#"{"hook_event_name":"Stop","session_id":"x1"}"#;
const CLAUDE_ENDS: &str = r#"{"hook_event_name":"SessionEnd","session_id":"c1"}"#;

/// True while any record has a dot. Read through `agent.list`, which is what both the command
/// line and the Agents box read.
async fn any_unseen(h: &mut Harness) -> bool {
    h.agents().await.iter().any(|a| a.unseen)
}

#[tokio::test]
async fn agent_list_sorts_waiting_first_then_working_then_unseen_idle() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (a, b) = two_agents(&mut h).await;
    h.api("pane.split", json!({"dir": "down"})).await.unwrap();
    let c = h.focused_pane(h.client.clone());
    h.report(
        c.clone(),
        AgentKind::Claude,
        r#"{"hook_event_name":"SessionStart","session_id":"c2"}"#,
    )
    .await;
    h.report(
        a.clone(),
        AgentKind::Claude,
        r#"{"hook_event_name":"UserPromptSubmit","session_id":"c1"}"#,
    )
    .await;
    h.report(b.clone(), AgentKind::Codex, CODEX_WAITS).await;
    h.report(
        c.clone(),
        AgentKind::Claude,
        r#"{"hook_event_name":"UserPromptSubmit","session_id":"c2"}"#,
    )
    .await;
    h.report(
        c.clone(),
        AgentKind::Claude,
        r#"{"hook_event_name":"Stop","session_id":"c2"}"#,
    )
    .await;
    let agents = h.agents().await;
    assert_eq!(
        agents.iter().map(|a| a.state).collect::<Vec<_>>(),
        vec![AgentState::Waiting, AgentState::Working, AgentState::Idle]
    );
    assert_eq!(agents[0].pane.as_ref(), Some(&b));
    assert_eq!(agents[1].pane.as_ref(), Some(&a));
    assert_eq!(agents[2].pane.as_ref(), Some(&c));
    assert!(
        agents[2].unseen,
        "working to idle left it unseen, which sorts it above a quiet idle"
    );
}

/// The count is the top bar's: every project, whatever the call asked for. The two filters
/// are asked for sets the unfiltered answer does not equal, so neither passes by being
/// ignored.
#[tokio::test]
async fn agent_list_counts_every_dot_and_narrows_by_workspace_and_state() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let here = h.model().client(&h.client).unwrap().workspace.clone();
    let mine = h.focused_pane(h.client.clone());
    let root = h.git_project("main").await;
    let there = second_workspace(&h, &root);
    let theirs = h.first_pane_of(&there.to_string()).await;
    h.report(mine.clone(), AgentKind::Claude, START_CLAUDE)
        .await;
    h.report(theirs.clone(), AgentKind::Codex, START_CODEX)
        .await;
    h.report(theirs.clone(), AgentKind::Codex, CODEX_WAITS)
        .await;

    let all = h.api("agent.list", json!({})).await.unwrap();
    assert_eq!(all["agents"].as_array().unwrap().len(), 2);
    assert_eq!(all["red_dots"], 1, "the waiting codex, and only it");

    let waiting = h
        .api("agent.list", json!({"state": "waiting"}))
        .await
        .unwrap();
    let waiting = waiting["agents"].as_array().unwrap();
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0]["kind"], "codex");

    let narrowed = h
        .api("agent.list", json!({"workspace": here.to_string()}))
        .await
        .unwrap();
    let rows = narrowed["agents"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "the other workspace's codex is not here");
    assert_eq!(rows[0]["kind"], "claude");
    assert_eq!(
        narrowed["red_dots"], 1,
        "the count is the top bar's, which counts every project whatever was asked for"
    );
}

#[tokio::test]
async fn agent_self_answers_for_the_calling_pane_and_says_so_when_there_is_none() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (a, _) = two_agents(&mut h).await;
    let me = h
        .api("agent.self", json!({"pane": a.to_string()}))
        .await
        .unwrap();
    assert_eq!(me["kind"], "claude");
    assert_eq!(me["session_id"], "c1");
    assert_eq!(me["pane"], a.to_string());
    assert!(
        me["place"].as_str().unwrap().contains(" › "),
        "the place is project › workspace › tab: {me}"
    );

    h.api("pane.split", json!({"dir": "right"})).await.unwrap();
    let empty = h.focused_pane(h.client.clone());
    let err = h
        .api("agent.self", json!({"pane": empty.to_string()}))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::NotFound);
    assert_eq!(err.message, "no agent is running in this pane");
}

/// `agent.self` outside a pane has no answer, and says which environment variable would have
/// given it one rather than answering for whoever is at the keyboard.
#[tokio::test]
async fn agent_self_without_a_pane_names_what_is_missing() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    two_agents(&mut h).await;
    let err = h.api("agent.self", json!({})).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidParams);
    assert!(err.message.contains("DOMUX_PANE"), "{}", err.message);
}

#[tokio::test]
async fn agent_focus_moves_the_keys_to_the_agents_pane_and_clears_its_dot() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (a, b) = two_agents(&mut h).await;
    h.report(b.clone(), AgentKind::Codex, CODEX_WAITS).await;
    h.api("pane.focus", json!({"pane": a.to_string()}))
        .await
        .unwrap();
    let codex = codex_of(&mut h).await;
    assert!(
        codex.unseen,
        "it started waiting while the keys were on {a}"
    );

    h.api("agent.focus", json!({"agent": codex.id.to_string()}))
        .await
        .unwrap();
    assert_eq!(h.focused_pane(h.client.clone()), b);
    assert!(
        !codex_of(&mut h).await.unseen,
        "opening the agent cleared its dot"
    );
}

/// The agent is in a tab this client is not on, so the switch is what the test is about: the
/// same call has to select the tab as well as focus the pane.
#[tokio::test]
async fn agent_focus_selects_the_agents_tab_when_the_client_is_on_another() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let first_tab = h.current_tab(h.client.clone());
    let a = h.focused_pane(h.client.clone());
    h.report(a.clone(), AgentKind::Claude, START_CLAUDE).await;
    h.api("tab.create", json!({})).await.unwrap();
    let second_tab = h.current_tab(h.client.clone());
    assert_ne!(first_tab, second_tab, "the client moved to the new tab");

    let claude = h.agents().await.into_iter().next().unwrap();
    h.api("agent.focus", json!({"agent": claude.id.to_string()}))
        .await
        .unwrap();
    assert_eq!(h.current_tab(h.client.clone()), first_tab);
    assert_eq!(h.focused_pane(h.client.clone()), a);
}

#[tokio::test]
async fn focusing_a_pane_or_typing_into_it_clears_unseen() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (a, b) = two_agents(&mut h).await;
    h.api("pane.focus", json!({"pane": a.to_string()}))
        .await
        .unwrap();

    h.report(b.clone(), AgentKind::Codex, CODEX_WAITS).await;
    assert!(any_unseen(&mut h).await, "waiting set the dot");
    h.api("pane.focus", json!({"pane": b.to_string()}))
        .await
        .unwrap();
    assert!(!any_unseen(&mut h).await, "focus cleared it");

    h.api("pane.focus", json!({"pane": a.to_string()}))
        .await
        .unwrap();
    // Back through working, because a second waiting event on a record that is already
    // waiting is not a change and sets no dot (`model::agent::attention`).
    h.report(b.clone(), AgentKind::Codex, CODEX_WORKS).await;
    h.report(b.clone(), AgentKind::Codex, CODEX_STOPS).await;
    assert!(any_unseen(&mut h).await, "working to idle set it again");
    h.api(
        "pane.send_text",
        json!({"pane": b.to_string(), "text": "yes\n"}),
    )
    .await
    .unwrap();
    assert!(!any_unseen(&mut h).await, "input cleared it");
}

/// A dot arrives while the keys are already on the agent's pane, so nothing moves. Focusing
/// the pane you are already in is still focusing it, and the dot goes.
#[tokio::test]
async fn focusing_the_pane_you_are_already_in_clears_its_dot() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (_, b) = two_agents(&mut h).await;
    assert_eq!(h.focused_pane(h.client.clone()), b, "the split focused it");
    h.report(b.clone(), AgentKind::Codex, CODEX_WAITS).await;
    assert!(any_unseen(&mut h).await, "the dot arrived under the keys");
    h.api("pane.focus", json!({"pane": b.to_string()}))
        .await
        .unwrap();
    assert!(!any_unseen(&mut h).await, "asking for it again cleared it");
}

/// Interface spec 6.5: a dot you did not act on is still there tomorrow. Reading the records
/// is not acting on them, and neither is working in another pane. The last two lines do the
/// one thing that clears it, so this cannot pass by nothing ever clearing a dot.
#[tokio::test]
async fn a_dot_survives_reading_the_records_and_working_in_another_pane() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (a, b) = two_agents(&mut h).await;
    h.api("pane.focus", json!({"pane": a.to_string()}))
        .await
        .unwrap();
    h.report(b.clone(), AgentKind::Codex, CODEX_WAITS).await;
    let codex = codex_of(&mut h).await;
    assert!(codex.unseen);

    h.api("agent.list", json!({})).await.unwrap();
    h.api("agent.get", json!({"agent": codex.id.to_string()}))
        .await
        .unwrap();
    h.api("agent.self", json!({"pane": b.to_string()}))
        .await
        .unwrap();
    assert!(any_unseen(&mut h).await, "reading is not acting");

    h.api("pane.focus", json!({"pane": a.to_string()}))
        .await
        .unwrap();
    h.api(
        "pane.send_text",
        json!({"pane": a.to_string(), "text": "ls\n"}),
    )
    .await
    .unwrap();
    assert!(any_unseen(&mut h).await, "the other pane is not this one");

    h.api("pane.focus", json!({"pane": b.to_string()}))
        .await
        .unwrap();
    assert!(!any_unseen(&mut h).await, "and this one is");
}

/// Unseen clears on focus and on input, and on nothing else. Enter on a pane whose child
/// exited is input to that pane, and closing it hands the tab's focus to a neighbour, so the
/// key must not reach the neighbour's agent: the reader cleared a dead pane away and never
/// looked at the record next door. The last two lines do the one thing that does clear it, so
/// this cannot pass by nothing ever clearing a dot.
///
/// The pane that dies holds no record of its own, which is the point: a record there would
/// take a dot from exiting and get it cleared by the same key, and either outcome would leave
/// something unseen in the model whichever pane the clearing read.
#[tokio::test]
async fn closing_an_exited_pane_leaves_the_dot_of_the_agent_in_the_next_one() {
    let mut cfg = Config::default();
    cfg.terminal.remain_on_exit = true;
    let mut h = Harness::start(cfg, 80, 24).await;
    let shell = h.focused_pane(h.client.clone());
    h.api("pane.split", json!({"dir": "right"})).await.unwrap();
    let b = h.focused_pane(h.client.clone());
    h.report(b.clone(), AgentKind::Codex, START_CODEX).await;
    h.api("pane.focus", json!({"pane": shell.to_string()}))
        .await
        .unwrap();
    h.report(b.clone(), AgentKind::Codex, CODEX_WAITS).await;
    assert!(codex_of(&mut h).await.unseen, "waiting set the dot");

    // The pane the keys are on dies, and Enter closes it. Focus lands on pane b, which is
    // where the dot is.
    h.exit_pane(shell.clone(), Some(0)).await;
    h.key(h.client.clone(), "Enter").await;
    h.api("server.info", json!({})).await.unwrap();
    assert_eq!(
        h.focused_pane(h.client.clone()),
        b,
        "closing the dead pane moved the keys to pane b"
    );
    assert!(
        codex_of(&mut h).await.unseen,
        "the key was aimed at the dead pane, so the record in pane b is still unseen"
    );

    h.api("pane.focus", json!({"pane": b.to_string()}))
        .await
        .unwrap();
    assert!(!codex_of(&mut h).await.unseen, "and focusing it clears it");
}

/// Principle 9: the verb exists, so a caller learns it and learns why it does nothing. The
/// words end the way the unbuilt register in `core::tests` keys off.
#[tokio::test]
async fn the_messaging_verbs_answer_unavailable_and_say_why() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    two_agents(&mut h).await;
    for method in ["agent.send", "agent.read", "agent.wait"] {
        let err = h.api(method, json!({"agent": "main"})).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::Unavailable, "{method}");
        assert_eq!(
            err.message,
            format!("domux has no messaging between agents, so {method} is not built yet"),
            "{method}"
        );
    }
}

/// M3 plan assumption 32, as decision record 0030 leaves it. A delete takes the workspace, so
/// it takes the records in it; a clear keeps the workspace and its panes, so the agents running
/// there keep running. The other workspace's record is untouched by either, so neither passes
/// by removing every record there is.
#[tokio::test]
async fn deleting_a_workspace_removes_its_records_and_clearing_one_does_not() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let (_root, w1, w2) = h.git_project_with_two_slots().await;
    let p1 = h.first_pane_of(&w1.to_string()).await;
    let p2 = h.first_pane_of(&w2.to_string()).await;
    h.report(p1.clone(), AgentKind::Claude, START_CLAUDE).await;
    h.report(p2.clone(), AgentKind::Codex, START_CODEX).await;
    assert_eq!(h.agents().await.len(), 2);

    support::api_at(
        h.socket_path(),
        "workspace.clear",
        json!({"workspace": w1.to_string(), "yes": true}),
    )
    .await
    .expect("workspace.clear");
    assert_eq!(
        h.agents().await.len(),
        2,
        "a clear keeps the panes, so it keeps the sessions running in them"
    );

    support::api_at(
        h.socket_path(),
        "workspace.delete",
        json!({"workspace": w2.to_string(), "yes": true}),
    )
    .await
    .expect("workspace.delete");
    let left = h.agents().await;
    assert_eq!(
        left.len(),
        1,
        "the deleted slot's record went with the slot"
    );
    assert_eq!(left[0].workspace, w1, "and only that slot's");
}

/// A clear keeps the workspace and its panes, so the agents running in the slot are still
/// running and still own their records (decision record 0030). It used to take the records
/// whose session was over, and there are none of those to take.
#[tokio::test]
async fn a_clear_leaves_the_agents_running_in_the_slot_alone() {
    let mut h = Harness::start(Config::default(), 120, 24).await;
    let (_root, w1, _w2) = h.git_project_with_two_slots().await;
    let live_pane = h.first_pane_of(&w1.to_string()).await;
    h.report(live_pane.clone(), AgentKind::Claude, START_CLAUDE)
        .await;
    let before = h.agents().await;
    assert_eq!(before.len(), 1);
    let live = before[0].clone();

    support::api_at(
        h.socket_path(),
        "workspace.clear",
        json!({"workspace": w1.to_string(), "yes": true}),
    )
    .await
    .expect("workspace.clear");

    let left = h.agents().await;
    assert_eq!(left.len(), 1, "the session is still running");
    // The same record, not a fresh one the observer put back: an id is never reissued, so an
    // id that survived is the record that survived.
    assert_eq!(left[0].id, live.id);
}

#[tokio::test]
async fn an_ambiguous_target_lists_the_candidates() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    two_agents(&mut h).await;
    let ws = h.model().client(&h.client).unwrap().workspace.to_string();
    let err = h.api("agent.get", json!({"agent": ws})).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::Ambiguous);
    assert!(err.message.contains("2 agents are in"), "{}", err.message);
    assert_eq!(err.data.unwrap().as_array().unwrap().len(), 2);
}

/// The addressing rule takes no liveness from the calling verb any more, because every record
/// is a running session (decision record 0030). A workspace holding one record names it; a
/// workspace holding two asks for the tab.
#[tokio::test]
async fn a_workspace_target_names_the_one_record_in_it() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (claude_pane, codex_pane) = two_agents(&mut h).await;
    h.report(claude_pane, AgentKind::Claude, CLAUDE_ENDS).await;
    let ws = {
        let model = h.model();
        model
            .client(&h.client)
            .expect("the client")
            .workspace
            .to_string()
    };
    assert_eq!(h.agents().await.len(), 1, "the claude session is over");

    h.api("agent.focus", json!({"agent": ws.clone()}))
        .await
        .unwrap();
    assert_eq!(
        h.focused_pane(h.client.clone()),
        codex_pane,
        "the one record left is the one the workspace names"
    );
}

/// Every way input reaches a pane clears the dot on the agent there. Each round puts the dot
/// back through working, because a record that is already idle does not raise a second one.
#[tokio::test]
async fn a_key_a_paste_and_the_two_send_calls_each_clear_the_dot() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let (_, b) = two_agents(&mut h).await;
    assert_eq!(h.focused_pane(h.client.clone()), b, "the keys are on it");

    for act in ["key", "paste", "send_text", "send_key"] {
        h.report(b.clone(), AgentKind::Codex, CODEX_WORKS).await;
        h.report(b.clone(), AgentKind::Codex, CODEX_STOPS).await;
        assert!(any_unseen(&mut h).await, "{act}: the dot is there to clear");
        match act {
            "key" => h.key(h.client.clone(), "y").await,
            "paste" => h.paste(h.client.clone(), "yes").await,
            "send_text" => {
                h.api(
                    "pane.send_text",
                    json!({"pane": b.to_string(), "text": "y"}),
                )
                .await
                .unwrap();
            }
            _ => {
                h.api(
                    "pane.send_key",
                    json!({"pane": b.to_string(), "key": "Enter"}),
                )
                .await
                .unwrap();
            }
        }
        // A key and a paste travel on the attach connection, which is not the connection
        // `agent.list` uses, so the read has to wait for the frame that key produced.
        h.frame(h.client.clone()).await;
        assert!(!any_unseen(&mut h).await, "{act} cleared it");
    }
}

/// Removing a project takes the records of every workspace under it, for the reason deleting
/// one workspace does: a record naming a workspace the model does not hold has no place line,
/// nothing can focus it and nothing can resume it.
#[tokio::test]
async fn removing_a_project_takes_the_records_of_its_workspaces() {
    let mut h = Harness::start(Config::default(), 80, 24).await;
    let mine = h.focused_pane(h.client.clone());
    let root = h.git_project("main").await;
    let (project, there) = second_project(&h, &root);
    let theirs = h.first_pane_of(&there.to_string()).await;
    h.report(mine.clone(), AgentKind::Claude, START_CLAUDE)
        .await;
    h.report(theirs.clone(), AgentKind::Codex, START_CODEX)
        .await;
    assert_eq!(h.agents().await.len(), 2);

    h.api(
        "project.remove",
        json!({"project": project.to_string(), "yes": true}),
    )
    .await
    .unwrap();
    let left = h.agents().await;
    assert_eq!(left.len(), 1, "the removed project's record went");
    assert_eq!(left[0].kind, AgentKind::Claude, "and only it");
}

/// The codex record as `agent.list` reports it now.
async fn codex_of(h: &mut Harness) -> domux_core::api::AgentInfo {
    h.agents()
        .await
        .into_iter()
        .find(|x| x.kind == AgentKind::Codex)
        .expect("the codex record")
}

/// The git project the test just added and its `main` workspace, which is a second workspace
/// without the two worktree creates `git_project_with_two_slots` runs.
fn second_project(h: &Harness, root: &std::path::Path) -> (ProjectId, WorkspaceId) {
    let canonical = root.canonicalize().expect("the project root is there");
    let model = h.model();
    let project = model
        .project_at(&canonical)
        .expect("git_project registered the repository");
    (project.id.clone(), project.workspaces[0].id.clone())
}

fn second_workspace(h: &Harness, root: &std::path::Path) -> WorkspaceId {
    second_project(h, root).1
}
