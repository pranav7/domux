//! Raw mode, the alternate screen, keyboard enhancement, bracketed paste and focus reports,
//! restored on drop and before a panic message prints (principle 11).
//!
//! V2 never enables mouse reporting (the roadmap rules mouse out of V2.0), so there is
//! nothing of that kind to turn off here. Everything this module turns on has its pair in
//! `write_restore`, and the test holds the two together.

use anyhow::Result;
use crossterm::cursor::{SetCursorStyle, Show};
use crossterm::event::{
    DisableBracketedPaste, DisableFocusChange, EnableBracketedPaste, EnableFocusChange,
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::{execute, queue};
use std::io::{stdout, IsTerminal, Write};

/// Holds the terminal settings the client changed. Dropping it puts them back, whether the
/// client detached, lost the server, was signalled or panicked.
pub struct TerminalGuard {
    keyboard_enhancement: bool,
}

impl TerminalGuard {
    pub fn enter(keyboard_enhancement: bool) -> Result<TerminalGuard> {
        enable_raw_mode()?;
        // The guard exists from here on, so a failure after raw mode is on still restores.
        let guard = TerminalGuard {
            keyboard_enhancement,
        };
        write_enter(&mut stdout(), keyboard_enhancement)?;
        Ok(guard)
    }

    /// Restores everything `enter` changed. Safe to call twice; used by the panic hook.
    ///
    /// The escape sequence is written only to a terminal: a piped stdout, and a test runner,
    /// would otherwise be handed control bytes they have no use for. Raw mode is left either
    /// way, since crossterm sets it on the controlling terminal rather than on stdout.
    pub fn restore(keyboard_enhancement: bool) {
        if stdout().is_terminal() {
            let _ = write_restore(&mut stdout(), keyboard_enhancement);
        }
        let _ = disable_raw_mode();
    }

    /// Installs a panic hook that restores the terminal before the default hook prints.
    pub fn install_panic_hook(keyboard_enhancement: bool) {
        install_panic_hook_with(move || TerminalGuard::restore(keyboard_enhancement));
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        TerminalGuard::restore(self.keyboard_enhancement);
    }
}

/// Everything the client turns on, in the order it is turned on.
fn write_enter(out: &mut impl Write, keyboard_enhancement: bool) -> std::io::Result<()> {
    queue!(
        out,
        EnterAlternateScreen,
        EnableBracketedPaste,
        EnableFocusChange
    )?;
    if keyboard_enhancement {
        queue!(
            out,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;
    }
    out.flush()
}

/// The same settings put back, in the reverse order. A parameter rather than stdout so a
/// test reads the bytes the terminal would have read.
fn write_restore(out: &mut impl Write, keyboard_enhancement: bool) -> std::io::Result<()> {
    if keyboard_enhancement {
        queue!(out, PopKeyboardEnhancementFlags)?;
    }
    execute!(
        out,
        DisableFocusChange,
        DisableBracketedPaste,
        SetCursorStyle::DefaultUserShape,
        Show,
        LeaveAlternateScreen
    )
}

/// The hook with its restore as a parameter, so a test can watch it run without a terminal.
fn install_panic_hook_with(restore: impl Fn() + Send + Sync + 'static) {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore();
        default_hook(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn restore_puts_every_setting_back_in_the_reverse_order() {
        let mut out = Vec::new();
        write_restore(&mut out, true).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "\x1b[<1u\x1b[?1004l\x1b[?2004l\x1b[0 q\x1b[?25h\x1b[?1049l"
        );
    }

    #[test]
    fn restore_leaves_the_keyboard_flags_alone_when_none_were_pushed() {
        let mut out = Vec::new();
        write_restore(&mut out, false).unwrap();
        assert!(
            !String::from_utf8(out).unwrap().contains("\x1b[<1u"),
            "popping flags that were never pushed unbalances the terminal's own stack"
        );
    }

    #[test]
    fn every_setting_enter_turns_on_is_turned_off_by_restore() {
        let mut on = Vec::new();
        write_enter(&mut on, true).unwrap();
        let mut off = Vec::new();
        write_restore(&mut off, true).unwrap();
        let on = String::from_utf8(on).unwrap();
        let off = String::from_utf8(off).unwrap();
        for (set, unset) in [
            ("\x1b[?1049h", "\x1b[?1049l"),
            ("\x1b[?2004h", "\x1b[?2004l"),
            ("\x1b[?1004h", "\x1b[?1004l"),
            ("\x1b[>1u", "\x1b[<1u"),
        ] {
            assert!(on.contains(set), "enter must set {set:?}");
            assert!(off.contains(unset), "restore must unset {unset:?}");
        }
    }

    /// The user is left in raw mode with no cursor if the message prints first, so the order
    /// is the whole point of the hook.
    #[test]
    fn the_panic_hook_restores_the_terminal_before_the_default_hook_prints() {
        let order: Arc<Mutex<Vec<&'static str>>> = Arc::new(Mutex::new(Vec::new()));
        let saved = std::panic::take_hook();
        let recorded = order.clone();
        std::panic::set_hook(Box::new(move |_| {
            recorded.lock().unwrap().push("default");
        }));
        let recorded = order.clone();
        install_panic_hook_with(move || recorded.lock().unwrap().push("restore"));
        let panicked = std::panic::catch_unwind(|| panic!("a deliberate panic"));
        std::panic::set_hook(saved);
        assert!(panicked.is_err());
        assert_eq!(*order.lock().unwrap(), vec!["restore", "default"]);
    }
}
