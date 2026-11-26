//! Whether the reader is holding the super key, cmd on macOS, so the Navigator writes the agents'
//! numbers only while it is down (decision record 0065).
//!
//! A terminal reports cmd pressed on its own only when it is asked to report every key as an
//! escape code, and in that mode the text a key composes, such as `é` after a dead key, comes in
//! a field crossterm does not read. So the client asks macOS rather than the terminal. The answer
//! is the machine's, so only a client on the reader's own Mac asks, and only while its terminal
//! has the focus.

use std::time::Duration;

/// How often the client looks while its terminal has the focus: often enough that the numbers
/// come with the key, and seldom enough to cost nothing.
pub const POLL: Duration = Duration::from_millis(50);

/// Answers whether the super key is down now.
pub type Reader = Box<dyn Fn() -> bool + Send>;

/// The reader for this client: macOS's answer on the Mac the reader is at, and `None` over ssh
/// or on another system, where nothing here can tell.
pub fn reader(remote: bool) -> Option<Reader> {
    if remote {
        return None;
    }
    platform()
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

    #[test]
    fn super_key_is_not_read_over_ssh() {
        assert!(reader(true).is_none());
    }

    /// On the reader's own machine macOS answers, and no other system does.
    #[test]
    fn super_key_is_read_only_on_macos() {
        assert_eq!(reader(false).is_some(), cfg!(target_os = "macos"));
    }

    /// Nobody holds cmd while the suite runs, and the call answers without a permission.
    #[cfg(target_os = "macos")]
    #[test]
    fn super_key_reads_as_up_when_nobody_holds_it() {
        let read = reader(false).expect("macOS answers");
        assert!(!read());
    }
}
