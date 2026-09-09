//! Behaviour the pane depends on, checked against libghostty-vt.

use domux_term::{
    Color, CursorShape, Emulator, EmulatorConfig, GhosttyEmulator, Grid, Key, KeyEvent, Mode, Mods,
    MouseAction, MouseButton, MouseEvent, Rgb, ScrollbackPos, Size,
};

fn make(cols: u16, rows: u16) -> GhosttyEmulator {
    GhosttyEmulator::new(EmulatorConfig {
        size: Size { cols, rows },
        scrollback_lines: 100,
        default_fg: Rgb {
            r: 0xcd,
            g: 0xd6,
            b: 0xf4,
        },
        default_bg: Rgb {
            r: 0x1e,
            g: 0x1e,
            b: 0x2e,
        },
    })
    .expect("ghostty emulator")
}

fn row_text(g: &Grid, row: u16) -> String {
    g.row(row)
        .iter()
        .filter(|c| c.width > 0)
        .map(|c| c.text.as_str())
        .collect::<String>()
        .trim_end()
        .to_string()
}

fn text_of_row(e: &mut dyn Emulator, row: u16) -> String {
    let mut g = Grid::new(e.size());
    e.snapshot_grid(&mut g);
    row_text(&g, row)
}

/// `text_in_range`, asserting that the read itself worked. `None` is the emulator saying it
/// could not read the range, which is a different answer from empty text - the test below
/// that is about that difference calls `text_in_range` directly.
fn text(e: &mut dyn Emulator, start: ScrollbackPos, end: ScrollbackPos) -> String {
    e.text_in_range(start, end)
        .expect("the emulator could not read the range")
}

fn responses(e: &mut dyn Emulator) -> Vec<u8> {
    let mut out = Vec::new();
    e.take_responses(&mut out);
    out
}

fn encode(e: &mut dyn Emulator, key: Key, mods: Mods) -> Vec<u8> {
    let mut out = Vec::new();
    e.encode_key(&KeyEvent::press(key, mods), &mut out);
    out
}

#[test]
fn plain_text_lands_in_the_first_row() {
    let mut e = make(20, 4);
    e.feed(b"hello");
    assert_eq!(text_of_row(&mut e, 0), "hello");
    assert_eq!(e.cursor().col, 5);
}

#[test]
fn erased_cells_keep_the_active_background() {
    let mut e = make(20, 4);
    e.feed(b"\x1b[48;2;10;20;30m\x1b[2K\x1b[2;1H\x1b[48;5;42m\x1b[2K");
    let mut g = Grid::new(e.size());
    e.snapshot_grid(&mut g);

    for col in 0..20 {
        assert_eq!(
            g.cell(0, col).bg,
            Color::Rgb(Rgb {
                r: 10,
                g: 20,
                b: 30
            })
        );
        assert_eq!(g.cell(1, col).bg, Color::Indexed(42));
    }
}

#[test]
fn resize_keeps_text_and_reports_the_new_size() {
    let mut e = make(20, 4);
    e.feed(b"keep me");
    e.resize(Size { cols: 30, rows: 6 });
    assert_eq!(e.size(), Size { cols: 30, rows: 6 });
    assert_eq!(text_of_row(&mut e, 0), "keep me");
}

#[test]
fn cursor_position_report_is_answered() {
    let mut e = make(20, 4);
    e.feed(b"ab\x1b[6n");
    assert_eq!(responses(&mut e), b"\x1b[1;3R".to_vec());
}

#[test]
fn primary_device_attributes_are_answered() {
    let mut e = make(20, 4);
    e.feed(b"\x1b[c");
    let r = responses(&mut e);
    assert!(
        r.starts_with(b"\x1b[?"),
        "{:?}",
        String::from_utf8_lossy(&r)
    );
    assert!(r.ends_with(b"c"));
}

#[test]
fn responses_are_drained_once() {
    let mut e = make(20, 4);
    e.feed(b"\x1b[6n");
    assert!(!responses(&mut e).is_empty());
    assert!(responses(&mut e).is_empty());
}

