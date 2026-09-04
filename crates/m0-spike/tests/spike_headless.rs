use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::Read;
use std::time::{Duration, Instant};

#[test]
fn spike_runs_panes_writes_stats_and_exits_on_time() {
    let stats = std::env::temp_dir().join(format!("m0-spike-stats-{}.json", std::process::id()));
    let pty = native_pty_system();
    let pair = pty
        .openpty(PtySize {
            rows: 30,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_m0-spike"));
    cmd.args([
        "--emulator",
        "alacritty",
        "--panes",
        "2",
        "--exit-after",
        "2",
        "--stats-out",
    ]);
    cmd.arg(&stats);
    cmd.args(["--", "sh", "-c", "printf 'pane says hi'; sleep 5"]);
    cmd.env("TERM", "xterm-256color");
    let mut child = pair.slave.spawn_command(cmd).unwrap();
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().unwrap();
    let mut output = Vec::new();
    let start = Instant::now();
    let mut buf = [0u8; 4096];
    while start.elapsed() < Duration::from_secs(10) {
        match reader.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => output.extend_from_slice(&buf[..n]),
        }
    }
    let status = child.wait().unwrap();
    assert!(status.success(), "spike exit status {status:?}");
    let text = String::from_utf8_lossy(&output);
    // ratatui writes only the cells that changed, moving the cursor over the unchanged
    // blanks, so the pane's words arrive separated by CSI sequences rather than as one
    // string. Strip the sequences and check the words land in order.
    let visible = visible_text(&output);
    let mut rest = visible.as_str();
    for word in ["pane", "says", "hi"] {
        let at = rest.find(word).unwrap_or_else(|| {
            panic!("rendered output missing {word:?}; visible text was {visible:?}")
        });
        rest = &rest[at + word.len()..];
    }
    assert!(
        text.contains("\x1b[?1049l"),
        "alternate screen was not left"
    );
    assert!(
        text.contains("\x1b[?2004l"),
        "bracketed paste was not disabled"
    );
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&stats).unwrap()).unwrap();
    assert_eq!(json["panes"], 2);
    assert!(json["frames"].as_u64().unwrap() >= 1);
    assert!(json["frame_us"]["p99"].as_u64().is_some());
    assert!(json["cpu_ms_after_settle"].as_u64().is_some());
    let _ = std::fs::remove_file(&stats);
}

/// Drops CSI, OSC, and charset escape sequences so only the printed cells remain.
fn visible_text(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match chars.next() {
            // CSI: parameters and intermediates, then one final byte.
            Some('[') => {
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() || c == '~' {
                        break;
                    }
                }
            }
            // OSC: runs to BEL or ST.
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == '\u{7}' {
                        break;
                    }
                    if c == '\u{1b}' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            // Charset selection and other two-byte escapes.
            Some('(') | Some(')') => {
                chars.next();
            }
            _ => {}
        }
    }
    out
}
