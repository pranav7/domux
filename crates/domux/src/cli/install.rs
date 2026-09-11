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
    /// The agent's configuration directory, when it is not the one under your home directory
    #[arg(long)]
    pub dir: Option<PathBuf>,
}

pub fn run(cmd: InstallCmd) -> anyhow::Result<()> {
    let registry = Registry::builtin();
    let dir = config_dir(&registry, cmd.kind, cmd.dir.clone())?;
    let plan = plan(&registry, cmd.kind, &dir, &binary_path()?)?;
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

/// Which directory the hooks go in: the one the reader asked for, then the one the agent's own
/// variable names, then the agent's directory under home (decision record 0036).
///
/// The variable is read here rather than in `agents::install`, the way `home` is: the installer
/// stays a function of the paths it is given, and the environment is read once, at the edge.
fn config_dir(
    registry: &Registry,
    kind: AgentKind,
    asked: Option<PathBuf>,
) -> anyhow::Result<PathBuf> {
    let target = &registry
        .for_kind(kind)
        .ok_or_else(|| anyhow::anyhow!("no manifest for {kind}"))?
        .hooks;
    if let Some(dir) = asked {
        return Ok(dir);
    }
    let named = target
        .dir_env()
        .and_then(std::env::var_os)
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty());
    match named {
        Some(dir) => Ok(dir),
        None => Ok(target.dir_in(&home()?)),
    }
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
