//! Terminal emulation for domux panes: the `Emulator` trait and its implementations.

pub mod emulator;
pub mod key;
pub mod types;

pub use emulator::{Emulator, EmulatorConfig, EmulatorKind};
pub use key::{Key, KeyAction, KeyEvent, Mods};
pub use types::{Attrs, Cell, Color, Cursor, CursorShape, Grid, Rgb, Size};

/// Builds an emulator of the requested kind. Returns an error when the kind's feature is
/// off so callers can report "not built with ghostty" instead of panicking. The `ghostty`
/// and `alacritty` arms arrive with their modules.
pub fn new_emulator(
    kind: EmulatorKind,
    _config: EmulatorConfig,
) -> Result<Box<dyn Emulator>, String> {
    Err(format!("{kind:?} is not enabled in this build"))
}
