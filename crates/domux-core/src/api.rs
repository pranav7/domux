//! The control API's wire types: errors, events, and (Task 8) requests, responses and methods.

use crate::ids::{ClientId, PaneId, TabId, WorkspaceId};
use crate::keymap::Action;
use crate::model::{Direction, Focus, RegionKind};
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Request {
    pub id: Value,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Response {
    pub id: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ApiError>,
}

impl Response {
    pub fn ok(id: Value, result: Value) -> Response {
        Response {
            id,
            result: Some(result),
            error: None,
        }
    }
    pub fn err(id: Value, error: ApiError) -> Response {
        Response {
            id,
            result: None,
            error: Some(error),
        }
    }
}

/// Every params struct: JSON in, positional args in.
pub trait Params: Sized + serde::de::DeserializeOwned + JsonSchema {
    /// Builds the params from an action's positional arguments. The default accepts none.
    fn from_args(args: &[String]) -> Result<Self, ApiError> {
        if args.is_empty() {
            serde_json::from_value(Value::Object(Default::default()))
                .map_err(|e| ApiError::invalid_params(e.to_string()))
        } else {
            Err(ApiError::invalid_params(format!(
                "this method takes no positional arguments, got {args:?}"
            )))
        }
    }
}

fn arg<T: std::str::FromStr>(args: &[String], i: usize, what: &str) -> Result<T, ApiError>
where
    T::Err: std::fmt::Display,
{
    let s = args
        .get(i)
        .ok_or_else(|| ApiError::invalid_params(format!("missing {what} (argument {})", i + 1)))?;
    s.parse::<T>()
        .map_err(|e| ApiError::invalid_params(format!("{what} {s:?}: {e}")))
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoParams {}
impl Params for NoParams {}

/// Confirms that a request completed without an API error.
///
/// `ok` never reports whether state changed. In particular, do not copy the return value from
/// `Model::resize_pane` into this field. That value only says an ancestor with the requested axis
/// owned the resize request.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Ack {
    pub ok: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClientParams {
    #[serde(default)]
    pub client: Option<ClientId>,
}
impl Params for ClientParams {}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SubscribeParams {
    #[serde(default)]
    pub filter: Vec<String>,
}
impl Params for SubscribeParams {
    fn from_args(args: &[String]) -> Result<Self, ApiError> {
        Ok(SubscribeParams {
            filter: args.to_vec(),
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TabListParams {
    #[serde(default)]
    pub workspace: Option<String>,
}
impl Params for TabListParams {}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TabCreateParams {
    #[serde(default)]
    pub workspace: Option<String>,
    #[serde(default)]
    pub client: Option<ClientId>,
    #[serde(default)]
    pub cwd: Option<PathBuf>,
}
impl Params for TabCreateParams {}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TabRenameParams {
    #[serde(default)]
    pub tab: Option<String>,
    /// `None` opens the prompt in the calling view; `Some("")` clears the name.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub client: Option<ClientId>,
}
impl Params for TabRenameParams {
    fn from_args(args: &[String]) -> Result<Self, ApiError> {
        Ok(TabRenameParams {
            tab: None,
            name: if args.is_empty() {
                None
            } else {
                Some(args.join(" "))
            },
            client: None,
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TabTargetParams {
    #[serde(default)]
    pub tab: Option<String>,
    #[serde(default)]
    pub client: Option<ClientId>,
}
impl Params for TabTargetParams {
    fn from_args(args: &[String]) -> Result<Self, ApiError> {
        Ok(TabTargetParams {
            tab: args.first().cloned(),
            client: None,
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TabSelectParams {
    pub tab: String,
    #[serde(default)]
    pub client: Option<ClientId>,
}
impl Params for TabSelectParams {
    fn from_args(args: &[String]) -> Result<Self, ApiError> {
        Ok(TabSelectParams {
            tab: arg::<String>(args, 0, "tab")?,
            client: None,
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PaneTargetParams {
    #[serde(default)]
    pub pane: Option<String>,
    #[serde(default)]
    pub client: Option<ClientId>,
}
impl Params for PaneTargetParams {
    fn from_args(args: &[String]) -> Result<Self, ApiError> {
        Ok(PaneTargetParams {
            pane: args.first().cloned(),
            client: None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PaneSplitParams {
    #[serde(default)]
    pub pane: Option<String>,
    pub dir: Direction,
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    #[serde(default)]
    pub client: Option<ClientId>,
}
impl Params for PaneSplitParams {
    fn from_args(args: &[String]) -> Result<Self, ApiError> {
        Ok(PaneSplitParams {
            pane: None,
            dir: arg(args, 0, "direction")?,
            cwd: None,
            client: None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PaneResizeParams {
    #[serde(default)]
    pub pane: Option<String>,
    pub dir: Direction,
    pub cells: u16,
    #[serde(default)]
    pub client: Option<ClientId>,
}
impl Params for PaneResizeParams {
    fn from_args(args: &[String]) -> Result<Self, ApiError> {
        Ok(PaneResizeParams {
            pane: None,
            dir: arg(args, 0, "direction")?,
            cells: arg(args, 1, "cells")?,
            client: None,
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PaneSendTextParams {
    #[serde(default)]
    pub pane: Option<String>,
    pub text: String,
    #[serde(default)]
    pub client: Option<ClientId>,
}
impl Params for PaneSendTextParams {
    fn from_args(args: &[String]) -> Result<Self, ApiError> {
        Ok(PaneSendTextParams {
            pane: None,
            text: args.join(" "),
            client: None,
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PaneSendKeyParams {
    #[serde(default)]
    pub pane: Option<String>,
    /// A key name as in domux.toml: `Enter`, `C-c`, `S-Left`.
    pub key: String,
    #[serde(default)]
    pub client: Option<ClientId>,
}
impl Params for PaneSendKeyParams {
    fn from_args(args: &[String]) -> Result<Self, ApiError> {
        Ok(PaneSendKeyParams {
            pane: None,
            key: arg::<String>(args, 0, "key")?,
            client: None,
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PaneReadParams {
    #[serde(default)]
    pub pane: Option<String>,
    /// Lines from the bottom, scrollback included. Default: the visible rows.
    #[serde(default)]
    pub lines: Option<usize>,
    #[serde(default)]
    pub client: Option<ClientId>,
}
impl Params for PaneReadParams {
    fn from_args(args: &[String]) -> Result<Self, ApiError> {
        Ok(PaneReadParams {
            pane: None,
            lines: args.first().map(|_| arg(args, 0, "lines")).transpose()?,
            client: None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FocusRegionParams {
    pub region: RegionKind,
    #[serde(default)]
    pub client: Option<ClientId>,
}
impl Params for FocusRegionParams {
    fn from_args(args: &[String]) -> Result<Self, ApiError> {
        let name = arg::<String>(args, 0, "region")?;
        let region = serde_json::from_value(Value::String(name.clone())).map_err(|_| {
            ApiError::invalid_params(format!(
                "region {name:?}: expected overlay, switcher, agents_overlay, sidebar_projects or sidebar_agents"
            ))
        })?;
        Ok(FocusRegionParams {
            region,
            client: None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ClientInfo {
    pub id: ClientId,
    pub cols: u16,
    pub rows: u16,
    pub workspace: crate::ids::WorkspaceId,
    pub tab: TabId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ServerInfo {
    pub version: String,
    pub protocol: u32,
    pub socket: PathBuf,
    pub state_dir: PathBuf,
    pub config_file: PathBuf,
    pub pid: u32,
    pub started_at: String,
    pub config_error: Option<String>,
    pub clients: Vec<ClientInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ConfigReloadResult {
    pub error: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TabInfo {
    pub id: TabId,
    /// 1-based position in the workspace.
    pub index: usize,
    pub name: Option<String>,
    pub panes: Vec<PaneId>,
    pub focused: PaneId,
    pub zoomed: Option<PaneId>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PaneInfo {
    pub id: PaneId,
    pub tab: TabId,
    pub cwd: PathBuf,
    pub command: Option<String>,
    pub title: Option<String>,
    pub pid: Option<u32>,
    pub focused: bool,
    pub zoomed: bool,
    pub copy_mode: bool,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ZoomResult {
    pub zoomed: Option<PaneId>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PaneReadResult {
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FocusResult {
    pub focus: Focus,
}

/// One entry per method: variant, wire name, params type, result type. The macro writes
/// `Method`, `NAMES`, `name`, `from_request`, `from_action` and `schema` from this one table
/// so they cannot drift.
macro_rules! methods {
    ($( $variant:ident = $name:literal : $params:ty => $result:ty ),* $(,)?) => {
        /// A parsed API method.
        ///
        /// This value does not retain the original action text. Callers that need to look up a
        /// configured key with `keymap::key_for` must keep the `Action` alongside this value.
        #[derive(Debug, Clone, PartialEq)]
        pub enum Method {
            $( $variant($params), )*
        }

        impl Method {
            pub const NAMES: &'static [&'static str] = &[ $( $name, )* ];

            pub fn name(&self) -> &'static str {
                match self { $( Method::$variant(_) => $name, )* }
            }

            pub fn from_request(name: &str, params: Value) -> Result<Method, ApiError> {
                let params = if params.is_null() { Value::Object(Default::default()) } else { params };
                match name {
                    $( $name => serde_json::from_value::<$params>(params).map(Method::$variant).map_err(|e| ApiError::invalid_params(format!("{}: {e}", $name))), )*
                    other => Err(ApiError::not_found(format!("method {other} does not exist; run domux2 api schema for the list"))),
                }
            }

            pub fn from_action(action: &Action) -> Result<Method, ApiError> {
                match action.method.as_str() {
                    $( $name => <$params as Params>::from_args(&action.args).map(Method::$variant), )*
                    other => Err(ApiError::not_found(format!("action {other:?} is not a method; run domux2 api schema for the list"))),
                }
            }
        }

        /// The schema `domux2 api schema` prints: every method's params and result, and
        /// the event union.
        pub fn schema() -> Value {
            let mut methods = serde_json::Map::new();
            $(
                methods.insert($name.to_string(), serde_json::json!({
                    "params": schemars::schema_for!($params),
                    "result": schemars::schema_for!($result),
                }));
            )*
            serde_json::json!({
                "methods": methods,
                "events": schemars::schema_for!(Event),
                "request": schemars::schema_for!(Request),
                "response": schemars::schema_for!(Response),
            })
        }
    };
}

methods! {
    ServerInfo = "server.info": NoParams => ServerInfo,
    ServerStop = "server.stop": NoParams => Ack,
    EventsSubscribe = "events.subscribe": SubscribeParams => Ack,
    ConfigReload = "config.reload": NoParams => ConfigReloadResult,
    ClientDetach = "client.detach": ClientParams => Ack,
    Help = "help": ClientParams => Ack,
    TabList = "tab.list": TabListParams => Vec<TabInfo>,
    TabCreate = "tab.create": TabCreateParams => TabInfo,
    TabRename = "tab.rename": TabRenameParams => Ack,
    TabClearName = "tab.clear_name": TabTargetParams => Ack,
    TabClose = "tab.close": TabTargetParams => Ack,
    TabSelect = "tab.select": TabSelectParams => Ack,
    PaneList = "pane.list": TabTargetParams => Vec<PaneInfo>,
    PaneSplit = "pane.split": PaneSplitParams => PaneInfo,
    PaneClose = "pane.close": PaneTargetParams => Ack,
    PaneFocus = "pane.focus": PaneTargetParams => Ack,
    PaneZoom = "pane.zoom": PaneTargetParams => ZoomResult,
    PaneCopyMode = "pane.copy_mode": PaneTargetParams => Ack,
    PaneResize = "pane.resize": PaneResizeParams => Ack,
    PaneSendText = "pane.send_text": PaneSendTextParams => Ack,
    PaneSendKey = "pane.send_key": PaneSendKeyParams => Ack,
    PaneRead = "pane.read": PaneReadParams => PaneReadResult,
    FocusLeft = "focus.left": ClientParams => FocusResult,
    FocusRight = "focus.right": ClientParams => FocusResult,
    FocusUp = "focus.up": ClientParams => FocusResult,
    FocusDown = "focus.down": ClientParams => FocusResult,
    FocusLast = "focus.last": ClientParams => FocusResult,
    FocusRegion = "focus.region": FocusRegionParams => FocusResult,
    FocusPane = "focus.pane": ClientParams => FocusResult,
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXPECTED_METHOD_NAMES: &[&str] = &[
        "server.info",
        "server.stop",
        "events.subscribe",
        "config.reload",
        "client.detach",
        "help",
        "tab.list",
        "tab.create",
        "tab.rename",
        "tab.clear_name",
        "tab.close",
        "tab.select",
        "pane.list",
        "pane.split",
        "pane.close",
        "pane.focus",
        "pane.zoom",
        "pane.copy_mode",
        "pane.resize",
        "pane.send_text",
        "pane.send_key",
        "pane.read",
        "focus.left",
        "focus.right",
        "focus.up",
        "focus.down",
        "focus.last",
        "focus.region",
        "focus.pane",
    ];

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

    #[test]
    fn every_default_keymap_action_resolves_to_a_method() {
        let km = crate::keymap::Keymap::defaults();
        for b in km.bindings.iter().chain(km.global.iter()) {
            Method::from_action(&b.action)
                .unwrap_or_else(|e| panic!("{} -> {}: {e}", b.key, b.action));
        }
    }

    #[test]
    fn requests_parse_by_name_and_params_object() {
        let m = Method::from_request("pane.split", serde_json::json!({"dir": "right"})).unwrap();
        assert_eq!(m.name(), "pane.split");
        match m {
            Method::PaneSplit(p) => assert_eq!(p.dir, crate::model::Direction::Right),
            other => panic!("{other:?}"),
        }
        let m = Method::from_request("server.info", Value::Null).unwrap();
        assert!(matches!(m, Method::ServerInfo(_)));
        let err = Method::from_request("pane.explode", Value::Null).unwrap_err();
        assert_eq!(err.code, ErrorCode::NotFound);
        assert_eq!(
            err.message,
            "method pane.explode does not exist; run domux2 api schema for the list"
        );
        let err =
            Method::from_request("pane.split", serde_json::json!({"dir": "sideways"})).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidParams);
    }

    #[test]
    fn requests_reject_unknown_params_fields_for_every_method() {
        let cases = [
            ("server.info", serde_json::json!({})),
            ("server.stop", serde_json::json!({})),
            ("events.subscribe", serde_json::json!({})),
            ("config.reload", serde_json::json!({})),
            ("client.detach", serde_json::json!({})),
            ("help", serde_json::json!({})),
            ("tab.list", serde_json::json!({})),
            ("tab.create", serde_json::json!({})),
            ("tab.rename", serde_json::json!({})),
            ("tab.clear_name", serde_json::json!({})),
            ("tab.close", serde_json::json!({})),
            ("tab.select", serde_json::json!({"tab": "1"})),
            ("pane.list", serde_json::json!({})),
            ("pane.split", serde_json::json!({"dir": "right"})),
            ("pane.close", serde_json::json!({})),
            ("pane.focus", serde_json::json!({})),
            ("pane.zoom", serde_json::json!({})),
            ("pane.copy_mode", serde_json::json!({})),
            (
                "pane.resize",
                serde_json::json!({"dir": "left", "cells": 2}),
            ),
            ("pane.send_text", serde_json::json!({"text": "hello"})),
            ("pane.send_key", serde_json::json!({"key": "Enter"})),
            ("pane.read", serde_json::json!({})),
            ("focus.left", serde_json::json!({})),
            ("focus.right", serde_json::json!({})),
            ("focus.up", serde_json::json!({})),
            ("focus.down", serde_json::json!({})),
            ("focus.last", serde_json::json!({})),
            ("focus.region", serde_json::json!({"region": "overlay"})),
            ("focus.pane", serde_json::json!({})),
        ];
        assert_eq!(cases.len(), EXPECTED_METHOD_NAMES.len());
        for (name, mut params) in cases {
            params
                .as_object_mut()
                .unwrap()
                .insert("unexpected".into(), Value::from(true));
            let err = Method::from_request(name, params).unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidParams, "{name}");
            assert!(err.message.contains("unknown field"), "{name}: {err}");
        }
    }

    #[test]
    fn positional_args_map_to_params() {
        let a = crate::keymap::Action::parse("pane.resize left 2").unwrap();
        match Method::from_action(&a).unwrap() {
            Method::PaneResize(p) => {
                assert_eq!(p.dir, crate::model::Direction::Left);
                assert_eq!(p.cells, 2);
            }
            other => panic!("{other:?}"),
        }
        let a = crate::keymap::Action::parse("tab.select 3").unwrap();
        assert!(
            matches!(Method::from_action(&a).unwrap(), Method::TabSelect(TabSelectParams { ref tab, .. }) if tab == "3")
        );
        let a = crate::keymap::Action::parse("server.info now").unwrap();
        assert_eq!(
            Method::from_action(&a).unwrap_err().code,
            ErrorCode::InvalidParams
        );
    }

    #[test]
    fn schema_and_method_names_match_the_wire_contract() {
        let s = schema();
        let methods = s["methods"].as_object().unwrap();
        assert_eq!(Method::NAMES, EXPECTED_METHOD_NAMES);
        for name in EXPECTED_METHOD_NAMES {
            assert!(
                methods.contains_key(*name),
                "{name} missing from the schema"
            );
            assert!(methods[*name]["params"].is_object());
            assert!(methods[*name]["result"].is_object());
        }
        assert_eq!(methods.len(), EXPECTED_METHOD_NAMES.len());
        let events = s["events"].to_string();
        for name in Event::NAMES {
            assert!(
                events.contains(name),
                "{name} missing from the events schema"
            );
        }
    }

    #[test]
    fn result_fields_match_the_wire_contract() {
        let zoom = ZoomResult {
            zoomed: Some(PaneId("p_1234".into())),
        };
        assert_eq!(
            serde_json::to_string(&zoom).unwrap(),
            r#"{"zoomed":"p_1234"}"#
        );

        let pane = PaneInfo {
            id: PaneId("p_1234".into()),
            tab: TabId("t_5678".into()),
            cwd: PathBuf::from("/tmp"),
            command: Some("sh".into()),
            title: None,
            pid: Some(42),
            focused: true,
            zoomed: false,
            copy_mode: true,
            cols: 80,
            rows: 24,
        };
        assert_eq!(
            serde_json::to_string(&pane).unwrap(),
            r#"{"id":"p_1234","tab":"t_5678","cwd":"/tmp","command":"sh","title":null,"pid":42,"focused":true,"zoomed":false,"copy_mode":true,"cols":80,"rows":24}"#
        );
    }

    #[test]
    fn focus_results_serialize_both_variants_as_adjacent_tags() {
        let pane = FocusResult {
            focus: Focus::Pane(PaneId("p_1234".into())),
        };
        assert_eq!(
            serde_json::to_string(&pane).unwrap(),
            r#"{"focus":{"kind":"pane","value":"p_1234"}}"#
        );

        let region = FocusResult {
            focus: Focus::Region(RegionKind::Switcher),
        };
        assert_eq!(
            serde_json::to_string(&region).unwrap(),
            r#"{"focus":{"kind":"region","value":"switcher"}}"#
        );
    }

    #[test]
    fn focus_results_round_trip_through_json() {
        for result in [
            FocusResult {
                focus: Focus::Pane(PaneId("p_1234".into())),
            },
            FocusResult {
                focus: Focus::Region(RegionKind::Switcher),
            },
        ] {
            let json = serde_json::to_string(&result).unwrap();
            assert_eq!(serde_json::from_str::<FocusResult>(&json).unwrap(), result);
        }
    }

    #[test]
    fn focus_result_schema_matches_the_adjacent_wire_tags() {
        let result = &schema()["methods"]["focus.left"]["result"];
        let variants = result["$defs"]["Focus"]["oneOf"].as_array().unwrap();
        assert_eq!(variants.len(), 2);

        assert_eq!(variants[0]["properties"]["kind"]["const"], "pane");
        assert_eq!(variants[0]["properties"]["value"]["$ref"], "#/$defs/PaneId");
        assert_eq!(
            variants[0]["required"],
            serde_json::json!(["kind", "value"])
        );

        assert_eq!(variants[1]["properties"]["kind"]["const"], "region");
        assert_eq!(
            variants[1]["properties"]["value"]["$ref"],
            "#/$defs/RegionKind"
        );
        assert_eq!(
            variants[1]["required"],
            serde_json::json!(["kind", "value"])
        );
        assert_eq!(result["$defs"]["PaneId"]["type"], "string");
        assert!(result["$defs"]["RegionKind"]["enum"]
            .as_array()
            .unwrap()
            .contains(&Value::from("switcher")));
    }

    #[test]
    fn responses_carry_the_request_id_and_one_of_result_or_error() {
        let ok = Response::ok(Value::from(7), serde_json::json!({"x": 1}));
        assert_eq!(
            serde_json::to_string(&ok).unwrap(),
            r#"{"id":7,"result":{"x":1}}"#
        );
        let err = Response::err(Value::from("a"), ApiError::refused("main can't be deleted"));
        let v = serde_json::to_value(&err).unwrap();
        assert_eq!(v["id"], "a");
        assert_eq!(v["error"]["code"], "refused");
        assert!(v.get("result").is_none());
    }
}
