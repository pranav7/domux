//! The bare command and `attach`: start the server when needed, then attach.

use super::{call, call_as, socket};
use domux_client::{attach, control, AttachOutcome};
use domux_core::api::{ProjectAdded, ProjectInfo, WorkspaceInfo};
use domux_core::names::BIN_NAME;
use serde_json::json;
use std::ffi::OsString;
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

/// The bare command, which is how the attach is normally typed. Inside a pane it refuses: a
/// second whole screen drawn inside one pane of the screen it is drawing takes the keys from
/// the outer client (principle 1) and puts a second accent border on the screen (principle
/// 2). Nobody types it there wanting that; they type it out of habit.
///
/// `attach` typed in full still attaches, and that is deliberate. The bare command is the
/// habit; naming the verb is a choice. It is also the only way to reach a different server from
/// inside a pane: `DOMUX_SOCKET` set by hand points at that server, but `DOMUX_PANE` is still
/// the outer pane's, so a guard on both forms would refuse a thing worth doing.
///
/// Every other subcommand is unaffected. `DOMUX_SOCKET` is exported into every pane so a shell
/// there can reach its own server, which is the whole reason the variable exists.
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
        //
        // The server this starts seeds the directory it was started in, so the offer below
        // has nothing to offer: the question is only ever asked of a server that was already
        // running somewhere else.
        super::server::start(super::server::Announce::No).await?;
    }
    offer_to_register_here().await?;
    // `attach` returns with the terminal already restored, so the line below lands on a
    // terminal the reader can type into again (principle 11).
    eprintln!("{}", ending(attach(&socket).await?)?);
    Ok(())
}

/// Asks, once, whether the directory this was typed in should become a project, and registers
/// it when the answer is yes.
///
/// Decision record 0006 settled that attach reconnects to what is registered rather than
/// following the shell, and that still holds: this does not move the client to the directory
/// on its own. What it adds is the offer, because the old behaviour had no way to say yes.
/// Typing the bare command somewhere new landed you in another project's tabs with nothing
/// on the screen about the directory you were standing in, and the only way out was to know
/// that `open` existed.
///
/// It asks rather than registering, so a bare command typed in a scratch directory, a
/// downloads folder or somebody else's checkout does not quietly leave a project behind.
///
/// Everything here is best effort. A directory that cannot be read, a server that will not
/// answer `project.list`, a reader who is not on a terminal: each one skips the offer and
/// attaches, because the attach is what was asked for and the offer is an extra.
async fn offer_to_register_here() -> anyhow::Result<()> {
    let Ok(cwd) = std::env::current_dir() else {
        return Ok(());
    };
    // Not a terminal means nothing to ask and nobody to answer. A script that pipes into
    // the client must not stop on a question it cannot see.
    if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
        return Ok(());
    }
    let projects: Vec<ProjectInfo> = call_as("project.list", json!({})).await?;
    let workspaces: Vec<WorkspaceInfo> = call_as("workspace.list", json!({})).await?;
    let known: Vec<PathBuf> = projects
        .iter()
        .map(|p| p.root.clone())
        .chain(workspaces.iter().map(|w| w.path.clone()))
        .collect();
    if is_inside_any(&cwd, &known) {
        return Ok(());
    }
    if !answered_yes(&question(&cwd))? {
        eprintln!("Left unregistered. Run {BIN_NAME} open . to register it later.");
        return Ok(());
    }
    // The same two calls `open` makes, in the same order and for the same reason: the
    // registration is true whether or not the switch works, so it is said first.
    let added: ProjectAdded = call_as("project.add", json!({ "path": cwd })).await?;
    if !added.adopted.is_empty() {
        eprintln!("Adopted {}.", added.adopted.join(", "));
    }
    call("workspace.focus", json!({ "workspace": added.workspace })).await?;
    Ok(())
}

/// The question, as the state, the object and the next action (principle 9).
fn question(cwd: &Path) -> String {
    format!(
        "{} is not a project yet. Register it? [y/N] ",
        cwd.display()
    )
}

/// Whether `dir` is one of `known` or lives under one of them.
///
/// Both sides are resolved, because the server answers with the path it canonicalized and
/// this reads the path the shell is standing in: on a Mac `/tmp/x` here is `/private/tmp/x`
/// there, and comparing them as written would offer to register a directory that is already
/// a project. A path that will not resolve is compared as it stands.
fn is_inside_any(dir: &Path, known: &[PathBuf]) -> bool {
    let dir = resolved(dir);
    known.iter().any(|k| dir.starts_with(resolved(k)))
}

fn resolved(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Reads one line and answers whether it said yes. Anything else, end of input included, is
/// no: the default in the prompt is the answer that changes nothing (principle 10).
fn answered_yes(question: &str) -> anyhow::Result<bool> {
    let mut err = std::io::stderr();
    write!(err, "{question}")?;
    err.flush()?;
    let mut line = String::new();
    if std::io::stdin().lock().read_line(&mut line)? == 0 {
        eprintln!();
        return Ok(false);
    }
    Ok(says_yes(&line))
}

/// Whether one typed line said yes.
fn says_yes(line: &str) -> bool {
    matches!(line.trim().to_lowercase().as_str(), "y" | "yes")
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
    fn only_yes_registers_and_everything_else_leaves_the_directory_alone() {
        assert!(says_yes("y"));
        assert!(says_yes("Y\n"));
        assert!(says_yes("  yes  "));
        assert!(!says_yes(""));
        assert!(!says_yes("n"));
        assert!(!says_yes("yeah"));
        assert!(!says_yes("yes please"));
    }

    #[test]
    fn a_directory_under_a_registered_path_is_already_at_home() {
        let known = vec![PathBuf::from("/repo/audrey-app"), PathBuf::from("/notes")];
        assert!(is_inside_any(Path::new("/repo/audrey-app"), &known));
        assert!(is_inside_any(
            Path::new("/repo/audrey-app/crates/api"),
            &known
        ));
        assert!(is_inside_any(Path::new("/notes"), &known));
    }

    /// Component by component, not character by character: `/repo/audrey-app-2` is a
    /// different directory from `/repo/audrey-app` and must still be offered.
    #[test]
    fn a_directory_whose_name_merely_starts_the_same_is_not_at_home() {
        let known = vec![PathBuf::from("/repo/audrey-app")];
        assert!(!is_inside_any(Path::new("/repo/audrey-app-2"), &known));
        assert!(!is_inside_any(Path::new("/repo"), &known));
        assert!(!is_inside_any(Path::new("/elsewhere"), &known));
    }

    #[test]
    fn nothing_registered_means_every_directory_is_offered() {
        assert!(!is_inside_any(Path::new("/repo/audrey-app"), &[]));
    }

    #[test]
    fn the_question_names_the_directory_and_defaults_to_leaving_it_alone() {
        let said = question(Path::new("/repo/audrey-app"));
        assert_eq!(
            said,
            "/repo/audrey-app is not a project yet. Register it? [y/N] "
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
