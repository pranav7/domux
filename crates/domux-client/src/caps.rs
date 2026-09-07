//! What the outer terminal can do, from the environment and two OSC queries.

use domux_core::proto::Capabilities;
use domux_term::Rgb;
use std::io::{IsTerminal, Write};
use std::time::{Duration, Instant};

/// The environment the capabilities are read from, as values rather than as `std::env`
/// calls, so `detect` is a pure function a test can drive.
#[derive(Debug, Clone)]
pub struct CapsEnv {
    pub colorterm: Option<String>,
    pub term: Option<String>,
    pub term_program: Option<String>,
    pub ssh_tty: Option<String>,
    pub keyboard_enhancement: bool,
}

impl CapsEnv {
    pub fn from_process() -> CapsEnv {
        let var = |n: &str| std::env::var(n).ok().filter(|v| !v.is_empty());
        CapsEnv {
            colorterm: var("COLORTERM"),
            term: var("TERM"),
            term_program: var("TERM_PROGRAM"),
            ssh_tty: var("SSH_TTY"),
            keyboard_enhancement: crossterm::terminal::supports_keyboard_enhancement()
                .unwrap_or(false),
        }
    }
}

const RICH_TERMINALS: &[&str] = &["ghostty", "WezTerm", "iTerm.app", "kitty"];

/// A terminal that says nothing about itself gets nothing: every capability is false and
/// both default colours stay absent. A guess here would show as wrong colour on the screen.
pub fn detect(env: &CapsEnv) -> Capabilities {
    let truecolor = matches!(env.colorterm.as_deref(), Some("truecolor") | Some("24bit"));
    let rich = env
        .term_program
        .as_deref()
        .is_some_and(|p| RICH_TERMINALS.contains(&p))
        || env
            .term
            .as_deref()
            .is_some_and(|t| t.starts_with("xterm-ghostty") || t.starts_with("xterm-kitty"));
    Capabilities {
        truecolor,
        kitty_keyboard: env.keyboard_enhancement,
        hyperlinks: rich,
        osc52: rich || env.ssh_tty.is_some(),
        default_fg: None,
        default_bg: None,
    }
}

/// `rgb:rrrr/gggg/bbbb` or `rgb:rr/gg/bb` as terminals answer OSC 10 and 11.
pub fn parse_osc_color(s: &str) -> Option<Rgb> {
    let rest = s.strip_prefix("rgb:")?;
    let mut parts = rest.split('/');
    let mut channel = || -> Option<u8> {
        let p = parts.next()?;
        let v = u16::from_str_radix(p, 16).ok()?;
        Some(if p.len() > 2 { (v >> 8) as u8 } else { v as u8 })
    };
    Some(Rgb {
        r: channel()?,
        g: channel()?,
        b: channel()?,
    })
}

/// Asks the terminal for its default foreground and background. Must run in raw mode and
/// before the event stream reads stdin. A terminal that does not answer within `timeout`
/// gives `(None, None)`; the server then uses its own defaults.
///
/// Stdin that is not a terminal answers nothing, so the query is not written at all: a test
/// runner or a piped stdin would otherwise be sent escape bytes and read for 100 ms.
pub fn query_default_colors(timeout: Duration) -> (Option<Rgb>, Option<Rgb>) {
    if !std::io::stdin().is_terminal() {
        return (None, None);
    }
    let mut out = std::io::stdout();
    if out
        .write_all(b"\x1b]10;?\x07\x1b]11;?\x07")
        .and_then(|_| out.flush())
        .is_err()
    {
        return (None, None);
    }
    let text = read_replies(libc::STDIN_FILENO, timeout);
    (find_reply(&text, "10"), find_reply(&text, "11"))
}

