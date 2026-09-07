//! The server log (roadmap decision 1): tracing to `server.log`, rotated by size at 10 MB
//! with one previous file kept as `server.log.1`.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const MAX_BYTES: u64 = 10 * 1024 * 1024;

struct Rotating {
    path: PathBuf,
    file: File,
    written: u64,
}

impl Rotating {
    fn open(path: &Path) -> io::Result<Rotating> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        let written = file.metadata()?.len();
        Ok(Rotating {
            path: path.to_path_buf(),
            file,
            written,
        })
    }
}

impl Write for Rotating {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.written + buf.len() as u64 > MAX_BYTES {
            let previous = self.path.with_extension("log.1");
            let _ = std::fs::rename(&self.path, previous);
            self.file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)?;
            self.written = 0;
        }
        let n = self.file.write(buf)?;
        self.written += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

/// Installs the global tracing subscriber writing to `path`. `DOMUX_LOG` sets the filter
/// (`info` by default).
pub fn init(path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let writer = Mutex::new(Rotating::open(path)?);
    let filter = tracing_subscriber::EnvFilter::try_from_env("DOMUX_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(writer)
        .init();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The file is grown with `set_len` rather than written: `Rotating::open` takes the
    /// starting size from the metadata, so a sparse 10 MB file exercises the threshold
    /// without putting 10 MB through the writer.
    #[test]
    fn the_log_rotates_at_the_size_limit_and_keeps_one_previous_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.log");
        File::create(&path).unwrap().set_len(MAX_BYTES).unwrap();
        let mut rotating = Rotating::open(&path).unwrap();
        assert_eq!(rotating.written, MAX_BYTES);
        rotating.write_all(b"after the limit\n").unwrap();
        rotating.flush().unwrap();
        let previous = dir.path().join("server.log.1");
        assert_eq!(
            std::fs::metadata(&previous).unwrap().len(),
            MAX_BYTES,
            "the full file is kept as server.log.1"
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "after the limit\n",
            "the new server.log holds only what was written after the rotation"
        );
    }

    #[test]
    fn a_write_under_the_limit_appends_rather_than_rotating() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.log");
        let mut rotating = Rotating::open(&path).unwrap();
        rotating.write_all(b"one\n").unwrap();
        rotating.write_all(b"two\n").unwrap();
        rotating.flush().unwrap();
        assert!(!dir.path().join("server.log.1").exists());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "one\ntwo\n");
    }
}
