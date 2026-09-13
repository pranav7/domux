//! Omarchy's current theme, read from Omarchy's own files: the colours its terminal draws, and
//! whether they changed since the client last looked. Design section 6.4 says which file holds
//! the colours and why, and 6.3 what a theme change does to the files.
//!
//! Nothing here writes a file or runs a program. The session polls and sends; this module only
//! answers.

use domux_core::theme::TerminalColors;
use domux_term::Rgb;
use std::io::ErrorKind;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// Omarchy's current theme state, relative to HOME.
pub const CURRENT_DIR: &str = ".local/state/omarchy/current";
/// The file `omarchy-theme-set` writes the theme's name to, relative to HOME.
pub const THEME_NAME_FILE: &str = ".local/state/omarchy/current/theme.name";
/// The directory `omarchy-theme-set` replaces with each theme it sets, relative to HOME.
pub const THEME_DIR: &str = ".local/state/omarchy/current/theme";

/// `THEME_NAME_FILE` and `THEME_DIR` inside `CURRENT_DIR`.
const NAME_IN_CURRENT: &str = "theme.name";
const THEME_IN_CURRENT: &str = "theme";

const COLORS_TOML: &str = "colors.toml";
const GHOSTTY_CONF: &str = "ghostty.conf";

/// The `colors.toml` key for each palette slot, as Omarchy's terminal templates use them.
const SLOT_KEYS: [&str; 16] = [
    "background",
    "red",
    "green",
    "yellow",
    "blue",
    "magenta",
    "cyan",
    "foreground",
    "muted",
    "bright_red",
    "bright_green",
    "bright_yellow",
    "bright_blue",
    "bright_magenta",
    "bright_cyan",
    "bright_foreground",
];

/// Six hex digits, as a colour.
fn hex6(digits: &str) -> Option<Rgb> {
    if digits.len() != 6 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |i: usize| u8::from_str_radix(&digits[i..i + 2], 16).ok();
    Some(Rgb {
        r: channel(0)?,
        g: channel(2)?,
        b: channel(4)?,
    })
}

/// The background, the foreground and the 16 slots of `colors.toml`, or `None` when the file
/// does not carry every key in `SLOT_KEYS` as `#` and six hex digits.
///
/// The file is read the way `parse_colors_file` in `omarchy-theme-color` reads it, not as
/// TOML, because Omarchy accepts files a TOML parser refuses. Each line is split at its first
/// `=`. The key loses every quote and space, and an empty key, one that starts with `#`, or one
/// with other characters than letters, digits, `_` and `-` is skipped. A value with a quote in
/// it is the text between its first two quotes, and any other value is trimmed. A value with a
/// character Omarchy refuses is skipped, so the line before it stands; otherwise a later line
/// wins. Omarchy reads with `while read`, which never reads a last line that has no newline, so
/// neither does this.
pub fn colors_from_colors_toml(text: &str) -> Option<TerminalColors> {
    let mut values: [Option<&str>; 16] = [None; 16];
    let mut lines: Vec<&str> = text.split('\n').collect();
    // The text after the last newline: empty when the file ends in one, and never read either way.
    lines.pop();
    for line in lines {
        let (key, value) = line.split_once('=').unwrap_or((line, ""));
        let key: String = key
            .chars()
            .filter(|c| !matches!(c, '"' | '\'' | ' '))
            .collect();
        if key.is_empty() || key.starts_with('#') {
            continue;
        }
        if !key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            continue;
        }
        let value = match value.find(['"', '\'']) {
            Some(open) => {
                let rest = &value[open + 1..];
                &rest[..rest.find(['"', '\'']).unwrap_or(rest.len())]
            }
            None => value.trim_matches(|c: char| c.is_ascii_whitespace() || c == '\x0b'),
        };
        let allowed = |c: char| c.is_ascii_alphanumeric() || "#(),._+/% -".contains(c);
        if !value.chars().all(allowed) {
            continue;
        }
        if let Some(slot) = SLOT_KEYS.iter().position(|k| *k == key) {
            values[slot] = Some(value);
        }
    }
    let mut palette = [None; 16];
    for (slot, value) in values.into_iter().enumerate() {
        palette[slot] = Some(hex6(value?.strip_prefix('#')?)?);
    }
    Some(TerminalColors {
        bg: palette[0],
        fg: palette[7],
        palette,
    })
}

