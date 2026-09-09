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
    pub worktrees: WorktreesConfig,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KeysConfig {
    pub leader: String,
    /// After the leader. Key name to action, for example `"\\" = "pane.split right"`.
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

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorktreesConfig {
    /// The ref a new slot branches from and a cleared slot resets to. `None` means detect
    /// it per project from `origin/HEAD`, falling back to `main` (architecture spec 2).
    pub base: Option<String>,
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
                // The unshifted half of the key `|` lives on: a split is common enough that
                // it should not need a shift.
                ("\\", "pane.split right"),
                ("-", "pane.split down"),
                ("z", "pane.zoom"),
                ("[", "pane.copy_mode"),
                // Empties the pane, screen and scrollback. The key a reader reaches for when
                // the shell in front of them will not run `clear` because its line editor is
                // holding something they did not type.
                ("k", "pane.clear"),
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
                // Destructive, so the key asks first: see `Core::confirmation_for`.
                ("x", "tab.close"),
                ("R", "tab.clear_name"),
                (",", "tab.rename"),
                ("d", "client.detach"),
                ("?", "help"),
                ("s", "switcher.open"),
                ("b", "sidebar.toggle"),
                ("N", "workspace.rename"),
                ("n", "workspace.clear_name"),
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
            // Keys inside the Projects box, with no leader (interface spec section 10).
            // M3 adds "Tab" = "focus.next_region" with the Agents box it crosses to.
            list: map(&[
                ("j", "list.down"),
                ("k", "list.up"),
                ("Down", "list.down"),
                ("Up", "list.up"),
                ("Enter", "list.activate"),
                ("Esc", "focus.pane"),
                ("/", "list.filter"),
                ("?", "help"),
                ("n", "workspace.rename"),
            ]),
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

/// A config error with the position the person needs to fix it, when there is one.
///
/// The position is optional because it does not always arrive: a toml error can come without
/// a span, and an error found after parsing - a key whose value is not a key name, a file
/// that could not be read - has no position in the file at all. An absent line renders as
/// absent, never as line 1: a notice that sends the reader to the wrong line is worse than
/// one that sends them to the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    pub line: Option<usize>,
    pub column: Option<usize>,
    /// One line. Never empty, never a control character: it is drawn into the top bar, where
    /// a newline would be dropped and would silently join two clauses into one word.
    pub message: String,
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.line {
            Some(line) => write!(f, "domux.toml line {line}: {}", self.message),
            None => write!(f, "domux.toml: {}", self.message),
        }
    }
}

impl std::error::Error for ConfigError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigWarning(pub String);

#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    pub config: Config,
    pub warnings: Vec<ConfigWarning>,
}

pub const KNOWN_TABLES: &[&str] = &["keys", "terminal", "worktrees"];
pub const KNOWN_KEYS: &[(&str, &[&str])] = &[
    (
        "keys",
        &["leader", "bindings", "global", "passthrough", "list"],
    ),
    ("keys.passthrough", &["commands", "keys"]),
    ("terminal", &["shell", "scrollback", "remain_on_exit"]),
    ("worktrees", &["base"]),
];

