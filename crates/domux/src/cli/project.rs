//! `project ...`: each subcommand is one project.* call.

use super::{call, call_that_asks, print_line};
use clap::{Args, Subcommand};
use serde_json::json;

#[derive(Args)]
pub struct ProjectCmd {
    #[command(subcommand)]
    pub action: ProjectAction,
}

#[derive(Subcommand)]
pub enum ProjectAction {
    /// Register a path and adopt the worktrees beside it
    Add {
        /// The folder to register. A git repository becomes a git project; anything else
        /// becomes a folder project, which has main and no slots
        path: String,
    },
    /// Let a project go. The folder and its worktrees stay on disk
    Remove {
        /// A project id or name. Leave it out with --all
        project: Option<String>,
        /// Remove it without asking
        #[arg(long)]
        yes: bool,
        /// Let every project go, for starting over
        #[arg(long, conflicts_with = "project")]
        all: bool,
    },
    /// List the projects as JSON
    List,
}

pub async fn run(cmd: ProjectCmd) -> anyhow::Result<()> {
    match cmd.action {
        // The record it made, on stdout. A create that said nothing would leave the caller
        // with no way to name what it had just made, and the id is in the answer already.
        ProjectAction::Add { path } => {
            let added = call("project.add", json!({ "path": path })).await?;
            print_line(&serde_json::to_string_pretty(&added)?)?;
        }
        // It asks first, and the answer to a removal that went through is the project no
        // longer being in `project list`.
        ProjectAction::Remove { project, yes, all } => {
            call_that_asks(
                "project.remove",
                json!({ "project": project, "yes": yes, "all": all }),
            )
            .await?;
        }
        ProjectAction::List => {
            let projects = call("project.list", json!({})).await?;
            print_line(&serde_json::to_string_pretty(&projects)?)?;
        }
    }
    Ok(())
}
