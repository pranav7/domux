//! Golden fixtures, one per fixture, produced by libghostty-vt and reviewed by hand.
//!
//! M0 compared these against alacritty_terminal and found five emulator differences, two of
//! them visible in daily programs. That comparison chose the engine and is recorded in
//! docs/decisions/0001-terminal-emulator.md; the second implementation is gone.

use domux_term::golden::{check, check_chunking, list_fixtures};
use domux_term::{EmulatorConfig, GhosttyEmulator, Rgb};

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

#[test]
fn every_fixture_matches_its_golden() {
    let fixtures = list_fixtures();
    assert!(!fixtures.is_empty(), "no fixtures under fixtures/golden");
    let mut failures = Vec::new();
    for fixture in &fixtures {
        let mut emulator = GhosttyEmulator::new(config(fixture.size)).expect("ghostty emulator");
        if let Err(e) = check(fixture, &mut emulator) {
            failures.push(format!("{}: {e}", fixture.name));
        }
        let mut make = || GhosttyEmulator::new(config(fixture.size)).expect("ghostty emulator");
        if let Err(e) = check_chunking(fixture, &mut make) {
            failures.push(format!("{} (chunking): {e}", fixture.name));
        }
    }
    assert!(
        failures.is_empty(),
        "golden mismatches:\n{}",
        failures.join("\n")
    );
}

#[test]
fn fixture_names_parse_their_size() {
    let f = list_fixtures()
        .into_iter()
        .find(|f| f.name == "sgr-attributes")
        .expect("fixture");
    assert_eq!(f.size, domux_term::Size { cols: 80, rows: 24 });
}
