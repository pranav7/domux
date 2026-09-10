//! What link, if any, is under one cell of a pane (decision record 0020).
//!
//! Two readings, in order. A cell the program marked with OSC 8 carries its target, and that
//! target is the answer whatever the text says. Otherwise the run of text around the cell is
//! read, and it is a link only if it is an http or https URL or a path that exists.
//!
//! Nothing here opens anything. It answers what was pointed at, and refuses everything it
//! does not recognise, so the one place that decides what may be handed to the desktop is
//! this one.

use domux_term::{Emulator, ScrollbackPos};
use std::path::{Path, PathBuf};

/// What a cell points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    /// An http or https address, from OSC 8 or from the text.
    Url(String),
    /// A file or directory that exists, absolute by the time it gets here.
    Path(PathBuf),
}

impl Link {
    /// What the opener is handed. A path goes as its own string rather than as a `file://`
    /// URL: the opener takes a path, and building a URL would mean escaping a name that the
    /// filesystem already spells exactly.
    pub fn target(&self) -> String {
        match self {
            Link::Url(url) => url.clone(),
            Link::Path(path) => path.display().to_string(),
        }
    }

    /// The short form for the row that says what was opened. A URL says itself; a path says
    /// its last component, because the row is narrow and the directories above it are the
    /// part the reader already knows.
    pub fn said(&self) -> String {
        match self {
            Link::Url(url) => url.clone(),
            Link::Path(path) => path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| path.display().to_string()),
        }
    }
}

/// Characters a URL in prose collects at its end and does not own. A closing bracket is
/// included: a URL that really ends in one is rarer than a URL written inside a pair.
const TRAILING: &[char] = &[
    '.', ',', ';', ':', '!', '?', ')', ']', '}', '>', '"', '\'', '`',
];

/// The same at the front, which is where a line of prose puts the other half of the pair.
const LEADING: &[char] = &['(', '[', '{', '<', '"', '\'', '`'];

/// The link under the cell at `row`, `col` of the pane's visible grid, or `None`.
///
/// `offset` is how far copy mode is holding the viewport above the live screen, so the row the
/// reader clicked is the row this reads. `cwd` is the pane's working directory, which is what
/// a relative path is resolved against.
pub fn at(
    emulator: &mut dyn Emulator,
    offset: usize,
    cwd: Option<&Path>,
    row: u16,
    col: u16,
) -> Option<Link> {
    let size = emulator.size();
    if size.cols == 0 || size.rows == 0 {
        return None;
    }
    let top = emulator.scrollback_len().saturating_sub(offset);
    let abs_row = top + row as usize;
    let pos = ScrollbackPos { row: abs_row, col };
    if let Some(uri) = emulator.hyperlink_at(pos) {
        return from_marked(&uri, cwd);
    }
    let (first, last) = emulator.logical_line(abs_row);
    let text = emulator.text_in_range(
        ScrollbackPos { row: first, col: 0 },
        ScrollbackPos {
            row: last,
            col: size.cols.saturating_sub(1),
        },
    )?;
    // The rows of a soft-wrapped line are full width by definition, so the cell the reader
    // clicked is this far along the line the rows were rejoined into.
    let index = abs_row.saturating_sub(first) * size.cols as usize + col as usize;
    from_text(&text, index, cwd)
}

/// A target the program marked with OSC 8. Only http, https and file are accepted: the
/// sequence carries whatever the program wrote, and a scheme nobody here understands would be
/// handed to the desktop to interpret, which is a program on this machine chosen by text that
/// arrived over a pipe.
fn from_marked(uri: &str, cwd: Option<&Path>) -> Option<Link> {
    let uri = uri.trim();
    if uri.starts_with("http://") || uri.starts_with("https://") {
        return Some(Link::Url(uri.to_string()));
    }
    // `file://host/path`, whose host is a machine name this cannot reach, and `file:///path`,
    // whose host is empty and means this one. Only the second is opened, and it is opened as
    // the path it names.
    let rest = uri.strip_prefix("file://")?;
    let path = rest.strip_prefix('/').map(|p| format!("/{p}"))?;
    existing_path(&path, cwd)
}

/// The link in `text` at character `index`, if the run around it is one.
fn from_text(text: &str, index: usize, cwd: Option<&Path>) -> Option<Link> {
    let run = run_at(text, index)?;
    if run.starts_with("http://") || run.starts_with("https://") {
        // A bare scheme with nothing after it is not an address.
        let rest = run
            .trim_start_matches("http://")
            .trim_start_matches("https://");
        return (!rest.is_empty()).then(|| Link::Url(run.to_string()));
    }
    existing_path(run, cwd)
}

/// The run of non-space characters covering character `index`, with the punctuation a line of
/// prose wraps it in trimmed off both ends. `None` when the character there is a space or the
/// index is past the line.
fn run_at(text: &str, index: usize) -> Option<&str> {
    // Characters and not bytes, because `index` was counted in cells. A wide grapheme takes
    // two cells and one character, so a line holding one shifts the count and a click to the
    // right of it finds the neighbouring run rather than nothing, which is the better of the
    // two failures available here.
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let (_, under) = *chars.get(index)?;
    if under.is_whitespace() {
        return None;
    }
    let mut first = index;
    while first > 0 && !chars[first - 1].1.is_whitespace() {
        first -= 1;
    }
    let mut last = index;
    while last + 1 < chars.len() && !chars[last + 1].1.is_whitespace() {
        last += 1;
    }
    let start = chars[first].0;
    let end = chars[last].0 + chars[last].1.len_utf8();
    let run = text[start..end]
        .trim_start_matches(LEADING)
        .trim_end_matches(TRAILING);
    (!run.is_empty()).then_some(run)
}

