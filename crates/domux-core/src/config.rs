//! `domux.toml`: one struct per table, defaults from the specs, parse errors with a line
//! number, unknown tables and keys reported and kept out of the way. M1 reads `[keys]` and
//! `[terminal]`; there is no layout table (roadmap section 5.5).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub keys: KeysConfig,
    pub terminal: TerminalConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KeysConfig {
    pub leader: String,
    /// After the leader. Key name to action, for example `"|" = "pane.split right"`.
    pub bindings: BTreeMap<String, String>,
    /// Without the leader.
    pub global: BTreeMap<String, String>,
    pub passthrough: PassthroughConfig,
    /// Inside the Projects and Agents boxes (interface spec section 10). Parsed in M1, used
    /// from M2.
    pub list: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PassthroughConfig {
    pub commands: Vec<String>,
    pub keys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TerminalConfig {
    /// `None` means `$SHELL`, then `/bin/sh`.
    pub shell: Option<String>,
    pub scrollback: usize,
    pub remain_on_exit: bool,
}

impl TerminalConfig {
    pub fn shell_or_default(&self) -> String {
        self.shell
            .clone()
            .or_else(|| std::env::var("SHELL").ok().filter(|s| !s.is_empty()))
            .unwrap_or_else(|| "/bin/sh".to_string())
    }
}

fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

impl Default for KeysConfig {
    fn default() -> Self {
        KeysConfig {
            leader: "C-a".into(),
            bindings: map(&[
                ("|", "pane.split right"),
                ("-", "pane.split down"),
                ("z", "pane.zoom"),
                ("[", "pane.copy_mode"),
                ("c", "tab.create"),
                ("1", "tab.select 1"),
                ("2", "tab.select 2"),
                ("3", "tab.select 3"),
                ("4", "tab.select 4"),
                ("5", "tab.select 5"),
                ("6", "tab.select 6"),
                ("7", "tab.select 7"),
                ("8", "tab.select 8"),
                ("9", "tab.select 9"),
                ("R", "tab.clear_name"),
                (",", "tab.rename"),
                ("d", "client.detach"),
                ("?", "help"),
            ]),
            global: map(&[
                ("C-h", "focus.left"),
                ("C-j", "focus.down"),
                ("C-k", "focus.up"),
                ("C-l", "focus.right"),
                ("C-\\", "focus.last"),
                ("S-Left", "pane.resize left 2"),
                ("S-Right", "pane.resize right 2"),
            ]),
            passthrough: PassthroughConfig::default(),
            // Keys inside the Projects and Agents boxes. Empty in M1: M2 and M3 add the
            // actions they implement (roadmap section 5.5).
            list: BTreeMap::new(),
        }
    }
}

impl Default for PassthroughConfig {
    fn default() -> Self {
        PassthroughConfig {
            commands: vec!["nvim".into(), "vim".into(), "fzf".into()],
            keys: vec![
                "C-h".into(),
                "C-j".into(),
                "C-k".into(),
                "C-l".into(),
                "C-\\".into(),
            ],
        }
    }
}

impl Default for TerminalConfig {
    fn default() -> Self {
        TerminalConfig {
            shell: None,
            scrollback: 10000,
            remain_on_exit: false,
        }
    }
}

/// A parse error with the position the person needs to fix it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("domux.toml line {line}: {message}")]
pub struct ConfigError {
    pub line: usize,
    pub column: usize,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigWarning(pub String);

#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    pub config: Config,
    pub warnings: Vec<ConfigWarning>,
}

pub const KNOWN_TABLES: &[&str] = &["keys", "terminal"];
pub const KNOWN_KEYS: &[(&str, &[&str])] = &[
    (
        "keys",
        &["leader", "bindings", "global", "passthrough", "list"],
    ),
    ("keys.passthrough", &["commands", "keys"]),
    ("terminal", &["shell", "scrollback", "remain_on_exit"]),
];

impl Config {
    /// Parses a whole file. Keys the user sets replace the defaults key by key; binding
    /// tables merge with the defaults, and an empty action string removes a default binding.
    pub fn parse(text: &str) -> Result<Parsed, ConfigError> {
        let doc: toml::Table = toml::from_str(text).map_err(|e| to_config_error(text, e))?;
        let mut warnings = Vec::new();
        for (name, value) in &doc {
            if !KNOWN_TABLES.contains(&name.as_str()) {
                let line = line_of_key(text, name).unwrap_or(1);
                warnings.push(ConfigWarning(format!("unknown table [{name}] (line {line}) is ignored until the milestone that reads it")));
                continue;
            }
            if let Some(table) = value.as_table() {
                warn_unknown_keys(text, name, table, &mut warnings);
            }
        }
        let user: Config = toml::from_str(text).map_err(|e| to_config_error(text, e))?;
        // Build the value in one expression. Starting from `Config::default()` and then
        // assigning fields trips `clippy::field_reassign_with_default`, which is in
        // `clippy::all`, which the gate runs as `-D warnings`.
        let defaults = KeysConfig::default();
        let mut keys = KeysConfig {
            leader: user.keys.leader,
            passthrough: user.keys.passthrough,
            bindings: defaults.bindings.clone(),
            global: defaults.global.clone(),
            list: defaults.list.clone(),
        };
        merge(&mut keys.bindings, user.keys.bindings, &defaults.bindings);
        merge(&mut keys.global, user.keys.global, &defaults.global);
        merge(&mut keys.list, user.keys.list, &defaults.list);
        // Name every field rather than using `..Config::default()`, so a table added in a
        // later milestone is a compile error here instead of a silently dropped user setting.
        let config = Config {
            keys,
            terminal: user.terminal,
        };
        Ok(Parsed { config, warnings })
    }
}

/// `user` came from `#[serde(default)]`, so it already holds the defaults for keys the file
/// did not mention. Merge so a mentioned key wins and an empty value unbinds.
fn merge(
    into: &mut BTreeMap<String, String>,
    user: BTreeMap<String, String>,
    defaults: &BTreeMap<String, String>,
) {
    for (k, v) in user {
        if v.trim().is_empty() {
            into.remove(&k);
        } else if defaults.get(&k) != Some(&v) || !into.contains_key(&k) {
            into.insert(k, v);
        }
    }
}

fn warn_unknown_keys(
    text: &str,
    table: &str,
    value: &toml::Table,
    warnings: &mut Vec<ConfigWarning>,
) {
    let Some((_, known)) = KNOWN_KEYS.iter().find(|(t, _)| *t == table) else {
        return;
    };
    for (key, v) in value {
        let path = format!("{table}.{key}");
        if !known.contains(&key.as_str()) {
            let line = line_of_key(text, key).unwrap_or(1);
            warnings.push(ConfigWarning(format!(
                "unknown key {path} (line {line}) is ignored"
            )));
        } else if let Some(sub) = v.as_table() {
            if KNOWN_KEYS.iter().any(|(t, _)| *t == path) {
                warn_unknown_keys(text, &path, sub, warnings);
            }
        }
    }
}

fn to_config_error(text: &str, e: toml::de::Error) -> ConfigError {
    let (line, column) = match e.span() {
        Some(span) => position(text, span.start),
        None => (1, 1),
    };
    ConfigError {
        line,
        column,
        message: e.message().to_string(),
    }
}

/// 1-based line and column of a byte offset.
fn position(text: &str, offset: usize) -> (usize, usize) {
    let before = &text[..offset.min(text.len())];
    let line = before.matches('\n').count() + 1;
    let column = before
        .rsplit('\n')
        .next()
        .map(|s| s.chars().count())
        .unwrap_or(0)
        + 1;
    (line, column)
}

/// The first line whose trimmed text starts with `[key]`, `[key.`, or `key =`.
fn line_of_key(text: &str, key: &str) -> Option<usize> {
    text.lines()
        .position(|l| {
            let l = l.trim_start();
            l.starts_with(&format!("[{key}]"))
                || l.starts_with(&format!("[{key}."))
                || l.starts_with(&format!("{key} "))
                || l.starts_with(&format!("{key}="))
                || l.starts_with(&format!("\"{key}\""))
        })
        .map(|i| i + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_architecture_spec_keymap() {
        let c = Config::default();
        assert_eq!(c.keys.leader, "C-a");
        assert_eq!(
            c.keys.bindings.get("|").map(String::as_str),
            Some("pane.split right")
        );
        assert_eq!(
            c.keys.bindings.get("-").map(String::as_str),
            Some("pane.split down")
        );
        assert_eq!(
            c.keys.bindings.get("z").map(String::as_str),
            Some("pane.zoom")
        );
        assert_eq!(
            c.keys.bindings.get("[").map(String::as_str),
            Some("pane.copy_mode")
        );
        assert_eq!(
            c.keys.bindings.get("c").map(String::as_str),
            Some("tab.create")
        );
        assert_eq!(
            c.keys.bindings.get("9").map(String::as_str),
            Some("tab.select 9")
        );
        assert_eq!(
            c.keys.bindings.get("R").map(String::as_str),
            Some("tab.clear_name")
        );
        assert_eq!(
            c.keys.bindings.get(",").map(String::as_str),
            Some("tab.rename")
        );
        assert_eq!(
            c.keys.bindings.get("d").map(String::as_str),
            Some("client.detach")
        );
        assert_eq!(c.keys.bindings.get("?").map(String::as_str), Some("help"));
        assert_eq!(
            c.keys.global.get("C-h").map(String::as_str),
            Some("focus.left")
        );
        assert_eq!(
            c.keys.global.get("C-\\").map(String::as_str),
            Some("focus.last")
        );
        assert_eq!(
            c.keys.global.get("S-Left").map(String::as_str),
            Some("pane.resize left 2")
        );
        assert_eq!(c.keys.passthrough.commands, vec!["nvim", "vim", "fzf"]);
        assert_eq!(
            c.keys.passthrough.keys,
            vec!["C-h", "C-j", "C-k", "C-l", "C-\\"]
        );
        assert_eq!(c.terminal.scrollback, 10000);
        assert!(!c.terminal.remain_on_exit);
        assert_eq!(c.terminal.shell, None);
        assert!(c.keys.list.is_empty(), "the list table arrives with M2");
        assert!(
            !c.keys.bindings.contains_key("s")
                && !c.keys.bindings.contains_key("b")
                && !c.keys.bindings.contains_key("a"),
            "switcher, sidebar and agents overlay keys arrive with their milestones"
        );
    }

    #[test]
    fn user_values_override_defaults_and_bindings_merge() {
        let parsed = Config::parse("[keys]\nleader = \"C-b\"\n[keys.bindings]\n\"g\" = \"pane.split right\"\n[terminal]\nscrollback = 500\n").unwrap();
        assert_eq!(parsed.config.keys.leader, "C-b");
        assert_eq!(
            parsed.config.keys.bindings.get("g").map(String::as_str),
            Some("pane.split right")
        );
        assert_eq!(
            parsed.config.keys.bindings.get("|").map(String::as_str),
            Some("pane.split right"),
            "defaults stay unless overridden"
        );
        assert_eq!(parsed.config.terminal.scrollback, 500);
        assert!(parsed.warnings.is_empty());
    }

    #[test]
    fn unbinding_a_default_uses_an_empty_string() {
        let parsed = Config::parse("[keys.bindings]\n\"z\" = \"\"\n").unwrap();
        assert!(!parsed.config.keys.bindings.contains_key("z"));
    }

    #[test]
    fn parse_errors_carry_the_line_number() {
        let err =
            Config::parse("[keys]\nleader = \"C-a\"\n[terminal]\nscrollback = \n").unwrap_err();
        assert_eq!(err.line, 4);
        assert!(err.message.contains("expected"), "{}", err.message);
        assert!(err.to_string().starts_with("domux.toml line 4: "), "{err}");
    }

    #[test]
    fn unknown_tables_and_keys_are_kept_as_warnings_not_errors() {
        let parsed =
            Config::parse("[worktrees]\nbase = \"origin/main\"\n[terminal]\ncolour = \"x\"\n")
                .unwrap();
        assert_eq!(parsed.warnings.len(), 2);
        assert!(parsed.warnings.iter().any(|w| w.0 == "unknown table [worktrees] (line 1) is ignored until the milestone that reads it"), "{:?}", parsed.warnings);
        assert!(
            parsed
                .warnings
                .iter()
                .any(|w| w.0 == "unknown key terminal.colour (line 4) is ignored"),
            "{:?}",
            parsed.warnings
        );
    }
}