/// The background, the foreground and the 16 slots of a Ghostty config: `background`,
/// `foreground`, and `palette = N=COLOR` for N from 0 to 15. Lines are `key = value`, and one
/// that starts with `#` is a comment. A later line wins, as it does in Ghostty, and a colour is
/// six hex digits with or without `#`; a value that is not one sets nothing. The error names the
/// first colour the file does not set.
pub fn colors_from_ghostty_conf(text: &str) -> Result<TerminalColors, String> {
    let colour = |value: &str| {
        let value = value.trim().trim_matches('"');
        hex6(value.strip_prefix('#').unwrap_or(value))
    };
    let mut colors = TerminalColors::default();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "background" => colors.bg = colour(value).or(colors.bg),
            "foreground" => colors.fg = colour(value).or(colors.fg),
            "palette" => {
                let Some((slot, value)) = value.split_once('=') else {
                    continue;
                };
                let slot = slot.trim();
                if !slot.bytes().all(|b| b.is_ascii_digit()) {
                    continue;
                }
                if let (Ok(slot @ 0..=15), Some(rgb)) = (slot.parse::<usize>(), colour(value)) {
                    colors.palette[slot] = Some(rgb);
                }
            }
            _ => {}
        }
    }
    if colors.bg.is_none() {
        return Err(format!("{GHOSTTY_CONF} sets no background"));
    }
    if colors.fg.is_none() {
        return Err(format!("{GHOSTTY_CONF} sets no foreground"));
    }
    if let Some(slot) = colors.palette.iter().position(Option::is_none) {
        return Err(format!("{GHOSTTY_CONF} sets no palette {slot}"));
    }
    Ok(colors)
}

/// The colours of the theme in `theme_dir`: from `colors.toml` when it carries all of them,
/// else from `ghostty.conf`, which Omarchy rendered from `colors.toml` after completing it, or
/// which the theme ships itself. Every error names the directory.
pub fn read_theme(theme_dir: &Path) -> Result<TerminalColors, String> {
    let complete = std::fs::read(theme_dir.join(COLORS_TOML))
        .ok()
        .and_then(|bytes| colors_from_colors_toml(&String::from_utf8_lossy(&bytes)));
    if let Some(colors) = complete {
        return Ok(colors);
    }
    let dir = theme_dir.display();
    match std::fs::read(theme_dir.join(GHOSTTY_CONF)) {
        Ok(bytes) => colors_from_ghostty_conf(&String::from_utf8_lossy(&bytes))
            .map_err(|reason| format!("{dir}: {reason}")),
        Err(err) if err.kind() == ErrorKind::NotFound => Err(format!(
            "{dir} has no complete {COLORS_TOML} and no {GHOSTTY_CONF}"
        )),
        Err(err) => Err(format!("{dir}: could not read {GHOSTTY_CONF}: {err}")),
    }
}

/// Whether the terminal is drawing this theme: it answered a background and a foreground, and
/// both are the theme's. The palette is not compared.
pub fn matches(answers: &TerminalColors, theme: &TerminalColors) -> bool {
    answers.bg.is_some() && answers.fg.is_some() && answers.bg == theme.bg && answers.fg == theme.fg
}

/// One file as a stamp sees it: its inode, modification time and length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    inode: u64,
    mtime: i64,
    mtime_nsec: i64,
    len: u64,
}

impl FileStamp {
    fn of(path: &Path) -> Option<FileStamp> {
        let meta = std::fs::metadata(path).ok()?;
        Some(FileStamp {
            inode: meta.ino(),
            mtime: meta.mtime(),
            mtime_nsec: meta.mtime_nsec(),
            len: meta.len(),
        })
    }
}

/// What the files a theme change touches looked like: `theme.name`, `theme/colors.toml` and
/// `theme/ghostty.conf`, each absent or its inode, time and length. `omarchy-theme-set` copies
/// every file into a new directory, so each theme set changes the stamp, the same theme set
/// again included, and an edit in place changes the time.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Stamp {
    theme_name: Option<FileStamp>,
    colors_toml: Option<FileStamp>,
    ghostty_conf: Option<FileStamp>,
}

impl Stamp {
    /// The stamp of the Omarchy current directory `current`: three `stat` calls.
    pub fn take(current: &Path) -> Stamp {
        let theme = current.join(THEME_IN_CURRENT);
        Stamp {
            theme_name: FileStamp::of(&current.join(NAME_IN_CURRENT)),
            colors_toml: FileStamp::of(&theme.join(COLORS_TOML)),
            ghostty_conf: FileStamp::of(&theme.join(GHOSTTY_CONF)),
        }
    }
}

