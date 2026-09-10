//! The transcript reader: the recap and the session name, cached by path and modification
//! time (architecture spec 3.6). Carried over from V1's `scanRecap`, in `recap.go` at commit
//! e2fe7eb in this repository's V1 history, minus the directory-name encoding V2 does not
//! need because the hooks give `transcript_path`.

use serde_json::Value;
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Files up to this size are scanned whole, as V1 does.
pub const FULL_SCAN_BYTES: u64 = 8 * 1024 * 1024;
/// Above the limit: this much from the start, where the `ai-title` sits.
pub const HEAD_BYTES: u64 = 512 * 1024;
/// Above the limit: this much from the end, where the last summary and rename sit.
pub const TAIL_BYTES: u64 = 2 * 1024 * 1024;

/// What one transcript says. Both fields are absent until the agent produces them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Transcript {
    /// The agent's one-line summary of its last turn.
    pub recap: Option<String>,
    /// The name the agent gave the session with `/rename`.
    pub name: Option<String>,
}

/// One per core. Holds the last result per path with the modification time it was read at.
#[derive(Default)]
pub struct RecapReader {
    cache: HashMap<PathBuf, (SystemTime, Transcript)>,
}

impl RecapReader {
    pub fn read(&mut self, path: &Path) -> Transcript {
        let Ok(meta) = std::fs::metadata(path) else {
            return Transcript::default();
        };
        let Ok(mtime) = meta.modified() else {
            return Transcript::default();
        };
        if let Some((seen, t)) = self.cache.get(path) {
            if *seen == mtime {
                return t.clone();
            }
        }
        let text = read_bounded(path, meta.len()).unwrap_or_default();
        let t = scan(&text);
        self.cache.insert(path.to_path_buf(), (mtime, t.clone()));
        t
    }

    /// The record went away; stop holding its text.
    pub fn forget(&mut self, path: &Path) {
        self.cache.remove(path);
    }

    pub fn cached(&self) -> usize {
        self.cache.len()
    }
}

/// The whole file under the limit; above it, the head and the tail with the partial lines at
/// each cut dropped. Both branches read lossily: a transcript half-written by a crashed
/// process can carry one invalid byte, and a good recap sitting in an earlier valid line
/// must survive that rather than being thrown away with the whole file.
fn read_bounded(path: &Path, len: u64) -> std::io::Result<String> {
    if len <= FULL_SCAN_BYTES {
        let bytes = std::fs::read(path)?;
        return Ok(String::from_utf8_lossy(&bytes).into_owned());
    }
    let mut f = std::fs::File::open(path)?;
    let mut head = vec![0u8; HEAD_BYTES as usize];
    let n = f.read(&mut head)?;
    head.truncate(n);
    if let Some(cut) = head.iter().rposition(|b| *b == b'\n') {
        head.truncate(cut + 1);
    }
    f.seek(SeekFrom::End(-(TAIL_BYTES as i64)))?;
    let mut tail = Vec::new();
    f.read_to_end(&mut tail)?;
    if let Some(cut) = tail.iter().position(|b| *b == b'\n') {
        tail.drain(..=cut);
    }
    let mut out = String::from_utf8_lossy(&head).into_owned();
    out.push_str(&String::from_utf8_lossy(&tail));
    Ok(out)
}

const STDOUT_OPEN: &str = "<local-command-stdout>";
const STDOUT_CLOSE: &str = "</local-command-stdout>";

