//! The control API's wire types: errors, events, and (Task 8) requests, responses and methods.

use crate::facts::FactKey;
use crate::ids::{ClientId, PaneId, ProjectId, TabId, WorkspaceId};
use crate::keymap::Action;
use crate::model::{Direction, Focus, RegionKind};
use crate::names::BIN_NAME;
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
    /// A destructive operation asking for consent. `question` names the object and its
    /// consequence; `removes` and `keeps` are the lists a caller prints under it. The
    /// message is the whole thing in one sentence for a caller that only shows messages,
    /// and `data` is the same content structured for one that can lay it out.
    pub fn needs_confirmation(
        question: impl Into<String>,
        removes: Vec<String>,
        keeps: Vec<String>,
    ) -> ApiError {
        let question = question.into();
        ApiError {
            code: ErrorCode::Refused,
            message: format!("{question} Answer with --yes"),
            data: Some(
                serde_json::json!({ "confirmation": question, "removes": removes, "keeps": keeps }),
            ),
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
    // The project, workspace, switcher, sidebar and list events of M2.
    #[serde(rename = "project.added")]
    ProjectAdded {
        project: ProjectId,
        name: String,
        root: PathBuf,
    },
    #[serde(rename = "project.removed")]
    ProjectRemoved { project: ProjectId, name: String },
    #[serde(rename = "workspace.created")]
    WorkspaceCreated {
        project: ProjectId,
        workspace: WorkspaceId,
        handle: String,
        path: PathBuf,
    },
    #[serde(rename = "workspace.cleared")]
    WorkspaceCleared {
        workspace: WorkspaceId,
        base: String,
    },
    #[serde(rename = "workspace.deleted")]
    WorkspaceDeleted {
        project: ProjectId,
        workspace: WorkspaceId,
        handle: String,
        /// True when the workspace record went because its path was gone, not because
        /// someone deleted it.
        pruned: bool,
    },
    #[serde(rename = "workspace.renamed")]
    WorkspaceRenamed {
        workspace: WorkspaceId,
        name: Option<String>,
    },
    #[serde(rename = "workspace.switched")]
    WorkspaceSwitched {
        client: ClientId,
        workspace: WorkspaceId,
        tab: TabId,
    },
    #[serde(rename = "fact.updated")]
    FactUpdated { key: FactKey, present: bool },
    #[serde(rename = "sidebar.toggled")]
    SidebarToggled { client: ClientId, open: bool },
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

/// Builds `Event::NAMES` and `Event::name` from one list, so a variant cannot exist
/// without a name to answer with. The match has no catch-all: a variant added to `Event`
/// above without a line added here fails to compile with "non-exhaustive patterns"
/// instead of quietly leaving `events.subscribe` unable to filter on it.
macro_rules! event_names {
    ( $( $pat:pat => $name:literal ),* $(,)? ) => {
        impl Event {
            /// The event names of this build, in declaration order. Used by
            /// `events.subscribe` filters and by the schema.
            pub const NAMES: &'static [&'static str] = &[ $( $name, )* ];

            pub fn name(&self) -> &'static str {
                match self {
                    $( $pat => $name, )*
                }
            }
        }
    };
}