/// Why a client does not follow Omarchy's theme.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotFollowing {
    /// The theme could not be read; the string says why and names the directory.
    Unreadable(String),
    /// The terminal's answers are not the theme's background and foreground, so this terminal
    /// is not drawing Omarchy's theme.
    NotOmarchysTheme,
}

/// What one look at Omarchy's theme found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Polled {
    /// The stamp is the one from the last look, so nothing was read.
    Unchanged,
    /// The theme was read and gives the colours last sent.
    Same,
    /// The theme was read and gives these colours, which differ from the last sent.
    Changed(TerminalColors),
    /// The theme could not be read; the string says why and names the directory.
    Unreadable(String),
}

/// A client following Omarchy's current theme.
#[derive(Debug)]
pub struct Follow {
    current: PathBuf,
    stamp: Stamp,
}

impl Follow {
    /// Reads Omarchy's current theme under `home` once, and follows it only when the terminal's
    /// `answers` show it is drawing that theme.
    pub fn start(home: &Path, answers: &TerminalColors) -> Result<Follow, NotFollowing> {
        let current = home.join(CURRENT_DIR);
        let stamp = Stamp::take(&current);
        let theme =
            read_theme(&current.join(THEME_IN_CURRENT)).map_err(NotFollowing::Unreadable)?;
        if !matches(answers, &theme) {
            return Err(NotFollowing::NotOmarchysTheme);
        }
        Ok(Follow { current, stamp })
    }

    /// Reads the theme only when the stamp changed since the last look, and keeps the new stamp
    /// whether or not the read works, so a file that stays broken is reported once.
    pub fn poll(&mut self, last_sent: &TerminalColors) -> Polled {
        if Stamp::take(&self.current) == self.stamp {
            return Polled::Unchanged;
        }
        self.read_now(last_sent)
    }

    /// Reads the theme whatever the stamp, and records the stamp. The stamp is taken before the
    /// read, so a change made during the read is read again at the next poll.
    pub fn read_now(&mut self, last_sent: &TerminalColors) -> Polled {
        self.stamp = Stamp::take(&self.current);
        match read_theme(&self.current.join(THEME_IN_CURRENT)) {
            Ok(colors) if colors == *last_sent => Polled::Same,
            Ok(colors) => Polled::Changed(colors),
            Err(reason) => Polled::Unreadable(reason),
        }
    }
}

/// Omarchy theme directories and colours for tests, shared by this module's tests and the
/// session's.
#[cfg(test)]
pub(crate) mod fixture {
    use super::*;
    use std::fs;

    /// `/usr/share/omarchy/themes/ristretto/colors.toml`, word for word.
    pub(crate) const RISTRETTO_COLORS_TOML: &str = r##"mode = "dark"

accent = "#f38d70"
selection = "#403e41"
muted = "#72696a"

background = "#2c2525"
dark_background = "#211b1b"
darker_background = "#181414"
lighter_background = "#3d2f2a"

foreground = "#e6d9db"
dark_foreground = "#72696a"
light_foreground = "#c3b7b8"
bright_foreground = "#e6d9db"

red = "#fd6883"
yellow = "#f9cc6c"
orange = "#fb9a77"
green = "#adda78"
cyan = "#85dacc"
blue = "#f38d70"
magenta = "#a8a9eb"
brown = "#7d4d3b"

bright_red = "#ff8297"
bright_yellow = "#fcd675"
bright_green = "#c8e292"
bright_cyan = "#9bf1e1"
bright_blue = "#f8a788"
bright_magenta = "#bebffd"
"##;

    /// `/usr/share/omarchy/themes/catppuccin/colors.toml`, word for word.
    pub(crate) const CATPPUCCIN_COLORS_TOML: &str = r##"mode = "dark"

accent = "#89b4fa"
selection = "#45475a"
muted = "#585b70"

background = "#1e1e2e"
dark_background = "#161622"
darker_background = "#101019"
lighter_background = "#313244"

foreground = "#cdd6f4"
dark_foreground = "#6c7086"
light_foreground = "#bac2de"
bright_foreground = "#cdd6f4"

red = "#f38ba8"
yellow = "#f9e2af"
orange = "#f6b6ab"
green = "#a6e3a1"
cyan = "#94e2d5"
blue = "#89b4fa"
magenta = "#f5c2e7"
brown = "#7b5b55"

bright_red = "#f38ba8"
bright_yellow = "#f9e2af"
bright_green = "#a6e3a1"
bright_cyan = "#94e2d5"
bright_blue = "#89b4fa"
bright_magenta = "#f5c2e7"
"##;

