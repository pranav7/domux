//! Throughput budget kept as a regression test. Runs only with DOMUX_PROFILE=1 in release
//! mode, mirroring V1's TestSwitcherStartupBudget.
//!
//! DOMUX_PROFILE=1 cargo test --release -p domux-term --test budget -- --nocapture

use domux_term::{Emulator, EmulatorConfig, GhosttyEmulator, Grid, Rgb, Size};
use std::time::Instant;

fn make() -> GhosttyEmulator {
    GhosttyEmulator::new(EmulatorConfig {
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
    })
    .unwrap()
}

/// 10 MB of `yes` output must feed in under this many milliseconds, and a 200x50 snapshot
/// must take under this many microseconds. Set to the measured worst case times 1.5 on an
/// Apple M3 Pro (feed 135 ms, snapshot 285 us; see docs/decisions/0001-terminal-emulator.md).
/// These are regression fences, not targets.
const FEED_BUDGET_MS: u128 = 250;
const SNAPSHOT_BUDGET_US: u128 = 500;

fn check() {
    let bytes: Vec<u8> = b"y\r\n"
        .iter()
        .cycle()
        .take(10 * 1024 * 1024)
        .copied()
        .collect();
    let mut e = make();
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
        "ghostty: feed 10 MB in {} ms ({:.0} MB/s), snapshot 200x50 in {} us",
        feed.as_millis(),
        10.0 / feed.as_secs_f64(),
        snap.as_micros()
    );
    assert!(feed.as_millis() < FEED_BUDGET_MS, "feed over budget");
    assert!(
        snap.as_micros() < SNAPSHOT_BUDGET_US,
        "snapshot over budget"
    );
}

#[test]
fn the_emulator_stays_within_the_throughput_budget() {
    if std::env::var_os("DOMUX_PROFILE").is_none() {
        eprintln!("skipped: set DOMUX_PROFILE=1 and use --release");
        return;
    }
    check();
}
