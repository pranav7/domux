//! `open <path>`: the path becomes a project and this client switches to it.
//!
//! Two calls, because that is two operations: `project.add` registers the path, and
//! `workspace.focus` moves the client onto the project's main. Every other M2 subcommand is
//! one call; this one is the domain model's Open row, which is both halves.

use super::{call, call_as};
use clap::Args;
use domux_core::api::ProjectAdded;
use serde_json::json;

#[derive(Args)]
pub struct OpenCmd {
    /// The folder to open. A git repository becomes a git project; anything else becomes a
    /// folder project, which has main and no slots
    pub path: String,
}

pub async fn run(cmd: OpenCmd) -> anyhow::Result<()> {
    let added: ProjectAdded = call_as("project.add", json!({ "path": cmd.path })).await?;
    // Said as soon as it is true, and before the switch, because it stays true whether or
    // not the switch works: the worktrees are registered either way, and a reader whose
    // switch failed still has slots they did not ask for.
    //
    // It does not say "added audrey-app". `project.add` answers a path that is already a
    // project exactly as it answers a new one, so this cannot tell a registration from a
    // path that was already there, and a command that has not done something must not say
    // it did. What it can say is what was adopted, which is empty in both the quiet cases.
    if !added.adopted.is_empty() {
        eprintln!("Adopted {}.", added.adopted.join(", "));
    }
    call("workspace.focus", json!({ "workspace": added.workspace })).await?;
    Ok(())
}
