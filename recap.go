package main

import (
	"bufio"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"syscall"
	"time"
)

// Claude Code stores each session as a JSONL transcript under
// ~/.claude/projects/<sanitized-cwd>/<session-id>.jsonl and writes
// {"type":"ai-title","aiTitle":"…"} entries holding a one-line recap.
//
// To pick the *right* transcript for a tmux session we don't guess by mtime
// (a project dir accumulates many past sessions plus subagent files). Instead
// we read Claude's own live session registry at ~/.claude/sessions/<pid>.json:
// each holds {sessionId, cwd, pid, updatedAt}. Matching by cwd + a live pid
// gives the exact running session, and doubles as the "claude is live" gate —
// no live session for a cwd ⇒ no recap (so idle workspaces stay clean).
//
// No hook fires when the ai-title changes, so the title itself is still read by
// scanning the resolved transcript; results are cached by path + mtime.

// claudeProjectDirName mirrors Claude Code's transcript-dir encoding: every
// non-alphanumeric rune in the absolute cwd becomes '-'.
func claudeProjectDirName(path string) string {
	var b strings.Builder
	b.Grow(len(path))
	for _, r := range path {
		switch {
		case r >= 'a' && r <= 'z', r >= 'A' && r <= 'Z', r >= '0' && r <= '9':
			b.WriteRune(r)
		default:
			b.WriteByte('-')
		}
	}
	return b.String()
}

type claudeSession struct {
	SessionID string
	Cwd       string
	Pid       int
	UpdatedAt int64
	TTY       string // controlling tty (e.g. "ttys011"), derived from Pid via ps
}

// readClaudeSessions returns the entries from Claude's live session registry.
// Stale entries (dead processes) are filtered out via pidAlive. Read once per
// picker refresh and reused across sessions.
func readClaudeSessions() []claudeSession {
	home, err := os.UserHomeDir()
	if err != nil {
		return nil
	}
	dir := filepath.Join(home, ".claude", "sessions")
	entries, err := os.ReadDir(dir)
	if err != nil {
		return nil
	}
	var out []claudeSession
	for _, e := range entries {
		if e.IsDir() || !strings.HasSuffix(e.Name(), ".json") {
			continue
		}
		data, err := os.ReadFile(filepath.Join(dir, e.Name()))
		if err != nil {
			continue
		}
		var rec struct {
			SessionID string `json:"sessionId"`
			Cwd       string `json:"cwd"`
			Pid       int    `json:"pid"`
			UpdatedAt int64  `json:"updatedAt"`
		}
		if json.Unmarshal(data, &rec) != nil || rec.SessionID == "" {
			continue
		}
		if !pidAlive(rec.Pid) {
			continue
		}
		out = append(out, claudeSession{SessionID: rec.SessionID, Cwd: rec.Cwd, Pid: rec.Pid, UpdatedAt: rec.UpdatedAt})
	}
	// Derive each session's controlling tty from its pid so a recap can be pinned
	// to the exact tmux pane it runs in (a pane's tty == the tty of the claude
	// process inside it). Without this, multiple claude sessions sharing one cwd
	// (e.g. several tmux windows in the same repo) are indistinguishable by cwd
	// and every window row shows the same recap.
	ttys := ttysForPids(out)
	for i := range out {
		out[i].TTY = ttys[out[i].Pid]
	}
	return out
}

// ttysForPids returns each session's controlling tty keyed by pid via a single
// batched `ps` call. Pids with no controlling tty ("??") are absent from the map.
func ttysForPids(sessions []claudeSession) map[int]string {
	if len(sessions) == 0 {
		return nil
	}
	pids := make([]string, 0, len(sessions))
	for _, s := range sessions {
		if s.Pid > 0 {
			pids = append(pids, strconv.Itoa(s.Pid))
		}
	}
	if len(pids) == 0 {
		return nil
	}
	out, err := exec.Command("ps", "-o", "pid=,tty=", "-p", strings.Join(pids, ",")).Output()
	if err != nil {
		return nil
	}
	return parsePsTTYLines(string(out))
}

// parsePsTTYLines parses `ps -o pid=,tty=` output ("  49115 ttys011") into a
// pid→tty map. Lines whose tty marks no controlling terminal ("??" on macOS, "?"
// on Linux) or is blank are skipped, so an empty tty never becomes a match key.
func parsePsTTYLines(out string) map[int]string {
	result := map[int]string{}
	for _, line := range strings.Split(out, "\n") {
		fields := strings.Fields(line)
		if len(fields) != 2 {
			continue
		}
		pid, err := strconv.Atoi(fields[0])
		if err != nil {
			continue
		}
		tty := fields[1]
		if tty == "" || tty == "?" || tty == "??" {
			continue
		}
		result[pid] = tty
	}
	return result
}

