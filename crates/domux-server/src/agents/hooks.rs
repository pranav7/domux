//! Hook payload adapters. `agent report --agent <kind>` is installed for every hook event of
//! every kind (architecture spec 3.4): each of Claude, Codex and OpenCode writes a different
//! JSON object on stdin, so each kind has an adapter here that turns one payload into one
//! `AgentReport`. An event domux does not track parses to `event: None` rather than failing, so
//! a new hook event in a future release of an agent never breaks the hook.

use domux_core::model::agent::{AgentEvent, AgentKind, AgentReport};
use serde_json::Value;
use std::fmt;
use std::path::PathBuf;

/// The Claude Code events the installer writes for Claude, in the order it writes them. V1
/// installs eight; V2 adds `PostToolUse` because the state machine names it as a working event
/// (architecture spec 3.3, M3 plan assumption 19).
pub const EVENTS_CLAUDE: [&str; 9] = [
    "SessionStart",
    "SessionEnd",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "Notification",
    "PreCompact",
    "PostCompact",
    "Stop",
];

/// The Codex events the installer writes for Codex. V1 installs five; V2 adds `SessionStart`
/// and `SessionEnd`, which Codex accepts or ignores (architecture spec open question 5).
pub const EVENTS_CODEX: [&str; 7] = [
    "SessionStart",
    "SessionEnd",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PermissionRequest",
    "Stop",
];

/// The OpenCode events the generated plugin emits, in the order it lists them: its two tool
/// handlers, then the seven its `event` switch takes.
///
/// OpenCode has no hooks file, so nothing installs these; the plugin is domux's own file
/// (`agents::install::opencode_plugin`) and this is the list it and `parse_opencode` share. The
/// tests walk it, so an event added to the plugin without a fixture is a failure rather than a
/// gap: `session.error` was in the plugin and in the adapter with no fixture, and the loop that
/// looked complete covered eight of nine.
pub const EVENTS_OPENCODE: [&str; 9] = [
    "tool.execute.before",
    "tool.execute.after",
    "session.created",
    "message.updated",
    "permission.asked",
    "permission.replied",
    "session.idle",
    "session.error",
    "session.deleted",
];

/// A hook payload that could not become a report. The Claude, Codex and OpenCode names carry
/// their reason in prose, not their variant name, so a caller can log or print it as is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookError {
    NotJson { kind: AgentKind, message: String },
    NotAnObject { kind: AgentKind },
}

impl fmt::Display for HookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HookError::NotJson { kind, message } => {
                write!(f, "the {kind} hook payload is not JSON: {message}")
            }
            HookError::NotAnObject { kind } => {
                write!(f, "the {kind} hook payload is not a JSON object")
            }
        }
    }
}

impl std::error::Error for HookError {}

/// Parses one hook payload into one report. A payload that names an event domux does not
/// track is not an error: the report carries `event: None`, and `agent.report` answers `Ack`
/// without touching the record, so a new hook event in a future release never fails the hook.
pub fn parse(kind: AgentKind, text: &str) -> Result<AgentReport, HookError> {
    let value: Value = serde_json::from_str(text).map_err(|e| HookError::NotJson {
        kind,
        message: e.to_string(),
    })?;
    if !value.is_object() {
        return Err(HookError::NotAnObject { kind });
    }
    Ok(match kind {
        AgentKind::Claude => parse_claude(&value),
        AgentKind::Codex => parse_codex(&value),
        AgentKind::Opencode => parse_opencode(&value),
    })
}