/// Reads a transcript once. Dispatch is by the entry's JSON type, not by a substring, so an
/// assistant message that quotes these strings is not mistaken for one (V1's note).
///
/// **The recap is the agent's summary of its last turn, and the summary has to belong to that
/// turn** (MUX-20). Claude Code writes an `away_summary` only now and then, so the freshest one
/// in a long session can describe work from several turns back; M3 showed it anyway, and a row
/// that had just finished something said what it had been doing half an hour earlier. So the
/// summary is used only when `last_turn` finds it after the last prompt, which is what "this
/// turn" means. Otherwise the recap is the last thing the agent actually said. A stale summary
/// is still better than nothing, so it stands in where the agent said nothing in words, and the
/// `ai-title` is last.
///
/// **The name has three sources and they are read in this order** (MUX-19). Claude Code
/// writes a `custom-title` entry every time it checkpoints a renamed session, so a rename made
/// at any point in the session is restated near the end of the file where the tail read always
/// reaches it. `agent-name` carries the same string beside it and stands in when there is no
/// `custom-title`. The `/rename` slash command's own `<command-args>` is last: it is what M3
/// read and it is what stopped working. That entry is written once, at the moment of the
/// rename, so a rename early in a long session falls outside the tail window; and no release
/// of Claude Code the author has a transcript from writes it at all, which is why a renamed
/// session went on showing `claude`.
pub fn scan(text: &str) -> Transcript {
    let mut title: Option<String> = None;
    let mut summary: Option<(String, String)> = None; // (timestamp, text)
    let mut custom_title: Option<String> = None;
    let mut agent_name: Option<String> = None;
    let mut renamed: Option<String> = None;
    let mut pending_recap = false;
    for line in text.lines() {
        if !relevant(line) {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        let kind = v.get("type").and_then(Value::as_str).unwrap_or("");
        let subtype = v.get("subtype").and_then(Value::as_str).unwrap_or("");
        let stamp = v
            .get("timestamp")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        match (kind, subtype) {
            ("custom-title", _) => {
                if let Some(t) = string_field(&v, "customTitle") {
                    custom_title = Some(t);
                }
            }
            ("agent-name", _) => {
                if let Some(t) = string_field(&v, "agentName") {
                    agent_name = Some(t);
                }
            }
            ("ai-title", _) => {
                if let Some(t) = v
                    .get("aiTitle")
                    .and_then(Value::as_str)
                    .filter(|s| !s.trim().is_empty())
                {
                    title = Some(t.to_string());
                }
            }
            ("system", "away_summary") => {
                if let Some(c) = v.get("content").and_then(Value::as_str) {
                    let line = recap_line(c);
                    if !line.is_empty() {
                        summary = Some((stamp, line));
                    }
                }
            }
            ("system", "local_command") if pending_recap => {
                if let Some(inner) = v
                    .get("content")
                    .and_then(Value::as_str)
                    .and_then(stdout_inner)
                {
                    let line = recap_line(&inner);
                    if !line.is_empty() {
                        summary = Some((stamp, line));
                    }
                }
                pending_recap = false;
            }
            ("user", _) => {
                let content = v
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if content.contains("<command-name>/recap</command-name>") {
                    pending_recap = true;
                } else if content.contains("<command-name>/rename</command-name>") {
                    pending_recap = false;
                    if let Some(arg) = command_args(content) {
                        renamed = Some(arg);
                    }
                } else if content.contains("<command-name>") {
                    pending_recap = false;
                }
            }
            _ => {}
        }
    }
    let turn = last_turn(text);
    let summary = summary.map(|(_, s)| s);
    let recap = if turn.summary_is_this_turn {
        summary
    } else {
        turn.said.or(summary)
    }
    .or(title);
    Transcript {
        recap,
        name: custom_title.or(agent_name).or(renamed),
    }
}

/// The turn the transcript ends on: the last thing the agent said in words, and whether the
/// freshest summary was written inside that turn.
#[derive(Debug, Default)]
struct LastTurn {
    said: Option<String>,
    summary_is_this_turn: bool,
}

/// How far back `last_turn` reads before it gives up. A turn is a prompt and the entries the
/// agent wrote answering it; a long one runs to a few hundred, and past that the answer is
/// worth less than the reading. Giving up says the summary is not this turn's, which is the
/// cautious half of the rule.
const TURN_LINES: usize = 500;

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
    line.contains("\"ai-title\"")
        || line.contains("\"custom-title\"")
        || line.contains("\"agent-name\"")
        || line.contains("\"away_summary\"")
        || line.contains("\"local_command\"")
        || line.contains("<command-name>")
}

