//! The emulator trait: the seam between domux and a terminal emulation library.

use crate::key::KeyEvent;
use crate::types::{Cursor, Grid, Rgb, Size};

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
