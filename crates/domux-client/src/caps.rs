//! What the outer terminal can do, from the environment, and what its colours are, from one
//! batch of queries at attach.

use domux_core::proto::Capabilities;
use domux_core::theme::TerminalColors;
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
    /// How long crossterm's keyboard probe took to be answered, or `None` when it got no
    /// answer. It asks for the flags and then for device attributes, and gives up after 2 s when
    /// neither arrives. A terminal answers the attach batch about as fast as it answered the
    /// probe, and one that did not answer it will not answer the batch either, so this sets the
    /// batch's cap.
    pub probe_took: Option<Duration>,
}

impl CapsEnv {
    pub fn from_process() -> CapsEnv {
        let var = |n: &str| std::env::var(n).ok().filter(|v| !v.is_empty());
        // `Err` when neither the flags nor the device attributes answer came back in 2 s.
        let asked = Instant::now();
        let probe = crossterm::terminal::supports_keyboard_enhancement();
        let took = asked.elapsed();
        CapsEnv {
            colorterm: var("COLORTERM"),
            term: var("TERM"),
            term_program: var("TERM_PROGRAM"),
            ssh_tty: var("SSH_TTY"),
            keyboard_enhancement: probe.as_ref().is_ok_and(|supported| *supported),
            probe_took: probe.is_ok().then_some(took),
        }
    }
}

const RICH_TERMINALS: &[&str] = &["ghostty", "WezTerm", "iTerm.app", "kitty"];

/// A terminal that says nothing about itself gets nothing: every capability is false and
/// every colour stays absent. A guess here would show as wrong colour on the screen.
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
        colors: TerminalColors::default(),
    }
}

/// How much longer than the keyboard probe took attach waits for the batch's answers. A terminal
/// that answers ends the wait sooner, at its device attributes answer, so the cap is only felt
/// by one that answers slowly. An answer that arrives after the wait is read by the event stream
/// as keys and typed into the focused pane, which is what a longer cap keeps out. A terminal
/// that took 2 s over the probe takes about that over the batch, so its cap is 3 s.
pub const ATTACH_ANSWER_CAP: Duration = Duration::from_secs(1);

/// How long attach waits when the keyboard probe got no answer: a terminal that did not answer
/// device attributes two seconds ago will not answer them now, and its attach should cost no
/// more than a short wait (principle 8).
pub const SILENT_TERMINAL_CAP: Duration = Duration::from_millis(100);

/// The cap on the attach read, from how long the keyboard probe took to be answered.
pub fn attach_cap(probe_took: Option<Duration>) -> Duration {
    match probe_took {
        Some(took) => took + ATTACH_ANSWER_CAP,
        None => SILENT_TERMINAL_CAP,
    }
}

/// The palette slots the batch asks for, 0 to 15.
const SLOTS: usize = 16;

/// The queries attach writes, in the order the terminal answers them: the default foreground
/// (OSC 10), the default background (OSC 11), one OSC 4 per palette slot, because every
/// terminal that answers OSC 4 accepts that form, and device attributes (DA1) last. Every
/// terminal answers DA1, so its answer says the terminal has answered all it is going to.
pub fn attach_batch() -> Vec<u8> {
    let mut batch = b"\x1b]10;?\x07\x1b]11;?\x07".to_vec();
    for n in 0..SLOTS {
        batch.extend(format!("\x1b]4;{n};?\x07").as_bytes());
    }
    batch.extend(b"\x1b[c");
    batch
}

/// A colour as terminals answer OSC 10, 11 and 4: `rgb:` and three channels of one to four hex
/// digits. A channel of `k` digits is scaled to a byte as `v * 255 / (16^k - 1)`, rounded, so
/// `f`, `ff`, `fff` and `ffff` are all 255.
pub fn parse_osc_color(s: &str) -> Option<Rgb> {
    let rest = s.strip_prefix("rgb:")?;
    let mut parts = rest.split('/');
    let mut channel = || -> Option<u8> {
        let p = parts.next()?;
        if p.is_empty() || p.len() > 4 || !p.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let v = u32::from_str_radix(p, 16).ok()?;
        let max = (1u32 << (4 * p.len())) - 1;
        u8::try_from((v * 255 + max / 2) / max).ok()
    };
    let rgb = Rgb {
        r: channel()?,
        g: channel()?,
        b: channel()?,
    };
    parts.next().is_none().then_some(rgb)
}

