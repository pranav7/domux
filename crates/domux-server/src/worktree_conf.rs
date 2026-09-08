//! `.domux/worktree.conf`: what a new slot needs from the main checkout. One directive per
//! line, in the order written. V1's `worktree_setup.go` is the behaviour this matches.
//!
//! The file lives in the project's repository, so its lines are as trusted as the project's
//! build is and no more. Two rules follow, and both are enforced at one place each:
//! `checked_relative` and `dst_in_slot` between them keep a `link` or a `copy` inside the slot,
//! and `parse` drops an argument carrying a control character or a text direction control.
//! What is left after those: a `link` or `copy` source is resolved through the main checkout's
//! own links, so a link there that leads outside is followed, and an argument can still hold a
//! zero-width character, which changes nothing about what the line runs. Nothing here spawns a process:
//! `run_lines` hands the run lines back and the caller types them into the workspace's first
//! pane, so slow setup is watched rather than waited on.

use crate::git::WORKTREE_DIR;
use crate::persist::with_suffix;
use std::fmt;
use std::path::{Component, Path, PathBuf};

/// Where the file lives, under the main checkout.
pub const CONF_PATH: &str = ".domux/worktree.conf";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    /// Symlink the slot's path at this relative path to the main checkout's file or folder.
    Link,
    /// Copy the main checkout's file to the slot, as an independent file.
    Copy,
    /// A shell line the workspace's first pane runs.
    Run,
}

impl Verb {
    /// The word the file uses, which is also the word every message uses.
    fn word(self) -> &'static str {
        match self {
            Verb::Link => "link",
            Verb::Copy => "copy",
            Verb::Run => "run",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Directive {
    pub verb: Verb,
    /// A path for `link` and `copy`, the rest of the line for `run`.
    pub arg: String,
}

/// What one apply did, for the summary line and for the warnings the pill and the log show.
#[derive(Debug, Default)]
pub struct Applied {
    pub linked: usize,
    pub copied: usize,
    pub skipped: Vec<String>,
}

impl Applied {
    pub fn summary(&self) -> Summary {
        Summary {
            linked: self.linked,
            copied: self.copied,
            ran: 0,
            skipped: self.skipped.len(),
        }
    }

    pub fn failures(&self) -> Vec<String> {
        self.skipped.clone()
    }
}

/// `linked 2, copied 1, ran 1, 1 skipped`, V1's `summarizeSetup` line. `ran` is filled by
/// the caller once it has typed the run lines into the pane.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Summary {
    pub linked: usize,
    pub copied: usize,
    pub ran: usize,
    pub skipped: usize,
}

impl fmt::Display for Summary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        if self.linked > 0 {
            parts.push(format!("linked {}", self.linked));
        }
        if self.copied > 0 {
            parts.push(format!("copied {}", self.copied));
        }
        if self.ran > 0 {
            parts.push(format!("ran {}", self.ran));
        }
        if self.skipped > 0 {
            parts.push(format!("{} skipped", self.skipped));
        }
        f.write_str(&parts.join(", "))
    }
}

/// Blank lines and `#` comments are skipped. An unknown verb or a missing argument is a
/// warning and that line is dropped, so an older domux reads a newer file without failing.
///
/// An argument holding a control character is dropped the same way. A run line is typed into
/// a pane, where a carriage return submits and an escape starts a sequence the emulator acts
/// on, so such a line would run something other than what the line shows. A link or a copy
/// path with one in it names a file whose name no one can read back, so all three verbs are
/// held to the one rule.
pub fn parse(text: &str) -> (Vec<Directive>, Vec<String>) {
    let mut directives = Vec::new();
    let mut warnings = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (verb, rest) = line.split_once(' ').unwrap_or((line, ""));
        let arg = rest.trim().to_string();
        let verb = match verb {
            "link" => Verb::Link,
            "copy" => Verb::Copy,
            "run" => Verb::Run,
            _ => {
                warnings.push(format!("unknown directive {line:?}"));
                continue;
            }
        };
        let word = verb.word();
        if arg.is_empty() {
            warnings.push(format!("{word}: missing argument"));
            continue;
        }
        if arg.chars().any(char::is_control) {
            warnings.push(format!("{word}: the argument contains a control character"));
            continue;
        }
        if arg.chars().any(is_direction_control) {
            warnings.push(format!(
                "{word}: the argument contains a text direction control"
            ));
            continue;
        }
        directives.push(Directive { verb, arg });
    }
    (directives, warnings)
}