// pidAlive reports whether pid is a live process. EPERM means alive but owned
// by another user — still alive for our purposes.
func pidAlive(pid int) bool {
	if pid <= 0 {
		return false
	}
	err := syscall.Kill(pid, 0)
	return err == nil || err == syscall.EPERM
}

// bestLiveSession returns the most recently active live Claude session whose cwd
// exactly matches one of paths, and true; (zero, false) when none match.
func bestLiveSession(sessions []claudeSession, paths ...string) (claudeSession, bool) {
	want := make(map[string]bool, len(paths))
	for _, p := range paths {
		if p != "" {
			want[p] = true
		}
	}
	if len(want) == 0 {
		return claudeSession{}, false
	}
	var best claudeSession
	found := false
	for _, s := range sessions {
		if !want[s.Cwd] {
			continue
		}
		if !found || s.UpdatedAt > best.UpdatedAt {
			best, found = s, true
		}
	}
	return best, found
}

// normalizeTTY strips the "/dev/" prefix tmux reports on pane_tty so it compares
// equal to the bare tty name ps emits (e.g. "/dev/ttys011" → "ttys011").
func normalizeTTY(tty string) string {
	return strings.TrimPrefix(strings.TrimSpace(tty), "/dev/")
}

// bestLiveSessionByTTY returns the most recently active live Claude session whose
// controlling tty matches one of ttys, and true. This pins a recap to the exact
// tmux pane running it — the reliable way to tell apart several claude sessions
// that share a cwd (multiple windows in one repo). Empty ttys never match, so a
// detached/ttyless session is never mis-assigned to a pane.
func bestLiveSessionByTTY(sessions []claudeSession, ttys ...string) (claudeSession, bool) {
	want := make(map[string]bool, len(ttys))
	for _, t := range ttys {
		if n := normalizeTTY(t); n != "" {
			want[n] = true
		}
	}
	if len(want) == 0 {
		return claudeSession{}, false
	}
	var best claudeSession
	found := false
	for _, s := range sessions {
		if s.TTY == "" || !want[normalizeTTY(s.TTY)] {
			continue
		}
		if !found || s.UpdatedAt > best.UpdatedAt {
			best, found = s, true
		}
	}
	return best, found
}

// recapForSession returns the recap of a specific live Claude session along with
// the timestamp of the transcript entry it came from (zero when the recap is an
// ai-title fallback, which carries no timestamp).
func recapForSession(s claudeSession) (string, time.Time) {
	home, err := os.UserHomeDir()
	if err != nil {
		return "", time.Time{}
	}
	path := filepath.Join(home, ".claude", "projects", claudeProjectDirName(s.Cwd), s.SessionID+".jsonl")
	return cachedRecap(path)
}

// recapForLiveSession returns the ai-title of the live Claude session whose cwd
// exactly matches one of paths, preferring the most recently active. "" when no
// live session matches.
func recapForLiveSession(sessions []claudeSession, paths ...string) string {
	best, ok := bestLiveSession(sessions, paths...)
	if !ok {
		return ""
	}
	recap, _ := recapForSession(best)
	return recap
}

// recapVisibleAfterClear reports whether a recap from a transcript entry stamped
// recapTime should still show, given the workspace's last `clear` time (RFC3339,
// "" = never cleared). A clear hides every recap dated at-or-before it; the line
// reappears only once a fresh recap entry (newer timestamp) is written — so the
// same Claude session shows its next recap, but not the stale one it had.
func recapVisibleAfterClear(recapTime time.Time, clearedAt string) bool {
	if clearedAt == "" {
		return true
	}
	t, err := time.Parse(timeFormat, clearedAt)
	if err != nil {
		return true
	}
	return recapTime.After(t)
}

type recapCacheEntry struct {
	mtime     time.Time
	title     string
	recapTime time.Time
}

var (
	recapMu    sync.Mutex
	recapCache = map[string]recapCacheEntry{}
)

// cachedRecap returns the transcript's recap and the timestamp of the entry it
// came from, caching by path + mtime so periodic refreshes don't re-scan
// multi-MB transcripts. It prefers the freshest away_summary/manual /recap
// (which carry a timestamp) and falls back to the ai-title (which Claude
// generates once, rarely refreshes, and carries no timestamp — so its recapTime
// is zero).
func cachedRecap(path string) (string, time.Time) {
	info, err := os.Stat(path)
	if err != nil {
		return "", time.Time{}
	}
	recapMu.Lock()
	defer recapMu.Unlock()
	if e, ok := recapCache[path]; ok && e.mtime.Equal(info.ModTime()) {
		return e.title, e.recapTime
	}
	summary, summaryTime, title := scanRecap(path)
	recap, recapTime := summary, summaryTime
	if recap == "" {
		recap, recapTime = title, time.Time{}
	}
	recapCache[path] = recapCacheEntry{mtime: info.ModTime(), title: recap, recapTime: recapTime}
	return recap, recapTime
}

const (
	stdoutOpen  = "<local-command-stdout>"
	stdoutClose = "</local-command-stdout>"
	recapCmd    = "<command-name>/recap</command-name>"
)

