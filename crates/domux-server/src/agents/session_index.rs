//! The Codex session index: where a Codex session's name comes from (decision record 0049).
//!
//! Codex's `/rename` appends one line to `session_index.jsonl` in its home directory,
//! `{"id": <thread id>, "thread_name": <name>, "updated_at": <time>}`, and never edits a line in
//! place. The newest line for a thread is its name, which is how Codex itself resolves one. A
//! root thread's id is the `session_id` its hooks send, so the record already holds the key.
//!
//! One index serves every Codex session under that home, so the reader holds one cursor per
//! index rather than one per record, and reads it the way the transcript reader reads a
//! transcript: forward, taking only what Codex appended (`agents::tail`).

use crate::agents::manifests::{NameSource, Registry};
use crate::agents::recap::string_field;
use crate::agents::tail::{Entries, Tail};
use domux_core::ids::AgentId;
use domux_core::model::Model;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The index's file name inside the Codex home.
pub const INDEX_FILE: &str = "session_index.jsonl";

/// The index a Codex session's name is written to, found from its rollout: Codex keeps a
/// rollout under `sessions/` or `archived_sessions/` in its home, dated folders below that, so
/// the home is the parent of the nearest folder with either name.
///
/// Found from the path the hook sent rather than from `CODEX_HOME`, because the server's
/// environment is not the agent's: a session started with its own `CODEX_HOME` writes its
/// rollout and its index under that, and the rollout path is the one place that says so. A
/// path of any other shape has no index domux can name, and the row shows the kind.
pub fn index_for(transcript: &Path) -> Option<PathBuf> {
    let sessions = transcript.ancestors().skip(1).find(|dir| {
        matches!(
            dir.file_name().and_then(|n| n.to_str()),
            Some("sessions" | "archived_sessions")
        )
    })?;
    let home = sessions.parent().filter(|h| !h.as_os_str().is_empty())?;
    Some(home.join(INDEX_FILE))
}

/// One per core. Holds a cursor per index and, behind it, the newest name for each thread.
#[derive(Default)]
pub struct SessionIndexReader {
    open: HashMap<PathBuf, Tail<Names>>,
}

impl SessionIndexReader {
    /// The name the index at `index` holds for `session`, after reading whatever Codex has
    /// appended since last time. `None` when the index is not there or names no such thread.
    pub fn name(&mut self, index: &Path, session: &str) -> Option<String> {
        self.open
            .entry(index.to_path_buf())
            .or_default()
            .read(index)?
            .by_thread
            .get(session)
            .cloned()
    }

    /// Stops holding the cursor of every index no record reads any more. An index is shared by
    /// every session under its home, so no single record's end is the moment to drop it.
    pub fn keep_only<'a>(&mut self, in_use: impl IntoIterator<Item = &'a PathBuf>) {
        let in_use: std::collections::HashSet<&PathBuf> = in_use.into_iter().collect();
        self.open.retain(|path, _| in_use.contains(path));
    }

    pub fn cached(&self) -> usize {
        self.open.len()
    }
}

/// What an index says: the newest name for each thread that has one.
#[derive(Default)]
struct Names {
    by_thread: HashMap<String, String>,
}

impl Entries for Names {
    /// A later line for a thread replaces an earlier one. A blank name is skipped rather than
    /// taken as a clear, which is the rule every name source follows (`recap::string_field`):
    /// a name the agent set stands until the agent sets another.
    fn feed(&mut self, text: &str) {
        for line in text.lines() {
            let Ok(v) = serde_json::from_str::<Value>(line.trim()) else {
                continue;
            };
            if let (Some(thread), Some(name)) =
                (string_field(&v, "id"), string_field(&v, "thread_name"))
            {
                self.by_thread.insert(thread, name);
            }
        }
    }
}

