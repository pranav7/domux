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
            .live_agent_on_pane(&p.pane)
            .map(|a| (a.id.clone(), a.kind, a.pid));
        match (seen, live) {
            // A known agent is in the foreground and a record already lives on this pane.
            // Only that record's own process decides what happens: what is in front of the
            // pane may be a tool the record is running, and a tool can itself be an agent -
            // an agent that runs another one, or its own kind, is the case that has to
            // survive here.
            (Some((kind, pid)), Some((id, live_kind, held))) => match held {
                // Its own process is gone. The record exits, and the process in front starts
                // a record of its own rather than taking this one over: the session that
                // ended keeps its record, and with it its session id and its place in the
                // list of things that can be resumed.
                Some(old) if old != pid && !inspector.is_alive(old) => {
                    events.extend(model.agent_process_gone(&id, now));
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
                    events.extend(model.agent_process_gone(&id, now));
                }
            }
            // A record with no process id yet: a hook arrived before the first tick. Leave it.
            // A later tick binds a process to it, or `pane_gone` exits it.
            (None, Some((_, _, None))) => {}
            (None, None) => {}
        }
    }
    events
}

/// A pane's child exited, or the pane closed. Every record there exits at once, without
/// waiting for the next tick (architecture spec 3.5).
pub fn pane_gone(model: &mut Model, pane: &PaneId, now: &str) -> Vec<Event> {
    let ids: Vec<_> = model
        .agents
        .iter()
        .filter(|a| a.state.is_live() && a.pane.as_ref() == Some(pane))
        .map(|a| a.id.clone())
        .collect();
    ids.iter()
        .flat_map(|id| model.agent_process_gone(id, now))
        .collect()
}

/// Removes every exited record nothing can ever reach again (MUX-22).
///
/// One property, and it is permanent: **the record has no session id.** It was made by the
/// observer and no hook ever reported on it, so there is no session for `agent.resume` to name,
/// and no hook can find it again either - `Model::report_agent` matches a returning session by
/// its id, and a record without one is unreachable by every route there is. It can be dismissed
/// and nothing else, which is a row asking the reader to tidy up after domux.
///
/// **Three near misses that are deliberately left alone**, each because the reason is temporary
/// and the record is worth more than the row it costs:
///
/// - **A pane that is gone, or one another agent has taken over.** `agent.resume` refuses both,
///   and both free up: the pane comes back, or the reader starts the session themselves and its
///   `SessionStart` brings this record back with its recap and its name.
/// - **A kind that does not resume.** Codex and OpenCode carry no resume command in V2.0, and
///   that is a gap in this release rather than a fact about the record. Pruning on it would
///   delete rows for a reason that is due to stop being true, and the reader would never meet
///   `resume::RESUME_UNAVAILABLE`, which is how they learn that the other kinds are coming.
/// - **Age.** An exited record is not less useful for being old; it is the oldest ones a reader
///   goes looking for. What made the list unreadable was the sidebar carrying every one of them,
///   and `RowForm::shows` is where that was answered.
///
/// It runs on the tick and on a pane going, so a record is taken as it exits rather than on a
/// schedule of its own.
///
/// Nothing calls `AgentsState::forget_record` after it, and nothing needs to: a record with no
/// session id never carried a transcript path, so none of them put a transcript in the cache.
/// The working words are freed by `release_words_of`, off the `AgentDismissed` events below.
pub fn prune_unresumable(model: &mut Model) -> Vec<Event> {
    let gone: Vec<_> = model
        .agents
        .iter()
        .filter(|a| a.state == AgentState::Exited && a.session_id.is_none())
        .map(|a| a.id.clone())
        .collect();
    gone.iter()
        // `dismiss_agent` is the one removal, and it refuses a live record. Every id here is
        // exited, so the refusal cannot fire; a `flat_map` rather than an unwrap says so
        // without a panic nobody could act on.
        .flat_map(|id| model.dismiss_agent(id).unwrap_or_default())
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
