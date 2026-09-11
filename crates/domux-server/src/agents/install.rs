//! Hook installers. `install <kind>` prints what it would do and writes nothing; `--apply`
//! writes it, keeping the previous file as a backup, and a second run changes nothing
//! (architecture spec 3.4). An apply also removes the hook lines V1 wrote, because the
//! subcommands they call do not exist after the cut-over and a hook that fails interrupts the
//! agent's session.
//!
//! Claude and Codex share one file shape: a `hooks` object keyed by event name, each value a
//! list of entries, each entry an optional `matcher` and a list of
//! `{"type": "command", "command": "..."}`. OpenCode has no such file, so its manifest names a
//! plugin, and this module generates it.

use crate::agents::manifests::{HookTarget, Registry};
use domux_core::model::agent::AgentKind;
use serde_json::{json, Map, Value};
use std::fmt;
use std::path::{Path, PathBuf};

/// Why an install could not be planned or written. Each message names the file, so the reader
/// knows which one to look at.
#[derive(Debug)]
pub enum InstallError {
    Read { path: PathBuf, message: String },
    Write { path: PathBuf, message: String },
    NoManifest(AgentKind),
}

impl fmt::Display for InstallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InstallError::Read { path, message } => {
                write!(f, "cannot read {}: {message}", path.display())
            }
            InstallError::Write { path, message } => {
                write!(f, "cannot write {}: {message}", path.display())
            }
            InstallError::NoManifest(kind) => write!(f, "no manifest for {kind}"),
        }
    }
}

impl std::error::Error for InstallError {}

/// What an install would do. `preview` prints it; `apply` performs it.
#[derive(Debug, Clone)]
pub struct Plan {
    pub kind: AgentKind,
    pub path: PathBuf,
    /// The file as it is, when it is there.
    pub before: Option<String>,
    /// The file as it would be.
    pub after: String,
    /// `(event, command)` pairs this install adds.
    pub added: Vec<(String, String)>,
    /// `(event, command)` pairs it removes, V1's lines included.
    pub removed: Vec<(String, String)>,
    /// Anything else worth saying, one line each.
    pub notes: Vec<String>,
}

impl Plan {
    /// True when the file already says what this install would say. `preview` says so and
    /// `apply` writes nothing, so the two never disagree about whether a run did anything.
    pub fn changes_nothing(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }
}

/// The command every installed hook runs: the binary's absolute path, so a hook that runs in
/// the agent's environment finds it whatever that environment's PATH holds (M3 plan assumption
/// 16). The kind selects both the input adapter and the SessionStart output format.
pub fn hook_command(bin: &Path, kind: AgentKind) -> String {
    format!(
        "{} agent report --agent {kind}",
        shell_command_path(&bin.to_string_lossy())
    )
}

/// A path a shell reads as one word is written bare; anything else is single quoted (V1's
/// `shellCommandPath`, commit 34db116).
///
/// Deliberately not `resume::shell_quote`, which quotes every value it is given, and named
/// apart from it so the two cannot be mistaken for one rule. They are two functions in V1 too,
/// and the difference is load-bearing here: this path is written into `settings.json` and
/// `hooks.json`, files a person reads and edits by hand, so a plain path stays plain and only
/// one that would not survive the shell gains quotes.
/// `a_binary_path_that_needs_quoting_is_quoted_for_the_shell` pins both halves.
fn shell_command_path(path: &str) -> String {
    if path.contains(|c: char| " \t\n'\"\\$`!*?[]{}()<>|&;".contains(c)) {
        format!("'{}'", path.replace('\'', "'\\''"))
    } else {
        path.to_string()
    }
}

/// A hook line V1 wrote, its tmux dotfile bridge included. These go: V1's `ai-state` and
/// `workspace` subcommands are gone after the cut-over, so the line would fail on every event.
///
/// The strings are V1's own literals (V1's `isCopiedClaudeCodexHook`, commit 34db116). They
/// name what is written in the author's file today, so they must not follow any later rename.
pub fn is_v1_line(command: &str) -> bool {
    command.contains("domux ai-state")
        || command.contains("ai-state --agent")
        || command.contains("domux workspace occupied")
        || command.contains("domux workspace free")
        || command.contains(".tmux-claude-")
        || command.contains(".tmux-workspace-")
}

