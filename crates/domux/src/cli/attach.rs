//! The bare command and `attach`: start the server when needed, then attach.

use super::socket;
use domux_client::{attach, control, AttachOutcome};
use domux_core::names::BIN_NAME;
use std::ffi::OsString;

/// The bare command, which is how the attach is normally typed. Inside a pane it refuses: a
/// second whole screen drawn inside one pane of the screen it is drawing takes the keys from
/// the outer client (principle 1) and puts a second accent border on the screen (principle
/// 2). Nobody types it there wanting that; they type it out of habit.
///
/// `attach` typed in full still attaches, and every other subcommand is unaffected:
/// `DOMUX_SOCKET` is exported into every pane so that a shell there can reach its own server,
/// which is the whole reason the variable exists.
pub async fn run_bare() -> anyhow::Result<()> {
    let socket = socket();
    if inside_a_pane(std::env::var_os("DOMUX_PANE")) && control::is_live(&socket).await {
        return Err(nested_attach());
    }
    run().await
}

pub async fn run() -> anyhow::Result<()> {
    let socket = socket();
    if !control::is_live(&socket).await {
        // The attach that follows is the answer, so the start says nothing: telling the
        // reader to attach would name an action already underway.
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

/// Whether this shell is already inside a pane.
///
/// `DOMUX_PANE` and not `DOMUX_SOCKET`, though the server exports both. `DOMUX_SOCKET` is also
/// a documented override - it is how you point the CLI at a scratch server beside the real one,
/// and how the tests reach a harness - so a shell that exports it is not in a pane, and
/// refusing there would tell the reader they are somewhere they are not while leaving them no
/// way to attach at all. `DOMUX_PANE` carries a pane id and the server is the only thing that
/// sets it, so it means what this asks. An `ssh` to another machine carries neither.
fn inside_a_pane(pane_var: Option<OsString>) -> bool {
    pane_var.is_some_and(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_detach_and_a_stopped_server_are_endings_the_reader_is_told_about() {
        assert_eq!(
            ending(AttachOutcome::Detached("detached".into())).unwrap(),
            "Detached. Run domux2 to reattach."
        );
        assert_eq!(
            ending(AttachOutcome::ServerStopped).unwrap(),
            "The server stopped."
        );
    }

    #[test]
    fn a_lost_connection_and_a_refusal_are_failures_with_a_next_action() {
        let lost = ending(AttachOutcome::ConnectionLost)
            .unwrap_err()
            .to_string();
        assert_eq!(
            lost,
            "Lost the connection to the server. Run domux2 server status."
        );
        // The server's own sentence, which already names both versions and the way out.
        let refused = ending(AttachOutcome::Refused(
            "the server is domux 2.0.0 and this client is 1.9.0".into(),
        ))
        .unwrap_err()
        .to_string();
        assert_eq!(
            refused,
            "the server is domux 2.0.0 and this client is 1.9.0"
        );
    }

    #[test]
    fn a_shell_is_inside_a_pane_only_when_the_pane_variable_has_a_value() {
        assert!(!inside_a_pane(None));
        assert!(!inside_a_pane(Some(OsString::new())));
        assert!(inside_a_pane(Some(OsString::from("p_0001"))));
    }

    /// The variable this reads has to be one only the server sets. `DOMUX_SOCKET` is also the
    /// documented way to point the CLI at a scratch server beside the real one, so reading it
    /// here refused every attach for anyone who exports it - and told them they were inside a
    /// pane when they were not, with no way to attach at all.
    #[test]
    fn the_guard_reads_the_pane_variable_and_not_the_socket_override() {
        let source = include_str!("attach.rs");
        let call = source
            .lines()
            .find(|l| l.contains("inside_a_pane(std::env::var_os("))
            .expect("the guard's call site");
        assert!(call.contains("DOMUX_PANE"), "{call}");
        assert!(!call.contains("DOMUX_SOCKET"), "{call}");
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
