//! Converts crossterm events into domux key events. The pane owns the key from here on
//! (principle 1): the client reads it and sends it, and never acts on it itself.

use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyEventKind, KeyModifiers};
use domux_term::{Key, KeyAction, KeyEvent, Mods};

/// The domux key for a crossterm key, or `None` for a key domux has no name for. A key with
/// no name is dropped rather than sent as something else.
pub fn to_key_event(k: &CtKey) -> Option<KeyEvent> {
    let key = match k.code {
        KeyCode::Char(c) => Key::Char(c),
        KeyCode::Enter => Key::Enter,
        KeyCode::Tab => Key::Tab,
        KeyCode::BackTab => Key::Tab,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Esc => Key::Escape,
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::Left => Key::Left,
        KeyCode::Right => Key::Right,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        KeyCode::PageUp => Key::PageUp,
        KeyCode::PageDown => Key::PageDown,
        KeyCode::Insert => Key::Insert,
        KeyCode::Delete => Key::Delete,
        KeyCode::F(n) => Key::F(n),
        _ => return None,
    };
    let mut mods = Mods::empty();
    if k.modifiers.contains(KeyModifiers::SHIFT) || matches!(k.code, KeyCode::BackTab) {
        mods |= Mods::SHIFT;
    }
    if k.modifiers.contains(KeyModifiers::CONTROL) {
        mods |= Mods::CTRL;
    }
    if k.modifiers.contains(KeyModifiers::ALT) {
        mods |= Mods::ALT;
    }
    if k.modifiers.contains(KeyModifiers::SUPER) {
        mods |= Mods::SUPER;
    }
    let action = match k.kind {
        KeyEventKind::Press => KeyAction::Press,
        KeyEventKind::Repeat => KeyAction::Repeat,
        KeyEventKind::Release => KeyAction::Release,
    };
    Some(KeyEvent { key, mods, action })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyEventKind, KeyEventState, KeyModifiers};

    #[test]
    fn crossterm_keys_map_to_domux_keys_with_modifiers() {
        let ev = CtKey {
            code: KeyCode::Char('c'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };
        assert_eq!(
            to_key_event(&ev),
            Some(KeyEvent::press(Key::Char('c'), Mods::CTRL))
        );
        let ev = CtKey {
            code: KeyCode::BackTab,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };
        assert_eq!(
            to_key_event(&ev),
            Some(KeyEvent::press(Key::Tab, Mods::SHIFT))
        );
        let ev = CtKey {
            code: KeyCode::Null,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };
        assert_eq!(to_key_event(&ev), None);
    }
}
