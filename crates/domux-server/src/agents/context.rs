//! The `SessionStart` context block. The agent report subcommand prints it on stdout and
//! Claude Code adds it to the session's context (architecture spec 3.4).
//!
//! It says where this agent is, that peers exist, and names the verbs. `peek` and `whoami`
//! work in M3; `send`, `wait` and `read` answer with an `unavailable` error naming M4 until
//! M4 fills them in, so the block says so rather than showing an example that fails (M3 plan
//! assumption 39).

use domux_core::model::{Agent, Model};
use domux_core::names::{BIN_NAME, PRODUCT_NAME};

/// The exact text M3 prints. M4 replaces the last paragraph with one example per verb.
pub fn session_start_context(model: &Model, agent: &Agent) -> String {
    let place = place_of(model, agent);
    let pane = agent
        .pane
        .as_ref()
        .map(|p| p.to_string())
        .unwrap_or_else(|| "no pane".into());
    let peers = model
        .agents
        .iter()
        .filter(|a| a.id != agent.id && a.state.is_live())
        .count();
    let peer_line = match peers {
        0 => "No other agent is running right now; others may start while you work.".to_string(),
        1 => "One other agent is running right now.".to_string(),
        n => format!("{n} other agents are running right now."),
    };
    format!(
        "[{product}] You are agent {id} ({kind}) in {place}, pane {pane}, working directory {cwd}.\n\
         \n\
         {peer_line} {product} is the multiplexer around all of you. These commands talk to it:\n\
         \n\
         \x20 {bin} peek      every agent: kind, place, state, recap\n\
         \x20 {bin} whoami    this agent's project, workspace, tab and pane\n\
         \n\
         Messaging between agents is not built yet: {bin} send, {bin} wait and {bin} read\n\
         answer \"not built yet; messaging arrives in M4\" until then.\n",
        product = PRODUCT_NAME,
        id = agent.id,
        kind = agent.kind,
        place = place,
        pane = pane,
        cwd = agent.cwd.display(),
        peer_line = peer_line,
        bin = BIN_NAME,
    )
}

/// `project › workspace › tab`, the domain model's place. The tab is dropped when the record
/// names no pane the model still holds.
pub fn place_of(model: &Model, agent: &Agent) -> String {
    place(model, agent, true)
}

/// The same place with the tab left off: the sidebar's agent row has no room for it
/// (interface spec 6.3). One function answers both, so the two lines cannot drift apart and
/// nothing has to take the place back apart to shorten it.
pub fn place_without_tab(model: &Model, agent: &Agent) -> String {
    place(model, agent, false)
}

fn place(model: &Model, agent: &Agent, with_tab: bool) -> String {
    let workspace = model.workspace(&agent.workspace);
    let project = workspace
        .and_then(|w| model.project_of_workspace(&w.id))
        .map(|p| p.name.clone())
        .unwrap_or_default();
    let ws_name = workspace.map(|w| w.display_name()).unwrap_or_default();
    let tab = with_tab
        .then(|| agent.pane.as_ref().or(agent.last_pane.as_ref()))
        .flatten()
        .and_then(|p| model.pane_location(p))
        .and_then(|l| model.tab(&l.tab))
        .map(|t| t.name.clone().unwrap_or_else(|| t.id.to_string()));
    match tab {
        Some(t) => format!("{project} › {ws_name} › {t}"),
        None => format!("{project} › {ws_name}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domux_core::ids::AgentId;
    use domux_core::model::agent::{AgentKind, AgentSource};
    use std::path::PathBuf;

    #[test]
    fn a_place_names_the_tab_and_the_short_form_stops_at_the_workspace() {
        let mut model = Model::new(7);
        let (_project, ws, _) = model
            .add_git_project(PathBuf::from("/repo/audrey-app"), "main".into())
            .unwrap();
        let (tab, pane, _) = model
            .create_tab(&ws, PathBuf::from("/repo/audrey-app"))
            .unwrap();
        let agent = Agent::new(
            AgentId("a_5e21".into()),
            AgentKind::Claude,
            ws,
            pane,
            PathBuf::from("/repo/audrey-app"),
            AgentSource::Hook,
            "2026-09-04T14:32:00+00:00",
        );
        assert_eq!(
            place_of(&model, &agent),
            format!("audrey-app › main › {tab}")
        );
        assert_eq!(place_without_tab(&model, &agent), "audrey-app › main");
    }
}
