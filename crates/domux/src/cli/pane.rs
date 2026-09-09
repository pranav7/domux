//! `pane ...`: one pane.* call on the pane named by DOMUX_PANE.

use super::{call, call_as, location, print_line};
use clap::{Args, Subcommand};
use domux_core::api::PaneReadResult;
use serde_json::json;

#[derive(Args)]
pub struct PaneCmd {
    #[command(subcommand)]
    pub action: PaneAction,
}

#[derive(Subcommand)]
pub enum PaneAction {
    /// Split this pane: left, right, up or down
    Split { dir: String },
    /// Close this pane
    Close,
    /// Focus a pane by id (default: this pane)
    Focus { pane: Option<String> },
    /// Zoom this pane, or restore the layout
    Zoom,
    /// Print the last lines of this pane, scrollback included
    Read {
        #[arg(long)]
        lines: Option<usize>,
    },
    /// Empty this pane: its screen and its scrollback
    Clear,
    /// Type text into this pane
    SendText { text: String },
    /// Send one key by name: Enter, C-c, S-Left
    SendKey { key: String },
}

pub async fn run(cmd: PaneCmd) -> anyhow::Result<()> {
    // Absent from the environment means "the view's focused pane", which is what the
    // server reads a null `pane` as.
    let pane = location::pane_from_env();
    match cmd.action {
        PaneAction::Split { dir } => {
            call("pane.split", json!({ "pane": pane, "dir": dir })).await?;
        }
        PaneAction::Close => {
            call("pane.close", json!({ "pane": pane })).await?;
        }
        PaneAction::Focus { pane: target } => {
            call("pane.focus", json!({ "pane": target.or(pane) })).await?;
        }
        PaneAction::Zoom => {
            call("pane.zoom", json!({ "pane": pane })).await?;
        }
        PaneAction::Read { lines } => {
            let r: PaneReadResult =
                call_as("pane.read", json!({ "pane": pane, "lines": lines })).await?;
            // An empty answer and one blank line are not the same fact, so a pane with
            // nothing on it prints nothing.
            if !r.text.is_empty() {
                print_line(&r.text)?;
            }
        }
        PaneAction::Clear => {
            call("pane.clear", json!({ "pane": pane })).await?;
        }
        PaneAction::SendText { text } => {
            call("pane.send_text", json!({ "pane": pane, "text": text })).await?;
        }
        PaneAction::SendKey { key } => {
            call("pane.send_key", json!({ "pane": pane, "key": key })).await?;
        }
    }
    Ok(())
}
