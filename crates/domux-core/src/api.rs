//! The control API's wire types: errors, events, and (Task 8) requests, responses and methods.

use crate::ids::{ClientId, PaneId, TabId, WorkspaceId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

/// Stable error codes. Fixed by roadmap section 5.4; never add one without updating it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    NotFound,
    Ambiguous,
    InvalidParams,
    /// The operation is not allowed, for example deleting `main`.
    Refused,
    /// The state changed underneath the caller.
    Conflict,
    /// A dependency such as `gh`, the network, or a not-yet-built feature is missing.
    Unavailable,
    /// A pane or workspace is occupied.
    Busy,
    Internal,
}

/// An API error. Messages follow design principle 9: name the state, the object, and the
/// next action, in one sentence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, thiserror::Error)]
#[error("{code:?}: {message}")]
pub struct ApiError {
    pub code: ErrorCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl ApiError {
    fn new(code: ErrorCode, message: impl Into<String>) -> ApiError {
        ApiError {
            code,
            message: message.into(),
            data: None,
        }
    }
    pub fn not_found(m: impl Into<String>) -> ApiError {
        ApiError::new(ErrorCode::NotFound, m)
    }
    pub fn ambiguous(m: impl Into<String>, candidates: Vec<String>) -> ApiError {
        ApiError {
            code: ErrorCode::Ambiguous,
            message: m.into(),
            data: Some(Value::from(candidates)),
        }
    }
    pub fn invalid_params(m: impl Into<String>) -> ApiError {
        ApiError::new(ErrorCode::InvalidParams, m)
    }
    pub fn refused(m: impl Into<String>) -> ApiError {
        ApiError::new(ErrorCode::Refused, m)
    }
    pub fn conflict(m: impl Into<String>) -> ApiError {
        ApiError::new(ErrorCode::Conflict, m)
    }
    pub fn unavailable(m: impl Into<String>) -> ApiError {
        ApiError::new(ErrorCode::Unavailable, m)
    }
    pub fn busy(m: impl Into<String>) -> ApiError {
        ApiError::new(ErrorCode::Busy, m)
    }
    pub fn internal(m: impl Into<String>) -> ApiError {
        ApiError::new(ErrorCode::Internal, m)
    }
}

/// Everything the server tells subscribers. Serialized as `{"event": "<ns>.<past tense>", ...}`.
/// M1 adds the server, client, config, tab and pane events; M2 to M4 add theirs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "event")]
pub enum Event {
    #[serde(rename = "server.started")]
    ServerStarted { version: String, socket: PathBuf },
    #[serde(rename = "server.stopping")]
    ServerStopping,
    #[serde(rename = "client.attached")]
    ClientAttached { client: ClientId },
    #[serde(rename = "client.detached")]
    ClientDetached { client: ClientId },
    #[serde(rename = "config.reloaded")]
    ConfigReloaded { error: Option<String> },
    #[serde(rename = "tab.created")]
    TabCreated { workspace: WorkspaceId, tab: TabId },
    #[serde(rename = "tab.renamed")]
    TabRenamed { tab: TabId, name: Option<String> },
    #[serde(rename = "tab.closed")]
    TabClosed { workspace: WorkspaceId, tab: TabId },
    #[serde(rename = "tab.selected")]
    TabSelected { client: ClientId, tab: TabId },
    #[serde(rename = "pane.spawned")]
    PaneSpawned {
        tab: TabId,
        pane: PaneId,
        cwd: PathBuf,
    },
    #[serde(rename = "pane.exited")]
    PaneExited { pane: PaneId, status: Option<i32> },
    #[serde(rename = "pane.closed")]
    PaneClosed { tab: TabId, pane: PaneId },
    #[serde(rename = "pane.focused")]
    PaneFocused { tab: TabId, pane: PaneId },
    #[serde(rename = "pane.resized")]
    PaneResized { pane: PaneId, cols: u16, rows: u16 },
    #[serde(rename = "pane.zoomed")]
    PaneZoomed { tab: TabId, pane: Option<PaneId> },
}

impl Event {
    /// The event names of this build, in declaration order. Used by `events.subscribe`
    /// filters and by the schema.
    pub const NAMES: &'static [&'static str] = &[
        "server.started",
        "server.stopping",
        "client.attached",
        "client.detached",
        "config.reloaded",
        "tab.created",
        "tab.renamed",
        "tab.closed",
        "tab.selected",
        "pane.spawned",
        "pane.exited",
        "pane.closed",
        "pane.focused",
        "pane.resized",
        "pane.zoomed",
    ];

    pub fn name(&self) -> &'static str {
        match self {
            Event::ServerStarted { .. } => "server.started",
            Event::ServerStopping => "server.stopping",
            Event::ClientAttached { .. } => "client.attached",
            Event::ClientDetached { .. } => "client.detached",
            Event::ConfigReloaded { .. } => "config.reloaded",
            Event::TabCreated { .. } => "tab.created",
            Event::TabRenamed { .. } => "tab.renamed",
            Event::TabClosed { .. } => "tab.closed",
            Event::TabSelected { .. } => "tab.selected",
            Event::PaneSpawned { .. } => "pane.spawned",
            Event::PaneExited { .. } => "pane.exited",
            Event::PaneClosed { .. } => "pane.closed",
            Event::PaneFocused { .. } => "pane.focused",
            Event::PaneResized { .. } => "pane.resized",
            Event::PaneZoomed { .. } => "pane.zoomed",
        }
    }

    /// True when `filter` (a list of names or `ns.*` globs) admits this event. An empty
    /// filter admits everything.
    pub fn matches(&self, filter: &[String]) -> bool {
        if filter.is_empty() {
            return true;
        }
        let name = self.name();
        filter.iter().any(|f| match f.strip_suffix(".*") {
            Some(ns) => name.starts_with(ns) && name[ns.len()..].starts_with('.'),
            None => f == name,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_serialize_with_dotted_names() {
        let e = Event::TabCreated {
            workspace: WorkspaceId("w_c3a1".into()),
            tab: TabId("t_41b2".into()),
        };
        let json = serde_json::to_value(&e).unwrap();
        assert_eq!(json["event"], "tab.created");
        assert_eq!(json["tab"], "t_41b2");
        assert_eq!(e.name(), "tab.created");
        let unit = Event::ServerStopping;
        assert_eq!(
            serde_json::to_string(&unit).unwrap(),
            r#"{"event":"server.stopping"}"#
        );
    }

    #[test]
    fn every_event_name_matches_its_namespace_dot_past_tense_shape() {
        for name in Event::NAMES {
            let (ns, verb) = name.split_once('.').expect("dotted");
            assert!(!ns.is_empty() && !verb.is_empty(), "{name}");
            assert!(
                name.chars()
                    .all(|c| c.is_ascii_lowercase() || c == '.' || c == '_'),
                "{name}"
            );
        }
    }

    #[test]
    fn api_error_serializes_code_as_snake_case() {
        let e = ApiError::not_found("tab t_0000 does not exist; run domux2 api tab.list");
        let json = serde_json::to_value(&e).unwrap();
        assert_eq!(json["code"], "not_found");
        assert_eq!(
            json["message"],
            "tab t_0000 does not exist; run domux2 api tab.list"
        );
        assert!(json.get("data").is_none() || json["data"].is_null());
    }
}
