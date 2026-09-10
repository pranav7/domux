//! `stay-awake on|off|toggle|status`, and `stay-awake install --full`.
//!
//! The first four are the API call the key makes, so a script and a keypress reach one
//! handler. The install is not a call: it writes two files under `/Library` and `/etc` and
//! needs no server, so it runs here, and it previews unless it is told to apply.

use super::{call_as, print_line, socket};
use clap::{Args, Subcommand};
use domux_client::control;
use domux_core::api::{ServerInfo, StayAwakeResult};
use domux_core::names::BIN_NAME;
use domux_server::command::RealRunner;
use domux_server::stay_awake::full_plan;

#[derive(Args)]
pub struct StayAwakeCmd {
    #[command(subcommand)]
    pub action: StayAwakeAction,
}

#[derive(Subcommand)]
pub enum StayAwakeAction {
    /// Hold this machine awake
    On,
    /// Let it sleep again
    Off,
    /// Hold it awake, or let it sleep, whichever it is not doing
    Toggle,
    /// Print whether this machine is being held awake
    Status,
    /// Set up full mode, which also stops the lid putting it to sleep
    Install(InstallArgs),
}

#[derive(Args)]
pub struct InstallArgs {
    /// Full mode: the two files under /Library and /etc that the lid needs on macOS
    #[arg(long)]
    pub full: bool,
    /// Write the files. Without this, the command prints what it would do
    #[arg(long)]
    pub apply: bool,
}

pub async fn run(cmd: StayAwakeCmd) -> anyhow::Result<()> {
    match cmd.action {
        StayAwakeAction::On => set("stay_awake.enable").await,
        StayAwakeAction::Off => set("stay_awake.disable").await,
        StayAwakeAction::Toggle => set("stay_awake.toggle").await,
        StayAwakeAction::Status => status().await,
        StayAwakeAction::Install(args) => install(args),
    }
}

/// The three that change something all print what the machine is doing afterwards, so the
/// command ends in the state and not in a claim about it (principle 8).
///
/// On stdout, and not silent the way `workspace rename` is: `toggle` answers a question the
/// caller cannot answer for itself, and the three read as one thing only if they all say
/// where they left the machine.
///
/// A note is a failure of what was asked for even when the hold itself is on, so it goes to
/// stderr and leaves a status of 1: a script that turns full mode on and gets partial mode
/// should be able to tell.
async fn set(method: &str) -> anyhow::Result<()> {
    let r: StayAwakeResult = call_as(method, serde_json::json!({})).await?;
    print_line(&label(r.on))?;
    match r.note {
        Some(note) => anyhow::bail!("{note}"),
        None => Ok(()),
    }
}

/// Reads the running server rather than starting one: asking whether a machine is held awake
/// must not be the thing that starts a server (principle 12).
async fn status() -> anyhow::Result<()> {
    if !control::is_live(&socket()).await {
        return print_line(&format!(
            "Stay awake: unknown, because no server is running. Start it with {BIN_NAME} server start."
        ));
    }
    let info: ServerInfo = call_as("server.info", serde_json::json!({})).await?;
    print_line(&label(info.stay_awake))
}

fn label(on: bool) -> String {
    if on {
        "Stay awake is on.".into()
    } else {
        "Stay awake is off.".into()
    }
}

fn install(args: InstallArgs) -> anyhow::Result<()> {
    if !args.full {
        anyhow::bail!(
            "There is nothing to install for the mode this runs in. Run {BIN_NAME} stay-awake install --full to set up the mode that also holds the lid."
        );
    }
    if std::env::consts::OS != "macos" {
        anyhow::bail!(
            "Full mode needs no files on this system. Set mode = \"full\" under [stay_awake] in the config file and the hold covers the lid too."
        );
    }
    let plan = full_plan(&std::env::var("USER").unwrap_or_else(|_| "root".into()));
    if !args.apply {
        // Line by line, so a reader who pipes it into `head` ends the output rather than
        // meeting a panic.
        for line in plan.preview().lines() {
            print_line(line)?;
        }
        return Ok(());
    }
    plan.apply(&RealRunner::default())
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    for w in &plan.writes {
        print_line(&format!("Wrote {}.", w.path))?;
    }
    print_line(&format!(
        "Now set mode = \"full\" under [stay_awake] in the config file, then run {BIN_NAME} config reload."
    ))
}
