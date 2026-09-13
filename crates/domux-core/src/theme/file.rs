//! One theme layer: a theme's own values as its file writes them, before inheritance.

use super::role::Role;
use super::value::ColorValue;
use crate::config::{line_of_key, one_line, position, ConfigWarning};
use serde::Deserialize;
use std::collections::BTreeMap;
use toml::{Spanned, Value};

/// One theme's own values. A role the layer does not set comes from the theme it extends.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ThemeLayer {
    /// The theme this one starts from, as written. Unset means `domux`.
    pub extends: Option<String>,
    pub roles: BTreeMap<Role, ColorValue>,
}

impl ThemeLayer {
    /// Parses a theme file. `file_label` names the file in every message, for example
    /// `themes/ristretto.toml`. A file that is not TOML is an error; anything else wrong drops
    /// only the key it is on, with a warning.
    pub fn parse(
        file_label: &str,
        text: &str,
    ) -> Result<(ThemeLayer, Vec<ConfigWarning>), ConfigWarning> {
        let top: BTreeMap<String, Spanned<Value>> =
            toml::from_str(text).map_err(|e| not_toml(file_label, text, &e))?;
        let line_at = |offset: usize| Some(position(text, offset).0);
        // Each warning keeps the offset it is about, so the reader meets them in file order.
        let mut found: Vec<(usize, String)> = Vec::new();
        let mut warn = |offset: usize, line: Option<usize>, message: String| {
            let message = match line {
                Some(line) => format!("{file_label} line {line}: {message}"),
                None => format!("{file_label}: {message}"),
            };
            found.push((offset, one_line(&message)));
        };
        let mut layer = ThemeLayer::default();
        for (key, value) in &top {
            let offset = value.span().start;
            match key.as_str() {
                "extends" => match value.get_ref() {
                    Value::String(name) => layer.extends = Some(name.clone()),
                    other => warn(
                        offset,
                        line_at(offset),
                        format!("extends {other} is not a theme name: it is a string; extends is ignored"),
                    ),
                },
                "roles" => {
                    if !value.get_ref().is_table() {
                        warn(
                            offset,
                            line_at(offset),
                            "roles is not a table; it is ignored".to_string(),
                        );
                    }
                }
                _ => warn(
                    offset,
                    line_of_key(text, key).or_else(|| line_at(offset)),
                    format!("unknown key {key} is ignored"),
                ),
            }
        }
        if top.get("roles").is_some_and(|v| v.get_ref().is_table()) {
            #[derive(Deserialize)]
            struct Roles {
                roles: BTreeMap<String, Spanned<Value>>,
            }
            let roles: Roles = toml::from_str(text).map_err(|e| not_toml(file_label, text, &e))?;
            for (name, value) in &roles.roles {
                let offset = value.span().start;
                let line = line_at(offset);
                let Some(role) = Role::from_name(name) else {
                    warn(offset, line, format!("unknown role {name} is ignored"));
                    continue;
                };
                let written = match value.get_ref() {
                    Value::String(written) => written,
                    other => {
                        warn(
                            offset,
                            line,
                            format!("roles.{name} {other} is not a colour: a colour is a string; the role is ignored"),
                        );
                        continue;
                    }
                };
                match ColorValue::parse(written) {
                    Ok(ColorValue::Default) if !role.is_ground() => warn(
                        offset,
                        line,
                        format!("roles.{name} {written:?} is not a colour for {name}: only a ground takes default; the role is ignored"),
                    ),
                    Ok(color) => {
                        layer.roles.insert(role, color);
                    }
                    Err(reason) => warn(
                        offset,
                        line,
                        format!("roles.{name} {written:?} is not a colour: {reason}; the role is ignored"),
                    ),
                }
            }
        }
        found.sort_by_key(|(offset, _)| *offset);
        let warnings = found
            .into_iter()
            .map(|(_, message)| ConfigWarning(message))
            .collect();
        Ok((layer, warnings))
    }
}