/// Walks back from the end of the transcript to the prompt that started the last turn.
///
/// Backwards, and stopping at the prompt, because everything it asks about is at the end: the
/// answer is a handful of entries however long the session is. The forward pass above cannot
/// answer either question - "is this the newest summary" is a forward question and "was it
/// written after the last prompt" is not - and answering the second one forwards would mean
/// parsing every user entry in the file, which is what `relevant` exists to avoid.
///
/// A prompt is a `user` entry whose message is text the reader typed. A tool result is a
/// `user` entry too, and its content is a list of blocks rather than a string, which is what
/// tells the two apart. A slash command is text but it is not a turn, so it does not stop the
/// walk.
fn last_turn(text: &str) -> LastTurn {
    let mut out = LastTurn::default();
    // A `/recap` writes its line as the stdout of a local command, and walking back we meet
    // that output before the command that produced it. Any slash command's stdout looks the
    // same, so the output only counts once `/recap` itself turns up under it.
    let mut said_by_a_command = false;
    for line in text.lines().rev().take(TURN_LINES) {
        let line = line.trim();
        if !line.starts_with('{') {
            continue;
        }
        // The same cheap filter the forward pass uses, for the same reason: a transcript is
        // mostly attachments and tool results, and none of them is any of these.
        if !(line.contains("\"assistant\"")
            || line.contains("\"user\"")
            || line.contains("\"away_summary\"")
            || line.contains("\"local_command\""))
        {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        // A subagent writes into its parent's transcript, and what it said is not what this
        // session said.
        if v.get("isSidechain").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let kind = v.get("type").and_then(Value::as_str).unwrap_or("");
        let subtype = v.get("subtype").and_then(Value::as_str).unwrap_or("");
        match (kind, subtype) {
            ("system", "away_summary") => {
                out.summary_is_this_turn = true;
                return out;
            }
            ("system", "local_command") => said_by_a_command = true,
            ("assistant", _) => {
                if out.said.is_none() {
                    let line = recap_line(&assistant_text(&v));
                    if !line.is_empty() {
                        out.said = Some(line);
                    }
                }
            }
            ("user", _) => {
                let Some(text) = v
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(Value::as_str)
                else {
                    continue;
                };
                if said_by_a_command && text.contains("<command-name>/recap</command-name>") {
                    out.summary_is_this_turn = true;
                    return out;
                }
                if !text.trim_start().starts_with("<command-name>") {
                    return out;
                }
            }
            _ => {}
        }
    }
    out
}

/// The words in one assistant entry: its text blocks joined, with tool calls and thinking
/// left out. A message written as a bare string reads the same way, because both shapes are in
/// the transcripts this reads.
fn assistant_text(v: &Value) -> String {
    let Some(content) = v.get("message").and_then(|m| m.get("content")) else {
        return String::new();
    };
    if let Some(text) = content.as_str() {
        return text.to_string();
    }
    let Some(blocks) = content.as_array() else {
        return String::new();
    };
    blocks
        .iter()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|b| b.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join(" ")
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
    use std::path::{Path, PathBuf};

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/transcripts")
            .join(format!("{name}.jsonl"))
    }

    fn text(name: &str) -> String {
        std::fs::read_to_string(fixture(name)).unwrap()
    }

    #[test]
    fn the_freshest_summary_wins_over_the_ai_title() {
        let t = scan(&text("full"));
        assert_eq!(
            t.recap.as_deref(),
            Some("Wrote the guard and deleted the duplicate checks")
        );
        assert_eq!(t.name, None);
    }

    /// A session that has produced no summary at all recaps with the last thing the agent said
    /// (MUX-20). The `ai-title` in the same fixture is what M3 showed, and it names the whole
    /// conversation rather than the turn that just finished.
    #[test]
    fn a_session_with_no_summary_recaps_with_the_last_thing_the_agent_said() {
        let t = scan(&text("fresh"));
        assert_eq!(t.recap.as_deref(), Some("I will read the file first"));
    }

    /// And with the agent's words gone the `ai-title` is still there to fall back on, which is
    /// the last rung of the ladder.
    #[test]
    fn a_session_with_neither_a_summary_nor_words_recaps_with_the_ai_title() {
        let only_title: String = text("fresh")
            .lines()
            .filter(|l| !l.contains("\"assistant\""))
            .map(|l| format!("{l}\n"))
            .collect();
        assert_eq!(
            scan(&only_title).recap.as_deref(),
            Some("Session check cleanup")
        );
    }

    /// The rule MUX-20 turns on: a summary from before the last prompt is not this turn's, so
    /// the agent's own last words win over it. `full` ends with a `/recap` inside the last turn
    /// and its summary does win, which is the case above; here another prompt and another turn
    /// follow it.
    #[test]
    fn a_summary_from_an_earlier_turn_gives_way_to_the_last_thing_the_agent_said() {
        let later = format!(
            "{}{}{}",
            text("full"),
            "{\"type\":\"user\",\"timestamp\":\"2026-09-04T11:00:00.000Z\",\"message\":{\"role\":\"user\",\"content\":\"now do the token refresh\"}}\n",
            "{\"type\":\"assistant\",\"timestamp\":\"2026-09-04T11:00:09.000Z\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"Refreshing the token on every request now. Two tests cover it.\"}]}}\n",
        );
        assert_eq!(
            scan(&later).recap.as_deref(),
            Some("Refreshing the token on every request now")
        );
    }

    /// A turn the agent has not answered in words yet keeps the earlier summary rather than
    /// showing nothing: a stale line says more than a blank one, and it is the last rung
    /// before the title.
    #[test]
    fn a_turn_with_no_words_yet_keeps_the_earlier_summary() {
        let asked = format!(
            "{}{}",
            text("full"),
            "{\"type\":\"user\",\"timestamp\":\"2026-09-04T11:00:00.000Z\",\"message\":{\"role\":\"user\",\"content\":\"now do the token refresh\"}}\n",
        );
        assert_eq!(
            scan(&asked).recap.as_deref(),
            Some("Wrote the guard and deleted the duplicate checks")
        );
    }

    /// A subagent writes into its parent's transcript, and what it said is not what this
    /// session said.
    #[test]
    fn a_subagents_words_are_not_this_sessions_recap() {
        let with_sidechain = format!(
            "{}{}",
            text("fresh"),
            "{\"type\":\"assistant\",\"isSidechain\":true,\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"Subagent reporting back.\"}]}}\n",
        );
        assert_eq!(
            scan(&with_sidechain).recap.as_deref(),
            Some("I will read the file first")
        );
    }

    /// Thinking and tool calls are not words the reader was told. An assistant entry carrying
    /// only those is passed over for the one under it that has text.
    #[test]
    fn a_turn_that_is_all_tool_calls_reads_back_to_the_last_words() {
        let tools = format!(
            "{}{}",
            text("fresh"),
            "{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"tool_use\",\"name\":\"Read\",\"input\":{}}]}}\n",
        );
        assert_eq!(
            scan(&tools).recap.as_deref(),
            Some("I will read the file first")
        );
    }

    /// The source MUX-19 added, and the one a real Claude Code session writes. The last
    /// checkpoint wins, an assistant turn that says the words in its prose is not one, and the
    /// `ai-title` in the same file is still only a recap.
    #[test]
    fn the_session_name_is_the_last_custom_title() {
        let t = scan(&text("titled"));
        assert_eq!(t.name.as_deref(), Some("token-refresh-fix"));
        assert_eq!(
            t.recap.as_deref(),
            Some("Read the file"),
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
    fn the_session_name_is_the_last_rename_and_is_absent_until_one_happens() {
        assert_eq!(scan(&text("fresh")).name, None);
        let t = scan(&text("renamed"));
        assert_eq!(t.name.as_deref(), Some("auth-cleanup"));
        assert_eq!(
            t.recap.as_deref(),
            Some("I will read the file first"),
            "renaming does not change the recap"
        );
        let two = format!("{}{}", text("renamed"), "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"<command-name>/rename</command-name>\\n<command-args>token-refresh-fix</command-args>\"}}\n");
        assert_eq!(scan(&two).name.as_deref(), Some("token-refresh-fix"));
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
            "not json\n{\"type\":\"ai-title\",\"aiTitle\":\"Still read\"}\n",
        )
        .unwrap();
        assert_eq!(
            r.read(&path).recap.as_deref(),
            Some("Still read"),
            "one bad line does not stop the scan"
        );
    }

    #[test]
    fn a_second_read_of_an_unchanged_file_is_served_from_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        std::fs::copy(fixture("fresh"), &path).unwrap();
        let mut r = RecapReader::default();
        assert_eq!(
            r.read(&path).recap.as_deref(),
            Some("I will read the file first")
        );
        assert_eq!(r.cached(), 1);

        // Change the file's content without changing its mtime. A reader that
        // re-scans regardless of mtime would see the new content; the cache must
        // not, because `read` only re-scans when the mtime it saw last time has
        // moved.
        let original_mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
        std::fs::write(&path, text("full")).unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(original_mtime)
            .unwrap();
        assert_eq!(
            r.read(&path).recap.as_deref(),
            Some("I will read the file first"),
            "an unchanged mtime must be served from the cache, not re-scanned"
        );
        assert_eq!(r.cached(), 1);
    }

    #[test]
    fn a_changed_file_is_re_read_and_forget_drops_it_from_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        std::fs::copy(fixture("fresh"), &path).unwrap();
        let mut r = RecapReader::default();
        assert_eq!(
            r.read(&path).recap.as_deref(),
            Some("I will read the file first")
        );
        assert_eq!(r.cached(), 1);
        std::fs::write(&path, "").unwrap();
        // The mtime moved, so the empty file is read again and the recap goes.
        assert_eq!(r.read(&path), Transcript::default());
        r.forget(&path);
        assert_eq!(r.cached(), 0);
    }

    #[test]
    fn a_transcript_larger_than_the_full_scan_limit_reads_its_head_and_tail_but_not_the_excluded_middle(
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("huge.jsonl");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "{{\"type\":\"ai-title\",\"aiTitle\":\"Early title\"}}").unwrap();
        let filler = "{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"padding padding padding padding padding\"}]}}";
        for i in 0..120_000 {
            writeln!(f, "{filler}").unwrap();
            if i == 60_000 {
                // Sits deep in the excluded middle, nowhere near either the head or
                // the tail window. If the size guard above were removed and the
                // whole file scanned, this away_summary would beat the head's
                // title and the assertion below would see it instead: proof the
                // middle is truly dropped, not merely that the fixture is big.
                writeln!(
                    f,
                    "{{\"type\":\"system\",\"subtype\":\"away_summary\",\"timestamp\":\"2026-09-04T10:30:00.000Z\",\"content\":\"Wrong: this sits in the excluded middle.\"}}"
                )
                .unwrap();
            }
        }
        // A rename that only exists in the tail window: if the tail read broke (a
        // bad seek offset, say), this would come back absent.
        writeln!(
            f,
            "{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"<command-name>/rename</command-name>\\n<command-args>tail-proof</command-args>\"}}}}"
        )
        .unwrap();
        f.flush().unwrap();
        assert!(
            std::fs::metadata(&path).unwrap().len() > FULL_SCAN_BYTES,
            "the fixture is big enough to trip the limit"
        );
        let mut r = RecapReader::default();
        let t = r.read(&path);
        assert_eq!(
            t.recap.as_deref(),
            Some("padding padding padding padding padding"),
            "the tail's own last words, and never the summary buried in the excluded middle"
        );
        assert_ne!(
            t.recap.as_deref(),
            Some("Wrong: this sits in the excluded middle"),
            "a summary buried in the excluded middle must not reach the recap"
        );
        assert_eq!(
            t.name.as_deref(),
            Some("tail-proof"),
            "the tail must still be read for the rename"
        );
    }

    #[test]
    fn a_bad_utf8_byte_after_a_good_line_does_not_discard_the_whole_transcript() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad_utf8.jsonl");
        // Simulates a process that crashed mid-write: a good line, then an invalid
        // UTF-8 byte with no closing newline. Well under FULL_SCAN_BYTES, so this
        // exercises the full-scan branch of read_bounded, not the head/tail branch
        // (which was already lossy).
        let mut bytes =
            b"{\"type\":\"ai-title\",\"aiTitle\":\"Valid before the crash\"}\n".to_vec();
        bytes.push(0xFF);
        std::fs::write(&path, &bytes).unwrap();
        let mut r = RecapReader::default();
        assert_eq!(
            r.read(&path).recap.as_deref(),
            Some("Valid before the crash"),
            "an invalid byte must not discard the whole transcript"
        );
    }
}
