//! `install claude|codex|opencode [--apply]`.
//!
//! The one subcommand that is not an API call: installing hooks writes a file in the reader's
//! home directory and needs no server, so it runs here. Without `--apply` it prints what it
//! would do and writes nothing.

use super::print_line;
use clap::Args;
use domux_core::model::agent::AgentKind;
use domux_core::names::BIN_NAME;
use domux_server::agents::install::{apply, hook_binary, plan, preview};
use domux_server::agents::manifests::{HookTarget, Registry};
use std::path::{Path, PathBuf};

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
    let target = &registry
        .for_kind(cmd.kind)
        .ok_or_else(|| anyhow::anyhow!("no manifest for {}", cmd.kind))?
        .hooks;
    let dir = config_dir(target, cmd.dir.clone())?;
    let linked = home()?.join("bin").join(BIN_NAME);
    let bin = hook_binary(&linked, &running_binary()?);
    let mut plan = plan(&registry, cmd.kind, &dir, &bin)?;
    // A reader who has something at the `~/bin` path expects the install to write it, and it is
    // most likely V1 or a link that a `cargo clean` broke. `symlink_metadata` rather than
    // `exists`, so a broken link counts as something. First among the notes, so an apply says it
    // straight after the line that names the binary. It names neither hooks nor a plugin, so it
    // is true for every kind.
    if bin != linked && linked.symlink_metadata().is_ok() {
        plan.notes.insert(
            0,
            format!(
                "{} is not this binary, so the install passes over it.",
                linked.display()
            ),
        );
    }
    if !cmd.apply {
        // Line by line, so a reader who pipes it into `head` ends the output rather than
        // meeting a panic.
        for line in preview(&plan).lines() {
            print_line(line)?;
        }
        return Ok(());
    }
    // Said before anything is written: a second run of an install that is already there must not
    // write another backup or claim it changed something. It still names the binary, because
    // hooks that run the wrong binary look installed too.
    if plan.changes_nothing() {
        print_line(&format!(
            "Nothing to change. The {} hooks are already installed at {}.",
            plan.kind,
            plan.path.display()
        ))?;
        return print_line(&runs(target, &bin));
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
    print_line(&runs(target, &bin))?;
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
fn config_dir(target: &HookTarget, asked: Option<PathBuf>) -> anyhow::Result<PathBuf> {
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

/// The line that names the binary the hooks run, or the plugin for OpenCode, as the manifest
/// names what it writes. A file whose hooks run the wrong binary looks installed from every other
/// line an install prints, so an apply says it whether or not it wrote anything.
fn runs(target: &HookTarget, bin: &Path) -> String {
    format!("{} {}.", target.runs(), bin.display())
}

/// The binary doing the install. A hook runs it by its absolute path, so it finds this binary
/// whatever PATH the agent's environment holds, unless the `~/bin` path is the same file
/// (`agents::install::hook_binary`, decision record 0040).
fn running_binary() -> anyhow::Result<PathBuf> {
    std::env::current_exe().map_err(|e| {
        anyhow::anyhow!(
            "cannot read this binary's own path, so a hook would have no command to run: {e}"
        )
    })
}
