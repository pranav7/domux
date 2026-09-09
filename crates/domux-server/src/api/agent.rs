//! `agent.*`: the records namespace. `agents.*` (Task 14) opens the overlay that lists them.

use super::{ok, Ctx};
use crate::agents::{context, hooks, manifests::RecapSource};
use domux_core::api::{AgentReportParams, AgentReportResult, ApiError};
use domux_core::model::agent::{AgentEvent, AgentState};
use domux_core::model::{AgentReportOutcome, Model};
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
    ctx.events.extend(events);
    // The Agents box, the agents overlay and the top bar's count all read the record, so a
    // report that changed one is a report that changed the screen.
    ctx.view_dirty = true;

    // The word is per working agent and is freed the moment the agent stops working.
    if to != AgentState::Working {
        ctx.agents.words.release(&agent);
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
                let recap_events = ctx.model.set_agent_recap(&agent, t.recap);
                ctx.events.extend(recap_events);
                ctx.model.set_agent_name(&agent, t.name);
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
