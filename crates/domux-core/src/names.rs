//! The names that change at the M3 cut-over. Edit this file and the `[[bin]]` name in
//! `crates/domux/Cargo.toml`; no other source spells `domux2`. Tests do, on purpose: they
//! pin what a user reads today, so the rename fails them and someone reads each message
//! once.

/// The binary and the prefix of every path until the cut-over.
pub const BIN_NAME: &str = "domux2";
/// The product name shown to people. Unchanged by the cut-over.
pub const PRODUCT_NAME: &str = "domux";
pub const STATE_DIR_NAME: &str = "domux2";
pub const CONFIG_DIR_NAME: &str = "domux2";
pub const SOCKET_FILE_NAME: &str = "domux2.sock";
/// The name of the socket directory under `/tmp` when `XDG_RUNTIME_DIR` is unset: `domux2-<uid>`.
pub const SOCKET_DIR_PREFIX: &str = "domux2-";

#[cfg(test)]
mod tests {
    use super::BIN_NAME;
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

    /// The file's lines with its inline test modules removed. Every `#[cfg(test)]` in this
    /// workspace is a top level attribute on a module whose closing brace is the next `}` at
    /// column 0, which is what this relies on and what `cargo fmt` keeps true.
    fn lines_outside_test_modules(text: &str) -> Vec<(usize, &str)> {
        let mut out = Vec::new();
        let mut in_test = false;
        for (i, line) in text.lines().enumerate() {
            if line == "#[cfg(test)]" {
                in_test = true;
                continue;
            }
            if in_test {
                if line == "}" {
                    in_test = false;
                }
                continue;
            }
            out.push((i + 1, line));
        }
        out
    }

    /// This file promises that nothing else spells the binary name. That promise was false for
    /// the whole of M1 - `BIN_NAME` had no uses at all and 34 messages spelled `domux2` - so at
    /// the cut-over every one of them would have gone on naming a command that no longer
    /// existed. Nothing caught it, because a promise in a doc comment is not a test.
    ///
    /// Test code is exempt on purpose: a test pins what a user sees today, and one written
    /// against the constant would pass through the rename having checked nothing.
    #[test]
    fn nothing_outside_this_file_spells_the_binary_name() {
        let root = workspace_root();
        let mut sources = Vec::new();
        for crate_dir in std::fs::read_dir(root.join("crates")).expect("read crates/") {
            let src = crate_dir
                .expect("read a crate directory")
                .path()
                .join("src");
            if src.is_dir() {
                rust_sources(&src, &mut sources);
            }
        }
        assert!(sources.len() > 20, "found only {} sources", sources.len());

        let mut offenders = Vec::new();
        for path in sources {
            if path.ends_with("domux-core/src/names.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("read a source file");
            for (n, line) in lines_outside_test_modules(&text) {
                if line.contains(BIN_NAME) {
                    let rel = path.strip_prefix(&root).unwrap_or(&path);
                    offenders.push(format!("{}:{n}: {}", rel.display(), line.trim()));
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "{} place(s) spell the binary name instead of using names::BIN_NAME:\n{}",
            offenders.len(),
            offenders.join("\n")
        );
    }
}
