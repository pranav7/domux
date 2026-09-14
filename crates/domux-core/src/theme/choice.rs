//! Which theme `[theme] name` asks for.

/// The theme a config names. Only `auto`, `domux` and `terminal` are reserved; any other
/// valid name is a file under `themes/`, which is looked for before a built-in of that name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThemeChoice {
    /// The theme `theme::auto` names for the client's desktop.
    Auto,
    /// `domux` or `terminal`.
    Builtin(&'static str),
    /// The name of a theme file, without `.toml`.
    File(String),
}

/// The longest theme name.
pub const MAX_NAME_LEN: usize = 64;

impl ThemeChoice {
    /// The choice a name makes, or `None` when the name is not a theme name: 1 to 64
    /// characters of `a-z`, `0-9`, `-` and `_`, starting with a letter or digit. A name that
    /// fails is never used as a path.
    pub fn parse(name: &str) -> Option<ThemeChoice> {
        match name {
            "auto" => return Some(ThemeChoice::Auto),
            "domux" => return Some(ThemeChoice::Builtin("domux")),
            "terminal" => return Some(ThemeChoice::Builtin("terminal")),
            _ => {}
        }
        let bytes = name.as_bytes();
        let starts_well = bytes
            .first()
            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
        let all_allowed = bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-' || *b == b'_');
        (starts_well && all_allowed && bytes.len() <= MAX_NAME_LEN)
            .then(|| ThemeChoice::File(name.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_auto_domux_and_terminal_are_reserved_names() {
        assert_eq!(ThemeChoice::parse("auto"), Some(ThemeChoice::Auto));
        assert_eq!(
            ThemeChoice::parse("domux"),
            Some(ThemeChoice::Builtin("domux"))
        );
        assert_eq!(
            ThemeChoice::parse("terminal"),
            Some(ThemeChoice::Builtin("terminal"))
        );
        assert_eq!(
            ThemeChoice::parse("ristretto"),
            Some(ThemeChoice::File("ristretto".to_string()))
        );
    }

    #[test]
    fn a_theme_name_is_lowercase_letters_digits_dash_and_underscore_from_a_letter_or_digit() {
        let longest = "a".repeat(64);
        for good in [
            "a",
            "9",
            "catppuccin-mocha",
            "my_theme",
            "2x",
            "a-_0",
            &longest,
        ] {
            assert_eq!(
                ThemeChoice::parse(good),
                Some(ThemeChoice::File(good.to_string())),
                "{good:?}"
            );
        }
        let too_long = "a".repeat(65);
        for bad in [
            "", "../x", "Nord", "-x", "_x", "a b", "a.toml", "a/b", "é", &too_long,
        ] {
            assert_eq!(ThemeChoice::parse(bad), None, "{bad:?}");
        }
    }
}