/// The characters that reorder how the rest of a line reads without changing what it says, so
/// `run make<U+202E>...` shows the reader one command and hands the pane another. They are not
/// control characters in Unicode's sense, so `char::is_control` does not cover them. Only this
/// set is refused, not the whole of `Cf`: a zero-width joiner has innocent uses in text and
/// drives nothing.
fn is_direction_control(c: char) -> bool {
    matches!(c, '\u{61c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

/// Applies every `link` and `copy` in order. `run` is skipped here: `run_lines` gives it to
/// the caller, which types it into the workspace's first pane. Best effort: a failure is
/// recorded and the rest still runs.
pub fn apply(main: &Path, slot: &Path, directives: &[Directive]) -> Applied {
    let mut applied = Applied::default();
    let refusal = refuse_roots(main, slot);
    for d in directives {
        let word = match d.verb {
            Verb::Run => continue,
            verb => verb.word(),
        };
        let result = match &refusal {
            Some(reason) => Err(reason.clone()),
            None if d.verb == Verb::Link => link_into(main, slot, &d.arg),
            None => copy_into(main, slot, &d.arg),
        };
        match result {
            Ok(()) if d.verb == Verb::Link => applied.linked += 1,
            Ok(()) => applied.copied += 1,
            Err(reason) => applied.skipped.push(format!("{word} {}: {reason}", d.arg)),
        }
    }
    applied
}

/// The shell lines, in order, for the workspace's first pane.
pub fn run_lines(directives: &[Directive]) -> Vec<String> {
    directives
        .iter()
        .filter(|d| d.verb == Verb::Run)
        .map(|d| d.arg.clone())
        .collect()
}

/// The one check of the two directories every directive works between, so an operation added
/// later cannot miss it. A relative path resolves against wherever the server was started,
/// which is somebody's checkout, and the empty path is the worst of those: it is `.`. Neither
/// directory is created here either, because a slot that is not there is a workspace that was
/// removed, not one to build a tree for.
fn refuse_roots(main: &Path, slot: &Path) -> Option<String> {
    if !main.is_absolute() {
        return Some(format!(
            "the main checkout \"{}\" is not an absolute path; pass the project root",
            main.display()
        ));
    }
    if !slot.is_absolute() {
        return Some(format!(
            "the slot \"{}\" is not an absolute path; pass the slot directory under {}",
            slot.display(),
            WORKTREE_DIR
        ));
    }
    if !main.is_dir() {
        return Some(format!(
            "the main checkout \"{}\" is not a directory; check the project root",
            main.display()
        ));
    }
    if !slot.is_dir() {
        return Some(format!(
            "the slot \"{}\" is not a directory; create the workspace before applying its setup",
            slot.display()
        ));
    }
    // A slot that is the main checkout under another name (a symlink, or `/tmp` against
    // `/private/tmp`) is the same directory, and a `link` there would delete the real file and
    // leave a loop in its place. Both are directories that are there, so both resolve; a path
    // that somehow does not resolve is compared as it was given rather than called different.
    let real = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    if real(main) == real(slot) {
        return Some("refusing to apply the setup to the main checkout itself".to_string());
    }
    None
}

/// A `link` or `copy` argument, resolved to the path to join onto each of the two roots.
/// worktree.conf is a file in the project's repository, so a hostile or a careless line can
/// name any path the author can write. An absolute path, a `..`, or a path naming no file at
/// all is refused here, at the one place both verbs resolve their argument: `copy /etc/hosts`
/// would otherwise read outside the checkout, `link ../..` would write outside the slot, and
/// `link .` would delete the slot and leave a link to the main checkout where it was.
fn checked_relative(rel: &str) -> Result<PathBuf, String> {
    let mut path = PathBuf::new();
    for component in Path::new(rel).components() {
        match component {
            Component::Normal(part) => path.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(format!(
                    "the path \"{rel}\" leaves the main checkout with \"..\"; name a path inside it"
                ))
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "the path \"{rel}\" is absolute; name a path relative to the main checkout"
                ))
            }
        }
    }
    if path.as_os_str().is_empty() {
        return Err(format!(
            "the path \"{rel}\" names no file; name a path relative to the main checkout"
        ));
    }
    Ok(path)
}

