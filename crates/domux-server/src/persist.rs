//! The persistence task: debounce 100 ms, write `state.json.tmp`, rename, keep `.bak`.

use domux_core::state_file::{to_json, StateFile};
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::mpsc;

pub const DEBOUNCE: Duration = Duration::from_millis(100);

/// `path` with `suffix` appended to the whole file name, so `state.json` gives
/// `state.json.tmp`. `Path::with_extension` would replace `json` rather than keep it, which
/// is only the same thing while every caller's file happens to be named `<stem>.json`.
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = OsString::from(path.as_os_str());
    name.push(suffix);
    PathBuf::from(name)
}

/// Writes `text` to `path` atomically, keeping the previous file as `<path>.bak`.
pub fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    let tmp = with_suffix(path, ".tmp");
    let bak = with_suffix(path, ".bak");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&tmp, text)?;
    if path.exists() {
        std::fs::rename(path, &bak)?;
    }
    std::fs::rename(&tmp, path)
}

/// Receives snapshots; after 100 ms of quiet, writes the latest one. Every snapshot the core
/// sends is complete, so dropping older ones loses nothing. When the sender closes (the
/// core exited) the pending snapshot is written at once, so `ServerHandle::stop` can await
/// this task and know the file is on disk.
pub async fn spawn(path: PathBuf, mut rx: mpsc::Receiver<StateFile>) {
    let mut pending: Option<StateFile> = None;
    loop {
        let next = match pending {
            Some(_) => tokio::time::timeout(DEBOUNCE, rx.recv()).await,
            None => Ok(rx.recv().await),
        };
        match next {
            Ok(Some(file)) => pending = Some(file),
            Ok(None) => {
                if let Some(f) = pending.take() {
                    write(&path, &f);
                }
                return;
            }
            Err(_) => {
                if let Some(f) = pending.take() {
                    write(&path, &f);
                }
            }
        }
    }
}

fn write(path: &Path, file: &StateFile) {
    if let Err(e) = write_atomic(path, &to_json(file)) {
        tracing::error!("could not write {}: {e}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domux_core::model::Model;
    use domux_core::state_file::{parse, snapshot};
    use std::path::PathBuf;

    fn a_state_file(root: &str) -> StateFile {
        let mut model = Model::new(7);
        model.add_folder_project(PathBuf::from(root)).unwrap();
        snapshot(&model, "2026-09-04T14:32:00+00:00")
    }

    #[test]
    fn write_atomic_keeps_the_previous_file_as_bak_and_leaves_no_tmp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        write_atomic(&path, "first").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first");
        assert!(!dir.path().join("state.json.bak").exists());
        write_atomic(&path, "second").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("state.json.bak")).unwrap(),
            "first"
        );
        assert!(
            !dir.path().join("state.json.tmp").exists(),
            "the temporary file is renamed, not left behind"
        );
    }

    #[test]
    fn write_atomic_creates_the_directory_it_writes_into() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("state.json");
        write_atomic(&path, "x").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "x");
    }

    /// The suffixes are appended to the whole file name, so a file that is not named
    /// `<stem>.json` still gets `<path>.tmp` and `<path>.bak` beside it.
    #[test]
    fn the_temporary_and_backup_names_append_to_the_whole_file_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pr-cache.v2.json");
        assert_eq!(
            with_suffix(&path, ".tmp"),
            dir.path().join("pr-cache.v2.json.tmp")
        );
        assert_eq!(
            with_suffix(&path, ".bak"),
            dir.path().join("pr-cache.v2.json.bak")
        );
    }

    /// The debounce drops every snapshot but the last, and none of them reaches the disk
    /// before the quiet period. Waiting on the file rather than sleeping a fixed time keeps
    /// the assertion honest on a loaded machine.
    #[tokio::test(start_paused = true)]
    async fn the_debounce_writes_only_the_last_snapshot_of_a_burst() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let (tx, rx) = mpsc::channel(64);
        let task = tokio::spawn(spawn(path.clone(), rx));
        for root in ["/a", "/b", "/c"] {
            tx.send(a_state_file(root)).await.unwrap();
        }
        tokio::time::sleep(DEBOUNCE * 3).await;
        let written = parse(&std::fs::read_to_string(&path).expect("state.json")).unwrap();
        assert_eq!(written.projects.len(), 1);
        assert_eq!(written.projects[0].root, PathBuf::from("/c"));
        assert!(
            !path.with_file_name("state.json.bak").exists(),
            "a burst is one write, so there is nothing to back up yet"
        );
        drop(tx);
        task.await.unwrap();
    }

    /// `ServerHandle::stop` awaits this task and then reports the file is on disk, so a
    /// snapshot still pending when the core exits must be written rather than dropped.
    #[tokio::test]
    async fn a_pending_snapshot_is_written_when_the_core_exits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let (tx, rx) = mpsc::channel(64);
        let task = tokio::spawn(spawn(path.clone(), rx));
        tx.send(a_state_file("/only")).await.unwrap();
        drop(tx);
        // No sleep: the task returns once the channel closes, and the file is on disk by then.
        tokio::time::timeout(Duration::from_secs(10), task)
            .await
            .expect("the persistence task did not finish before the deadline")
            .unwrap();
        let written = parse(&std::fs::read_to_string(&path).expect("state.json")).unwrap();
        assert_eq!(written.projects[0].root, PathBuf::from("/only"));
    }
}