/// Claude Code: `hook_event_name`, `session_id`, `transcript_path`, `cwd`, and `message` on a
/// notification (architecture spec 3.4).
pub fn parse_claude(v: &Value) -> AgentReport {
    let event = match string(v, "hook_event_name").as_deref() {
        Some("SessionStart") => Some(AgentEvent::SessionStart),
        Some("SessionEnd") => Some(AgentEvent::SessionEnd),
        Some("UserPromptSubmit") => Some(AgentEvent::UserPromptSubmit),
        Some("PreToolUse") => Some(AgentEvent::PreToolUse),
        Some("PostToolUse") => Some(AgentEvent::PostToolUse),
        Some("Notification") => Some(AgentEvent::Notification),
        Some("PreCompact") => Some(AgentEvent::PreCompact),
        Some("PostCompact") => Some(AgentEvent::PostCompact),
        Some("Stop") => Some(AgentEvent::Stop),
        _ => None,
    };
    AgentReport {
        event,
        session_id: string(v, "session_id"),
        transcript_path: path(v, "transcript_path"),
        cwd: path(v, "cwd"),
        reason: if event == Some(AgentEvent::Notification) {
            string(v, "message")
        } else {
            None
        },
    }
}

/// Codex: the same envelope shape as Claude, its own event names, and a session field recorded
/// under one of three names (M3 plan assumption 13); the adapter accepts all three.
/// `PermissionRequest` is the waiting event. The transcript path is Codex's own `rollout_path`,
/// absent when the payload does not carry one.
pub fn parse_codex(v: &Value) -> AgentReport {
    let event = match string(v, "hook_event_name").as_deref() {
        Some("SessionStart") => Some(AgentEvent::SessionStart),
        Some("SessionEnd") => Some(AgentEvent::SessionEnd),
        Some("UserPromptSubmit") => Some(AgentEvent::UserPromptSubmit),
        Some("PreToolUse") => Some(AgentEvent::PreToolUse),
        Some("PostToolUse") => Some(AgentEvent::PostToolUse),
        Some("PermissionRequest") => Some(AgentEvent::Notification),
        Some("Stop") => Some(AgentEvent::Stop),
        _ => None,
    };
    AgentReport {
        event,
        session_id: string(v, "session_id")
            .or_else(|| string(v, "thread_id"))
            .or_else(|| string(v, "conversation_id")),
        transcript_path: path(v, "rollout_path"),
        cwd: path(v, "cwd"),
        reason: if event == Some(AgentEvent::Notification) {
            string(v, "message")
        } else {
            None
        },
    }
}

/// OpenCode: the plugin the installer writes for OpenCode is domux's own file, so domux picks
/// the payload: Claude's field spelling with OpenCode's own event names (M3 plan assumption 14).
pub fn parse_opencode(v: &Value) -> AgentReport {
    let event = match string(v, "hook_event_name").as_deref() {
        Some("session.created") => Some(AgentEvent::SessionStart),
        Some("tool.execute.before") => Some(AgentEvent::PreToolUse),
        Some("tool.execute.after") => Some(AgentEvent::PostToolUse),
        Some("message.updated") => Some(AgentEvent::UserPromptSubmit),
        Some("permission.asked") => Some(AgentEvent::Notification),
        Some("permission.replied") => Some(AgentEvent::PostToolUse),
        Some("session.idle") | Some("session.error") => Some(AgentEvent::Stop),
        Some("session.deleted") => Some(AgentEvent::SessionEnd),
        _ => None,
    };
    AgentReport {
        event,
        session_id: string(v, "session_id"),
        transcript_path: None,
        cwd: path(v, "cwd"),
        reason: if event == Some(AgentEvent::Notification) {
            string(v, "message")
        } else {
            None
        },
    }
}

