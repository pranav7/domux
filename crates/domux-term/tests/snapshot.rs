//! A pane's screen crosses an upgrade as a Ghostty snapshot (decision 0045). What comes back
//! must be the screen that went in: the cells, the cursor, the scrollback, the title, the
//! working directory, the modes, and a sequence the program was half way through writing.

use domux_term::{
    Color, Emulator, EmulatorConfig, GhosttyEmulator, Grid, Mode, Rgb, ScrollbackPos, Size,
};

fn config(cols: u16, rows: u16) -> EmulatorConfig {
    EmulatorConfig {
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
    }
}

fn make(cols: u16, rows: u16) -> GhosttyEmulator {
    GhosttyEmulator::new(config(cols, rows)).expect("ghostty emulator")
}

/// Encodes `e` and decodes the bytes into a second emulator, the way an upgrade does.
fn carried(e: &GhosttyEmulator) -> GhosttyEmulator {
    let bytes = e.encode_snapshot().expect("the screen encodes");
    GhosttyEmulator::decode_snapshot(&bytes, 100).expect("the screen decodes")
}

fn grid(e: &mut GhosttyEmulator) -> Grid {
    let mut g = Grid::new(e.size());
    e.snapshot_grid(&mut g);
    g
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

#[test]
fn a_decoded_snapshot_draws_the_screen_it_was_encoded_from() {
    let mut e = make(20, 4);
    for n in 0..10 {
        e.feed(format!("line {n}\r\n").as_bytes());
    }
    e.feed(b"\x1b[31mred\x1b[0m plain");
    e.feed(b"\x1b]2;the title\x07");
    e.feed(b"\x1b]7;file://host/tmp/work\x07");
    e.feed(b"\x1b[?2004h");
    let mut back = carried(&e);
    assert_eq!(
        back.size(),
        Size { cols: 20, rows: 4 },
        "the size comes from the snapshot"
    );
    assert_eq!(grid(&mut back), grid(&mut e), "every cell");
    assert_eq!(back.cursor(), e.cursor());
    assert_eq!(back.scrollback_len(), e.scrollback_len());
    let top = ScrollbackPos { row: 0, col: 0 };
    let end = ScrollbackPos { row: 2, col: 19 };
    assert_eq!(
        back.text_in_range(top, end),
        e.text_in_range(top, end),
        "the scrollback"
    );
    assert_eq!(back.title().as_deref(), Some("the title"));
    assert_eq!(back.cwd(), e.cwd());
    assert!(back.mode_active(Mode::BracketedPaste), "the modes");
}

#[test]
fn a_decoded_snapshot_finishes_a_sequence_the_program_was_half_way_through() {
    let mut e = make(20, 4);
    // The reader stops between two chunks, and nothing says a chunk ends at ground.
    e.feed(b"\x1b[3");
    let mut back = carried(&e);
    back.feed(b"1mred");
    let g = grid(&mut back);
    assert_eq!(row_text(&g, 0), "red");
    assert_eq!(
        g.row(0)[0].fg,
        Color::Indexed(1),
        "the escape the snapshot held applied"
    );
}

#[test]
fn a_decoded_snapshot_keeps_the_alternate_screen_and_the_primary_under_it() {
    let mut e = make(20, 4);
    e.feed(b"primary");
    e.feed(b"\x1b[?1049h\x1b[Halternate");
    let mut back = carried(&e);
    assert!(back.mode_active(Mode::AltScreen));
    assert_eq!(row_text(&grid(&mut back), 0), "alternate");
    back.feed(b"\x1b[?1049l");
    assert_eq!(row_text(&grid(&mut back), 0), "primary");
}

#[test]
fn a_decoded_snapshot_answers_the_program() {
    let e = make(20, 4);
    let mut back = carried(&e);
    back.feed(b"\x1b[6n");
    let mut out = Vec::new();
    back.take_responses(&mut out);
    assert_eq!(
        out, b"\x1b[1;1R",
        "the cursor position report reaches the program"
    );
    back.feed(b"\x07");
    assert!(back.take_bell(), "and so does the bell");
}

#[test]
fn a_decoded_snapshot_can_be_carried_again() {
    let mut e = make(20, 4);
    e.feed(b"once\r\n\x1b[3");
    let twice = carried(&carried(&e));
    let mut twice = twice;
    twice.feed(b"2mgreen");
    let g = grid(&mut twice);
    assert_eq!(row_text(&g, 0), "once");
    assert_eq!(g.row(1)[0].fg, Color::Indexed(2));
}

#[test]
fn bytes_that_are_not_a_snapshot_are_refused() {
    assert!(GhosttyEmulator::decode_snapshot(b"not a snapshot", 100).is_err());
    assert!(GhosttyEmulator::decode_snapshot(&[], 100).is_err());
}