#[test]
fn arrow_keys_follow_application_cursor_mode() {
    let mut e = make(20, 4);
    assert_eq!(encode(&mut e, Key::Up, Mods::empty()), b"\x1b[A".to_vec());
    e.feed(b"\x1b[?1h");
    assert_eq!(encode(&mut e, Key::Up, Mods::empty()), b"\x1bOA".to_vec());
}

#[test]
fn control_and_function_keys_use_legacy_encoding_by_default() {
    let mut e = make(20, 4);
    assert_eq!(encode(&mut e, Key::Char('c'), Mods::CTRL), b"\x03".to_vec());
    assert_eq!(encode(&mut e, Key::Enter, Mods::empty()), b"\r".to_vec());
    assert_eq!(
        encode(&mut e, Key::F(5), Mods::empty()),
        b"\x1b[15~".to_vec()
    );
    assert_eq!(encode(&mut e, Key::Char('a'), Mods::ALT), b"\x1ba".to_vec());
}

#[test]
fn shift_enter_is_distinguishable_once_kitty_flags_are_pushed() {
    let mut e = make(20, 4);
    assert_eq!(encode(&mut e, Key::Enter, Mods::empty()), b"\r".to_vec());
    // Legacy shift+enter follows Ghostty's own function key table (CSI 27;2;13~, see
    // src/input/function_keys.zig in the pinned Ghostty).
    assert!(!encode(&mut e, Key::Enter, Mods::SHIFT).is_empty());
    e.feed(b"\x1b[>1u"); // push disambiguate escape codes
                         // CSI 13;2u is what kitty specifies and what makes shift+enter work in Claude Code.
    assert_eq!(
        encode(&mut e, Key::Enter, Mods::SHIFT),
        b"\x1b[13;2u".to_vec()
    );
}

#[test]
fn paste_is_bracketed_only_when_requested() {
    let mut e = make(20, 4);
    let mut out = Vec::new();
    e.encode_paste("hi", &mut out);
    assert_eq!(out, b"hi".to_vec());
    e.feed(b"\x1b[?2004h");
    out.clear();
    e.encode_paste("hi", &mut out);
    assert_eq!(out, b"\x1b[200~hi\x1b[201~".to_vec());
}

#[test]
fn wide_grapheme_occupies_two_cells() {
    let mut e = make(20, 4);
    e.feed("漢x".as_bytes());
    let mut g = Grid::new(e.size());
    e.snapshot_grid(&mut g);
    assert_eq!(g.cell(0, 0).text, "漢");
    assert_eq!(g.cell(0, 0).width, 2);
    assert_eq!(g.cell(0, 1).width, 0);
    assert_eq!(g.cell(0, 2).text, "x");
}

#[test]
fn a_long_grapheme_cluster_is_read_whole() {
    // A base character with far more combining marks than a small fixed buffer would hold.
    // The emulator must return every codepoint in one cell.
    let mut e = make(20, 4);
    let mut input = String::from("a");
    for _ in 0..40 {
        input.push('\u{0301}'); // combining acute accent
    }
    e.feed(input.as_bytes());
    let mut g = Grid::new(e.size());
    e.snapshot_grid(&mut g);
    let text = g.cell(0, 0).text.as_str();
    assert!(text.starts_with('a'), "{text:?}");
    assert!(
        text.chars().filter(|c| *c == '\u{0301}').count() >= 16,
        "expected the whole cluster, got {} marks",
        text.chars().filter(|c| *c == '\u{0301}').count()
    );
}

#[test]
fn cursor_shape_follows_decscusr() {
    let mut e = make(20, 4);
    e.feed(b"\x1b[5 q"); // blinking bar
    assert_eq!(e.cursor().shape, CursorShape::Bar);
    assert!(e.cursor().blink);
    e.feed(b"\x1b[?25l");
    assert!(!e.cursor().visible);
}

