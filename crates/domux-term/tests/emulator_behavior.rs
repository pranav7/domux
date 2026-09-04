use domux_term::{
    new_emulator, Emulator, EmulatorConfig, EmulatorKind, Grid, Key, KeyEvent, Mods, Rgb, Size,
};

#[allow(dead_code)]
fn make(kind: EmulatorKind, cols: u16, rows: u16) -> Box<dyn Emulator> {
    new_emulator(
        kind,
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
        },
    )
    .expect("emulator enabled")
}

#[allow(dead_code)]
fn text_of_row(e: &mut dyn Emulator, row: u16) -> String {
    let mut g = Grid::new(e.size());
    e.snapshot_grid(&mut g);
    g.row(row)
        .iter()
        .filter(|c| c.width > 0)
        .map(|c| c.text.as_str())
        .collect::<String>()
        .trim_end()
        .to_string()
}

#[allow(dead_code)]
fn responses(e: &mut dyn Emulator) -> Vec<u8> {
    let mut out = Vec::new();
    e.take_responses(&mut out);
    out
}

#[allow(dead_code)]
fn encode(e: &mut dyn Emulator, key: Key, mods: Mods) -> Vec<u8> {
    let mut out = Vec::new();
    e.encode_key(&KeyEvent::press(key, mods), &mut out);
    out
}

macro_rules! behavior_tests {
    ($module:ident, $kind:expr) => {
        mod $module {
            use super::*;

            #[test]
            fn plain_text_lands_in_the_first_row() {
                let mut e = make($kind, 20, 4);
                e.feed(b"hello");
                assert_eq!(text_of_row(e.as_mut(), 0), "hello");
                assert_eq!(e.cursor().col, 5);
            }

            #[test]
            fn resize_keeps_text_and_reports_the_new_size() {
                let mut e = make($kind, 20, 4);
                e.feed(b"keep me");
                e.resize(Size { cols: 30, rows: 6 });
                assert_eq!(e.size(), Size { cols: 30, rows: 6 });
                assert_eq!(text_of_row(e.as_mut(), 0), "keep me");
            }

            #[test]
            fn cursor_position_report_is_answered() {
                let mut e = make($kind, 20, 4);
                e.feed(b"ab\x1b[6n");
                assert_eq!(responses(e.as_mut()), b"\x1b[1;3R".to_vec());
            }

            #[test]
            fn primary_device_attributes_are_answered() {
                let mut e = make($kind, 20, 4);
                e.feed(b"\x1b[c");
                let r = responses(e.as_mut());
                assert!(
                    r.starts_with(b"\x1b[?"),
                    "{:?}",
                    String::from_utf8_lossy(&r)
                );
                assert!(r.ends_with(b"c"));
            }

            #[test]
            fn responses_are_drained_once() {
                let mut e = make($kind, 20, 4);
                e.feed(b"\x1b[6n");
                assert!(!responses(e.as_mut()).is_empty());
                assert!(responses(e.as_mut()).is_empty());
            }

            #[test]
            fn arrow_keys_follow_application_cursor_mode() {
                let mut e = make($kind, 20, 4);
                assert_eq!(
                    encode(e.as_mut(), Key::Up, Mods::empty()),
                    b"\x1b[A".to_vec()
                );
                e.feed(b"\x1b[?1h");
                assert_eq!(
                    encode(e.as_mut(), Key::Up, Mods::empty()),
                    b"\x1bOA".to_vec()
                );
            }

            #[test]
            fn control_and_function_keys_use_legacy_encoding_by_default() {
                let mut e = make($kind, 20, 4);
                assert_eq!(
                    encode(e.as_mut(), Key::Char('c'), Mods::CTRL),
                    b"\x03".to_vec()
                );
                assert_eq!(
                    encode(e.as_mut(), Key::Enter, Mods::empty()),
                    b"\r".to_vec()
                );
                assert_eq!(
                    encode(e.as_mut(), Key::F(5), Mods::empty()),
                    b"\x1b[15~".to_vec()
                );
                assert_eq!(
                    encode(e.as_mut(), Key::Char('a'), Mods::ALT),
                    b"\x1ba".to_vec()
                );
            }

            #[test]
            fn shift_enter_is_distinguishable_once_kitty_flags_are_pushed() {
                let mut e = make($kind, 20, 4);
                assert_eq!(
                    encode(e.as_mut(), Key::Enter, Mods::empty()),
                    b"\r".to_vec()
                );
                // Legacy shift+enter differs by emulator: libghostty-vt follows Ghostty's
                // own function key table (CSI 27;2;13~, see vendor/ghostty
                // src/input/function_keys.zig), termwiz sends a plain CR. Both encode
                // something; the exact bytes are a row in the decision record.
                assert!(!encode(e.as_mut(), Key::Enter, Mods::SHIFT).is_empty());
                e.feed(b"\x1b[>1u"); // push disambiguate escape codes
                assert_eq!(
                    encode(e.as_mut(), Key::Enter, Mods::SHIFT),
                    b"\x1b[13;2u".to_vec()
                );
            }

            #[test]
            fn paste_is_bracketed_only_when_requested() {
                let mut e = make($kind, 20, 4);
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
                let mut e = make($kind, 20, 4);
                e.feed("漢x".as_bytes());
                let mut g = Grid::new(e.size());
                e.snapshot_grid(&mut g);
                assert_eq!(g.cell(0, 0).text, "漢");
                assert_eq!(g.cell(0, 0).width, 2);
                assert_eq!(g.cell(0, 1).width, 0);
                assert_eq!(g.cell(0, 2).text, "x");
            }

            #[test]
            fn cursor_shape_follows_decscusr() {
                let mut e = make($kind, 20, 4);
                e.feed(b"\x1b[5 q"); // blinking bar
                assert_eq!(e.cursor().shape, domux_term::CursorShape::Bar);
                assert!(e.cursor().blink);
                e.feed(b"\x1b[?25l");
                assert!(!e.cursor().visible);
            }
        }
    };
}

#[cfg(feature = "ghostty")]
behavior_tests!(ghostty, EmulatorKind::Ghostty);
#[cfg(feature = "alacritty")]
behavior_tests!(alacritty, EmulatorKind::Alacritty);
