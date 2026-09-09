//! `agent.*`: the records namespace. `agents.*` (Task 14) opens the overlay that lists them.

use super::{ok, Ctx};
// `crate::agents::resume` is named in full below rather than imported: the handler in this
// file is called `resume` too, and one of the two would have to be renamed to something it is
// not.
use crate::agents::{context, hooks, manifests::RecapSource};
use domux_core::api::{
    Ack, AgentInfo, AgentListParams, AgentListResult, AgentReportParams, AgentReportResult,
    AgentResumeParams, AgentResumeResult, AgentSelfParams, AgentTargetParams, ApiError,
    FocusResult,
};
use domux_core::ids::{AgentId, PaneId};
use domux_core::model::agent::{Agent, AgentEvent, AgentState};
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
        agent,
        from,
        to,
        events,
        ..
    } = ctx.model.report_agent(&pane, p.kind, parsed, &now)?;
    // `Model::report_agent` leaves a record whose session is over alone for every hook but
    // `SessionStart`, down to its last activity time (M3 plan assumption 7), and says so by
    // answering `exited` to `exited`. So nothing below may touch that record either: a hook
    // that arrives after the session ended must not rewrite its recap or its session name,
    // which outlive the session and are what the exited row shows.
    let applied = from != AgentState::Exited || to != AgentState::Exited;
    // Any events it did produce belong to another record it exited on the way, so they
    // travel and the screen changed even when this record did not.
    let changed = applied || !events.is_empty();
    // Every record these events touched, and not only the one the hook named. A report exits
    // the record whose pane a new session took, and that record was working and holding a
    // word a moment ago. Before the early return below, because a hook that changed nothing
    // about its own record still displaced the other one.
    ctx.agents.release_words_of(&events);
    ctx.events.extend(events);
    // The Agents box, the agents overlay and the top bar's count all read the records, and
    // nothing turns an event into a redraw, so a report that changed one says so here.
    ctx.view_dirty |= changed;
    if !applied {
        return ok(AgentReportResult {
            agent: Some(agent),
            state: Some(to),
            context: None,
        });
    }

    // Recap and session name, re-read on the events the architecture spec names, plus
    // `SessionStart` so a resumed session shows its recap at once (M3 plan assumption 6).
    if matches!(
        event,
        AgentEvent::Stop | AgentEvent::UserPromptSubmit | AgentEvent::SessionStart
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
        .live_agent_on_pane(&pane)
        .ok_or_else(|| ApiError::not_found("no agent is running in this pane"))?;
    ok(info_for(model, a))
}

/// Switches this client to the agent's workspace, selects its tab and focuses its pane
/// (interface spec 6.8). One function; Enter on a live row of the Agents box calls it too.
///
/// Nothing here clears the dot. `Model::focus_pane` does, for every route to a pane at once,
/// and this is one of those routes.
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
/// Nothing here clears the record's dot either. The dot means "something changed since you last
/// looked", and what changed about this record is that it exited - which is still true after the
/// line is typed and stays true until the hook says the session is back.
pub fn resume(ctx: &mut Ctx, p: AgentResumeParams) -> Result<Value, ApiError> {
    // `AgentResumeParams` carries no pane, so this is the target or the calling client's
    // cursor. A record is resumed by name or by the row you are looking at; the pane you happen
    // to be typing in does not name one, because the record that would answer for it is the
    // live agent there and a live agent is exactly what this refuses.
    let id = resolve(
        ctx,
        &AgentTargetParams {
            agent: p.agent,
            pane: None,
            client: p.client,
        },
    )?;
    let (command, pane) = plan_resume(ctx, &id)?;
    // One carriage return, which is what Enter sends: the shell reads the whole thing as one
    // line and runs it, and the line is in the history afterwards.
    ctx.write_to_pane(&pane, format!("{command}\r").as_bytes())?;
    ok(AgentResumeResult {
        agent: id,
        pane,
        command,
    })
}