#[test]
fn scrollback_grows_as_lines_scroll_off_and_snapshot_at_offset_shows_them() {
    let mut e = make(10, 3);
    // A fresh emulator has no history at all.
    assert_eq!(e.scrollback_len(), 0);
    for i in 0..6 {
        e.feed(format!("line{i}\r\n").as_bytes());
    }
    // Six lines were written into three rows plus one trailing newline: the
    // visible rows are line4, line5, blank; line0 to line3 are in the scrollback.
    assert_eq!(e.scrollback_len(), 4);
    let mut g = Grid::new(e.size());
    e.snapshot_grid_at(0, &mut g);
    assert_eq!(row_text(&g, 0), "line4");
    e.snapshot_grid_at(2, &mut g);
    assert_eq!(row_text(&g, 0), "line2");
    assert_eq!(row_text(&g, 1), "line3");
    assert_eq!(row_text(&g, 2), "line4");
    e.snapshot_grid_at(4, &mut g);
    assert_eq!(row_text(&g, 0), "line0");
    // An offset past the top clamps to the top.
    e.snapshot_grid_at(99, &mut g);
    assert_eq!(row_text(&g, 0), "line0");
}

#[test]
fn text_in_range_joins_rows_with_newlines_and_trims_trailing_blanks() {
    let mut e = make(10, 3);
    for i in 0..6 {
        e.feed(format!("line{i}\r\n").as_bytes());
    }
    let joined = text(
        &mut e,
        ScrollbackPos { row: 1, col: 2 },
        ScrollbackPos { row: 3, col: 1 },
    );
    assert_eq!(joined, "ne1\nline2\nli");
    // Row `scrollback_len()` is the first visible row, so row 4 is the live screen's top row.
    let whole_row = text(
        &mut e,
        ScrollbackPos { row: 4, col: 0 },
        ScrollbackPos { row: 4, col: 9 },
    );
    assert_eq!(whole_row, "line4");
}

#[test]
fn title_follows_osc_0_and_2() {
    let mut e = make(20, 4);
    assert_eq!(e.title(), None);
    e.feed(b"\x1b]2;nvim main.rs\x07");
    assert_eq!(e.title(), Some("nvim main.rs".to_string()));
    e.feed(b"\x1b]0;zsh\x1b\\");
    assert_eq!(e.title(), Some("zsh".to_string()));
    // An empty title is the library's own encoding of "no title", so it reads as absent.
    e.feed(b"\x1b]2;\x07");
    assert_eq!(e.title(), None);
}

#[test]
fn cwd_follows_osc_7() {
    let mut e = make(20, 4);
    assert_eq!(e.cwd(), None);
    e.feed(b"\x1b]7;file://localhost/Users/pranav/projects\x1b\\");
    assert_eq!(
        e.cwd(),
        Some(std::path::PathBuf::from("/Users/pranav/projects"))
    );
    e.feed(b"\x1b]7;file:///tmp/a%20b\x07");
    assert_eq!(e.cwd(), Some(std::path::PathBuf::from("/tmp/a b")));
}

#[test]
fn bell_is_reported_once() {
    let mut e = make(20, 4);
    assert!(!e.take_bell());
    e.feed(b"ding\x07");
    assert!(e.take_bell());
    assert!(!e.take_bell());
}

#[test]
fn modes_report_alt_screen_bracketed_paste_focus_and_app_cursor() {
    let mut e = make(20, 4);
    assert!(!e.mode_active(Mode::AltScreen));
    e.feed(b"\x1b[?1049h");
    assert!(e.mode_active(Mode::AltScreen));
    e.feed(b"\x1b[?1049l");
    assert!(!e.mode_active(Mode::AltScreen));
    e.feed(b"\x1b[?2004h");
    assert!(e.mode_active(Mode::BracketedPaste));
    e.feed(b"\x1b[?1004h");
    assert!(e.mode_active(Mode::FocusEvents));
    e.feed(b"\x1b[?1h");
    assert!(e.mode_active(Mode::AppCursor));
}

#[test]
fn focus_is_encoded_only_when_the_program_asked_for_it() {
    let mut e = make(20, 4);
    let mut out = Vec::new();
    e.encode_focus(true, &mut out);
    assert!(out.is_empty());
    e.feed(b"\x1b[?1004h");
    e.encode_focus(true, &mut out);
    assert_eq!(out, b"\x1b[I".to_vec());
    out.clear();
    e.encode_focus(false, &mut out);
    assert_eq!(out, b"\x1b[O".to_vec());
}

