//! `agent.*`: the records namespace. `agents.*` (Task 14) opens the overlay that lists them.

use super::{ok, Ctx};
use crate::agents::{context, hooks, manifests::RecapSource};
use domux_core::api::{
    AgentInfo, AgentListParams, AgentListResult, AgentReportParams, AgentReportResult,
    AgentSelfParams, AgentTargetParams, ApiError, FocusResult,
};
use domux_core::ids::AgentId;
use domux_core::model::agent::{Agent, AgentEvent};
use domux_core::model::{AgentReportOutcome, Model};
use domux_core::names::BIN_NAME;
use serde_json::Value;

/// One hook payload. The pane comes from `DOMUX_PANE`; the kind names the adapter.
pub fn report(ctx: &mut Ctx, p: AgentReportParams) -> Result<Value, ApiError> {
    let pane = p.pane.ok_or_else(|| {
        ApiError::invalid_params(
            "agent.report needs a pane; the hook ran without DOMUX_PANE set, so it is not inside a domux pane",
        )
    })?;
    // The hook posts the payload as the agent wrote it. A caller that hands over the raw
    // text sends a JSON string; the CLI parses it first and sends the object.
    let text = match &p.payload {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let parsed =
        hooks::parse(p.kind, &text).map_err(|e| ApiError::invalid_params(e.to_string()))?;
    let Some(event) = parsed.event else {
        // A hook event domux does not track. Nothing changes and nothing fails, so a new
        // event in a future release of an agent never breaks the hook.
        return ok(AgentReportResult {
            agent: None,
            state: None,
            context: None,
        });
    };
    let now = ctx.deps.clock.now().to_rfc3339();
    let AgentReportOutcome {
        agent, to, events, ..
    } = ctx.model.report_agent(&pane, p.kind, parsed, &now)?;
    // `to` is `None` when the report ended the session, in which case `Model::report_agent`
    // has already removed the record. Nothing below may run: there is no row left to write a
    // recap or a session name onto (decision record 0028).
    let applied = to.is_some();
    // Any events it did produce belong to another record it ended on the way, so they travel
    // and the screen changed even when this record did not.
    let changed = applied || !events.is_empty();
    // Every record these events touched, and not only the one the hook named. A report exits
    // the record whose pane a new session took, and that record was working and holding a
    // word a moment ago. Before the early return below, because a hook that changed nothing
    // about its own record still displaced the other one.
    ctx.agents.release_words_of(&events);
    ctx.events.extend(events);
    // The Navigator reads the records and nothing turns an event into a redraw, so a report
    // that changed one says so here.
    ctx.view_dirty |= changed;
    let Some(to) = to else {
        return ok(AgentReportResult {
            agent: Some(agent),
            state: None,
            context: None,
        });
    };

    // Recap and session name, re-read on the events the architecture spec names, plus
    // `SessionStart` so a resumed session shows its recap at once (M3 plan assumption 6), plus
    // the three MUX-20 added.
    //
    // `Notification` is the one that matters: an agent that has stopped to ask you something is
    // the row you read hardest, and M3 left it showing whatever the last `Stop` had found. The
    // two compact events come along because a compaction is a gap in the conversation and the
    // recap on the far side of it is worth re-reading; they cost nothing, being rare.
    //
    // `PreToolUse` and `PostToolUse` are deliberately not here. They fire many times a turn
    // over a transcript the agent is appending to, so the modification time has always moved
    // and every one of them would read the file whole. The row is turning a glyph while they
    // arrive, which already says the recap is a turn behind.
    if matches!(
        event,
        AgentEvent::Stop
            | AgentEvent::UserPromptSubmit
            | AgentEvent::SessionStart
            | AgentEvent::Notification
            | AgentEvent::PreCompact
            | AgentEvent::PostCompact
    ) {
        let source = ctx
            .agents
            .manifests
            .for_kind(p.kind)
            .map(|m| m.recap)
            .unwrap_or(RecapSource::None);
        if source == RecapSource::ClaudeTranscript {
            if let Some(path) = ctx
                .model
                .agent(&agent)
                .and_then(|a| a.transcript_path.clone())
            {
                let t = ctx.agents.recaps.read(&path);
                // The recap summarises the last turn, so re-deriving it every time is the
                // point: an absent one means the last turn produced none.
                let recap_events = ctx.model.set_agent_recap(&agent, t.recap);
                ctx.events.extend(recap_events);
                // The session name is not like that. The agent set it once and it stands
                // until the agent sets another, so an absent one is not evidence that it
                // was cleared: the reader answers `None` for a transcript it could not
                // read, and reads a transcript over `recap::FULL_SCAN_BYTES` as a head and
                // a tail, which can leave an early `/rename` outside the window. Never
                // fabricate cuts both ways, so a name is written only when one was found.
                if t.name.is_some() {
                    ctx.model.set_agent_name(&agent, t.name);
                }
            }
        }
    }

    let context = if event == AgentEvent::SessionStart {
        let model: &Model = ctx.model;
        model
            .agent(&agent)
            .map(|a| context::session_start_context(model, a))
    } else {
        None
    };
    ok(AgentReportResult {
        agent: Some(agent),
        state: Some(to),
        context,
    })
}

/// One record's facts, as the peek subcommand, `agent.get` and any Layer A subscriber see
/// them. The tab comes from the pane it is on, or the pane it last ran on once it has exited.
pub fn info_for(model: &Model, a: &Agent) -> AgentInfo {
    let location = a
        .pane
        .as_ref()
        .or(a.last_pane.as_ref())
        .and_then(|p| model.pane_location(p));
    AgentInfo {
        id: a.id.clone(),
        kind: a.kind,
        name: a.name.clone(),
        session_id: a.session_id.clone(),
        state: a.state,
        unseen: a.unseen,
        recap: a.recap.clone(),
        reason: a.reason.clone(),
        cwd: a.cwd.clone(),
        pane: a.pane.clone(),
        workspace: a.workspace.clone(),
        project: model
            .project_of_workspace(&a.workspace)
            .map(|p| p.id.clone()),
        tab: location.map(|l| l.tab),
        place: context::place_of(model, a),
        started_at: a.started_at.clone(),
        last_activity_at: a.last_activity_at.clone(),
    }
}

/// Every record in the interface's sort order (interface spec 6.7), with the red dot count.
///
/// One list for the command line and for the Agents box, so both surfaces sort the same way
/// (invariant 12). `workspace` and `state` narrow it; `red_dots` does not follow them,
/// because that number is the top bar's and the top bar counts every project (interface spec
/// 6.8). A caller that wants the dots of what it asked for counts the rows it got back.
pub fn list(ctx: &mut Ctx, p: AgentListParams) -> Result<Value, ApiError> {
    let workspace = match &p.workspace {
        Some(target) => Some(ctx.resolve_workspace_param(Some(target))?),
        None => None,
    };
    let model: &Model = ctx.model;
    let agents = model
        .sorted_agents()
        .into_iter()
        .filter(|a| workspace.as_ref().is_none_or(|w| &a.workspace == w))
        .filter(|a| p.state.is_none_or(|s| a.state == s))
        .map(|a| info_for(model, a))
        .collect();
    ok(AgentListResult {
        agents,
        red_dots: model.red_dot_count(),
    })
}

/// One record, named the way every `agent.*` target is named.
///
/// This reads a record rather than acting on it, and the ones
/// exactly what a reader asks about after a session ends.
pub fn get(ctx: &mut Ctx, p: AgentTargetParams) -> Result<Value, ApiError> {
    let id = resolve(ctx, &p)?;
    let model: &Model = ctx.model;
    // Reachable: `resolve` also answers from the calling client's Agents box cursor, which
    // holds an agent id and can outlive the record it named.
    let a = model.agent(&id).ok_or_else(|| {
        ApiError::not_found(format!("agent {id} does not exist; run {BIN_NAME} peek"))
    })?;
    ok(info_for(model, a))
}

/// The record for the pane the caller runs in, which is what the whoami subcommand prints.
///
/// It takes the pane rather than falling back to the focused one: the question is "which
/// agent am I", and the answer for a caller that is not in a pane is that there is none, not
/// the agent whoever is at the keyboard happens to be looking at.
pub fn self_(ctx: &mut Ctx, p: AgentSelfParams) -> Result<Value, ApiError> {
    let pane = p.pane.ok_or_else(|| {
        ApiError::invalid_params(format!(
            "{BIN_NAME} whoami needs a pane; run it inside a domux pane, where DOMUX_PANE is set"
        ))
    })?;
    let model: &Model = ctx.model;
    let a = model
        .agent_on_pane(&pane)
        .ok_or_else(|| ApiError::not_found("no agent is running in this pane"))?;
    ok(info_for(model, a))
}

/// Switches this client to the agent's workspace, selects its tab and focuses its pane
/// (interface spec 6.8). One function; Enter on a live row of the Agents box calls it too.
///
/// Nothing here clears the dot. `Model::focus_pane` does, for every route to a pane at once,
/// and this is one of those routes.
///
/// `Liveness::Live`: an exited record has no pane, so a workspace form that answered one would
/// resolve to a record this refuses two lines later.
pub fn focus(ctx: &mut Ctx, p: AgentTargetParams) -> Result<Value, ApiError> {
    let id = resolve(ctx, &p)?;
    let client = ctx.view()?;
    let agent = ctx.model.agent(&id).ok_or_else(|| {
        ApiError::not_found(format!("agent {id} does not exist; run {BIN_NAME} peek"))
    })?;
    // An exited record has no pane to go to. Refused rather than not found, and the message
    // names what to do instead: the record is there, it just has nowhere to put the keys.
    let pane = agent.pane.clone().ok_or_else(|| {
        ApiError::refused(format!(
            "agent {id} has exited; resume it with {BIN_NAME} agent resume {id}"
        ))
    })?;
    let tab = ctx
        .model
        .pane_location(&pane)
        .map(|l| l.tab)
        .ok_or_else(|| ApiError::not_found(format!("pane {pane} does not exist")))?;
    // `select_tab` moves the client's workspace and tab together and puts its keys on the
    // tab's focused pane; `focus_pane` then moves that to this agent's pane and every client
    // on the tab follows. So the two calls are the whole switch and nothing sets the focus by
    // hand afterwards.
    let events = ctx.model.select_tab(&client, &tab)?;
    ctx.events.extend(events);
    let events = ctx.model.focus_pane(&pane)?;
    ctx.events.extend(events);
    ctx.view_dirty = true;
    let focus = ctx
        .model
        .client(&client)
        .map(|c| c.focus.clone())
        .ok_or_else(|| ApiError::not_found(format!("client {client} is not attached")))?;
    ok(FocusResult { focus })
}

/// Types the agent's relaunch line into a shell in the pane it last ran in (architecture spec
/// section 5). One function; Enter on an exited row of either Agents box calls it too.
///
/// Nothing here changes the record. It stays exited until the session that starts reports its
/// own `SessionStart`, which is what brings it back, so a line that was typed and a shell that
/// never ran it are the same state in the list. That is the honest answer: domux typed a
/// command, it did not start an agent, and only the hook can say one is running (principle 4).
///
fn resolve(ctx: &Ctx, p: &AgentTargetParams) -> Result<AgentId, ApiError> {
    if let Some(target) = &p.agent {
        return ctx.model.resolve_agent_target(target);
    }
    if let Some(pane) = &p.pane {
        if let Some(a) = ctx.model.agent_on_pane(pane) {
            return Ok(a.id.clone());
        }
    }
    // `Ctx::view`, not `p.client`: `core::param_client` already reads that field into the
    // context, so reading it a second time here would be a second answer to one question.
    if let Ok(client) = ctx.view() {
        if let Some(id) = ctx
            .model
            .client(&client)
            .and_then(|v| v.agents_cursor.clone())
        {
            return Ok(id);
        }
    }
    Err(ApiError::invalid_params(format!(
        "name an agent: an agent id, a workspace with one agent, or workspace/tab; run {BIN_NAME} peek for the list"
    )))
}
