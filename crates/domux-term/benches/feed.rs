use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use domux_term::{Emulator, EmulatorConfig, GhosttyEmulator, Grid, Rgb, Size};

fn make(cols: u16, rows: u16) -> GhosttyEmulator {
    GhosttyEmulator::new(EmulatorConfig {
        size: Size { cols, rows },
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

fn yes_stream(bytes: usize) -> Vec<u8> {
    b"y\r\n".iter().cycle().take(bytes).copied().collect()
}

fn sgr_stream(bytes: usize) -> Vec<u8> {
    let line = b"\x1b[1;31mred bold\x1b[0m \x1b[38;2;10;200;30mtrue\x1b[0m \x1b[4mul\x1b[0m \x1b[7minv\x1b[0m \x1b[48;5;27m  \x1b[0m \xe6\xbc\xa2\xe5\xad\x97 done\r\n";
    line.iter().cycle().take(bytes).copied().collect()
}

fn feed_benches(c: &mut Criterion) {
    let ten_mb = 10 * 1024 * 1024;
    let inputs: Vec<(&str, Vec<u8>)> = vec![
        ("yes", yes_stream(ten_mb)),
        ("sgr", sgr_stream(ten_mb)),
        (
            "nvim-fixture",
            std::fs::read(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/fixtures/golden/nvim-habamax.120x40.bytes"
            ))
            .unwrap_or_default(),
        ),
    ];
    let mut group = c.benchmark_group("feed");
    group.sample_size(10);
    for (input_name, bytes) in &inputs {
        if bytes.is_empty() {
            continue;
        }
        group.throughput(Throughput::Bytes(bytes.len() as u64));
        group.bench_with_input(BenchmarkId::new(*input_name, "80x24"), bytes, |b, bytes| {
            b.iter(|| {
                let mut e = make(80, 24);
                for chunk in bytes.chunks(65536) {
                    e.feed(chunk);
                }
                let mut out = Vec::new();
                e.take_responses(&mut out);
            });
        });
    }
    group.finish();

    let mut snap = c.benchmark_group("snapshot");
    let mut e = make(200, 50);
    e.feed(&sgr_stream(200 * 50 * 4));
    let mut grid = Grid::new(Size {
        cols: 200,
        rows: 50,
    });
    snap.bench_function(BenchmarkId::new("ghostty", "200x50"), |b| {
        b.iter(|| e.snapshot_grid(&mut grid))
    });
    snap.finish();
}

criterion_group!(benches, feed_benches);
criterion_main!(benches);