/// A line an earlier install wrote, under either binary name. These are replaced rather than
/// left, so the cut-over's rename rewrites them. M4's `doctor` asks the same question of a
/// hook file it did not write.
pub fn is_v2_line(command: &str) -> bool {
    command.contains("agent report --agent")
}

/// Reads the file and works out what installing would change. Writes nothing.
///
/// `dir` is the kind's configuration directory, which the caller resolves: the kind's own
/// directory under home, the one its variable names, or the one the reader asked for (decision
/// record 0036). Nothing here reads the environment, so a test names the directory it means.
pub fn plan(
    registry: &Registry,
    kind: AgentKind,
    dir: &Path,
    bin: &Path,
) -> Result<Plan, InstallError> {
    let manifest = registry
        .for_kind(kind)
        .ok_or(InstallError::NoManifest(kind))?;
    let path = manifest.hooks.path_under(dir);
    let before = match std::fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            return Err(InstallError::Read {
                path,
                message: e.to_string(),
            })
        }
    };
    match manifest.hooks {
        HookTarget::OpencodePlugin => {
            let after = opencode_plugin(bin);
            let added = if before.as_deref() == Some(after.as_str()) {
                Vec::new()
            } else {
                vec![("plugin".to_string(), hook_command(bin, kind))]
            };
            Ok(Plan {
                kind,
                path,
                before,
                after,
                added,
                removed: Vec::new(),
                notes: vec!["Restart opencode for the plugin to load.".to_string()],
            })
        }
        HookTarget::ClaudeSettings | HookTarget::CodexHooks => {
            let mut root = match &before {
                Some(text) if !text.trim().is_empty() => {
                    serde_json::from_str(text).map_err(|e| InstallError::Read {
                        path: path.clone(),
                        message: e.to_string(),
                    })?
                }
                _ => Map::new(),
            };
            check_shape(&root, &path)?;
            let was = hook_lines(&root);
            let status_line = remove_v1_status_line(&mut root);
            patch_hooks(&mut root, manifest.hooks.events(), &hook_command(bin, kind));
            let is = hook_lines(&root);

            let mut removed = difference(&was, &is);
            let added = difference(&is, &was);
            let mut notes = Vec::new();
            if let Some(command) = status_line {
                removed.push(("statusLine".to_string(), command));
                notes.push(
                    "V1's status line runs a subcommand this version does not have, so it goes."
                        .to_string(),
                );
            }
            let after = serde_json::to_string_pretty(&Value::Object(root)).map_err(|e| {
                InstallError::Write {
                    path: path.clone(),
                    message: e.to_string(),
                }
            })?;
            Ok(Plan {
                kind,
                path,
                before,
                after: format!("{after}\n"),
                added,
                removed,
                notes,
            })
        }
    }
}

/// Refuses a file whose `hooks` is not the shape the installer patches, rather than quietly
/// leaving it alone. Nothing else in this module has to check again.
fn check_shape(root: &Map<String, Value>, path: &Path) -> Result<(), InstallError> {
    let Some(hooks) = root.get("hooks") else {
        return Ok(());
    };
    let Some(hooks) = hooks.as_object() else {
        return Err(InstallError::Read {
            path: path.to_path_buf(),
            message: "the hooks value is not a JSON object".to_string(),
        });
    };
    for (event, entries) in hooks {
        if !entries.is_array() {
            return Err(InstallError::Read {
                path: path.to_path_buf(),
                message: format!("the hooks value for {event} is not a list"),
            });
        }
    }
    Ok(())
}

/// Every `(event, command)` pair the file installs, in file order. An entry the installer does
/// not understand contributes nothing and is left where it is.
fn hook_lines(root: &Map<String, Value>) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Some(hooks) = root.get("hooks").and_then(Value::as_object) else {
        return out;
    };
    for (event, entries) in hooks {
        let Some(entries) = entries.as_array() else {
            continue;
        };
        for entry in entries {
            let Some(list) = entry.get("hooks").and_then(Value::as_array) else {
                continue;
            };
            for hook in list {
                if let Some(command) = hook.get("command").and_then(Value::as_str) {
                    out.push((event.clone(), command.to_string()));
                }
            }
        }
    }
    out
}

