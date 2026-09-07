//! One handler per method. A keybinding, a CLI subcommand and an API request all arrive
//! here as a `Method` and leave as a `Value` or an `ApiError`.

pub mod server;

use crate::client::ClientConn;
use crate::core::CoreMsg;
use crate::pane::PaneRuntime;
use crate::{CoreDeps, LoadedConfig};
use domux_core::api::{ApiError, Event, Method};
use domux_core::ids::{ClientId, PaneId};
use domux_core::model::Model;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::sync::mpsc;

/// Everything a handler may touch. Built by the core for one call.
pub struct Ctx<'a> {
    pub model: &'a mut Model,
    pub panes: &'a mut HashMap<PaneId, PaneRuntime>,
    pub clients: &'a mut HashMap<ClientId, ClientConn>,
    pub config: &'a mut LoadedConfig,
    pub deps: &'a CoreDeps,
    pub core_tx: &'a mpsc::Sender<CoreMsg>,
    pub socket_path: &'a PathBuf,
    pub state_dir: &'a PathBuf,
    pub started_at: &'a str,
    /// The view the call acts on: the pressing client for a keybinding, the `client` param
    /// or the most recent client for an API call.
    pub client: Option<ClientId>,
    pub events: Vec<Event>,
    pub stop_requested: bool,
}

impl Ctx<'_> {
    /// The client a view method acts on, or `not_found` when none is attached.
    pub fn view(&self) -> Result<ClientId, ApiError> {
        self.client
            .clone()
            .or_else(|| self.model.most_recent_client())
            .ok_or_else(|| ApiError::not_found("no client is attached; run domux2 to attach one"))
    }
}

pub fn dispatch(method: Method, ctx: &mut Ctx) -> Result<Value, ApiError> {
    match method {
        Method::ServerInfo(_) => server::info(ctx),
        Method::ServerStop(_) => server::stop(ctx),
        Method::EventsSubscribe(_) => Err(ApiError::invalid_params(
            "events.subscribe only works on a control API connection, where it turns the connection into a stream",
        )),
        other => Err(ApiError::unavailable(format!(
            "{} is not built yet",
            other.name()
        ))),
    }
}

pub fn ok<T: serde::Serialize>(value: T) -> Result<Value, ApiError> {
    serde_json::to_value(value).map_err(|e| ApiError::internal(e.to_string()))
}