    pub(crate) fn rgb(hex: u32) -> Rgb {
        Rgb {
            r: (hex >> 16) as u8,
            g: (hex >> 8) as u8,
            b: hex as u8,
        }
    }

    pub(crate) fn colors(bg: u32, fg: u32, palette: [u32; 16]) -> TerminalColors {
        TerminalColors {
            bg: Some(rgb(bg)),
            fg: Some(rgb(fg)),
            palette: palette.map(|slot| Some(rgb(slot))),
        }
    }

    /// Ristretto as `omarchy-themes.json` gives it.
    pub(crate) fn ristretto() -> TerminalColors {
        colors(
            0x2c2525,
            0xe6d9db,
            [
                0x2c2525, 0xfd6883, 0xadda78, 0xf9cc6c, 0xf38d70, 0xa8a9eb, 0x85dacc, 0xe6d9db,
                0x72696a, 0xff8297, 0xc8e292, 0xfcd675, 0xf8a788, 0xbebffd, 0x9bf1e1, 0xe6d9db,
            ],
        )
    }

    /// Catppuccin (Mocha) as `omarchy-themes.json` gives it.
    pub(crate) fn catppuccin() -> TerminalColors {
        colors(
            0x1e1e2e,
            0xcdd6f4,
            [
                0x1e1e2e, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xcdd6f4,
                0x585b70, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xcdd6f4,
            ],
        )
    }

    /// A HOME with Omarchy's current theme laid out as `omarchy-theme-set` leaves it.
    pub(crate) fn omarchy_home(name: &str, files: &[(&str, &str)]) -> tempfile::TempDir {
        let home = tempfile::tempdir().unwrap();
        let theme = home.path().join(THEME_DIR);
        fs::create_dir_all(&theme).unwrap();
        for (file, text) in files {
            fs::write(theme.join(file), text).unwrap();
        }
        fs::write(home.path().join(THEME_NAME_FILE), format!("{name}\n")).unwrap();
        home
    }