event_names! {
    Event::ServerStarted { .. } => "server.started",
    Event::ServerStopping => "server.stopping",
    Event::ClientAttached { .. } => "client.attached",
    Event::ClientDetached { .. } => "client.detached",
    Event::ConfigReloaded { .. } => "config.reloaded",
    Event::ProjectAdded { .. } => "project.added",
    Event::ProjectRemoved { .. } => "project.removed",
    Event::WorkspaceCreated { .. } => "workspace.created",
    Event::WorkspaceCleared { .. } => "workspace.cleared",
    Event::WorkspaceDeleted { .. } => "workspace.deleted",
    Event::WorkspaceRenamed { .. } => "workspace.renamed",
    Event::WorkspaceSwitched { .. } => "workspace.switched",
    Event::FactUpdated { .. } => "fact.updated",
    Event::SidebarToggled { .. } => "sidebar.toggled",
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

impl Event {
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
    /// The tab's name, for a caller that already knows it. `None` leaves the tab with its
    /// number, which is what every key and every other caller wants.
    ///
    /// It exists because `tab.rename` resolves its target inside the calling client's
    /// workspace, so a caller with no view cannot name a tab it just made somewhere else,
    /// and because a name is not always an afterthought: in V1's session file a window and
    /// its name are one fact, and splitting them into two calls is V2's artefact rather
    /// than something the caller meant (found by Task 23).
    #[serde(default)]
    pub name: Option<String>,
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
    ///
    /// Screen rows, and the answer can hold fewer lines than that: a line the screen wrapped is
    /// returned as the one line it was written as, so a wrapped URL reads back as a URL rather
    /// than as two halves. The count bounds what is read, not what comes back.
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
    /// The leader the running server is using, as a key name. The config file can disagree
    /// with it - it is read at start and on `config.reload`, so an edit not yet reloaded is
    /// not in force - and when the two disagree this is the one that answers keys.
    pub leader: String,
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
    /// Where the tab's focused pane is, as the server last read it. That is the directory
    /// the tab was opened at until its shell moves, and the shell's own directory after,
    /// because the server polls it from the pane rather than recording where it started.
    ///
    /// Without this field a tab's directory can only be read through `pane.list`, which
    /// resolves its target inside the calling client's workspace, so nothing could ask
    /// where a tab is unless it was already looking at it (found by Task 23).
    pub cwd: PathBuf,
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
                    other => Err(ApiError::not_found(format!("method {other} does not exist; run {BIN_NAME} api schema for the list"))),
                }
            }

            pub fn from_action(action: &Action) -> Result<Method, ApiError> {
                match action.method.as_str() {
                    $( $name => <$params as Params>::from_args(&action.args).map(Method::$variant), )*
                    other => Err(ApiError::not_found(format!("action {other:?} is not a method; run {BIN_NAME} api schema for the list"))),
                }
            }
        }

        /// The schema that `api schema` prints: every method's params and result, and
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
    PaneClear = "pane.clear": PaneTargetParams => Ack,
    FocusLeft = "focus.left": ClientParams => FocusResult,
    FocusRight = "focus.right": ClientParams => FocusResult,
    FocusUp = "focus.up": ClientParams => FocusResult,
    FocusDown = "focus.down": ClientParams => FocusResult,
    FocusLast = "focus.last": ClientParams => FocusResult,
    FocusRegion = "focus.region": FocusRegionParams => FocusResult,
    FocusPane = "focus.pane": ClientParams => FocusResult,
    ProjectList = "project.list": NoParams => Vec<ProjectInfo>,
    ProjectAdd = "project.add": ProjectAddParams => ProjectAdded,
    ProjectRemove = "project.remove": ProjectRemoveParams => Ack,
    WorkspaceList = "workspace.list": WorkspaceListParams => Vec<WorkspaceInfo>,
    WorkspaceCreate = "workspace.create": WorkspaceCreateParams => WorkspaceCreated,
    WorkspaceClear = "workspace.clear": WorkspaceClearParams => Ack,
    WorkspaceDelete = "workspace.delete": WorkspaceDeleteParams => Ack,
    WorkspaceRename = "workspace.rename": WorkspaceRenameParams => Ack,
    WorkspaceClearName = "workspace.clear_name": WorkspaceTargetParams => Ack,
    WorkspaceFocus = "workspace.focus": WorkspaceFocusParams => WorkspaceInfo,
    WorkspaceResume = "workspace.resume": WorkspaceTargetParams => Ack,
    SwitcherOpen = "switcher.open": ClientParams => Ack,
    SwitcherClose = "switcher.close": ClientParams => Ack,
    SidebarToggle = "sidebar.toggle": ClientParams => SidebarResult,
    SidebarShow = "sidebar.show": ClientParams => SidebarResult,
    SidebarHide = "sidebar.hide": ClientParams => SidebarResult,
    ListDown = "list.down": ClientParams => Ack,
    ListUp = "list.up": ClientParams => Ack,
    ListActivate = "list.activate": ClientParams => Ack,
    ListFilter = "list.filter": ClientParams => Ack,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectAddParams {
    pub path: String,
    #[serde(default)]
    pub client: Option<ClientId>,
}
impl Params for ProjectAddParams {
    fn from_args(args: &[String]) -> Result<Self, ApiError> {
        Ok(ProjectAddParams {
            path: arg::<String>(args, 0, "path")?,
            client: None,
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectRemoveParams {
    /// A project id or name. Absent with `all`, and absent for a key in a Projects box,
    /// which acts on the project of the row under the cursor.
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub yes: bool,
    /// Let every project go at once, for starting over. Names no project, because the answer
    /// to "which one" is all of them.
    #[serde(default)]
    pub all: bool,
}
impl Params for ProjectRemoveParams {
    fn from_args(args: &[String]) -> Result<Self, ApiError> {
        Ok(ProjectRemoveParams {
            // Optional, so `X = "project.remove"` in `[keys.list]` is a binding and not a
            // parse error: a key in a box names its target by where the cursor is.
            project: args.first().cloned(),
            yes: false,
            all: false,
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceListParams {
    #[serde(default)]
    pub project: Option<String>,
}
impl Params for WorkspaceListParams {
    fn from_args(args: &[String]) -> Result<Self, ApiError> {
        Ok(WorkspaceListParams {
            project: args.first().cloned(),
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceCreateParams {
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub base: Option<String>,
    #[serde(default)]
    pub client: Option<ClientId>,
}
impl Params for WorkspaceCreateParams {
    fn from_args(args: &[String]) -> Result<Self, ApiError> {
        Ok(WorkspaceCreateParams {
            project: args.first().cloned(),
            base: args.get(1).cloned(),
            client: None,
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceClearParams {
    #[serde(default)]
    pub workspace: Option<String>,
    #[serde(default)]
    pub yes: bool,
}
impl Params for WorkspaceClearParams {
    fn from_args(args: &[String]) -> Result<Self, ApiError> {
        Ok(WorkspaceClearParams {
            workspace: args.first().cloned(),
            yes: false,
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceDeleteParams {
    pub workspace: String,
    #[serde(default)]
    pub yes: bool,
    #[serde(default)]
    pub force: bool,
}
impl Params for WorkspaceDeleteParams {
    fn from_args(args: &[String]) -> Result<Self, ApiError> {
        Ok(WorkspaceDeleteParams {
            workspace: arg::<String>(args, 0, "workspace")?,
            yes: false,
            force: false,
        })
    }
}

/// `None` opens the name box in the calling view; `Some("")` clears the name. The same
/// rule as M1's `tab.rename` (M1 assumption 15).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceRenameParams {
    #[serde(default)]
    pub workspace: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub client: Option<ClientId>,
}
// `leader N` binds `workspace.rename` with no arguments, which must mean "open the name box
// here": no workspace, no name. Arguments become the name, joined, exactly as `tab.rename`
// does it.
impl Params for WorkspaceRenameParams {
    fn from_args(args: &[String]) -> Result<Self, ApiError> {
        Ok(WorkspaceRenameParams {
            workspace: None,
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
pub struct WorkspaceTargetParams {
    #[serde(default)]
    pub workspace: Option<String>,
}
// One optional positional target: `leader n` passes none and means "this workspace".
impl Params for WorkspaceTargetParams {
    fn from_args(args: &[String]) -> Result<Self, ApiError> {
        Ok(WorkspaceTargetParams {
            workspace: args.first().cloned(),
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceFocusParams {
    pub workspace: String,
    #[serde(default)]
    pub client: Option<ClientId>,
}
// A required positional target, through M1's `arg` helper so a missing one reports which
// argument it was.
impl Params for WorkspaceFocusParams {
    fn from_args(args: &[String]) -> Result<Self, ApiError> {
        Ok(WorkspaceFocusParams {
            workspace: arg::<String>(args, 0, "workspace")?,
            client: None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProjectInfo {
    pub id: ProjectId,
    pub name: String,
    pub root: PathBuf,
    pub kind: String,
    pub default_branch: Option<String>,
    pub workspaces: usize,
}

/// `adopted` lists the handles of the worktrees `project.add` found on disk, so the CLI's
/// `open` subcommand can say `added audrey-app with workspace-1, workspace-2`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProjectAdded {
    pub project: ProjectId,
    pub workspace: WorkspaceId,
    pub name: String,
    pub root: PathBuf,
    pub kind: String,
    pub adopted: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceInfo {
    pub id: WorkspaceId,
    pub project: ProjectId,
    pub handle: String,
    pub name: Option<String>,
    pub path: PathBuf,
    pub branch: Option<String>,
    pub pr: Option<String>,
    pub pr_state: Option<String>,
    pub tabs: usize,
}

/// What `workspace.create` answers with. `WorkspaceInfo` cannot serve: a create has two
/// facts of its own that no other call has, and neither may be guessed later. `base` is the
/// ref the slot was branched from, and `setup` is what `worktree.conf` did, `None` when the
/// project has no `worktree.conf` at all (principle 4: absent, not "linked 0, copied 0").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceCreated {
    pub id: WorkspaceId,
    pub project: ProjectId,
    pub handle: String,
    pub path: PathBuf,
    pub branch: String,
    pub base: String,
    pub setup: Option<String>,
    pub tabs: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SidebarResult {
    pub open: bool,
    pub visible: bool,
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
        "pane.clear",
        "focus.left",
        "focus.right",
        "focus.up",
        "focus.down",
        "focus.last",
        "focus.region",
        "focus.pane",
        "project.list",
        "project.add",
        "project.remove",
        "workspace.list",
        "workspace.create",
        "workspace.clear",
        "workspace.delete",
        "workspace.rename",
        "workspace.clear_name",
        "workspace.focus",
        "workspace.resume",
        "switcher.open",
        "switcher.close",
        "sidebar.toggle",
        "sidebar.show",
        "sidebar.hide",
        "list.down",
        "list.up",
        "list.activate",
        "list.filter",
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
        for b in km
            .bindings
            .iter()
            .chain(km.global.iter())
            .chain(km.list.iter())
        {
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
            ("pane.clear", serde_json::json!({})),
            ("focus.left", serde_json::json!({})),
            ("focus.right", serde_json::json!({})),
            ("focus.up", serde_json::json!({})),
            ("focus.down", serde_json::json!({})),
            ("focus.last", serde_json::json!({})),
            ("focus.region", serde_json::json!({"region": "overlay"})),
            ("focus.pane", serde_json::json!({})),
            ("project.list", serde_json::json!({})),
            ("project.add", serde_json::json!({"path": "/x"})),
            ("project.remove", serde_json::json!({"project": "p"})),
            ("workspace.list", serde_json::json!({})),
            ("workspace.create", serde_json::json!({"project": "p"})),
            ("workspace.clear", serde_json::json!({"workspace": "w"})),
            ("workspace.delete", serde_json::json!({"workspace": "w"})),
            (
                "workspace.rename",
                serde_json::json!({"workspace": "w", "name": "x"}),
            ),
            (
                "workspace.clear_name",
                serde_json::json!({"workspace": "w"}),
            ),
            ("workspace.focus", serde_json::json!({"workspace": "w"})),
            ("workspace.resume", serde_json::json!({"workspace": "w"})),
            ("switcher.open", serde_json::json!({})),
            ("switcher.close", serde_json::json!({})),
            ("sidebar.toggle", serde_json::json!({})),
            ("sidebar.show", serde_json::json!({})),
            ("sidebar.hide", serde_json::json!({})),
            ("list.down", serde_json::json!({})),
            ("list.up", serde_json::json!({})),
            ("list.activate", serde_json::json!({})),
            ("list.filter", serde_json::json!({})),
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
    fn a_confirmation_refusal_carries_the_same_content_in_the_message_and_the_data() {
        let e = ApiError::needs_confirmation(
            "Delete workspace-1? Removes the worktree and the local branch and closes 2 tabs.",
            vec![
                "the worktree at .domux/worktrees/workspace-1".into(),
                "the local branch workspace-1".into(),
            ],
            vec!["the remote branch and any pull request".into()],
        );
        assert_eq!(e.code, ErrorCode::Refused);
        assert!(e.message.ends_with(" Answer with --yes"), "{}", e.message);
        let data = e.data.unwrap();
        assert_eq!(
            data["confirmation"],
            "Delete workspace-1? Removes the worktree and the local branch and closes 2 tabs."
        );
        assert_eq!(data["removes"][1], "the local branch workspace-1");
        assert_eq!(data["keeps"][0], "the remote branch and any pull request");
    }

    #[test]
    fn every_m2_method_parses_from_its_name_and_answers_to_it() {
        // One params object per method, because the params structs deny unknown fields: a
        // single blob of every key would be rejected by the methods that take none.
        let cases = [
            ("project.list", serde_json::json!({})),
            ("project.add", serde_json::json!({"path": "/x"})),
            ("project.remove", serde_json::json!({"project": "p"})),
            ("workspace.list", serde_json::json!({})),
            ("workspace.create", serde_json::json!({"project": "p"})),
            ("workspace.clear", serde_json::json!({"workspace": "w"})),
            ("workspace.delete", serde_json::json!({"workspace": "w"})),
            (
                "workspace.rename",
                serde_json::json!({"workspace": "w", "name": "x"}),
            ),
            (
                "workspace.clear_name",
                serde_json::json!({"workspace": "w"}),
            ),
            ("workspace.focus", serde_json::json!({"workspace": "w"})),
            ("workspace.resume", serde_json::json!({"workspace": "w"})),
            ("switcher.open", serde_json::json!({})),
            ("switcher.close", serde_json::json!({})),
            ("sidebar.toggle", serde_json::json!({})),
            ("sidebar.show", serde_json::json!({})),
            ("sidebar.hide", serde_json::json!({})),
            ("list.down", serde_json::json!({})),
            ("list.up", serde_json::json!({})),
            ("list.activate", serde_json::json!({})),
            ("list.filter", serde_json::json!({})),
        ];
        for (name, params) in cases {
            assert!(
                Method::NAMES.contains(&name),
                "{name} is missing from Method::NAMES"
            );
            let m = Method::from_request(name, params).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            assert_eq!(m.name(), name);
        }
    }

    #[test]
    fn every_m2_method_also_parses_from_a_keybinding_action() {
        // `from_action` goes through `Params::from_args`, which every params type in the table
        // has to implement for the macro to compile at all. This pins the arguments each one
        // takes from a key.
        let cases: [(&str, &[&str]); 21] = [
            ("project.list", &[]),
            ("project.add", &["/x"]),
            ("project.remove", &["audrey-app"]),
            // `X` in the Projects box: the project is the row under the cursor, so the
            // binding names none.
            ("project.remove", &[]),
            ("workspace.list", &[]),
            ("workspace.create", &["audrey-app"]),
            ("workspace.clear", &[]),
            ("workspace.delete", &["workspace-1"]),
            ("workspace.rename", &[]),
            ("workspace.clear_name", &[]),
            ("workspace.focus", &["workspace-1"]),
            ("workspace.resume", &[]),
            ("switcher.open", &[]),
            ("switcher.close", &[]),
            ("sidebar.toggle", &[]),
            ("sidebar.show", &[]),
            ("sidebar.hide", &[]),
            ("list.down", &[]),
            ("list.up", &[]),
            ("list.activate", &[]),
            ("list.filter", &[]),
        ];
        for (name, args) in cases {
            let action = Action {
                method: name.to_string(),
                args: args.iter().map(|a| a.to_string()).collect(),
            };
            let m = Method::from_action(&action).unwrap_or_else(|e| panic!("{name}: {e:?}"));
            assert_eq!(m.name(), name);
        }
        // The three that need an argument say which one is missing rather than inventing a
        // target. `project.remove` is not among them: it has a target a key can see, the row
        // the cursor is on, and `api::project::remove` refuses when the keys are in no box.
        for name in ["workspace.delete", "workspace.focus", "project.add"] {
            let action = Action {
                method: name.to_string(),
                args: Vec::new(),
            };
            assert_eq!(
                Method::from_action(&action).unwrap_err().code,
                ErrorCode::InvalidParams,
                "{name}"
            );
        }
    }

    #[test]
    fn workspace_rename_takes_an_optional_name_the_way_tab_rename_does() {
        let open = Method::from_request("workspace.rename", serde_json::json!({})).unwrap();
        assert!(
            matches!(&open, Method::WorkspaceRename(p) if p.name.is_none()),
            "no name opens the name box"
        );
        let clear =
            Method::from_request("workspace.rename", serde_json::json!({"name": ""})).unwrap();
        assert!(
            matches!(&clear, Method::WorkspaceRename(p) if p.name.as_deref() == Some("")),
            "an empty name clears"
        );
    }

    #[test]
    fn destructive_methods_default_to_keeping_data() {
        let del = Method::from_request(
            "workspace.delete",
            serde_json::json!({"workspace": "workspace-1"}),
        )
        .unwrap();
        assert!(
            matches!(&del, Method::WorkspaceDelete(p) if !p.yes && !p.force),
            "delete confirms unless told not to"
        );
        let rm = Method::from_request(
            "project.remove",
            serde_json::json!({"project": "audrey-app"}),
        )
        .unwrap();
        assert!(matches!(&rm, Method::ProjectRemove(p) if !p.yes));
    }

    #[test]
    fn every_m2_event_has_a_name_and_serializes_with_it() {
        let events = vec![
            Event::ProjectAdded {
                project: ProjectId("pr_1".into()),
                name: "x".into(),
                root: PathBuf::from("/x"),
            },
            Event::ProjectRemoved {
                project: ProjectId("pr_1".into()),
                name: "x".into(),
            },
            Event::WorkspaceCreated {
                project: ProjectId("pr_1".into()),
                workspace: WorkspaceId("w_1".into()),
                handle: "workspace-1".into(),
                path: PathBuf::from("/x"),
            },
            Event::WorkspaceCleared {
                workspace: WorkspaceId("w_1".into()),
                base: "origin/main".into(),
            },
            Event::WorkspaceDeleted {
                project: ProjectId("pr_1".into()),
                workspace: WorkspaceId("w_1".into()),
                handle: "workspace-1".into(),
                pruned: false,
            },
            Event::WorkspaceRenamed {
                workspace: WorkspaceId("w_1".into()),
                name: Some("auth cleanup".into()),
            },
            Event::WorkspaceSwitched {
                client: ClientId("c_1".into()),
                workspace: WorkspaceId("w_1".into()),
                tab: TabId("t_1".into()),
            },
            Event::FactUpdated {
                key: FactKey::workspace(&WorkspaceId("w_1".into()), crate::facts::FACT_PR),
                present: true,
            },
            Event::SidebarToggled {
                client: ClientId("c_1".into()),
                open: true,
            },
        ];
        for e in &events {
            assert!(
                Event::NAMES.contains(&e.name()),
                "{} is missing from Event::NAMES",
                e.name()
            );
            let value = serde_json::to_value(e).unwrap();
            assert_eq!(value["event"], e.name());
        }
        assert_eq!(
            serde_json::to_value(&events[7]).unwrap()["key"],
            "w_1/pr",
            "a fact key is a string on the wire"
        );
    }

    /// The compile-time guard `event_names!` gives: this is the runtime half, pinning that
    /// every M1 and M2 variant actually round-trips through `NAMES`, not just that the
    /// macro's match is exhaustive. Kept as one list so a new variant that is missing from
    /// here (as opposed to missing from `event_names!`, which fails to compile) is still
    /// caught: this test is the fallback for whichever half of the guard a change slips past.
    #[test]
    fn every_known_event_variant_is_reachable_through_names() {
        let samples = vec![
            Event::ServerStarted {
                version: "0".into(),
                socket: PathBuf::from("/s"),
            },
            Event::ServerStopping,
            Event::ClientAttached {
                client: ClientId("c_1".into()),
            },
            Event::ClientDetached {
                client: ClientId("c_1".into()),
            },
            Event::ConfigReloaded { error: None },
            Event::ProjectAdded {
                project: ProjectId("pr_1".into()),
                name: "x".into(),
                root: PathBuf::from("/x"),
            },
            Event::ProjectRemoved {
                project: ProjectId("pr_1".into()),
                name: "x".into(),
            },
            Event::WorkspaceCreated {
                project: ProjectId("pr_1".into()),
                workspace: WorkspaceId("w_1".into()),
                handle: "workspace-1".into(),
                path: PathBuf::from("/x"),
            },
            Event::WorkspaceCleared {
                workspace: WorkspaceId("w_1".into()),
                base: "origin/main".into(),
            },
            Event::WorkspaceDeleted {
                project: ProjectId("pr_1".into()),
                workspace: WorkspaceId("w_1".into()),
                handle: "workspace-1".into(),
                pruned: false,
            },
            Event::WorkspaceRenamed {
                workspace: WorkspaceId("w_1".into()),
                name: None,
            },
            Event::WorkspaceSwitched {
                client: ClientId("c_1".into()),
                workspace: WorkspaceId("w_1".into()),
                tab: TabId("t_1".into()),
            },
            Event::FactUpdated {
                key: FactKey::workspace(&WorkspaceId("w_1".into()), crate::facts::FACT_PR),
                present: true,
            },
            Event::SidebarToggled {
                client: ClientId("c_1".into()),
                open: true,
            },
            Event::TabCreated {
                workspace: WorkspaceId("w_1".into()),
                tab: TabId("t_1".into()),
            },
            Event::TabRenamed {
                tab: TabId("t_1".into()),
                name: None,
            },
            Event::TabClosed {
                workspace: WorkspaceId("w_1".into()),
                tab: TabId("t_1".into()),
            },
            Event::TabSelected {
                client: ClientId("c_1".into()),
                tab: TabId("t_1".into()),
            },
            Event::PaneSpawned {
                tab: TabId("t_1".into()),
                pane: PaneId("p_1234".into()),
                cwd: PathBuf::from("/x"),
            },
            Event::PaneExited {
                pane: PaneId("p_1234".into()),
                status: None,
            },
            Event::PaneClosed {
                tab: TabId("t_1".into()),
                pane: PaneId("p_1234".into()),
            },
            Event::PaneFocused {
                tab: TabId("t_1".into()),
                pane: PaneId("p_1234".into()),
            },
            Event::PaneResized {
                pane: PaneId("p_1234".into()),
                cols: 80,
                rows: 24,
            },
            Event::PaneZoomed {
                tab: TabId("t_1".into()),
                pane: None,
            },
        ];
        assert_eq!(
            samples.len(),
            Event::NAMES.len(),
            "a variant was added to Event without a sample here, or NAMES grew without a \
             matching variant; event_names! already refuses to compile the first way, so a \
             mismatch here is the second"
        );
        for e in &samples {
            assert!(
                Event::NAMES.contains(&e.name()),
                "{} is missing from Event::NAMES",
                e.name()
            );
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

    /// The M2 results, the same way `result_fields_match_the_wire_contract` pins M1's: one
    /// full literal string per struct. Params are pinned elsewhere by `deny_unknown_fields`
    /// seeing real request JSON; a result is never sent back through it, so nothing else in
    /// this file would notice a renamed, reordered, dropped or newly added field. `setup`
    /// and `name` are asserted as `null` on purpose, not omitted: an absent key and a `null`
    /// key are different wire shapes, and `WorkspaceCreated.setup` in particular exists to
    /// say "no worktree.conf at all" (principle 4), which only holds if the key is there.
    #[test]
    fn every_m2_result_serializes_with_the_field_names_the_contract_names() {
        let project = ProjectInfo {
            id: ProjectId("pr_1".into()),
            name: "audrey-app".into(),
            root: PathBuf::from("/x"),
            kind: "git".into(),
            default_branch: Some("main".into()),
            workspaces: 2,
        };
        assert_eq!(
            serde_json::to_string(&project).unwrap(),
            r#"{"id":"pr_1","name":"audrey-app","root":"/x","kind":"git","default_branch":"main","workspaces":2}"#
        );

        let added = ProjectAdded {
            project: ProjectId("pr_1".into()),
            workspace: WorkspaceId("w_1".into()),
            name: "audrey-app".into(),
            root: PathBuf::from("/x"),
            kind: "git".into(),
            adopted: vec!["workspace-1".into()],
        };
        assert_eq!(
            serde_json::to_string(&added).unwrap(),
            r#"{"project":"pr_1","workspace":"w_1","name":"audrey-app","root":"/x","kind":"git","adopted":["workspace-1"]}"#
        );

        let workspace = WorkspaceInfo {
            id: WorkspaceId("w_1".into()),
            project: ProjectId("pr_1".into()),
            handle: "workspace-1".into(),
            name: None,
            path: PathBuf::from("/x"),
            branch: Some("workspace-1".into()),
            pr: Some("PR#212".into()),
            pr_state: Some("OPEN".into()),
            tabs: 1,
        };
        assert_eq!(
            serde_json::to_string(&workspace).unwrap(),
            r#"{"id":"w_1","project":"pr_1","handle":"workspace-1","name":null,"path":"/x","branch":"workspace-1","pr":"PR#212","pr_state":"OPEN","tabs":1}"#
        );
        // The same row with nothing observed about it. The literal above sets every fact, so
        // on its own it says nothing about how an absent one is written, which is the half
        // this test's doc comment is about: a workspace with no pull request answers `null`,
        // not a key that is not there. `serde_json`'s `Index` returns `Value::Null` for a
        // missing key too, so a caller reading `row["pr"]` cannot tell them apart and the
        // shape has to be pinned here, on the string.
        let unobserved = WorkspaceInfo {
            branch: None,
            pr: None,
            pr_state: None,
            ..workspace
        };
        assert_eq!(
            serde_json::to_string(&unobserved).unwrap(),
            r#"{"id":"w_1","project":"pr_1","handle":"workspace-1","name":null,"path":"/x","branch":null,"pr":null,"pr_state":null,"tabs":1}"#
        );

        let created = WorkspaceCreated {
            id: WorkspaceId("w_1".into()),
            project: ProjectId("pr_1".into()),
            handle: "workspace-1".into(),
            path: PathBuf::from("/x"),
            branch: "workspace-1".into(),
            base: "origin/main".into(),
            setup: None,
            tabs: 1,
        };
        assert_eq!(
            serde_json::to_string(&created).unwrap(),
            r#"{"id":"w_1","project":"pr_1","handle":"workspace-1","path":"/x","branch":"workspace-1","base":"origin/main","setup":null,"tabs":1}"#
        );

        let sidebar = SidebarResult {
            open: true,
            visible: false,
        };
        assert_eq!(
            serde_json::to_string(&sidebar).unwrap(),
            r#"{"open":true,"visible":false}"#
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
