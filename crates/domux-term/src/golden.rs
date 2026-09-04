//! Golden fixtures: recorded bytes in, expected grid text out, for every implementation.

use crate::emulator::Emulator;
use crate::types::{Grid, Size};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Fixture {
    pub name: String,
    pub size: Size,
    pub bytes_path: PathBuf,
    pub golden_path: PathBuf,
}

pub fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/golden")
}

/// Finds `<name>.<cols>x<rows>.bytes` files. The size lives in the file name so a fixture
/// is self-describing without a manifest.
pub fn list_fixtures() -> Vec<Fixture> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(fixtures_dir()) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("bytes") {
            continue;
        }
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let Some((name, size)) = stem.rsplit_once('.') else {
            continue;
        };
        let Some((cols, rows)) = size.split_once('x') else {
            continue;
        };
        let (Ok(cols), Ok(rows)) = (cols.parse::<u16>(), rows.parse::<u16>()) else {
            continue;
        };
        out.push(Fixture {
            name: name.to_string(),
            size: Size { cols, rows },
            bytes_path: path.clone(),
            golden_path: path.with_extension("golden"),
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Feeds `bytes` in chunks of `chunk` bytes, writing any responses to `responses`.
pub fn feed_chunked(
    emulator: &mut dyn Emulator,
    bytes: &[u8],
    chunk: usize,
    responses: &mut Vec<u8>,
) {
    for piece in bytes.chunks(chunk.max(1)) {
        emulator.feed(piece);
        emulator.take_responses(responses);
    }
}

pub fn render(emulator: &mut dyn Emulator) -> String {
    let mut grid = Grid::new(emulator.size());
    emulator.snapshot_grid(&mut grid);
    grid.to_text(&emulator.cursor())
}

/// Compares the emulator's grid after the fixture against the golden file. With
/// `UPDATE_GOLDEN=1` it writes the golden instead (atomically) and passes.
pub fn check(fixture: &Fixture, emulator: &mut dyn Emulator) -> Result<(), String> {
    let bytes = fs::read(&fixture.bytes_path).map_err(|e| format!("read fixture: {e}"))?;
    let mut responses = Vec::new();
    feed_chunked(emulator, &bytes, bytes.len(), &mut responses);
    let actual = render(emulator);

    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        let tmp = fixture.golden_path.with_extension("golden.tmp");
        fs::write(&tmp, &actual).map_err(|e| format!("write golden: {e}"))?;
        fs::rename(&tmp, &fixture.golden_path).map_err(|e| format!("rename golden: {e}"))?;
        return Ok(());
    }
    let expected = fs::read_to_string(&fixture.golden_path).map_err(|_| {
        format!(
            "missing golden {}; run with UPDATE_GOLDEN=1 and review it",
            fixture.golden_path.display()
        )
    })?;
    if actual != expected {
        return Err(format!(
            "grid differs from golden\n--- expected\n{expected}\n--- actual\n{actual}"
        ));
    }
    Ok(())
}

/// Every chunking must give the same grid. Call after `check` with a fresh emulator per size.
pub fn check_chunking(
    fixture: &Fixture,
    make: &mut dyn FnMut() -> Box<dyn Emulator>,
) -> Result<(), String> {
    let bytes = fs::read(&fixture.bytes_path).map_err(|e| format!("read fixture: {e}"))?;
    let mut reference = None;
    for chunk in [bytes.len().max(1), 1, 7] {
        let mut emulator = make();
        let mut responses = Vec::new();
        feed_chunked(emulator.as_mut(), &bytes, chunk, &mut responses);
        let text = render(emulator.as_mut());
        match &reference {
            None => reference = Some(text),
            Some(r) if *r != text => {
                return Err(format!(
                    "chunk size {chunk} gives a different grid\n--- whole\n{r}\n--- chunked\n{text}"
                ))
            }
            _ => {}
        }
    }
    Ok(())
}
