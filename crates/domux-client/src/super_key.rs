//! Whether the reader is holding the super key, cmd on macOS, so the Navigator writes the agents'
//! numbers only while it is down (decision records 0065 and 0066).
//!
//! A terminal reports cmd pressed on its own only when it is asked to report every key as an
//! escape code, and in that mode the text a key composes, such as `é` after a dead key, comes in
//! a field crossterm does not read. So the client asks macOS rather than the terminal. The answer
//! is the machine's, so only a client on the reader's own Mac asks, and only while its terminal
//! has the focus.
//!
//! It asks only where cmd and a number reaches domux, because numbers that show with cmd invite
//! a chord, and a chord the terminal keeps for itself does something else. Ghostty is the one
//! terminal that can say: `ghostty +list-keybinds` answers whether it still keeps the chord.

use crate::ghostty;
use std::time::Duration;

/// How often the client looks while its terminal has the focus: often enough that the numbers
/// come with the key, and seldom enough to cost nothing.
pub const POLL: Duration = Duration::from_millis(50);

/// Answers whether the super key is down now.
pub type Reader = Box<dyn Fn() -> bool + Send>;

/// What the client found at attach about cmd and a number.
pub enum Check {
    /// The chord reaches domux and macOS can say when cmd is down.
    Reads(Reader),
    /// Ghostty keeps cmd and these numbers for its own tabs, so the chord never arrives.
    GhosttyKeeps(Vec<u8>),
    /// Nothing here can say whether the chord arrives: over ssh, on another system, in another
    /// terminal, or in a Ghostty that did not answer.
    CannotTell,
}

/// The check for a client over ssh or not (`remote`), in the terminal `TERM_PROGRAM` names.
/// `keybinds` asks Ghostty what its config leaves bound, and is called only in Ghostty on a Mac
/// the reader is at, because it runs a program.
pub fn check(
    remote: bool,
    terminal: Option<&str>,
    keybinds: impl FnOnce() -> Option<String>,
) -> Check {
    if remote || !cfg!(target_os = "macos") || terminal != Some("ghostty") {
        return Check::CannotTell;
    }
    let Some(list) = keybinds() else {
        return Check::CannotTell;
    };
    let kept = ghostty::kept_digits(&list);
    if !kept.is_empty() {
        return Check::GhosttyKeeps(kept);
    }
    match platform() {
        Some(read) => Check::Reads(read),
        None => Check::CannotTell,
    }
}

#[cfg(target_os = "macos")]
fn platform() -> Option<Reader> {
    Some(Box::new(macos::command_down))
}

#[cfg(not(target_os = "macos"))]
fn platform() -> Option<Reader> {
    None
}

#[cfg(target_os = "macos")]
mod macos {
    /// `kCGEventSourceStateCombinedSessionState`: the keys held anywhere in the login session.
    const COMBINED_SESSION_STATE: i32 = 0;
    /// `kCGEventFlagMaskCommand`.
    const COMMAND: u64 = 0x0010_0000;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventSourceFlagsState(state: i32) -> u64;
    }

    /// Whether cmd is down. Reading the modifier flags needs no permission, where reading the
    /// other keys would.
    pub fn command_down() -> bool {
        // SAFETY: the function takes an integer by value and returns an integer. It reads no
        // memory of ours and has no precondition.
        let flags = unsafe { CGEventSourceFlagsState(COMBINED_SESSION_STATE) };
        flags & COMMAND != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEEPS: &str = "keybind = super+digit_1=goto_tab:1\nkeybind = super+9=last_tab\n";

    fn never_asked() -> Option<String> {
        panic!("Ghostty is asked only in Ghostty on the reader's own Mac")
    }

    #[test]
    fn super_key_is_not_read_over_ssh_or_outside_ghostty() {
        assert!(matches!(
            check(true, Some("ghostty"), never_asked),
            Check::CannotTell
        ));
        assert!(matches!(
            check(false, Some("iTerm.app"), never_asked),
            Check::CannotTell
        ));
        assert!(matches!(check(false, None, never_asked), Check::CannotTell));
    }

    /// A Ghostty that keeps the chord is named, so the reader can be told how to hand it over,
    /// and one that does not answer says nothing either way.
    #[cfg(target_os = "macos")]
    #[test]
    fn super_key_is_read_only_where_ghostty_leaves_cmd_and_a_number() {
        assert!(matches!(
            check(false, Some("ghostty"), || Some(KEEPS.into())),
            Check::GhosttyKeeps(kept) if kept == vec![1, 9]
        ));
        assert!(matches!(
            check(false, Some("ghostty"), || None),
            Check::CannotTell
        ));
        let Check::Reads(read) = check(false, Some("ghostty"), || Some(String::new())) else {
            panic!("a Ghostty that keeps no number leaves the chord to domux");
        };
        // Nobody holds cmd while the suite runs, and the call answers without a permission.
        assert!(!read());
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn super_key_is_never_read_off_macos() {
        assert!(matches!(
            check(false, Some("ghostty"), || Some(KEEPS.into())),
            Check::CannotTell
        ));
    }
}
