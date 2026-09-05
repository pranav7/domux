//! Key encoding for the alacritty path, using termwiz's encoder and alacritty's mode flags.

use crate::key::{Key, KeyAction, KeyEvent, Mods};
use alacritty_terminal::term::TermMode;
use termwiz::escape::csi::KittyKeyboardFlags;
use termwiz::input::{KeyCode, KeyCodeEncodeModes, KeyboardEncoding, Modifiers};

pub fn encode(mode: TermMode, key: &KeyEvent, out: &mut Vec<u8>) {
    let mut kitty = KittyKeyboardFlags::NONE;
    if mode.contains(TermMode::DISAMBIGUATE_ESC_CODES) {
        kitty |= KittyKeyboardFlags::DISAMBIGUATE_ESCAPE_CODES;
    }
    if mode.contains(TermMode::REPORT_EVENT_TYPES) {
        kitty |= KittyKeyboardFlags::REPORT_EVENT_TYPES;
    }
    if mode.contains(TermMode::REPORT_ALTERNATE_KEYS) {
        kitty |= KittyKeyboardFlags::REPORT_ALTERNATE_KEYS;
    }
    if mode.contains(TermMode::REPORT_ALL_KEYS_AS_ESC) {
        kitty |= KittyKeyboardFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES;
    }
    if mode.contains(TermMode::REPORT_ASSOCIATED_TEXT) {
        kitty |= KittyKeyboardFlags::REPORT_ASSOCIATED_TEXT;
    }
    let encoding = if kitty.is_empty() {
        KeyboardEncoding::Xterm
    } else {
        KeyboardEncoding::Kitty(kitty)
    };
    let modes = KeyCodeEncodeModes {
        encoding,
        application_cursor_keys: mode.contains(TermMode::APP_CURSOR),
        newline_mode: mode.contains(TermMode::LINE_FEED_NEW_LINE),
        modify_other_keys: None,
    };
    // Releases are only meaningful under the kitty protocol with event types reported.
    if key.action == KeyAction::Release && !kitty.contains(KittyKeyboardFlags::REPORT_EVENT_TYPES) {
        return;
    }
    // termwiz 0.23 puts `encode` on `KeyCode` and takes the modifiers separately.
    if let Ok(text) =
        key_code(key.key).encode(modifiers(key.mods), modes, key.action != KeyAction::Release)
    {
        out.extend_from_slice(text.as_bytes());
    }
}

fn key_code(key: Key) -> KeyCode {
    match key {
        Key::Char(c) => KeyCode::Char(c),
        Key::Enter => KeyCode::Enter,
        Key::Tab => KeyCode::Tab,
        Key::Backspace => KeyCode::Backspace,
        Key::Escape => KeyCode::Escape,
        Key::Up => KeyCode::UpArrow,
        Key::Down => KeyCode::DownArrow,
        Key::Left => KeyCode::LeftArrow,
        Key::Right => KeyCode::RightArrow,
        Key::Home => KeyCode::Home,
        Key::End => KeyCode::End,
        Key::PageUp => KeyCode::PageUp,
        Key::PageDown => KeyCode::PageDown,
        Key::Insert => KeyCode::Insert,
        Key::Delete => KeyCode::Delete,
        Key::F(n) => KeyCode::Function(n),
    }
}

fn modifiers(m: Mods) -> Modifiers {
    let mut out = Modifiers::NONE;
    if m.contains(Mods::SHIFT) {
        out |= Modifiers::SHIFT;
    }
    if m.contains(Mods::CTRL) {
        out |= Modifiers::CTRL;
    }
    if m.contains(Mods::ALT) {
        out |= Modifiers::ALT;
    }
    if m.contains(Mods::SUPER) {
        out |= Modifiers::SUPER;
    }
    out
}