/// Removes whatever is at `path`, following no symlink: a link to a folder is unlinked, not
/// emptied. `symlink_metadata` has already said something is there.
fn remove_entry(path: &Path) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if meta.is_dir() {
        std::fs::remove_dir_all(path).map_err(|e| e.to_string())
    } else {
        std::fs::remove_file(path).map_err(|e| e.to_string())
    }
}

/// The path in the slot to write `rel` at, with the folders above it created, and the one
/// guarantee that both verbs rest on: it is inside the slot.
///
/// `checked_relative` reads the directive's own text, and text is not enough. `Path::join` is
/// textual, and every syscall under it follows the links in the path, so a folder the slot
/// holds that is a link out of the slot carries the write out with it. It takes no attack:
/// `link vendor` puts a link to the main checkout at `slot/vendor`, and `link vendor/lib.txt`
/// after it would remove the main checkout's own file and leave a loop where it was. So the
/// walk starts at the resolved slot and takes one component at a time, descending only into a
/// folder that is a folder, creating the ones that are not there yet, and refusing anything
/// else, a link included whatever it leads to. The finished folder is then resolved once more, and the check on that is described
/// where it sits.
///
/// The last component is not resolved, and must not be: `remove_entry`, `symlink` and `rename`
/// all act on the name rather than on what it points at, which is what lets a link already in
/// the slot be replaced rather than written through.
fn dst_in_slot(slot: &Path, rel: &Path) -> Result<PathBuf, String> {
    let root = std::fs::canonicalize(slot).map_err(|e| format!("could not read the slot: {e}"))?;
    let name = rel
        .file_name()
        .ok_or_else(|| format!("the path \"{}\" names no file", rel.display()))?;
    let mut folder = root.clone();
    let mut walked = PathBuf::new();
    for part in rel.parent().unwrap_or(Path::new("")).components() {
        walked.push(part);
        folder.push(part);
        match std::fs::symlink_metadata(&folder) {
            Ok(meta) if meta.is_dir() => {}
            Ok(meta) if meta.is_symlink() => {
                // Where the link leads is not the point and is deliberately not looked at: the
                // walk refuses a link because following one is what carried the write out of
                // the slot, and a link that leads inside the slot today leads wherever it is
                // pointed tomorrow. So the refusal says the link is there, not that the path
                // escapes, which for a repository that tracks a symlinked folder would be
                // false.
                return Err(format!(
                    "the slot's \"{}\" is a link, and the setup does not walk through a link; \
                     name a path whose folders the slot holds",
                    walked.display()
                ));
            }
            Ok(_) => {
                return Err(format!(
                    "the slot's \"{}\" is a file, not a folder; name a path that does not go \
                     through it",
                    walked.display()
                ))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&folder).map_err(|e| {
                    format!("could not create the slot's \"{}\": {e}", walked.display())
                })?;
            }
            Err(e) => {
                return Err(format!(
                    "could not read the slot's \"{}\": {e}",
                    walked.display()
                ))
            }
        }
    }
    // The walk above descends only into folders it has just seen to be folders, so this second
    // resolve says nothing new about the state the walk read. What it catches is the state
    // changing after the walk read it, and that is one danger in two halves.
    //
    // The half it removes is in-process, and it is not hypothetical: the defect this guard was
    // added with was one directive creating a link that a later directive in the same file
    // walked through. A refactor that hoists this walk out of the loop, or caches its answer
    // across directives, brings that back, and this line is what refuses the write instead of
    // following it out of the slot.
    //
    // The half it cannot remove is another process swapping a component between this resolve
    // and the write below. Resolving and then writing is a race by construction, and no amount
    // of re-resolving closes it; only holding the folder open and writing relative to that
    // handle does, which needs a crate this milestone cannot add.
    //
    // So this check is deliberately untested: nothing a single-threaded test can build reaches
    // it, because the walk that runs first refuses every state that would make it fire. It is
    // kept for the in-process half, which is the one this file has already got wrong once.
    let folder = std::fs::canonicalize(&folder)
        .map_err(|e| format!("could not read the slot's \"{}\": {e}", walked.display()))?;
    if !folder.starts_with(&root) {
        return Err(format!(
            "the slot's \"{}\" resolves outside the slot; name a path the slot holds",
            walked.display()
        ));
    }
    Ok(folder.join(name))
}

