//! Terminal emulation for domux panes: the `Emulator` trait and its libghostty-vt
//! implementation.

pub mod emulator;
pub mod ghostty;
pub mod golden;
pub mod key;
pub mod types;

pub use emulator::{wrap_paste, Emulator, EmulatorConfig};
pub use ghostty::GhosttyEmulator;
pub use key::{Key, KeyAction, KeyEvent, Mods};
pub use types::{Attrs, Cell, Color, Cursor, CursorShape, Grid, Rgb, Size};
