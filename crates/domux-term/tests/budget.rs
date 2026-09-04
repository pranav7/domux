//! Throughput budget kept as a regression test. Runs only with DOMUX_PROFILE=1 in release
//! mode, mirroring V1's TestSwitcherStartupBudget.
//!
//! DOMUX_PROFILE=1 cargo test --release -p domux-term --all-features --test budget -- --nocapture

#![cfg_attr(
    not(any(feature = "ghostty", feature = "alacritty")),
    allow(unused_imports, dead_code)
)]

use domux_term::{new_emulator, Emulator, EmulatorConfig, EmulatorKind, Grid, Rgb, Size};
use std::time::Instant;

fn make(kind: EmulatorKind) -> Box<dyn Emulator> {
    new_emulator(
        kind,
        EmulatorConfig {
            size: Size {
                cols: 200,
                rows: 50,
            },
            scrollback_lines: 10000,
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
    .unwrap()
}

/// 10 MB of `yes` output must feed in under this many milliseconds, and a 200x50 snapshot
/// must take under this many microseconds. Set from the numbers Task 13 records.
const FEED_BUDGET_MS: u128 = 500;
const SNAPSHOT_BUDGET_US: u128 = 2000;

fn check(kind: EmulatorKind) {
    let bytes: Vec<u8> = b"y\r\n"
        .iter()
        .cycle()
        .take(10 * 1024 * 1024)
        .copied()
        .collect();
    let mut e = make(kind);
    let t = Instant::now();
    for chunk in bytes.chunks(65536) {
        e.feed(chunk);
    }
    let feed = t.elapsed();
    let mut grid = Grid::new(e.size());
    let t = Instant::now();
    e.snapshot_grid(&mut grid);
    let snap = t.elapsed();
    println!(
        "{kind:?}: feed 10 MB in {} ms ({:.0} MB/s), snapshot 200x50 in {} us",
        feed.as_millis(),
        10.0 / feed.as_secs_f64(),
        snap.as_micros()
    );
    assert!(
        feed.as_millis() < FEED_BUDGET_MS,
        "{kind:?} feed over budget"
    );
    assert!(
        snap.as_micros() < SNAPSHOT_BUDGET_US,
        "{kind:?} snapshot over budget"
    );
}

#[test]
fn emulators_stay_within_the_throughput_budget() {
    if std::env::var_os("DOMUX_PROFILE").is_none() {
        eprintln!("skipped: set DOMUX_PROFILE=1 and use --release");
        return;
    }
    #[cfg(feature = "ghostty")]
    check(EmulatorKind::Ghostty);
    #[cfg(feature = "alacritty")]
    check(EmulatorKind::Alacritty);
}