#[test]
fn alt_screen_is_reported_for_the_legacy_47_switch_too() {
    // Mode 1049's bit is not set when a program switches with 47, so the alternate screen is
    // read from the active screen rather than from a mode bit.
    let mut e = make(20, 4);
    e.feed(b"\x1b[?47h");
    assert!(e.mode_active(Mode::AltScreen));
    e.feed(b"\x1b[?47l");
    assert!(!e.mode_active(Mode::AltScreen));
}

#[test]
fn text_in_range_spans_more_rows_than_the_screen_and_clamps_past_the_end() {
    let mut e = make(10, 3);
    for i in 0..6 {
        e.feed(format!("line{i}\r\n").as_bytes());
    }
    // Seven rows exist: line0 to line5 in rows 0 to 5, then the blank row the cursor sits on.
    // A position past the end clamps onto the last row, and the blank row it lands on adds
    // nothing: selecting past the end of the text does not copy a phantom blank line.
    let all = text(
        &mut e,
        ScrollbackPos { row: 0, col: 0 },
        ScrollbackPos { row: 99, col: 99 },
    );
    assert_eq!(all, "line0\nline1\nline2\nline3\nline4\nline5");
    // The same range stopping on the last row of text reads the same.
    assert_eq!(
        text(
            &mut e,
            ScrollbackPos { row: 0, col: 0 },
            ScrollbackPos { row: 5, col: 9 }
        ),
        all
    );
}

#[test]
fn text_in_range_keeps_a_blank_row_inside_the_range_and_drops_trailing_ones() {
    let mut e = make(10, 4);
    e.feed(b"line0\r\n\r\nline2\r\n");
    // A blank row between two rows of text still ends its line.
    assert_eq!(
        text(
            &mut e,
            ScrollbackPos { row: 0, col: 0 },
            ScrollbackPos { row: 3, col: 9 }
        ),
        "line0\n\nline2"
    );
}

#[test]
fn text_in_range_reads_a_whole_wide_grapheme_from_either_of_its_cells() {
    let mut e = make(10, 3);
    e.feed("a\u{6f22}b".as_bytes());
    assert_eq!(
        text(
            &mut e,
            ScrollbackPos { row: 0, col: 0 },
            ScrollbackPos { row: 0, col: 9 }
        ),
        "a\u{6f22}b"
    );
    // Column 1 holds the grapheme and column 2 its zero-width spacer. Either cell reads as
    // the whole grapheme, never as half of one and never as empty.
    assert_eq!(
        text(
            &mut e,
            ScrollbackPos { row: 0, col: 1 },
            ScrollbackPos { row: 0, col: 1 }
        ),
        "\u{6f22}"
    );
    assert_eq!(
        text(
            &mut e,
            ScrollbackPos { row: 0, col: 2 },
            ScrollbackPos { row: 0, col: 2 }
        ),
        "\u{6f22}"
    );
}

#[test]
fn text_in_range_reads_a_reversed_range_as_the_forward_one() {
    let mut e = make(10, 3);
    for i in 0..5 {
        e.feed(format!("line{i}\r\n").as_bytes());
    }
    // No trailing newline, so the last row that exists holds text. The clamp case below
    // needs that: against the blank row a trailing newline leaves, both directions read ""
    // and the case pins nothing.
    e.feed(b"tail");
    // Reversed by row, and reversed by column within one row. A drag upward or leftward
    // produces exactly these, so both read as the range with its endpoints swapped.
    assert_eq!(
        text(
            &mut e,
            ScrollbackPos { row: 3, col: 0 },
            ScrollbackPos { row: 1, col: 0 }
        ),
        "line1\nline2\nl"
    );
    assert_eq!(
        text(
            &mut e,
            ScrollbackPos { row: 1, col: 4 },
            ScrollbackPos { row: 1, col: 1 }
        ),
        "ine1"
    );
    // Two rows past the end clamp onto the same row, which leaves the columns reversed even
    // though the range as given was not. It still reads as the forward range. Both sides are
    // asserted against the text, not against each other, so neither can pass by both being
    // empty.
    assert_eq!(
        text(
            &mut e,
            ScrollbackPos { row: 100, col: 0 },
            ScrollbackPos { row: 200, col: 9 },
        ),
        "tail"
    );
    assert_eq!(
        text(
            &mut e,
            ScrollbackPos { row: 100, col: 9 },
            ScrollbackPos { row: 200, col: 0 },
        ),
        "tail",
        "the clamp collapsed the rows and reversed the columns, and the swap undoes that"
    );
}

