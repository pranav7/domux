//! One pointer event, in cells.
//!
//! domux knows nothing about pixels: the client reports the cell the pointer was on and the
//! server routes that cell to a pane or to the chrome. The mouse protocols count buttons and
//! the wheel in one numbering, so `MouseButton` does too (decision 0014).

use crate::key::Mods;
use serde::{Deserialize, Serialize};

/// Which button an event is about. The wheel is a button in every mouse protocol: it reports
/// as button 64 up and 65 down, so it is a button here rather than a second kind of event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MouseAction {
    Press,
    Release,
    /// The pointer moved with the button held. Motion with no button held is not reported:
    /// nothing in domux reads it, and asking the outer terminal for it is the loudest mode on
    /// the wire (decision 0014).
    Drag,
}

/// A pointer event at one cell. `row` and `col` are relative to whatever the event is about:
/// the client sends them in screen cells and the server rebases them onto the pane it hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MouseEvent {
    pub button: MouseButton,
    pub action: MouseAction,
    pub mods: Mods,
    pub row: u16,
    pub col: u16,
}

impl MouseEvent {
    /// The same event at another cell, for rebasing a screen cell onto a pane's own grid.
    pub fn at(self, row: u16, col: u16) -> MouseEvent {
        MouseEvent { row, col, ..self }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The attach protocol serializes these across a socket, so a dropped derive has to fail
    /// here rather than in the crate that later carries them.
    #[test]
    fn the_mouse_types_carry_serde_derives() {
        fn serde_both<T: Serialize + serde::de::DeserializeOwned>() {}
        serde_both::<MouseButton>();
        serde_both::<MouseAction>();
        serde_both::<MouseEvent>();
    }

    #[test]
    fn at_moves_the_cell_and_keeps_the_rest() {
        let event = MouseEvent {
            button: MouseButton::Left,
            action: MouseAction::Press,
            mods: Mods::SHIFT,
            row: 9,
            col: 4,
        };
        let moved = event.at(2, 3);
        assert_eq!((moved.row, moved.col), (2, 3));
        assert_eq!(moved.button, MouseButton::Left);
        assert_eq!(moved.action, MouseAction::Press);
        assert_eq!(moved.mods, Mods::SHIFT);
    }
}
