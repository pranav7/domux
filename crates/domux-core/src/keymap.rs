//! Key names as `domux.toml` spells them, actions as method names with positional
//! arguments, and the keymap that resolves a key event to an action.

use crate::config::KeysConfig;
use domux_term::{Key, KeyAction, KeyEvent, Mods};
use std::fmt;

/// A key as written in the config: modifiers `C-`, `S-`, `M-` (alt), `D-` (super) in that
/// order, then a named key or one character.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyName {
    pub key: Key,
    pub mods: Mods,
}

const NAMED: &[(&str, Key)] = &[
    ("Enter", Key::Enter),
    ("Tab", Key::Tab),
    ("Backspace", Key::Backspace),
    ("Esc", Key::Escape),
    ("Space", Key::Char(' ')),
    ("Up", Key::Up),
    ("Down", Key::Down),
    ("Left", Key::Left),
    ("Right", Key::Right),
    ("Home", Key::Home),
    ("End", Key::End),
    ("PageUp", Key::PageUp),
    ("PageDown", Key::PageDown),
    ("Insert", Key::Insert),
    ("Delete", Key::Delete),
];

impl KeyName {
    pub fn parse(s: &str) -> Result<KeyName, String> {
        if s.is_empty() {
            return Err("empty key name".into());
        }
        let mut mods = Mods::empty();
        let mut rest = s;
        while let Some((prefix, tail)) = rest.split_once('-') {
            if tail.is_empty() {
                break; // the key itself is "-"
            }
            let m = match prefix {
                "C" => Mods::CTRL,
                "S" => Mods::SHIFT,
                "M" => Mods::ALT,
                "D" => Mods::SUPER,
                _ => break,
            };
            mods |= m;
            rest = tail;
        }
        let key = if let Some((_, k)) = NAMED.iter().find(|(n, _)| *n == rest) {
            *k
        } else if let Some(n) = rest
            .strip_prefix('F')
            .and_then(|n| n.parse::<u8>().ok())
            .filter(|n| (1..=12).contains(n))
        {
            Key::F(n)
        } else {
            let mut chars = rest.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Key::Char(c),
                _ => return Err(format!("unknown key {s:?}; write modifiers as C-, S-, M-, D- and keys as a character, Enter, Esc, Tab, Space, Backspace, arrows, Home, End, PageUp, PageDown, Insert, Delete or F1 to F12")),
            }
        };
        Ok(KeyName { key, mods })
    }

    /// True for a press or repeat of this key. For character keys the shift flag is
    /// ignored, because the character already carries the case (`|`, `R`).
    pub fn matches(&self, ev: &KeyEvent) -> bool {
        if ev.action == KeyAction::Release || ev.key != self.key {
            return false;
        }
        match self.key {
            Key::Char(_) => ev.mods.difference(Mods::SHIFT) == self.mods.difference(Mods::SHIFT),
            _ => ev.mods == self.mods,
        }
    }
}

impl fmt::Display for KeyName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (m, p) in [
            (Mods::CTRL, "C-"),
            (Mods::SHIFT, "S-"),
            (Mods::ALT, "M-"),
            (Mods::SUPER, "D-"),
        ] {
            if self.mods.contains(m) {
                f.write_str(p)?;
            }
        }
        if let Some((n, _)) = NAMED.iter().find(|(_, k)| *k == self.key) {
            return f.write_str(n);
        }
        match self.key {
            Key::F(n) => write!(f, "F{n}"),
            Key::Char(c) => write!(f, "{c}"),
            other => write!(f, "{other:?}"),
        }
    }
}

/// A method name plus positional arguments, as `"pane.split right"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    pub method: String,
    pub args: Vec<String>,
}

