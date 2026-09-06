//! The emulator trait: the seam between domux and a terminal emulation library.

use crate::key::KeyEvent;
use crate::types::{Cursor, Grid, Rgb, Size};
use std::path::PathBuf;

/// A position in the scrollback plus screen. `row` counts from the top of the scrollback:
/// rows `0..scrollback_len()` are history and `scrollback_len()..scrollback_len()+rows` are
/// the visible screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ScrollbackPos {
    pub row: usize,
    pub col: u16,
}

/// The terminal modes domux reads. Only the ones a feature needs are listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    /// DECSET 1049 or 47: the alternate screen is active, so there is no scrollback to walk.
    AltScreen,
    /// DECSET 2004.
    BracketedPaste,
    /// DECSET 1004: the program wants focus in and out reports.
    FocusEvents,
    /// DECSET 1: application cursor keys.
    AppCursor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmulatorConfig {
    pub size: Size,
    /// Lines of scrollback to keep.
    pub scrollback_lines: usize,
    /// Colors the emulator reports when a program queries OSC 10 and OSC 11. The renderer
    /// draws `Color::Default` with the outer terminal's defaults, so M1's client should pass
    /// the outer terminal's real colors here. M0 passes the spike's `--fg` and `--bg`.
    pub default_fg: Rgb,
    pub default_bg: Rgb,
}

/// One pane's terminal state. The implementation is owned by the core task and is `Send` so
/// the task can move between runtime threads; it is not `Sync` and needs no locking.
///
/// The spec names five operations: feed, resize, snapshot, cursor, encode key. Two more
/// follow from them: `take_responses` returns bytes the emulator must send back to the inner
/// program (cursor position reports, device attributes, color queries), and `encode_paste`
/// wraps pasted text according to bracketed paste mode.
pub trait Emulator: Send {
    /// Parses output bytes from the PTY. Any chunking must give the same result.
    fn feed(&mut self, bytes: &[u8]);

    /// Appends bytes the inner program is owed (replies to its queries). The caller writes
    /// them to the PTY after each `feed`.
    fn take_responses(&mut self, out: &mut Vec<u8>);

    fn resize(&mut self, size: Size);

    fn size(&self) -> Size;

    /// Fills `out` with the visible screen, resizing `out` to `self.size()` first.
    fn snapshot_grid(&mut self, out: &mut Grid);

    fn cursor(&self) -> Cursor;

    /// Encodes a key for the mode the inner program requested (application cursor keys,
    /// kitty keyboard protocol flags, and so on). Appends nothing for keys with no encoding.
    fn encode_key(&mut self, key: &KeyEvent, out: &mut Vec<u8>);

    /// Appends pasted text, wrapped in `ESC [ 200 ~` and `ESC [ 201 ~` when the inner
    /// program enabled bracketed paste (mode 2004).
    fn encode_paste(&self, text: &str, out: &mut Vec<u8>);

    /// Lines of history above the visible screen.
    fn scrollback_len(&self) -> usize;

    /// Like `snapshot_grid`, but the top row is `offset_from_bottom` lines above the live
    /// screen's top row. An offset past the oldest line clamps to the oldest line. Offset 0
    /// is `snapshot_grid`.
    fn snapshot_grid_at(&mut self, offset_from_bottom: usize, out: &mut Grid);

    /// The text between two positions, inclusive, rows joined with `\n`, each row's trailing
    /// blanks removed. Wide graphemes appear once. Positions past the end clamp.
    fn text_in_range(&mut self, start: ScrollbackPos, end: ScrollbackPos) -> String;

    /// The title the program set with OSC 0 or OSC 2, if any.
    fn title(&self) -> Option<String>;

    /// The working directory the shell reported with OSC 7, decoded from its file URL.
    fn cwd(&self) -> Option<PathBuf>;

    /// True once if a BEL arrived since the last call.
    fn take_bell(&mut self) -> bool;

    fn mode_active(&self, mode: Mode) -> bool;

    /// Appends the focus in or out report when the program enabled mode 1004; nothing otherwise.
    fn encode_focus(&self, focused: bool, out: &mut Vec<u8>);
}

/// Appends `text` with the bracketed paste markers when `bracketed` is set. Reading mode
/// 2004 is the emulator's job; wrapping the text is the same either way.
pub fn wrap_paste(text: &str, bracketed: bool, out: &mut Vec<u8>) {
    if bracketed {
        out.extend_from_slice(b"\x1b[200~");
    }
    out.extend_from_slice(text.as_bytes());
    if bracketed {
        out.extend_from_slice(b"\x1b[201~");
    }
}

/// Decodes the path of an OSC 7 `file://host/path` URL. Percent escapes are decoded; the
/// host is ignored because domux only ever runs locally.
pub fn osc7_path(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("file://")?;
    let path = &rest[rest.find('/')?..];
    let mut bytes = Vec::with_capacity(path.len());
    let mut it = path.bytes();
    while let Some(b) = it.next() {
        if b == b'%' {
            let hi = it.next()?;
            let lo = it.next()?;
            let hex = [hi, lo];
            let s = std::str::from_utf8(&hex).ok()?;
            bytes.push(u8::from_str_radix(s, 16).ok()?);
        } else {
            bytes.push(b);
        }
    }
    Some(PathBuf::from(String::from_utf8(bytes).ok()?))
}

/// The focus reports of mode 1004, for the path that encodes them by hand.
pub fn focus_report(focused: bool) -> &'static [u8] {
    if focused {
        b"\x1b[I"
    } else {
        b"\x1b[O"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc7_path_decodes_a_file_url_and_ignores_the_host() {
        assert_eq!(
            osc7_path("file://localhost/Users/pranav"),
            Some(PathBuf::from("/Users/pranav"))
        );
        assert_eq!(
            osc7_path("file:///tmp/a%20b"),
            Some(PathBuf::from("/tmp/a b"))
        );
    }

    #[test]
    fn osc7_path_reports_absent_for_anything_that_is_not_a_file_url() {
        // A bare path is what OSC 9 and OSC 1337 report; it is not a file URL.
        assert_eq!(osc7_path("/tmp"), None);
        assert_eq!(osc7_path("file://host-with-no-path"), None);
        // A truncated percent escape decodes to nothing rather than to a guess.
        assert_eq!(osc7_path("file:///tmp/a%2"), None);
    }

    #[test]
    fn focus_report_is_the_mode_1004_pair() {
        assert_eq!(focus_report(true), b"\x1b[I");
        assert_eq!(focus_report(false), b"\x1b[O");
    }
}
