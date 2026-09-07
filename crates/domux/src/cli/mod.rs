//! One file per API namespace. Each subcommand is one API call.
//!
//! Machine output follows principle 12: stdout carries data, stderr carries messages, and
//! a failure leaves a status of 1. So `pane read` and `api` print on stdout, while
//! "Server started" and "Config reloaded." print on stderr, and a subcommand that only
//! changed something says nothing at all.

pub mod api;
pub mod attach;
pub mod config;
pub mod events;
pub mod pane;
pub mod server;
pub mod tab;

use domux_client::control;
use domux_core::api::ApiError;
use domux_core::names::BIN_NAME;
use serde_json::Value;
use std::path::PathBuf;

pub fn socket() -> PathBuf {
    domux_core::paths::socket_path()
}

/// The one message for a server that is not listening, so every subcommand names the same
/// state, object and next action (principle 9).
pub fn not_running() -> anyhow::Error {
    anyhow::anyhow!("The server is not running. Start it with {BIN_NAME} server start.")
}

/// Calls a method and turns transport and API errors into one message for stderr.
pub async fn call(method: &str, params: Value) -> anyhow::Result<Value> {
    let socket = socket();
    if !control::is_live(&socket) {
        return Err(not_running());
    }
    match control::call(&socket, method, params).await? {
        Ok(v) => Ok(v),
        Err(e) => Err(api_error(e)),
    }
}

/// Calls a method and reads its answer as the result type this build declares. A server
/// that answered something else is reported rather than read past, so no field the CLI
/// prints was invented here.
pub async fn call_as<T: serde::de::DeserializeOwned>(
    method: &str,
    params: Value,
) -> anyhow::Result<T> {
    let value = call(method, params).await?;
    serde_json::from_value(value).map_err(|e| {
        anyhow::anyhow!(
            "the server's answer to {method} does not match this build of {BIN_NAME}: {e}"
        )
    })
}

/// An API error as one line: the stable code, then the server's own sentence.
fn api_error(e: ApiError) -> anyhow::Error {
    let code = serde_json::to_value(e.code)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "error".to_string());
    anyhow::anyhow!("{code}: {}", e.message)
}

/// Where the shell that runs us lives, from the variables the server sets in every pane.
pub mod location {
    pub fn pane_from_env() -> Option<String> {
        var("DOMUX_PANE")
    }
    pub fn tab_from_env() -> Option<String> {
        var("DOMUX_TAB")
    }
    pub fn workspace_from_env() -> Option<String> {
        var("DOMUX_WORKSPACE")
    }

    /// An empty variable is no answer at all, so it reads as absent rather than as a name
    /// no object has.
    fn var(name: &str) -> Option<String> {
        std::env::var(name).ok().filter(|s| !s.is_empty())
    }
}
