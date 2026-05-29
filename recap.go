package main

import (
	"bufio"
	"encoding/json"
	"os"
	"path/filepath"
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
		out = append(out, claudeSession{rec.SessionID, rec.Cwd, rec.Pid, rec.UpdatedAt})
	}
	return out
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

// recapForLiveSession returns the ai-title of the live Claude session whose cwd
// exactly matches one of paths, preferring the most recently active. "" when no
// live session matches.
func recapForLiveSession(sessions []claudeSession, paths ...string) string {
	want := make(map[string]bool, len(paths))
	for _, p := range paths {
		if p != "" {
			want[p] = true
		}
	}
	if len(want) == 0 {
		return ""
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
	if !found {
		return ""
	}
	home, err := os.UserHomeDir()
	if err != nil {
		return ""
	}
	path := filepath.Join(home, ".claude", "projects", claudeProjectDirName(best.Cwd), best.SessionID+".jsonl")
	return cachedRecap(path)
}

type recapCacheEntry struct {
	mtime time.Time
	title string
}

var (
	recapMu    sync.Mutex
	recapCache = map[string]recapCacheEntry{}
)

// cachedRecap returns the transcript's recap, caching by path + mtime so
// periodic refreshes don't re-scan multi-MB transcripts. It prefers the last
// user prompt (which updates every turn, so it stays fresh) and falls back to
// the ai-title (which Claude generates once and rarely refreshes, so it goes
// stale as a session moves on).
func cachedRecap(path string) string {
	info, err := os.Stat(path)
	if err != nil {
		return ""
	}
	recapMu.Lock()
	defer recapMu.Unlock()
	if e, ok := recapCache[path]; ok && e.mtime.Equal(info.ModTime()) {
		return e.title
	}
	summary, title := scanRecap(path)
	recap := summary
	if recap == "" {
		recap = title
	}
	recapCache[path] = recapCacheEntry{mtime: info.ModTime(), title: recap}
	return recap
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
func scanRecap(path string) (summary, title string) {
	f, err := os.Open(path)
	if err != nil {
		return "", ""
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
				Type    string          `json:"type"`
				Subtype string          `json:"subtype"`
				AITitle string          `json:"aiTitle"`
				Content json.RawMessage `json:"content"`
				Message struct {
					Content json.RawMessage `json:"content"`
				} `json:"message"`
			}
			if json.Unmarshal([]byte(strings.TrimSpace(line)), &rec) == nil {
				switch {
				case rec.Type == "ai-title" && rec.AITitle != "":
					title = rec.AITitle
				case rec.Type == "system" && rec.Subtype == "away_summary":
					if s := recapLine(jsonString(rec.Content)); s != "" {
						summary = s
					}
				case rec.Type == "system" && rec.Subtype == "local_command" && pendingRecap:
					if inner := stdoutInner(jsonString(rec.Content)); inner != "" {
						summary = recapLine(inner)
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
	return summary, title
}

func relevantRecapLine(line string) bool {
	return strings.Contains(line, `"ai-title"`) ||
		strings.Contains(line, `"away_summary"`) ||
		strings.Contains(line, `"local_command"`) ||
		strings.Contains(line, "<command-name>")
}

// lastAITitle returns only the last ai-title (used by tests and as a fallback).
func lastAITitle(path string) string {
	_, title := scanRecap(path)
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
// collapses whitespace, drops a leading "Goal:" label, keeps just the first
// sentence, and caps length. fitANSI further trims to column width.
func recapLine(s string) string {
	s = strings.Join(strings.Fields(s), " ")
	s = strings.TrimSpace(strings.TrimPrefix(s, "Goal:"))
	if i := strings.Index(s, ". "); i >= 0 {
		s = s[:i]
	}
	s = strings.TrimRight(s, ".")
	const max = 120
	if len(s) > max {
		s = strings.TrimSpace(s[:max]) + "…"
	}
	return s
}
