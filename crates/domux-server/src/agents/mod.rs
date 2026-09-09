//! Agents: records of AI coding sessions, fed by hooks and by the observer. The core owns one
//! `AgentsState`; every handler reaches it through `api::Ctx`.

pub mod context;
pub mod hooks;
pub mod install;
pub mod labels;
pub mod manifests;
pub mod observer;
pub mod recap;
pub mod resume;

/// `agents::report` is the roadmap's name for the handler that turns one hook payload into
/// one record change. It is a re-export rather than a second function: a keybinding, a CLI
/// subcommand and an API call all reach the same handler, and that handler lives with the
/// rest of the `agent.*` namespace in `api::agent`.
pub use crate::api::agent::report;

use labels::WorkingWords;
use manifests::Registry;
use recap::RecapReader;

/// Everything about agents the core holds that is not in the Model: the caches and the
/// declarations. None of it is persisted.
pub struct AgentsState {
    pub words: WorkingWords,
    pub recaps: RecapReader,
    pub manifests: Registry,
    /// Counts up once every `labels::GLYPH_INTERVAL` while an agent works (Task 17).
    pub glyph_tick: u64,
}

impl Default for AgentsState {
    fn default() -> AgentsState {
        AgentsState {
            words: WorkingWords::default(),
            recaps: RecapReader::default(),
            manifests: Registry::builtin(),
            glyph_tick: 0,
        }
    }
}
