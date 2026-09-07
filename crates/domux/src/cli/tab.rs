//! `tab ...`: each subcommand is one tab.* call on the tab named by DOMUX_TAB.

use super::{call, location};
use clap::{Args, Subcommand};
use serde_json::json;

#[derive(Args)]
pub struct TabCmd {
    #[command(subcommand)]
    pub action: TabAction,
}

#[derive(Subcommand)]
pub enum TabAction {
    /// Create a tab in this workspace and select it
    Create,
    /// Name this tab
    Name { name: String },
    /// Give this tab its number back
    ClearName,
    /// Close this tab and its panes
    Close,
    /// Select a tab by number or id
    Select { tab: String },
}

pub async fn run(cmd: TabCmd) -> anyhow::Result<()> {
    // Absent from the environment means "the view's own tab", which is what the server
    // reads a null `tab` as, so a shell outside a pane acts on the most recent client.
    let tab = location::tab_from_env();
    match cmd.action {
        TabAction::Create => {
            call(
                "tab.create",
                json!({ "workspace": location::workspace_from_env() }),
            )
            .await?
        }
        TabAction::Name { name } => call("tab.rename", json!({ "tab": tab, "name": name })).await?,
        TabAction::ClearName => call("tab.clear_name", json!({ "tab": tab })).await?,
        TabAction::Close => call("tab.close", json!({ "tab": tab })).await?,
        TabAction::Select { tab } => call("tab.select", json!({ "tab": tab })).await?,
    };
    Ok(())
}