/// Asks the terminal for its colours: writes the attach batch and reads the answers until the
/// device attributes answer arrives or `cap` passes. Must run in raw mode and before the event
/// stream reads stdin. A colour the terminal did not answer stays absent; the server then uses
/// its own.
///
/// Stdin that is not a terminal answers nothing, so the batch is not written at all: a test
/// runner or a piped stdin would otherwise be sent escape bytes and read for the whole cap.
pub fn ask_terminal_colors(cap: Duration) -> TerminalColors {
    if !std::io::stdin().is_terminal() {
        return TerminalColors::default();
    }
    let mut out = std::io::stdout();
    if out
        .write_all(&attach_batch())
        .and_then(|_| out.flush())
        .is_err()
    {
        return TerminalColors::default();
    }
    colors_from_answers(&read_answers(libc::STDIN_FILENO, cap))
}

/// The colours in what the terminal answered, each found by its code and slot.
pub fn colors_from_answers(text: &str) -> TerminalColors {
    TerminalColors {
        fg: find_answer(text, "10"),
        bg: find_answer(text, "11"),
        palette: std::array::from_fn(|n| find_answer(text, &format!("4;{n}"))),
    }
}

/// Reads answers off a descriptor until the device attributes answer arrives or `cap` passes.
///
/// The descriptor is read directly rather than through `std::io::Stdin`. A `StdinLock` reads
/// into an 8 KB `BufReader`, so a one byte request pulls every byte the terminal sent into a
/// buffer this loop cannot see: `poll` then reports an empty descriptor for the rest of the
/// window, the answers are stranded, and anything the user typed ahead is stranded with them,
/// because the event stream reads the descriptor and never that buffer.
///
/// One byte per read, so the loop stops on the byte that finishes the device attributes answer
/// and leaves whatever follows it - type-ahead - on the descriptor for the event stream.
fn read_answers(fd: std::os::fd::RawFd, cap: Duration) -> String {
    let deadline = Instant::now() + cap;
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
        // A signal, a resize say, interrupts the wait without ending it.
        if n < 0 && interrupted() {
            continue;
        }
        if n <= 0 {
            break;
        }
        // Safe: a one byte buffer this call owns, on a descriptor poll just called readable.
        let n = unsafe { libc::read(fd, byte.as_mut_ptr() as *mut libc::c_void, 1) };
        if n < 0 && interrupted() {
            continue;
        }
        if n <= 0 {
            break;
        }
        buf.push(byte[0]);
        if byte[0] == b'c' && ends_with_device_attributes(&buf) {
            break;
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

/// Whether the call that just failed was interrupted by a signal.
fn interrupted() -> bool {
    std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted
}

/// Whether `buf` ends with a device attributes answer: `ESC [ ?`, digits and `;`, then `c`.
fn ends_with_device_attributes(buf: &[u8]) -> bool {
    let Some((b'c', before)) = buf.split_last() else {
        return false;
    };
    let params = before
        .iter()
        .rev()
        .take_while(|b| b.is_ascii_digit() || **b == b';')
        .count();
    before[..before.len() - params].ends_with(b"\x1b[?")
}

/// The colour in one answer, or `None` when the terminal did not answer that one. `code` is
/// `10`, `11` or `4;n`, and is matched with the `ESC ]` before it and the `;` after it, so slot
/// 1 is not found in slot 10's answer and OSC 10 is not found in `4;10;`. The colour ends at
/// BEL or at the ESC of ST.
fn find_answer(text: &str, code: &str) -> Option<Rgb> {
    let prefix = format!("\x1b]{code};");
    let start = text.find(&prefix)? + prefix.len();
    let rest = &text[start..];
    let end = rest.find(['\x07', '\x1b']).unwrap_or(rest.len());
    parse_osc_color(&rest[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::os::fd::AsRawFd;

    const RED: Rgb = Rgb {
        r: 0xff,
        g: 0x00,
        b: 0x88,
    };
    const FG: Rgb = Rgb {
        r: 0xcd,
        g: 0xd6,
        b: 0xf4,
    };
    const BG: Rgb = Rgb {
        r: 0x1e,
        g: 0x1e,
        b: 0x2e,
    };
    const FG_ANSWER: &[u8] = b"\x1b]10;rgb:cdcd/d6d6/f4f4\x07";
    const BG_ANSWER: &[u8] = b"\x1b]11;rgb:1e1e/1e1e/2e2e\x07";
    const DEVICE_ATTRIBUTES: &[u8] = b"\x1b[?62;22c";

    #[test]
    fn the_attach_batch_asks_both_colours_every_slot_and_ends_with_device_attributes() {
        let mut want = b"\x1b]10;?\x07\x1b]11;?\x07".to_vec();
        for n in 0..16 {
            want.extend(format!("\x1b]4;{n};?\x07").as_bytes());
        }
        want.extend(b"\x1b[c");
        assert_eq!(
            String::from_utf8_lossy(&attach_batch()),
            String::from_utf8_lossy(&want)
        );
    }

    #[test]
    fn parse_osc_color_scales_1_2_3_and_4_digit_channels() {
        assert_eq!(parse_osc_color("rgb:f/0/8"), Some(RED));
        assert_eq!(parse_osc_color("rgb:ff/00/88"), Some(RED));
        assert_eq!(parse_osc_color("rgb:fff/000/888"), Some(RED));
        assert_eq!(parse_osc_color("rgb:ffff/0000/8888"), Some(RED));
        assert_eq!(
            parse_osc_color("rgb:2c2c/2525/2525"),
            Some(Rgb {
                r: 0x2c,
                g: 0x25,
                b: 0x25
            })
        );
        assert_eq!(parse_osc_color("rgb:1e/1e/2e"), Some(BG));
        assert_eq!(parse_osc_color("nonsense"), None);
        assert_eq!(parse_osc_color("rgb:ff/00"), None, "a channel is missing");
        assert_eq!(parse_osc_color("rgb:ff/00/88/00"), None, "one too many");
        assert_eq!(parse_osc_color("rgb:fffff/0/0"), None, "five digits");
        assert_eq!(parse_osc_color("rgb:/0/0"), None, "no digits");
        assert_eq!(parse_osc_color("rgb:+f/0/0"), None, "a sign is not a digit");
    }

    #[test]
    fn detect_reads_truecolor_and_terminal_program() {
        let env = CapsEnv {
            colorterm: Some("truecolor".into()),
            term: Some("xterm-256color".into()),
            term_program: Some("ghostty".into()),
            ssh_tty: None,
            keyboard_enhancement: true,
            probe_took: Some(Duration::from_millis(3)),
        };
        let c = detect(&env);
        assert!(c.truecolor && c.kitty_keyboard && c.hyperlinks && c.osc52);
        let plain = CapsEnv {
            colorterm: None,
            term: Some("xterm".into()),
            term_program: None,
            ssh_tty: None,
            keyboard_enhancement: false,
            probe_took: None,
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

    /// The branch that fires for ghostty and kitty over ssh, where `TERM_PROGRAM` is not
    /// forwarded but `TERM` is - the same case the assertion above calls the only clipboard
    /// route. Deleting the whole `TERM` arm used to leave the workspace green.
    #[test]
    fn detect_reads_a_rich_terminal_from_term_when_term_program_is_not_forwarded() {
        let over_ssh = |term: &str| CapsEnv {
            colorterm: None,
            term: Some(term.into()),
            term_program: None,
            ssh_tty: Some("/dev/pts/3".into()),
            keyboard_enhancement: false,
            probe_took: None,
        };
        for term in ["xterm-ghostty", "xterm-kitty"] {
            let c = detect(&over_ssh(term));
            assert!(c.hyperlinks, "{term} is a rich terminal");
            assert!(c.osc52, "{term}");
        }
        // `TERM` alone does not make a terminal rich: a plain one over ssh keeps OSC 52,
        // which it gets from `SSH_TTY`, and gains nothing else.
        let c = detect(&over_ssh("xterm-256color"));
        assert!(!c.hyperlinks, "xterm-256color is not a rich terminal");
        assert!(c.osc52, "but ssh still routes the clipboard");
    }

    /// Every name in `RICH_TERMINALS`, and the second spelling of truecolor. Three of the
    /// four names and the `24bit` arm were held by nothing.
    ///
    /// The names are written out rather than read from `RICH_TERMINALS`: a test that iterates
    /// the constant it exists to pin shrinks when the constant does, and passes forever.
    #[test]
    fn detect_reads_every_rich_terminal_name_and_both_truecolor_spellings() {
        let local = |program: &str, colorterm: Option<&str>| CapsEnv {
            colorterm: colorterm.map(str::to_string),
            term: Some("xterm-256color".into()),
            term_program: Some(program.into()),
            ssh_tty: None,
            keyboard_enhancement: false,
            probe_took: None,
        };
        for program in ["ghostty", "WezTerm", "iTerm.app", "kitty"] {
            let c = detect(&local(program, None));
            assert!(c.hyperlinks, "{program} is a rich terminal");
            assert!(c.osc52, "{program} can take a clipboard write");
        }
        assert!(!detect(&local("Terminal.app", None)).hyperlinks);
        for spelling in ["truecolor", "24bit"] {
            assert!(
                detect(&local("ghostty", Some(spelling))).truecolor,
                "{spelling}"
            );
        }
        assert!(!detect(&local("ghostty", Some("256"))).truecolor);
    }

    /// A terminal that answers nothing at all. Every capability is off and the colours stay
    /// absent: a default here would paint the screen in colours the terminal never named.
    #[test]
    fn a_terminal_that_says_nothing_gets_no_capabilities_and_no_colours() {
        let silent = CapsEnv {
            colorterm: None,
            term: None,
            term_program: None,
            ssh_tty: None,
            keyboard_enhancement: false,
            probe_took: None,
        };
        assert_eq!(detect(&silent), Capabilities::default());
        assert_eq!(detect(&silent).colors, TerminalColors::default());
        assert_eq!(colors_from_answers(""), TerminalColors::default());
    }

    #[test]
    fn the_cap_is_a_second_past_the_keyboard_probes_answer_and_100_ms_when_it_had_none() {
        assert_eq!(
            attach_cap(Some(Duration::from_millis(3))),
            Duration::from_millis(1003)
        );
        // A terminal that took 1.9 s over the probe takes about as long over the batch.
        assert_eq!(
            attach_cap(Some(Duration::from_millis(1900))),
            Duration::from_millis(2900)
        );
        assert_eq!(attach_cap(None), Duration::from_millis(100));
        assert_eq!(ATTACH_ANSWER_CAP, Duration::from_secs(1));
        assert_eq!(SILENT_TERMINAL_CAP, Duration::from_millis(100));
    }

    #[test]
    fn find_answer_reads_slot_1_without_finding_slot_10_and_ten_without_slot_4_10() {
        let slot_10 = "\x1b]4;10;rgb:ffff/0000/8888\x07";
        assert_eq!(find_answer(slot_10, "4;10"), Some(RED));
        assert_eq!(find_answer(slot_10, "4;1"), None, "slot 1 is not slot 10");
        assert_eq!(find_answer(slot_10, "10"), None, "OSC 10 is not slot 10");
        let both = format!("{slot_10}\x1b]4;1;rgb:cdcd/d6d6/f4f4\x07");
        assert_eq!(find_answer(&both, "4;1"), Some(FG));
        assert_eq!(find_answer(&both, "4;10"), Some(RED));
        let fg = "\x1b]10;rgb:cdcd/d6d6/f4f4\x07";
        assert_eq!(find_answer(fg, "10"), Some(FG));
        assert_eq!(find_answer(fg, "4;10"), None);
        assert_eq!(
            find_answer("", "10"),
            None,
            "no answer is absent, not black"
        );
    }

    #[test]
    fn answers_ended_by_bel_and_by_st_are_both_read() {
        let text = "\x1b]10;rgb:cdcd/d6d6/f4f4\x07\x1b]11;rgb:1e1e/1e1e/2e2e\x1b\\\
                    \x1b]4;3;rgb:ff/00/88\x1b\\\x1b]4;4;rgb:f/0/8\x07";
        let colors = colors_from_answers(text);
        assert_eq!((colors.fg, colors.bg), (Some(FG), Some(BG)));
        assert_eq!(colors.palette[3], Some(RED));
        assert_eq!(colors.palette[4], Some(RED));
        assert_eq!(
            colors.palette.iter().filter(|s| s.is_some()).count(),
            2,
            "a slot nobody answered is absent"
        );
    }

    /// The terminal answers the whole batch in one write, and the user has typed ahead. Every
    /// answer is collected, and the type-ahead is still on the descriptor for the event
    /// stream: a read that buffered would have taken it and lost those keystrokes.
    #[test]
    fn read_answers_stops_at_the_device_attributes_answer_and_leaves_what_follows_it() {
        let (reader, mut writer) = std::io::pipe().unwrap();
        let mut bytes = [FG_ANSWER, BG_ANSWER].concat();
        for n in 0..16 {
            bytes.extend(format!("\x1b]4;{n};rgb:ffff/0000/8888\x07").as_bytes());
        }
        bytes.extend(DEVICE_ATTRIBUTES);
        bytes.extend(b"hi");
        writer.write_all(&bytes).unwrap();
        let text = read_answers(reader.as_raw_fd(), Duration::from_millis(500));
        let colors = colors_from_answers(&text);
        assert_eq!((colors.fg, colors.bg), (Some(FG), Some(BG)));
        assert_eq!(colors.palette, [Some(RED); 16]);
        let mut rest = [0u8; 2];
        let mut reader = reader;
        reader.read_exact(&mut rest).unwrap();
        assert_eq!(&rest, b"hi", "type-ahead stays on the descriptor");
    }

    /// A terminal that answers OSC 10 and 11 and not OSC 4 still answers DA1, and the read
    /// ends there rather than waiting out the cap for slots that are never coming.
    #[test]
    fn read_answers_ends_on_device_attributes_without_waiting_for_slots_the_terminal_did_not_answer(
    ) {
        let (reader, mut writer) = std::io::pipe().unwrap();
        std::thread::spawn(move || {
            writer.write_all(FG_ANSWER).unwrap();
            std::thread::sleep(Duration::from_millis(20));
            writer.write_all(BG_ANSWER).unwrap();
            writer.write_all(DEVICE_ATTRIBUTES).unwrap();
            // Held open past the cap, so only the answer can end the read.
            std::thread::sleep(Duration::from_secs(3));
        });
        let started = Instant::now();
        let text = read_answers(reader.as_raw_fd(), Duration::from_secs(2));
        assert!(
            started.elapsed() < Duration::from_millis(1000),
            "the read waited {:?}",
            started.elapsed()
        );
        let colors = colors_from_answers(&text);
        assert_eq!((colors.fg, colors.bg), (Some(FG), Some(BG)));
        assert_eq!(colors.palette, [None; 16]);
    }

    extern "C" fn ignore_signal(_: libc::c_int) {}

    /// A signal that lands during the read, a window resized while attach waits, interrupts
    /// `poll`. The read carries on to the answers rather than ending there and leaving them to
    /// be typed into the pane.
    #[test]
    fn read_answers_carries_on_past_a_signal_that_interrupts_the_wait() {
        // Safe: a handler that does nothing, installed without SA_RESTART so `poll` sees EINTR,
        // for a signal nothing else in this test binary uses.
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = ignore_signal as *const () as libc::sighandler_t;
            libc::sigemptyset(&mut action.sa_mask);
            assert_eq!(
                libc::sigaction(libc::SIGUSR1, &action, std::ptr::null_mut()),
                0
            );
        }
        let (reader, mut writer) = std::io::pipe().unwrap();
        // Safe: the calling thread's own id.
        let reading = unsafe { libc::pthread_self() };
        let answerer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            // Safe: the reading thread is inside `read_answers` for the next two seconds.
            unsafe { libc::pthread_kill(reading, libc::SIGUSR1) };
            std::thread::sleep(Duration::from_millis(100));
            writer
                .write_all(&[BG_ANSWER, DEVICE_ATTRIBUTES].concat())
                .unwrap();
            std::thread::sleep(Duration::from_secs(3));
        });
        let text = read_answers(reader.as_raw_fd(), Duration::from_secs(2));
        assert_eq!(colors_from_answers(&text).bg, Some(BG), "{text:?}");
        drop(answerer);
    }

    /// A terminal that answers nothing costs the cap and gives nothing. Absent, not black.
    #[test]
    fn read_answers_gives_up_at_the_cap_when_nothing_answers() {
        let (reader, _writer) = std::io::pipe().unwrap();
        let started = Instant::now();
        let text = read_answers(reader.as_raw_fd(), Duration::from_millis(30));
        assert_eq!(text, "");
        assert!(started.elapsed() >= Duration::from_millis(25));
        assert!(started.elapsed() < Duration::from_millis(2000));
        assert_eq!(colors_from_answers(&text), TerminalColors::default());
    }

    /// A `c` typed ahead, and the `c`s inside the answers' own hex digits, end nothing: only
    /// `ESC [ ?`, digits and `;`, then `c` is the device attributes answer.
    #[test]
    fn a_c_that_is_not_a_device_attributes_answer_ends_nothing() {
        let (reader, mut writer) = std::io::pipe().unwrap();
        let bytes = [
            b"c1c;c[c?c".as_slice(),
            b"\x1b]10;rgb:1c1c/2c2c/3c3c\x07",
            BG_ANSWER,
            DEVICE_ATTRIBUTES,
            b"hi",
        ]
        .concat();
        writer.write_all(&bytes).unwrap();
        let text = read_answers(reader.as_raw_fd(), Duration::from_millis(500));
        let colors = colors_from_answers(&text);
        assert_eq!(
            colors.fg,
            Some(Rgb {
                r: 0x1c,
                g: 0x2c,
                b: 0x3c
            })
        );
        assert_eq!(colors.bg, Some(BG));
        assert!(text.ends_with("\x1b[?62;22c"), "{text:?}");
        let mut rest = [0u8; 2];
        let mut reader = reader;
        reader.read_exact(&mut rest).unwrap();
        assert_eq!(&rest, b"hi");
    }
}
