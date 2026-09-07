//! `config reload`

use super::call_as;
use clap::{Args, Subcommand};
use domux_core::api::ConfigReloadResult;

#[derive(Args)]
pub struct ConfigCmd {
    #[command(subcommand)]
    pub action: ConfigAction,
}

#[derive(Subcommand)]
pub enum ConfigAction {
    /// Reread domux.toml; a bad file keeps the previous config and reports the line
    Reload,
}

pub async fn run(cmd: ConfigCmd) -> anyhow::Result<()> {
    match cmd.action {
        ConfigAction::Reload => {
            let r: ConfigReloadResult = call_as("config.reload", serde_json::json!({})).await?;
            for w in &r.warnings {
                eprintln!("warning: {w}");
            }
            // A file that did not load is a failure of the reload, so it leaves a status
            // of 1 rather than a cheerful line over a config that never applied.
            if let Some(e) = r.error {
                anyhow::bail!("{e}");
            }
            eprintln!("Config reloaded.");
            Ok(())
        }
    }
}
