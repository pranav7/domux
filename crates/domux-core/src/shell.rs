//! How a path is written into a shell command a person reads, pastes or edits.

/// A full path as one word a POSIX shell reads back as the same path: written bare when
/// nothing in it is special to the shell, and single quoted otherwise (V1's
/// `shellCommandPath`, commit 34db116).
///
/// Bare when it can be, because the commands it goes into are read by people: a hook line in
/// `settings.json` or `hooks.json`, files a person edits by hand, and the command attach names
/// after a no, which a person pastes. A plain path stays plain and only one that would not
/// survive the shell gains quotes.
///
/// It is for full paths. `~` and `#` are special only at the start of a word, and a full path
/// starts with `/`, so neither needs quoting here.
pub fn word(path: &str) -> String {
    if path.contains(|c: char| " \t\n'\"\\$`!*?[]{}()<>|&;".contains(c)) {
        format!("'{}'", path.replace('\'', "'\\''"))
    } else {
        path.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_with_nothing_special_in_it_is_written_bare() {
        for path in [
            "/Users/a/bin/domux",
            "/home/u/code/audrey-app-2",
            "/tmp/x.y_z,1+2@3:4%5",
            // Special only at the start of a word, and a full path starts with `/`.
            "/home/u/a~b/c#d=e",
        ] {
            assert_eq!(word(path), path);
        }
    }

    #[test]
    fn a_path_a_shell_would_split_or_expand_is_single_quoted() {
        assert_eq!(word("/Users/a/my bin/domux"), "'/Users/a/my bin/domux'");
        assert_eq!(word("/home/u/$HOME"), "'/home/u/$HOME'");
        assert_eq!(word("/tmp/a*b"), "'/tmp/a*b'");
        assert_eq!(word("/tmp/a\tb"), "'/tmp/a\tb'");
        assert_eq!(word("/tmp/(x)"), "'/tmp/(x)'");
        assert_eq!(word("/tmp/a;b&c|d"), "'/tmp/a;b&c|d'");
    }

    /// A single quote cannot appear inside single quotes, so it closes them, is escaped, and
    /// opens them again.
    #[test]
    fn a_single_quote_inside_is_closed_escaped_and_reopened() {
        assert_eq!(word("/home/u/it's"), "'/home/u/it'\\''s'");
    }
}
