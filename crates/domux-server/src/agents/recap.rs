//! The transcript reader: the recap and the session name, read forward from a byte cursor per
//! file. Carried over from V1's `scanRecap`, in `recap.go` at commit e2fe7eb in this
//! repository's V1 history, minus the directory-name encoding V2 does not need because the
//! hooks give `transcript_path`.
//!
//! **A recap is an entry the agent wrote as a recap, and nothing else** (MUX-28). Claude Code
//! writes one as an `away_summary`, and `/recap` writes one as the output of a local command.
//! Where a session wrote neither, it has no recap and the row says nothing. M3 fell back to the
//! last thing the agent said in words and then to the `ai-title`, and both were shown in the
//! slot a recap belongs in: a row whose session had never written one said "Now let me verify
//! visually with screenshots before publishing", which is a sentence out of the middle of a
//! turn rather than an account of it.
//!
//! **The last recap stands until the agent writes another.** The entry arrives about once in
//! six turns, so blanking the recap at each new prompt would empty the row almost as fast as it
//! filled it. This is how the session name already behaves, and decision record 0034 records
//! the choice.

use crate::agents::manifests::{RecapSource, Registry};
use domux_core::api::Event;
use domux_core::ids::AgentId;
use domux_core::model::Model;
use serde_json::Value;
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// How much of a transcript domux has never seen before is read: enough to reach the last
/// recap and the last checkpoint, both of which sit at the end. Everything after the first
/// read is the bytes the agent appended, however large the file has grown.
pub const TAIL_BYTES: u64 = 2 * 1024 * 1024;

/// What one transcript says. Both fields are absent until the agent produces them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Transcript {
    /// The agent's own recap of what it is doing.
    pub recap: Option<String>,
    /// The name the agent gave the session.
    pub name: Option<String>,
}

/// One per core. Holds a cursor per transcript: how far it has read and what it found there.
#[derive(Default)]
pub struct RecapReader {
    open: HashMap<PathBuf, Reading>,
}

impl RecapReader {
    /// Reads whatever the agent has appended since last time.
    ///
    /// A transcript is appended to and never rewritten, so the bytes behind the cursor cannot
    /// change and re-reading them would answer what it answered before. This is what lets the
    /// core ask once a second: the question costs one `stat` while the file sits still, and a
    /// few kilobytes while the agent writes, rather than the whole file either way. M3 read
    /// the file whole on six hook events, and decision record 0027 kept the tool events out
    /// because at that price an 8 MB transcript would have been read dozens of times a turn.
    ///
    /// A file shorter than the cursor is not the file the cursor was counting, so the reading
    /// starts again from nothing.
    pub fn read(&mut self, path: &Path) -> Transcript {
        let Ok(meta) = std::fs::metadata(path) else {
            return Transcript::default();
        };
        let len = meta.len();
        let reading = self.open.entry(path.to_path_buf()).or_default();
        if len < reading.read_to {
            *reading = Reading::default();
        }
        if len == reading.read_to {
            return reading.transcript();
        }
        if !take(reading, path, len) {
            // Longer than the cursor, and still not the file the cursor was counting.
            *reading = Reading::default();
            take(reading, path, len);
        }
        reading.transcript()
    }

    /// The record went away; stop holding its cursor.
    pub fn forget(&mut self, path: &Path) {
        self.open.remove(path);
    }

    pub fn cached(&self) -> usize {
        self.open.len()
    }
}

