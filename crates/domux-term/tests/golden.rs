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