/// Reads OSC answers off a descriptor until both have arrived or `timeout` passes.
///
/// The descriptor is read directly rather than through `std::io::Stdin`. A `StdinLock` reads
/// into an 8 KB `BufReader`, so a one byte request pulls every byte the terminal sent into a
/// buffer this loop cannot see: `poll` then reports an empty descriptor for the rest of the
/// window, the answers are stranded, and anything the user typed ahead is stranded with them,
/// because the event stream reads the descriptor and never that buffer.
///
/// One byte per read, so the loop stops on the byte that finishes the second answer and
/// leaves whatever follows it - type-ahead - on the descriptor for the event stream.
fn read_replies(fd: std::os::fd::RawFd, timeout: Duration) -> String {
    let deadline = Instant::now() + timeout;
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    while Instant::now() < deadline {
        let mut fds = [libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        }];
        let left = deadline
            .saturating_duration_since(Instant::now())
            .as_millis() as i32;
        // Safe: one pollfd, a bounded timeout.
        let n = unsafe { libc::poll(fds.as_mut_ptr(), 1, left.max(1)) };
        if n <= 0 {
            break;
        }
        // Safe: a one byte buffer this call owns, on a descriptor poll just called readable.
        let n = unsafe { libc::read(fd, byte.as_mut_ptr() as *mut libc::c_void, 1) };
        if n <= 0 {
            break;
        }
        buf.push(byte[0]);
        if count_replies(&buf) >= 2 {
            break;
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

fn count_replies(buf: &[u8]) -> usize {
    let s = String::from_utf8_lossy(buf);
    s.matches("\x1b]1")
        .count()
        .min(s.matches('\x07').count() + s.matches("\x1b\\").count())
}

/// The colour in one OSC answer, or `None` when the terminal did not answer that one.
fn find_reply(text: &str, code: &str) -> Option<Rgb> {
    let start = text.find(&format!("\x1b]{code};"))? + 3 + code.len();
    let rest = &text[start..];
    let end = rest.find(['\x07', '\x1b']).unwrap_or(rest.len());
    parse_osc_color(&rest[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::os::fd::AsRawFd;

    #[test]
    fn parse_osc_color_reads_16_bit_channels() {
        assert_eq!(
            parse_osc_color("rgb:cdcd/d6d6/f4f4"),
            Some(Rgb {
                r: 0xcd,
                g: 0xd6,
                b: 0xf4
            })
        );
        assert_eq!(
            parse_osc_color("rgb:1e/1e/2e"),
            Some(Rgb {
                r: 0x1e,
                g: 0x1e,
                b: 0x2e
            })
        );
        assert_eq!(parse_osc_color("nonsense"), None);
    }

    #[test]
    fn detect_reads_truecolor_and_terminal_program() {
        let env = CapsEnv {
            colorterm: Some("truecolor".into()),
            term: Some("xterm-256color".into()),
            term_program: Some("ghostty".into()),
            ssh_tty: None,
            keyboard_enhancement: true,
        };
        let c = detect(&env);
        assert!(c.truecolor && c.kitty_keyboard && c.hyperlinks && c.osc52);
        let plain = CapsEnv {
            colorterm: None,
            term: Some("xterm".into()),
            term_program: None,
            ssh_tty: None,
            keyboard_enhancement: false,
        };
        let c = detect(&plain);
        assert!(!c.truecolor && !c.kitty_keyboard && !c.hyperlinks && !c.osc52);
        let ssh = CapsEnv {
            ssh_tty: Some("/dev/pts/3".into()),
            ..plain.clone()
        };
        assert!(
            detect(&ssh).osc52,
            "over ssh OSC 52 is the only clipboard route"
        );
    }

    /// A terminal that answers nothing at all. Every capability is off and both colours stay
    /// absent: a default here would paint the screen in colours the terminal never named.
    #[test]
    fn a_terminal_that_says_nothing_gets_no_capabilities_and_no_colours() {
        let silent = CapsEnv {
            colorterm: None,
            term: None,
            term_program: None,
            ssh_tty: None,
            keyboard_enhancement: false,
        };
        assert_eq!(detect(&silent), Capabilities::default());
        assert!(detect(&silent).default_fg.is_none() && detect(&silent).default_bg.is_none());
    }

    #[test]
    fn find_reply_reads_each_answer_and_leaves_a_missing_one_absent() {
        let both = "\x1b]10;rgb:cdcd/d6d6/f4f4\x07\x1b]11;rgb:1e1e/1e1e/2e2e\x07";
        assert_eq!(
            find_reply(both, "10"),
            Some(Rgb {
                r: 0xcd,
                g: 0xd6,
                b: 0xf4
            })
        );
        assert_eq!(
            find_reply(both, "11"),
            Some(Rgb {
                r: 0x1e,
                g: 0x1e,
                b: 0x2e
            })
        );
        // Some terminals end the answer with ST rather than BEL.
        assert_eq!(
            find_reply("\x1b]11;rgb:1e1e/1e1e/2e2e\x1b\\", "11"),
            Some(Rgb {
                r: 0x1e,
                g: 0x1e,
                b: 0x2e
            })
        );
        assert_eq!(find_reply("", "10"), None, "no answer is absent, not black");
        assert_eq!(find_reply(both, "12"), None);
    }

    #[test]
    fn count_replies_counts_only_finished_answers() {
        assert_eq!(count_replies(b"\x1b]10;rgb:1e/1e/2e"), 0);
        assert_eq!(count_replies(b"\x1b]10;rgb:1e/1e/2e\x07"), 1);
        assert_eq!(count_replies(b"\x1b]10;a\x07\x1b]11;b\x07"), 2);
    }

    /// The terminal answers both queries in one write, and the user has typed ahead. Both
    /// answers are collected, and the type-ahead is still on the descriptor for the event
    /// stream: a read that buffered would have taken it and lost those keystrokes.
    #[test]
    fn read_replies_collects_both_answers_and_leaves_what_follows_them() {
        let (reader, mut writer) = std::io::pipe().unwrap();
        writer
            .write_all(b"\x1b]10;rgb:cdcd/d6d6/f4f4\x07\x1b]11;rgb:1e1e/1e1e/2e2e\x07hi")
            .unwrap();
        let text = read_replies(reader.as_raw_fd(), Duration::from_millis(500));
        assert_eq!(
            (find_reply(&text, "10"), find_reply(&text, "11")),
            (
                Some(Rgb {
                    r: 0xcd,
                    g: 0xd6,
                    b: 0xf4
                }),
                Some(Rgb {
                    r: 0x1e,
                    g: 0x1e,
                    b: 0x2e
                })
            )
        );
        let mut rest = [0u8; 2];
        let mut reader = reader;
        reader.read_exact(&mut rest).unwrap();
        assert_eq!(&rest, b"hi", "type-ahead stays on the descriptor");
    }

    /// The two answers arrive in two writes, which is what a terminal that answers each
    /// query as it reaches it does. The loop polls again rather than stopping at the first.
    #[test]
    fn read_replies_collects_answers_that_arrive_in_two_writes() {
        let (reader, mut writer) = std::io::pipe().unwrap();
        std::thread::spawn(move || {
            writer.write_all(b"\x1b]10;rgb:cdcd/d6d6/f4f4\x07").unwrap();
            std::thread::sleep(Duration::from_millis(20));
            writer.write_all(b"\x1b]11;rgb:1e1e/1e1e/2e2e\x07").unwrap();
        });
        let text = read_replies(reader.as_raw_fd(), Duration::from_millis(2000));
        assert!(
            find_reply(&text, "10").is_some() && find_reply(&text, "11").is_some(),
            "{text:?}"
        );
    }

    /// A terminal that answers nothing costs the timeout and gives nothing. Absent, not black.
    #[test]
    fn read_replies_gives_up_at_the_deadline_when_nothing_answers() {
        let (reader, _writer) = std::io::pipe().unwrap();
        let started = Instant::now();
        let text = read_replies(reader.as_raw_fd(), Duration::from_millis(30));
        assert_eq!(text, "");
        assert!(started.elapsed() < Duration::from_millis(2000));
        assert_eq!(find_reply(&text, "10"), None);
    }
}