    /// Sets a theme the way `omarchy-theme-set` does: the new directory is built beside the old
    /// one, the old one is removed, the new one moved in, and the name written.
    pub(crate) fn set_theme(home: &Path, name: &str, files: &[(&str, &str)]) {
        let current = home.join(CURRENT_DIR);
        let next = current.join("next-theme");
        fs::create_dir_all(&next).unwrap();
        for (file, text) in files {
            fs::write(next.join(file), text).unwrap();
        }
        fs::remove_dir_all(current.join("theme")).unwrap();
        fs::rename(&next, current.join("theme")).unwrap();
        fs::write(home.join(THEME_NAME_FILE), format!("{name}\n")).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::*;
    use super::*;
    use std::fs;
    use std::time::{Duration, SystemTime};

    /// A `ghostty.conf` rendered the way `ghostty.conf.tpl` renders one.
    fn ghostty_conf(theme: &TerminalColors) -> String {
        let hex = |c: Option<Rgb>| {
            let c = c.expect("a rendered theme has every colour");
            format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
        };
        let mut text = format!(
            "background = {}\nforeground = {}\ncursor-color = {}\n\n",
            hex(theme.bg),
            hex(theme.fg),
            hex(theme.palette[15])
        );
        for (n, slot) in theme.palette.iter().enumerate() {
            text.push_str(&format!("palette = {n}={}\n", hex(*slot)));
        }
        text
    }

    /// Kernel file times are coarse, so a test that needs a new time sets one.
    fn set_mtime(path: &Path, secs: u64) {
        let file = fs::OpenOptions::new().write(true).open(path).unwrap();
        file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
            .unwrap();
    }

    #[test]
    fn the_paths_are_the_current_directory_and_the_two_names_in_it() {
        assert_eq!(
            Path::new(CURRENT_DIR).join(NAME_IN_CURRENT),
            Path::new(THEME_NAME_FILE)
        );
        assert_eq!(
            Path::new(CURRENT_DIR).join(THEME_IN_CURRENT),
            Path::new(THEME_DIR)
        );
    }

    #[test]
    fn colors_toml_with_every_key_gives_the_background_the_foreground_and_16_slots() {
        let theme = colors_from_colors_toml(RISTRETTO_COLORS_TOML).expect("complete");
        assert_eq!(theme, ristretto());
        assert_eq!(theme.palette[4], Some(rgb(0xf38d70)));
        assert_eq!(
            colors_from_colors_toml(CATPPUCCIN_COLORS_TOML),
            Some(catppuccin())
        );
    }

    #[test]
    fn colors_toml_missing_a_key_or_with_a_value_that_is_not_six_hex_digits_is_incomplete() {
        for key in SLOT_KEYS {
            let without: String = RISTRETTO_COLORS_TOML
                .lines()
                .filter(|line| !line.starts_with(&format!("{key} =")))
                .map(|line| format!("{line}\n"))
                .collect();
            assert_eq!(colors_from_colors_toml(&without), None, "without {key}");
        }
        for bad in ["#f38d7", "#f38d700", "f38d70", "#f38d7g", "rgb(1,2,3)", ""] {
            let text = format!("{RISTRETTO_COLORS_TOML}blue = \"{bad}\"\n");
            assert_eq!(colors_from_colors_toml(&text), None, "blue = {bad:?}");
        }
        assert_eq!(colors_from_colors_toml(""), None);
    }

    #[test]
    fn colors_toml_is_read_the_way_omarchy_reads_it() {
        // Spaces around and inside a quoted key, single quotes, an unquoted value, a comment
        // line, and a quoted value with a comment after it, as `parse_colors_file` reads them.
        let edited = RISTRETTO_COLORS_TOML
            .replace(
                "background = \"#2c2525\"",
                "  \" back ground\" = '#123456' # the ground",
            )
            .replacen("foreground = \"#e6d9db\"", "foreground=#abcdef   ", 1)
            .replace(
                "blue = \"#f38d70\"",
                "# blue = \"#000000\"\nblue = \"#010203\"",
            )
            .replace("red = \"#fd6883\"", "red = \"#0a0b0c\" # a red");
        let theme = colors_from_colors_toml(&edited).expect("complete");
        assert_eq!(theme.bg, Some(rgb(0x123456)));
        assert_eq!(theme.palette[0], Some(rgb(0x123456)));
        assert_eq!(theme.fg, Some(rgb(0xabcdef)));
        assert_eq!(theme.palette[7], Some(rgb(0xabcdef)));
        assert_eq!(theme.palette[4], Some(rgb(0x010203)));
        assert_eq!(theme.palette[1], Some(rgb(0x0a0b0c)));

        // A later line wins, and a value with characters Omarchy refuses is skipped, so the
        // line before it stands.
        let later = format!("{RISTRETTO_COLORS_TOML}green = \"#111111\"\ngreen = \"#2;2\"\n");
        assert_eq!(
            colors_from_colors_toml(&later).unwrap().palette[2],
            Some(rgb(0x111111))
        );
        // Upper-case hex digits are hex digits.
        let upper = format!("{RISTRETTO_COLORS_TOML}cyan = \"#ABCDEF\"\n");
        assert_eq!(
            colors_from_colors_toml(&upper).unwrap().palette[6],
            Some(rgb(0xabcdef))
        );
        // `while read` never reads a last line with no newline, so Omarchy does not see it.
        let unterminated = format!("{RISTRETTO_COLORS_TOML}yellow = \"#222222\"");
        assert_eq!(
            colors_from_colors_toml(&unterminated).unwrap().palette[3],
            Some(rgb(0xf9cc6c))
        );
        let last_key_unterminated = RISTRETTO_COLORS_TOML.trim_end();
        assert_eq!(colors_from_colors_toml(last_key_unterminated), None);
    }

    #[test]
    fn ghostty_conf_gives_the_background_the_foreground_and_16_slots_and_a_later_line_wins() {
        let text = ghostty_conf(&ristretto());
        assert_eq!(colors_from_ghostty_conf(&text), Ok(ristretto()));

        let later = format!(
            "{text}# a comment = #000000\nbackground = #101010\npalette = 4=#202020\n  foreground   =   #303030  \n"
        );
        let theme = colors_from_ghostty_conf(&later).unwrap();
        assert_eq!(theme.bg, Some(rgb(0x101010)));
        assert_eq!(theme.fg, Some(rgb(0x303030)));
        assert_eq!(theme.palette[4], Some(rgb(0x202020)));
        // Slot 0 and slot 7 are what their own lines say, not the background and foreground.
        assert_eq!(theme.palette[0], Some(rgb(0x2c2525)));
        assert_eq!(theme.palette[7], Some(rgb(0xe6d9db)));
    }

    #[test]
    fn ghostty_conf_reads_a_colour_with_or_without_its_hash() {
        let text = ghostty_conf(&catppuccin())
            .replace("background = #1e1e2e", "background = 1e1e2e")
            .replace("palette = 12=#89b4fa", "palette = 12=89B4FA");
        assert_eq!(colors_from_ghostty_conf(&text), Ok(catppuccin()));
    }

    #[test]
    fn ghostty_conf_missing_a_slot_is_an_error_that_names_it() {
        let text = ghostty_conf(&ristretto());
        let without = |drop: &str| -> String {
            text.lines()
                .filter(|line| !line.starts_with(drop))
                .map(|line| format!("{line}\n"))
                .collect()
        };
        assert_eq!(
            colors_from_ghostty_conf(&without("palette = 3=")),
            Err("ghostty.conf sets no palette 3".to_string())
        );
        assert_eq!(
            colors_from_ghostty_conf(&without("palette = 1")),
            Err("ghostty.conf sets no palette 1".to_string())
        );
        assert_eq!(
            colors_from_ghostty_conf(&without("background")),
            Err("ghostty.conf sets no background".to_string())
        );
        assert_eq!(
            colors_from_ghostty_conf(&without("foreground")),
            Err("ghostty.conf sets no foreground".to_string())
        );
        // A colour that is not six hex digits sets nothing.
        let bad = text.replace("palette = 9=#ff8297", "palette = 9=#ff829");
        assert_eq!(
            colors_from_ghostty_conf(&bad),
            Err("ghostty.conf sets no palette 9".to_string())
        );
        assert_eq!(
            colors_from_ghostty_conf(""),
            Err("ghostty.conf sets no background".to_string())
        );
    }

    #[test]
    fn the_theme_is_read_from_colors_toml_when_it_is_complete_and_from_ghostty_conf_otherwise() {
        let conf = ghostty_conf(&catppuccin());
        let both = omarchy_home(
            "ristretto",
            &[
                ("colors.toml", RISTRETTO_COLORS_TOML),
                ("ghostty.conf", &conf),
            ],
        );
        assert_eq!(read_theme(&both.path().join(THEME_DIR)), Ok(ristretto()));

        let incomplete = RISTRETTO_COLORS_TOML.replace("bright_cyan = \"#9bf1e1\"\n", "");
        let fallback = omarchy_home(
            "ristretto",
            &[("colors.toml", &incomplete), ("ghostty.conf", &conf)],
        );
        assert_eq!(
            read_theme(&fallback.path().join(THEME_DIR)),
            Ok(catppuccin())
        );

        let only_conf = omarchy_home("catppuccin", &[("ghostty.conf", &conf)]);
        assert_eq!(
            read_theme(&only_conf.path().join(THEME_DIR)),
            Ok(catppuccin())
        );

        let broken_conf = omarchy_home(
            "ristretto",
            &[
                ("colors.toml", &incomplete),
                ("ghostty.conf", "background = #000000\n"),
            ],
        );
        let dir = broken_conf.path().join(THEME_DIR);
        assert_eq!(
            read_theme(&dir),
            Err(format!(
                "{}: ghostty.conf sets no foreground",
                dir.display()
            ))
        );
    }

    #[test]
    fn a_theme_directory_with_neither_file_is_an_error_that_names_the_directory() {
        let home = omarchy_home("empty", &[("icons.theme", "Yaru\n")]);
        let dir = home.path().join(THEME_DIR);
        let err = read_theme(&dir).unwrap_err();
        assert!(err.contains(&dir.display().to_string()), "{err}");
        assert_eq!(
            err,
            format!(
                "{} has no complete colors.toml and no ghostty.conf",
                dir.display()
            )
        );

        let missing = home.path().join("nowhere");
        let err = read_theme(&missing).unwrap_err();
        assert!(err.contains(&missing.display().to_string()), "{err}");

        let incomplete = omarchy_home(
            "incomplete",
            &[("colors.toml", "background = \"#000000\"\n")],
        );
        let dir = incomplete.path().join(THEME_DIR);
        assert_eq!(
            read_theme(&dir),
            Err(format!(
                "{} has no complete colors.toml and no ghostty.conf",
                dir.display()
            ))
        );
    }

    #[test]
    fn matches_needs_the_background_and_the_foreground_answered_and_equal_to_the_file() {
        let theme = ristretto();
        assert!(matches(&theme, &theme));

        // The palette is not part of the check.
        let only_grounds = TerminalColors {
            bg: theme.bg,
            fg: theme.fg,
            palette: [None; 16],
        };
        assert!(matches(&only_grounds, &theme));

        let no_bg = TerminalColors {
            bg: None,
            ..theme.clone()
        };
        assert!(!matches(&no_bg, &theme));
        let no_fg = TerminalColors {
            fg: None,
            ..theme.clone()
        };
        assert!(!matches(&no_fg, &theme));
        let other_bg = TerminalColors {
            bg: Some(rgb(0x2c2526)),
            ..theme.clone()
        };
        assert!(!matches(&other_bg, &theme));
        let other_fg = TerminalColors {
            fg: Some(rgb(0xe6d9dc)),
            ..theme.clone()
        };
        assert!(!matches(&other_fg, &theme));
        assert!(!matches(&TerminalColors::default(), &theme));
        assert!(!matches(&catppuccin(), &theme));
    }

    #[test]
    fn the_stamp_changes_when_the_directory_is_replaced_and_when_theme_name_is_written_again_with_the_same_name(
    ) {
        let home = omarchy_home("ristretto", &[("colors.toml", RISTRETTO_COLORS_TOML)]);
        let current = home.path().join(CURRENT_DIR);
        let first = Stamp::take(&current);
        assert_eq!(Stamp::take(&current), first, "nothing changed");

        // The same theme set again: the same bytes, in new files.
        set_theme(
            home.path(),
            "ristretto",
            &[("colors.toml", RISTRETTO_COLORS_TOML)],
        );
        let name = home.path().join(THEME_NAME_FILE);
        set_mtime(&name, 1_000);
        set_mtime(&home.path().join(THEME_DIR).join("colors.toml"), 1_000);
        let replaced = Stamp::take(&current);
        assert_ne!(replaced, first, "the directory was replaced");

        // theme.name written again in place, with the same name and so the same length.
        fs::write(&name, "ristretto\n").unwrap();
        set_mtime(&name, 2_000);
        let rewritten = Stamp::take(&current);
        assert_ne!(rewritten, replaced, "theme.name was written again");

        // ghostty.conf appearing changes it too.
        fs::write(home.path().join(THEME_DIR).join("ghostty.conf"), "").unwrap();
        assert_ne!(Stamp::take(&current), rewritten, "ghostty.conf appeared");

        // A current directory that is not there is a stamp too, and a different one.
        let gone = Stamp::take(&home.path().join("nowhere"));
        assert_eq!(gone, Stamp::default());
        assert_ne!(gone, rewritten);
    }

    #[test]
    fn poll_reads_nothing_while_the_stamp_is_unchanged() {
        let home = omarchy_home("ristretto", &[("colors.toml", RISTRETTO_COLORS_TOML)]);
        let mut follow = Follow::start(home.path(), &ristretto()).expect("follows");
        assert_eq!(follow.poll(&ristretto()), Polled::Unchanged);
        assert_eq!(follow.poll(&ristretto()), Polled::Unchanged);

        // Garbage of the same length with the time put back leaves the stamp as it was, so a
        // poll that read the file would say it is unreadable.
        let file = home.path().join(THEME_DIR).join("colors.toml");
        let mtime = fs::metadata(&file).unwrap().modified().unwrap();
        fs::write(&file, "x".repeat(RISTRETTO_COLORS_TOML.len())).unwrap();
        let handle = fs::OpenOptions::new().write(true).open(&file).unwrap();
        handle.set_modified(mtime).unwrap();
        drop(handle);
        assert_eq!(follow.poll(&ristretto()), Polled::Unchanged);
    }

    #[test]
    fn poll_gives_the_new_colours_once_after_a_theme_change_and_same_when_they_equal_the_last_sent()
    {
        let home = omarchy_home("ristretto", &[("colors.toml", RISTRETTO_COLORS_TOML)]);
        let mut follow = Follow::start(home.path(), &ristretto()).expect("follows");
        let mut last_sent = ristretto();

        set_theme(
            home.path(),
            "catppuccin",
            &[("colors.toml", CATPPUCCIN_COLORS_TOML)],
        );
        assert_eq!(follow.poll(&last_sent), Polled::Changed(catppuccin()));
        last_sent = catppuccin();
        assert_eq!(follow.poll(&last_sent), Polled::Unchanged);

        // Catppuccin set again: new files, the same colours.
        set_theme(
            home.path(),
            "catppuccin",
            &[("colors.toml", CATPPUCCIN_COLORS_TOML)],
        );
        assert_eq!(follow.poll(&last_sent), Polled::Same);
        assert_eq!(follow.poll(&last_sent), Polled::Unchanged);

        // Colours that differ from the last sent only in the palette are a change.
        let partial = TerminalColors {
            palette: [None; 16],
            ..catppuccin()
        };
        set_theme(
            home.path(),
            "catppuccin",
            &[("colors.toml", CATPPUCCIN_COLORS_TOML)],
        );
        assert_eq!(follow.poll(&partial), Polled::Changed(catppuccin()));
    }

    #[test]
    fn a_file_that_cannot_be_parsed_is_unreadable_once_and_unchanged_until_it_changes() {
        let home = omarchy_home("ristretto", &[("colors.toml", RISTRETTO_COLORS_TOML)]);
        let mut follow = Follow::start(home.path(), &ristretto()).expect("follows");
        let last_sent = ristretto();

        set_theme(
            home.path(),
            "broken",
            &[("colors.toml", "background = \"#000000\"\n")],
        );
        let dir = home.path().join(THEME_DIR);
        match follow.poll(&last_sent) {
            Polled::Unreadable(reason) => {
                assert!(reason.contains(&dir.display().to_string()), "{reason}")
            }
            other => panic!("expected unreadable, got {other:?}"),
        }
        assert_eq!(follow.poll(&last_sent), Polled::Unchanged);
        assert_eq!(follow.poll(&last_sent), Polled::Unchanged);

        // A directory removed between polls is unreadable once too.
        fs::remove_dir_all(&dir).unwrap();
        assert!(matches!(follow.poll(&last_sent), Polled::Unreadable(_)));
        assert_eq!(follow.poll(&last_sent), Polled::Unchanged);

        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("colors.toml"), CATPPUCCIN_COLORS_TOML).unwrap();
        assert_eq!(follow.poll(&last_sent), Polled::Changed(catppuccin()));
    }

