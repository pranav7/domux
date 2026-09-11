//! The cut-over renamed the state and config directories, so a machine that ran the old name
//! has its files where `paths` no longer looks. The server moves them once at start, and
//! nothing else in the old directory is touched. M3 plan, Task 23, step 4.

use std::path::{Path, PathBuf};

/// The files the server keeps in its state directory and carries across the rename. The log
/// stays: a new one starts in the new directory and the old one is still there to read. The
/// stay awake process id file comes along so a hold the old server left is adopted, not
/// leaked.
pub const STATE_FILES: [&str; 4] = [
    "state.json",
    "state.json.bak",
    "pr-cache.json",
    "stay-awake.pid",
];

/// Moves each of `STATE_FILES` that exists in `old` and not yet in `new`. A file already in
/// `new` wins, so a server that has run under the new name is never overwritten by what the
/// old one left. `old` itself and everything else in it stay. Answers the paths it moved to.
pub fn move_state_files(old: &Path, new: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut moved = Vec::new();
    for name in STATE_FILES {
        let from = old.join(name);
        let to = new.join(name);
        if from.is_file() && !to.exists() {
            std::fs::create_dir_all(new)?;
            std::fs::rename(&from, &to)?;
            moved.push(to);
        }
    }
    Ok(moved)
}

/// Moves the config file on the same terms: only when `old` exists and `new` does not. Answers
/// whether it moved.
pub fn move_config_file(old: &Path, new: &Path) -> std::io::Result<bool> {
    if !old.is_file() || new.exists() {
        return Ok(false);
    }
    if let Some(dir) = new.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::rename(old, new)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn the_state_files_move_once_and_the_rest_of_the_old_directory_stays() {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("domux-old");
        let new = tmp.path().join("domux-new");
        write(&old.join("state.json"), "{}");
        write(&old.join("state.json.bak"), "{}");
        write(&old.join("server.log"), "log");
        write(&new.join("sessions").join("one.json"), "v1");

        let moved = move_state_files(&old, &new).unwrap();

        assert_eq!(
            moved,
            vec![new.join("state.json"), new.join("state.json.bak")]
        );
        assert!(new.join("state.json").is_file());
        assert!(!old.join("state.json").exists());
        assert!(old.join("server.log").is_file(), "the log is not carried");
        assert!(
            new.join("sessions").join("one.json").is_file(),
            "V1's sessions are untouched"
        );
        assert!(old.is_dir(), "the old directory is left for the reader");
        assert!(
            move_state_files(&old, &new).unwrap().is_empty(),
            "a second start moves nothing"
        );
    }

    #[test]
    fn a_file_already_under_the_new_name_is_never_overwritten() {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("old");
        let new = tmp.path().join("new");
        write(&old.join("state.json"), "old");
        write(&new.join("state.json"), "new");

        let moved = move_state_files(&old, &new).unwrap();

        assert!(moved.is_empty());
        assert_eq!(
            std::fs::read_to_string(new.join("state.json")).unwrap(),
            "new"
        );
        assert_eq!(
            std::fs::read_to_string(old.join("state.json")).unwrap(),
            "old"
        );
    }

    #[test]
    fn a_missing_old_directory_moves_nothing_and_creates_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("never-there");
        let new = tmp.path().join("new");
        assert!(move_state_files(&old, &new).unwrap().is_empty());
        assert!(!new.exists(), "no move, no directory");
    }

    #[test]
    fn the_config_file_moves_once_and_yields_to_one_already_there() {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("old").join("domux.toml");
        let new = tmp.path().join("new").join("domux.toml");
        write(&old, "[keys]");

        assert!(move_config_file(&old, &new).unwrap());
        assert_eq!(std::fs::read_to_string(&new).unwrap(), "[keys]");
        assert!(!old.exists());
        assert!(
            !move_config_file(&old, &new).unwrap(),
            "nothing left to move"
        );

        write(&old, "older");
        assert!(!move_config_file(&old, &new).unwrap(), "the new file wins");
        assert_eq!(std::fs::read_to_string(&new).unwrap(), "[keys]");
    }
}