/// The lines of `a` that `b` does not have, counting repeats: two copies in `a` and one in `b`
/// leaves one. Comparing the file before against the file after is what makes a re-run report
/// nothing, rather than the installer reporting what it meant to do.
fn difference(a: &[(String, String)], b: &[(String, String)]) -> Vec<(String, String)> {
    let mut matched = vec![false; b.len()];
    let mut out = Vec::new();
    for line in a {
        match b
            .iter()
            .enumerate()
            .find(|(i, other)| !matched[*i] && *other == line)
        {
            Some((i, _)) => matched[i] = true,
            None => out.push(line.clone()),
        }
    }
    out
}

/// Removes V1's status line and returns its command. A status line the author wrote is left
/// alone, as V1's own installer leaves it (M3 plan assumption 18).
fn remove_v1_status_line(root: &mut Map<String, Value>) -> Option<String> {
    let command = root
        .get("statusLine")?
        .get("command")?
        .as_str()
        .filter(|c| c.contains("claude-statusline"))?
        .to_string();
    root.remove("statusLine");
    Some(command)
}

/// Drops every line domux wrote before, then adds one command per event. Tool events carry the
/// `*` matcher, as V1 writes them.
fn patch_hooks(root: &mut Map<String, Value>, events: &[&str], command: &str) {
    let Some(hooks) = root
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
    else {
        return;
    };
    // Every event, not only the installed ones: an event dropped from the list would otherwise
    // keep an orphan line that no release writes any more.
    for event in hooks.keys().cloned().collect::<Vec<String>>() {
        let Some(entries) = hooks.get_mut(&event).and_then(Value::as_array_mut) else {
            continue;
        };
        for entry in entries.iter_mut() {
            let Some(list) = entry.get_mut("hooks").and_then(Value::as_array_mut) else {
                continue;
            };
            list.retain(|hook| {
                let command = hook.get("command").and_then(Value::as_str).unwrap_or("");
                !(is_v1_line(command) || is_v2_line(command))
            });
        }
        // An entry whose last line went carries nothing, and an event with no entries left is
        // an empty key. An entry the installer does not understand keeps its place.
        entries.retain(|entry| {
            entry
                .get("hooks")
                .and_then(Value::as_array)
                .is_none_or(|list| !list.is_empty())
        });
        if entries.is_empty() {
            hooks.remove(&event);
        }
    }
    for event in events {
        let entry = match *event {
            "PreToolUse" | "PostToolUse" | "PermissionRequest" => {
                json!({"matcher": "*", "hooks": [{"type": "command", "command": command}]})
            }
            _ => json!({"hooks": [{"type": "command", "command": command}]}),
        };
        if let Value::Array(entries) = hooks.entry(*event).or_insert_with(|| json!([])) {
            entries.push(entry);
        }
    }
}

/// What `install <kind>` prints without `--apply`.
pub fn preview(p: &Plan) -> String {
    let verb = if p.before.is_some() {
        "patch"
    } else {
        "create"
    };
    let mut out = format!("Would {verb} {} for {}:\n\n", p.path.display(), p.kind);
    if p.changes_nothing() {
        out.push_str("Nothing to change. The hooks are already installed.\n");
        return out;
    }
    for (event, command) in &p.removed {
        out.push_str(&format!("- {event:<18} {command}\n"));
    }
    for (event, command) in &p.added {
        out.push_str(&format!("+ {event:<18} {command}\n"));
    }
    for note in &p.notes {
        out.push_str(&format!("\n{note}\n"));
    }
    out.push_str("\nRun it again with --apply to write it.");
    if p.before.is_some() {
        out.push_str(" The previous file is kept as a backup.");
    }
    out.push('\n');
    out
}