/// The line and the pane to type it into, or the reason there is neither. Shared with
/// `workspace.resume`, which collects these refusals instead of stopping at one.
///
/// The order of the four refusals is the order the reader can act on them. A live record is
/// asking for the wrong verb. A kind that does not resume in V2.0 cannot be helped by anything
/// the reader does, so it is next, and it comes before the session id because a Codex record
/// with no session id has two reasons and only one of them is worth reading. A missing session
/// id is fixable by installing the hooks, which the message says. A pane that is gone is last,
/// because it is the only one where the line exists and there is nowhere to put it.
pub(crate) fn plan_resume(ctx: &Ctx, id: &AgentId) -> Result<(String, PaneId), ApiError> {
    let agent = ctx.model.agent(id).ok_or_else(|| {
        ApiError::not_found(format!("agent {id} does not exist; run {BIN_NAME} peek"))
    })?;
    // Refused rather than not found, and it names the verb that does work: the record is
    // there, it is just already running. The mirror image of `focus`, which refuses an exited
    // record and names this one.
    if agent.state.is_live() {
        return Err(ApiError::refused(format!(
            "agent {id} is {}, not exited; open it with {BIN_NAME} agent focus {id}",
            agent.state
        )));
    }
    let manifest = ctx
        .agents
        .manifests
        .for_kind(agent.kind)
        .ok_or_else(|| ApiError::internal(format!("no manifest for {}", agent.kind)))?;
    if manifest.resume_command.is_none() {
        return Err(ApiError::unavailable(format!(
            "{} {}",
            agent.kind,
            crate::agents::resume::RESUME_UNAVAILABLE
        )));
    }
    let session = agent.session_id.as_deref().ok_or_else(|| {
        ApiError::unavailable(format!(
            "this agent has no session id, so there is nothing to resume; it was seen by the observer and never reported a hook. Run {BIN_NAME} install {} to install the hooks",
            agent.kind
        ))
    })?;
    // `last_pane` and not `pane`: an exited record has no pane, and the point of resume is to
    // put the agent back where it was working.
    let pane = agent.last_pane.clone().ok_or_else(|| {
        ApiError::not_found(
            "this agent never ran in a pane domux knows, so there is nowhere to type the line",
        )
    })?;
    if ctx.model.pane(&pane).is_none() {
        return Err(ApiError::not_found(format!(
            "pane {pane} is gone; open a pane and run the command yourself"
        )));
    }
    let command =
        crate::agents::resume::resume_line(manifest, session, &agent.cwd).ok_or_else(|| {
            ApiError::internal("the manifest carries a resume command and no process name")
        })?;
    Ok((command, pane))
}

/// Removes an exited record from the list. `Model::dismiss_agent` refuses a live one.
pub fn dismiss(ctx: &mut Ctx, p: AgentTargetParams) -> Result<Value, ApiError> {
    let id = resolve(ctx, &p)?;
    // Read before the record goes, and used only after the refusal has had its chance: a
    // path read afterwards is always absent, so the cached transcript would outlive every
    // record that could ever ask for it again.
    let transcript = ctx.model.agent(&id).and_then(|a| a.transcript_path.clone());
    let events = ctx.model.dismiss_agent(&id)?;
    // The word and the transcript were keyed to a record that no longer exists, and an agent
    // id is never reissued, so nothing will ask for either again. The word goes through the
    // one rule that reads these events; the transcript is keyed by path, which no event
    // carries, so it is dropped here.
    //
    // **The word half frees nothing today, and it is kept anyway.** `Model::dismiss_agent`
    // refuses a live record, and every path that leaves the record in the list frees the word
    // on the way, so a record that can be dismissed is one that holds none. Not every path out
    // of `working`: a removal takes the record with it, and `api::project::remove` and
    // `Core::workspace_deleted` free the word themselves for exactly that reason. The three
    // paths that leave a record behind are held by
    // `a_stop_hook_gives_the_working_word_back_to_the_pool`,
    // `a_pane_that_exits_gives_back_the_working_words_of_its_agents` and
    // `a_session_that_takes_a_pane_gives_back_the_word_of_the_one_it_displaced`. The last of
    // those was a real leak until Task 17, which is the argument for keeping this: the pool is
    // 186 words, an agent id is never reissued, and a slot lost here is lost for the life of
    // the server, so a change that makes this path reachable must not depend on someone
    // remembering to add the release back. `dismissing_a_record_gives_its_working_word_back_
    // to_the_pool` puts a word in the pool by hand to hold the intent.
    ctx.agents.release_words_of(&events);
    if let Some(path) = transcript {
        ctx.agents.recaps.forget(&path);
    }
    ctx.events.extend(events);
    ctx.view_dirty = true;
    ok(Ack { ok: true })
}

/// `agent` names a record, a workspace with one live agent, or `workspace/tab`. With no
/// `agent`, the pane the caller is in, then the cursor row of the calling client's Agents box.
fn resolve(ctx: &Ctx, p: &AgentTargetParams) -> Result<AgentId, ApiError> {
    if let Some(target) = &p.agent {
        return ctx.model.resolve_agent_target(target);
    }
    if let Some(pane) = &p.pane {
        if let Some(a) = ctx.model.live_agent_on_pane(pane) {
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
