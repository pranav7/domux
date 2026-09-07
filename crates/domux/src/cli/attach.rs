//! The bare command and `attach`: start the server when needed, then attach.

use super::socket;
use domux_client::{attach, control, AttachOutcome};
use domux_core::names::BIN_NAME;
use std::ffi::OsString;

/// Bare `domux2`, which is how the attach is normally typed. Inside a pane it refuses: a
/// second whole screen drawn inside one pane of the screen it is drawing takes the keys from
/// the outer client (principle 1) and puts a second accent border on the screen (principle
/// 2). Nobody types it there wanting that; they type it out of habit.
///
/// `domux2 attach` typed in full still attaches, and every other subcommand is unaffected:
/// `DOMUX_SOCKET` is exported into every pane so that a shell there can reach its own server,
/// which is the whole reason the variable exists.
pub async fn run_bare() -> anyhow::Result<()> {
    let socket = socket();
    if inside_a_pane(std::env::var_os("DOMUX_SOCKET")) && control::is_live(&socket).await {
        return Err(nested_attach());
    }
    run().await
}

pub async fn run() -> anyhow::Result<()> {
    let socket = socket();
    if !control::is_live(&socket).await {
        // The attach that follows is the answer, so the start says nothing: "Attach with
        // domux2" would name an action already underway.
        super::server::start(super::server::Announce::No).await?;
    }
    // `attach` returns with the terminal already restored, so the line below lands on a
    // terminal the reader can type into again (principle 11).
    eprintln!("{}", ending(attach(&socket).await?)?);
    Ok(())
}

/// What the reader is told when the attach ends. The two outcomes that are not a failure
/// answer with the line to print, the two that are answer with the error to report, so a
/// refused attach cannot end in silence with a status of 0.
fn ending(outcome: AttachOutcome) -> anyhow::Result<String> {
    match outcome {
        AttachOutcome::Detached(_) => Ok(format!("Detached. Run {BIN_NAME} to reattach.")),
        AttachOutcome::ServerStopped => Ok("The server stopped.".to_string()),
        AttachOutcome::ConnectionLost => Err(anyhow::anyhow!(
            "Lost the connection to the server. Run {BIN_NAME} server status."
        )),
        AttachOutcome::Refused(reason) => Err(anyhow::anyhow!("{reason}")),
    }
}

/// The state, the object and the next action (principle 9), in the shape the tmux refusal in
/// `domux-client` already uses.
fn nested_attach() -> anyhow::Error {
    anyhow::anyhow!(
        "{BIN_NAME} is already running in this terminal. Its panes are this one's screen; run {BIN_NAME} tab create or another subcommand here, or open a new terminal to attach a second view."
    )
}

/// Whether this shell is already inside a pane. The server exports `DOMUX_SOCKET` into every
/// pane, so a value means the terminal we would draw on is itself a pane. An `ssh` into
/// another machine does not carry the variable, so a real second server is not caught by it.
fn inside_a_pane(socket_var: Option<OsString>) -> bool {
    socket_var.is_some_and(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shell_is_inside_a_pane_only_when_the_socket_variable_has_a_value() {
        assert!(!inside_a_pane(None));
        assert!(!inside_a_pane(Some(OsString::new())));
        assert!(inside_a_pane(Some(OsString::from(
            "/tmp/domux2-501/domux2.sock"
        ))));
    }

    #[test]
    fn the_nested_attach_refusal_names_the_state_the_object_and_the_next_action() {
        let said = nested_attach().to_string();
        assert!(
            said.starts_with("domux2 is already running in this terminal."),
            "{said}"
        );
        assert!(said.contains("domux2 tab create"), "{said}");
        assert!(said.contains("open a new terminal"), "{said}");
    }
}
