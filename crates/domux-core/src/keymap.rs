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
    /// Keys inside a box with no leader: the Projects box in M2, the Agents box in M3
    /// (interface spec section 10).
    pub list: Vec<Binding>,
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
        let mut table = |name: &str,
                         map: &std::collections::BTreeMap<String, String>|
         -> Vec<Binding> {
            let mut out = Vec::new();
            for (k, v) in map {
                match (KeyName::parse(k), Action::parse(v)) {
                    (Ok(key), Ok(action)) => {
                        // The action is checked against the methods the server has, and
                        // against what each one takes, so a typo is read out of the file
                        // by whoever is editing it rather than discovered later by
                        // pressing the key. The binding is kept even so: a bad key cannot
                        // be represented and has to be dropped, but a bad action can, and
                        // keeping it means the key answers with the reason instead of
                        // falling through to the pane and doing nothing at all.
                        if let Err(e) = crate::api::Method::from_action(&action) {
                            warnings.push(KeymapWarning(format!("[{name}] {k:?}: {}", e.message)));
                        }
                        out.push(Binding { key, action });
                    }
                    (Err(e), _) | (_, Err(e)) => {
                        warnings.push(KeymapWarning(format!("[{name}] {k:?}: {e}")))
                    }
                }
            }
            // The map iterates in key order, which has nothing to do with which key a
            // config file declared first: a `BTreeMap` does not remember that. When two
            // keys bind the same action - `j` and `Down` both to `list.down` in the
            // defaults - a lookup by action (`key_for`, `list_key_for`, `hint_for`) answers
            // with the first match, so this order is what decides which key a hint shows.
            // Shortest key first, alphabetically among equal lengths: the plain letter wins
            // over the named key it duplicates, since a hint reads faster as `j` than
            // `Down`.
            out.sort_by(|a, b| {
                let ak = a.key.to_string();
                let bk = b.key.to_string();
                (ak.len(), ak).cmp(&(bk.len(), bk))
            });
            out
        };
        let bindings = table("keys.bindings", &cfg.bindings);
        let global = table("keys.global", &cfg.global);
        let list = table("keys.list", &cfg.list);
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
                list,
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

    /// The action for a key pressed while a box has focus.
    pub fn list_for(&self, ev: &KeyEvent) -> Option<&Action> {
        self.list
            .iter()
            .find(|b| b.key.matches(ev))
            .map(|b| &b.action)
    }

    /// The configured key for a list action, as hint text: `⏎`, `esc`, `/`, `n`.
    pub fn list_key_for(&self, action: &str) -> Option<String> {
        self.list
            .iter()
            .find(|b| b.action.to_string() == action)
            .map(|b| hint_key(&b.key))
    }

    /// The configured key for an action, as hint text on a surface outside a box:
    /// `leader b` for a chord, the key itself for a global binding. The word `leader`
    /// names whatever the leader is; the help overlay prints the leader's own key name.
    pub fn hint_for(&self, action: &str) -> Option<String> {
        if let Some(b) = self
            .bindings
            .iter()
            .find(|b| b.action.to_string() == action)
        {
            return Some(format!("leader {}", b.key));
        }
        self.global
            .iter()
            .find(|b| b.action.to_string() == action)
            .map(|b| b.key.to_string())
    }
}

