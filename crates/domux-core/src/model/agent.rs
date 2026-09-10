//! The agent record and its state machine. `transition` is a pure function of (state, event)
//! and is table-tested over every pair (architecture spec section 3.3). `attention` says when
//! a transition turns `unseen` on (section 3.7; interface spec 6.5).

use crate::ids::{AgentId, PaneId, WorkspaceId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    Claude,
    Codex,
    Opencode,
}

impl AgentKind {
    pub const ALL: [AgentKind; 3] = [AgentKind::Claude, AgentKind::Codex, AgentKind::Opencode];

    pub fn as_str(&self) -> &'static str {
        match self {
            AgentKind::Claude => "claude",
            AgentKind::Codex => "codex",
            AgentKind::Opencode => "opencode",
        }
    }
}

impl fmt::Display for AgentKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for AgentKind {
    type Err = String;
    fn from_str(s: &str) -> Result<AgentKind, String> {
        match s {
            "claude" => Ok(AgentKind::Claude),
            "codex" => Ok(AgentKind::Codex),
            "opencode" => Ok(AgentKind::Opencode),
            other => Err(format!(
                "unknown agent kind {other:?}; expected claude, codex or opencode"
            )),
        }
    }
}

/// `unknown` means an agent is running and nothing is reporting. Never guessed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Working,
    Waiting,
    Compacting,
    Idle,
    Exited,
    Unknown,
}

impl AgentState {
    pub const ALL: [AgentState; 6] = [
        AgentState::Working,
        AgentState::Waiting,
        AgentState::Compacting,
        AgentState::Idle,
        AgentState::Exited,
        AgentState::Unknown,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            AgentState::Working => "working",
            AgentState::Waiting => "waiting",
            AgentState::Compacting => "compacting",
            AgentState::Idle => "idle",
            AgentState::Exited => "exited",
            AgentState::Unknown => "unknown",
        }
    }

    /// Every state but `exited`. One pane hosts at most one live agent.
    pub fn is_live(&self) -> bool {
        *self != AgentState::Exited
    }
}

impl fmt::Display for AgentState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Which records a target may name, said by the verb that is about to act on one.
///
/// The addressing rule is one rule (`Model::resolve_agent_target`) and the verbs that use it
/// do not agree about which records they can act on. `agent.focus` needs a pane to put the
/// keys on, so it wants a live record. `agent.resume` and `agent.dismiss` each refuse a live
/// one, so they want an exited record. `agent.get` reads a record and answers for either.
/// M3 filtered the two workspace forms to live records, which left them unable to name anything
/// the two exited-only verbs would accept. An agent id was answered whatever state it named,
/// then and now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Liveness {
    Live,
    Exited,
    Any,
}

impl Liveness {
    /// Whether a record in this state is one the verb can act on.
    pub fn accepts(self, state: AgentState) -> bool {
        match self {
            Liveness::Live => state.is_live(),
            Liveness::Exited => !state.is_live(),
            Liveness::Any => true,
        }
    }

    /// The adjective a refusal carries, so "no live agent in main" and "2 exited agents are in
    /// main" both say which records were looked at. The reader can see the rest of them in the
    /// list, and a refusal that did not say would read as a contradiction of what is there.
    pub fn adjective(self) -> &'static str {
        match self {
            Liveness::Live => "live ",
            Liveness::Exited => "exited ",
            Liveness::Any => "",
        }
    }
}

/// What a hook or the observer reports. Hook events carry Claude Code's names; Codex and
/// OpenCode payloads are adapted onto them (domux-server `agents::hooks`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentEvent {
    /// The observer saw a known agent command in the foreground with no hook yet.
    Observed,
    SessionStart,
    UserPromptSubmit,
    PreToolUse,
    PostToolUse,
    Notification,
    PreCompact,
    PostCompact,
    Stop,
    SessionEnd,
    /// The observer saw the process leave, or the pane exited or closed.
    ProcessGone,
}

impl AgentEvent {
    pub const ALL: [AgentEvent; 11] = [
        AgentEvent::Observed,
        AgentEvent::SessionStart,
        AgentEvent::UserPromptSubmit,
        AgentEvent::PreToolUse,
        AgentEvent::PostToolUse,
        AgentEvent::Notification,
        AgentEvent::PreCompact,
        AgentEvent::PostCompact,
        AgentEvent::Stop,
        AgentEvent::SessionEnd,
        AgentEvent::ProcessGone,
    ];

    pub fn is_hook(&self) -> bool {
        !matches!(self, AgentEvent::Observed | AgentEvent::ProcessGone)
    }
}