/// Reads what the agent has written past the cursor, and answers whether it was reading the
/// file it thought it was.
///
/// The byte before the cursor is the newline the last read stopped on, so a file that does not
/// have one there was replaced rather than appended to. A shorter file gives itself away by its
/// length; one replaced by something longer would otherwise be read from the middle of a line
/// for the rest of its life.
fn take(reading: &mut Reading, path: &Path, len: u64) -> bool {
    // A transcript domux has never seen may already be hours long, and the recap and the
    // checkpoint it wants are both at the end of it.
    let first = reading.read_to == 0;
    let from = if first && len > TAIL_BYTES {
        len - TAIL_BYTES
    } else {
        reading.read_to
    };
    let base = if first { from } else { from - 1 };
    let Ok(mut file) = std::fs::File::open(path) else {
        return true;
    };
    if file.seek(SeekFrom::Start(base)).is_err() {
        return true;
    }
    let mut bytes = Vec::new();
    if file.read_to_end(&mut bytes).is_err() {
        return true;
    }
    // Where the entries this read has not seen begin.
    let start = if !first {
        if bytes.first() != Some(&b'\n') {
            return false;
        }
        1
    } else if from > 0 {
        // Starting partway into the file lands partway into a line, and half an entry is not
        // one. Only the first read of a long transcript starts anywhere but a line boundary.
        match bytes.iter().position(|b| *b == b'\n') {
            Some(i) => i + 1,
            None => return true,
        }
    } else {
        0
    };
    // The agent may be midway through writing the last line. Consuming to the last newline
    // leaves that line for the next read, which is when it will be whole.
    let Some(rel) = bytes[start..].iter().rposition(|b| *b == b'\n') else {
        return true;
    };
    let consumed = start + rel + 1;
    // Lossily, because a transcript half-written by a crashed process can carry one invalid
    // byte, and a good recap in an earlier line must survive it. Cutting on newlines never
    // splits a character: no byte of a multi-byte character is a newline.
    reading.feed(&String::from_utf8_lossy(&bytes[start..consumed]));
    reading.read_to = base + consumed as u64;
    true
}

/// Every record that reads a transcript, asked once. The core calls this from its once-a-second
/// tick, and it is the only thing that reads a transcript.
///
/// The tick rather than the hooks, because the recap does not arrive with the hook that ends
/// the turn: Claude Code writes the entry minutes later, so M3 read the file before it was
/// there and then showed what it had found on the turn before. That is the "recaps don't match"
/// half of MUX-28. A poll answers whenever the entry lands, and one rule in one place replaces
/// the six events M3 re-read on.
pub fn poll(model: &mut Model, reader: &mut RecapReader, manifests: &Registry) -> Vec<Event> {
    let reading: Vec<(AgentId, PathBuf)> = model
        .agents
        .iter()
        .filter(|a| {
            manifests.for_kind(a.kind).map(|m| m.recap) == Some(RecapSource::ClaudeTranscript)
        })
        .filter_map(|a| a.transcript_path.clone().map(|p| (a.id.clone(), p)))
        .collect();
    let mut events = Vec::new();
    for (agent, path) in reading {
        let t = reader.read(&path);
        events.extend(model.set_agent_recap(&agent, t.recap));
        // The name is not read the same way. The agent set it once and it stands until the
        // agent sets another, so an absent one is not evidence that it was cleared: the reader
        // answers `None` for a transcript it could not read, and starts a long one `TAIL_BYTES`
        // from its end, which can leave an early `/rename` outside the window. Never fabricate
        // cuts both ways, so a name is written only when one was found.
        if t.name.is_some() {
            model.set_agent_name(&agent, t.name);
        }
    }
    events
}

const STDOUT_OPEN: &str = "<local-command-stdout>";
const STDOUT_CLOSE: &str = "</local-command-stdout>";

/// One transcript being read: the cursor, and what the entries behind it said.
///
/// Forward, and keeping only the latest of each thing, which is all an append-only file needs.
/// M3 read backwards as well, to ask whether the newest summary belonged to the turn the file
/// ended on. That question existed to choose between the summary and the agent's last words,
/// and with the words gone there is nothing left for it to arbitrate.
#[derive(Default)]
struct Reading {
    /// The byte the next read starts at, always on a line boundary.
    read_to: u64,
    recap: Option<String>,
    custom_title: Option<String>,
    agent_name: Option<String>,
    renamed: Option<String>,
    /// A `/recap` has been seen and its output is the next local command's.
    pending_recap: bool,
}

impl Reading {
    fn transcript(&self) -> Transcript {
        Transcript {
            recap: self.recap.clone(),
            name: self
                .custom_title
                .clone()
                .or_else(|| self.agent_name.clone())
                .or_else(|| self.renamed.clone()),
        }
    }

