//! A theme's chain: the named theme, the theme it extends, and so on down to `domux`.
//!
//! Names are looked up one way wherever they are written, in `[theme] name` or in `extends`:
//! `auto`, `domux` and `terminal` are always the built-in and never read a file, and any other
//! name is a file under `themes/` first and a built-in second.

use super::choice::ThemeChoice;
use super::file::ThemeLayer;
use super::role::Role;
use super::value::ColorValue;
use crate::config::{one_line, ConfigWarning};
use crate::names::THEMES_DIR_NAME;

/// The most themes a chain holds, the named theme and `domux` included.
pub const MAX_CHAIN: usize = 8;

/// A resolved theme: its layers from the named theme to `domux`, which sets every role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chain {
    layers: Vec<(String, ThemeLayer)>,
}

/// A theme that cannot be used. The warnings `resolve` answers with say why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Unused;

impl Chain {
    /// The themes in the chain, from the named one to `domux`.
    pub fn names(&self) -> Vec<&str> {
        self.layers.iter().map(|(name, _)| name.as_str()).collect()
    }

    /// The layers, from the named theme to `domux`.
    pub fn layers(&self) -> impl Iterator<Item = &ThemeLayer> {
        self.layers.iter().map(|(_, layer)| layer)
    }

    /// True when painting this theme reads the terminal's answers: some role's value, as the
    /// first layer that sets it writes it, is a form other than hex and `default`.
    pub fn reads_terminal(&self) -> bool {
        // A hex or default value always evaluates, so a role whose first value is one of them
        // never reaches a later layer's value.
        Role::ALL.iter().any(|role| {
            self.layers()
                .find_map(|layer| layer.roles.get(role))
                .is_some_and(|value| !matches!(value, ColorValue::Hex(_) | ColorValue::Default))
        })
    }
}

/// Resolves the chain a choice names. `builtins` is each built-in theme's name and text, and
/// `read(name)` reads `themes/<name>.toml`: `Ok(None)` when there is no such file, `Err` with a
/// reason when there is one that cannot be read.
///
/// `auto` names no theme by itself, since each client's desktop picks one when it paints
/// (`Themes::paint`), so it reads no file and answers the root, `domux`.
///
/// The warnings are everything to report, in the order they were found: a layer's own
/// warnings, a file that hides a built-in, and, with `Err(Unused)`, why the theme is not used.
pub fn resolve(
    choice: &ThemeChoice,
    builtins: &[(&str, &str)],
    read: impl Fn(&str) -> Result<Option<String>, String>,
) -> (Result<Chain, Unused>, Vec<ConfigWarning>) {
    let mut warnings = Vec::new();
    let result = walk(choice, builtins, read, &mut warnings);
    (result, warnings)
}

/// The name every chain ends at. It sets every role and extends nothing.
const ROOT: &str = "domux";

/// The theme file a name reads, as messages name it.
fn file_label(name: &str) -> String {
    format!("{THEMES_DIR_NAME}/{name}.toml")
}

