//! Key events as the client decodes them and the emulator encodes them.

use bitflags::bitflags;

/// The logical key. `Char` carries the character as typed (Shift+a is `Char('A')` with
/// `Mods::SHIFT`). Function keys are `F(1)` through `F(12)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct Mods: u8 {
        const SHIFT = 1 << 0;
        const CTRL = 1 << 1;
        const ALT = 1 << 2;
        const SUPER = 1 << 3;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum KeyAction {
    #[default]
    Press,
    Repeat,
    Release,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