// scanRecap reads a transcript once and returns the freshest session recap and
// the last non-empty ai-title.
//
// Claude Code's Session recap (on by default) auto-generates a coherent one-line
// summary whenever a session is unfocused for 3+ minutes — i.e. exactly the
// sessions the switcher lists — and persists it as a system/away_summary entry.
// Running /recap manually persists the same kind of summary as a system/
// local_command entry (stdout-wrapped) immediately after the /recap user entry.
// We take whichever is most recent (file order is chronological); ai-title is
// the fallback for sessions too short to have a recap yet.
//
// Dispatch is JSON-typed (not raw substring) so assistant messages that merely
// quote these markers — like this very session — can't be mistaken for recaps.
// Reads line-by-line via bufio.Reader (not Scanner) so arbitrarily long
// assistant/tool lines don't overflow; a cheap substring prefilter avoids
// JSON-parsing the bulk of the transcript.
func scanRecap(path string) (summary string, summaryTime time.Time, title string) {
	f, err := os.Open(path)
	if err != nil {
		return "", time.Time{}, ""
	}
	defer f.Close()

	r := bufio.NewReaderSize(f, 256*1024)
	pendingRecap := false
	for {
		line, err := r.ReadString('\n')
		if relevantRecapLine(line) {
			// System entries (ai-title, away_summary, local_command) carry text at
			// top-level .content; user entries carry it at .message.content.
			var rec struct {
				Type      string          `json:"type"`
				Subtype   string          `json:"subtype"`
				AITitle   string          `json:"aiTitle"`
				Timestamp string          `json:"timestamp"`
				Content   json.RawMessage `json:"content"`
				Message   struct {
					Content json.RawMessage `json:"content"`
				} `json:"message"`
			}
			if json.Unmarshal([]byte(strings.TrimSpace(line)), &rec) == nil {
				switch {
				case rec.Type == "ai-title" && rec.AITitle != "":
					title = rec.AITitle
				case rec.Type == "system" && rec.Subtype == "away_summary":
					if s := recapLine(jsonString(rec.Content)); s != "" {
						summary, summaryTime = s, parseRecapTime(rec.Timestamp)
					}
				case rec.Type == "system" && rec.Subtype == "local_command" && pendingRecap:
					if inner := stdoutInner(jsonString(rec.Content)); inner != "" {
						summary, summaryTime = recapLine(inner), parseRecapTime(rec.Timestamp)
					}
					pendingRecap = false
				case rec.Type == "user":
					switch c := jsonString(rec.Message.Content); {
					case strings.Contains(c, recapCmd):
						pendingRecap = true
					case strings.Contains(c, "<command-name>"):
						pendingRecap = false // a different slash command
					}
				}
			}
		}
		if err != nil {
			break
		}
	}
	return summary, summaryTime, title
}

// parseRecapTime parses a transcript entry timestamp (RFC3339, e.g.
// "2026-05-21T10:27:43.547Z"). Zero time on missing/unparseable input.
func parseRecapTime(ts string) time.Time {
	if ts == "" {
		return time.Time{}
	}
	t, err := time.Parse(time.RFC3339, ts)
	if err != nil {
		return time.Time{}
	}
	return t
}

func relevantRecapLine(line string) bool {
	return strings.Contains(line, `"ai-title"`) ||
		strings.Contains(line, `"away_summary"`) ||
		strings.Contains(line, `"local_command"`) ||
		strings.Contains(line, "<command-name>")
}

// lastAITitle returns only the last ai-title (used by tests and as a fallback).
func lastAITitle(path string) string {
	_, _, title := scanRecap(path)
	return title
}

// jsonString decodes a JSON value as a string, or "" if it isn't one (e.g. the
// content array of an assistant/user message).
func jsonString(raw json.RawMessage) string {
	var s string
	if json.Unmarshal(raw, &s) == nil {
		return s
	}
	return ""
}

// stdoutInner returns the raw text between <local-command-stdout>…</…> in a
// local_command content string, or "" if the markers are absent.
func stdoutInner(content string) string {
	i := strings.Index(content, stdoutOpen)
	j := strings.LastIndex(content, stdoutClose)
	if i < 0 || j < i+len(stdoutOpen) {
		return ""
	}
	return content[i+len(stdoutOpen) : j]
}

// recapLine reduces a multi-sentence session recap to a tidy single clause:
// collapses whitespace, drops a leading "Goal:" label, and keeps just the first
// sentence. No length cap — the picker wraps the clause across as many lines as
// it needs so the full recap stays readable instead of truncated mid-word.
func recapLine(s string) string {
	s = strings.Join(strings.Fields(s), " ")
	s = strings.TrimSpace(strings.TrimPrefix(s, "Goal:"))
	if i := strings.Index(s, ". "); i >= 0 {
		s = s[:i]
	}
	return strings.TrimRight(s, ".")
}
