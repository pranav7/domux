//! The clipboard: OSC 52 when the outer terminal accepts it, else pbcopy, wl-copy, xclip.

use base64::Engine;
use domux_core::proto::Capabilities;
use std::io::Write;
use std::process::{Command, Stdio};

pub fn osc52(text: &str) -> String {
    format!(
        "\x1b]52;c;{}\x07",
        base64::engine::general_purpose::STANDARD.encode(text)
    )
}

/// The first available clipboard command, in this order.
pub fn command_for(has: impl Fn(&str) -> bool) -> Option<&'static [&'static str]> {
    const CANDIDATES: &[&[&str]] = &[
        &["pbcopy"],
        &["wl-copy"],
        &["xclip", "-selection", "clipboard"],
    ];
    CANDIDATES.iter().copied().find(|c| has(c[0]))
}

fn on_path(cmd: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(cmd).is_file()))
        .unwrap_or(false)
}

/// Puts `text` on the outer terminal's clipboard. The error is a sentence for the hint row:
/// it names the state, the object and the next action (principle 9).
pub fn copy(text: &str, caps: &Capabilities) -> Result<(), String> {
    copy_into(text, caps, &mut std::io::stdout(), command_for(on_path))
}

/// The routing `copy` does, with the terminal and the search of `PATH` as parameters - the
/// same shape `terminal::write_restore` uses, and for the same reason: which route a
/// capability chooses is the behaviour worth holding, and it cannot be held through the real
/// stdout and the real clipboard.
pub fn copy_into(
    text: &str,
    caps: &Capabilities,
    out: &mut impl Write,
    cmd: Option<&[&str]>,
) -> Result<(), String> {
    if caps.osc52 {
        return out
            .write_all(osc52(text).as_bytes())
            .and_then(|_| out.flush())
            .map_err(|e| format!("could not write OSC 52: {e}"));
    }
    let Some(cmd) = cmd else {
        return Err("no clipboard tool found; install xclip or wl-clipboard, or use a terminal that accepts OSC 52".into());
    };
    let mut child = Command::new(cmd[0])
        .args(&cmd[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("{}: {e}", cmd[0]))?;
    child
        .stdin
        .take()
        .ok_or("no stdin")?
        .write_all(text.as_bytes())
        .map_err(|e| e.to_string())?;
    let status = child.wait().map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{} exited with {status}", cmd[0]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc52_base64_encodes_the_text() {
        assert_eq!(osc52("hi"), "\x1b]52;c;aGk=\x07");
    }

    fn caps(osc52: bool) -> Capabilities {
        Capabilities {
            osc52,
            ..Capabilities::default()
        }
    }

    /// A terminal that takes OSC 52 is written to directly and no process is started. Nothing
    /// held this before: the branch wrote to the real stdout, so it was skipped.
    #[test]
    fn copy_writes_osc52_when_the_terminal_takes_it() {
        let mut out: Vec<u8> = Vec::new();
        // The command is present and must still not be used: the capability decides.
        copy_into("hi", &caps(true), &mut out, Some(&["false"])).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "\x1b]52;c;aGk=\x07");
    }

    /// No route at all. The sentence names the state, the object and the next action, and it
    /// is what the hint row shows, so it is pinned literally.
    #[test]
    fn copy_names_the_fix_when_there_is_no_clipboard_route() {
        let mut out: Vec<u8> = Vec::new();
        let err = copy_into("hi", &caps(false), &mut out, None).unwrap_err();
        assert_eq!(
            err,
            "no clipboard tool found; install xclip or wl-clipboard, or use a terminal that accepts OSC 52"
        );
        assert!(
            out.is_empty(),
            "nothing is written to a terminal that cannot take it"
        );
    }

    /// The command route, on a command that consumes stdin and says how it ended - never the
    /// real clipboard, which a test must not write to.
    #[test]
    fn copy_runs_the_clipboard_command_and_reports_how_it_ended() {
        let mut out: Vec<u8> = Vec::new();
        copy_into(
            "hi",
            &caps(false),
            &mut out,
            Some(&["sh", "-c", "cat >/dev/null"]),
        )
        .unwrap();
        assert!(
            out.is_empty(),
            "the command route writes nothing to the terminal"
        );

        let err = copy_into(
            "hi",
            &caps(false),
            &mut out,
            Some(&["sh", "-c", "cat >/dev/null; exit 3"]),
        )
        .unwrap_err();
        assert!(err.contains("exited with"), "{err}");
        assert!(err.starts_with("sh"), "the error names the command: {err}");
    }

    #[test]
    fn command_for_prefers_pbcopy_then_wl_copy_then_xclip() {
        assert_eq!(command_for(|c| c == "pbcopy"), Some(&["pbcopy"][..]));
        assert_eq!(
            command_for(|c| c == "wl-copy" || c == "xclip"),
            Some(&["wl-copy"][..])
        );
        assert_eq!(
            command_for(|c| c == "xclip"),
            Some(&["xclip", "-selection", "clipboard"][..])
        );
        assert_eq!(command_for(|_| false), None);
    }
}