/// How a key reads in a hint: `Enter` is the glyph, `Esc` is the lower-case word, and
/// everything else prints as it is configured (principle 3).
pub fn hint_key(key: &KeyName) -> String {
    match key.to_string().as_str() {
        "Enter" => "⏎".to_string(),
        "Esc" => "esc".to_string(),
        other => other.to_string(),
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

    /// The two keys M3 adds, through the lookups that answer them: `leader a` opens the
    /// agents overlay, and `Tab` inside a box crosses to the other one.
    #[test]
    fn leader_a_opens_the_agents_overlay_and_tab_crosses_regions_in_a_list() {
        let km = Keymap::defaults();
        assert_eq!(
            km.binding_for(&press(Key::Char('a'), Mods::empty()))
                .unwrap()
                .to_string(),
            "agents.open"
        );
        assert_eq!(
            km.list_for(&press(Key::Tab, Mods::empty()))
                .unwrap()
                .to_string(),
            "focus.next_region"
        );
        assert_eq!(km.hint_for("agents.open").as_deref(), Some("leader a"));
        assert_eq!(km.list_key_for("focus.next_region").as_deref(), Some("Tab"));
    }

    /// A typo in an action used to be accepted in silence: the file loaded, `config.reload`
    /// answered "config reloaded", and the key was found to do nothing only by pressing it.
    /// The whole point of reloading is to learn whether the edit took.
    #[test]
    fn an_action_the_server_cannot_do_is_a_warning_and_the_binding_stays() {
        let mut cfg = crate::config::KeysConfig::default();
        cfg.bindings.insert("v".into(), "config.explode".into());
        let (km, warnings) = Keymap::from_config(&cfg).unwrap();
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].0.contains("\"v\""), "{:?}", warnings[0]);
        assert!(
            warnings[0].0.contains("config.explode"),
            "the warning names the action that is wrong: {:?}",
            warnings[0]
        );
        // Kept, unlike a bad key: the key answers with the reason when pressed rather than
        // falling through to the pane and doing nothing at all.
        assert!(km
            .binding_for(&press(Key::Char('v'), Mods::empty()))
            .is_some());
    }

    /// Arguments are part of the action, so an argument the method cannot take is the same
    /// typo and is read out of the file at the same moment.
    #[test]
    fn an_action_whose_arguments_do_not_fit_is_a_warning_too() {
        for action in ["tab.select", "pane.resize left twice"] {
            let mut cfg = crate::config::KeysConfig::default();
            cfg.bindings.insert("v".into(), action.into());
            let (_, warnings) = Keymap::from_config(&cfg).unwrap();
            assert_eq!(warnings.len(), 1, "{action:?}: {warnings:?}");
        }
    }

    #[test]
    fn the_default_keymap_binds_the_m2_actions_and_the_list_table() {
        let km = Keymap::defaults();
        let by_action = |a: &str| {
            km.bindings
                .iter()
                .find(|b| b.action.to_string() == a)
                .map(|b| b.key.to_string())
        };
        assert_eq!(by_action("switcher.open").as_deref(), Some("s"));
        assert_eq!(by_action("sidebar.toggle").as_deref(), Some("b"));
        assert_eq!(by_action("workspace.rename").as_deref(), Some("N"));
        assert_eq!(by_action("workspace.clear_name").as_deref(), Some("n"));
        let list = |a: &str| {
            km.list
                .iter()
                .find(|b| b.action.to_string() == a)
                .map(|b| b.key.to_string())
        };
        assert_eq!(list("list.down").as_deref(), Some("j"));
        assert_eq!(list("list.activate").as_deref(), Some("Enter"));
        assert_eq!(list("focus.pane").as_deref(), Some("Esc"));
        assert_eq!(list("workspace.rename").as_deref(), Some("n"));
    }

    #[test]
    fn hints_render_the_configured_key_and_not_the_default() {
        let mut cfg = KeysConfig::default();
        cfg.bindings.insert("B".into(), "sidebar.toggle".into());
        cfg.bindings.remove("b");
        cfg.list.insert("o".into(), "list.activate".into());
        cfg.list.remove("Enter");
        let (km, warnings) = Keymap::from_config(&cfg).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(km.hint_for("sidebar.toggle").as_deref(), Some("leader B"));
        assert_eq!(km.list_key_for("list.activate").as_deref(), Some("o"));
        let km = Keymap::defaults();
        assert_eq!(
            km.list_key_for("list.activate").as_deref(),
            Some("⏎"),
            "Enter renders as its glyph"
        );
        assert_eq!(km.list_key_for("focus.pane").as_deref(), Some("esc"));
    }

    /// Names the rule `table()` sorts by, rather than leaving it provable only by deleting
    /// the sort and reading an opaque `left: Some("Down") right: Some("j")`. Two keys bind
    /// `list.down` by default (`j` and `Down`); a third, longer one added here must not win.
    #[test]
    fn the_shorter_key_wins_when_one_action_has_two_keys() {
        let mut cfg = KeysConfig::default();
        cfg.list.insert("PageDown".into(), "list.down".into());
        let (km, warnings) = Keymap::from_config(&cfg).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            km.list_key_for("list.down").as_deref(),
            Some("j"),
            "a hint names the shortest key bound to the action, not the first in map order"
        );
        // Equal lengths break alphabetically, so the answer never depends on map order:
        // `f` and `g` are both one character, and `f` must win every time this runs.
        let mut cfg = KeysConfig::default();
        cfg.list.remove("/");
        cfg.list.insert("g".into(), "list.filter".into());
        cfg.list.insert("f".into(), "list.filter".into());
        let (km, warnings) = Keymap::from_config(&cfg).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(km.list_key_for("list.filter").as_deref(), Some("f"));
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