    #[test]
    fn read_now_reads_whatever_the_stamp() {
        let home = omarchy_home("ristretto", &[("colors.toml", RISTRETTO_COLORS_TOML)]);
        let mut follow = Follow::start(home.path(), &ristretto()).expect("follows");

        assert_eq!(follow.read_now(&ristretto()), Polled::Same);
        assert_eq!(follow.read_now(&catppuccin()), Polled::Changed(ristretto()));

        // read_now records the stamp it read under, so a poll after it reads nothing.
        set_theme(
            home.path(),
            "catppuccin",
            &[("colors.toml", CATPPUCCIN_COLORS_TOML)],
        );
        assert_eq!(follow.read_now(&ristretto()), Polled::Changed(catppuccin()));
        assert_eq!(follow.poll(&ristretto()), Polled::Unchanged);

        fs::remove_dir_all(home.path().join(THEME_DIR)).unwrap();
        assert!(matches!(
            follow.read_now(&catppuccin()),
            Polled::Unreadable(_)
        ));
        assert!(matches!(
            follow.read_now(&catppuccin()),
            Polled::Unreadable(_)
        ));
    }

    #[test]
    fn follow_start_is_refused_when_the_terminal_answers_are_not_omarchys_theme() {
        let home = omarchy_home("ristretto", &[("colors.toml", RISTRETTO_COLORS_TOML)]);
        assert_eq!(
            Follow::start(home.path(), &catppuccin()).unwrap_err(),
            NotFollowing::NotOmarchysTheme
        );
        assert_eq!(
            Follow::start(home.path(), &TerminalColors::default()).unwrap_err(),
            NotFollowing::NotOmarchysTheme
        );
        assert!(Follow::start(home.path(), &ristretto()).is_ok());
    }

    #[test]
    fn follow_start_is_refused_when_the_theme_cannot_be_read() {
        let nothing = tempfile::tempdir().unwrap();
        match Follow::start(nothing.path(), &ristretto()) {
            Err(NotFollowing::Unreadable(reason)) => assert!(
                reason.contains(&nothing.path().join(THEME_DIR).display().to_string()),
                "{reason}"
            ),
            other => panic!("expected unreadable, got {other:?}"),
        }

        let broken = omarchy_home("broken", &[("ghostty.conf", "background = #2c2525\n")]);
        assert!(matches!(
            Follow::start(broken.path(), &ristretto()),
            Err(NotFollowing::Unreadable(_))
        ));
    }
}