#[test]
fn text_in_range_reads_the_screen_when_there_is_no_scrollback() {
    // A fresh terminal and the alternate screen both have no history, so every row of the
    // range is a screen row.
    let mut e = make(10, 3);
    e.feed(b"hi");
    assert_eq!(e.scrollback_len(), 0);
    assert_eq!(
        text(
            &mut e,
            ScrollbackPos { row: 0, col: 0 },
            ScrollbackPos { row: 2, col: 9 }
        ),
        "hi"
    );

    let mut alt = make(10, 3);
    for i in 0..6 {
        alt.feed(format!("line{i}\r\n").as_bytes());
    }
    alt.feed(b"\x1b[?1049h");
    alt.feed(b"alt");
    assert!(alt.mode_active(Mode::AltScreen));
    assert_eq!(alt.scrollback_len(), 0);
    // The cursor kept its row, so the alternate screen holds two blank rows then the text.
    assert_eq!(
        text(
            &mut alt,
            ScrollbackPos { row: 0, col: 0 },
            ScrollbackPos { row: 2, col: 9 }
        ),
        "\n\nalt"
    );
}

#[test]
fn reading_the_scrollback_leaves_the_live_screen_where_it_was() {
    let mut e = make(10, 3);
    for i in 0..6 {
        e.feed(format!("line{i}\r\n").as_bytes());
    }
    let mut g = Grid::new(e.size());
    e.snapshot_grid_at(4, &mut g);
    text(
        &mut e,
        ScrollbackPos { row: 0, col: 0 },
        ScrollbackPos { row: 2, col: 9 },
    );
    e.snapshot_grid(&mut g);
    assert_eq!(row_text(&g, 0), "line4");
    assert_eq!(row_text(&g, 1), "line5");
    assert_eq!(e.cursor().row, 2);
}

#[test]
fn text_in_range_rejoins_a_soft_wrapped_line() {
    let mut e = make(10, 3);
    // Fifteen cells of text on a ten-column screen: the terminal broke the line, the writer
    // did not. Copying it back has to give the line that was written, not the two rows it
    // was drawn as - a wrapped URL pastes as one URL.
    e.feed(b"abcdefghijklmno\r\nend");
    assert_eq!(
        text(
            &mut e,
            ScrollbackPos { row: 0, col: 0 },
            ScrollbackPos { row: 2, col: 9 }
        ),
        "abcdefghijklmno\nend"
    );
}

/// The two answers a caller must not collapse: a range that holds no text reads as empty
/// text, and a read that could not happen reads as absent. Copy mode tells them apart -
/// one leaves the clipboard alone and says the selection was blank, the other says the read
/// failed - and it can only do that if the emulator does.
#[test]
fn text_in_range_reads_a_blank_range_as_empty_text_and_a_failed_read_as_absent() {
    let mut e = make(10, 3);
    e.feed(b"hi");
    assert_eq!(
        e.text_in_range(
            ScrollbackPos { row: 1, col: 0 },
            ScrollbackPos { row: 2, col: 9 }
        ),
        Some(String::new()),
        "blank rows hold no text, and that is an answer"
    );
    // A screen with no cells cannot be read at all. The empty string would be a fabricated
    // answer here: nothing was read, so nothing is reported.
    e.resize(Size { cols: 0, rows: 0 });
    assert_eq!(
        e.text_in_range(
            ScrollbackPos { row: 0, col: 0 },
            ScrollbackPos { row: 0, col: 0 }
        ),
        None
    );
}

/// The wheel over a pane belongs to the pane's program when the program asked for the mouse
/// (decision 0013), so the one question the server asks has to answer for every tracking mode
/// a program can set, and has to be false before any of them is set.
#[test]
fn mouse_tracking_is_active_for_every_tracking_mode_a_program_can_set() {
    for enable in [
        &b"\x1b[?9h"[..],
        &b"\x1b[?1000h"[..],
        &b"\x1b[?1002h"[..],
        &b"\x1b[?1003h"[..],
    ] {
        let mut e = make(20, 5);
        assert!(
            !e.mode_active(Mode::MouseTracking),
            "a program that asked for nothing has not asked for the mouse"
        );
        e.feed(enable);
        assert!(
            e.mode_active(Mode::MouseTracking),
            "{} turns mouse tracking on",
            String::from_utf8_lossy(&enable[1..])
        );
    }
}

