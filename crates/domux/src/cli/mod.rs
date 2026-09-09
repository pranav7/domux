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
pub mod open;
pub mod pane;
pub mod project;
pub mod server;
pub mod tab;
pub mod workspace;

use anyhow::Context;
use domux_client::control;
use domux_core::api::{ApiError, ErrorCode};
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
    answer(method, params).await?.map_err(api_error)
}

/// The server's own answer to one call: the result, or the error as the server wrote it.
///
/// `call` turns that error into a line for stderr, and loses `data` doing it. A caller that
/// needs the structured refusal reads it here instead.
async fn answer(method: &str, params: Value) -> anyhow::Result<Result<Value, ApiError>> {
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
    Ok(answer)
}

/// Calls a method that asks before it acts, and lays its question out for a reader who has
/// no screen to draw it on (principle 10).
///
/// A destructive call with no consent is refused with the whole consequence: the question in
/// `message`, and the same content in `data` as a list of what goes and a list of what stays.
/// A shell reader gets the lists, one item to a line under its label, which is the shape the
/// confirmation overlay draws, so a reader who has seen one and then the other is not told
/// two different things.
///
/// Only a subcommand that has a `--yes` flag may use this, because the last line names that
/// flag. Every other refusal still prints as one line.
pub async fn call_that_asks(method: &str, params: Value) -> anyhow::Result<Value> {
    match answer(method, params).await? {
        Ok(v) => Ok(v),
        Err(e) => Err(match question(&e) {
            Some(text) => anyhow::anyhow!(text),
            None => api_error(e),
        }),
    }
}

/// The question a refusal is asking, laid out, or `None` when the refusal is not a question.
///
/// Written from `data` rather than from `message`, because `message` ends in the server's own
/// "Answer with --yes", which names a parameter rather than the flag a shell reader has. The
/// question itself is `confirmation`, the same sentence without that tail.
fn question(err: &ApiError) -> Option<String> {
    if err.code != ErrorCode::Refused {
        return None;
    }
    // `ApiError::ambiguous` puts a bare array in `data`, so this asks whatever the refusal
    // carried rather than assuming it carried an object.
    let data = err.data.as_ref()?;
    let mut text = data.get("confirmation")?.as_str()?.to_string();
    for (label, key) in [("Removes:", "removes"), ("Keeps:", "keeps")] {
        let items: Vec<&str> = data
            .get(key)
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        // A label with nothing under it would read as "this removes nothing", which is a
        // fact the server did not report (principle 4).
        if items.is_empty() {
            continue;
        }
        text.push('\n');
        text.push_str(label);
        for item in items {
            text.push_str("\n  ");
            text.push_str(item);
        }
    }
    // On the same stream as the question, because a reader who redirects one and not the
    // other must not lose the half that says how to answer it.
    text.push_str("\nRun it again with --yes.");
    Some(text)
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
