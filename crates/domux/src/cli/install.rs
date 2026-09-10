//! `install claude|codex|opencode [--apply]`.
//!
//! The one subcommand that is not an API call: installing hooks writes a file in the reader's
//! home directory and needs no server, so it runs here. Without `--apply` it prints what it
//! would do and writes nothing.

use super::print_line;
use clap::Args;
use domux_core::model::agent::AgentKind;
use domux_core::names::BIN_NAME;
use domux_server::agents::install::{apply, plan, preview};
use domux_server::agents::manifests::Registry;
use std::path::PathBuf;

#[derive(Args)]
pub struct InstallCmd {
    /// The agent to install the hooks for: claude, codex or opencode
    #[arg(value_parser = super::agent::parse_kind)]
    pub kind: AgentKind,
    /// Write the file. Without this, the command prints what it would do
    #[arg(long)]
    pub apply: bool,
}

pub fn run(cmd: InstallCmd) -> anyhow::Result<()> {
    let plan = plan(&Registry::builtin(), cmd.kind, &home()?, &binary_path()?)?;
    if !cmd.apply {
        // Line by line, so a reader who pipes it into `head` ends the output rather than
        // meeting a panic.
        for line in preview(&plan).lines() {
            print_line(line)?;
        }
        return Ok(());
    }
    // Said before anything is written, and it is the whole answer: a second run of an install
    // that is already there must not write another backup or claim it changed something.
    if plan.changes_nothing() {
        return print_line(&format!(
            "Nothing to change. The {} hooks are already installed at {}.",
            plan.kind,
            plan.path.display()
        ));
    }
    let backup = apply(&plan)?;
    let verb = if plan.before.is_some() {
        "Patched"
    } else {
        "Created"
    };
    print_line(&format!("{verb} {}.", plan.path.display()))?;
    // `apply` answers the file's own path when there was nothing to back up, which is a file
    // that was not there before.
    if backup != plan.path {
        print_line(&format!("The previous file is at {}.", backup.display()))?;
    }
    for note in &plan.notes {
        print_line(note)?;
    }
    Ok(())
}

/// Where the hooks go. Read from `HOME` rather than from a home directory crate, so a test can
/// point an install at a temp directory rather than at the reader's own files.
fn home() -> anyhow::Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| anyhow::anyhow!("HOME is not set, so there is no place to install hooks"))
}

/// The absolute path a hook runs, so it finds this binary whatever PATH the agent's environment
/// holds (M3 plan assumption 16). The symlink in `~/bin` wins when it is there, because it
/// survives a rebuild that moves the executable.
fn binary_path() -> anyhow::Result<PathBuf> {
    let linked = home()?.join("bin").join(BIN_NAME);
    if linked.exists() {
        return Ok(linked);
    }
    std::env::current_exe().map_err(|e| {
        anyhow::anyhow!(
            "cannot read this program's own path, so a hook would have no command to run: {e}"
        )
    })
}