/// `run` as a path that exists, or `None`. A `~` is the reader's home, a relative path is the
/// pane's working directory, and a `file.rs:12:3` suffix is dropped once when the path with it
/// does not exist.
fn existing_path(run: &str, cwd: Option<&Path>) -> Option<Link> {
    if let Some(found) = resolve(run, cwd) {
        return Some(Link::Path(found));
    }
    // Editors and compilers write a line and a column after a path. The suffix is dropped
    // only after the whole run has failed, so a file whose name really ends in `:12` still
    // opens as itself.
    let trimmed = run.trim_end_matches(|c: char| c.is_ascii_digit() || c == ':');
    if trimmed == run || trimmed.is_empty() {
        return None;
    }
    resolve(trimmed, cwd).map(Link::Path)
}

/// One candidate, resolved and checked against the disk.
fn resolve(run: &str, cwd: Option<&Path>) -> Option<PathBuf> {
    let path = if let Some(rest) = run.strip_prefix("~/") {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join(rest))?
    } else if run == "~" {
        PathBuf::from(std::env::var_os("HOME")?)
    } else {
        let raw = PathBuf::from(run);
        if raw.is_absolute() {
            raw
        } else {
            // A relative run means nothing without a directory to read it against, and a
            // pane that never reported one leaves it unknown rather than the server's own
            // (principle 4).
            cwd?.join(raw)
        }
    };
    // `exists` and not `canonicalize`: a symlink is opened as the name the reader saw, which
    // is what every other program on the desktop does with one.
    path.exists().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_url_in_a_line_is_found_from_any_of_its_cells() {
        let line = "Published https://claude.ai/code/artifact/8fc now";
        for i in [10, 20, 42] {
            assert_eq!(
                from_text(line, i, None),
                Some(Link::Url("https://claude.ai/code/artifact/8fc".into())),
                "at {i}"
            );
        }
        assert_eq!(from_text(line, 0, None), None, "the word before it");
        assert_eq!(from_text(line, 9, None), None, "the space before it");
    }

    #[test]
    fn a_url_loses_the_punctuation_a_sentence_left_on_it() {
        assert_eq!(
            from_text("see https://example.com/x.", 10, None),
            Some(Link::Url("https://example.com/x".into()))
        );
        assert_eq!(
            from_text("(https://example.com/x)", 5, None),
            Some(Link::Url("https://example.com/x".into())),
            "the opening bracket is part of no run because it is not trimmed at the front, \
             so this is the one that has to come off the end"
        );
    }

    #[test]
    fn a_bare_scheme_is_not_an_address() {
        assert_eq!(from_text("https://", 2, None), None);
        assert_eq!(from_text("nothing here", 3, None), None);
    }

    #[test]
    fn a_marked_cell_answers_its_own_target_and_refuses_a_scheme_nobody_asked_for() {
        assert_eq!(
            from_marked("https://example.com/x", None),
            Some(Link::Url("https://example.com/x".into()))
        );
        assert_eq!(from_marked("mailto:someone@example.com", None), None);
        assert_eq!(from_marked("javascript:alert(1)", None), None);
        assert_eq!(from_marked("/etc/passwd", None), None, "not a URL at all");
    }

    #[test]
    fn a_path_is_a_link_only_when_it_is_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.md");
        std::fs::write(&file, "x").unwrap();
        let absolute = file.display().to_string();
        assert_eq!(
            from_text(&absolute, 3, None),
            Some(Link::Path(file.clone()))
        );
        assert_eq!(
            from_text("notes.md", 2, Some(dir.path())),
            Some(Link::Path(dir.path().join("notes.md"))),
            "a relative run is read against the pane's directory"
        );
        assert_eq!(
            from_text("notes.md", 2, None),
            None,
            "and means nothing without one"
        );
        assert_eq!(
            from_text("missing.md", 2, Some(dir.path())),
            None,
            "a name that is not there is not a link"
        );
    }

    #[test]
    fn a_line_and_column_suffix_comes_off_a_path_that_exists_without_it() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("main.rs"), "x").unwrap();
        assert_eq!(
            from_text("main.rs:12:3", 2, Some(dir.path())),
            Some(Link::Path(dir.path().join("main.rs")))
        );
        // The suffix comes off only when the whole run failed, so a file that really carries
        // one still opens as itself.
        std::fs::write(dir.path().join("odd:12"), "x").unwrap();
        assert_eq!(
            from_text("odd:12", 1, Some(dir.path())),
            Some(Link::Path(dir.path().join("odd:12")))
        );
    }

    #[test]
    fn a_file_url_opens_the_path_it_names_and_only_on_this_machine() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("page.html");
        std::fs::write(&file, "x").unwrap();
        assert_eq!(
            from_marked(&format!("file://{}", file.display()), None),
            Some(Link::Path(file.clone()))
        );
        assert_eq!(
            from_marked("file://otherhost/etc/passwd", None),
            None,
            "a host this cannot reach is not opened as a local path"
        );
    }

    #[test]
    fn what_is_said_about_a_link_is_the_url_or_the_file_name() {
        assert_eq!(
            Link::Url("https://example.com/x".into()).said(),
            "https://example.com/x"
        );
        assert_eq!(
            Link::Path(PathBuf::from("/a/b/notes.md")).said(),
            "notes.md"
        );
        assert_eq!(
            Link::Path(PathBuf::from("/a/b/notes.md")).target(),
            "/a/b/notes.md"
        );
    }
}
