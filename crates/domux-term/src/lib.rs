//! Terminal emulation for domux panes: the `Emulator` trait and its implementations.

pub mod emulator;
pub mod golden;
pub mod key;
pub mod types;

#[cfg(feature = "alacritty")]
pub mod alacritty;
#[cfg(feature = "ghostty")]
pub mod ghostty;

pub use emulator::{Emulator, EmulatorConfig, EmulatorKind};
pub use key::{Key, KeyAction, KeyEvent, Mods};
pub use types::{Attrs, Cell, Color, Cursor, CursorShape, Grid, Rgb, Size};

/// Builds an emulator of the requested kind. Returns an error when the kind's feature is
/// off so callers can report "not built with ghostty" instead of panicking.
pub fn new_emulator(
    kind: EmulatorKind,
    config: EmulatorConfig,
) -> Result<Box<dyn Emulator>, String> {
    match kind {
        #[cfg(feature = "ghostty")]
        EmulatorKind::Ghostty => Ok(Box::new(ghostty::GhosttyEmulator::new(config)?)),
        #[cfg(feature = "alacritty")]
        EmulatorKind::Alacritty => Ok(Box::new(alacritty::AlacrittyEmulator::new(config))),
        #[allow(unreachable_patterns)]
        other => {
            let _ = config;
            Err(format!("{other:?} is not enabled in this build"))
        }
    }
}