fn walk(
    choice: &ThemeChoice,
    builtins: &[(&str, &str)],
    read: impl Fn(&str) -> Result<Option<String>, String>,
    warnings: &mut Vec<ConfigWarning>,
) -> Result<Chain, Unused> {
    let unused = |warnings: &mut Vec<ConfigWarning>, message: String| {
        warnings.push(ConfigWarning(one_line(&format!(
            "{message}; the theme is not used"
        ))));
        Unused
    };
    let mut name = match choice {
        ThemeChoice::Auto => ROOT.to_string(),
        ThemeChoice::Builtin(name) => name.to_string(),
        ThemeChoice::File(name) => name.clone(),
    };
    // Each layer with the label its messages use.
    let mut layers: Vec<(String, String, ThemeLayer)> = Vec::new();
    loop {
        // Where the name was written: the config for the first, the layer before otherwise.
        let written_by = layers.last().map(|(_, label, _)| label.clone());
        if let Some(start) = layers.iter().position(|(n, _, _)| *n == name) {
            let mut after: Vec<&str> = layers[start + 1..]
                .iter()
                .map(|(n, _, _)| n.as_str())
                .collect();
            after.push(&name);
            let message = format!(
                "{}: extends {}",
                layers[start].1,
                after.join(", which extends ")
            );
            return Err(unused(warnings, message));
        }
        if layers.len() == MAX_CHAIN {
            let message = format!(
                "{}: the chain of themes it extends is longer than {MAX_CHAIN}",
                layers[0].1
            );
            return Err(unused(warnings, message));
        }
        // How a message names this name: after what extends it, or as the config's.
        let named = |shown: &str| match &written_by {
            Some(by) => format!("{by}: extends {shown}"),
            None => format!("theme.name {shown}"),
        };
        let reserved = match ThemeChoice::parse(&name) {
            Some(ThemeChoice::File(_)) => false,
            Some(ThemeChoice::Builtin(_)) => true,
            // A config's auto starts at the root, so only an extends gets here.
            Some(ThemeChoice::Auto) => {
                let message = format!("{}, which is not a theme", named(&name));
                return Err(unused(warnings, message));
            }
            None => {
                let message = format!(
                    "{}, which is not a theme name; use lowercase letters, digits, - and _",
                    named(&format!("{name:?}"))
                );
                return Err(unused(warnings, message));
            }
        };
        let builtin = builtins
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, text)| *text);
        let file = if reserved {
            None
        } else {
            match read(&name) {
                Ok(file) => file,
                Err(reason) => {
                    return Err(unused(warnings, format!("{}: {reason}", file_label(&name))))
                }
            }
        };
        let (label, text) = match (file, builtin) {
            (Some(text), builtin) => {
                if builtin.is_some() {
                    warnings.push(ConfigWarning(format!(
                        "{} hides the built-in theme {name}; rename the file to use the built-in",
                        file_label(&name)
                    )));
                }
                (file_label(&name), text)
            }
            (None, Some(text)) => (format!("{name} (built in)"), text.to_string()),
            (None, None) => {
                let missing = if reserved {
                    format!("there is no built-in theme {name}")
                } else {
                    format!("there is no {}", file_label(&name))
                };
                let message = match written_by {
                    None => format!("theme.name {name}: {missing}"),
                    Some(by) => format!("{by}: extends {name}, and {missing}"),
                };
                return Err(unused(warnings, message));
            }
        };
        let layer = match ThemeLayer::parse(&label, &text) {
            Ok((layer, found)) => {
                warnings.extend(found);
                layer
            }
            Err(error) => {
                warnings.push(error);
                return Err(Unused);
            }
        };
        let next = layer.extends.clone();
        let root = reserved && name == ROOT;
        layers.push((name, label, layer));
        if root {
            break;
        }
        name = next.unwrap_or_else(|| ROOT.to_string());
    }
    Ok(Chain {
        layers: layers
            .into_iter()
            .map(|(name, _, layer)| (name, layer))
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::builtin::BUILTIN;
    use domux_term::Rgb;

    fn file(name: &str) -> ThemeChoice {
        ThemeChoice::File(name.to_string())
    }

    /// A `read` over an in-memory `themes/` directory.
    fn files<'a>(
        dir: &'a [(&'a str, &'a str)],
    ) -> impl Fn(&str) -> Result<Option<String>, String> + 'a {
        move |name| {
            Ok(dir
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, text)| text.to_string()))
        }
    }

    fn messages(warnings: &[ConfigWarning]) -> Vec<&str> {
        warnings.iter().map(|w| w.0.as_str()).collect()
    }

    #[test]
    fn a_theme_with_no_extends_extends_domux() {
        let dir = [("mine", "[roles]\naccent = \"#f38d70\"\n")];
        let (chain, warnings) = resolve(&file("mine"), BUILTIN, files(&dir));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(chain.expect("resolves").names(), ["mine", "domux"]);
    }

    #[test]
    fn an_unset_role_comes_from_the_theme_it_extends() {
        let dir = [
            (
                "night",
                "extends = \"base\"\n[roles]\naccent = \"#f38d70\"\n",
            ),
            (
                "base",
                "extends = \"terminal\"\n[roles]\nfill = \"#010203\"\n",
            ),
        ];
        let (chain, warnings) = resolve(&file("night"), BUILTIN, files(&dir));
        assert!(warnings.is_empty(), "{warnings:?}");
        let chain = chain.expect("resolves");
        assert_eq!(chain.names(), ["night", "base", "terminal", "domux"]);
        let first = |role: Role| {
            chain
                .layers()
                .find_map(|layer| layer.roles.get(&role).copied())
        };
        let rgb = |r, g, b| ColorValue::Hex(Rgb { r, g, b });
        assert_eq!(first(Role::Accent), Some(rgb(0xf3, 0x8d, 0x70)));
        assert_eq!(first(Role::Fill), Some(rgb(1, 2, 3)));
        assert_eq!(first(Role::Text), Some(ColorValue::Foreground));
        assert_eq!(first(Role::Claude), Some(rgb(0xde, 0x73, 0x56)));
    }

    #[test]
    fn a_syntax_error_makes_the_theme_unused() {
        let dir = [
            ("night", "extends = \"broken\"\n"),
            ("broken", "[roles]\n\nfill = blend 1/9\n"),
        ];
        let (chain, warnings) = resolve(&file("night"), BUILTIN, files(&dir));
        assert_eq!(chain, Err(Unused));
        let messages = messages(&warnings);
        assert_eq!(messages.len(), 1, "{messages:?}");
        assert!(
            messages[0].starts_with("themes/broken.toml line 3: "),
            "{messages:?}"
        );
        assert!(
            messages[0].ends_with("; the theme is not used"),
            "{messages:?}"
        );
    }

    #[test]
    fn a_cycle_in_extends_makes_the_theme_unused() {
        let dir = [("a", "extends = \"b\"\n"), ("b", "extends = \"a\"\n")];
        let (chain, warnings) = resolve(&file("a"), BUILTIN, files(&dir));
        assert_eq!(chain, Err(Unused));
        assert_eq!(
            messages(&warnings),
            ["themes/a.toml: extends b, which extends a; the theme is not used"]
        );

        // The cycle is named from the theme it returns to, not from the one the config names.
        let dir = [
            ("x", "extends = \"a\"\n"),
            ("a", "extends = \"b\"\n"),
            ("b", "extends = \"c\"\n"),
            ("c", "extends = \"a\"\n"),
        ];
        let (chain, warnings) = resolve(&file("x"), BUILTIN, files(&dir));
        assert_eq!(chain, Err(Unused));
        assert_eq!(
            messages(&warnings),
            ["themes/a.toml: extends b, which extends c, which extends a; the theme is not used"]
        );

        let dir = [("me", "extends = \"me\"\n")];
        let (chain, warnings) = resolve(&file("me"), BUILTIN, files(&dir));
        assert_eq!(chain, Err(Unused));
        assert_eq!(
            messages(&warnings),
            ["themes/me.toml: extends me; the theme is not used"]
        );
    }

    #[test]
    fn a_chain_longer_than_eight_makes_the_theme_unused() {
        // t1 extends t2 and so on; the last file extends domux.
        let texts: Vec<(String, String)> = (1..=8)
            .map(|i| (format!("t{i}"), format!("extends = \"t{}\"\n", i + 1)))
            .collect();
        let with_last = |last: usize| {
            let mut dir: Vec<(&str, &str)> = texts[..last - 1]
                .iter()
                .map(|(n, t)| (n.as_str(), t.as_str()))
                .collect();
            dir.push((texts[last - 1].0.as_str(), "extends = \"domux\"\n"));
            dir
        };

        // Seven files and domux are eight themes.
        let dir = with_last(7);
        let (chain, warnings) = resolve(&file("t1"), BUILTIN, files(&dir));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            chain.expect("eight themes resolve").names().len(),
            MAX_CHAIN
        );

        // Eight files and domux are nine.
        let dir = with_last(8);
        let (chain, warnings) = resolve(&file("t1"), BUILTIN, files(&dir));
        assert_eq!(chain, Err(Unused));
        assert_eq!(
            messages(&warnings),
            ["themes/t1.toml: the chain of themes it extends is longer than 8; the theme is not used"]
        );
    }

    #[test]
    fn a_missing_file_named_by_the_config_warns() {
        let (chain, warnings) = resolve(&file("ristreto"), BUILTIN, files(&[]));
        assert_eq!(chain, Err(Unused));
        assert_eq!(
            messages(&warnings),
            ["theme.name ristreto: there is no themes/ristreto.toml; the theme is not used"]
        );

        let dir = [("night", "extends = \"ristretto\"\n")];
        let (chain, warnings) = resolve(&file("night"), BUILTIN, files(&dir));
        assert_eq!(chain, Err(Unused));
        assert_eq!(
            messages(&warnings),
            ["themes/night.toml: extends ristretto, and there is no themes/ristretto.toml; the theme is not used"]
        );
    }

    #[test]
    fn a_file_that_cannot_be_read_or_an_extends_that_names_no_theme_makes_the_theme_unused() {
        let (chain, warnings) = resolve(&file("night"), BUILTIN, |_| {
            Err("permission denied".to_string())
        });
        assert_eq!(chain, Err(Unused));
        assert_eq!(
            messages(&warnings),
            ["themes/night.toml: permission denied; the theme is not used"]
        );

        let dir = [("night", "extends = \"auto\"\n")];
        let (chain, warnings) = resolve(&file("night"), BUILTIN, files(&dir));
        assert_eq!(chain, Err(Unused));
        assert_eq!(
            messages(&warnings),
            ["themes/night.toml: extends auto, which is not a theme; the theme is not used"]
        );

        // A name that is not a theme name is never used as a path.
        let dir = [("night", "extends = \"../Nord\"\n")];
        let read = |name: &str| {
            assert_eq!(
                name, "night",
                "read a file for a name that is not a theme name"
            );
            files(&dir)(name)
        };
        let (chain, warnings) = resolve(&file("night"), BUILTIN, read);
        assert_eq!(chain, Err(Unused));
        assert_eq!(
            messages(&warnings),
            ["themes/night.toml: extends \"../Nord\", which is not a theme name; use lowercase letters, digits, - and _; the theme is not used"]
        );
    }

    #[test]
    fn a_reserved_name_never_reads_a_file() {
        let read = |name: &str| -> Result<Option<String>, String> {
            panic!("read themes/{name}.toml for a reserved name")
        };
        for (choice, names) in [
            (ThemeChoice::Auto, vec!["domux"]),
            (ThemeChoice::Builtin("domux"), vec!["domux"]),
            (ThemeChoice::Builtin("terminal"), vec!["terminal", "domux"]),
        ] {
            let (chain, warnings) = resolve(&choice, BUILTIN, read);
            assert!(warnings.is_empty(), "{choice:?}: {warnings:?}");
            assert_eq!(chain.expect("resolves").names(), names, "{choice:?}");
        }

        // A file that extends a reserved name reads only itself.
        let read = |name: &str| {
            assert_eq!(name, "mine", "read themes/{name}.toml for a reserved name");
            Ok(Some("extends = \"terminal\"\n".to_string()))
        };
        let (chain, warnings) = resolve(&file("mine"), BUILTIN, read);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            chain.expect("resolves").names(),
            ["mine", "terminal", "domux"]
        );
    }

    #[test]
    fn a_file_named_after_a_later_built_in_wins_with_a_warning() {
        let nord = "[roles]\naccent = \"#88c0d0\"\n";
        let builtins = [BUILTIN, &[("nord", nord)]].concat();
        let accent = |chain: &Chain| {
            chain
                .layers()
                .find_map(|layer| layer.roles.get(&Role::Accent).copied())
        };

        // With no file, nord is the built-in.
        let (chain, warnings) = resolve(&file("nord"), &builtins, files(&[]));
        assert!(warnings.is_empty(), "{warnings:?}");
        let chain = chain.expect("resolves");
        assert_eq!(chain.names(), ["nord", "domux"]);
        assert_eq!(
            accent(&chain),
            Some(ColorValue::Hex(Rgb {
                r: 0x88,
                g: 0xc0,
                b: 0xd0
            }))
        );

        // A file of that name wins, and says which built-in it hides.
        let dir = [("nord", "[roles]\naccent = \"#010203\"\n")];
        let (chain, warnings) = resolve(&file("nord"), &builtins, files(&dir));
        assert_eq!(
            messages(&warnings),
            ["themes/nord.toml hides the built-in theme nord; rename the file to use the built-in"]
        );
        let chain = chain.expect("resolves");
        assert_eq!(chain.names(), ["nord", "domux"]);
        assert_eq!(
            accent(&chain),
            Some(ColorValue::Hex(Rgb { r: 1, g: 2, b: 3 }))
        );

        // extends looks the name up the same way.
        let dir = [
            ("night", "extends = \"nord\"\n"),
            ("nord", "[roles]\naccent = \"#010203\"\n"),
        ];
        let (chain, warnings) = resolve(&file("night"), &builtins, files(&dir));
        assert_eq!(
            messages(&warnings),
            ["themes/nord.toml hides the built-in theme nord; rename the file to use the built-in"]
        );
        assert_eq!(
            accent(&chain.expect("resolves")),
            Some(ColorValue::Hex(Rgb { r: 1, g: 2, b: 3 }))
        );
    }

    #[test]
    fn a_layers_own_warnings_come_with_the_chain() {
        let dir = [("mine", "[roles]\nacent = \"#f38d70\"\n")];
        let (chain, warnings) = resolve(&file("mine"), BUILTIN, files(&dir));
        assert!(chain.is_ok());
        assert_eq!(
            messages(&warnings),
            ["themes/mine.toml line 2: unknown role acent is ignored"]
        );
    }

    #[test]
    fn reads_terminal_is_false_for_domux_and_for_a_hex_file_and_true_for_terminal_and_a_file_with_a_blend(
    ) {
        let chain = |choice: ThemeChoice, dir: &[(&str, &str)]| {
            resolve(&choice, BUILTIN, files(dir)).0.expect("resolves")
        };
        assert!(!chain(ThemeChoice::Builtin("domux"), &[]).reads_terminal());
        assert!(chain(ThemeChoice::Builtin("terminal"), &[]).reads_terminal());
        let hex = [(
            "hex",
            "[roles]\naccent = \"#f38d70\"\nsidebar_background = \"default\"\n",
        )];
        assert!(!chain(file("hex"), &hex).reads_terminal());
        let blend = [("blend", "[roles]\nfill = \"blend 1/9\"\n")];
        assert!(chain(file("blend"), &blend).reads_terminal());

        // A file that writes every role terminal sets in hex paints nothing from the answers.
        let mut text = String::from("extends = \"terminal\"\n[roles]\n");
        for role in Role::ALL {
            text.push_str(&format!("{} = \"#010203\"\n", role.name()));
        }
        let all_hex = [("all", text.as_str())];
        assert!(!chain(file("all"), &all_hex).reads_terminal());
    }
}
