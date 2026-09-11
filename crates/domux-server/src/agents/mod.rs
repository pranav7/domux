//! Agents: records of AI coding sessions, fed by hooks and by the observer. The core owns one
//! `AgentsState`; every handler reaches it through `api::Ctx`.

pub mod context;
pub mod hooks;
pub mod install;
pub mod labels;
pub mod manifests;
pub mod observer;
pub mod recap;

/// `agents::report` is the roadmap's name for the handler that turns one hook payload into
/// one record change. It is a re-export rather than a second function: a keybinding, a CLI
/// subcommand and an API call all reach the same handler, and that handler lives with the
/// rest of the `agent.*` namespace in `api::agent`.
pub use crate::api::agent::report;

use domux_core::api::Event;
use domux_core::ids::AgentId;
use domux_core::model::agent::AgentState;
use labels::WorkingWords;
use manifests::Registry;
use recap::RecapReader;
use std::path::Path;

/// Everything about agents the core holds that is not in the Model: the caches and the
/// declarations. None of it is persisted.
pub struct AgentsState {
    pub words: WorkingWords,
    pub recaps: RecapReader,
    pub manifests: Registry,
    /// Counts up once every `labels::ANIMATION_INTERVAL` while an agent works. The glyph's
    /// frame and the place of the band along a working word are both read off it, so every
    /// client on the server draws one animation (Task 17, MUX-26).
    pub tick: u64,
}

impl AgentsState {
    /// Drops what one record leaves behind when it goes: its working word and its cached
    /// transcript.
    ///
    /// Id-driven where `release_words_of` is event-driven, and both are needed. This one runs
    /// before the removal that would produce the events, and a transcript is keyed by path,
    /// which no event carries. An agent id is never reissued, so nothing will ask for either
    /// again.
    pub fn forget_record(&mut self, agent: &AgentId, transcript: Option<&Path>) {
        self.words.release(agent);
        if let Some(path) = transcript {
            self.recaps.forget(path);
        }
    }

    /// Frees the working word of every record these events say has stopped working or has
    /// gone. One rule in one place, because the records a change touches are not only the one
    /// that was asked about: `Model::report_agent` exits the record it takes a pane from and
    /// removes a placeholder it resumes over, and both of those held a word if they were
    /// working. The pool is 186 words and an agent id is never reissued, so a word that is
    /// not freed is a slot lost for the life of the server.
    ///
    /// Driven by the events rather than by the caller's own record, because the events are
    /// what say which records changed. A caller that released only the record it named is how
    /// the displaced one was missed.
    pub fn release_words_of(&mut self, events: &[Event]) {
        for e in events {
            match e {
                Event::AgentStateChanged { agent, to, .. } if *to != AgentState::Working => {
                    self.words.release(agent)
                }
                Event::AgentExited { agent, .. } => self.words.release(agent),
                _ => {}
            }
        }
    }
}

impl Default for AgentsState {
    fn default() -> AgentsState {
        AgentsState {
            words: WorkingWords::default(),
            recaps: RecapReader::default(),
            manifests: Registry::builtin(),
            tick: 0,
        }
    }
}