/// Who created a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentSource {
    Hook,
    Observer,
    Restore,
}

/// The state machine of architecture spec section 3.3. Hook events on an `exited` record are
/// ignored, except `SessionStart`, which is how a resumed session comes back (M3 plan
/// assumption). `Observed` never downgrades a state a hook set; on `exited` it says a new
/// process is there, and the Model turns that into a new record.
pub fn transition(state: AgentState, event: AgentEvent) -> AgentState {
    use AgentEvent::*;
    use AgentState::*;
    match (state, event) {
        (Exited, Observed) => Unknown,
        (s, Observed) => s,
        (_, SessionStart) => Idle,
        (
            Exited,
            UserPromptSubmit | PreToolUse | PostToolUse | Notification | PreCompact | PostCompact
            | Stop,
        ) => Exited,
        (_, UserPromptSubmit | PreToolUse | PostToolUse | PostCompact) => Working,
        (_, Notification) => Waiting,
        (_, PreCompact) => Compacting,
        (_, Stop) => Idle,
        (_, SessionEnd | ProcessGone) => Exited,
    }
}

/// True when moving from `from` to `to` turns `unseen` on: an agent starts waiting, goes from
/// working to idle, or exits (interface spec 6.5).
pub fn attention(from: AgentState, to: AgentState) -> bool {
    use AgentState::*;
    (to == Waiting && from != Waiting)
        || (from == Working && to == Idle)
        || (to == Exited && from != Exited)
}

/// One AI coding session domux knows about (architecture spec 3.2 plus `name`, plus the
/// M3 plan's additions `workspace`, `last_pane`, `transcript_path`, `source`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Agent {
    pub id: AgentId,
    pub kind: AgentKind,
    /// The session name the agent set (Claude's `/rename`). Absent until set.
    pub name: Option<String>,
    /// The agent's own id from its hook payload. Enables resume and recaps.
    pub session_id: Option<String>,
    /// Working directory at session start; resume runs from here.
    pub cwd: PathBuf,
    /// Where it runs; `None` once it exits.
    pub pane: Option<PaneId>,
    /// While running. Not persisted.
    #[serde(skip)]
    pub pid: Option<u32>,
    pub state: AgentState,
    /// The Notification message while waiting, for example "permission needed".
    pub reason: Option<String>,
    /// The agent's own one-line summary of its last turn, from its transcript.
    pub recap: Option<String>,
    /// Stored, not rendered in V2.0 (interface spec open question 15).
    pub last_message: Option<String>,
    pub started_at: String,
    pub last_activity_at: String,
    /// Something changed since you last looked. Independent of state.
    pub unseen: bool,
    /// The workspace the session started in. An exited record keeps its place through this.
    pub workspace: WorkspaceId,
    /// The pane it last ran in, for the place line's tab and for resume in its original pane.
    #[serde(default)]
    pub last_pane: Option<PaneId>,
    /// From the hook payload; the recap reader reads its tail.
    #[serde(default)]
    pub transcript_path: Option<PathBuf>,
    pub source: AgentSource,
}

impl Agent {
    /// A fresh `unknown` record on `pane`. The caller applies the first event.
    pub fn new(
        id: AgentId,
        kind: AgentKind,
        workspace: WorkspaceId,
        pane: PaneId,
        cwd: PathBuf,
        source: AgentSource,
        now: &str,
    ) -> Agent {
        Agent {
            id,
            kind,
            name: None,
            session_id: None,
            cwd,
            pane: Some(pane.clone()),
            pid: None,
            state: AgentState::Unknown,
            reason: None,
            recap: None,
            last_message: None,
            started_at: now.to_string(),
            last_activity_at: now.to_string(),
            unseen: false,
            workspace,
            last_pane: Some(pane),
            transcript_path: None,
            source,
        }
    }

    /// The session name, else the kind standing in (interface spec 6.2).
    pub fn display_name(&self) -> String {
        self.name
            .clone()
            .unwrap_or_else(|| self.kind.as_str().to_string())
    }

    /// Waiting, or unseen: the row's dot is red and the count includes it (interface 6.8).
    pub fn needs_you(&self) -> bool {
        self.state == AgentState::Waiting || self.unseen
    }
}

