//! Raw mode, the alternate screen, wheel, keyboard enhancement, bracketed paste and focus
//! reports, restored on drop and before a panic message prints (principle 11).
//!
//! V2 enables normal mouse tracking only so the outer terminal reports wheel position. The
//! client ignores clicks and all motion. Alternate scroll stays disabled so an unreported
//! gesture cannot turn into Up or Down keys for the focused pane. Everything this module
//! changes has its pair in `write_restore`, and the test holds the two together.

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
use std::sync::atomic::{AtomicBool, Ordering};

/// Whether keyboard enhancement flags are pushed on the terminal's own stack and not yet
/// popped. A panic restores twice - the hook runs, then unwinding drops the guard - and
/// popping flags that were never pushed unbalances that stack, so the pop belongs to
/// whichever restore runs first.
static KEYBOARD_FLAGS_PUSHED: AtomicBool = AtomicBool::new(false);

/// Whether the panic hook is installed. `attach` can run more than once in one process, and
/// each install wraps the hook before it, so without this a single panic would restore once
/// per attach.
static PANIC_HOOK_INSTALLED: AtomicBool = AtomicBool::new(false);

/// XTSAVE mode 1007, then reset it. Ghostty otherwise translates scrolling on the alternate
/// screen into cursor keys, which the focused pane would receive as ordinary input.
const SAVE_AND_DISABLE_ALTERNATE_SCROLL: &[u8] = b"\x1b[?1007s\x1b[?1007l";
const RESTORE_ALTERNATE_SCROLL: &[u8] = b"\x1b[?1007r";

/// Save normal and SGR mouse modes, then enable them. Normal tracking is the narrow terminal
/// mode that includes wheel position; unlike crossterm's broad mouse command, it does not ask
/// the terminal for drag or pointer-motion events. Non-wheel events are ignored by the client.
const SAVE_AND_ENABLE_WHEEL_REPORTING: &[u8] = b"\x1b[?1000s\x1b[?1006s\x1b[?1000h\x1b[?1006h";
const RESTORE_WHEEL_REPORTING: &[u8] = b"\x1b[?1006r\x1b[?1000r";

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
        // Only a terminal is written to, the same guard `restore` uses. A piped stdout would
        // otherwise be handed the set sequences and never their unset, which is the one
        // pairing the crate promises.
        if stdout().is_terminal() {
            write_enter(&mut stdout(), keyboard_enhancement)?;
            if keyboard_enhancement {
                KEYBOARD_FLAGS_PUSHED.store(true, Ordering::SeqCst);
            }
        }
        Ok(guard)
    }

    /// Restores everything `enter` changed. Safe to call twice: the second call writes no
    /// keyboard pop, because the first one took it.
    ///
    /// The escape sequence is written only to a terminal: a piped stdout, and a test runner,
    /// would otherwise be handed control bytes they have no use for. Raw mode is left either
    /// way, since crossterm sets it on the controlling terminal rather than on stdout.
    pub fn restore(keyboard_enhancement: bool) {
        let pop = take_pushed_flags(&KEYBOARD_FLAGS_PUSHED) && keyboard_enhancement;
        if stdout().is_terminal() {
            let _ = write_restore(&mut stdout(), pop);
        }
        let _ = disable_raw_mode();
    }

    /// Installs a panic hook that restores the terminal before the default hook prints.
    pub fn install_panic_hook(keyboard_enhancement: bool) {
        install_panic_hook_once(&PANIC_HOOK_INSTALLED, move || {
            TerminalGuard::restore(keyboard_enhancement)
        });
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        TerminalGuard::restore(self.keyboard_enhancement);
    }
}

/// Everything the client turns on, in the order it is turned on.
fn write_enter(out: &mut impl Write, keyboard_enhancement: bool) -> std::io::Result<()> {
    queue!(out, EnterAlternateScreen)?;
    out.write_all(SAVE_AND_DISABLE_ALTERNATE_SCROLL)?;
    out.write_all(SAVE_AND_ENABLE_WHEEL_REPORTING)?;
    queue!(out, EnableBracketedPaste, EnableFocusChange)?;
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
    queue!(
        out,
        DisableFocusChange,
        DisableBracketedPaste,
        SetCursorStyle::DefaultUserShape,
        Show
    )?;
    out.write_all(RESTORE_WHEEL_REPORTING)?;
    out.write_all(RESTORE_ALTERNATE_SCROLL)?;
    execute!(out, LeaveAlternateScreen)
}

/// Whether this restore owns the pop: true once per push, for whichever restore is first.
fn take_pushed_flags(pushed: &AtomicBool) -> bool {
    pushed.swap(false, Ordering::SeqCst)
}

/// Wraps the current hook, and only the first time. The flag is a parameter so a test can
/// ask the question without touching the process-wide one.
fn install_panic_hook_once(installed: &AtomicBool, restore: impl Fn() + Send + Sync + 'static) {
    if installed.swap(true, Ordering::SeqCst) {
        return;
    }
    install_panic_hook_with(restore);
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
    use std::sync::atomic::AtomicUsize;
    use std::sync::{Arc, Mutex};

    /// The panic hook is process-wide, so the tests that install one take turns.
    fn hook_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn restore_puts_every_setting_back_in_the_reverse_order() {
        let mut out = Vec::new();
        write_restore(&mut out, true).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "\x1b[<1u\x1b[?1004l\x1b[?2004l\x1b[0 q\x1b[?25h\x1b[?1006r\x1b[?1000r\x1b[?1007r\x1b[?1049l"
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
            ("\x1b[?1007s\x1b[?1007l", "\x1b[?1007r"),
            (
                "\x1b[?1000s\x1b[?1006s\x1b[?1000h\x1b[?1006h",
                "\x1b[?1006r\x1b[?1000r",
            ),
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
        let _serialised = hook_lock();
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

    /// A panic restores twice: the hook runs, then unwinding drops the guard. The pop of the
    /// keyboard flags belongs to the first of them, because the second would pop flags that
    /// are no longer pushed and unbalance the terminal's own stack.
    #[test]
    fn the_keyboard_flags_are_popped_once_for_one_push() {
        let pushed = AtomicBool::new(true);
        assert!(take_pushed_flags(&pushed), "the first restore pops");
        assert!(!take_pushed_flags(&pushed), "the second restore does not");
    }

    /// Two attaches in one process. The second must not wrap the hook again: one panic would
    /// then restore twice, which is the double pop above by another route.
    #[test]
    fn the_panic_hook_is_installed_once_however_many_attaches_run() {
        let _serialised = hook_lock();
        let restores = Arc::new(AtomicUsize::new(0));
        let saved = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let installed = AtomicBool::new(false);
        // Only this thread's panic is counted: a test failing on another thread runs the
        // process-wide hook too, and this test is about how many layers it has, not how many
        // panics the run had.
        let mine = std::thread::current().id();
        for _ in 0..2 {
            let counted = restores.clone();
            install_panic_hook_once(&installed, move || {
                if std::thread::current().id() == mine {
                    counted.fetch_add(1, Ordering::SeqCst);
                }
            });
        }
        let panicked = std::panic::catch_unwind(|| panic!("a deliberate panic"));
        std::panic::set_hook(saved);
        assert!(panicked.is_err());
        assert_eq!(
            restores.load(Ordering::SeqCst),
            1,
            "a second attach must not stack a second restore"
        );
    }
}
