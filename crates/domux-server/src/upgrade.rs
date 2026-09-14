//! What `server upgrade` hands the new server, and the exec that hands it over (decision 0045).
//!
//! The handover is a directory under the state directory: `handoff.json`, and one screen file
//! per pane that had a screen to carry. The descriptors it names are open in this process and
//! stay open across the exec, which is the whole trick; the file only says which is which.

use crate::pane::HandedPty;
use anyhow::{Context, Result};
use domux_core::ids::PaneId;
use domux_core::names::HANDOFF_FILE_NAME;
use domux_term::Size;
use serde::{Deserialize, Serialize};
use std::io;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::os::unix::io::RawFd;
use std::path::{Path, PathBuf};

/// The handover format this build writes and reads. `server.upgrade` names the one the new
/// binary reads, and a server that writes another refuses before it has changed anything, so
/// a change to `Handoff` that an older build cannot read bumps this.
pub const HANDOFF_FORMAT: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Handoff {
    pub format: u32,
    /// The version of the server that wrote it, for the new server's log.
    pub from_version: String,
    /// When the first server of this process started. An upgrade does not restart the
    /// process, so `server status` goes on saying when it did.
    pub started_at: String,
    /// The listening socket.
    pub listener: RawFd,
    pub panes: Vec<HandedPane>,
    /// The model's agent records. Kept as JSON rather than typed here, so records the new
    /// build cannot read cost the records and not the handover.
    pub agents: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HandedPane {
    pub pane: PaneId,
    pub fd: RawFd,
    pub pid: Option<u32>,
    /// The pane's size, for the empty screen a pane gets when its own cannot be restored.
    pub size: Size,
    /// The screen file's name in the handover directory, absent when the screen could not be
    /// encoded.
    pub screen: Option<String>,
}

impl HandedPane {
    pub fn pty(&self) -> HandedPty {
        HandedPty {
            fd: self.fd,
            pid: self.pid,
        }
    }
}

/// The screen file of `pane`.
pub fn screen_file_name(pane: &PaneId) -> String {
    format!("{pane}.screen")
}

/// Writes the handover into `dir`, replacing whatever an earlier one left there, and answers
/// the path of `handoff.json`. `screens` are the encoded screens, named as `handoff.panes`
/// names them. The directory is private and so is every file: a screen holds whatever the pane
/// showed.
pub fn write(dir: &Path, handoff: &Handoff, screens: &[(String, Vec<u8>)]) -> Result<PathBuf> {
    match std::fs::remove_dir_all(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).with_context(|| format!("clear {}", dir.display())),
    }
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .with_context(|| format!("create {}", dir.display()))?;
    for (name, bytes) in screens {
        private_write(&dir.join(name), bytes)?;
    }
    let path = dir.join(HANDOFF_FILE_NAME);
    let text = serde_json::to_vec_pretty(handoff).context("encode the handover")?;
    private_write(&path, &text)?;
    Ok(path)
}

fn private_write(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("create {}", path.display()))?;
    file.write_all(bytes)
        .with_context(|| format!("write {}", path.display()))
}

/// Reads the handover at `path`. A format this build does not read is refused by number: the
/// descriptors it names cannot be interpreted any other way.
pub fn read(path: &Path) -> Result<Handoff> {
    let text = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let value: serde_json::Value =
        serde_json::from_slice(&text).with_context(|| format!("parse {}", path.display()))?;
    let format = value.get("format").and_then(|f| f.as_u64());
    if format != Some(u64::from(HANDOFF_FORMAT)) {
        anyhow::bail!(
            "{} is handover format {}, and this build reads format {HANDOFF_FORMAT}",
            path.display(),
            format.map_or_else(|| "unknown".to_string(), |f| f.to_string())
        );
    }
    serde_json::from_value(value).with_context(|| format!("read {}", path.display()))
}

/// A pane's screen from the handover in `dir`, or `None` when there is none to read.
pub fn read_screen(dir: &Path, pane: &HandedPane) -> Option<Vec<u8>> {
    let name = pane.screen.as_ref()?;
    match std::fs::read(dir.join(name)) {
        Ok(bytes) => Some(bytes),
        Err(e) => {
            tracing::warn!(pane = %pane.pane, "the screen file {name} could not be read: {e}");
            None
        }
    }
}

/// Replaces this process with `binary server run --handoff <handoff>`. A real exec returns
/// only when it failed, so `Ok` means this process is no longer the server: the caller ends
/// without stopping anything, because what it would stop now belongs to the new one.
pub trait Exec: Send + Sync {
    fn exec(&self, binary: &Path, handoff: &Path) -> io::Result<()>;
}

pub struct RealExec;

impl Exec for RealExec {
    fn exec(&self, binary: &Path, handoff: &Path) -> io::Result<()> {
        use std::os::unix::process::CommandExt;
        // The environment, the working directory and the three standard descriptors carry over
        // as they are, which is what a server started by `server start` had: stderr is the log.
        Err(std::process::Command::new(binary)
            .args(["server", "run", "--handoff"])
            .arg(handoff)
            .exec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_handoff() -> Handoff {
        Handoff {
            format: HANDOFF_FORMAT,
            from_version: "1.0.1".into(),
            started_at: "2026-09-14T11:54:57+01:00".into(),
            listener: 9,
            panes: vec![
                HandedPane {
                    pane: PaneId("p_0001".into()),
                    fd: 12,
                    pid: Some(4242),
                    size: Size { cols: 80, rows: 24 },
                    screen: Some(screen_file_name(&PaneId("p_0001".into()))),
                },
                HandedPane {
                    pane: PaneId("p_0002".into()),
                    fd: 13,
                    pid: Some(4243),
                    size: Size { cols: 80, rows: 24 },
                    screen: None,
                },
            ],
            agents: serde_json::json!([]),
        }
    }

    #[test]
    fn a_written_handoff_reads_back_with_its_screens() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().join("handoff");
        let handoff = a_handoff();
        let path = write(
            &dir,
            &handoff,
            &[("p_0001.screen".into(), b"screen".to_vec())],
        )
        .unwrap();
        let back = read(&path).unwrap();
        assert_eq!(back, handoff);
        assert_eq!(
            read_screen(&dir, &back.panes[0]).as_deref(),
            Some(&b"screen"[..])
        );
        assert_eq!(
            read_screen(&dir, &back.panes[1]),
            None,
            "a pane with no screen file"
        );
    }

    #[test]
    fn a_handoff_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().join("handoff");
        let path = write(
            &dir,
            &a_handoff(),
            &[("p_0001.screen".into(), b"x".to_vec())],
        )
        .unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(&dir.join("p_0001.screen")), 0o600);
    }

    #[test]
    fn a_second_handoff_replaces_what_the_first_left() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().join("handoff");
        write(
            &dir,
            &a_handoff(),
            &[("p_0001.screen".into(), b"old".to_vec())],
        )
        .unwrap();
        write(&dir, &a_handoff(), &[]).unwrap();
        assert!(!dir.join("p_0001.screen").exists());
    }

    #[test]
    fn a_handoff_in_another_format_is_refused_by_number() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(HANDOFF_FILE_NAME);
        std::fs::write(&path, r#"{"format": 2, "listener": 3}"#).unwrap();
        let said = format!("{:#}", read(&path).unwrap_err());
        assert!(said.contains("handover format 2"), "{said}");
        assert!(
            said.contains(&format!("reads format {HANDOFF_FORMAT}")),
            "{said}"
        );
    }
}