    fn feed(&mut self, text: &str) {
        for line in text.lines() {
            self.entry(line);
        }
    }

    /// One entry. Dispatch is by the entry's JSON type, not by a substring, so an assistant
    /// message that quotes these strings is not mistaken for one (V1's note).
    ///
    /// **The name has three sources and they are read in this order** (MUX-19). Claude Code
    /// writes a `custom-title` entry every time it checkpoints a renamed session, so a rename
    /// made at any point is restated near the end of the file, where a reader that starts at
    /// the end still meets it. `agent-name` carries the same string beside it and stands in
    /// when there is no `custom-title`. The `/rename` command's own `<command-args>` is last:
    /// it is what M3 read and it is what stopped working, because that entry is written once,
    /// at the moment of the rename.
    fn entry(&mut self, line: &str) {
        if !relevant(line) {
            return;
        }
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else {
            return;
        };
        let kind = v.get("type").and_then(Value::as_str).unwrap_or("");
        let subtype = v.get("subtype").and_then(Value::as_str).unwrap_or("");
        match (kind, subtype) {
            ("custom-title", _) => {
                if let Some(t) = string_field(&v, "customTitle") {
                    self.custom_title = Some(t);
                }
            }
            ("agent-name", _) => {
                if let Some(t) = string_field(&v, "agentName") {
                    self.agent_name = Some(t);
                }
            }
            ("system", "away_summary") => {
                if let Some(c) = v.get("content").and_then(Value::as_str) {
                    let line = recap_line(c);
                    if !line.is_empty() {
                        self.recap = Some(line);
                    }
                }
            }
            ("system", "local_command") if self.pending_recap => {
                if let Some(inner) = v
                    .get("content")
                    .and_then(Value::as_str)
                    .and_then(stdout_inner)
                {
                    let line = recap_line(&inner);
                    if !line.is_empty() {
                        self.recap = Some(line);
                    }
                }
                self.pending_recap = false;
            }
            ("user", _) => {
                let content = v
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if content.contains("<command-name>/recap</command-name>") {
                    self.pending_recap = true;
                } else if content.contains("<command-name>/rename</command-name>") {
                    self.pending_recap = false;
                    if let Some(arg) = command_args(content) {
                        self.renamed = Some(arg);
                    }
                } else if content.contains("<command-name>") {
                    self.pending_recap = false;
                }
            }
            _ => {}
        }
    }
}

/// Reads a whole transcript at once, which is what a test has and what the first read of a
/// short file does.
pub fn scan(text: &str) -> Transcript {
    let mut reading = Reading::default();
    reading.feed(text);
    reading.transcript()
}

