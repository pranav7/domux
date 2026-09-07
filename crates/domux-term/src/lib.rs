//! Terminal emulation for domux panes: the `Emulator` trait and its libghostty-vt
//! implementation.

pub mod emulator;
pub mod ghostty;
pub mod golden;
pub mod key;
pub mod types;

pub use emulator::{
    focus_report, osc7_path, wrap_paste, Emulator, EmulatorConfig, Mode, ScrollbackPos,
};
pub use ghostty::GhosttyEmulator;
pub use key::{Key, KeyAction, KeyEvent, Mods};
pub use types::{Attrs, Cell, Color, Cursor, CursorShape, Grid, Rgb, Size};
