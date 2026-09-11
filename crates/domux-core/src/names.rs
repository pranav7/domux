//! The names the M3 cut-over changed. Until 2026-09-11 the binary, the state directory, the
//! config directory and the socket were all `domux2`, so V2 could run beside V1. They are
//! `domux` now. `OLD_NAME` is what the server moves the state and config away from, once, on a
//! machine that ran the old name.
//!
//! No source outside this file spells the old name, and a test below keeps it that way: a
//! message that named the old command would name one that no longer exists.

/// The binary, and the prefix of every path.
pub const BIN_NAME: &str = "domux";
/// The product name shown to people. The same word as `BIN_NAME` since the cut-over.
pub const PRODUCT_NAME: &str = "domux";
pub const STATE_DIR_NAME: &str = "domux";
pub const CONFIG_DIR_NAME: &str = "domux";
pub const SOCKET_FILE_NAME: &str = "domux.sock";
/// The name of the socket directory under `/tmp` when `XDG_RUNTIME_DIR` is unset: `domux-<uid>`.
pub const SOCKET_DIR_PREFIX: &str = "domux-";

/// What the binary, the state directory and the config directory were called before the
/// cut-over. `paths::old_state_dir_in` and `paths::old_config_file_in` derive from it, and the
/// server moves the files it finds there once. Nothing else spells it.
pub const OLD_NAME: &str = "domux2";

/// V1's state directory name, for `import v1` to read V1's session files from. It is the same
/// word as `STATE_DIR_NAME` since the cut-over, and is spelled apart on purpose: this one names
/// V1's directory, and `sessions/` under it is V1's and is never written.
pub const V1_STATE_DIR_NAME: &str = "domux";
/// Where V1 keeps one JSON file per session, under its state directory.
pub const V1_SESSIONS_DIR_NAME: &str = "sessions";

#[cfg(test)]
mod tests {
    use super::OLD_NAME;
    use std::path::{Path, PathBuf};

    fn workspace_root() -> PathBuf {
        // `crates/domux-core` -> the workspace.
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("the crate sits two levels below the workspace root")
            .to_path_buf()
    }

    fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("read a source directory") {
            let path = entry.expect("read a directory entry").path();
            if path.is_dir() {
                rust_sources(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

    /// Before the cut-over this test kept every source but this file from spelling the
    /// binary name, so the rename would fail each message once and someone would read it.
    /// It did that job on 2026-09-11: 199 lines in 22 files were read. It now guards the
    /// other direction, that the old name does not come back through a rebase or a habit.
    /// Tests are included this time, because a test that spells the old name pins what a
    /// user no longer sees.
    #[test]
    fn nothing_outside_this_file_spells_the_old_name() {
        let root = workspace_root();
        let mut sources = Vec::new();
        for crate_dir in std::fs::read_dir(root.join("crates")).expect("read crates/") {
            let dir = crate_dir.expect("read a crate directory").path();
            if dir.is_dir() {
                rust_sources(&dir, &mut sources);
            }
        }
        assert!(sources.len() > 20, "found only {} sources", sources.len());

        let mut offenders = Vec::new();
        for path in sources {
            if path.ends_with("domux-core/src/names.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("read a source file");
            for (i, line) in text.lines().enumerate() {
                if line.contains(OLD_NAME) {
                    let rel = path.strip_prefix(&root).unwrap_or(&path);
                    offenders.push(format!("{}:{}: {}", rel.display(), i + 1, line.trim()));
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "{} place(s) spell the old name {OLD_NAME:?}, which nothing answers to since the cut-over:\n{}",
            offenders.len(),
            offenders.join("\n")
        );
    }
}
