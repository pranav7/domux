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
    if caps.osc52 {
        let mut out = std::io::stdout();
        return out
            .write_all(osc52(text).as_bytes())
            .and_then(|_| out.flush())
            .map_err(|e| format!("could not write OSC 52: {e}"));
    }
    let Some(cmd) = command_for(on_path) else {
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