/// One JSON string field, absent when it is missing, not a string, or only spaces. Every name
/// source reads its field this way, so a checkpoint written with an empty title cannot blank a
/// name the reader already has (principle 4).
fn string_field(v: &Value, field: &str) -> Option<String> {
    v.get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// A cheap substring filter so the bulk of a transcript is never parsed as JSON.
///
/// The two name entries are matched on their hyphenated type words. `agentName` is a field on
/// ordinary messages in a session run by a named teammate, and the hyphen is what keeps those
/// lines out of the parser.
fn relevant(line: &str) -> bool {
    line.contains("\"custom-title\"")
        || line.contains("\"agent-name\"")
        || line.contains("\"away_summary\"")
        || line.contains("\"local_command\"")
        || line.contains("<command-name>")
}

fn command_args(content: &str) -> Option<String> {
    let start = content.find("<command-args>")? + "<command-args>".len();
    let end = content[start..].find("</command-args>")? + start;
    let arg = content[start..end].trim();
    if arg.is_empty() {
        None
    } else {
        Some(arg.to_string())
    }
}

fn stdout_inner(content: &str) -> Option<String> {
    let i = content.find(STDOUT_OPEN)? + STDOUT_OPEN.len();
    let j = content.rfind(STDOUT_CLOSE)?;
    if j < i {
        return None;
    }
    Some(content[i..j].to_string())
}

/// V1's `recapLine`: collapse whitespace, drop a leading `Goal:`, keep the first sentence,
/// drop a trailing full stop. No length cap: the row wraps (interface spec 12.20).
pub fn recap_line(s: &str) -> String {
    let joined = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = joined
        .strip_prefix("Goal:")
        .unwrap_or(&joined)
        .trim()
        .to_string();
    let first = match trimmed.find(". ") {
        Some(i) => trimmed[..i].to_string(),
        None => trimmed,
    };
    first.trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/transcripts")
            .join(format!("{name}.jsonl"))
    }

    fn text(name: &str) -> String {
        std::fs::read_to_string(fixture(name)).unwrap()
    }

    /// A recap entry is what a recap is. `full` holds an `away_summary` and, after it, the
    /// output of a `/recap`, and the later one is the recap.
    #[test]
    fn the_last_recap_entry_is_the_recap() {
        let t = scan(&text("full"));
        assert_eq!(
            t.recap.as_deref(),
            Some("Wrote the guard and deleted the duplicate checks")
        );
        assert_eq!(t.name, None);
    }

    /// MUX-28, the noise half: a session that wrote no recap has none. `fresh` carries an
    /// `ai-title` and a turn of the agent's words, and M3 showed each of them in turn as
    /// though the agent had written a recap.
    #[test]
    fn a_session_that_wrote_no_recap_reads_as_absent() {
        let t = scan(&text("fresh"));
        assert_eq!(t.recap, None);
        assert!(
            text("fresh").contains("ai-title"),
            "and not because the fixture is bare"
        );
    }

    /// The last recap stands until the agent writes another, which is what the name already
    /// does. Recaps arrive about once in six turns, so a rule that blanked the row at each new
    /// prompt would empty it almost as fast as it filled it (decision record 0034).
    #[test]
    fn a_recap_stands_through_the_turns_after_it() {
        let later = format!(
            "{}{}{}",
            text("full"),
            "{\"type\":\"user\",\"timestamp\":\"2026-09-04T11:00:00.000Z\",\"message\":{\"role\":\"user\",\"content\":\"now do the token refresh\"}}\n",
            "{\"type\":\"assistant\",\"timestamp\":\"2026-09-04T11:00:09.000Z\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"Refreshing the token on every request now. Two tests cover it.\"}]}}\n",
        );
        assert_eq!(
            scan(&later).recap.as_deref(),
            Some("Wrote the guard and deleted the duplicate checks")
        );
    }

    /// A `/recap` writes its line as the stdout of a local command, so the output counts only
    /// under the command that produced it. Any other slash command's output looks the same.
    #[test]
    fn only_a_recap_commands_output_is_read_as_a_recap() {
        let renamed = scan(&text("renamed"));
        assert_eq!(
            renamed.recap, None,
            "the /rename output is a local command's too"
        );
        assert_eq!(renamed.name.as_deref(), Some("auth-cleanup"));
    }

    /// The source MUX-19 added, and the one a real Claude Code session writes. The last
    /// checkpoint wins, and an assistant turn that says the words in its prose is not one.
    #[test]
    fn the_session_name_is_the_last_custom_title() {
        let t = scan(&text("titled"));
        assert_eq!(t.name.as_deref(), Some("token-refresh-fix"));
        assert_eq!(
            t.recap, None,
            "the name and the recap are read from one file and neither is the other"
        );
    }

    /// `agent-name` stands in where there is no `custom-title`, and the `/rename` args stand in
    /// where there is neither. Three sources in one order, checked by taking the first away and
    /// then the second, because each fallback passes on its own with the one above it present.
    #[test]
    fn the_name_falls_back_to_agent_name_and_then_to_the_rename_command() {
        let full = text("titled");
        let no_title: String = full
            .lines()
            .filter(|l| !l.contains("\"custom-title\""))
            .map(|l| format!("{l}\n"))
            .collect();
        assert_eq!(scan(&no_title).name.as_deref(), Some("token-refresh-fix"));
        let neither: String = no_title
            .lines()
            .filter(|l| !l.contains("\"agent-name\""))
            .map(|l| format!("{l}\n"))
            .collect();
        assert_eq!(scan(&neither).name, None);
        assert_eq!(
            scan(&format!("{neither}{}", text("renamed")))
                .name
                .as_deref(),
            Some("auth-cleanup"),
            "the /rename command's own args, which is what M3 read"
        );
    }

    /// A checkpoint written with an empty title leaves the name it found alone. Claude Code
    /// writes one of these entries per checkpoint, so a blank one is a line the reader will
    /// meet, and blanking a name on it would flicker the row back to `claude` (principle 4).
    #[test]
    fn an_empty_title_does_not_clear_a_name() {
        let t = scan(&format!(
            "{}{}",
            text("titled"),
            "{\"type\":\"custom-title\",\"customTitle\":\"   \",\"sessionId\":\"c1\"}\n"
        ));
        assert_eq!(t.name.as_deref(), Some("token-refresh-fix"));
    }

    #[test]
    fn recap_line_collapses_whitespace_drops_the_goal_label_and_keeps_one_sentence() {
        assert_eq!(
            recap_line("Goal:  Replaced three   session checks with one guard.\nTests pass."),
            "Replaced three session checks with one guard"
        );
        assert_eq!(recap_line("One clause only."), "One clause only");
        assert_eq!(recap_line("   "), "");
    }

    #[test]
    fn a_transcript_that_is_not_there_or_is_not_json_reads_as_absent() {
        let mut r = RecapReader::default();
        assert_eq!(
            r.read(Path::new("/nonexistent/x.jsonl")),
            Transcript::default()
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.jsonl");
        std::fs::write(
            &path,
            "not json\n{\"type\":\"system\",\"subtype\":\"away_summary\",\"content\":\"Still read\"}\n",
        )
        .unwrap();
        assert_eq!(
            r.read(&path).recap.as_deref(),
            Some("Still read"),
            "one bad line does not stop the reading"
        );
    }

    /// The reading picks up where it left off, so the bytes behind the cursor are never read
    /// twice. Proved by rewriting them into something the reader would answer differently if
    /// it went back over them.
    #[test]
    fn a_read_takes_only_what_the_agent_has_appended() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        let first =
            "{\"type\":\"system\",\"subtype\":\"away_summary\",\"content\":\"The first recap.\"}\n";
        std::fs::write(&path, first).unwrap();
        let mut r = RecapReader::default();
        assert_eq!(r.read(&path).recap.as_deref(), Some("The first recap"));
        assert_eq!(r.cached(), 1);

        // Those same bytes, overwritten in place with a name the reader has never been told,
        // padded to the length they had so the cursor still lands on the line's end. JSON
        // ignores the padding. A reader that went back over the bytes behind its cursor would
        // come away with the name; one that takes only what was appended cannot see it.
        let ghost = "{\"type\":\"custom-title\",\"customTitle\":\"ghost\"}";
        let behind = format!("{ghost}{}\n", " ".repeat(first.len() - 1 - ghost.len()));
        assert_eq!(behind.len(), first.len());
        let appended = "{\"type\":\"system\",\"subtype\":\"away_summary\",\"content\":\"The second recap.\"}\n";
        std::fs::write(&path, format!("{behind}{appended}")).unwrap();
        let t = r.read(&path);
        assert_eq!(
            t.recap.as_deref(),
            Some("The second recap"),
            "the appended line is read"
        );
        assert_eq!(
            t.name, None,
            "and the bytes behind the cursor are not read again"
        );
    }

    /// A file that sits still is one `stat`, which is what makes a poll once a second cheap.
    #[test]
    fn a_read_of_an_unchanged_file_answers_without_opening_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        std::fs::write(
            &path,
            "{\"type\":\"system\",\"subtype\":\"away_summary\",\"content\":\"Held.\"}\n",
        )
        .unwrap();
        let mut r = RecapReader::default();
        assert_eq!(r.read(&path).recap.as_deref(), Some("Held"));
        // Unreadable from here on: a reader that opened the file again would answer absent.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        assert_eq!(
            r.read(&path).recap.as_deref(),
            Some("Held"),
            "the length is unchanged, so there was nothing to open it for"
        );
    }

    /// A half-written last line is left for the next read, which is when it will be whole.
    #[test]
    fn a_line_the_agent_is_still_writing_is_read_once_it_ends() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        let entry =
            "{\"type\":\"system\",\"subtype\":\"away_summary\",\"content\":\"Torn in half.\"}\n";
        let (head, tail) = entry.split_at(40);
        std::fs::write(&path, head).unwrap();
        let mut r = RecapReader::default();
        assert_eq!(r.read(&path).recap, None, "half an entry is not one");
        std::fs::write(&path, entry).unwrap();
        assert_eq!(r.read(&path).recap.as_deref(), Some("Torn in half"));
        assert!(!tail.is_empty());
    }

    /// A transcript shorter than the cursor is not the transcript the cursor was counting, so
    /// the reading starts again rather than seeking past the end of the new one.
    #[test]
    fn a_replaced_transcript_is_read_from_the_beginning_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        std::fs::write(
            &path,
            "{\"type\":\"system\",\"subtype\":\"away_summary\",\"content\":\"The long gone one.\"}\n",
        )
        .unwrap();
        let mut r = RecapReader::default();
        assert_eq!(r.read(&path).recap.as_deref(), Some("The long gone one"));
        std::fs::write(
            &path,
            "{\"type\":\"custom-title\",\"customTitle\":\"new\"}\n",
        )
        .unwrap();
        let t = r.read(&path);
        assert_eq!(t.name.as_deref(), Some("new"));
        assert_eq!(t.recap, None, "the recap went with the file that held it");
        r.forget(&path);
        assert_eq!(r.cached(), 0);
    }

    /// A transcript replaced by a longer one is read from the beginning again. Length alone
    /// cannot tell this from an append, so the reader checks that the byte behind its cursor is
    /// still the newline it stopped on.
    #[test]
    fn a_transcript_replaced_by_a_longer_one_is_read_from_the_beginning_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        std::fs::write(
            &path,
            "{\"type\":\"custom-title\",\"customTitle\":\"the-short-one\"}\n",
        )
        .unwrap();
        let mut r = RecapReader::default();
        assert_eq!(r.read(&path).name.as_deref(), Some("the-short-one"));
        let longer = "{\"type\":\"custom-title\",\"customTitle\":\"the-longer-one\"}\n{\"type\":\"system\",\"subtype\":\"away_summary\",\"content\":\"And its recap.\"}\n";
        assert!(
            longer.len() > "{\"type\":\"custom-title\",\"customTitle\":\"the-short-one\"}\n".len()
        );
        std::fs::write(&path, longer).unwrap();
        let t = r.read(&path);
        assert_eq!(t.name.as_deref(), Some("the-longer-one"));
        assert_eq!(t.recap.as_deref(), Some("And its recap"));
    }

    /// The first read of a transcript that is already long starts `TAIL_BYTES` from its end,
    /// because the recap and the checkpoint it wants are both there.
    #[test]
    fn the_first_read_of_a_long_transcript_starts_near_its_end() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("huge.jsonl");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(
            f,
            "{{\"type\":\"system\",\"subtype\":\"away_summary\",\"content\":\"Wrong: this sits outside the window.\"}}"
        )
        .unwrap();
        let filler = "{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"padding padding padding padding padding\"}]}}";
        for _ in 0..30_000 {
            writeln!(f, "{filler}").unwrap();
        }
        writeln!(
            f,
            "{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"<command-name>/rename</command-name>\\n<command-args>tail-proof</command-args>\"}}}}"
        )
        .unwrap();
        f.flush().unwrap();
        assert!(
            std::fs::metadata(&path).unwrap().len() > TAIL_BYTES,
            "the fixture is long enough to trip the window"
        );
        let mut r = RecapReader::default();
        let t = r.read(&path);
        assert_eq!(
            t.name.as_deref(),
            Some("tail-proof"),
            "the end of the file is read"
        );
        assert_eq!(
            t.recap, None,
            "and the recap before the window is not, which is the window doing its work"
        );
    }
}