/// A file that is not TOML: the error with its line, and the theme is not used.
fn not_toml(file_label: &str, text: &str, e: &toml::de::Error) -> ConfigWarning {
    let reason = one_line(e.message());
    ConfigWarning(match e.span() {
        Some(span) => {
            let line = position(text, span.start).0;
            format!("{file_label} line {line}: {reason}; the theme is not used")
        }
        None => format!("{file_label}: {reason}; the theme is not used"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use domux_term::Rgb;

    const LABEL: &str = "themes/ristretto.toml";

    fn warnings(text: &str) -> (ThemeLayer, Vec<String>) {
        let (layer, warnings) = ThemeLayer::parse(LABEL, text).expect("parses");
        (layer, warnings.into_iter().map(|w| w.0).collect())
    }

    #[test]
    fn a_layer_reads_extends_and_its_roles() {
        let (layer, warnings) = warnings(
            "extends = \"terminal\"\n\n[roles]\nhint_key = \"palette 12\"\nfill = \"blend 2/9\"\n",
        );
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(layer.extends.as_deref(), Some("terminal"));
        assert_eq!(
            layer.roles.into_iter().collect::<Vec<_>>(),
            [
                (Role::Fill, ColorValue::Blend { n: 2, d: 9 }),
                (Role::HintKey, ColorValue::Palette(12)),
            ]
        );
    }

    #[test]
    fn an_unknown_role_is_a_warning_with_its_line() {
        let (layer, warnings) = warnings(
            "extends = \"domux\"\n\n[roles]\n# the accent\nacent = \"#f38d70\"\ntext = \"#e6d9db\"\n",
        );
        assert_eq!(
            warnings,
            ["themes/ristretto.toml line 5: unknown role acent is ignored"]
        );
        assert_eq!(
            layer.roles.into_iter().collect::<Vec<_>>(),
            [(
                Role::Text,
                ColorValue::Hex(Rgb {
                    r: 0xe6,
                    g: 0xd9,
                    b: 0xdb
                })
            )]
        );
    }

    #[test]
    fn an_unknown_top_level_key_is_a_warning_with_its_line() {
        let (layer, warnings) = warnings("extends = \"domux\"\nname = \"night\"\n[colours]\n");
        assert_eq!(
            warnings,
            [
                "themes/ristretto.toml line 2: unknown key name is ignored",
                "themes/ristretto.toml line 3: unknown key colours is ignored",
            ]
        );
        assert_eq!(layer.extends.as_deref(), Some("domux"));
        assert!(layer.roles.is_empty());
    }

    #[test]
    fn a_bad_value_drops_that_role_with_its_line_and_keeps_the_rest() {
        let text = "[roles]\naccent = \"#f38d70\"\n\n\nfill = \"blend 10/9\"\nborder = 3\nrule = \"blend 1/9\"\n";
        let (layer, warnings) = warnings(text);
        assert_eq!(
            warnings,
            [
                "themes/ristretto.toml line 5: roles.fill \"blend 10/9\" is not a colour: a blend is 0 to 1; the role is ignored",
                "themes/ristretto.toml line 6: roles.border 3 is not a colour: a colour is a string; the role is ignored",
            ]
        );
        assert_eq!(
            layer.roles.into_iter().collect::<Vec<_>>(),
            [
                (Role::Rule, ColorValue::Blend { n: 1, d: 9 }),
                (
                    Role::Accent,
                    ColorValue::Hex(Rgb {
                        r: 0xf3,
                        g: 0x8d,
                        b: 0x70
                    })
                ),
            ]
        );
    }

    #[test]
    fn default_on_a_role_that_is_not_a_ground_is_a_warning() {
        let text =
            "[roles]\nsidebar_background = \"default\"\nfill = \"default\"\ntext = \"default\"\n";
        let (layer, warnings) = warnings(text);
        assert_eq!(
            warnings,
            ["themes/ristretto.toml line 4: roles.text \"default\" is not a colour for text: only a ground takes default; the role is ignored"]
        );
        assert_eq!(
            layer.roles.into_iter().collect::<Vec<_>>(),
            [
                (Role::Fill, ColorValue::Default),
                (Role::SidebarBackground, ColorValue::Default),
            ]
        );
    }

    #[test]
    fn a_layer_that_is_not_toml_is_an_error_with_its_line() {
        let err = ThemeLayer::parse(LABEL, "extends = \"domux\"\n[roles]\nfill = blend 1/9\n")
            .expect_err("not toml");
        assert!(
            err.0.starts_with("themes/ristretto.toml line 3: "),
            "{}",
            err.0
        );
        assert!(err.0.ends_with("; the theme is not used"), "{}", err.0);
        assert!(!err.0.contains('\n'), "{}", err.0);
    }

    #[test]
    fn an_extends_or_roles_of_the_wrong_type_is_a_warning() {
        let (layer, warnings) = warnings("extends = 3\nroles = \"fill\"\n");
        assert_eq!(
            warnings,
            [
                "themes/ristretto.toml line 1: extends 3 is not a theme name: it is a string; extends is ignored",
                "themes/ristretto.toml line 2: roles is not a table; it is ignored",
            ]
        );
        assert_eq!(layer, ThemeLayer::default());
    }
}
