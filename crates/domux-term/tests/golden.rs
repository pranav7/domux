//! Golden fixtures run against every enabled implementation.
//!
//! One golden per fixture, produced by libghostty-vt and reviewed by hand. Where
//! alacritty_terminal disagrees the difference is a finding for the decision record, not a
//! second golden. Findings recorded during M0, all emulator-attributable:
//!
//! - finding: sgr-attributes. alacritty_terminal drops SGR 21 (double underline), SGR 53
//!   (overline), and SGR 5 (blink); libghostty-vt keeps all three. Its cell flags have no
//!   blink or overline bit at all.
//! - finding: cursor-and-erase. A horizontal tab leaves a literal TAB character in an
//!   alacritty_terminal cell; libghostty-vt advances the cursor and leaves spaces, which is
//!   what a grid should hold.
//! - finding: wide-and-emoji. Regional indicator flags (for example the two codepoints of
//!   the Japan flag) are two wide cells in libghostty-vt and two narrow cells in
//!   alacritty_terminal, so a row of flags ends two columns apart. Every other grapheme
//!   tested (ZWJ sequences, skin tones, variation selectors, CJK) agrees.

use domux_term::golden::list_fixtures;

#[cfg(any(feature = "ghostty", feature = "alacritty"))]
use domux_term::golden::{check, check_chunking};
#[cfg(any(feature = "ghostty", feature = "alacritty"))]
use domux_term::{new_emulator, EmulatorConfig, EmulatorKind, Rgb};

#[cfg(any(feature = "ghostty", feature = "alacritty"))]
fn config(size: domux_term::Size) -> EmulatorConfig {
    EmulatorConfig {
        size,
        scrollback_lines: 1000,
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

#[cfg(any(feature = "ghostty", feature = "alacritty"))]
fn run_all(kind: EmulatorKind) {
    let fixtures = list_fixtures();
    assert!(!fixtures.is_empty(), "no fixtures under fixtures/golden");
    let mut failures = Vec::new();
    for fixture in &fixtures {
        let mut emulator = new_emulator(kind, config(fixture.size)).expect("emulator enabled");
        if let Err(e) = check(fixture, emulator.as_mut()) {
            failures.push(format!("{}: {e}", fixture.name));
        }
        let mut make = || new_emulator(kind, config(fixture.size)).expect("emulator enabled");
        if let Err(e) = check_chunking(fixture, &mut make) {
            failures.push(format!("{} (chunking): {e}", fixture.name));
        }
    }
    assert!(
        failures.is_empty(),
        "golden mismatches for {kind:?}:\n{}",
        failures.join("\n")
    );
}

#[cfg(feature = "ghostty")]
#[test]
fn every_fixture_matches_its_golden_with_ghostty() {
    run_all(EmulatorKind::Ghostty);
}

#[cfg(feature = "alacritty")]
#[test]
#[ignore = "3 fixtures differ: SGR 21/53/5 dropped, tab left in the cell, flag width. See the findings at the top of this file and docs/decisions/0001-terminal-emulator.md"]
fn every_fixture_matches_its_golden_with_alacritty() {
    run_all(EmulatorKind::Alacritty);
}

#[test]
fn fixture_names_parse_their_size() {
    let f = list_fixtures()
        .into_iter()
        .find(|f| f.name == "sgr-attributes")
        .expect("fixture");
    assert_eq!(f.size, domux_term::Size { cols: 80, rows: 24 });
}
