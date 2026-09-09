//! The transcript reader: the recap and the session name, cached by path and modification
//! time (architecture spec 3.6). Carried over from V1's `scanRecap`, in `recap.go` on the
//! `main` branch (`git show main:recap.go`), minus the directory-name encoding V2 does not
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

/// Reads a transcript once. The recap prefers the freshest `away_summary` or `/recap` output
/// and falls back to the last `ai-title`; the name is the last `/rename`. Dispatch is by the
/// entry's JSON type, not by a substring, so an assistant message that quotes these strings
/// is not mistaken for one (V1's note).
pub fn scan(text: &str) -> Transcript {
    let mut title: Option<String> = None;
    let mut summary: Option<(String, String)> = None; // (timestamp, text)
    let mut name: Option<String> = None;
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
                        name = Some(arg);
                    }
                } else if content.contains("<command-name>") {
                    pending_recap = false;
                }
            }
            _ => {}
        }
    }
    Transcript {
        recap: summary.map(|(_, s)| s).or(title),
        name,
    }
}

/// A cheap substring filter so the bulk of a transcript is never parsed as JSON.
fn relevant(line: &str) -> bool {
    line.contains("\"ai-title\"")
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

    #[test]
    fn a_session_with_only_an_ai_title_recaps_with_it() {
        let t = scan(&text("fresh"));
        assert_eq!(t.recap.as_deref(), Some("Session check cleanup"));
    }

    #[test]
    fn the_session_name_is_the_last_rename_and_is_absent_until_one_happens() {
        assert_eq!(scan(&text("fresh")).name, None);
        let t = scan(&text("renamed"));
        assert_eq!(t.name.as_deref(), Some("auth-cleanup"));
        assert_eq!(
            t.recap.as_deref(),
            Some("Session check cleanup"),
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
            Some("Session check cleanup")
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
            Some("Session check cleanup"),
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
            Some("Session check cleanup")
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
            Some("Early title"),
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
