//! Converts crossterm events into domux key events and handles the spike's C-] chord.

use crossterm::event::{KeyCode, KeyEvent as CtKey, KeyEventKind, KeyModifiers};
use domux_term::{Key, KeyAction, KeyEvent, Mods};

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

/// What the loop should do with a key while the chord state machine runs.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    Forward(KeyEvent),
    NextPane,
    Quit,
    Nothing,
}

#[derive(Default)]
pub struct Chord {
    pending: bool,
}

impl Chord {
    pub fn is_prefix(k: &KeyEvent) -> bool {
        k.key == Key::Char(']') && k.mods == Mods::CTRL && k.action != KeyAction::Release
    }

    pub fn handle(&mut self, k: KeyEvent) -> Action {
        if self.pending {
            self.pending = false;
            return match (k.key, k.mods) {
                (Key::Char('n'), m) if m.is_empty() => Action::NextPane,
                (Key::Char('q'), m) if m.is_empty() => Action::Quit,
                _ if Self::is_prefix(&k) => Action::Forward(k),
                _ => Action::Nothing,
            };
        }
        if Self::is_prefix(&k) {
            self.pending = true;
            return Action::Nothing;
        }
        Action::Forward(k)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chord_next_quit_and_literal_prefix() {
        let mut c = Chord::default();
        let prefix = KeyEvent::press(Key::Char(']'), Mods::CTRL);
        assert_eq!(c.handle(prefix), Action::Nothing);
        assert_eq!(
            c.handle(KeyEvent::press(Key::Char('n'), Mods::empty())),
            Action::NextPane
        );
        assert_eq!(c.handle(prefix), Action::Nothing);
        assert_eq!(
            c.handle(KeyEvent::press(Key::Char('q'), Mods::empty())),
            Action::Quit
        );
        assert_eq!(c.handle(prefix), Action::Nothing);
        assert_eq!(c.handle(prefix), Action::Forward(prefix));
        let a = KeyEvent::press(Key::Char('a'), Mods::empty());
        assert_eq!(c.handle(a), Action::Forward(a));
    }
}