/// A program that asked for no mouse is sent no mouse. Anything else would type rubbish into
/// its input: a program that never enabled tracking reads a report as the characters it is
/// made of.
#[test]
fn encode_mouse_writes_nothing_when_the_program_asked_for_no_mouse() {
    let mut e = make(20, 5);
    let mut out = Vec::new();
    e.encode_mouse(&wheel_up(2, 3), &mut out);
    assert!(out.is_empty(), "got {out:?}");
}

/// The report Claude Code answered with a redraw when it was sent by hand, which is the
/// gesture this whole change exists for. SGR because the program asked for SGR; the columns
/// and rows are one-based in the protocol and zero-based in the pane.
#[test]
fn encode_mouse_writes_the_sgr_wheel_report_the_program_asked_for() {
    let mut e = make(20, 5);
    e.feed(b"\x1b[?1000h\x1b[?1006h");
    let mut out = Vec::new();
    e.encode_mouse(&wheel_up(2, 3), &mut out);
    assert_eq!(String::from_utf8_lossy(&out), "\x1b[<64;4;3M");
    out.clear();
    e.encode_mouse(
        &MouseEvent {
            button: MouseButton::WheelDown,
            action: MouseAction::Press,
            mods: Mods::empty(),
            row: 0,
            col: 0,
        },
        &mut out,
    );
    assert_eq!(String::from_utf8_lossy(&out), "\x1b[<65;1;1M");
}

/// Normal tracking with no SGR is the older report, and the format is the program's choice
/// rather than domux2's: the encoder is configured from the terminal every time.
#[test]
fn encode_mouse_follows_the_report_format_the_program_set() {
    let mut e = make(20, 5);
    e.feed(b"\x1b[?1000h");
    let mut out = Vec::new();
    e.encode_mouse(&wheel_up(2, 3), &mut out);
    assert_eq!(
        out,
        b"\x1b[M\x60$#".to_vec(),
        "x10 encoding: 32 + 64 for wheel up, then column and row at 32 + one-based"
    );
}

fn wheel_up(row: u16, col: u16) -> MouseEvent {
    MouseEvent {
        button: MouseButton::WheelUp,
        action: MouseAction::Press,
        mods: Mods::empty(),
        row,
        col,
    }
}

/// What a triple click selects. A line the screen wrapped is one line, however many rows it
/// was drawn on, and a row the writer ended is one row however full it is.
#[test]
fn logical_line_covers_every_row_a_wrapped_line_was_drawn_on() {
    let mut e = make(5, 4);
    // 12 cells with no break in them: rows 0, 1 and 2 are one line. Then a line of its own.
    e.feed(b"abcdefghijkl\r\nend");
    assert_eq!(e.logical_line(0), (0, 2), "asked about the first row");
    assert_eq!(e.logical_line(1), (0, 2), "asked about the middle row");
    assert_eq!(e.logical_line(2), (0, 2), "asked about the last row");
    assert_eq!(e.logical_line(3), (3, 3), "the line after it stands alone");
}

/// A row that exactly fills the width is not a wrapped line: the writer ended it. Nothing but
/// the emulator's own wrap flag can tell the two apart, which is why this question is asked of
/// the emulator rather than measured from the text.
#[test]
fn logical_line_is_one_row_when_a_line_exactly_fills_the_width() {
    let mut e = make(5, 4);
    e.feed(b"abcde\r\nfg");
    assert_eq!(e.logical_line(0), (0, 0));
    assert_eq!(e.logical_line(1), (1, 1));
}

/// A row past the end of the screen clamps rather than answering about a row that is not
/// there, the same clamp `text_in_range` makes.
#[test]
fn logical_line_clamps_a_row_past_the_end() {
    let mut e = make(5, 3);
    e.feed(b"hi");
    let last = e.scrollback_len() + 2;
    assert_eq!(e.logical_line(9_999), (last, last));
}