impl Config {
    /// Parses a whole file. Keys the user sets replace the defaults key by key; binding
    /// tables merge with the defaults, and an empty action string removes a default binding.
    pub fn parse(text: &str) -> Result<Parsed, ConfigError> {
        let doc: toml::Table = toml::from_str(text).map_err(|e| to_config_error(text, e))?;
        let mut warnings = Vec::new();
        for (name, value) in &doc {
            if !KNOWN_TABLES.contains(&name.as_str()) {
                let message = match line_of_key(text, name) {
                    Some(line) => format!(
                        "unknown table [{name}] (line {line}) is ignored until the milestone that reads it"
                    ),
                    None => format!(
                        "unknown table [{name}] is ignored until the milestone that reads it"
                    ),
                };
                warnings.push(ConfigWarning(message));
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
            worktrees: user.worktrees,
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
            let message = match line_of_key(text, key) {
                Some(line) => format!("unknown key {path} (line {line}) is ignored"),
                None => format!("unknown key {path} is ignored"),
            };
            warnings.push(ConfigWarning(message));
        } else if let Some(sub) = v.as_table() {
            if KNOWN_KEYS.iter().any(|(t, _)| *t == path) {
                warn_unknown_keys(text, &path, sub, warnings);
            }
        }
    }
}

fn to_config_error(text: &str, e: toml::de::Error) -> ConfigError {
    let (line, column) = match e.span() {
        Some(span) => {
            let (line, column) = position(text, span.start);
            (Some(line), Some(column))
        }
        // No span, so no position. Absent, not line 1 (roadmap section 3: never fabricate).
        None => (None, None),
    };
    ConfigError {
        line,
        column,
        message: one_line(e.message()),
    }
}

/// A message the top bar can draw: one line, no control characters, no empty result.
///
/// toml writes a two-clause error over two lines, `invalid string` then `expected \`"\`, \`'\``,
/// and the renderer drops control characters, so the raw message reaches the bar as
/// `invalid stringexpected`. Join the clauses with `; ` here, where the text is made, so the
/// API result and the notice say the same one-line thing.
pub(crate) fn one_line(message: &str) -> String {
    let joined = message
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("; ");
    let clean = crate::text::sanitize_for_display(&joined);
    if clean.trim().is_empty() {
        // A message that sanitized away still has to say something (principle 9).
        "the file could not be parsed".to_string()
    } else {
        clean
    }
}

/// 1-based line and column of a byte offset. Clamped so a span at or past end of input names
/// the file's actual last line rather than one past it, and safe against an offset that lands
/// inside a multi-byte character rather than on a char boundary.
fn position(text: &str, offset: usize) -> (usize, usize) {
    let mut end = offset.min(text.len());
    let before = loop {
        match text.get(..end) {
            Some(s) => break s,
            None => end -= 1,
        }
    };
    let line = before.matches('\n').count() + 1;
    let max_line = text.lines().count().max(1);
    let line = line.min(max_line);
    let column = before
        .rsplit('\n')
        .next()
        .map(|s| s.chars().count())
        .unwrap_or(0)
        + 1;
    (line, column)
}

/// The first line whose trimmed text starts with `[key]`, `[key.`, or `key` followed by
/// (optional whitespace and) `=`. Returns `None` rather than a guess when no line matches -
/// the caller must render that as an absent position, never as a default line number.
fn line_of_key(text: &str, key: &str) -> Option<usize> {
    text.lines()
        .position(|l| {
            let l = l.trim_start();
            if l.starts_with(&format!("[{key}]")) || l.starts_with(&format!("[{key}.")) {
                return true;
            }
            if let Some(rest) = l.strip_prefix(key) {
                if rest.trim_start().starts_with('=') {
                    return true;
                }
            }
            if let Some(rest) = l.strip_prefix(&format!("\"{key}\"")) {
                if rest.trim_start().starts_with('=') {
                    return true;
                }
            }
            false
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
        // The spec's keymap put this on `|`; see the M1 deviations.
        assert_eq!(
            c.keys.bindings.get("\\").map(String::as_str),
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
        // M2 adds the switcher, sidebar and workspace name keys; the agents overlay stays
        // unbound until M3.
        assert_eq!(
            c.keys.bindings.get("s").map(String::as_str),
            Some("switcher.open")
        );
        assert_eq!(
            c.keys.bindings.get("b").map(String::as_str),
            Some("sidebar.toggle")
        );
        assert_eq!(
            c.keys.bindings.get("N").map(String::as_str),
            Some("workspace.rename")
        );
        assert_eq!(
            c.keys.bindings.get("n").map(String::as_str),
            Some("workspace.clear_name")
        );
        assert!(
            !c.keys.bindings.contains_key("a"),
            "agents overlay arrives with M3"
        );
        assert_eq!(
            c.keys.list.get("Enter").map(String::as_str),
            Some("list.activate")
        );
        assert_eq!(
            c.keys.list.get("Esc").map(String::as_str),
            Some("focus.pane")
        );
        assert!(
            !c.keys.list.contains_key("Tab"),
            "Tab crosses nothing until M3 adds the Agents box"
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
            parsed.config.keys.bindings.get("\\").map(String::as_str),
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
        assert_eq!(err.line, Some(4));
        assert!(err.message.contains("expected"), "{}", err.message);
        assert!(err.to_string().starts_with("domux.toml line 4: "), "{err}");
    }

    /// The message is drawn into the top bar, which drops control characters. toml writes a
    /// two-clause error over two lines, so the raw text arrives there as `invalid
    /// stringexpected` - two clauses welded into one word.
    #[test]
    fn a_parse_error_message_is_one_line() {
        let err =
            Config::parse("[keys]\nleader = \"C-a\"\n[terminal]\nscrollback = \n").unwrap_err();
        assert_eq!(err.message, "invalid string; expected `\"`, `'`");
        assert!(!err.message.contains('\n'));
        assert_eq!(
            err.to_string(),
            "domux.toml line 4: invalid string; expected `\"`, `'`"
        );
    }

    #[test]
    fn one_line_joins_clauses_and_never_answers_with_nothing() {
        assert_eq!(
            one_line("invalid string\nexpected `\"`"),
            "invalid string; expected `\"`"
        );
        assert_eq!(one_line("  spaced  \n\n  out  "), "spaced; out");
        assert_eq!(one_line("plain"), "plain");
        assert_eq!(one_line("\u{7}\u{200b}"), "the file could not be parsed");
    }

    /// An error with no position renders without one. A guessed line 1 sends the reader to a
    /// line that is not the mistake, which is worse than sending them to the file (roadmap
    /// section 3: never fabricate).
    #[test]
    fn an_error_with_no_position_names_the_file_and_no_line() {
        let err = ConfigError {
            line: None,
            column: None,
            message: "[keys] leader: unknown key \"Ctrl-Q\"".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "domux.toml: [keys] leader: unknown key \"Ctrl-Q\""
        );
    }

    #[test]
    fn overriding_an_existing_default_binding_replaces_it() {
        let parsed = Config::parse("[keys.bindings]\n\"z\" = \"tab.create\"\n").unwrap();
        assert_eq!(
            parsed.config.keys.bindings.get("z").map(String::as_str),
            Some("tab.create"),
            "a mentioned key must win over the default it replaces"
        );
        assert_eq!(
            parsed.config.keys.bindings.get("\\").map(String::as_str),
            Some("pane.split right"),
            "an untouched default must survive alongside the override"
        );
    }

    #[test]
    fn partial_passthrough_table_keeps_the_other_fields_default() {
        let parsed = Config::parse("[keys.passthrough]\ncommands = [\"nvim\"]\n").unwrap();
        assert_eq!(parsed.config.keys.passthrough.commands, vec!["nvim"]);
        assert_eq!(
            parsed.config.keys.passthrough.keys,
            vec!["C-h", "C-j", "C-k", "C-l", "C-\\"],
            "a field the file never mentions must keep PassthroughConfig::default()"
        );
    }

    #[test]
    fn unknown_key_line_is_found_despite_a_tab_before_the_equals() {
        let parsed = Config::parse("[terminal]\ncolour\t= \"x\"\n").unwrap();
        assert!(
            parsed
                .warnings
                .iter()
                .any(|w| w.0 == "unknown key terminal.colour (line 2) is ignored"),
            "{:?}",
            parsed.warnings
        );
    }

    #[test]
    fn unknown_key_warning_omits_the_line_when_the_heuristic_cannot_find_it() {
        // A dotted key at the top level puts `colour` on a line that does not start with
        // `colour`, so the line-finding heuristic misses. The warning must say so plainly
        // rather than naming a line the parser never produced.
        let parsed = Config::parse("terminal.colour = \"x\"\n").unwrap();
        assert!(
            parsed
                .warnings
                .iter()
                .any(|w| w.0 == "unknown key terminal.colour is ignored"),
            "{:?}",
            parsed.warnings
        );
    }

    #[test]
    fn parse_error_line_never_exceeds_the_files_line_count() {
        for text in [
            "[keys]\nleader = \"C-a\n",
            "[keys]\nx = [1, 2\n",
            "[keys\n",
            "[keys]\nx =\n",
        ] {
            let err = Config::parse(text).unwrap_err();
            let max_line = text.lines().count().max(1);
            let line = err.line.expect("a syntax error has a span");
            assert!(
                line <= max_line,
                "{text:?} reported line {line} past the file's {max_line} lines"
            );
        }
    }

    #[test]
    fn position_clamps_the_line_to_the_files_last_line() {
        let text = "a\nb\nc\nd\n";
        assert_eq!(position(text, text.len()), (4, 1));
    }

    #[test]
    fn position_does_not_panic_on_a_non_char_boundary_offset() {
        let text = "\u{e9}"; // two UTF-8 bytes; offset 1 lands inside the character
        let (line, column) = position(text, 1);
        assert_eq!(line, 1);
        assert!(column >= 1);
    }

    #[test]
    fn unknown_tables_and_keys_are_kept_as_warnings_not_errors() {
        let parsed =
            Config::parse("[bogus]\nbase = \"origin/main\"\n[terminal]\ncolour = \"x\"\n").unwrap();
        assert_eq!(parsed.warnings.len(), 2);
        assert!(
            parsed.warnings.iter().any(|w| w.0
                == "unknown table [bogus] (line 1) is ignored until the milestone that reads it"),
            "{:?}",
            parsed.warnings
        );
        assert!(
            parsed
                .warnings
                .iter()
                .any(|w| w.0 == "unknown key terminal.colour (line 4) is ignored"),
            "{:?}",
            parsed.warnings
        );
    }

    #[test]
    fn worktrees_base_parses_and_defaults_to_absent() {
        let parsed = Config::parse("").unwrap();
        assert_eq!(
            parsed.config.worktrees.base, None,
            "no base means detect it from origin/HEAD"
        );
        let parsed = Config::parse("[worktrees]\nbase = \"origin/develop\"\n").unwrap();
        assert_eq!(
            parsed.config.worktrees.base.as_deref(),
            Some("origin/develop")
        );
        assert!(parsed.warnings.is_empty(), "{:?}", parsed.warnings);
    }

    #[test]
    fn an_unknown_key_under_worktrees_warns_with_its_line_and_keeps_the_rest() {
        let parsed = Config::parse("[worktrees]\nbase = \"origin/main\"\ndepth = 1\n").unwrap();
        assert_eq!(parsed.config.worktrees.base.as_deref(), Some("origin/main"));
        assert_eq!(parsed.warnings.len(), 1);
        assert!(
            parsed.warnings[0].0.contains("depth"),
            "{:?}",
            parsed.warnings
        );
    }
}