/// What a lookup in the main checkout says. A file that is there but cannot be read is not a
/// file that is missing, and reporting it as missing sends the author looking for something
/// they can see (never fabricate).
fn source_error(rel: &str, e: &std::io::Error) -> String {
    if e.kind() == std::io::ErrorKind::NotFound {
        format!("source missing: {rel}")
    } else {
        format!("could not read {rel} in the main checkout: {e}")
    }
}

/// `slot/<rel>` becomes a symlink to `main/<rel>`, for a file or a folder alike. Anything
/// already at that name in the slot is removed first, so a second apply lands the same way
/// the first did.
fn link_into(main: &Path, slot: &Path, rel: &str) -> Result<(), String> {
    let checked = checked_relative(rel)?;
    let src = main.join(&checked);
    if let Err(e) = std::fs::symlink_metadata(&src) {
        return Err(source_error(rel, &e));
    }
    let dst = dst_in_slot(slot, &checked)?;
    if std::fs::symlink_metadata(&dst).is_ok() {
        remove_entry(&dst)?;
    }
    std::os::unix::fs::symlink(&src, &dst).map_err(|e| e.to_string())
}

/// `main/<rel>` is copied to `slot/<rel>` as an independent file, with its mode. Written to
/// `<name>.tmp` and renamed, so a reader in the slot sees the whole file or none of it.
fn copy_into(main: &Path, slot: &Path, rel: &str) -> Result<(), String> {
    let checked = checked_relative(rel)?;
    let src = main.join(&checked);
    let meta = std::fs::metadata(&src).map_err(|e| source_error(rel, &e))?;
    if meta.is_dir() {
        return Err(format!("copy is for files; use link for the folder {rel}"));
    }
    let dst = dst_in_slot(slot, &checked)?;
    // The scratch name carries the whole file name, so `keep.yml` writes `keep.yml.tmp` and a
    // real `keep.tmp` beside it is not the file being written. Anything at that name is ours
    // to remove: a leftover from a crash, or a symlink a copy would otherwise write through.
    let tmp = with_suffix(&dst, ".tmp");
    if std::fs::symlink_metadata(&tmp).is_ok() {
        remove_entry(&tmp)?;
    }
    std::fs::copy(&src, &tmp).map_err(|e| e.to_string())?;
    // A rename that fails leaves the whole file under the scratch name, in the author's git
    // working tree, until the next apply of this same directive. Take it back.
    std::fs::rename(&tmp, &dst).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}