impl Action {
    pub fn parse(s: &str) -> Result<Action, String> {
        let mut parts = s.split_whitespace();
        let method = parts
            .next()
            .ok_or_else(|| "empty action".to_string())?
            .to_string();
        Ok(Action {
            method,
            args: parts.map(str::to_string).collect(),
        })
    }
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.method)?;
        for a in &self.args {
            write!(f, " {a}")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub key: KeyName,
    pub action: Action,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeymapWarning(pub String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keymap {
    pub leader: KeyName,
    pub bindings: Vec<Binding>,
    pub global: Vec<Binding>,
    pub passthrough_commands: Vec<String>,
    pub passthrough_keys: Vec<KeyName>,
}

impl Keymap {
    pub fn defaults() -> Keymap {
        Keymap::from_config(&KeysConfig::default())
            .expect("the default keymap parses")
            .0
    }

    /// Builds the keymap. A bad leader is an error; a bad key or action elsewhere is a
    /// warning and that binding is skipped, so one typo never disables the keyboard.
    pub fn from_config(cfg: &KeysConfig) -> Result<(Keymap, Vec<KeymapWarning>), String> {
        let leader = KeyName::parse(&cfg.leader).map_err(|e| format!("[keys] leader: {e}"))?;
        let mut warnings = Vec::new();
        let mut table =
            |name: &str, map: &std::collections::BTreeMap<String, String>| -> Vec<Binding> {
                let mut out = Vec::new();
                for (k, v) in map {
                    match (KeyName::parse(k), Action::parse(v)) {
                        (Ok(key), Ok(action)) => out.push(Binding { key, action }),
                        (Err(e), _) | (_, Err(e)) => {
                            warnings.push(KeymapWarning(format!("[{name}] {k:?}: {e}")))
                        }
                    }
                }
                out
            };
        let bindings = table("keys.bindings", &cfg.bindings);
        let global = table("keys.global", &cfg.global);
        let mut passthrough_keys = Vec::new();
        for k in &cfg.passthrough.keys {
            match KeyName::parse(k) {
                Ok(key) => passthrough_keys.push(key),
                Err(e) => {
                    warnings.push(KeymapWarning(format!("[keys.passthrough] keys {k:?}: {e}")))
                }
            }
        }
        Ok((
            Keymap {
                leader,
                bindings,
                global,
                passthrough_commands: cfg.passthrough.commands.clone(),
                passthrough_keys,
            },
            warnings,
        ))
    }

    /// The action for a key pressed after the leader.
    pub fn binding_for(&self, ev: &KeyEvent) -> Option<&Action> {
        self.bindings
            .iter()
            .find(|b| b.key.matches(ev))
            .map(|b| &b.action)
    }

    /// The action for a key pressed without the leader, unless `foreground` is a passthrough
    /// command and the key is a passthrough key.
    pub fn global_for(&self, ev: &KeyEvent, foreground: Option<&str>) -> Option<&Action> {
        let binding = self.global.iter().find(|b| b.key.matches(ev))?;
        let passes = foreground
            .is_some_and(|cmd| self.passthrough_commands.iter().any(|c| c == cmd))
            && self.passthrough_keys.iter().any(|k| k.matches(ev));
        if passes {
            None
        } else {
            Some(&binding.action)
        }
    }

    /// The configured key for an action, as hint text: `C-a ,` for a leader binding, `C-h`
    /// for a global one. Matches the whole action string, so `pane.split right` and
    /// `pane.split down` give different keys.
    pub fn key_for(&self, action: &str) -> Option<String> {
        if let Some(b) = self
            .bindings
            .iter()
            .find(|b| b.action.to_string() == action)
        {
            return Some(format!("{} {}", self.leader, b.key));
        }
        self.global
            .iter()
            .find(|b| b.action.to_string() == action)
            .map(|b| b.key.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domux_term::{Key, KeyAction, KeyEvent, Mods};

    fn press(key: Key, mods: Mods) -> KeyEvent {
        KeyEvent {
            key,
            mods,
            action: KeyAction::Press,
        }
    }

    #[test]
    fn key_names_parse_and_print_the_same_spelling() {
        for name in [
            "C-a",
            "S-Left",
            "Enter",
            "Esc",
            "Tab",
            "Space",
            "Backspace",
            "F5",
            "C-\\",
            "M-x",
            "|",
            ",",
            "C-S-Up",
            "PageDown",
        ] {
            let k = KeyName::parse(name).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(k.to_string(), name);
        }
        assert_eq!(
            KeyName::parse("C-a").unwrap(),
            KeyName {
                key: Key::Char('a'),
                mods: Mods::CTRL
            }
        );
        assert_eq!(
            KeyName::parse("S-Left").unwrap(),
            KeyName {
                key: Key::Left,
                mods: Mods::SHIFT
            }
        );
        assert!(KeyName::parse("Ctrl-a").is_err());
        assert!(KeyName::parse("").is_err());
    }

    #[test]
    fn char_keys_match_regardless_of_the_shift_flag() {
        let bar = KeyName::parse("|").unwrap();
        assert!(bar.matches(&press(Key::Char('|'), Mods::SHIFT)));
        assert!(bar.matches(&press(Key::Char('|'), Mods::empty())));
        assert!(!bar.matches(&press(Key::Char('|'), Mods::CTRL)));
        let upper = KeyName::parse("R").unwrap();
        assert!(upper.matches(&press(Key::Char('R'), Mods::SHIFT)));
        assert!(!upper.matches(&press(Key::Char('r'), Mods::empty())));
        let left = KeyName::parse("S-Left").unwrap();
        assert!(!left.matches(&press(Key::Left, Mods::empty())));
        assert!(!KeyName::parse("a").unwrap().matches(&KeyEvent {
            key: Key::Char('a'),
            mods: Mods::empty(),
            action: KeyAction::Release
        }));
    }

    #[test]
    fn actions_split_method_and_positional_args() {
        let a = Action::parse("pane.split right").unwrap();
        assert_eq!(a.method, "pane.split");
        assert_eq!(a.args, vec!["right"]);
        assert_eq!(a.to_string(), "pane.split right");
        assert_eq!(Action::parse("help").unwrap().method, "help");
        assert!(Action::parse("").is_err());
    }

    #[test]
    fn defaults_build_and_lookup_finds_bindings_and_globals() {
        let km = Keymap::defaults();
        assert_eq!(km.leader, KeyName::parse("C-a").unwrap());
        assert_eq!(
            km.binding_for(&press(Key::Char('\\'), Mods::empty()))
                .unwrap()
                .to_string(),
            "pane.split right"
        );
        assert_eq!(
            km.global_for(&press(Key::Char('h'), Mods::CTRL), Some("zsh"))
                .unwrap()
                .to_string(),
            "focus.left"
        );
        assert!(
            km.global_for(&press(Key::Char('h'), Mods::CTRL), Some("nvim"))
                .is_none(),
            "passthrough"
        );
        assert!(
            km.global_for(&press(Key::Char('h'), Mods::CTRL), None)
                .is_some(),
            "unknown foreground does not pass through"
        );
        assert_eq!(
            km.global_for(&press(Key::Left, Mods::SHIFT), Some("nvim"))
                .unwrap()
                .to_string(),
            "pane.resize left 2",
            "only listed keys pass through"
        );
        assert_eq!(km.key_for("tab.rename"), Some("C-a ,".to_string()));
        assert_eq!(km.key_for("focus.left"), Some("C-h".to_string()));
        assert_eq!(km.key_for("pane.split right"), Some("C-a \\".to_string()));
    }

    #[test]
    fn a_bad_key_name_is_a_warning_and_the_binding_is_skipped() {
        let mut cfg = crate::config::KeysConfig::default();
        cfg.bindings.insert("Ctrl-x".into(), "pane.zoom".into());
        let (km, warnings) = Keymap::from_config(&cfg).unwrap();
        assert_eq!(warnings.len(), 1);
        assert!(
            warnings[0].0.starts_with("[keys.bindings] \"Ctrl-x\": "),
            "{}",
            warnings[0].0
        );
        assert!(km
            .bindings
            .iter()
            .all(|b| b.action.method != "pane.zoom" || b.key.to_string() == "z"));
        cfg.leader = "nope".into();
        assert!(
            Keymap::from_config(&cfg).is_err(),
            "a bad leader is an error, since nothing would work"
        );
    }
}
