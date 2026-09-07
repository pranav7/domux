//! The server log (roadmap decision 1): tracing to `server.log`, rotated by size at 10 MB
//! with one previous file kept as `server.log.1`.

use std::ffi::OsString;
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
            let previous = with_suffix(&self.path, ".1");
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

/// The whole file name plus `suffix`. `Path::with_extension` would replace the extension
/// rather than keep it, so a log configured as `activity.txt` would rotate to `activity.1`
/// instead of `activity.txt.1`. The same reasoning as `persist::write_atomic`.
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = OsString::from(path.as_os_str());
    name.push(suffix);
    PathBuf::from(name)
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
        .try_init()
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
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

    /// The rotation keeps the whole configured name: a log at `activity.txt` becomes
    /// `activity.txt.1`, not `activity.1`.
    #[test]
    fn rotation_appends_to_the_whole_configured_file_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("activity.txt");
        File::create(&path).unwrap().set_len(MAX_BYTES).unwrap();
        let mut rotating = Rotating::open(&path).unwrap();
        rotating.write_all(b"new\n").unwrap();
        assert!(dir.path().join("activity.txt.1").exists());
        assert!(!dir.path().join("activity.1").exists());
    }

    /// `init` installs the one global subscriber a process may have, so a second call is a
    /// reported error rather than a panic that takes the server down.
    #[test]
    fn init_creates_the_log_and_reports_a_subscriber_conflict() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.log");
        init(&path).unwrap();
        assert!(path.exists());
        let error = init(&dir.path().join("second.log")).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("global default trace dispatcher"),
            "the conflict is returned: {error}"
        );
    }
}