/// What one hook payload says, after the adapter read it. `event` is the only required part.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub struct AgentReport {
    pub event: Option<AgentEvent>,
    pub session_id: Option<String>,
    pub transcript_path: Option<PathBuf>,
    pub cwd: Option<PathBuf>,
    /// The Notification message.
    pub reason: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use AgentEvent::*;
    use AgentState::*;

    /// Every (state, event) pair, in `AgentEvent::ALL` order per row:
    /// Observed, SessionStart, UserPromptSubmit, PreToolUse, PostToolUse, Notification,
    /// PreCompact, PostCompact, Stop, SessionEnd, ProcessGone.
    const TABLE: [(AgentState, [AgentState; 11]); 6] = [
        (
            Working,
            [
                Working, Idle, Working, Working, Working, Waiting, Compacting, Working, Idle,
                Exited, Exited,
            ],
        ),
        (
            Waiting,
            [
                Waiting, Idle, Working, Working, Working, Waiting, Compacting, Working, Idle,
                Exited, Exited,
            ],
        ),
        (
            Compacting,
            [
                Compacting, Idle, Working, Working, Working, Waiting, Compacting, Working, Idle,
                Exited, Exited,
            ],
        ),
        (
            Idle,
            [
                Idle, Idle, Working, Working, Working, Waiting, Compacting, Working, Idle, Exited,
                Exited,
            ],
        ),
        (
            Exited,
            [
                Unknown, Idle, Exited, Exited, Exited, Exited, Exited, Exited, Exited, Exited,
                Exited,
            ],
        ),
        (
            Unknown,
            [
                Unknown, Idle, Working, Working, Working, Waiting, Compacting, Working, Idle,
                Exited, Exited,
            ],
        ),
    ];

    #[test]
    fn transition_matches_the_table_for_every_state_and_event_pair() {
        assert_eq!(AgentEvent::ALL.len(), 11);
        assert_eq!(AgentState::ALL.len(), 6);
        let mut checked = 0;
        for (state, expected) in TABLE {
            for (i, event) in AgentEvent::ALL.iter().enumerate() {
                assert_eq!(
                    transition(state, *event),
                    expected[i],
                    "({state:?}, {event:?})"
                );
                checked += 1;
            }
        }
        assert_eq!(checked, 66, "every pair was checked");
    }

    #[test]
    fn attention_turns_on_at_start_waiting_working_to_idle_and_exit_only() {
        let mut on = Vec::new();
        for from in AgentState::ALL {
            for to in AgentState::ALL {
                if attention(from, to) {
                    on.push((from, to));
                }
            }
        }
        let mut expected = vec![(Working, Idle)];
        for from in AgentState::ALL {
            if from != Waiting {
                expected.push((from, Waiting));
            }
            if from != Exited {
                expected.push((from, Exited));
            }
        }
        on.sort_by_key(|(a, b)| (a.as_str(), b.as_str()));
        expected.sort_by_key(|(a, b)| (a.as_str(), b.as_str()));
        assert_eq!(on, expected);
    }

    #[test]
    fn kinds_and_states_round_trip_as_lowercase_words() {
        for k in AgentKind::ALL {
            assert_eq!(k.as_str().parse::<AgentKind>().unwrap(), k);
            assert_eq!(
                serde_json::to_string(&k).unwrap(),
                format!("\"{}\"", k.as_str())
            );
        }
        assert_eq!("claude".parse::<AgentKind>().unwrap(), AgentKind::Claude);
        assert_eq!(
            "gemini".parse::<AgentKind>().unwrap_err(),
            "unknown agent kind \"gemini\"; expected claude, codex or opencode"
        );
        for s in AgentState::ALL {
            assert_eq!(
                serde_json::to_string(&s).unwrap(),
                format!("\"{}\"", s.as_str())
            );
        }
        assert!(Working.is_live() && Unknown.is_live() && !Exited.is_live());
        assert!(SessionStart.is_hook() && !Observed.is_hook() && !ProcessGone.is_hook());
    }

    #[test]
    fn display_name_is_the_session_name_or_the_kind() {
        let mut a = Agent::new(
            AgentId("a_5e21".into()),
            AgentKind::Claude,
            crate::ids::WorkspaceId("w_c3a1".into()),
            PaneId("p_8f2a".into()),
            std::path::PathBuf::from("/x"),
            AgentSource::Hook,
            "2026-09-04T14:32:00+00:00",
        );
        assert_eq!(a.display_name(), "claude");
        a.name = Some("auth-cleanup".into());
        assert_eq!(a.display_name(), "auth-cleanup");
        assert_eq!(a.state, Unknown);
        assert_eq!(a.last_pane, Some(PaneId("p_8f2a".into())));
        assert!(!a.unseen);
    }
}
