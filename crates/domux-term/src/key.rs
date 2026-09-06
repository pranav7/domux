//! Key events as the client decodes them and the emulator encodes them.

use bitflags::bitflags;
use serde::{Deserialize, Serialize};

/// The logical key. `Char` carries the character as typed (Shift+a is `Char('A')` with
/// `Mods::SHIFT`). Function keys are `F(1)` through `F(12)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Key {
    Char(char),
    Enter,
    Tab,
    Backspace,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    F(u8),
}

bitflags! {
    // `Serialize` and `Deserialize` have to be named here even though the impls come from
    // the bitflags `serde` feature: the feature implements them for the hidden inner type
    // and the derive is what forwards the public type to it. The set renders as a string of
    // flag names.
    //
    // A line comment, not a doc comment: this is about the derive, not about the type.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
    pub struct Mods: u8 {
        const SHIFT = 1 << 0;
        const CTRL = 1 << 1;
        const ALT = 1 << 2;
        const SUPER = 1 << 3;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum KeyAction {
    #[default]
    Press,
    Repeat,
    Release,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KeyEvent {
    pub key: Key,
    pub mods: Mods,
    pub action: KeyAction,
}

impl KeyEvent {
    pub fn press(key: Key, mods: Mods) -> Self {
        KeyEvent {
            key,
            mods,
            action: KeyAction::Press,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The attach protocol serializes key events across a socket, so a dropped derive has to
    /// fail here rather than in the crate that later carries them.
    #[test]
    fn the_key_types_carry_serde_derives() {
        fn serde_both<T: Serialize + serde::de::DeserializeOwned>() {}
        serde_both::<Key>();
        serde_both::<Mods>();
        serde_both::<KeyAction>();
        serde_both::<KeyEvent>();
    }
}