/// A string field, absent when missing, not a string, or empty after trimming. Never
/// fabricate a value the payload did not carry.
fn string(v: &Value, key: &str) -> Option<String> {
    let s = v.get(key)?.as_str()?.trim();
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

fn path(v: &Value, key: &str) -> Option<PathBuf> {
    string(v, key).map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use domux_core::model::agent::{AgentEvent, AgentKind};
    use std::path::{Path, PathBuf};

    fn fixture(kind: &str, name: &str) -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/hooks")
            .join(kind)
            .join(format!("{name}.json"));
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    #[test]
    fn every_claude_fixture_maps_to_the_event_the_state_machine_names() {
        let cases = [
            ("session_start", AgentEvent::SessionStart),
            ("user_prompt_submit", AgentEvent::UserPromptSubmit),
            ("pre_tool_use", AgentEvent::PreToolUse),
            ("post_tool_use", AgentEvent::PostToolUse),
            ("notification", AgentEvent::Notification),
            ("pre_compact", AgentEvent::PreCompact),
            ("post_compact", AgentEvent::PostCompact),
            ("stop", AgentEvent::Stop),
            ("session_end", AgentEvent::SessionEnd),
        ];
        assert_eq!(
            cases.len(),
            EVENTS_CLAUDE.len(),
            "one fixture per installed event"
        );
        for (name, expected) in cases {
            let r = parse(AgentKind::Claude, &fixture("claude", name)).unwrap();
            assert_eq!(r.event, Some(expected), "{name}");
            assert_eq!(
                r.session_id.as_deref(),
                Some("3f6a1c22-8d4e-4b90-9c7f-2a1b5e6d0f31"),
                "{name}"
            );
            assert_eq!(
                r.cwd,
                Some(PathBuf::from("/Users/pranav/projects/domux")),
                "{name}"
            );
            assert!(
                r.transcript_path
                    .as_ref()
                    .unwrap()
                    .ends_with("3f6a1c22-8d4e-4b90-9c7f-2a1b5e6d0f31.jsonl"),
                "{name}"
            );
        }
    }

    #[test]
    fn a_claude_notification_carries_its_message_as_the_reason() {
        let r = parse(AgentKind::Claude, &fixture("claude", "notification")).unwrap();
        assert_eq!(
            r.reason.as_deref(),
            Some("Claude needs your permission to use Bash")
        );
        let r = parse(AgentKind::Claude, &fixture("claude", "stop")).unwrap();
        assert_eq!(r.reason, None, "only a notification has a reason");
    }

    #[test]
    fn a_codex_permission_request_is_the_waiting_event() {
        let r = parse(AgentKind::Codex, &fixture("codex", "permission_request")).unwrap();
        assert_eq!(r.event, Some(AgentEvent::Notification));
        assert_eq!(r.reason.as_deref(), Some("Codex wants to run `git push`"));
        assert_eq!(
            r.session_id.as_deref(),
            Some("0199c6de-4f0a-7b31-9a2c-51d0e6b8a774")
        );
        assert_eq!(
            r.transcript_path, None,
            "this fixture carries no rollout_path"
        );
    }

    #[test]
    fn every_codex_fixture_maps_to_the_event_the_state_machine_names() {
        let cases = [
            ("session_start", AgentEvent::SessionStart),
            ("user_prompt_submit", AgentEvent::UserPromptSubmit),
            ("pre_tool_use", AgentEvent::PreToolUse),
            ("post_tool_use", AgentEvent::PostToolUse),
            ("permission_request", AgentEvent::Notification),
            ("stop", AgentEvent::Stop),
            ("session_end", AgentEvent::SessionEnd),
        ];
        assert_eq!(
            cases.len(),
            EVENTS_CODEX.len(),
            "one fixture per installed event"
        );
        for (name, expected) in cases {
            let r = parse(AgentKind::Codex, &fixture("codex", name)).unwrap();
            assert_eq!(r.event, Some(expected), "{name}");
            assert_eq!(
                r.session_id.as_deref(),
                Some("0199c6de-4f0a-7b31-9a2c-51d0e6b8a774"),
                "{name}"
            );
            assert_eq!(
                r.cwd,
                Some(PathBuf::from("/Users/pranav/projects/audrey-app")),
                "{name}"
            );
        }
    }

    #[test]
    fn codex_reads_the_transcript_path_from_rollout_path() {
        let r = parse(
            AgentKind::Codex,
            r#"{"hook_event_name":"Stop","session_id":"t-1","rollout_path":"/Users/pranav/.codex/sessions/t-1.jsonl"}"#,
        )
        .unwrap();
        assert_eq!(
            r.transcript_path,
            Some(PathBuf::from("/Users/pranav/.codex/sessions/t-1.jsonl"))
        );
        let r = parse(
            AgentKind::Codex,
            r#"{"hook_event_name":"Stop","session_id":"t-1"}"#,
        )
        .unwrap();
        assert_eq!(
            r.transcript_path, None,
            "absent when the payload names no rollout_path"
        );
    }

    #[test]
    fn codex_reads_the_session_under_any_of_the_three_names_it_may_use() {
        for field in ["session_id", "thread_id", "conversation_id"] {
            let text = format!("{{\"hook_event_name\": \"Stop\", \"{field}\": \"t-1\"}}");
            assert_eq!(
                parse(AgentKind::Codex, &text)
                    .unwrap()
                    .session_id
                    .as_deref(),
                Some("t-1"),
                "{field}"
            );
        }
    }

    #[test]
    fn the_opencode_plugin_events_map_onto_the_state_machine() {
        let cases = [
            ("session_start", Some(AgentEvent::SessionStart)),
            ("tool_before", Some(AgentEvent::PreToolUse)),
            ("tool_after", Some(AgentEvent::PostToolUse)),
            ("message_updated", Some(AgentEvent::UserPromptSubmit)),
            ("permission_asked", Some(AgentEvent::Notification)),
            ("permission_replied", Some(AgentEvent::PostToolUse)),
            ("session_idle", Some(AgentEvent::Stop)),
            ("session_error", Some(AgentEvent::Stop)),
            ("session_deleted", Some(AgentEvent::SessionEnd)),
        ];
        assert_eq!(
            cases.len(),
            EVENTS_OPENCODE.len(),
            "one fixture per event the plugin emits"
        );
        for (name, expected) in cases {
            assert_eq!(
                parse(AgentKind::Opencode, &fixture("opencode", name))
                    .unwrap()
                    .event,
                expected,
                "{name}"
            );
        }
    }

    /// A permission request is what an OpenCode reason is for, and the plugin is what has to
    /// send one: `string` answers absent for a value that is not a string, so a `message` that
    /// arrives as an object leaves the record with no reason at all. Nothing renders `reason`
    /// in M3, so this is the pin that says what the plugin owes the adapter.
    #[test]
    fn an_opencode_permission_request_carries_its_message_as_the_reason() {
        let r = parse(
            AgentKind::Opencode,
            &fixture("opencode", "permission_asked"),
        )
        .unwrap();
        assert_eq!(
            r.reason.as_deref(),
            Some("opencode wants to edit src/auth.ts")
        );
        let r = parse(AgentKind::Opencode, &fixture("opencode", "session_idle")).unwrap();
        assert_eq!(r.reason, None, "only a permission request has a reason");
        let r = parse(
            AgentKind::Opencode,
            r#"{"hook_event_name":"permission.asked","session_id":"s","message":{"title":"edit"}}"#,
        )
        .unwrap();
        assert_eq!(
            r.reason, None,
            "a message that is not a string is absent, which is why the plugin sends text"
        );
    }

    #[test]
    fn an_event_domux_does_not_track_parses_with_no_event_and_no_error() {
        let r = parse(
            AgentKind::Claude,
            r#"{"hook_event_name": "SubagentStop", "session_id": "s"}"#,
        )
        .unwrap();
        assert_eq!(r.event, None);
        assert_eq!(r.session_id.as_deref(), Some("s"));
    }

    #[test]
    fn a_payload_that_is_not_json_names_the_kind_and_the_first_line() {
        let err = parse(AgentKind::Claude, "not json at all").unwrap_err();
        assert_eq!(
            err.to_string(),
            "the claude hook payload is not JSON: expected ident at line 1 column 2"
        );
        let err = parse(AgentKind::Claude, "[]").unwrap_err();
        assert_eq!(
            err.to_string(),
            "the claude hook payload is not a JSON object"
        );
    }

    #[test]
    fn an_empty_transcript_path_or_cwd_is_absent_not_empty() {
        let r = parse(
            AgentKind::Claude,
            r#"{"hook_event_name":"Stop","transcript_path":"","cwd":""}"#,
        )
        .unwrap();
        assert_eq!(r.transcript_path, None);
        assert_eq!(r.cwd, None);
    }
}