/// Writes the file, keeping the previous one as `<path>.domux-backup-<YYYYmmddHHMMSS>` (V1's
/// `backupIfExists` convention, commit 34db116). Returns the backup's path, or the file's own
/// path when there was nothing to back up: the file was not there, or the plan changes nothing
/// and this wrote nothing at all.
pub fn apply(p: &Plan) -> Result<PathBuf, InstallError> {
    if p.changes_nothing() {
        return Ok(p.path.clone());
    }
    if let Some(parent) = p.path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| InstallError::Write {
            path: p.path.clone(),
            message: e.to_string(),
        })?;
    }
    let mut backup = p.path.clone();
    if p.before.is_some() {
        backup = backup_path(
            &p.path,
            &chrono::Local::now().format("%Y%m%d%H%M%S").to_string(),
        );
        // A copy of the file, not a write of what `plan` read: it keeps the permissions, which
        // a settings file that holds a key wants kept.
        std::fs::copy(&p.path, &backup).map_err(|e| InstallError::Write {
            path: backup.clone(),
            message: e.to_string(),
        })?;
    }
    let tmp = suffixed(&p.path, ".tmp");
    std::fs::write(&tmp, &p.after).map_err(|e| InstallError::Write {
        path: tmp.clone(),
        message: e.to_string(),
    })?;
    std::fs::rename(&tmp, &p.path).map_err(|e| InstallError::Write {
        path: p.path.clone(),
        message: e.to_string(),
    })?;
    Ok(backup)
}

/// Where the file that `apply` replaces is kept.
pub fn backup_path(path: &Path, stamp: &str) -> PathBuf {
    suffixed(path, &format!(".domux-backup-{stamp}"))
}

/// The path with `suffix` after the whole file name, so `settings.json` keeps its extension
/// and the new file sorts next to it.
fn suffixed(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

/// The OpenCode plugin. OpenCode has no hooks file, so domux writes the plugin and picks the
/// payload: the field names Claude uses, with OpenCode's own event names, so one adapter shape
/// covers all three kinds (`agents::hooks::parse_opencode`).
pub fn opencode_plugin(bin: &Path) -> String {
    let path = Value::String(bin.to_string_lossy().into_owned());
    format!("const domux = {path}\n{PLUGIN_BODY}")
}

/// The plugin below the line that names the binary. OpenCode runs on Bun, so the plugin spawns
/// the command and writes the payload to its standard input, as V1's plugin does.
///
/// `messageText` is why the plugin has a helper at all. `agents::hooks::string` answers absent
/// for a field that is not a JSON string, and it is right to: the rule is never to fabricate a
/// value the payload did not carry. So a `message` that arrives as an object reaches the record
/// as no reason, and the reason of a permission request is exactly what M4's toast reads. The
/// plugin is domux's own file, so it is the end that owes the adapter a string, and JSON text is
/// the one form that keeps whatever the event carried without guessing at a field inside it.
const PLUGIN_BODY: &str = r#"
function messageText(message) {
  if (message === null || message === undefined) return null
  if (typeof message === "string") return message
  try {
    return JSON.stringify(message) ?? null
  } catch {
    return null
  }
}

async function report(event, input) {
  try {
    const payload = JSON.stringify({
      hook_event_name: event,
      session_id: input?.sessionID ?? input?.session?.id ?? null,
      cwd: input?.directory ?? process.cwd(),
      message: messageText(input?.message),
    })
    const proc = Bun.spawn([domux, "agent", "report", "--agent", "opencode"], {
      stdin: "pipe",
      stdout: "ignore",
      stderr: "ignore",
    })
    proc.stdin.write(payload)
    proc.stdin.end()
    await proc.exited
  } catch {
  }
}

export const DomuxPlugin = async () => ({
  "tool.execute.before": (input) => report("tool.execute.before", input),
  "tool.execute.after": (input) => report("tool.execute.after", input),
  event: async ({ event }) => {
    switch (event.type) {
      case "session.created":
      case "message.updated":
      case "permission.asked":
      case "permission.replied":
      case "session.idle":
      case "session.error":
      case "session.deleted":
        await report(event.type, event.properties)
        break
    }
  },
})
"#;
