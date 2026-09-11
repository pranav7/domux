//! The observer: once a second, and at once when a pane's child exits.
//!
//! It answers two questions per pane. Is the foreground process a known agent that has no
//! record yet? Then create one as `unknown` (architecture spec 3.5): an agent is running and
//! nothing is reporting. Is a record's own process gone? Then mark the record `exited`.
//!
//! What is in the foreground is not the same question as whether the agent is alive: an agent
//! running a tool puts that tool in the foreground. So a record exits when its own process id
//! goes away, never because the foreground changed.
//!
//! That holds when the tool is itself an agent. An agent that runs another kind, or its own
//! kind, still has its own process, and the observer asks about that process rather than
//! reading the name in front of the pane. Reading the name would exit a live agent's record,
//! and a hook after that exit changes nothing, so the record would never come back.

use crate::agents::manifests::Registry;
use crate::process::{ForegroundProcess, ProcessInspector};
use domux_core::api::Event;
use domux_core::ids::PaneId;
use domux_core::model::agent::AgentState;
use domux_core::model::Model;

/// One pane and what M1's inspector said is in front of it this tick.
#[derive(Debug, Clone)]
pub struct PaneProcess {
    pub pane: PaneId,
    pub foreground: Option<ForegroundProcess>,
}

/// One pass over every pane. The core calls this from its once-a-second tick with the
/// foreground processes it already read for the pane boxes.
pub fn run(
    model: &mut Model,
    panes: &[PaneProcess],
    manifests: &Registry,
    inspector: &dyn ProcessInspector,
    now: &str,
) -> Vec<Event> {
    let mut events = Vec::new();
    for p in panes {
        // The kind and process id of a known agent in the foreground, if that is what is there.
        let seen = p
            .foreground
            .as_ref()
            .and_then(|f| manifests.for_process(&f.name).map(|m| (m.kind, f.pid)));
        let live = model
            .agent_on_pane(&p.pane)
            .map(|a| (a.id.clone(), a.kind, a.pid));
        match (seen, live) {
            // A known agent is in the foreground and a record already lives on this pane.
            // Only that record's own process decides what happens: what is in front of the
            // pane may be a tool the record is running, and a tool can itself be an agent -
            // an agent that runs another one, or its own kind, is the case that has to
            // survive here.
            (Some((kind, pid)), Some((id, live_kind, held))) => match held {
                // Its own process is gone. The record ends, and the process in front starts
                // a record of its own rather than taking this one over: two sessions are two
                // records, whatever pane they share.
                Some(old) if old != pid && !inspector.is_alive(old) => {
                    events.extend(model.agent_process_gone(&id));
                    let (_, created) = model.observe_agent(&p.pane, kind, Some(pid), now);
                    events.extend(created);
                }
                // A record a hook made, which no tick has bound a process to yet. The process
                // in front is that agent when it is the same kind. When it is not, it is a
                // tool the agent is running and there is nothing here to bind: nothing exits
                // either, because a record with no process id is a record with no evidence of
                // an exit, and an exit is never guessed.
                None if kind == live_kind => model.set_agent_pid(&id, Some(pid)),
                // Its own process is still there, so the record stands as it is. The process
                // in front is never bound over one that answers.
                None | Some(_) => {}
            },
            // An agent is running and nothing has reported on it.
            (Some((kind, pid)), None) => {
                let (_, created) = model.observe_agent(&p.pane, kind, Some(pid), now);
                events.extend(created);
            }
            // Something else is in the foreground, which an agent running a tool looks like.
            // Only a process id that is gone exits a record.
            (None, Some((id, _, Some(pid)))) => {
                if !inspector.is_alive(pid) {
                    events.extend(model.agent_process_gone(&id));
                }
            }
            // A record with no process id yet: a hook arrived before the first tick. Leave it.
            // A later tick binds a process to it, or `pane_gone` ends it.
            (None, Some((_, _, None))) => {}
            (None, None) => {}
        }
    }
    events
}

/// A pane's child exited, or the pane closed. Every record there ends at once, without
/// waiting for the next tick (architecture spec 3.5).
pub fn pane_gone(model: &mut Model, pane: &PaneId) -> Vec<Event> {
    let ids: Vec<_> = model
        .agents
        .iter()
        .filter(|a| a.pane.as_ref() == Some(pane))
        .map(|a| a.id.clone())
        .collect();
    ids.iter()
        .flat_map(|id| model.agent_process_gone(id))
        .collect()
}

/// True while any record works, which is what the animation ticker needs to know: a glyph
/// only turns while there is something for it to report on (principle 7).
pub fn any_working(model: &Model) -> bool {
    model
        .agents
        .iter()
        .any(|a| matches!(a.state, AgentState::Working | AgentState::Compacting))
}
