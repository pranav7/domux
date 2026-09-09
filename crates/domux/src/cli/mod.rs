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
pub mod import;
pub mod pane;
pub mod server;
pub mod tab;

use anyhow::Context;
use domux_client::control;
use domux_core::api::ApiError;
use domux_core::names::BIN_NAME;
use serde_json::Value;
use std::io::Write;
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
    if !control::is_live(&socket).await {
        return Err(not_running());
    }
    // The probe cannot rule out a server that dies, or closes the connection, between it and
    // this call. That one case used to reach the reader as a bare "EOF while parsing a value
    // at line 1 column 0", which names neither the state, the object nor the next action.
    let answer = control::call(&socket, method, params)
        .await
        .with_context(|| {
            format!(
                "The server did not answer {method} on {}. Run {BIN_NAME} server status.",
                socket.display()
            )
        })?;
    match answer {
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

/// Whether the reader is still there. A reader that stopped reading is the normal end of
/// output, not a failure, so `events | head` and `api pane.list | head` end quietly.
#[derive(Debug, PartialEq)]
pub enum Wrote {
    Line,
    ReaderGone,
}

/// Writes one line of data. Rust ignores SIGPIPE, so a write to a pipe whose reader has gone
/// returns `BrokenPipe` and `println!` turns that into a panic and a status of 101. Every
/// line of data the CLI prints goes through here instead.
pub fn write_line(out: &mut impl Write, text: &str) -> anyhow::Result<Wrote> {
    match writeln!(out, "{text}").and_then(|()| out.flush()) {
        Ok(()) => Ok(Wrote::Line),
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(Wrote::ReaderGone),
        Err(e) => Err(anyhow::Error::new(e).context("write to stdout")),
    }
}

/// One line of data on stdout.
pub fn print_line(text: &str) -> anyhow::Result<()> {
    write_line(&mut std::io::stdout().lock(), text)?;
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A writer that fails every write with the kind it was made with.
    struct Failing(std::io::ErrorKind);

    impl Write for Failing {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(self.0, "no"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_reader_that_went_away_ends_the_output_rather_than_failing() {
        let mut gone = Failing(std::io::ErrorKind::BrokenPipe);
        assert_eq!(write_line(&mut gone, "{}").unwrap(), Wrote::ReaderGone);
    }

    #[test]
    fn any_other_write_failure_is_still_a_failure() {
        let mut full = Failing(std::io::ErrorKind::StorageFull);
        let e = write_line(&mut full, "{}").unwrap_err();
        assert!(format!("{e:#}").contains("write to stdout"), "{e:#}");
    }

    #[test]
    fn a_line_that_was_written_says_so() {
        let mut out = Vec::new();
        assert_eq!(write_line(&mut out, "one").unwrap(), Wrote::Line);
        assert_eq!(String::from_utf8(out).unwrap(), "one\n");
    }
}