/// Every Codex record's name, asked once. The core calls this from its once-a-second tick,
/// beside `recap::poll`: a `/rename` sends no hook, so nothing but a poll would see it.
///
/// Answers whether any record's name changed. A name is not a transition and makes no event,
/// so this is what tells the tick the row has something new to draw.
pub fn poll(model: &mut Model, reader: &mut SessionIndexReader, manifests: &Registry) -> bool {
    let reading: Vec<(AgentId, PathBuf, String)> = model
        .agents
        .iter()
        .filter(|a| {
            manifests.for_kind(a.kind).map(|m| m.name) == Some(NameSource::CodexSessionIndex)
        })
        .filter_map(|a| {
            let index = index_for(a.transcript_path.as_deref()?)?;
            Some((a.id.clone(), index, a.session_id.clone()?))
        })
        .collect();
    reader.keep_only(reading.iter().map(|(_, index, _)| index));
    let mut changed = false;
    for (agent, index, session) in reading {
        // Written only when one was found, as the transcript's name is: a read that finds none
        // is not evidence that the agent cleared it.
        if let Some(name) = reader.name(&index, &session) {
            changed |= model.set_agent_name(&agent, Some(name));
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(thread: &str, name: &str) -> String {
        format!(
            "{{\"id\":\"{thread}\",\"thread_name\":\"{name}\",\"updated_at\":\"2026-09-14T10:38:51Z\"}}\n"
        )
    }

    /// An index in a fresh Codex home, holding `lines`.
    fn index_with(lines: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(INDEX_FILE);
        std::fs::write(&path, lines).unwrap();
        (dir, path)
    }

    /// Codex appends a line per rename and resolves a thread's name from the newest one, so a
    /// session renamed twice is called what it was called last.
    #[test]
    fn the_newest_entry_for_a_thread_is_its_name() {
        let (_dir, path) = index_with(&format!(
            "{}{}",
            entry("t-1", "babysit"),
            entry("t-1", "babysit-1244")
        ));
        let mut r = SessionIndexReader::default();
        assert_eq!(r.name(&path, "t-1").as_deref(), Some("babysit-1244"));
    }

    /// One index holds every thread under its home, and a record reads only its own.
    #[test]
    fn another_threads_entry_is_not_this_sessions_name() {
        let (_dir, path) = index_with(&format!(
            "{}{}",
            entry("t-1", "babysit-1244"),
            entry("t-2", "agent-harness")
        ));
        let mut r = SessionIndexReader::default();
        assert_eq!(r.name(&path, "t-1").as_deref(), Some("babysit-1244"));
        assert_eq!(r.name(&path, "t-2").as_deref(), Some("agent-harness"));
        assert_eq!(
            r.name(&path, "t-3"),
            None,
            "a thread never renamed has none"
        );
        assert_eq!(r.cached(), 1, "and all three answers came from one cursor");
    }

    /// Codex writes an empty name when a name is cleared. The row keeps the name it has, as it
    /// does for a transcript checkpoint with an empty title.
    #[test]
    fn a_blank_name_does_not_clear_the_one_before_it() {
        let (_dir, path) = index_with(&format!(
            "{}{}{}",
            entry("t-1", "babysit-1244"),
            entry("t-1", ""),
            entry("t-1", "   ")
        ));
        let mut r = SessionIndexReader::default();
        assert_eq!(r.name(&path, "t-1").as_deref(), Some("babysit-1244"));
    }

    /// A rename made after the first read is appended, and the next read finds it.
    #[test]
    fn a_rename_appended_after_a_read_is_found_on_the_next_one() {
        let (_dir, path) = index_with(&entry("t-2", "agent-harness"));
        let mut r = SessionIndexReader::default();
        assert_eq!(r.name(&path, "t-1"), None);
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        std::io::Write::write_all(&mut f, entry("t-1", "babysit-1244").as_bytes()).unwrap();
        assert_eq!(r.name(&path, "t-1").as_deref(), Some("babysit-1244"));
    }

    /// Deleting a thread makes Codex rewrite the index without that thread's lines, so the file
    /// the cursor was counting is gone and a shorter one stands in its place. The reading
    /// starts again, and a name the rewrite took out is not answered from memory.
    #[test]
    fn a_shorter_rewritten_index_is_read_from_the_beginning_again() {
        let (_dir, path) = index_with(&format!(
            "{}{}",
            entry("t-1", "a-long-gone-thread-name"),
            entry("t-2", "agent-harness")
        ));
        let mut r = SessionIndexReader::default();
        assert_eq!(
            r.name(&path, "t-1").as_deref(),
            Some("a-long-gone-thread-name")
        );
        std::fs::write(&path, entry("t-2", "agent-harness")).unwrap();
        assert_eq!(r.name(&path, "t-1"), None, "the deleted thread's line went");
        assert_eq!(r.name(&path, "t-2").as_deref(), Some("agent-harness"));
    }

    #[test]
    fn a_missing_index_or_a_line_that_is_not_json_names_nothing() {
        let mut r = SessionIndexReader::default();
        assert_eq!(
            r.name(Path::new("/nonexistent/session_index.jsonl"), "t-1"),
            None
        );
        let (_dir, path) = index_with(&format!("not json\n{}", entry("t-1", "still-read")));
        assert_eq!(
            r.name(&path, "t-1").as_deref(),
            Some("still-read"),
            "one bad line does not stop the reading"
        );
    }

    /// The home is the parent of the rollout's `sessions` or `archived_sessions` folder, however
    /// deep the dated folders go, and wherever the home itself lives.
    #[test]
    fn the_index_is_found_in_the_home_above_a_sessions_or_archived_sessions_rollout() {
        assert_eq!(
            index_for(Path::new(
                "/home/pranav/.codex/sessions/2026/09/14/rollout-2026-09-14T10-38-51-t-1.jsonl"
            )),
            Some(PathBuf::from("/home/pranav/.codex/session_index.jsonl"))
        );
        assert_eq!(
            index_for(Path::new(
                "/home/pranav/.codex/archived_sessions/rollout-2026-09-14T10-38-51-t-1.jsonl"
            )),
            Some(PathBuf::from("/home/pranav/.codex/session_index.jsonl"))
        );
        assert_eq!(
            index_for(Path::new(
                "/work/codex-home/sessions/2026/09/14/rollout-t-1.jsonl"
            )),
            Some(PathBuf::from("/work/codex-home/session_index.jsonl")),
            "a home moved with CODEX_HOME is found the same way"
        );
    }

    /// A path that is not under a `sessions` or `archived_sessions` folder is not a rollout
    /// domux knows the home of, and naming an index for it would be a guess.
    #[test]
    fn a_path_of_any_other_shape_has_no_index() {
        for path in [
            "/Users/pranav/.claude/projects/-Users-pranav-domux/c1.jsonl",
            "/tmp/rollout-t-1.jsonl",
            "sessions/rollout-t-1.jsonl",
            "/home/pranav/.codex/sessions",
        ] {
            assert_eq!(index_for(Path::new(path)), None, "{path}");
        }
    }
}
