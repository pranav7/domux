//! `workspace ...`: each subcommand is one workspace.* call, on the workspace
//! `DOMUX_WORKSPACE` names or on the one you name.

use super::{call, call_that_asks, location, print_line};
use clap::{Args, Subcommand};
use serde_json::json;

#[derive(Args)]
pub struct WorkspaceCmd {
    #[command(subcommand)]
    pub action: WorkspaceAction,
}

#[derive(Subcommand)]
pub enum WorkspaceAction {
    /// Create a workspace in this project at the lowest free number
    Create {
        /// A project id or name; the default is the project this shell is in
        #[arg(long)]
        project: Option<String>,
        /// The ref to branch from; the default comes from [worktrees] base or origin/HEAD
        #[arg(long)]
        base: Option<String>,
    },
    /// Name this workspace; an empty name clears it
    Name { name: String },
    /// Give this workspace its handle back
    ClearName,
    /// Reset this workspace's branch to the base and clean its tree
    Clear {
        /// A workspace id, handle, name or branch; the default is this workspace
        workspace: Option<String>,
        /// Clear it without asking, throwing away anything uncommitted or unpushed
        #[arg(long)]
        yes: bool,
    },
    /// Delete a workspace: its worktree, its branch, and its slot
    Delete {
        /// A workspace id, handle, name or branch
        workspace: String,
        /// Delete it without asking
        #[arg(long)]
        yes: bool,
        /// Delete it even with uncommitted or unpushed changes
        #[arg(long)]
        force: bool,
    },
    /// List the workspaces as JSON
    List {
        /// A project id or name; the default is every project
        #[arg(long)]
        project: Option<String>,
    },
}

pub async fn run(cmd: WorkspaceCmd) -> anyhow::Result<()> {
    // Absent from the environment means "the view's own workspace", which is what the server
    // reads a null `workspace` as, so a shell outside a pane acts on the most recent client.
    let here = location::workspace_from_env();
    match cmd.action {
        // The slot it made, on stdout: its number, its branch, the ref it came from and what
        // worktree.conf did are all facts of this call that no later call reports.
        WorkspaceAction::Create { project, base } => {
            let made = call(
                "workspace.create",
                json!({ "project": project, "base": base }),
            )
            .await?;
            print_line(&serde_json::to_string_pretty(&made)?)?;
        }
        // An empty argument is a clear, the rule `tab name` follows. No argument at all is a
        // different request, "ask me for one here", and a shell has nowhere to draw the name
        // box, so this subcommand always sends a name.
        WorkspaceAction::Name { name } => {
            call(
                "workspace.rename",
                json!({ "workspace": here, "name": name }),
            )
            .await?;
        }
        WorkspaceAction::ClearName => {
            call("workspace.clear_name", json!({ "workspace": here })).await?;
        }
        // Plain `call`, not `call_that_asks`: a clear does not ask a shell. Whether there is
        // anything to lose is a git question, so only the job can answer it, and it refuses
        // in words that name `--yes` themselves.
        WorkspaceAction::Clear { workspace, yes } => {
            call(
                "workspace.clear",
                json!({ "workspace": workspace.or(here), "yes": yes }),
            )
            .await?;
        }
        // Named, never taken from the environment: a delete removes a worktree and a branch,
        // and a subcommand that could act on wherever the shell happened to be is one typo
        // from removing the wrong one.
        WorkspaceAction::Delete {
            workspace,
            yes,
            force,
        } => {
            call_that_asks(
                "workspace.delete",
                json!({ "workspace": workspace, "yes": yes, "force": force }),
            )
            .await?;
        }
        WorkspaceAction::List { project } => {
            let rows = call("workspace.list", json!({ "project": project })).await?;
            print_line(&serde_json::to_string_pretty(&rows)?)?;
        }
    }
    Ok(())
}
